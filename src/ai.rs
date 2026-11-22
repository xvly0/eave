
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_NULL};

pub const AI_MODELS: [(&str, &str); 5] = [
    ("3.5 Lite", "gemini-3.5-flash-lite"),
    ("3.8 Flash", "gemini-3.8-flash"),
    ("3.0 Flash", "gemini-3-flash-preview"),
    ("3.1 Lite", "gemini-3.1-flash-lite"),
    ("Gemma 4", "gemma-4-26b-a4b-it"),
];

pub const DEFAULT_AI_MODEL: &str = "gemini-3.5-flash-lite";

pub const AI_HOTKEYS: [(&str, u32, u32); 4] = [
    ("Alt + Space", 0x0001 | 0x4000, 0x20),
    ("Ctrl + Space", 0x0002 | 0x4000, 0x20),
    ("Alt + Q", 0x0001 | 0x4000, 0x51),
    ("Ctrl + Alt + E", 0x0002 | 0x0001 | 0x4000, 0x45),
];

pub fn model_index_for_id(id: &str) -> Option<usize> {
    AI_MODELS
        .iter()
        .position(|(_, m)| m.eq_ignore_ascii_case(id.trim()))
}

pub fn model_label_for_id(id: &str) -> &str {
    model_index_for_id(id)
        .map(|i| AI_MODELS[i].0)
        .unwrap_or(id)
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum AiPhase {
    #[default]
    Idle,
    Input,
    Busy,
    Done,
}

#[derive(Clone, Debug, Default)]
pub struct AiStreamSnapshot {
    pub request_id: u64,
    pub running: bool,
    pub finished: bool,
    pub response: String,
    pub error: Option<String>,
    pub model_label: Option<String>,
}

pub struct AiService {
    shared: Arc<Mutex<AiStreamSnapshot>>,
    cancel_flag: Arc<AtomicBool>,
    next_req_id: AtomicU64,
    wake_hwnd_raw: isize,
}

impl AiService {
    pub fn new(wake_hwnd: HWND) -> Self {
        Self {
            shared: Arc::new(Mutex::new(AiStreamSnapshot::default())),
            cancel_flag: Arc::new(AtomicBool::new(false)),
            next_req_id: AtomicU64::new(1),
            wake_hwnd_raw: wake_hwnd.0 as isize,
        }
    }

    pub fn snapshot(&self) -> AiStreamSnapshot {
        self.shared
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default()
    }

    pub fn cancel(&mut self) {
        self.cancel_flag.store(true, Ordering::SeqCst);
        self.cancel_flag = Arc::new(AtomicBool::new(false));
        if let Ok(mut g) = self.shared.lock() {
            g.running = false;
        }
    }

    pub fn start_request(
        &mut self,
        api_key: String,
        model: String,
        prompt: String,
        history: Vec<(String, String)>,
    ) -> u64 {
        self.cancel_flag.store(true, Ordering::SeqCst);
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel_flag = Arc::clone(&cancel);

        let req_id = self.next_req_id.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut g) = self.shared.lock() {
            *g = AiStreamSnapshot {
                request_id: req_id,
                running: true,
                finished: false,
                response: String::new(),
                error: None,
                model_label: None,
            };
        }

        let shared = Arc::clone(&self.shared);
        let wake_raw = self.wake_hwnd_raw;
        let _ = thread::Builder::new()
            .name("eave-ai".to_string())
            .spawn(move || {
                run_gemini_stream(
                    req_id,
                    api_key,
                    model,
                    prompt,
                    history,
                    shared,
                    cancel,
                    wake_raw,
                );
            });
        req_id
    }
}

fn wake_ui(wake_raw: isize) {
    if wake_raw != 0 {
        unsafe {
            let _ = PostMessageW(HWND(wake_raw as *mut c_void), WM_NULL, WPARAM(0), LPARAM(0));
        }
    }
}

type HInternet = *mut c_void;

const WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY: u32 = 4;
const WINHTTP_FLAG_SECURE: u32 = 0x0080_0000;
const WINHTTP_ADDREQ_FLAG_ADD: u32 = 0x2000_0000;
const WINHTTP_ADDREQ_FLAG_REPLACE: u32 = 0x8000_0000;
const WINHTTP_QUERY_STATUS_CODE: u32 = 19;
const WINHTTP_QUERY_FLAG_NUMBER: u32 = 0x2000_0000;
const INTERNET_DEFAULT_HTTPS_PORT: u16 = 443;

#[link(name = "winhttp")]
extern "system" {
    fn WinHttpOpen(
        psz_agent_w: *const u16,
        dw_access_type: u32,
        psz_proxy_w: *const u16,
        psz_proxy_bypass_w: *const u16,
        dw_flags: u32,
    ) -> HInternet;
    fn WinHttpSetTimeouts(
        h_internet: HInternet,
        n_resolve_timeout: i32,
        n_connect_timeout: i32,
        n_send_timeout: i32,
        n_receive_timeout: i32,
    ) -> i32;
    fn WinHttpConnect(
        h_session: HInternet,
        pswz_server_name: *const u16,
        n_server_port: u16,
        dw_reserved: u32,
    ) -> HInternet;
    fn WinHttpOpenRequest(
        h_connect: HInternet,
        pwsz_verb: *const u16,
        pwsz_object_name: *const u16,
        pwsz_version: *const u16,
        pwsz_referrer: *const u16,
        ppwsz_accept_types: *const *const u16,
        dw_flags: u32,
    ) -> HInternet;
    fn WinHttpAddRequestHeaders(
        h_request: HInternet,
        lpsz_headers: *const u16,
        dw_headers_length: u32,
        dw_modifiers: u32,
    ) -> i32;
    fn WinHttpSendRequest(
        h_request: HInternet,
        lpsz_headers: *const u16,
        dw_headers_length: u32,
        lp_optional: *const c_void,
        dw_optional_length: u32,
        dw_total_length: u32,
        dw_context: usize,
    ) -> i32;
    fn WinHttpReceiveResponse(h_request: HInternet, lp_reserved: *mut c_void) -> i32;
    fn WinHttpQueryHeaders(
        h_request: HInternet,
        dw_info_level: u32,
        pwsz_name: *const u16,
        lp_buffer: *mut c_void,
        lpdw_buffer_length: *mut u32,
        lpdw_index: *mut u32,
    ) -> i32;
    fn WinHttpQueryDataAvailable(h_request: HInternet, lpdw_number_of_bytes_available: *mut u32)
        -> i32;
    fn WinHttpReadData(
        h_request: HInternet,
        lp_buffer: *mut c_void,
        dw_number_of_bytes_to_read: u32,
        lpdw_number_of_bytes_read: *mut u32,
    ) -> i32;
    fn WinHttpCloseHandle(h_internet: HInternet) -> i32;
}

struct WinHttpGuard(HInternet);
impl Drop for WinHttpGuard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                let _ = WinHttpCloseHandle(self.0);
            }
        }
    }
}

fn to_wide_null(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

enum StreamAttemptOutcome {
    Success,
    Cancelled,
    RetryableError(String),
    FatalError(String),
}

fn push_delta_smoothly(
    req_id: u64,
    delta: &str,
    shared: &Mutex<AiStreamSnapshot>,
    cancel: &AtomicBool,
    wake_raw: isize,
) {
    if delta.is_empty() {
        return;
    }
    let mut start = 0usize;
    let bytes = delta.as_bytes();
    while start < bytes.len() {
        if cancel.load(Ordering::SeqCst) {
            return;
        }
        let mut end = start;
        while end < bytes.len() && bytes[end].is_ascii_whitespace() {
            end += 1;
        }
        while end < bytes.len() && !bytes[end].is_ascii_whitespace() {
            end += 1;
        }
        if end < bytes.len() && bytes[end] == b' ' {
            end += 1;
        }
        while end < bytes.len() && !delta.is_char_boundary(end) {
            end += 1;
        }
        let word = &delta[start..end];
        if !word.is_empty() {
            if let Ok(mut g) = shared.lock() {
                if g.request_id == req_id {
                    g.response.push_str(word);
                }
            }
            wake_ui(wake_raw);
            if end < bytes.len() {
                thread::sleep(std::time::Duration::from_millis(38));
            }
        }
        start = end;
    }
}

#[allow(clippy::too_many_arguments)]
fn try_gemini_stream_once(
    req_id: u64,
    key: &str,
    model_id: &str,
    sys_prompt: &str,
    history: &[(String, String)],
    prompt: &str,
    with_google_search: bool,
    recv_timeout_ms: i32,
    shared: &Mutex<AiStreamSnapshot>,
    cancel: &AtomicBool,
    wake_raw: isize,
) -> StreamAttemptOutcome {
    let is_gemma = model_id.starts_with("gemma-");
    let tools_field = if with_google_search && !is_gemma {
        ",\"tools\":[{\"google_search\":{}}]"
    } else {
        ""
    };
    let mut contents_json = String::from("[");
    let mut first = true;
    for (i, (role, text)) in history.iter().enumerate() {
        if !first {
            contents_json.push(',');
        }
        first = false;
        let msg = if is_gemma && i == 0 && role == "user" {
            format!("{sys_prompt}\n\n{text}")
        } else {
            text.clone()
        };
        contents_json.push_str(&format!(
            "{{\"role\":\"{}\",\"parts\":[{{\"text\":\"{}\"}}]}}",
            json_escape(role),
            json_escape(&msg)
        ));
    }
    if !first {
        contents_json.push(',');
    }
    let cur_user_msg = if is_gemma && history.is_empty() {
        format!("{sys_prompt}\n\n{prompt}")
    } else {
        prompt.to_string()
    };
    contents_json.push_str(&format!(
        "{{\"role\":\"user\",\"parts\":[{{\"text\":\"{}\"}}]}}",
        json_escape(&cur_user_msg)
    ));
    contents_json.push(']');

    let body_json = if is_gemma {
        format!(
            "{{\"contents\":{contents_json},\"generationConfig\":{{\"maxOutputTokens\":1024}}}}"
        )
    } else {
        format!(
            "{{\"systemInstruction\":{{\"parts\":[{{\"text\":\"{}\"}}]}},\
              \"contents\":{contents_json}\
              {tools_field},\
              \"generationConfig\":{{\"maxOutputTokens\":1024}}}}",
            json_escape(sys_prompt),
        )
    };
    let body_bytes = body_json.as_bytes();

    let agent_w = to_wide_null("Eave/0.1");
    let host_w = to_wide_null("generativelanguage.googleapis.com");
    let verb_w = to_wide_null("POST");
    let path_str =
        format!("/v1beta/models/{model_id}:streamGenerateContent?alt=sse&key={key}");
    let path_w = to_wide_null(&path_str);
    let headers_str = format!(
        "Content-Type: application/json\r\nAccept: text/event-stream\r\nx-goog-api-key: {key}\r\n"
    );
    let headers_w = to_wide_null(&headers_str);

    let h_sess = unsafe {
        WinHttpOpen(
            agent_w.as_ptr(),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            std::ptr::null(),
            std::ptr::null(),
            0,
        )
    };
    if h_sess.is_null() {
        return StreamAttemptOutcome::FatalError("Impossibile inizializzare WinHTTP.".to_string());
    }
    let _g_sess = WinHttpGuard(h_sess);
    unsafe {
        let _ = WinHttpSetTimeouts(h_sess, 6_000, 7_000, 10_000, recv_timeout_ms);
    }

    let h_conn = unsafe {
        WinHttpConnect(
            h_sess,
            host_w.as_ptr(),
            INTERNET_DEFAULT_HTTPS_PORT,
            0,
        )
    };
    if h_conn.is_null() {
        return StreamAttemptOutcome::RetryableError(
            "Impossibile connettersi a generativelanguage.googleapis.com.".to_string(),
        );
    }
    let _g_conn = WinHttpGuard(h_conn);

    let h_req = unsafe {
        WinHttpOpenRequest(
            h_conn,
            verb_w.as_ptr(),
            path_w.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            WINHTTP_FLAG_SECURE,
        )
    };
    if h_req.is_null() {
        return StreamAttemptOutcome::FatalError("Errore creazione richiesta HTTPS.".to_string());
    }
    let _g_req = WinHttpGuard(h_req);

    unsafe {
        let _ = WinHttpAddRequestHeaders(
            h_req,
            headers_w.as_ptr(),
            u32::MAX,
            WINHTTP_ADDREQ_FLAG_ADD | WINHTTP_ADDREQ_FLAG_REPLACE,
        );
    }

    if cancel.load(Ordering::SeqCst) {
        return StreamAttemptOutcome::Cancelled;
    }

    let sent = unsafe {
        WinHttpSendRequest(
            h_req,
            std::ptr::null(),
            0,
            body_bytes.as_ptr() as *const c_void,
            body_bytes.len() as u32,
            body_bytes.len() as u32,
            0,
        )
    };
    if sent == 0 {
        return StreamAttemptOutcome::RetryableError(
            "Connessione fallita: verifica internet o proxy.".to_string(),
        );
    }

    if unsafe { WinHttpReceiveResponse(h_req, std::ptr::null_mut()) } == 0 {
        return StreamAttemptOutcome::RetryableError(
            "Nessuna risposta dalle API Gemini (timeout o modello sovraccarico).".to_string(),
        );
    }

    let mut status_code: u32 = 0;
    let mut status_len = std::mem::size_of::<u32>() as u32;
    unsafe {
        let _ = WinHttpQueryHeaders(
            h_req,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            std::ptr::null(),
            &mut status_code as *mut u32 as *mut c_void,
            &mut status_len,
            std::ptr::null_mut(),
        );
    }

    if status_code != 200 {
        let err_raw = read_all_body(h_req, cancel, 8192);
        let detail = extract_json_string_field(&err_raw, "message")
            .unwrap_or_else(|| format!("HTTP {status_code}"));
        return match status_code {
            401 | 403 => StreamAttemptOutcome::FatalError(
                "API Key non valida o senza permessi.".to_string(),
            ),
            400 => StreamAttemptOutcome::RetryableError(format!("Richiesta non valida: {detail}")),
            404 => StreamAttemptOutcome::RetryableError(format!(
                "Modello '{model_id}' non disponibile."
            )),
            429 => StreamAttemptOutcome::RetryableError(format!(
                "Quota esaurita per '{model_id}' (429)."
            )),
            500..=599 => StreamAttemptOutcome::RetryableError(format!(
                "Server Gemini '{model_id}' occupato ({status_code})."
            )),
            _ => StreamAttemptOutcome::RetryableError(format!(
                "Errore Gemini ({status_code}): {detail}"
            )),
        };
    }

    let base_label = model_label_for_id(model_id);
    if let Ok(mut g) = shared.lock() {
        if g.request_id == req_id {
            g.model_label = Some(base_label.to_string());
        }
    }

    let mut pending = Vec::<u8>::with_capacity(4096);
    let mut got_any_text = false;
    let mut grounded_web = false;
    loop {
        if cancel.load(Ordering::SeqCst) {
            return StreamAttemptOutcome::Cancelled;
        }
        let mut avail: u32 = 0;
        if unsafe { WinHttpQueryDataAvailable(h_req, &mut avail) } == 0 || avail == 0 {
            break;
        }
        let chunk_len = (avail as usize).min(16 * 1024);
        let mut buf = vec![0u8; chunk_len];
        let mut read: u32 = 0;
        if unsafe {
            WinHttpReadData(
                h_req,
                buf.as_mut_ptr() as *mut c_void,
                chunk_len as u32,
                &mut read,
            )
        } == 0
            || read == 0
        {
            break;
        }
        pending.extend_from_slice(&buf[..read as usize]);

        while let Some(nl_pos) = pending.iter().position(|&b| b == b'\n') {
            let line_bytes: Vec<u8> = pending.drain(..=nl_pos).collect();
            let line = String::from_utf8_lossy(&line_bytes);
            let trimmed = line.trim();
            if let Some(json_payload) = trimmed.strip_prefix("data:") {
                let payload = json_payload.trim();
                if payload.is_empty() || payload == "[DONE]" {
                    continue;
                }
                if !grounded_web
                    && (payload.contains("\"webSearchQueries\"")
                        || payload.contains("\"groundingChunks\""))
                {
                    grounded_web = true;
                    if let Ok(mut g) = shared.lock() {
                        if g.request_id == req_id {
                            g.model_label = Some(format!("{base_label} · Web"));
                        }
                    }
                    wake_ui(wake_raw);
                }
                let delta = extract_gemini_sse_text(payload);
                if !delta.is_empty() {
                    got_any_text = true;
                    push_delta_smoothly(req_id, &delta, shared, cancel, wake_raw);
                }
            }
        }
    }

    if !pending.is_empty() && !cancel.load(Ordering::SeqCst) {
        let line = String::from_utf8_lossy(&pending);
        if let Some(json_payload) = line.trim().strip_prefix("data:") {
            let payload = json_payload.trim();
            if !grounded_web
                && (payload.contains("\"webSearchQueries\"")
                    || payload.contains("\"groundingChunks\""))
            {
                if let Ok(mut g) = shared.lock() {
                    if g.request_id == req_id {
                        g.model_label = Some(format!("{base_label} · Web"));
                    }
                }
            }
            let delta = extract_gemini_sse_text(payload);
            if !delta.is_empty() {
                got_any_text = true;
                push_delta_smoothly(req_id, &delta, shared, cancel, wake_raw);
            }
        }
    }

    if cancel.load(Ordering::SeqCst) {
        StreamAttemptOutcome::Cancelled
    } else if got_any_text {
        StreamAttemptOutcome::Success
    } else {
        StreamAttemptOutcome::RetryableError("Risposta vuota ricevuta da Gemini.".to_string())
    }
}

static GOOGLE_SEARCH_TOOL_BLOCKED: AtomicBool = AtomicBool::new(false);

#[allow(clippy::too_many_arguments)]
fn run_gemini_stream(
    req_id: u64,
    api_key: String,
    model: String,
    prompt: String,
    history: Vec<(String, String)>,
    shared: Arc<Mutex<AiStreamSnapshot>>,
    cancel: Arc<AtomicBool>,
    wake_raw: isize,
) {
    let started_at = std::time::Instant::now();
    let key = api_key.trim();
    if key.is_empty() {
        finish_with_error(
            req_id,
            "Inserisci la tua Gemini API Key in Impostazioni → Eave AI.",
            &shared,
            wake_raw,
        );
        return;
    }

    let primary_model = if model.trim().is_empty() {
        DEFAULT_AI_MODEL
    } else {
        model.trim()
    };

    let sys_prompt = "You are Eave AI, a compact assistant living inside the Windows notch. \
        Always reply in the same language as the user, clearly, directly, and well-formatted \
        using clean Markdown when helpful (clear line breaks, bold, bullet lists, code blocks, \
        blockquotes, compact tables, or LaTeX math).";

    let mut candidates: Vec<&str> = vec![primary_model];
    for fb in [
        "gemini-3.5-flash-lite",
        "gemini-3-flash-preview",
        "gemini-3.1-flash-lite",
    ] {
        if !candidates.iter().any(|m| m.eq_ignore_ascii_case(fb)) {
            candidates.push(fb);
        }
    }

    let mut last_err = "Nessuna risposta dalle API Gemini.".to_string();
    for (idx, &cand_model) in candidates.iter().enumerate() {
        let tool_modes: &[bool] =
            if cand_model.starts_with("gemma-") || GOOGLE_SEARCH_TOOL_BLOCKED.load(Ordering::Relaxed)
            {
                &[false]
            } else {
                &[true, false]
            };
        let mut tool_429_on_search = false;
        for &use_google_search in tool_modes {
            if cancel.load(Ordering::SeqCst) {
                return;
            }
            let recv_timeout_ms = if idx == 0 { 4_500 } else { 12_000 };
            match try_gemini_stream_once(
                req_id,
                key,
                cand_model,
                sys_prompt,
                &history,
                &prompt,
                use_google_search,
                recv_timeout_ms,
                &shared,
                &cancel,
                wake_raw,
            ) {
                StreamAttemptOutcome::Success => {
                    if !use_google_search && tool_429_on_search {
                        GOOGLE_SEARCH_TOOL_BLOCKED.store(true, Ordering::Relaxed);
                    }
                    let min_wave_dur = std::time::Duration::from_millis(1350);
                    while started_at.elapsed() < min_wave_dur && !cancel.load(Ordering::SeqCst) {
                        thread::sleep(std::time::Duration::from_millis(30));
                    }
                    if !cancel.load(Ordering::SeqCst) {
                        if let Ok(mut g) = shared.lock() {
                            if g.request_id == req_id {
                                g.running = false;
                                g.finished = true;
                            }
                        }
                        wake_ui(wake_raw);
                    }
                    return;
                }
                StreamAttemptOutcome::Cancelled => return,
                StreamAttemptOutcome::FatalError(err) => {
                    finish_with_error(req_id, &err, &shared, wake_raw);
                    return;
                }
                StreamAttemptOutcome::RetryableError(err) => {
                    if use_google_search && err.contains("429") {
                        tool_429_on_search = true;
                    }
                    let is_timeout_or_503 = err.contains("timeout") || err.contains("occupato");
                    last_err = err;
                    if is_timeout_or_503 {
                        break;
                    }
                }
            }
        }
    }

    if !cancel.load(Ordering::SeqCst) {
        finish_with_error(req_id, &last_err, &shared, wake_raw);
    }
}

fn read_all_body(h_req: HInternet, cancel: &AtomicBool, cap: usize) -> String {
    let mut out = Vec::new();
    while out.len() < cap && !cancel.load(Ordering::SeqCst) {
        let mut avail = 0u32;
        if unsafe { WinHttpQueryDataAvailable(h_req, &mut avail) } == 0 || avail == 0 {
            break;
        }
        let n = (avail as usize).min(cap - out.len());
        let mut buf = vec![0u8; n];
        let mut read = 0u32;
        if unsafe { WinHttpReadData(h_req, buf.as_mut_ptr() as *mut c_void, n as u32, &mut read) }
            == 0
            || read == 0
        {
            break;
        }
        out.extend_from_slice(&buf[..read as usize]);
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn finish_with_error(
    req_id: u64,
    msg: &str,
    shared: &Mutex<AiStreamSnapshot>,
    wake_raw: isize,
) {
    if let Ok(mut g) = shared.lock() {
        if g.request_id == req_id {
            g.running = false;
            g.finished = true;
            g.error = Some(msg.to_string());
        }
    }
    wake_ui(wake_raw);
}

pub fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 16);
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out
}

pub fn extract_gemini_sse_text(json: &str) -> String {
    let mut out = String::new();
    let bytes = json.as_bytes();
    let mut i = 0usize;
    while i + 6 < bytes.len() {
        if let Some(rel) = json[i..].find("\"text\"") {
            let key_start = i + rel;
            let mut p = key_start + 6;
            while p < bytes.len() && bytes[p].is_ascii_whitespace() {
                p += 1;
            }
            if p < bytes.len() && bytes[p] == b':' {
                p += 1;
                while p < bytes.len() && bytes[p].is_ascii_whitespace() {
                    p += 1;
                }
                if p < bytes.len() && bytes[p] == b'"' {
                    if let Some((decoded, end_pos)) = decode_json_string(json, p) {
                        let obj_start = json[..key_start].rfind('{').unwrap_or(0);
                        let obj_end = json[end_pos..]
                            .find('}')
                            .map(|r| end_pos + r)
                            .unwrap_or(bytes.len());
                        let part_slice = &json[obj_start..obj_end];
                        let is_thought = part_slice.contains("\"thought\":true")
                            || part_slice.contains("\"thought\": true");
                        if !is_thought {
                            out.push_str(&decoded);
                        }
                        i = end_pos;
                        continue;
                    }
                }
            }
            i = key_start + 6;
        } else {
            break;
        }
    }
    out
}

fn extract_json_string_field(json: &str, field: &str) -> Option<String> {
    let needle = format!("\"{field}\"");
    let pos = json.find(&needle)?;
    let bytes = json.as_bytes();
    let mut p = pos + needle.len();
    while p < bytes.len() && bytes[p].is_ascii_whitespace() {
        p += 1;
    }
    if p >= bytes.len() || bytes[p] != b':' {
        return None;
    }
    p += 1;
    while p < bytes.len() && bytes[p].is_ascii_whitespace() {
        p += 1;
    }
    if p >= bytes.len() || bytes[p] != b'"' {
        return None;
    }
    decode_json_string(json, p).map(|(s, _)| s)
}

fn decode_json_string(src: &str, quote_idx: usize) -> Option<(String, usize)> {
    let bytes = src.as_bytes();
    if bytes.get(quote_idx) != Some(&b'"') {
        return None;
    }
    let mut out = String::new();
    let mut i = quote_idx + 1;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'"' {
            return Some((out, i + 1));
        }
        if b == b'\\' {
            i += 1;
            if i >= bytes.len() {
                break;
            }
            match bytes[i] {
                b'"' => out.push('"'),
                b'\\' => out.push('\\'),
                b'/' => out.push('/'),
                b'n' => out.push('\n'),
                b'r' => out.push('\r'),
                b't' => out.push('\t'),
                b'b' => out.push('\u{0008}'),
                b'f' => out.push('\u{000C}'),
                b'u' if i + 4 < bytes.len() => {
                    if let Ok(code) = u16::from_str_radix(&src[i + 1..i + 5], 16) {
                        // Handle UTF-16 surrogate pairs (e.g. emojis)
                        if (0xD800..=0xDBFF).contains(&code)
                            && i + 10 < bytes.len()
                            && &bytes[i + 5..i + 7] == b"\\u"
                        {
                            if let Ok(low) = u16::from_str_radix(&src[i + 7..i + 11], 16) {
                                if (0xDC00..=0xDFFF).contains(&low) {
                                    let cp = 0x10000
                                        + (((code as u32 - 0xD800) << 10) | (low as u32 - 0xDC00));
                                    if let Some(ch) = char::from_u32(cp) {
                                        out.push(ch);
                                    }
                                    i += 11;
                                    continue;
                                }
                            }
                        }
                        if let Some(ch) = char::from_u32(code as u32) {
                            out.push(ch);
                        }
                        i += 5;
                        continue;
                    }
                }
                other => out.push(other as char),
            }
            i += 1;
        } else {
            let ch = src[i..].chars().next()?;
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    None
}

// Exact port of `siri-wave-rs/src/wave.wgsl` into a fast premultiplied BGRA8 buffer

pub const AI_WAVE_W: u32 = 180;
pub const AI_WAVE_H: u32 = 72;

#[inline]
fn spectral4(s: usize) -> (f32, f32, f32) {
    let x = s as f32;
    (
        ((x - 3.0).abs() - 1.0).clamp(0.0, 1.0),
        (2.0 - (x - 2.0).abs()).clamp(0.0, 1.0),
        (2.0 - (x - 4.0).abs()).clamp(0.0, 1.0),
    )
}

/// (`AI_WAVE_W * AI_WAVE_H * 4` bytes, premultiplied BGRA8).
pub fn render_siri_wave_frame(bgra: &mut [u8], width: u32, height: u32, t: f32, energy: f32) {
    let w = width as usize;
    let h = height as usize;
    if bgra.len() < w * h * 4 || w == 0 || h == 0 {
        return;
    }
    let e = energy.clamp(0.0, 1.0);

    const PI: f32 = std::f32::consts::PI;
    const AMPLITUDE: f32 = 0.32;
    const FREQ: f32 = 1.1;
    const ABER_FREQ: f32 = 1.0;
    const SPEED: f32 = 2.4;
    const WAVE_SCALE: f32 = 0.6;
    const ABERRATION: f32 = 2.6;
    const THICKNESS: f32 = 3.0;
    const INTENSITY: f32 = 2.0;
    const FALLOFF: f32 = 1.7;
    const EDGE_MASK: f32 = 0.4;
    const BAND_FILL: f32 = 30000.0;
    const BAND_THICK: f32 = 0.08;
    const SOFTNESS: f32 = 2.5;
    const LOW_AMP: f32 = 6.0;
    const LOW_INT: f32 = 1.5;
    const MID_ABER: f32 = 0.8;
    const MID_ABAMP: f32 = 0.05;
    const MID_SOFT: f32 = 0.4;
    const HIGH_ABER: f32 = 0.5;
    const HIGH_ABAMP: f32 = 0.06;

    let rw = width as f32;
    let rh = height as f32;
    let aspect = 1.0_f32;
    let low = (0.45 + 0.45 * (t * 0.8).sin() * (t * 0.37 + 1.0).sin()).clamp(0.0, 1.0) * e;
    let mid = (0.40 + 0.40 * (t * 1.7 + 2.0).sin() * (t * 0.53).sin()).clamp(0.0, 1.0) * e;
    let high = (0.30 + 0.30 * (t * 2.9 + 4.0).sin() * (t * 0.71 + 2.0).sin()).clamp(0.0, 1.0) * e;

    let drift = t.rem_euclid(20.0 * PI) * SPEED;
    let amp_scale = 0.14 + 0.86 * e;
    let a1 = (AMPLITUDE + 0.01 * low * LOW_AMP) * amp_scale;
    let a2 = (a1 + (mid * MID_ABAMP + high * HIGH_ABAMP) * amp_scale).max(a1);
    let ab_max = (ABERRATION + mid * MID_ABER + high * HIGH_ABER) * (0.28 + 0.72 * e);
    let th = 0.01 * (THICKNESS * (0.72 + 0.28 * e));
    let inten = 0.01 * (INTENSITY * (0.76 + 0.24 * e) + low * LOW_INT);
    let soft = 0.01 * (SOFTNESS + mid * MID_SOFT).max(0.0);
    let soft2 = soft * soft;
    let band_amt = 1e-4 * BAND_FILL * inten * (0.25 + 0.75 * e);
    let inv_scale = 1.0 / WAVE_SCALE;

    let hues = [spectral4(0), spectral4(1), spectral4(2), spectral4(3)];
    let den_r = hues[0].0 + hues[1].0 + hues[2].0 + hues[3].0;
    let den_g = hues[0].1 + hues[1].1 + hues[2].1 + hues[3].1;
    let den_b = hues[0].2 + hues[1].2 + hues[2].2 + hues[3].2;
    let inv_den = (1.0 / den_r, 1.0 / den_g, 1.0 / den_b);

    let mut col_ymain = vec![0.0_f32; w];
    let mut col_yl = vec![[0.0_f32; 4]; w];
    let mut col_gauss = vec![0.0_f32; w];
    for x in 0..w {
        let px_screen = ((x as f32 + 0.5) * 2.0 / rw - 1.0) * aspect;
        let px = px_screen * inv_scale;
        let x_n = px;
        let env_c = (PI * 0.5 * (0.9 * x_n).abs().min(1.0)).cos();
        let env = env_c * env_c;
        col_ymain[x] = a1 * env * (px * FREQ + drift).sin();
        for s in 0..4 {
            let ab = -ab_max + (2.0 * ab_max) * (s as f32 / 3.0);
            col_yl[x][s] = a2 * env * (px * ABER_FREQ + drift + ab).sin();
        }
        let gx = x_n * FALLOFF;
        col_gauss[x] = (-(gx * gx)).exp();
    }

    let dim_mul = 0.68 + 0.32 * e;
    for y in 0..h {
        let y_screen = ((y as f32 + 0.5) * 2.0 / rh - 1.0) * 0.62;
        let py = y_screen * inv_scale;
        let em_t = ((y_screen.abs() - 1.0) / -EDGE_MASK).clamp(0.0, 1.0);
        let em = em_t * em_t * (3.0 - 2.0 * em_t);
        let v_edge = ((1.0 - ((y as f32 + 0.5) * 2.0 / rh - 1.0).abs()) / 0.18).clamp(0.0, 1.0);
        let v_mask = em * v_edge * v_edge * (3.0 - 2.0 * v_edge);

        let row_off = y * w * 4;
        for x in 0..w {
            let y_main = col_ymain[x];
            let yl = col_yl[x];
            let mut nr = 0.0_f32;
            let mut ng = 0.0_f32;
            let mut nb = 0.0_f32;

            for s in 0..4 {
                let y_s = yl[s];
                let d = (py - y_s).abs();
                let line = inten / ((d * d + soft2).sqrt() + th);
                let lo = y_main.min(y_s);
                let hi = y_main.max(y_s);
                let d_band = (py - hi).max(lo - py).max(0.0);
                let band = band_amt / (d_band + BAND_THICK);
                let w_val = line + band;
                let (hr, hg, hb) = hues[s];
                nr += hr * w_val;
                ng += hg * w_val;
                nb += hb * w_val;
            }

            let dm = (py - y_main).abs();
            let core = 0.5 * inten / ((dm * dm + soft2).sqrt() + th);
            let mask = v_mask * col_gauss[x];

            let mut r = ((nr * inv_den.0 + core).max(0.0).powf(1.5) * mask).clamp(0.0, 1.0);
            let mut g = ((ng * inv_den.1 + core).max(0.0).powf(1.5) * mask).clamp(0.0, 1.0);
            let mut b = ((nb * inv_den.2 + core).max(0.0).powf(1.5) * mask).clamp(0.0, 1.0);

            if e < 0.999 {
                let luma = (0.299 * r + 0.587 * g + 0.114 * b) * dim_mul;
                let gr = (luma * 0.96).clamp(0.0, 1.0);
                let gg = (luma * 0.98).clamp(0.0, 1.0);
                let gb = (luma * 1.04).clamp(0.0, 1.0);
                r = gr + (r - gr) * e;
                g = gg + (g - gg) * e;
                b = gb + (b - gb) * e;
            }

            // Premultiplied alpha based on max channel luminance so it composites cleanly on #000
            let idx = row_off + x * 4;
            bgra[idx] = (b * 255.0) as u8;
            bgra[idx + 1] = (g * 255.0) as u8;
            bgra[idx + 2] = (r * 255.0) as u8;
            bgra[idx + 3] = 255;
        }
    }
}

// Win32 Clipboard UTF-16 helpers (for Paste API Key, Include Clipboard, Copy Answer)

const CF_UNICODETEXT: u32 = 13;

pub fn read_clipboard_text() -> Option<String> {
    unsafe {
        if OpenClipboard(HWND(std::ptr::null_mut())).is_err() {
            return None;
        }
        let res = (|| {
            let h = GetClipboardData(CF_UNICODETEXT).ok()?;
            let ptr = GlobalLock(windows::Win32::Foundation::HGLOBAL(h.0)) as *const u16;
            if ptr.is_null() {
                return None;
            }
            let mut len = 0usize;
            while len < 65_536 && *ptr.add(len) != 0 {
                len += 1;
            }
            let slice = std::slice::from_raw_parts(ptr, len);
            let text = String::from_utf16_lossy(slice);
            let _ = GlobalUnlock(windows::Win32::Foundation::HGLOBAL(h.0));
            Some(text)
        })();
        let _ = CloseClipboard();
        res
    }
}

pub fn write_clipboard_text(text: &str) -> bool {
    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let bytes = wide.len() * 2;
    unsafe {
        if OpenClipboard(HWND(std::ptr::null_mut())).is_err() {
            return false;
        }
        let ok = (|| {
            let _ = EmptyClipboard();
            let hmem = GlobalAlloc(GMEM_MOVEABLE, bytes).ok()?;
            let ptr = GlobalLock(hmem) as *mut u16;
            if ptr.is_null() {
                return None;
            }
            std::ptr::copy_nonoverlapping(wide.as_ptr(), ptr, wide.len());
            let _ = GlobalUnlock(hmem);
            SetClipboardData(
                CF_UNICODETEXT,
                windows::Win32::Foundation::HANDLE(hmem.0),
            )
            .ok()?;
            Some(())
        })()
        .is_some();
        let _ = CloseClipboard();
        ok
    }
}

pub const MAX_AI_TURNS: usize = 10;

pub fn ai_history_path() -> std::path::PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    base.join("eave").join("ai_history.json")
}

pub fn serialize_ai_turns(turns: &[(String, String)]) -> String {
    let start = turns.len().saturating_sub(MAX_AI_TURNS);
    let mut out = String::from("[");
    for (i, (q, a)) in turns[start..].iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str("{\"q\":\"");
        out.push_str(&json_escape(q));
        out.push_str("\",\"a\":\"");
        out.push_str(&json_escape(a));
        out.push_str("\"}");
    }
    out.push(']');
    out
}

pub fn parse_ai_turns(json: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let bytes = json.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        let Some(rel_q) = json[i..].find("\"q\"") else {
            break;
        };
        let mut p = i + rel_q + 3;
        while p < bytes.len() && (bytes[p].is_ascii_whitespace() || bytes[p] == b':') {
            p += 1;
        }
        let Some((q, after_q)) = decode_json_string(json, p) else {
            i = p.saturating_add(1);
            continue;
        };
        let Some(rel_a) = json[after_q..].find("\"a\"") else {
            break;
        };
        let mut pa = after_q + rel_a + 3;
        while pa < bytes.len() && (bytes[pa].is_ascii_whitespace() || bytes[pa] == b':') {
            pa += 1;
        }
        let Some((a, after_a)) = decode_json_string(json, pa) else {
            i = pa.saturating_add(1);
            continue;
        };
        out.push((q, a));
        i = after_a;
    }
    if out.len() > MAX_AI_TURNS {
        let drop_n = out.len() - MAX_AI_TURNS;
        out.drain(0..drop_n);
    }
    out
}

pub fn load_ai_turns() -> Vec<(String, String)> {
    let path = ai_history_path();
    match std::fs::read_to_string(&path) {
        Ok(contents) => parse_ai_turns(&contents),
        Err(_) => Vec::new(),
    }
}

pub fn save_ai_turns(turns: &[(String, String)]) {
    let path = ai_history_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&path, serialize_ai_turns(turns));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ai_turns_json_roundtrip_and_cap() {
        let mut sample = Vec::new();
        for idx in 0..12 {
            sample.push((
                format!("Domanda \"{idx}\" con \\ slash"),
                format!("Risposta {idx}\nRiga 2 🚀"),
            ));
        }
        let encoded = serialize_ai_turns(&sample);
        let decoded = parse_ai_turns(&encoded);
        assert_eq!(decoded.len(), MAX_AI_TURNS);
        assert_eq!(decoded[0], sample[2]);
        assert_eq!(decoded[9], sample[11]);
    }

    #[test]
    fn json_escape_handles_quotes_newlines_and_backslashes() {
        assert_eq!(json_escape("a\"b\\c\nd"), "a\\\"b\\\\c\\nd");
    }

    #[test]
    fn sse_extractor_parses_text_and_skips_thought_parts() {
        let sample = r#"{"candidates":[{"content":{"parts":[{"thought":true,"text":"internal plan"},{"text":"Ciao \u00e8 pronto!\nLinea 2"}]}}]}"#;
        assert_eq!(extract_gemini_sse_text(sample), "Ciao è pronto!\nLinea 2");
    }

    #[test]
    fn siri_wave_renders_bright_center_and_dark_edges() {
        let mut buf = vec![0u8; (AI_WAVE_W * AI_WAVE_H * 4) as usize];
        render_siri_wave_frame(&mut buf, AI_WAVE_W, AI_WAVE_H, 1.35, 1.0);
        assert!(buf[0] < 10 && buf[1] < 10 && buf[2] < 10);
        let mut max_luma = 0u8;
        for px in buf.chunks_exact(4) {
            max_luma = max_luma.max(px[0]).max(px[1]).max(px[2]);
        }
        assert!(max_luma > 120, "expected bright wave pixels, got {max_luma}");
    }
}
