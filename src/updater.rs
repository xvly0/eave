use std::ffi::OsStr;
use std::fs;
use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;

use windows::core::{HSTRING, PCWSTR};
use windows::Foundation::Uri;
use windows::Storage::Streams::DataReader;
use windows::Web::Http::HttpClient;
use windows::Win32::Foundation::{HWND, LPARAM, SYSTEMTIME, WPARAM};
use windows::Win32::System::WinRT::{RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, SW_SHOWNORMAL, WM_NULL};

pub const APP_NAME: &str = "Eave";
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const DEFAULT_GITHUB_REPO: &str = "xvly0/eave";

extern "system" {
    fn GetLocalTime(lp_system_time: *mut SYSTEMTIME);
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum UpdateStatus {
    #[default]
    Idle,
    Checking,
    UpToDate {
        checked_at: String,
    },
    NoReleases {
        checked_at: String,
    },
    Available {
        version: String,
        url: String,
        download_url: Option<String>,
    },
    Downloading {
        version: String,
    },
    Restarting,
    Error(String),
}

pub struct UpdateChecker {
    status: Arc<Mutex<UpdateStatus>>,
    notify_hwnd: isize,
}

impl UpdateChecker {
    pub fn new(hwnd: HWND) -> Self {
        Self {
            status: Arc::new(Mutex::new(UpdateStatus::Idle)),
            notify_hwnd: hwnd.0 as isize,
        }
    }

    pub fn status(&self) -> UpdateStatus {
        self.status
            .lock()
            .map(|g| g.clone())
            .unwrap_or(UpdateStatus::Idle)
    }

    pub fn check_async(&self, repo: &str) {
        {
            let Ok(mut guard) = self.status.lock() else {
                return;
            };
            if matches!(
                *guard,
                UpdateStatus::Checking | UpdateStatus::Downloading { .. } | UpdateStatus::Restarting
            ) {
                return;
            }
            *guard = UpdateStatus::Checking;
        }

        let repo_clean = sanitize_repo(repo);
        let status_arc = Arc::clone(&self.status);
        let wake_hwnd = self.notify_hwnd;
        thread::spawn(move || {
            let ro_ok = unsafe { RoInitialize(RO_INIT_MULTITHREADED).is_ok() };
            let result = fetch_latest_release(&repo_clean);
            if ro_ok {
                unsafe { RoUninitialize() };
            }
            if let Ok(mut guard) = status_arc.lock() {
                *guard = result;
            }
            if wake_hwnd != 0 {
                unsafe {
                    let _ = PostMessageW(HWND(wake_hwnd as *mut _), WM_NULL, WPARAM(0), LPARAM(0));
                }
            }
        });
    }

    pub fn install_update_async(&self, version: String, download_url: String) {
        {
            let Ok(mut guard) = self.status.lock() else {
                return;
            };
            if matches!(
                *guard,
                UpdateStatus::Downloading { .. } | UpdateStatus::Restarting
            ) {
                return;
            }
            *guard = UpdateStatus::Downloading {
                version: version.clone(),
            };
        }

        let status_arc = Arc::clone(&self.status);
        let wake_hwnd = self.notify_hwnd;
        thread::spawn(move || {
            let ro_ok = unsafe { RoInitialize(RO_INIT_MULTITHREADED).is_ok() };
            let res = download_and_replace_self(&download_url);
            if ro_ok {
                unsafe { RoUninitialize() };
            }
            match res {
                Ok(()) => {
                    if let Ok(mut guard) = status_arc.lock() {
                        *guard = UpdateStatus::Restarting;
                    }
                    std::process::exit(0);
                }
                Err(msg) => {
                    if let Ok(mut guard) = status_arc.lock() {
                        *guard = UpdateStatus::Error(msg);
                    }
                    if wake_hwnd != 0 {
                        unsafe {
                            let _ =
                                PostMessageW(HWND(wake_hwnd as *mut _), WM_NULL, WPARAM(0), LPARAM(0));
                        }
                    }
                }
            }
        });
    }
}

/// Removes any leftover `eave.exe.old` renamed during a previous self-update.
/// Returns `true` if a backup existed (meaning Eave just restarted from an update).
pub fn cleanup_old_binary() -> bool {
    if let Ok(exe) = std::env::current_exe() {
        let old = old_backup_path(&exe);
        if old.exists() {
            for _ in 0..5 {
                if fs::remove_file(&old).is_ok() || !old.exists() {
                    return true;
                }
                thread::sleep(std::time::Duration::from_millis(40));
            }
            return true;
        }
    }
    false
}

fn local_exe_size() -> Option<u64> {
    let exe = std::env::current_exe().ok()?;
    fs::metadata(exe).ok().map(|m| m.len())
}

fn old_backup_path(exe: &std::path::Path) -> PathBuf {
    let mut s = exe.as_os_str().to_os_string();
    s.push(".old");
    PathBuf::from(s)
}

fn new_staging_path(exe: &std::path::Path) -> PathBuf {
    let mut s = exe.as_os_str().to_os_string();
    s.push(".new");
    PathBuf::from(s)
}

fn download_and_replace_self(download_url: &str) -> Result<(), String> {
    let exe_path = std::env::current_exe().map_err(|_| "Cannot locate current executable".to_string())?;
    let bytes = http_download_bytes(download_url)?;
    if bytes.len() < 100_000 || &bytes[..2] != b"MZ" {
        return Err("Downloaded file is not a valid Windows executable".to_string());
    }

    let new_path = new_staging_path(&exe_path);
    let old_path = old_backup_path(&exe_path);
    fs::write(&new_path, &bytes).map_err(|_| "Cannot write update file next to eave.exe".to_string())?;

    if old_path.exists() {
        let _ = fs::remove_file(&old_path);
    }
    // Windows allows renaming a running executable even though it cannot be unlinked in place.
    if let Err(_) = fs::rename(&exe_path, &old_path) {
        let _ = fs::remove_file(&new_path);
        return Err("Cannot replace running executable (check folder permissions)".to_string());
    }
    if let Err(_) = fs::rename(&new_path, &exe_path) {
        let _ = fs::rename(&old_path, &exe_path);
        let _ = fs::remove_file(&new_path);
        return Err("Failed to finalize update binary".to_string());
    }

    Command::new(&exe_path)
        .spawn()
        .map_err(|_| "Updated binary saved, please relaunch Eave".to_string())?;
    Ok(())
}

fn http_download_bytes(url: &str) -> Result<Vec<u8>, String> {
    let client = HttpClient::new().map_err(|_| "HTTP client unavailable".to_string())?;
    if let Ok(headers) = client.DefaultRequestHeaders() {
        let _ = headers.Append(
            &HSTRING::from("User-Agent"),
            &HSTRING::from(format!("{APP_NAME}/{CURRENT_VERSION}")),
        );
    }
    let uri = Uri::CreateUri(&HSTRING::from(url)).map_err(|_| "Invalid download URL".to_string())?;
    let resp = client
        .GetAsync(&uri)
        .map_err(|_| "Download request failed".to_string())?
        .get()
        .map_err(|_| "Could not download update from GitHub".to_string())?;
    let code = resp.StatusCode().map(|s| s.0).unwrap_or(0);
    if !(200..300).contains(&code) {
        return Err(format!("Download failed (HTTP {code})"));
    }
    let buf = resp
        .Content()
        .map_err(|_| "Empty response".to_string())?
        .ReadAsBufferAsync()
        .map_err(|_| "Read failed".to_string())?
        .get()
        .map_err(|_| "Failed reading download stream".to_string())?;
    let len = buf.Length().map_err(|_| "Invalid buffer length".to_string())? as usize;
    let reader = DataReader::FromBuffer(&buf).map_err(|_| "Buffer reader failed".to_string())?;
    let mut out = vec![0u8; len];
    reader
        .ReadBytes(&mut out)
        .map_err(|_| "Failed copying downloaded bytes".to_string())?;
    Ok(out)
}

pub fn sanitize_repo(repo: &str) -> String {
    let trimmed = repo
        .trim()
        .trim_start_matches("https://github.com/")
        .trim_start_matches("http://github.com/")
        .trim_matches('/');
    if trimmed.contains('/') && !trimmed.contains(' ') {
        trimmed.to_string()
    } else {
        DEFAULT_GITHUB_REPO.to_string()
    }
}

pub fn github_repo_url(repo: &str) -> String {
    format!("https://github.com/{}", sanitize_repo(repo))
}

pub fn open_external_url(url: &str) {
    let verb: Vec<u16> = OsStr::new("open").encode_wide().chain(Some(0)).collect();
    let target: Vec<u16> = OsStr::new(url).encode_wide().chain(Some(0)).collect();
    unsafe {
        ShellExecuteW(
            None,
            PCWSTR(verb.as_ptr()),
            PCWSTR(target.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
    }
}

fn local_hhmm() -> String {
    let mut st = SYSTEMTIME::default();
    unsafe { GetLocalTime(&mut st) };
    format!("{:02}:{:02}", st.wHour, st.wMinute)
}

fn fetch_latest_release(repo: &str) -> UpdateStatus {
    let url = format!("https://api.github.com/repos/{repo}/releases/latest");
    let Ok(client) = HttpClient::new() else {
        return UpdateStatus::Error("HTTP client unavailable".to_string());
    };
    if let Ok(headers) = client.DefaultRequestHeaders() {
        let _ = headers.Append(
            &HSTRING::from("User-Agent"),
            &HSTRING::from(format!("{APP_NAME}/{CURRENT_VERSION}")),
        );
        let _ = headers.Append(
            &HSTRING::from("Accept"),
            &HSTRING::from("application/vnd.github+json"),
        );
    }
    let Ok(uri) = Uri::CreateUri(&HSTRING::from(&url)) else {
        return UpdateStatus::Error("Invalid repository URL".to_string());
    };
    let Ok(op) = client.GetAsync(&uri) else {
        return UpdateStatus::Error("Could not reach GitHub".to_string());
    };
    let Ok(resp) = op.get() else {
        return UpdateStatus::Error("Could not reach GitHub".to_string());
    };
    let status_code = resp.StatusCode().map(|s| s.0).unwrap_or(0);
    if status_code == 404 {
        return UpdateStatus::NoReleases {
            checked_at: local_hhmm(),
        };
    }
    if !(200..300).contains(&status_code) {
        return UpdateStatus::Error(format!("GitHub returned HTTP {status_code}"));
    }
    let body = resp
        .Content()
        .ok()
        .and_then(|c| c.ReadAsStringAsync().ok())
        .and_then(|op| op.get().ok())
        .map(|h| h.to_string())
        .unwrap_or_default();

    let Some(tag) = json_extract_string(&body, "tag_name") else {
        return UpdateStatus::Error("Invalid release response".to_string());
    };
    let html_url = json_extract_string(&body, "html_url")
        .unwrap_or_else(|| format!("https://github.com/{repo}/releases/latest"));
    let (download_url, remote_size) = match json_extract_exe_asset(&body) {
        Some((u, sz)) => (Some(u), sz),
        None => (None, None),
    };

    let newer_semver = is_newer_version(&tag, CURRENT_VERSION);
    let same_ver_rebuild = !newer_semver
        && parse_semver(&tag).is_some()
        && parse_semver(&tag) == parse_semver(CURRENT_VERSION)
        && download_url.is_some()
        && remote_size
            .zip(local_exe_size())
            .is_some_and(|(rs, ls)| rs > 100_000 && rs != ls);

    if newer_semver || same_ver_rebuild {
        UpdateStatus::Available {
            version: tag.trim_start_matches('v').to_string(),
            url: html_url,
            download_url,
        }
    } else {
        UpdateStatus::UpToDate {
            checked_at: local_hhmm(),
        }
    }
}

pub fn parse_semver(raw: &str) -> Option<(u32, u32, u32)> {
    let cleaned = raw
        .trim()
        .trim_start_matches(['v', 'V'])
        .split(['-', '+'])
        .next()?;
    let mut parts = cleaned.split('.');
    let major: u32 = parts.next()?.parse().ok()?;
    let minor: u32 = parts.next().unwrap_or("0").parse().ok()?;
    let patch: u32 = parts.next().unwrap_or("0").parse().ok()?;
    Some((major, minor, patch))
}

pub fn is_newer_version(remote: &str, local: &str) -> bool {
    match (parse_semver(remote), parse_semver(local)) {
        (Some(r), Some(l)) => r > l,
        _ => false,
    }
}

fn json_extract_exe_asset(json: &str) -> Option<(String, Option<u64>)> {
    let assets_slice = if let Some(pos) = json.find("\"assets\"") {
        let after = &json[pos + "\"assets\"".len()..];
        let bracket = after.find('[')?;
        &after[bracket + 1..]
    } else {
        json
    };

    let bytes = assets_slice.as_bytes();
    let mut i = 0usize;
    let mut in_str = false;
    let mut esc = false;
    let mut depth = 0i32;
    let mut obj_start: Option<usize> = None;

    while i < bytes.len() {
        let b = bytes[i];
        if in_str {
            if esc {
                esc = false;
            } else if b == b'\\' {
                esc = true;
            } else if b == b'"' {
                in_str = false;
            }
        } else {
            match b {
                b'"' => in_str = true,
                b'{' => {
                    if depth == 0 {
                        obj_start = Some(i);
                    }
                    depth += 1;
                }
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        if let Some(start) = obj_start.take() {
                            let obj = &assets_slice[start..=i];
                            if let Some(url) = json_extract_string(obj, "browser_download_url") {
                                if url.to_ascii_lowercase().ends_with(".exe") {
                                    let size = json_extract_u64(obj, "size");
                                    return Some((url, size));
                                }
                            }
                        }
                    }
                }
                b']' if depth == 0 => break,
                _ => {}
            }
        }
        i += 1;
    }
    None
}

fn json_extract_u64(json: &str, key: &str) -> Option<u64> {
    let needle = format!("\"{key}\"");
    let start = json.find(&needle)? + needle.len();
    let rest = json[start..].trim_start().strip_prefix(':')?.trim_start();
    let end = rest
        .bytes()
        .take_while(|b| b.is_ascii_digit())
        .count();
    if end == 0 {
        return None;
    }
    rest[..end].parse().ok()
}

fn json_extract_string(json: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let start = json.find(&needle)? + needle.len();
    extract_json_colon_string(&json[start..])
}

fn extract_json_colon_string(slice: &str) -> Option<String> {
    let rest = slice.trim_start().strip_prefix(':')?.trim_start();
    let rest = rest.strip_prefix('"')?;
    let mut out = String::new();
    let mut chars = rest.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(out),
            '\\' => {
                if let Some(esc) = chars.next() {
                    match esc {
                        '"' | '\\' | '/' => out.push(esc),
                        'n' => out.push('\n'),
                        'r' => out.push('\r'),
                        't' => out.push('\t'),
                        other => out.push(other),
                    }
                }
            }
            other => out.push(other),
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semver_comparison_handles_v_prefix_and_prerelease() {
        assert!(is_newer_version("v0.2.0", "0.1.0"));
        assert!(is_newer_version("1.0.0", "0.9.9"));
        assert!(!is_newer_version("v0.1.0", "0.1.0"));
        assert!(!is_newer_version("v0.0.9", "0.1.0"));
        assert!(is_newer_version("v0.1.1-rc1", "0.1.0"));
    }

    #[test]
    fn json_extractor_parses_github_release_fields() {
        let sample = r#"{"tag_name": "v0.3.2", "html_url": "https://github.com/acme/eave/releases/tag/v0.3.2", "assets": [{"name": "notes.txt", "size": 512, "browser_download_url": "https://github.com/acme/eave/releases/download/v0.3.2/notes.txt"}, {"name": "eave.exe", "size": 2456789, "browser_download_url": "https://github.com/acme/eave/releases/download/v0.3.2/eave.exe"}]}"#;
        assert_eq!(json_extract_string(sample, "tag_name").as_deref(), Some("v0.3.2"));
        assert_eq!(
            json_extract_string(sample, "html_url").as_deref(),
            Some("https://github.com/acme/eave/releases/tag/v0.3.2")
        );
        assert_eq!(
            json_extract_exe_asset(sample),
            Some((
                "https://github.com/acme/eave/releases/download/v0.3.2/eave.exe".to_string(),
                Some(2_456_789),
            ))
        );
    }

    #[test]
    fn sanitize_repo_accepts_short_and_full_urls() {
        assert_eq!(sanitize_repo("https://github.com/user/eave/"), "user/eave");
        assert_eq!(sanitize_repo("user/eave"), "user/eave");
        assert_eq!(sanitize_repo("invalid"), DEFAULT_GITHUB_REPO);
    }
}
