use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub notch_width: f32,
    pub notch_height: f32,
    pub expanded_width: f32,
    pub expanded_height: f32,
    pub top_offset: f32,
    pub horizontal_offset: f32,
    pub glass_tint: [u8; 4],
    pub open_on_hover: bool,
    pub minimal_mode: bool,
    pub show_mascot: bool,
    pub show_clock: bool,
    pub show_lyrics: bool,
    pub hide_fullscreen: bool,
    pub monitor_index: usize,
    pub page_order: [u8; 6],
    pub page_enabled: [bool; 6],
    pub clock_ring_mode: u8,
    pub widget_left: u8,
    pub widget_right: u8,
    pub widget_city_1: u8,
    pub widget_city_2: u8,
    pub widget_countdown: u8,
    pub widget_weather_city: u8,
    pub osd_volume: bool,
    pub osd_brightness: bool,
    pub osd_caps_lock: bool,
    pub osd_mic_mute: bool,
    pub osd_mic_in_use: bool,
    pub osd_charging: bool,
    pub osd_wifi: bool,
    pub osd_bluetooth: bool,
    pub osd_audio_device: bool,
    pub osd_notifications: bool,
    pub ai_enabled: bool,
    pub ai_api_key: String,
    pub ai_model: String,
    pub ai_hotkey: u8,
    pub auto_check_updates: bool,
    pub update_repo: String,
    pub todo_remind_start: bool,
    pub todo_remind_end: bool,
    pub todo_snooze_mins: u8,
    pub todo_week_starts_monday: bool,
    pub todo_calendar_default_week: bool,
    /// 0 = Auto, 1 = 100%, 2 = 115%, 3 = 130%, 4 = 150%
    pub size_preset: u8,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            notch_width: 166.0,
            notch_height: 30.0,
            expanded_width: 438.0,
            expanded_height: 172.0,
            top_offset: 0.0,
            horizontal_offset: 0.0,
            glass_tint: [0, 0, 0, 255],
            open_on_hover: true,
            minimal_mode: false,
            show_mascot: true,
            show_clock: true,
            show_lyrics: true,
            hide_fullscreen: true,
            monitor_index: 0,
            page_order: [0, 1, 2, 3, 4, 5],
            page_enabled: [true, false, false, true, true, true],
            clock_ring_mode: 0,
            widget_left: 0,
            widget_right: 1,
            widget_city_1: 0,
            widget_city_2: 1,
            widget_countdown: 0,
            widget_weather_city: 0,
            osd_volume: true,
            osd_brightness: true,
            osd_caps_lock: true,
            osd_mic_mute: true,
            osd_mic_in_use: true,
            osd_charging: true,
            osd_wifi: true,
            osd_bluetooth: true,
            osd_audio_device: true,
            osd_notifications: true,
            ai_enabled: true,
            ai_api_key: String::new(),
            ai_model: crate::ai::DEFAULT_AI_MODEL.to_string(),
            ai_hotkey: 2,
            auto_check_updates: true,
            update_repo: crate::updater::DEFAULT_GITHUB_REPO.to_string(),
            todo_remind_start: true,
            todo_remind_end: true,
            todo_snooze_mins: 1,
            todo_week_starts_monday: false,
            todo_calendar_default_week: false,
            size_preset: 0,
        }
    }
}

impl Config {
    pub fn glass_tint_f32(&self) -> [f32; 4] {
        [
            self.glass_tint[0] as f32 / 255.0,
            self.glass_tint[1] as f32 / 255.0,
            self.glass_tint[2] as f32 / 255.0,
            self.glass_tint[3] as f32 / 255.0,
        ]
    }

    pub fn glass_tint_hex(&self) -> String {
        format!(
            "#{:02X}{:02X}{:02X}{:02X}",
            self.glass_tint[0], self.glass_tint[1], self.glass_tint[2], self.glass_tint[3]
        )
    }

    pub fn resolve_path() -> PathBuf {
        if let Ok(exe) = env::current_exe() {
            if let Some(dir) = exe.parent() {
                let portable = dir.join("eave.cfg");
                if portable.exists() {
                    return portable;
                }
            }
        }
        if let Ok(appdata) = env::var("APPDATA") {
            return PathBuf::from(appdata).join("eave").join("eave.cfg");
        }
        PathBuf::from("eave.cfg")
    }

    pub fn is_legacy_auto_config(&self) -> bool {
        (self.glass_tint == [0x0E, 0x10, 0x16, 0xD2] && self.expanded_width == 380.0)
            || (self.glass_tint == [2, 2, 4, 246] && self.expanded_width == 446.0)
            || (self.glass_tint == [0, 0, 0, 255]
                && self.notch_width == 158.0
                && self.expanded_width == 420.0)
    }

    pub fn load_or_create() -> (Self, PathBuf) {
        let path = Self::resolve_path();
        if let Ok(contents) = fs::read_to_string(&path) {
            let parsed = Self::parse_str(&contents);
            if !parsed.is_legacy_auto_config() {
                return (parsed, path);
            }
        }
        let cfg = Self::default();
        let _ = cfg.save_to(&path);
        (cfg, path)
    }

    pub fn load_from(path: &Path) -> Self {
        match fs::read_to_string(path) {
            Ok(contents) => Self::parse_str(&contents),
            Err(_) => Self::default(),
        }
    }

    pub fn save_to(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }
        fs::write(path, self.to_kv_string())
    }

    pub fn to_kv_string(&self) -> String {
        format!(
            "# Eave configuration\n\
             # Edit values and save; restart or use tray Reload/Reset.\n\n\
             notch_width = {}\n\
             notch_height = {}\n\
             expanded_width = {}\n\
             expanded_height = {}\n\
             top_offset = {}\n\
             horizontal_offset = {}\n\
             glass_tint = {}\n\
             open_on_hover = {}\n\
             minimal_mode = {}\n\
             show_mascot = {}\n\
             show_clock = {}\n\
             show_lyrics = {}\n\
             hide_fullscreen = {}\n\
             monitor_index = {}\n\
             page_order = {}\n\
             page_enabled = {}\n\
             clock_ring_mode = {}\n\
             widget_left = {}\n\
             widget_right = {}\n\
             widget_city_1 = {}\n\
             widget_city_2 = {}\n\
             widget_countdown = {}\n\
             widget_weather_city = {}\n\
             osd_volume = {}\n\
             osd_brightness = {}\n\
             osd_caps_lock = {}\n\
             osd_mic_mute = {}\n\
             osd_mic_in_use = {}\n\
             osd_charging = {}\n\
             osd_wifi = {}\n\
             osd_bluetooth = {}\n\
             osd_audio_device = {}\n\
             osd_notifications = {}\n\
             ai_enabled = {}\n\
             ai_api_key = {}\n\
             ai_model = {}\n\
             ai_hotkey = {}\n\
             auto_check_updates = {}\n\
             update_repo = {}\n\
             todo_remind_start = {}\n\
             todo_remind_end = {}\n\
             todo_snooze_mins = {}\n\
             todo_week_starts_monday = {}\n\
             todo_calendar_default_week = {}\n\
             size_preset = {}\n",
            self.notch_width,
            self.notch_height,
            self.expanded_width,
            self.expanded_height,
            self.top_offset,
            self.horizontal_offset,
            self.glass_tint_hex(),
            self.open_on_hover,
            self.minimal_mode,
            self.show_mascot,
            self.show_clock,
            self.show_lyrics,
            self.hide_fullscreen,
            self.monitor_index,
            format_page_order(self.page_order),
            format_page_enabled(self.page_enabled),
            format_clock_ring_mode(self.clock_ring_mode),
            self.widget_left,
            self.widget_right,
            self.widget_city_1,
            self.widget_city_2,
            self.widget_countdown,
            self.widget_weather_city,
            self.osd_volume,
            self.osd_brightness,
            self.osd_caps_lock,
            self.osd_mic_mute,
            self.osd_mic_in_use,
            self.osd_charging,
            self.osd_wifi,
            self.osd_bluetooth,
            self.osd_audio_device,
            self.osd_notifications,
            self.ai_enabled,
            self.ai_api_key.trim(),
            self.ai_model.trim(),
            self.ai_hotkey,
            self.auto_check_updates,
            self.update_repo.trim(),
            self.todo_remind_start,
            self.todo_remind_end,
            self.todo_snooze_mins,
            self.todo_week_starts_monday,
            self.todo_calendar_default_week,
            self.size_preset,
        )
    }

    pub fn page_slot(&self, page_idx: usize) -> usize {
        self.page_order
            .iter()
            .position(|&p| p as usize == page_idx)
            .unwrap_or(page_idx.min(5))
    }

    pub fn first_enabled_page_idx(&self) -> usize {
        self.page_order
            .iter()
            .copied()
            .find(|&p| self.page_enabled.get(p as usize).copied().unwrap_or(false))
            .map(|p| p as usize)
            .unwrap_or(0)
    }

    pub fn next_page_idx(&self, page_idx: usize) -> usize {
        let slot = self.page_slot(page_idx);
        for step in 1..=6 {
            let cand = self.page_order[(slot + step) % 6] as usize;
            if self.page_enabled.get(cand).copied().unwrap_or(true) {
                return cand;
            }
        }
        page_idx.min(5)
    }

    pub fn prev_page_idx(&self, page_idx: usize) -> usize {
        let slot = self.page_slot(page_idx);
        for step in 1..=6 {
            let cand = self.page_order[(slot + 6 - step) % 6] as usize;
            if self.page_enabled.get(cand).copied().unwrap_or(true) {
                return cand;
            }
        }
        page_idx.min(5)
    }

    pub fn parse_str(input: &str) -> Self {
        let mut cfg = Self::default();
        for raw_line in input.lines() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                continue;
            }
            let Some((raw_key, raw_val)) = line.split_once('=') else {
                continue;
            };
            let key = raw_key.trim().to_ascii_lowercase();
            let trimmed_val = raw_val.trim();
            let val = trimmed_val
                .split(" #")
                .next()
                .unwrap_or(trimmed_val)
                .trim();

            match key.as_str() {
                "notch_width" => {
                    if let Ok(v) = val.parse::<f32>() {
                        if v.is_finite() {
                            cfg.notch_width = v.clamp(120.0, 600.0);
                        }
                    }
                }
                "notch_height" => {
                    if let Ok(v) = val.parse::<f32>() {
                        if v.is_finite() {
                            cfg.notch_height = v.clamp(24.0, 120.0);
                        }
                    }
                }
                "expanded_width" => {
                    if let Ok(v) = val.parse::<f32>() {
                        if v.is_finite() {
                            cfg.expanded_width = v.clamp(240.0, 900.0);
                        }
                    }
                }
                "expanded_height" => {
                    if let Ok(v) = val.parse::<f32>() {
                        if v.is_finite() {
                            cfg.expanded_height = v.clamp(96.0, 480.0);
                        }
                    }
                }
                "top_offset" => {
                    if let Ok(v) = val.parse::<f32>() {
                        if v.is_finite() {
                            cfg.top_offset = v.clamp(0.0, 120.0);
                        }
                    }
                }
                "horizontal_offset" => {
                    if let Ok(v) = val.parse::<f32>() {
                        if v.is_finite() {
                            cfg.horizontal_offset = v.clamp(-2000.0, 2000.0);
                        }
                    }
                }
                "glass_tint" => {
                    if let Some(rgba) = parse_hex_rgba(val) {
                        cfg.glass_tint = rgba;
                    }
                }
                "open_on_hover" => {
                    if let Some(b) = parse_bool(val) {
                        cfg.open_on_hover = b;
                    }
                }
                "minimal_mode" => {
                    if let Some(b) = parse_bool(val) {
                        cfg.minimal_mode = b;
                    }
                }
                "show_mascot" => {
                    if let Some(b) = parse_bool(val) {
                        cfg.show_mascot = b;
                    }
                }
                "show_clock" => {
                    if let Some(b) = parse_bool(val) {
                        cfg.show_clock = b;
                    }
                }
                "show_lyrics" => {
                    if let Some(b) = parse_bool(val) {
                        cfg.show_lyrics = b;
                    }
                }
                "hide_fullscreen" => {
                    if let Some(b) = parse_bool(val) {
                        cfg.hide_fullscreen = b;
                    }
                }
                "monitor_index" => {
                    if let Ok(v) = val.parse::<usize>() {
                        cfg.monitor_index = v;
                    }
                }
                "page_order" => {
                    if let Some(order) = parse_page_order(val) {
                        cfg.page_order = order;
                    }
                }
                "page_enabled" => {
                    if let Some(en) = parse_page_enabled(val) {
                        cfg.page_enabled = en;
                    }
                }
                "clock_ring_mode" => {
                    if let Some(m) = parse_clock_ring_mode(val) {
                        cfg.clock_ring_mode = m;
                    }
                }
                "widget_left" => {
                    if let Ok(v) = val.parse::<u8>() {
                        cfg.widget_left = v.min(7);
                    }
                }
                "widget_right" => {
                    if let Ok(v) = val.parse::<u8>() {
                        cfg.widget_right = v.min(7);
                    }
                }
                "widget_city_1" => {
                    if let Ok(v) = val.parse::<u8>() {
                        cfg.widget_city_1 = v.min(15);
                    }
                }
                "widget_city_2" => {
                    if let Ok(v) = val.parse::<u8>() {
                        cfg.widget_city_2 = v.min(15);
                    }
                }
                "widget_countdown" => {
                    if let Ok(v) = val.parse::<u8>() {
                        cfg.widget_countdown = v.min(4);
                    }
                }
                "widget_weather_city" => {
                    if let Ok(v) = val.parse::<u8>() {
                        cfg.widget_weather_city = v.min(16);
                    }
                }
                "osd_volume" => {
                    if let Some(b) = parse_bool(val) {
                        cfg.osd_volume = b;
                    }
                }
                "osd_brightness" => {
                    if let Some(b) = parse_bool(val) {
                        cfg.osd_brightness = b;
                    }
                }
                "osd_caps_lock" => {
                    if let Some(b) = parse_bool(val) {
                        cfg.osd_caps_lock = b;
                    }
                }
                "osd_mic_mute" => {
                    if let Some(b) = parse_bool(val) {
                        cfg.osd_mic_mute = b;
                    }
                }
                "osd_mic_in_use" => {
                    if let Some(b) = parse_bool(val) {
                        cfg.osd_mic_in_use = b;
                    }
                }
                "osd_charging" => {
                    if let Some(b) = parse_bool(val) {
                        cfg.osd_charging = b;
                    }
                }
                "osd_wifi" => {
                    if let Some(b) = parse_bool(val) {
                        cfg.osd_wifi = b;
                    }
                }
                "osd_bluetooth" => {
                    if let Some(b) = parse_bool(val) {
                        cfg.osd_bluetooth = b;
                    }
                }
                "osd_audio_device" => {
                    if let Some(b) = parse_bool(val) {
                        cfg.osd_audio_device = b;
                    }
                }
                "osd_notifications" => {
                    if let Some(b) = parse_bool(val) {
                        cfg.osd_notifications = b;
                    }
                }
                "ai_enabled" => {
                    if let Some(b) = parse_bool(val) {
                        cfg.ai_enabled = b;
                    }
                }
                "ai_api_key" => {
                    cfg.ai_api_key = val.trim().to_string();
                }
                "ai_model" => {
                    let m = val.trim();
                    if !m.is_empty() {
                        if matches!(
                            m,
                            "gemini-2.5-flash"
                                | "gemini-2.5-flash-lite"
                                | "gemini-2.0-flash"
                                | "gemini-4-hydrogen"
                                | "gemini-4-argon"
                                | "gemini-4-carbon"
                        ) {
                            cfg.ai_model = crate::ai::DEFAULT_AI_MODEL.to_string();
                        } else {
                            cfg.ai_model = m.to_string();
                        }
                    }
                }
                "ai_hotkey" => {
                    if let Ok(v) = val.parse::<u8>() {
                        cfg.ai_hotkey = v.min(3);
                    }
                }
                "auto_check_updates" => {
                    if let Some(b) = parse_bool(val) {
                        cfg.auto_check_updates = b;
                    }
                }
                "update_repo" => {
                    let r = val.trim();
                    if !r.is_empty() {
                        cfg.update_repo = crate::updater::sanitize_repo(r);
                    }
                }
                "todo_remind_start" => {
                    if let Some(b) = parse_bool(val) {
                        cfg.todo_remind_start = b;
                    }
                }
                "todo_remind_end" => {
                    if let Some(b) = parse_bool(val) {
                        cfg.todo_remind_end = b;
                    }
                }
                "todo_snooze_mins" => {
                    if let Ok(v) = val.parse::<u8>() {
                        cfg.todo_snooze_mins = v.min(2);
                    }
                }
                "todo_week_starts_monday" => {
                    if let Some(b) = parse_bool(val) {
                        cfg.todo_week_starts_monday = b;
                    }
                }
                "todo_calendar_default_week" => {
                    if let Some(b) = parse_bool(val) {
                        cfg.todo_calendar_default_week = b;
                    }
                }
                "size_preset" => {
                    if let Ok(v) = val.parse::<u8>() {
                        cfg.size_preset = v.min(4);
                    }
                }
                _ => {}
            }
        }
        cfg
    }
}

fn format_clock_ring_mode(mode: u8) -> &'static str {
    match mode {
        1 => "water",
        2 => "todo",
        3 => "auto",
        _ => "off",
    }
}

fn parse_clock_ring_mode(s: &str) -> Option<u8> {
    match s.trim().to_ascii_lowercase().as_str() {
        "0" | "off" | "none" => Some(0),
        "1" | "water" => Some(1),
        "2" | "todo" | "to-do" => Some(2),
        "3" | "auto" => Some(3),
        _ => None,
    }
}

fn format_page_enabled(enabled: [bool; 6]) -> String {
    enabled
        .iter()
        .map(|&b| if b { "true" } else { "false" })
        .collect::<Vec<_>>()
        .join(",")
}

fn parse_page_enabled(s: &str) -> Option<[bool; 6]> {
    let mut out = [true; 6];
    let mut count = 0usize;
    for tok in s.split(',') {
        if count >= 6 {
            return None;
        }
        out[count] = parse_bool(tok.trim())?;
        count += 1;
    }
    if (count == 5 || count == 6) && out.iter().any(|&b| b) {
        Some(out)
    } else {
        None
    }
}

fn format_page_order(order: [u8; 6]) -> String {
    const NAMES: [&str; 6] = ["music", "focus", "water", "shelf", "todo", "widgets"];
    order
        .iter()
        .map(|&i| NAMES.get(i as usize).copied().unwrap_or("music"))
        .collect::<Vec<_>>()
        .join(",")
}

fn parse_page_order(s: &str) -> Option<[u8; 6]> {
    let mut out = [0u8; 6];
    let mut seen = [false; 6];
    let mut count = 0usize;
    for tok in s.split(',') {
        let id: u8 = match tok.trim().to_ascii_lowercase().as_str() {
            "0" | "music" => 0,
            "1" | "focus" | "pomodoro" => 1,
            "2" | "water" => 2,
            "3" | "shelf" => 3,
            "4" | "todo" | "to-do" | "todolist" => 4,
            "5" | "widgets" | "widget" => 5,
            _ => return None,
        };
        if seen[id as usize] || count >= 6 {
            return None;
        }
        seen[id as usize] = true;
        out[count] = id;
        count += 1;
    }
    if count == 5 && !seen[5] {
        out[5] = 5;
        Some(out)
    } else if count == 6 {
        Some(out)
    } else {
        None
    }
}

fn parse_bool(s: &str) -> Option<bool> {
    match s.to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Some(true),
        "false" | "0" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn parse_hex_rgba(s: &str) -> Option<[u8; 4]> {
    let hex = s.trim().strip_prefix('#').unwrap_or(s.trim());
    match hex.len() {
        6 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            Some([r, g, b, 210])
        }
        8 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            let a = u8::from_str_radix(&hex[6..8], 16).ok()?;
            Some([r, g, b, a])
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_default_config() {
        let cfg = Config::default();
        let text = cfg.to_kv_string();
        let parsed = Config::parse_str(&text);
        assert_eq!(cfg, parsed);
    }

    #[test]
    fn parses_custom_values_and_ignores_invalid() {
        let input = "\
            # comment\n\
            ; another comment\n\
            notch_width = 250\n\
            notch_height = invalid\n\
            horizontal_offset = 120\n\
            glass_tint = #112233CC\n\
            open_on_hover = false\n\
            minimal_mode = yes\n\
            show_mascot = true\n\
            show_clock = 1\n\
            hide_fullscreen = false\n\
            monitor_index = 2\n\
        ";
        let cfg = Config::parse_str(input);
        assert_eq!(cfg.notch_width, 250.0);
        assert_eq!(cfg.notch_height, Config::default().notch_height);
        assert_eq!(cfg.horizontal_offset, 120.0);
        assert_eq!(cfg.glass_tint, [0x11, 0x22, 0x33, 0xCC]);
        assert!(!cfg.open_on_hover);
        assert!(cfg.minimal_mode);
        assert!(cfg.show_mascot);
        assert!(cfg.show_clock);
        assert!(!cfg.hide_fullscreen);
        assert_eq!(cfg.monitor_index, 2);
    }

    #[test]
    fn detects_legacy_milestone_configs() {
        let m1 = Config::parse_str("expanded_width = 380\nglass_tint = #0E1016D2\n");
        let m2 = Config::parse_str("expanded_width = 446\nglass_tint = #020204F6\n");
        let m4 = Config::parse_str("notch_width = 158\nexpanded_width = 420\nglass_tint = #000000FF\n");
        let current = Config::default();
        assert!(m1.is_legacy_auto_config());
        assert!(m2.is_legacy_auto_config());
        assert!(m4.is_legacy_auto_config());
        assert!(!current.is_legacy_auto_config());
    }
}
