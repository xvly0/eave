use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use windows::core::{h, HSTRING};
use windows::Foundation::Uri;
use windows::Web::Http::HttpClient;
use windows::Win32::System::WinRT::{RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED};

const RETRY_COOLDOWN: Duration = Duration::from_secs(60);
const MAX_CACHE_ENTRIES: usize = 64;

pub const INSTRUMENTAL_MARKER: &str = "♪";

pub fn is_instrumental_text(text: &str) -> bool {
    let trimmed = text.trim();
    !trimmed.is_empty()
        && trimmed
            .chars()
            .all(|ch| ch.is_whitespace() || matches!(ch, '♪' | '♫' | '♩' | '♬' | '🎵' | '🎶'))
}

#[derive(Clone, Debug, PartialEq)]
pub struct LyricLine {
    pub time_secs: f64,
    pub text: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LyricLinePair {
    pub prev_prev: String,
    pub prev: String,
    pub current: String,
    pub next: String,
    pub next_after: String,
    pub next_next: Option<LyricLine>,
    pub active_index: usize,
    pub prev_time_secs: Option<f64>,
    pub current_time_secs: Option<f64>,
    pub next_time_secs: Option<f64>,
    pub has_synced: bool,
    pub pre_start: bool,
}

enum CacheEntry {
    Pending,
    Ready(Vec<LyricLine>),
    FailedUntil(Instant),
}

enum FetchMsg {
    Fetch {
        track_key: String,
        title: String,
        artist: String,
    },
    Shutdown,
}

struct ComGuard {
    initialized: bool,
}

impl ComGuard {
    fn init_mta() -> Self {
        // Safety: initializes WinRT MTA on this dedicated worker thread; paired with RoUninitialize in Drop.
        let initialized = unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.is_ok();
        Self { initialized }
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.initialized {
            // Safety: matches successful RoInitialize on the same thread.
            unsafe { RoUninitialize() };
        }
    }
}

pub struct LyricsService {
    cache: Arc<Mutex<HashMap<String, CacheEntry>>>,
    tx: Sender<FetchMsg>,
    worker: Option<JoinHandle<()>>,
}

impl LyricsService {
    pub fn start() -> Self {
        let cache = Arc::new(Mutex::new(HashMap::new()));
        let (tx, rx) = mpsc::channel::<FetchMsg>();

        let cache_worker = Arc::clone(&cache);
        let worker = thread::Builder::new()
            .name("eave-lyrics".to_string())
            .spawn(move || run_worker(cache_worker, rx))
            .ok();

        Self { cache, tx, worker }
    }

    pub fn get_lines(
        &self,
        track_key: &str,
        title: &str,
        artist: &str,
        position_secs: f64,
    ) -> LyricLinePair {
        let clean_title = title.trim();
        if track_key.is_empty() || clean_title.is_empty() {
            return LyricLinePair::default();
        }

        let now = Instant::now();
        let should_fetch = {
            let Ok(mut guard) = self.cache.lock() else {
                return LyricLinePair::default();
            };

            match guard.get(track_key) {
                Some(CacheEntry::Ready(lines)) => {
                    return lookup_lyric_lines(lines, position_secs);
                }
                Some(CacheEntry::Pending) => false,
                Some(CacheEntry::FailedUntil(until)) if now < *until => false,
                _ => {
                    if guard.len() >= MAX_CACHE_ENTRIES {
                        guard.clear();
                    }
                    guard.insert(track_key.to_string(), CacheEntry::Pending);
                    true
                }
            }
        };

        if should_fetch {
            let _ = self.tx.send(FetchMsg::Fetch {
                track_key: track_key.to_string(),
                title: clean_title.to_string(),
                artist: artist.trim().to_string(),
            });
        }

        LyricLinePair::default()
    }

    pub fn shutdown(&mut self) {
        let _ = self.tx.send(FetchMsg::Shutdown);
        if let Some(handle) = self.worker.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for LyricsService {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn run_worker(cache: Arc<Mutex<HashMap<String, CacheEntry>>>, rx: Receiver<FetchMsg>) {
    let _com = ComGuard::init_mta();
    if !_com.initialized {
        return;
    }

    let Ok(client) = HttpClient::new() else {
        return;
    };
    if let Ok(headers) = client.DefaultRequestHeaders() {
        if let Ok(ua) = headers.UserAgent() {
            let _ = ua.TryParseAdd(h!("Eave/0.1"));
        }
    }

    while let Ok(first_msg) = rx.recv() {
        let mut latest = match first_msg {
            FetchMsg::Shutdown => break,
            FetchMsg::Fetch {
                track_key,
                title,
                artist,
            } => (track_key, title, artist),
        };

        let mut should_shutdown = false;
        while let Ok(next_msg) = rx.try_recv() {
            match next_msg {
                FetchMsg::Shutdown => {
                    should_shutdown = true;
                    break;
                }
                FetchMsg::Fetch {
                    track_key,
                    title,
                    artist,
                } => {
                    if let Ok(mut guard) = cache.lock() {
                        if matches!(guard.get(&latest.0), Some(CacheEntry::Pending)) {
                            guard.remove(&latest.0);
                        }
                    }
                    latest = (track_key, title, artist);
                }
            }
        }
        if should_shutdown {
            break;
        }

        let (track_key, title, artist) = latest;
        let result = fetch_lyrics_from_lrclib(&client, &title, &artist);

        if let Ok(mut guard) = cache.lock() {
            match result {
                Ok(lines) => {
                    guard.insert(track_key, CacheEntry::Ready(lines));
                }
                Err(()) => {
                    guard.insert(
                        track_key,
                        CacheEntry::FailedUntil(Instant::now() + RETRY_COOLDOWN),
                    );
                }
            }
        }
    }

    let _ = client.Close();
}

fn fetch_lyrics_from_lrclib(
    client: &HttpClient,
    title: &str,
    artist: &str,
) -> Result<Vec<LyricLine>, ()> {
    let url = build_lrclib_search_url(title, artist);
    let uri = Uri::CreateUri(&HSTRING::from(url)).map_err(|_| ())?;
    let op = client.GetStringAsync(&uri).map_err(|_| ())?;
    let body = op.get().map_err(|_| ())?.to_string_lossy();

    if let Some(lrc_raw) = extract_synced_lyrics_from_json(&body) {
        return Ok(parse_lrc(&lrc_raw));
    }
    if let Some(plain_raw) = extract_plain_lyrics_from_json(&body) {
        return Ok(parse_plain_lyrics(&plain_raw));
    }
    Ok(Vec::new())
}

pub fn build_lrclib_search_url(title: &str, artist: &str) -> String {
    let mut url = String::from("https://lrclib.net/api/search?track_name=");
    url.push_str(&percent_encode(title));
    if !artist.trim().is_empty() {
        url.push_str("&artist_name=");
        url.push_str(&percent_encode(artist));
    }
    url
}

pub fn percent_encode(input: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(input.len() + 8);
    for &b in input.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push('%');
            out.push(HEX[(b >> 4) as usize] as char);
            out.push(HEX[(b & 0x0F) as usize] as char);
        }
    }
    out
}

fn normalized_display_lines(lines: &[LyricLine]) -> Vec<LyricLine> {
    let Some(first_content_idx) = lines.iter().position(|l| !l.text.trim().is_empty()) else {
        return Vec::new();
    };
    let slice = &lines[first_content_idx..];
    let mut out: Vec<LyricLine> = Vec::with_capacity(slice.len());
    for (i, line) in slice.iter().enumerate() {
        let is_blank_or_note = line.text.trim().is_empty() || is_instrumental_text(&line.text);
        if is_blank_or_note {
            let has_simultaneous_lyric = [i.checked_sub(1), Some(i + 1)]
                .into_iter()
                .flatten()
                .filter_map(|idx| slice.get(idx))
                .any(|adj| {
                    !adj.text.trim().is_empty()
                        && !is_instrumental_text(&adj.text)
                        && (adj.time_secs - line.time_secs).abs() <= 0.05
                });
            if has_simultaneous_lyric {
                continue;
            }
            if out.last().is_some_and(|p| p.text == INSTRUMENTAL_MARKER) {
                continue;
            }
            out.push(LyricLine {
                time_secs: line.time_secs,
                text: INSTRUMENTAL_MARKER.to_string(),
            });
        } else {
            out.push(line.clone());
        }
    }
    out
}

pub fn lookup_lyric_lines(lines: &[LyricLine], position_secs: f64) -> LyricLinePair {
    let display = normalized_display_lines(lines);
    if display.is_empty() {
        return LyricLinePair::default();
    }

    let pos = if position_secs.is_finite() {
        position_secs.max(0.0)
    } else {
        0.0
    };

    let idx = display.partition_point(|line| line.time_secs <= pos);
    if idx == 0 {
        let first = display.first();
        let second = display.get(1).cloned();
        let (next, next_time_secs) = first
            .map(|l| (l.text.clone(), Some(l.time_secs)))
            .unwrap_or((String::new(), None));
        let next_after = second.as_ref().map(|l| l.text.clone()).unwrap_or_default();
        return LyricLinePair {
            prev_prev: String::new(),
            prev: String::new(),
            current: String::new(),
            next,
            next_after,
            next_next: second,
            active_index: 0,
            prev_time_secs: None,
            current_time_secs: None,
            next_time_secs,
            has_synced: true,
            pre_start: true,
        };
    }

    let curr_idx = idx - 1;
    let curr_line = &display[curr_idx];
    let current = curr_line.text.clone();
    let current_time_secs = Some(curr_line.time_secs);
    let active_index = curr_idx + 1;

    let (prev, prev_time_secs) = curr_idx
        .checked_sub(1)
        .and_then(|i| display.get(i))
        .map(|l| (l.text.clone(), Some(l.time_secs)))
        .unwrap_or((String::new(), None));
    let prev_prev = curr_idx
        .checked_sub(2)
        .and_then(|i| display.get(i))
        .map(|l| l.text.clone())
        .unwrap_or_default();

    let (next, next_time_secs) = display
        .get(curr_idx + 1)
        .map(|l| (l.text.clone(), Some(l.time_secs)))
        .unwrap_or((String::new(), None));
    let second = display.get(curr_idx + 2).cloned();
    let next_after = second.as_ref().map(|l| l.text.clone()).unwrap_or_default();

    LyricLinePair {
        prev_prev,
        prev,
        current,
        next,
        next_after,
        next_next: second,
        active_index,
        prev_time_secs,
        current_time_secs,
        next_time_secs,
        has_synced: true,
        pre_start: false,
    }
}

pub fn parse_plain_lyrics(plain: &str) -> Vec<LyricLine> {
    plain
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .enumerate()
        .map(|(i, line)| LyricLine {
            time_secs: i as f64 * 3.5,
            text: line.to_string(),
        })
        .collect()
}

pub fn parse_lrc(lrc: &str) -> Vec<LyricLine> {
    let mut offset_secs = 0.0_f64;
    let mut parsed = Vec::new();

    for raw_line in lrc.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }

        let mut rest = line;
        let mut timestamps = Vec::new();

        while rest.starts_with('[') {
            let Some(close_idx) = rest.find(']') else {
                break;
            };
            let tag = &rest[1..close_idx];
            if let Some(ts) = parse_lrc_timestamp(tag) {
                timestamps.push(ts);
                rest = &rest[close_idx + 1..];
            } else {
                if let Some(off_ms) = parse_offset_tag(tag) {
                    offset_secs = off_ms as f64 / 1000.0;
                }
                rest = &rest[close_idx + 1..];
                if timestamps.is_empty() {
                    break;
                }
            }
        }

        if timestamps.is_empty() {
            continue;
        }

        let text = strip_inline_timestamps(rest).trim().to_string();
        for ts in timestamps {
            parsed.push(LyricLine {
                time_secs: (ts - offset_secs).max(0.0),
                text: text.clone(),
            });
        }
    }

    parsed.sort_by(|a, b| a.time_secs.total_cmp(&b.time_secs));
    parsed
}

fn parse_lrc_timestamp(tag: &str) -> Option<f64> {
    let tag = tag.trim();
    let colon_idx = tag.find(':')?;
    let mins_str = &tag[..colon_idx];
    let sec_part = &tag[colon_idx + 1..];

    if mins_str.is_empty() || !mins_str.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let mins: f64 = mins_str.parse().ok()?;

    let (secs_str, frac_secs) = match sec_part.find(['.', ':']) {
        Some(sep) => {
            let s = &sec_part[..sep];
            let frac_raw = &sec_part[sep + 1..];
            if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            if !frac_raw.is_empty() && !frac_raw.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let frac = if frac_raw.is_empty() {
                0.0
            } else {
                let digits = frac_raw.len().min(6) as i32;
                let val: f64 = frac_raw[..digits as usize].parse().ok()?;
                val / 10_f64.powi(digits)
            };
            (s, frac)
        }
        None => {
            if sec_part.is_empty() || !sec_part.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            (sec_part, 0.0)
        }
    };

    let secs: f64 = secs_str.parse().ok()?;
    Some(mins * 60.0 + secs + frac_secs)
}

fn parse_offset_tag(tag: &str) -> Option<i64> {
    let trimmed = tag.trim();
    let rest = trimmed
        .strip_prefix("offset:")
        .or_else(|| trimmed.strip_prefix("OFFSET:"))?;
    rest.trim().parse::<i64>().ok()
}

fn strip_inline_timestamps(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('<') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        if let Some(end) = after.find('>') {
            let candidate = &after[..end];
            if parse_lrc_timestamp(candidate).is_some() {
                rest = &after[end + 1..];
                continue;
            }
        }
        out.push('<');
        rest = after;
    }
    out.push_str(rest);
    out
}

pub fn extract_synced_lyrics_from_json(json: &str) -> Option<String> {
    let candidates = extract_all_json_fields_from_array(json, "syncedLyrics");
    pick_best_synced_lyrics(&candidates)
}

pub fn extract_plain_lyrics_from_json(json: &str) -> Option<String> {
    extract_all_json_fields_from_array(json, "plainLyrics")
        .into_iter()
        .next()
}

fn pick_best_synced_lyrics(candidates: &[String]) -> Option<String> {
    if candidates.is_empty() {
        return None;
    }
    if candidates.len() <= 2 {
        return Some(candidates[0].clone());
    }

    struct Sig {
        t0: f64,
        t1: Option<f64>,
        line_count: usize,
    }
    let sigs: Vec<Option<Sig>> = candidates
        .iter()
        .map(|raw| {
            let parsed = parse_lrc(raw);
            let non_empty: Vec<f64> = parsed
                .iter()
                .filter(|l| !l.text.trim().is_empty())
                .map(|l| l.time_secs)
                .collect();
            let &t0 = non_empty.first()?;
            Some(Sig {
                t0,
                t1: non_empty.get(1).copied(),
                line_count: non_empty.len(),
            })
        })
        .collect();

    let mut best_idx = 0usize;
    let mut best_score = (0usize, 0usize);

    for (i, sig_i_opt) in sigs.iter().enumerate() {
        let Some(sig_i) = sig_i_opt else {
            continue;
        };
        let mut votes = 0usize;
        for sig_j_opt in &sigs {
            let Some(sig_j) = sig_j_opt else {
                continue;
            };
            let t0_close = (sig_i.t0 - sig_j.t0).abs() <= 0.75;
            let t1_close = match (sig_i.t1, sig_j.t1) {
                (Some(a), Some(b)) => (a - b).abs() <= 0.75,
                _ => true,
            };
            if t0_close && t1_close {
                votes += 1;
            }
        }
        let score = (votes, sig_i.line_count);
        if score > best_score {
            best_score = score;
            best_idx = i;
        }
    }

    Some(candidates[best_idx].clone())
}

fn extract_all_json_fields_from_array(json: &str, field_name: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut scanner = JsonScanner::new(json);
    scanner.skip_ws();
    if !scanner.eat_byte(b'[') {
        return out;
    }

    loop {
        scanner.skip_ws();
        if scanner.eat_byte(b']') {
            return out;
        }

        if scanner.peek_byte() == Some(b'{') {
            if let Some(val) = scanner.scan_object_for_field(field_name) {
                if !val.trim().is_empty() {
                    out.push(val);
                }
            }
        } else if scanner.skip_value().is_none() {
            return out;
        }

        scanner.skip_ws();
        if scanner.eat_byte(b',') {
            continue;
        }
        if scanner.eat_byte(b']') {
            return out;
        }
        return out;
    }
}

struct JsonScanner<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> JsonScanner<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            bytes: input.as_bytes(),
            pos: 0,
        }
    }

    fn peek_byte(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn next_byte(&mut self) -> Option<u8> {
        let b = self.peek_byte()?;
        self.pos += 1;
        Some(b)
    }

    fn eat_byte(&mut self, expected: u8) -> bool {
        if self.peek_byte() == Some(expected) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn skip_ws(&mut self) {
        while let Some(b) = self.peek_byte() {
            if matches!(b, b' ' | b'\t' | b'\r' | b'\n') {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn scan_object_for_field(&mut self, field_name: &str) -> Option<String> {
        if !self.eat_byte(b'{') {
            return None;
        }
        let mut found: Option<String> = None;

        loop {
            self.skip_ws();
            if self.eat_byte(b'}') {
                return found;
            }

            let key = self.parse_string()?;
            self.skip_ws();
            if !self.eat_byte(b':') {
                return None;
            }
            self.skip_ws();

            if key == field_name && found.is_none() {
                if self.peek_byte() == Some(b'"') {
                    let val = self.parse_string()?;
                    if !val.trim().is_empty() {
                        found = Some(val);
                    }
                } else {
                    self.skip_value()?;
                }
            } else {
                self.skip_value()?;
            }

            self.skip_ws();
            if self.eat_byte(b',') {
                continue;
            }
            if self.eat_byte(b'}') {
                return found;
            }
            return None;
        }
    }

    fn parse_string(&mut self) -> Option<String> {
        if !self.eat_byte(b'"') {
            return None;
        }
        let mut out = String::new();
        while let Some(b) = self.next_byte() {
            match b {
                b'"' => return Some(out),
                b'\\' => {
                    let esc = self.next_byte()?;
                    match esc {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{0008}'),
                        b'f' => out.push('\u{000C}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let code = self.parse_hex4()?;
                            if (0xD800..=0xDBFF).contains(&code) {
                                // UTF-16 surrogate pair
                                if self.peek_byte() == Some(b'\\')
                                    && self.bytes.get(self.pos + 1) == Some(&b'u')
                                {
                                    self.pos += 2;
                                    let low = self.parse_hex4()?;
                                    if (0xDC00..=0xDFFF).contains(&low) {
                                        let cp = 0x10000
                                            + (((code as u32 - 0xD800) << 10)
                                                | (low as u32 - 0xDC00));
                                        if let Some(ch) = char::from_u32(cp) {
                                            out.push(ch);
                                        }
                                    }
                                }
                            } else if let Some(ch) = char::from_u32(code as u32) {
                                out.push(ch);
                            }
                        }
                        _ => out.push(esc as char),
                    }
                }
                0..=0x7F => out.push(b as char),
                _ => {
                    let start = self.pos - 1;
                    let char_len = if b & 0xE0 == 0xC0 {
                        2
                    } else if b & 0xF0 == 0xE0 {
                        3
                    } else if b & 0xF8 == 0xF0 {
                        4
                    } else {
                        return None;
                    };
                    let end = start + char_len;
                    if end > self.bytes.len() {
                        return None;
                    }
                    let slice = std::str::from_utf8(&self.bytes[start..end]).ok()?;
                    out.push_str(slice);
                    self.pos = end;
                }
            }
        }
        None
    }

    fn parse_hex4(&mut self) -> Option<u16> {
        let mut val: u16 = 0;
        for _ in 0..4 {
            let b = self.next_byte()?;
            let nibble = match b {
                b'0'..=b'9' => b - b'0',
                b'a'..=b'f' => b - b'a' + 10,
                b'A'..=b'F' => b - b'A' + 10,
                _ => return None,
            };
            val = (val << 4) | nibble as u16;
        }
        Some(val)
    }

    fn skip_string(&mut self) -> Option<()> {
        if !self.eat_byte(b'"') {
            return None;
        }
        while let Some(b) = self.next_byte() {
            match b {
                b'"' => return Some(()),
                b'\\' => {
                    let esc = self.next_byte()?;
                    if esc == b'u' {
                        for _ in 0..4 {
                            self.next_byte()?;
                        }
                    }
                }
                _ => {}
            }
        }
        None
    }

    fn skip_value(&mut self) -> Option<()> {
        self.skip_ws();
        match self.peek_byte()? {
            b'"' => self.skip_string(),
            b'{' => {
                self.pos += 1;
                loop {
                    self.skip_ws();
                    if self.eat_byte(b'}') {
                        return Some(());
                    }
                    self.skip_string()?;
                    self.skip_ws();
                    if !self.eat_byte(b':') {
                        return None;
                    }
                    self.skip_value()?;
                    self.skip_ws();
                    if self.eat_byte(b',') {
                        continue;
                    }
                    if self.eat_byte(b'}') {
                        return Some(());
                    }
                    return None;
                }
            }
            b'[' => {
                self.pos += 1;
                loop {
                    self.skip_ws();
                    if self.eat_byte(b']') {
                        return Some(());
                    }
                    self.skip_value()?;
                    self.skip_ws();
                    if self.eat_byte(b',') {
                        continue;
                    }
                    if self.eat_byte(b']') {
                        return Some(());
                    }
                    return None;
                }
            }
            b't' => self.skip_literal(b"true"),
            b'f' => self.skip_literal(b"false"),
            b'n' => self.skip_literal(b"null"),
            b'-' | b'0'..=b'9' => {
                while let Some(b) = self.peek_byte() {
                    if matches!(b, b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E') {
                        self.pos += 1;
                    } else {
                        break;
                    }
                }
                Some(())
            }
            _ => None,
        }
    }

    fn skip_literal(&mut self, lit: &[u8]) -> Option<()> {
        if self.bytes.get(self.pos..self.pos + lit.len()) == Some(lit) {
            self.pos += lit.len();
            Some(())
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_encoding_handles_spaces_reserved_and_utf8() {
        assert_eq!(
            percent_encode("Hello World & You?"),
            "Hello%20World%20%26%20You%3F"
        );
        assert_eq!(percent_encode("A-Z_0.9~"), "A-Z_0.9~");
        assert_eq!(percent_encode("東京"), "%E6%9D%B1%E4%BA%AC");
        assert_eq!(
            build_lrclib_search_url("Blue Bird", "Ikimono Gakari"),
            "https://lrclib.net/api/search?track_name=Blue%20Bird&artist_name=Ikimono%20Gakari"
        );
        assert_eq!(
            build_lrclib_search_url("Track Only", "  "),
            "https://lrclib.net/api/search?track_name=Track%20Only"
        );
    }

    #[test]
    fn json_extractor_picks_first_non_empty_synced_lyrics_and_unescapes() {
        let json = r#"[
            {
                "id": 1,
                "trackName": "Decoy {\"syncedLyrics\":\"[00:00.00]Fake\"}",
                "syncedLyrics": null,
                "plainLyrics": "Only plain"
            },
            {
                "id": 2,
                "trackName": "Real Song",
                "nested": {"arr": [1, true, false, null]},
                "syncedLyrics": "[00:12.34]Line \"one\" \\ slash\n[00:15.00]Emoji \uD83C\uDFB5 \u00e9"
            }
        ]"#;

        let extracted = extract_synced_lyrics_from_json(json).expect("should extract lyrics");
        assert_eq!(
            extracted,
            "[00:12.34]Line \"one\" \\ slash\n[00:15.00]Emoji \u{1F3B5} \u{00E9}"
        );
    }

    #[test]
    fn json_extractor_handles_empty_or_malformed_json() {
        assert_eq!(extract_synced_lyrics_from_json("[]"), None);
        assert_eq!(
            extract_synced_lyrics_from_json(r#"[{"id":1,"syncedLyrics":""}]"#),
            None
        );
        assert_eq!(extract_synced_lyrics_from_json("not json"), None);
        assert_eq!(extract_synced_lyrics_from_json(r#"[{"id":1"#), None);
    }

    #[test]
    fn lrc_parser_handles_timestamps_multi_tags_offset_and_inline_tags() {
        let lrc = "\
[ar:Test Artist]
[ti:Test Song]
[offset:500]
[00:10.50]First line
[00:12.00][01:02.250]Chorus <00:12.40>word
[00:20:50]Colon subseconds
[00:30]Whole seconds
";
        let lines = parse_lrc(lrc);
        assert_eq!(lines.len(), 5);
        assert!((lines[0].time_secs - 10.0).abs() < 1e-6);
        assert_eq!(lines[0].text, "First line");

        assert!((lines[1].time_secs - 11.5).abs() < 1e-6);
        assert_eq!(lines[1].text, "Chorus word");

        assert!((lines[2].time_secs - 20.0).abs() < 1e-6);
        assert_eq!(lines[2].text, "Colon subseconds");

        assert!((lines[3].time_secs - 29.5).abs() < 1e-6);
        assert_eq!(lines[3].text, "Whole seconds");

        assert!((lines[4].time_secs - 61.75).abs() < 1e-6);
        assert_eq!(lines[4].text, "Chorus word");
    }

    #[test]
    fn lookup_returns_current_and_next_for_position() {
        let lines = parse_lrc(
            "[00:05.00]Line A\n[00:10.00]\n[00:15.00]Line B\n[00:20.00]Line C\n",
        );

        let before = lookup_lyric_lines(&lines, 2.0);
        assert_eq!(
            before,
            LyricLinePair {
                prev_prev: String::new(),
                prev: String::new(),
                current: String::new(),
                next: "Line A".to_string(),
                next_after: INSTRUMENTAL_MARKER.to_string(),
                next_next: Some(LyricLine {
                    time_secs: 10.0,
                    text: INSTRUMENTAL_MARKER.to_string(),
                }),
                active_index: 0,
                prev_time_secs: None,
                current_time_secs: None,
                next_time_secs: Some(5.0),
                has_synced: true,
                pre_start: true,
            }
        );

        let at_a = lookup_lyric_lines(&lines, 5.0);
        assert_eq!(
            at_a,
            LyricLinePair {
                prev_prev: String::new(),
                prev: String::new(),
                current: "Line A".to_string(),
                next: INSTRUMENTAL_MARKER.to_string(),
                next_after: "Line B".to_string(),
                next_next: Some(LyricLine {
                    time_secs: 15.0,
                    text: "Line B".to_string(),
                }),
                active_index: 1,
                prev_time_secs: None,
                current_time_secs: Some(5.0),
                next_time_secs: Some(10.0),
                has_synced: true,
                pre_start: false,
            }
        );

        let at_gap = lookup_lyric_lines(&lines, 12.0);
        assert_eq!(
            at_gap,
            LyricLinePair {
                prev_prev: String::new(),
                prev: "Line A".to_string(),
                current: INSTRUMENTAL_MARKER.to_string(),
                next: "Line B".to_string(),
                next_after: "Line C".to_string(),
                next_next: Some(LyricLine {
                    time_secs: 20.0,
                    text: "Line C".to_string(),
                }),
                active_index: 2,
                prev_time_secs: Some(5.0),
                current_time_secs: Some(10.0),
                next_time_secs: Some(15.0),
                has_synced: true,
                pre_start: false,
            }
        );

        let at_b = lookup_lyric_lines(&lines, 16.0);
        assert_eq!(
            at_b,
            LyricLinePair {
                prev_prev: "Line A".to_string(),
                prev: INSTRUMENTAL_MARKER.to_string(),
                current: "Line B".to_string(),
                next: "Line C".to_string(),
                next_after: String::new(),
                next_next: None,
                active_index: 3,
                prev_time_secs: Some(10.0),
                current_time_secs: Some(15.0),
                next_time_secs: Some(20.0),
                has_synced: true,
                pre_start: false,
            }
        );

        let at_end = lookup_lyric_lines(&lines, 25.0);
        assert_eq!(
            at_end,
            LyricLinePair {
                prev_prev: INSTRUMENTAL_MARKER.to_string(),
                prev: "Line B".to_string(),
                current: "Line C".to_string(),
                next: String::new(),
                next_after: String::new(),
                next_next: None,
                active_index: 4,
                prev_time_secs: Some(15.0),
                current_time_secs: Some(20.0),
                next_time_secs: None,
                has_synced: true,
                pre_start: false,
            }
        );

        assert_eq!(lookup_lyric_lines(&[], 5.0), LyricLinePair::default());
    }

    #[test]
    fn empty_lrc_timestamp_renders_instrumental_marker_and_collapses_runs() {
        assert!(is_instrumental_text("♪"));
        assert!(is_instrumental_text(" ♫  ♬ "));
        assert!(is_instrumental_text("🎵"));
        assert!(!is_instrumental_text(""));
        assert!(!is_instrumental_text("   "));
        assert!(!is_instrumental_text("Yeah ♪"));

        let lines = parse_lrc(
            "[00:00.00]\n[00:04.00]Line 1\n[00:08.00]Line 2\n[00:12.00]\n[00:14.00]\n[00:16.00]Line 3\n[00:20.00]Line 4\n",
        );

        let pre_start = lookup_lyric_lines(&lines, 2.0);
        assert!(pre_start.pre_start);
        assert_eq!(pre_start.current, "");
        assert_eq!(pre_start.next, "Line 1");

        let at_line_2 = lookup_lyric_lines(&lines, 9.0);
        assert_eq!(at_line_2.current, "Line 2");
        assert_eq!(at_line_2.next, INSTRUMENTAL_MARKER);
        assert_eq!(at_line_2.next_after, "Line 3");
        assert_eq!(at_line_2.active_index, 2);

        let across_empty_marker = lookup_lyric_lines(&lines, 13.0);
        assert_eq!(across_empty_marker.current, INSTRUMENTAL_MARKER);
        assert_eq!(across_empty_marker.current_time_secs, Some(12.0));
        assert_eq!(across_empty_marker.active_index, 3);
        assert_eq!(across_empty_marker.prev, "Line 2");
        assert_eq!(across_empty_marker.prev_prev, "Line 1");
        assert_eq!(across_empty_marker.next, "Line 3");
        assert_eq!(across_empty_marker.next_after, "Line 4");

        let across_collapsed_second = lookup_lyric_lines(&lines, 15.0);
        assert_eq!(across_collapsed_second, across_empty_marker);

        let at_line_4 = lookup_lyric_lines(&lines, 21.0);
        assert_eq!(at_line_4.prev_prev, INSTRUMENTAL_MARKER);
        assert_eq!(at_line_4.prev, "Line 3");
        assert_eq!(at_line_4.current, "Line 4");
        assert_eq!(at_line_4.active_index, 5);
        assert_eq!(at_line_4.next, "");
        assert_eq!(at_line_4.next_after, "");
    }

    #[test]
    fn plain_lyrics_fallback_extracts_and_synthesizes_timestamps() {
        let json = r#"[
            {
                "id": 10,
                "trackName": "Decoy {\"plainLyrics\":\"nope\"}",
                "syncedLyrics": null,
                "plainLyrics": ""
            },
            {
                "id": 11,
                "syncedLyrics": null,
                "plainLyrics": "First plain line\n\n  Second plain line  \nThird plain line"
            }
        ]"#;

        assert_eq!(extract_synced_lyrics_from_json(json), None);
        let plain = extract_plain_lyrics_from_json(json).expect("should extract plainLyrics");
        let lines = parse_plain_lyrics(&plain);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].text, "First plain line");
        assert!((lines[0].time_secs - 0.0).abs() < 1e-6);
        assert_eq!(lines[1].text, "Second plain line");
        assert!((lines[1].time_secs - 3.5).abs() < 1e-6);
        assert_eq!(lines[2].text, "Third plain line");
        assert!((lines[2].time_secs - 7.0).abs() < 1e-6);

        let pair = lookup_lyric_lines(&lines, 4.0);
        assert_eq!(pair.prev, "First plain line");
        assert_eq!(pair.current, "Second plain line");
        assert_eq!(pair.next, "Third plain line");
        assert_eq!(pair.next_after, "");
        assert_eq!(pair.active_index, 2);
        assert!(pair.has_synced);
    }

    #[test]
    fn json_extractor_prefers_timing_consensus_over_early_outlier() {
        let json = r#"[
            { "id": 1, "syncedLyrics": "[00:02.86] 29 pearls in your kiss\n[00:10.02] A singing smile" },
            { "id": 2, "syncedLyrics": "[00:07.07] 29 pearls in your kiss\n[00:10.19] A singing smile" },
            { "id": 3, "syncedLyrics": "[00:07.10] 29 pearls in your kiss\n[00:10.20] A singing smile" },
            { "id": 4, "syncedLyrics": "[00:07.07] 29 pearls in your kiss\n[00:10.19] A singing smile" }
        ]"#;
        let picked = extract_synced_lyrics_from_json(json).expect("should pick consensus");
        assert!(
            picked.starts_with("[00:07.07]"),
            "expected consensus [00:07.07], got: {picked}"
        );
    }
}
