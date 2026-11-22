use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use windows::Foundation::Uri;
use windows::Web::Http::HttpClient;
use windows::Win32::Foundation::{FILETIME, SYSTEMTIME};
use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};
use windows::core::HSTRING;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[repr(usize)]
pub enum NotchPage {
    Music = 0,
    Pomodoro = 1,
    Water = 2,
    Shelf = 3,
    Todo = 4,
    Widgets = 5,
}

impl NotchPage {
    pub const ALL: [NotchPage; 6] = [
        NotchPage::Music,
        NotchPage::Pomodoro,
        NotchPage::Water,
        NotchPage::Shelf,
        NotchPage::Todo,
        NotchPage::Widgets,
    ];

    pub const fn index(self) -> usize {
        self as usize
    }

    pub const fn from_index(idx: usize) -> Self {
        match idx {
            0 => Self::Music,
            1 => Self::Pomodoro,
            2 => Self::Water,
            3 => Self::Shelf,
            4 => Self::Todo,
            5 => Self::Widgets,
            _ => Self::Music,
        }
    }

    pub const fn next(self) -> Self {
        Self::from_index((self.index() + 1) % Self::ALL.len())
    }

    pub const fn prev(self) -> Self {
        let len = Self::ALL.len();
        Self::from_index((self.index() + len - 1) % len)
    }

    #[allow(dead_code)]
    pub const fn title(self) -> &'static str {
        match self {
            Self::Music => "Music",
            Self::Pomodoro => "Pomodoro",
            Self::Water => "Water",
            Self::Shelf => "Shelf",
            Self::Todo => "Todo",
            Self::Widgets => "Widgets",
        }
    }
}

pub const WORLD_CITIES: [(&str, i32); 16] = [
    ("Tokyo", 9 * 60),
    ("London", 0),
    ("New York", -5 * 60),
    ("Rome", 60),
    ("Paris", 60),
    ("Dubai", 4 * 60),
    ("Singapore", 8 * 60),
    ("Sydney", 10 * 60),
    ("Los Angeles", -8 * 60),
    ("Seoul", 9 * 60),
    ("Berlin", 60),
    ("Cairo", 2 * 60),
    ("Mumbai", 5 * 60 + 30),
    ("Toronto", -5 * 60),
    ("Istanbul", 3 * 60),
    ("Sao Paulo", -3 * 60),
];

pub const COUNTDOWN_TARGETS: [(&str, u16, u16); 5] = [
    ("New Year", 1, 1),
    ("Christmas", 12, 25),
    ("Summer", 6, 21),
    ("Halloween", 10, 31),
    ("Valentine's", 2, 14),
];

pub const WEATHER_CITIES: [(&str, f64, f64); 17] = [
    ("Auto", 0.0, 0.0),
    ("Tokyo", 35.6762, 139.6503),
    ("London", 51.5074, -0.1278),
    ("New York", 40.7128, -74.0060),
    ("Rome", 41.9028, 12.4964),
    ("Paris", 48.8566, 2.3522),
    ("Dubai", 25.2048, 55.2708),
    ("Singapore", 1.3521, 103.8198),
    ("Sydney", -33.8688, 151.2093),
    ("Los Angeles", 34.0522, -118.2437),
    ("Seoul", 37.5665, 126.9780),
    ("Berlin", 52.5200, 13.4050),
    ("Cairo", 30.0444, 31.2357),
    ("Mumbai", 19.0760, 72.8777),
    ("Toronto", 43.6532, -79.3832),
    ("Istanbul", 41.0082, 28.9784),
    ("Sao Paulo", -23.5505, -46.6333),
];

#[derive(Clone, Debug, Default)]
pub struct WeatherSnapshot {
    pub city: String,
    pub temp_c: i32,
    pub wmo_code: u16,
    pub is_day: bool,
    pub valid: bool,
}

impl WeatherSnapshot {
    pub fn condition_label(&self) -> &'static str {
        match self.wmo_code {
            0 => {
                if self.is_day {
                    "Clear sky"
                } else {
                    "Clear night"
                }
            }
            1 | 2 => "Partly cloudy",
            3 => "Overcast",
            45 | 48 => "Foggy",
            51..=57 => "Drizzle",
            61..=67 | 80..=82 => "Rainy",
            71..=77 | 85..=86 => "Snowy",
            95..=99 => "Thunderstorm",
            _ => {
                if self.is_day {
                    "Partly sunny"
                } else {
                    "Clear evening"
                }
            }
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum WidgetKind {
    ThisWeek = 0,
    Battery = 1,
    Water = 2,
    FocusToday = 3,
    WorldClock = 4,
    Moon = 5,
    Countdown = 6,
    Weather = 7,
}

impl WidgetKind {
    pub const ALL: [WidgetKind; 8] = [
        WidgetKind::ThisWeek,
        WidgetKind::Battery,
        WidgetKind::Water,
        WidgetKind::FocusToday,
        WidgetKind::WorldClock,
        WidgetKind::Moon,
        WidgetKind::Countdown,
        WidgetKind::Weather,
    ];

    pub const fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::ThisWeek,
            1 => Self::Battery,
            2 => Self::Water,
            3 => Self::FocusToday,
            4 => Self::WorldClock,
            5 => Self::Moon,
            6 => Self::Countdown,
            7 => Self::Weather,
            _ => Self::ThisWeek,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::ThisWeek => "This week",
            Self::Battery => "Battery",
            Self::Water => "Water",
            Self::FocusToday => "Focus today",
            Self::WorldClock => "World clock",
            Self::Moon => "Moon",
            Self::Countdown => "Countdown",
            Self::Weather => "Weather",
        }
    }
}

#[repr(C)]
#[derive(Default)]
struct SystemPowerStatusFfi {
    ac_line_status: u8,
    battery_flag: u8,
    battery_life_percent: u8,
    system_status_flag: u8,
    battery_life_time: u32,
    battery_full_life_time: u32,
}

#[link(name = "kernel32")]
extern "system" {
    fn GetSystemPowerStatus(lp_system_power_status: *mut SystemPowerStatusFfi) -> i32;
    fn GetLocalTime(lp_system_time: *mut SYSTEMTIME);
    fn GetSystemTime(lp_system_time: *mut SYSTEMTIME);
}

#[repr(C)]
#[derive(Copy, Clone)]
struct DevPropKeyFfi {
    fmtid: [u8; 16],
    pid: u32,
}

#[link(name = "cfgmgr32")]
extern "system" {
    fn CM_Get_Device_ID_List_SizeW(
        pul_len: *mut u32,
        psz_filter: *const u16,
        ul_flags: u32,
    ) -> u32;
    fn CM_Get_Device_ID_ListW(
        psz_filter: *const u16,
        buffer: *mut u16,
        buffer_len: u32,
        ul_flags: u32,
    ) -> u32;
    fn CM_Locate_DevNodeW(
        pdn_dev_inst: *mut u32,
        p_device_id: *const u16,
        ul_flags: u32,
    ) -> u32;
    fn CM_Get_DevNode_Status(
        pul_status: *mut u32,
        pul_problem_number: *mut u32,
        dn_dev_inst: u32,
        ul_flags: u32,
    ) -> u32;
    fn CM_Get_DevNode_PropertyW(
        dn_dev_inst: u32,
        property_key: *const DevPropKeyFfi,
        property_type: *mut u32,
        property_buffer: *mut u8,
        property_buffer_size: *mut u32,
        ul_flags: u32,
    ) -> u32;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BatteryDevice {
    pub name: String,
    pub pct: u8,
    pub charging: bool,
    pub is_bluetooth: bool,
    pub is_ac_only: bool,
}

fn clean_bt_device_name(raw: &str) -> String {
    let mut s = raw.trim().to_string();
    for suffix in [
        " Hands-Free AG Audio",
        " Hands-Free AG",
        " Hands-Free",
        " Avrcp Transport",
        " Stereo",
        " Audio",
        " Device",
    ] {
        if let Some(stripped) = s.strip_suffix(suffix) {
            if !stripped.trim().is_empty() {
                s = stripped.trim().to_string();
            }
        }
    }
    if s.is_empty() {
        "Bluetooth".to_string()
    } else {
        s
    }
}

fn extract_bt_mac(upper_id: &str) -> Option<String> {
    if let Some(rest) = upper_id.split("DEV_").nth(1) {
        let cand: String = rest
            .chars()
            .take_while(|c| c.is_ascii_hexdigit())
            .collect();
        if cand.len() == 12 {
            return Some(cand);
        }
    }
    for tok in upper_id.split(['\\', '&', '_']) {
        if tok.len() == 12 && tok.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Some(tok.to_string());
        }
    }
    None
}

fn query_bluetooth_batteries() -> Vec<BatteryDevice> {
    use std::collections::HashSet;

    const CR_SUCCESS: u32 = 0;
    const CM_GETIDLIST_FILTER_PRESENT: u32 = 0x0000_0100;
    const CM_LOCATE_DEVNODE_NORMAL: u32 = 0;
    const DN_STARTED: u32 = 0x0000_0008;
    const DEVPROP_TYPE_BYTE: u32 = 0x0000_0003;
    const DEVPROP_TYPE_BOOLEAN: u32 = 0x0000_0011;
    const DEVPROP_TYPE_STRING: u32 = 0x0000_0012;

    const KEY_BT_BATTERY: DevPropKeyFfi = DevPropKeyFfi {
        fmtid: [
            0x19, 0xA3, 0x4E, 0x10, 0xE2, 0x6E, 0x01, 0x47, 0xBD, 0x47, 0x8D, 0xDB, 0xF4, 0x25,
            0xBB, 0xE5,
        ],
        pid: 2,
    };
    const KEY_NAME: DevPropKeyFfi = DevPropKeyFfi {
        fmtid: [
            0x30, 0xF1, 0x25, 0xB7, 0xEF, 0x47, 0x1A, 0x10, 0xA5, 0xF1, 0x02, 0x60, 0x8C, 0x9E,
            0xEB, 0xAC,
        ],
        pid: 10,
    };
    const KEY_IS_CONNECTED: DevPropKeyFfi = DevPropKeyFfi {
        fmtid: [
            0x26, 0x63, 0xDA, 0x83, 0xA6, 0x97, 0x88, 0x40, 0x94, 0x53, 0xA1, 0x92, 0x3F, 0x57,
            0x3B, 0x29,
        ],
        pid: 15,
    };

    let mut out: Vec<BatteryDevice> = Vec::new();
    let mut len: u32 = 0;
    if unsafe {
        CM_Get_Device_ID_List_SizeW(&mut len, std::ptr::null(), CM_GETIDLIST_FILTER_PRESENT)
    } != CR_SUCCESS
        || len <= 1
    {
        return out;
    }

    let mut buf = vec![0u16; len as usize];
    if unsafe {
        CM_Get_Device_ID_ListW(
            std::ptr::null(),
            buf.as_mut_ptr(),
            len,
            CM_GETIDLIST_FILTER_PRESENT,
        )
    } != CR_SUCCESS
    {
        return out;
    }

    let mut bt_nodes: Vec<(u32, Option<String>, bool)> = Vec::new();
    let mut connected_macs: HashSet<String> = HashSet::new();

    let mut pos = 0usize;
    while pos < buf.len() && buf[pos] != 0 {
        let end = buf[pos..].iter().position(|&c| c == 0).unwrap_or(0) + pos;
        let id_slice = &buf[pos..=end];
        let id_str = String::from_utf16_lossy(&buf[pos..end]);
        pos = end + 1;

        let upper = id_str.to_ascii_uppercase();
        if !(upper.starts_with("BTHENUM\\")
            || upper.starts_with("BTHLE\\")
            || upper.starts_with("BTHLEDEVICE\\")
            || upper.starts_with("BTHHFENUM\\"))
        {
            continue;
        }

        let mut dev_inst: u32 = 0;
        if unsafe { CM_Locate_DevNodeW(&mut dev_inst, id_slice.as_ptr(), CM_LOCATE_DEVNODE_NORMAL) }
            != CR_SUCCESS
        {
            continue;
        }

        let mut status: u32 = 0;
        let mut problem: u32 = 0;
        if unsafe { CM_Get_DevNode_Status(&mut status, &mut problem, dev_inst, 0) } != CR_SUCCESS
            || (status & DN_STARTED) == 0
            || problem != 0
        {
            continue;
        }

        let mut conn_type: u32 = 0;
        let mut conn_val: u8 = 0;
        let mut conn_sz: u32 = 1;
        let is_conn = unsafe {
            CM_Get_DevNode_PropertyW(
                dev_inst,
                &KEY_IS_CONNECTED,
                &mut conn_type,
                &mut conn_val,
                &mut conn_sz,
                0,
            )
        } == CR_SUCCESS
            && conn_type == DEVPROP_TYPE_BOOLEAN
            && conn_val != 0;

        let mac = extract_bt_mac(&upper);
        if is_conn {
            if let Some(ref m) = mac {
                connected_macs.insert(m.clone());
            }
        }
        bt_nodes.push((dev_inst, mac, is_conn));
    }

    for (dev_inst, mac, self_conn) in bt_nodes {
        let is_connected = self_conn
            || mac
                .as_ref()
                .is_some_and(|m| connected_macs.contains(m));
        if !is_connected {
            continue;
        }

        let mut prop_type: u32 = 0;
        let mut bat_val: u8 = 255;
        let mut bat_sz: u32 = 1;
        let cr_bat = unsafe {
            CM_Get_DevNode_PropertyW(
                dev_inst,
                &KEY_BT_BATTERY,
                &mut prop_type,
                &mut bat_val,
                &mut bat_sz,
                0,
            )
        };
        if cr_bat != CR_SUCCESS || prop_type != DEVPROP_TYPE_BYTE || bat_val > 100 {
            continue;
        }

        let mut name_type: u32 = 0;
        let mut name_bytes = [0u8; 256];
        let mut name_sz: u32 = name_bytes.len() as u32;
        let name = if unsafe {
            CM_Get_DevNode_PropertyW(
                dev_inst,
                &KEY_NAME,
                &mut name_type,
                name_bytes.as_mut_ptr(),
                &mut name_sz,
                0,
            )
        } == CR_SUCCESS
            && name_type == DEVPROP_TYPE_STRING
            && name_sz >= 2
        {
            let u16_cnt = (name_sz as usize / 2).min(128);
            let mut wchars = Vec::with_capacity(u16_cnt);
            for c in name_bytes[..u16_cnt * 2].chunks_exact(2) {
                let v = u16::from_le_bytes([c[0], c[1]]);
                if v == 0 {
                    break;
                }
                wchars.push(v);
            }
            clean_bt_device_name(&String::from_utf16_lossy(&wchars))
        } else {
            "Bluetooth".to_string()
        };

        if let Some(existing) = out.iter_mut().find(|d| d.name.eq_ignore_ascii_case(&name)) {
            existing.pct = bat_val;
        } else {
            out.push(BatteryDevice {
                name,
                pct: bat_val,
                charging: false,
                is_bluetooth: true,
                is_ac_only: false,
            });
        }
    }
    out
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BtPopupEvent {
    Connected { name: String, pct: u8 },
    Disconnected { name: String },
}

#[derive(Clone, Debug, Default)]
pub struct AiViewData {
    pub phase: crate::ai::AiPhase,
    pub prompt: String,
    pub submitted_prompt: String,
    pub response: String,
    pub error: Option<String>,
    pub history: Vec<(String, String)>,
    pub turns: Vec<(String, String)>,
    pub viewing_turn: Option<usize>,
    pub chunks: Vec<(usize, usize, f32)>,
    pub copied_feedback: bool,
    pub has_api_key: bool,
    pub model_label: String,
    pub scroll_y: f32,
    pub sel_anchor: Option<(usize, u32)>,
    pub sel_focus: Option<(usize, u32)>,
    pub sel_dragging: bool,
}

impl AiViewData {
    pub fn clear_selection(&mut self) {
        self.sel_anchor = None;
        self.sel_focus = None;
        self.sel_dragging = false;
    }

    pub fn normalized_selection(&self) -> Option<((usize, u32), (usize, u32))> {
        let (a, b) = (self.sel_anchor?, self.sel_focus?);
        if a == b {
            return None;
        }
        if a < b {
            Some((a, b))
        } else {
            Some((b, a))
        }
    }
    pub fn total_turns(&self) -> usize {
        if self.phase == crate::ai::AiPhase::Busy {
            self.turns.len() + 1
        } else {
            self.turns.len()
        }
    }

    pub fn active_turn_index(&self) -> usize {
        let total = self.total_turns();
        if total == 0 {
            return 0;
        }
        match self.viewing_turn {
            Some(idx) if idx < self.turns.len() => idx,
            _ => total - 1,
        }
    }

    pub fn displayed_turn(&self) -> (&str, &str, bool) {
        if let Some(idx) = self.viewing_turn {
            if let Some((q, a)) = self.turns.get(idx) {
                return (q.as_str(), a.as_str(), false);
            }
        }
        let body = self.error.as_deref().unwrap_or(self.response.as_str());
        (self.submitted_prompt.as_str(), body, true)
    }
}

#[derive(Clone, Debug)]
pub struct WidgetState {
    pub selected_week_day: usize,
    pub battery_pct: u8,
    pub battery_charging: bool,
    pub battery_has_battery: bool,
    pub battery_devices: Vec<BatteryDevice>,
    pub selected_battery_idx: usize,
    pub popup_bt_name: String,
    pub popup_bt_pct: u8,
    pub popup_bt_connected: bool,
    pub active_toast: Option<crate::osd_monitors::ToastNotification>,
    pub active_network: Option<crate::osd_monitors::NetworkEvent>,
    pub active_audio_device: String,
    pub active_mic_app: String,
    pub osd_alert_progress: f32,
    pub update_available_ver: String,
    pub ai: AiViewData,
    known_bt_names: Vec<String>,
    pub weather: WeatherSnapshot,
    weather_shared: Arc<Mutex<Option<WeatherSnapshot>>>,
    weather_city_requested: u8,
    weather_last_fetch: Option<Instant>,
    weather_fetching: Arc<AtomicBool>,
}

impl Default for WidgetState {
    fn default() -> Self {
        Self::new()
    }
}

impl WidgetState {
    pub fn new() -> Self {
        let (_, today_idx) = Self::current_week_days();
        let mut s = Self {
            selected_week_day: today_idx,
            battery_pct: 100,
            battery_charging: true,
            battery_has_battery: false,
            battery_devices: Vec::new(),
            selected_battery_idx: 0,
            popup_bt_name: String::new(),
            popup_bt_pct: 100,
            popup_bt_connected: true,
            active_toast: None,
            active_network: None,
            active_audio_device: String::new(),
            active_mic_app: String::new(),
            osd_alert_progress: 1.0,
            update_available_ver: String::new(),
            ai: AiViewData::default(),
            known_bt_names: Vec::new(),
            weather: WeatherSnapshot::default(),
            weather_shared: Arc::new(Mutex::new(None)),
            weather_city_requested: u8::MAX,
            weather_last_fetch: None,
            weather_fetching: Arc::new(AtomicBool::new(false)),
        };
        s.refresh_power_status();
        s.known_bt_names = s
            .battery_devices
            .iter()
            .filter(|d| d.is_bluetooth)
            .map(|d| d.name.clone())
            .collect();
        s
    }

    pub fn refresh_power_status(&mut self) {
        let mut st = SystemPowerStatusFfi::default();
        let ok = unsafe { GetSystemPowerStatus(&mut st) };
        if ok != 0 {
            let no_bat = (st.battery_flag & 128) != 0 || st.battery_life_percent == 255;
            self.battery_has_battery = !no_bat;
            self.battery_pct = if no_bat {
                100
            } else {
                st.battery_life_percent.min(100)
            };
            self.battery_charging = st.ac_line_status == 1 || (st.battery_flag & 8) != 0;
        }

        let mut list = Vec::new();
        if self.battery_has_battery {
            list.push(BatteryDevice {
                name: "Laptop".to_string(),
                pct: self.battery_pct,
                charging: self.battery_charging,
                is_bluetooth: false,
                is_ac_only: false,
            });
        }
        list.extend(query_bluetooth_batteries());
        self.battery_devices = list;
        if self.selected_battery_idx >= self.battery_devices.len() {
            self.selected_battery_idx = 0;
        }
    }

    pub fn poll_bluetooth_events(&mut self) -> Option<BtPopupEvent> {
        self.refresh_power_status();
        let current_bt: Vec<(String, u8)> = self
            .battery_devices
            .iter()
            .filter(|d| d.is_bluetooth)
            .map(|d| (d.name.clone(), d.pct))
            .collect();
        let newly_connected = current_bt
            .iter()
            .find(|(name, _)| !self.known_bt_names.iter().any(|k| k.eq_ignore_ascii_case(name)))
            .cloned();
        let newly_disconnected = if newly_connected.is_none() {
            self.known_bt_names
                .iter()
                .find(|old| !current_bt.iter().any(|(cur, _)| cur.eq_ignore_ascii_case(old)))
                .cloned()
        } else {
            None
        };
        self.known_bt_names = current_bt.into_iter().map(|(n, _)| n).collect();
        if let Some((name, pct)) = newly_connected {
            self.popup_bt_name = name.clone();
            self.popup_bt_pct = pct;
            self.popup_bt_connected = true;
            return Some(BtPopupEvent::Connected { name, pct });
        }
        if let Some(name) = newly_disconnected {
            self.popup_bt_name = name.clone();
            self.popup_bt_pct = 0;
            self.popup_bt_connected = false;
            return Some(BtPopupEvent::Disconnected { name });
        }
        None
    }

    pub fn active_battery(&self) -> Option<BatteryDevice> {
        self.battery_devices.get(self.selected_battery_idx).cloned()
    }

    pub fn cycle_battery_device(&mut self) {
        self.refresh_power_status();
        if self.battery_devices.len() > 1 {
            self.selected_battery_idx = (self.selected_battery_idx + 1) % self.battery_devices.len();
        }
    }

    pub fn current_week_days() -> ([u16; 7], usize) {
        let mut st = SYSTEMTIME::default();
        unsafe { GetLocalTime(&mut st) };
        let today_idx = ((st.wDayOfWeek + 6) % 7) as usize;
        let jdn_today = ymd_to_jdn(st.wYear as i32, st.wMonth as i32, st.wDay as i32);
        let mon_jdn = jdn_today - today_idx as i32;
        let mut days = [1u16; 7];
        for (i, slot) in days.iter_mut().enumerate() {
            let (_, _, d) = jdn_to_ymd(mon_jdn + i as i32);
            *slot = d as u16;
        }
        (days, today_idx)
    }

    pub fn local_weather_date() -> (String, &'static str, u16) {
        let mut st = SYSTEMTIME::default();
        unsafe { GetLocalTime(&mut st) };
        const MONTHS: [&str; 12] = [
            "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
        ];
        const DAYS: [&str; 7] = [
            "Sunday",
            "Monday",
            "Tuesday",
            "Wednesday",
            "Thursday",
            "Friday",
            "Saturday",
        ];
        let mon = MONTHS[(st.wMonth.saturating_sub(1) as usize).min(11)];
        let wd = DAYS[(st.wDayOfWeek as usize).min(6)];
        (format!("{mon} {}", st.wDay), wd, st.wHour)
    }

    pub fn poll_weather(&mut self, city_setting: u8) -> bool {
        let mut updated = false;
        if let Ok(mut guard) = self.weather_shared.try_lock() {
            if let Some(snap) = guard.take() {
                self.weather = snap;
                updated = true;
            }
        }

        let city_changed = self.weather_city_requested != city_setting;
        let stale = match self.weather_last_fetch {
            None => true,
            Some(t) => t.elapsed() >= Duration::from_secs(30 * 60),
        };
        if (city_changed || stale)
            && !self.weather_fetching.swap(true, Ordering::SeqCst)
        {
            self.weather_city_requested = city_setting;
            self.weather_last_fetch = Some(Instant::now());
            let shared = Arc::clone(&self.weather_shared);
            let fetching = Arc::clone(&self.weather_fetching);
            let _ = std::thread::Builder::new()
                .name("eave-weather".to_string())
                .spawn(move || {
                    if let Some(snap) = fetch_weather_blocking(city_setting) {
                        if let Ok(mut guard) = shared.lock() {
                            *guard = Some(snap);
                        }
                    }
                    fetching.store(false, Ordering::SeqCst);
                });
        }
        updated
    }

    pub fn moon_phase() -> (&'static str, f32, f32, u32) {
        let mut st = SYSTEMTIME::default();
        unsafe { GetSystemTime(&mut st) };
        let jdn = ymd_to_jdn(st.wYear as i32, st.wMonth as i32, st.wDay as i32) as f64
            + (st.wHour as f64 + st.wMinute as f64 / 60.0) / 24.0;
        const SYNODIC: f64 = 29.530_588_67;
        let age = ((jdn - 2_451_550.26) % SYNODIC + SYNODIC) % SYNODIC;
        let cycle = (age / SYNODIC) as f32;
        let illum = (0.5 * (1.0 - (std::f32::consts::TAU * cycle).cos())).clamp(0.0, 1.0);
        let name = if cycle < 0.03 || cycle >= 0.97 {
            "New Moon"
        } else if cycle < 0.22 {
            "Waxing Crescent"
        } else if cycle < 0.28 {
            "First Quarter"
        } else if cycle < 0.47 {
            "Waxing Gibbous"
        } else if cycle < 0.53 {
            "Full Moon"
        } else if cycle < 0.72 {
            "Waning Gibbous"
        } else if cycle < 0.78 {
            "Last Quarter"
        } else {
            "Waning Crescent"
        };
        let half = SYNODIC * 0.5;
        let to_full = if age <= half {
            half - age
        } else {
            SYNODIC - age + half
        };
        (name, illum, cycle, to_full.round().max(0.0) as u32)
    }

    pub fn countdown_info(preset: u8) -> (&'static str, u32, f32) {
        let (label, m, d) = COUNTDOWN_TARGETS[(preset as usize).min(COUNTDOWN_TARGETS.len() - 1)];
        let mut st = SYSTEMTIME::default();
        unsafe { GetLocalTime(&mut st) };
        let today = ymd_to_jdn(st.wYear as i32, st.wMonth as i32, st.wDay as i32);
        let mut target = ymd_to_jdn(st.wYear as i32, m as i32, d as i32);
        if target < today {
            target = ymd_to_jdn(st.wYear as i32 + 1, m as i32, d as i32);
        }
        let days_left = (target - today).max(0) as u32;
        let prog = (1.0 - (days_left as f32 / 365.0)).clamp(0.04, 1.0);
        (label, days_left, prog)
    }

    pub fn week_day_info(day_idx: usize) -> (i32, &'static str, &'static str, u16, usize) {
        const WK_SHORT: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
        const MONTHS: [&str; 12] = [
            "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
        ];
        let mut st = SYSTEMTIME::default();
        unsafe { GetLocalTime(&mut st) };
        let today_idx = ((st.wDayOfWeek + 6) % 7) as usize;
        let jdn_today = ymd_to_jdn(st.wYear as i32, st.wMonth as i32, st.wDay as i32);
        let mon_jdn = jdn_today - today_idx as i32;
        let d_idx = day_idx.min(6);
        let sel_jdn = mon_jdn + d_idx as i32;
        let (_, m, d) = jdn_to_ymd(sel_jdn);
        let mon = MONTHS[(m - 1).clamp(0, 11) as usize];
        (sel_jdn, WK_SHORT[d_idx], mon, d as u16, today_idx)
    }

    pub fn world_clock_city(city_idx: u8) -> (&'static str, String, String) {
        let idx = (city_idx as usize).min(WORLD_CITIES.len() - 1);
        let (name, base_off_min) = WORLD_CITIES[idx];
        let mut utc = SYSTEMTIME::default();
        let mut loc = SYSTEMTIME::default();
        unsafe {
            GetSystemTime(&mut utc);
            GetLocalTime(&mut loc);
        }
        let off_min = base_off_min + dst_offset_minutes(city_dst_rule(idx), &utc, base_off_min);
        let utc_total = utc.wHour as i32 * 60 + utc.wMinute as i32;
        let city_total = (utc_total + off_min).rem_euclid(24 * 60);
        let hh = city_total / 60;
        let mm = city_total % 60;

        let utc_jdn = ymd_to_jdn(utc.wYear as i32, utc.wMonth as i32, utc.wDay as i32);
        let loc_jdn = ymd_to_jdn(loc.wYear as i32, loc.wMonth as i32, loc.wDay as i32);
        let loc_off_min = (loc_jdn - utc_jdn) * 24 * 60
            + (loc.wHour as i32 * 60 + loc.wMinute as i32)
            - utc_total;
        let diff_h = ((off_min - loc_off_min) as f32 / 60.0).round() as i32;
        let delta = if diff_h == 0 {
            "Same".to_string()
        } else if diff_h > 0 {
            format!("+{diff_h} h")
        } else {
            format!("{diff_h} h")
        };
        (name, format!("{hh:02}:{mm:02}"), delta)
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum DstRule {
    None,
    Eu,
    Us,
    Au,
}

fn city_dst_rule(city_idx: usize) -> DstRule {
    match city_idx {
        1 | 3 | 4 | 10 => DstRule::Eu,
        2 | 8 | 13 => DstRule::Us,
        7 => DstRule::Au,
        _ => DstRule::None,
    }
}

fn nth_sunday_day(year: i32, month: i32, nth: i32) -> i32 {
    let first_jdn = ymd_to_jdn(year, month, 1);
    let first_dow_sun0 = (first_jdn + 1).rem_euclid(7);
    let offset = (7 - first_dow_sun0) % 7;
    1 + offset + (nth - 1) * 7
}

fn last_sunday_day(year: i32, month: i32) -> i32 {
    let next_m = if month == 12 { 1 } else { month + 1 };
    let next_y = if month == 12 { year + 1 } else { year };
    let last_jdn = ymd_to_jdn(next_y, next_m, 1) - 1;
    let (_, _, last_day) = jdn_to_ymd(last_jdn);
    let last_dow_sun0 = (last_jdn + 1).rem_euclid(7);
    last_day - last_dow_sun0
}

fn utc_minutes_since_epoch(year: i32, month: i32, day: i32, hour: i32, min: i32) -> i64 {
    let jdn = ymd_to_jdn(year, month, day) as i64;
    jdn * 24 * 60 + hour as i64 * 60 + min as i64
}

fn dst_offset_minutes(rule: DstRule, utc: &SYSTEMTIME, base_off_min: i32) -> i32 {
    let y = utc.wYear as i32;
    let now_utc = utc_minutes_since_epoch(
        y,
        utc.wMonth as i32,
        utc.wDay as i32,
        utc.wHour as i32,
        utc.wMinute as i32,
    );
    match rule {
        DstRule::None => 0,
        DstRule::Eu => {
            let start = utc_minutes_since_epoch(y, 3, last_sunday_day(y, 3), 1, 0);
            let end = utc_minutes_since_epoch(y, 10, last_sunday_day(y, 10), 1, 0);
            if now_utc >= start && now_utc < end { 60 } else { 0 }
        }
        DstRule::Us => {
            let start = utc_minutes_since_epoch(y, 3, nth_sunday_day(y, 3, 2), 2, 0)
                - base_off_min as i64;
            let end = utc_minutes_since_epoch(y, 11, nth_sunday_day(y, 11, 1), 2, 0)
                - (base_off_min + 60) as i64;
            if now_utc >= start && now_utc < end { 60 } else { 0 }
        }
        DstRule::Au => {
            let end_apr = utc_minutes_since_epoch(y, 4, nth_sunday_day(y, 4, 1), 3, 0)
                - (base_off_min + 60) as i64;
            let start_oct = utc_minutes_since_epoch(y, 10, nth_sunday_day(y, 10, 1), 2, 0)
                - base_off_min as i64;
            if now_utc < end_apr || now_utc >= start_oct { 60 } else { 0 }
        }
    }
}

fn ymd_to_jdn(y: i32, m: i32, d: i32) -> i32 {
    let a = (14 - m) / 12;
    let y2 = y + 4800 - a;
    let m2 = m + 12 * a - 3;
    d + (153 * m2 + 2) / 5 + 365 * y2 + y2 / 4 - y2 / 100 + y2 / 400 - 32045
}

fn jdn_to_ymd(jdn: i32) -> (i32, i32, i32) {
    let a = jdn + 32044;
    let b = (4 * a + 3) / 146097;
    let c = a - (146097 * b) / 4;
    let d = (4 * c + 3) / 1461;
    let e = c - (1461 * d) / 4;
    let m = (5 * e + 2) / 153;
    let day = e - (153 * m + 2) / 5 + 1;
    let month = m + 3 - 12 * (m / 10);
    let year = 100 * b + d - 4800 + m / 10;
    (year, month, day)
}

fn current_local_jdn() -> i32 {
    let mut st = SYSTEMTIME::default();
    unsafe { GetLocalTime(&mut st) };
    ymd_to_jdn(st.wYear as i32, st.wMonth as i32, st.wDay as i32)
}

fn focus_history_path() -> PathBuf {
    if let Ok(appdata) = env::var("APPDATA") {
        let dir = PathBuf::from(appdata).join("eave");
        let _ = fs::create_dir_all(&dir);
        dir.join("focus_history.txt")
    } else {
        PathBuf::from("focus_history.txt")
    }
}

fn load_focus_history(today_jdn: i32) -> [u32; 7] {
    if cfg!(test) {
        return [0u32; 7];
    }
    let mut out = [0u32; 7];
    let path = focus_history_path();
    let Ok(contents) = fs::read_to_string(path) else {
        return out;
    };
    for line in contents.lines() {
        let Some((date_s, mins_s)) = line.trim().split_once('=') else {
            continue;
        };
        let mut parts = date_s.split('-');
        let (Some(y), Some(m), Some(d)) = (
            parts.next().and_then(|s| s.parse::<i32>().ok()),
            parts.next().and_then(|s| s.parse::<i32>().ok()),
            parts.next().and_then(|s| s.parse::<i32>().ok()),
        ) else {
            continue;
        };
        let Ok(mins) = mins_s.trim().parse::<u32>() else {
            continue;
        };
        let jdn = ymd_to_jdn(y, m, d);
        let age = today_jdn - jdn;
        if (0..7).contains(&age) {
            out[6 - age as usize] = mins;
        }
    }
    out
}

fn save_focus_history(today_jdn: i32, mins: &[u32; 7]) {
    if cfg!(test) {
        return;
    }
    let mut buf = String::new();
    for (i, &m) in mins.iter().enumerate() {
        if m == 0 {
            continue;
        }
        let jdn = today_jdn - (6 - i) as i32;
        let (y, mo, d) = jdn_to_ymd(jdn);
        buf.push_str(&format!("{y:04}-{mo:02}-{d:02}={m}\n"));
    }
    let _ = fs::write(focus_history_path(), buf);
}

fn http_get_text(client: &HttpClient, url: &str) -> Option<String> {
    let uri = Uri::CreateUri(&HSTRING::from(url)).ok()?;
    let op = client.GetStringAsync(&uri).ok()?;
    Some(op.get().ok()?.to_string_lossy())
}

fn json_extract_string(json: &str, key: &str) -> Option<String> {
    let pat = format!("\"{key}\"");
    let start = json.find(&pat)? + pat.len();
    let rest = json[start..].trim_start();
    let rest = rest.strip_prefix(':')?.trim_start();
    let rest = rest.strip_prefix('"')?;
    let mut out = String::new();
    let mut esc = false;
    for ch in rest.chars() {
        if esc {
            out.push(ch);
            esc = false;
        } else if ch == '\\' {
            esc = true;
        } else if ch == '"' {
            return Some(out);
        } else {
            out.push(ch);
        }
    }
    None
}

fn json_extract_f64(json: &str, key: &str) -> Option<f64> {
    let pat = format!("\"{key}\"");
    let start = json.find(&pat)? + pat.len();
    let rest = json[start..].trim_start();
    let rest = rest.strip_prefix(':')?.trim_start();
    let rest = rest.trim_start_matches('"');
    let num_str: String = rest
        .chars()
        .take_while(|c| c.is_ascii_digit() || matches!(c, '-' | '+' | '.'))
        .collect();
    num_str.parse::<f64>().ok()
}

fn fetch_weather_blocking(city_setting: u8) -> Option<WeatherSnapshot> {
    let client = HttpClient::new().ok()?;
    let (city, lat, lon) = if city_setting == 0 {
        if let Some(body) = http_get_text(&client, "https://ipwho.is/") {
            if let (Some(c), Some(la), Some(lo)) = (
                json_extract_string(&body, "city"),
                json_extract_f64(&body, "latitude"),
                json_extract_f64(&body, "longitude"),
            ) {
                (c, la, lo)
            } else if let Some(body2) = http_get_text(&client, "https://get.geojs.io/v1/ip/geo.json") {
                let c = json_extract_string(&body2, "city").unwrap_or_else(|| "Local".to_string());
                let la = json_extract_f64(&body2, "latitude")?;
                let lo = json_extract_f64(&body2, "longitude")?;
                (c, la, lo)
            } else {
                return None;
            }
        } else if let Some(body2) = http_get_text(&client, "https://get.geojs.io/v1/ip/geo.json") {
            let c = json_extract_string(&body2, "city").unwrap_or_else(|| "Local".to_string());
            let la = json_extract_f64(&body2, "latitude")?;
            let lo = json_extract_f64(&body2, "longitude")?;
            (c, la, lo)
        } else {
            return None;
        }
    } else {
        let idx = (city_setting as usize).min(WEATHER_CITIES.len() - 1);
        let (name, la, lo) = WEATHER_CITIES[idx];
        (name.to_string(), la, lo)
    };

    let url = format!(
        "https://api.open-meteo.com/v1/forecast?latitude={lat:.4}&longitude={lon:.4}&current=temperature_2m,weather_code,is_day&timezone=auto"
    );
    let body = http_get_text(&client, &url)?;
    let cur_slice = body
        .find("\"current\":{")
        .or_else(|| body.find("\"current\": {"))
        .map(|pos| &body[pos..])
        .unwrap_or(&body);
    let temp_c = json_extract_f64(cur_slice, "temperature_2m")?.round() as i32;
    let wmo_code = json_extract_f64(cur_slice, "weather_code")
        .unwrap_or(0.0)
        .round()
        .clamp(0.0, 99.0) as u16;
    let is_day = json_extract_f64(cur_slice, "is_day").unwrap_or(1.0) >= 0.5;

    Some(WeatherSnapshot {
        city,
        temp_c,
        wmo_code,
        is_day,
        valid: true,
    })
}

impl Default for NotchPage {
    fn default() -> Self {
        Self::Music
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PomodoroPhase {
    Focus,
    Break,
}

impl PomodoroPhase {
    #[allow(dead_code)]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Focus => "Focus",
            Self::Break => "Break",
        }
    }
}

#[derive(Clone, Debug)]
pub struct PomodoroState {
    pub phase: PomodoroPhase,
    pub running: bool,
    pub remaining_secs: f32,
    pub focus_secs: f32,
    pub break_secs: f32,
    pub long_break_secs: f32,
    pub current_round: u32,
    pub completed_sessions: u32,
    pub daily_focus_mins: [u32; 7],
    history_jdn: i32,
}

impl Default for PomodoroState {
    fn default() -> Self {
        Self::new()
    }
}

impl PomodoroState {
    pub const DEFAULT_FOCUS_SECS: f32 = 25.0 * 60.0;
    pub const DEFAULT_BREAK_SECS: f32 = 5.0 * 60.0;
    pub const DEFAULT_LONG_BREAK_SECS: f32 = 15.0 * 60.0;

    pub fn new() -> Self {
        let today = current_local_jdn();
        let daily_focus_mins = load_focus_history(today);
        Self {
            phase: PomodoroPhase::Focus,
            running: false,
            remaining_secs: Self::DEFAULT_FOCUS_SECS,
            focus_secs: Self::DEFAULT_FOCUS_SECS,
            break_secs: Self::DEFAULT_BREAK_SECS,
            long_break_secs: Self::DEFAULT_LONG_BREAK_SECS,
            current_round: 1,
            completed_sessions: 0,
            daily_focus_mins,
            history_jdn: today,
        }
    }

    fn sync_history_day(&mut self) {
        let today = current_local_jdn();
        if today > self.history_jdn {
            let shift = (today - self.history_jdn) as usize;
            if shift >= 7 {
                self.daily_focus_mins = [0; 7];
            } else {
                for i in 0..(7 - shift) {
                    self.daily_focus_mins[i] = self.daily_focus_mins[i + shift];
                }
                for slot in &mut self.daily_focus_mins[(7 - shift)..] {
                    *slot = 0;
                }
            }
            self.history_jdn = today;
        }
    }

    pub fn today_focus_mins(&self) -> u32 {
        let live_mins = if self.phase == PomodoroPhase::Focus && self.running {
            ((self.focus_secs - self.remaining_secs).max(0.0) / 60.0).floor() as u32
        } else {
            0
        };
        self.daily_focus_mins[6].saturating_add(live_mins)
    }

    pub fn focus_history_bars(&self) -> [f32; 7] {
        let mut vals = [0.0_f32; 7];
        for i in 0..7 {
            vals[i] = self.daily_focus_mins[i] as f32;
        }
        if self.phase == PomodoroPhase::Focus {
            vals[6] += (self.focus_secs - self.remaining_secs).max(0.0) / 60.0;
        }
        let max_m = vals
            .iter()
            .copied()
            .fold(60.0_f32, f32::max);
        let mut out = [0.0_f32; 7];
        for i in 0..7 {
            out[i] = if vals[i] <= 0.01 {
                0.0
            } else {
                (vals[i] / max_m).clamp(0.16, 1.0)
            };
        }
        out
    }

    pub fn toggle(&mut self) {
        self.running = !self.running;
    }

    pub fn reset(&mut self) {
        self.running = false;
        self.remaining_secs = self.phase_duration_secs();
    }

    pub fn switch_phase(&mut self) {
        self.running = false;
        self.phase = match self.phase {
            PomodoroPhase::Focus => PomodoroPhase::Break,
            PomodoroPhase::Break => PomodoroPhase::Focus,
        };
        self.remaining_secs = self.phase_duration_secs();
    }

    pub fn select_segment(&mut self, seg_idx: usize) {
        let idx = seg_idx.min(7);
        self.running = false;
        if idx % 2 == 0 {
            self.phase = PomodoroPhase::Focus;
            self.current_round = (idx / 2 + 1) as u32;
        } else if idx == 7 {
            self.phase = PomodoroPhase::Break;
            self.current_round = 4;
        } else {
            self.phase = PomodoroPhase::Break;
            self.current_round = ((idx + 1) / 2) as u32;
        }
        self.remaining_secs = self.phase_duration_secs();
    }

    pub fn tick(&mut self, dt: f32) -> bool {
        if !self.running || dt <= 0.0 || !dt.is_finite() {
            return false;
        }
        self.remaining_secs -= dt;
        if self.remaining_secs <= 0.0 {
            match self.phase {
                PomodoroPhase::Focus => {
                    self.sync_history_day();
                    let mins = (self.focus_secs / 60.0).round().max(1.0) as u32;
                    self.daily_focus_mins[6] = self.daily_focus_mins[6].saturating_add(mins);
                    save_focus_history(self.history_jdn, &self.daily_focus_mins);
                    self.completed_sessions = self.completed_sessions.saturating_add(1);
                    self.current_round = (self.current_round + 1).min(4);
                    self.phase = PomodoroPhase::Break;
                }
                PomodoroPhase::Break => {
                    if self.completed_sessions > 0 && self.completed_sessions % 4 == 0 {
                        self.current_round = 1;
                    }
                    self.phase = PomodoroPhase::Focus;
                }
            }
            self.running = false;
            self.remaining_secs = self.phase_duration_secs();
            return true;
        }
        false
    }

    pub fn phase_duration_secs(&self) -> f32 {
        match self.phase {
            PomodoroPhase::Focus => self.focus_secs,
            PomodoroPhase::Break => {
                if self.completed_sessions > 0 && self.completed_sessions % 4 == 0 {
                    self.long_break_secs
                } else {
                    self.break_secs
                }
            }
        }
    }

    pub fn total_round_mins(&self) -> u32 {
        let total_secs = 4.0 * self.focus_secs + 3.0 * self.break_secs + self.long_break_secs;
        (total_secs.max(0.0) / 60.0).round() as u32
    }

    pub fn progress(&self) -> f32 {
        let total = self.phase_duration_secs();
        if total <= 0.0 {
            return 0.0;
        }
        (1.0 - (self.remaining_secs / total)).clamp(0.0, 1.0)
    }

    pub fn formatted_remaining(&self) -> String {
        let secs = self.remaining_secs.max(0.0).ceil() as u32;
        format!("{:02}:{:02}", secs / 60, secs % 60)
    }

    pub fn display_phase_label(&self) -> &'static str {
        match self.phase {
            PomodoroPhase::Focus => "Focus",
            PomodoroPhase::Break => {
                if self.current_round >= 4 {
                    "Long break"
                } else {
                    "Short break"
                }
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WaterState {
    pub taken_ml: u32,
    pub goal_ml: u32,
    pub bottle_ml: u32,
    pub reminder_enabled: bool,
    pub show_gauge: bool,
    pub hourly_ml: [u32; 24],
}

impl Default for WaterState {
    fn default() -> Self {
        Self::new()
    }
}

impl WaterState {
    pub const DEFAULT_GOAL_ML: u32 = 2000;
    pub const DEFAULT_BOTTLE_ML: u32 = 250;

    pub fn new() -> Self {
        Self {
            taken_ml: 0,
            goal_ml: Self::DEFAULT_GOAL_ML,
            bottle_ml: Self::DEFAULT_BOTTLE_ML,
            reminder_enabled: false,
            show_gauge: false,
            hourly_ml: [0; 24],
        }
    }

    pub fn toggle_reminder(&mut self) {
        self.reminder_enabled = !self.reminder_enabled;
    }

    pub fn toggle_view(&mut self) {
        self.show_gauge = !self.show_gauge;
    }

    pub const fn window_start_hour_for(now_hour: u32) -> usize {
        let anchor = ((now_hour % 24) / 3) * 3;
        ((anchor as i32 - 3).rem_euclid(24)) as usize
    }

    pub fn visible_window(&self) -> ([u32; 8], [usize; 3]) {
        self.visible_window_for(current_local_hour())
    }

    pub fn visible_window_for(&self, now_hour: u32) -> ([u32; 8], [usize; 3]) {
        let start = Self::window_start_hour_for(now_hour);
        let mut bars = [0u32; 8];
        for (i, slot) in bars.iter_mut().enumerate() {
            *slot = self.hourly_ml[(start + i) % 24];
        }
        let labels = [start, (start + 3) % 24, (start + 6) % 24];
        (bars, labels)
    }

    pub fn add_drink(&mut self) -> u32 {
        self.add_amount(self.bottle_ml)
    }

    pub fn add_amount(&mut self, ml: u32) -> u32 {
        self.taken_ml = self.taken_ml.saturating_add(ml);
        let idx = (current_local_hour() % 24) as usize;
        self.hourly_ml[idx] = self.hourly_ml[idx].saturating_add(ml);
        self.taken_ml
    }

    #[allow(dead_code)]
    pub fn remove_drink(&mut self) -> u32 {
        let removed = self.taken_ml.min(self.bottle_ml);
        self.taken_ml = self.taken_ml.saturating_sub(self.bottle_ml);
        let idx = (current_local_hour() % 24) as usize;
        if self.hourly_ml[idx] >= removed {
            self.hourly_ml[idx] -= removed;
        } else {
            let mut rem = removed;
            for bucket in self.hourly_ml.iter_mut().rev() {
                let take = (*bucket).min(rem);
                *bucket -= take;
                rem -= take;
                if rem == 0 {
                    break;
                }
            }
        }
        self.taken_ml
    }

    pub fn reset_day(&mut self) {
        self.taken_ml = 0;
        self.hourly_ml = [0; 24];
    }

    pub fn progress(&self) -> f32 {
        if self.goal_ml == 0 {
            return 0.0;
        }
        (self.taken_ml as f32 / self.goal_ml as f32).clamp(0.0, 1.0)
    }

    #[allow(dead_code)]
    pub fn cups_taken(&self) -> u32 {
        if self.bottle_ml == 0 {
            return 0;
        }
        self.taken_ml / self.bottle_ml
    }
}

#[derive(Clone, Debug)]
pub struct TodoItem {
    pub text: String,
    pub done: bool,
    pub done_anim: f32,
    pub row_y_anim: f32,
    pub time_range: Option<String>,
    pub daily: bool,
    pub notified_day: Option<u32>,
    pub remind_again_at: Option<(u32, u16)>,
}

impl TodoItem {
    pub fn start_minutes(&self) -> Option<u16> {
        let raw = self.time_range.as_deref()?;
        let start = raw.split('-').next()?.trim();
        parse_hhmm(start)
    }

    pub fn end_minutes(&self) -> Option<u16> {
        let raw = self.time_range.as_deref()?;
        let end = raw.split('-').nth(1)?.trim();
        parse_hhmm(end)
    }
}

fn parse_hhmm(s: &str) -> Option<u16> {
    let mut parts = s.split(':');
    let hh: u16 = parts.next()?.parse().ok()?;
    let mm: u16 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || hh > 23 || mm > 59 {
        return None;
    }
    Some(hh * 60 + mm)
}

fn parse_scheduled_prefix(text: &str) -> (Option<String>, bool, &str) {
    if let Some(after_at) = text.strip_prefix('@') {
        if let Some((meta, rest)) = after_at.split_once(' ') {
            let mut meta_parts = meta.split('|');
            let time_part = meta_parts.next().unwrap_or("");
            let daily = meta_parts.next().map(|s| s == "daily").unwrap_or(false);
            let start_ok = time_part
                .split('-')
                .next()
                .and_then(parse_hhmm)
                .is_some();
            if start_ok && !rest.trim().is_empty() {
                return (Some(time_part.to_string()), daily, rest.trim());
            }
        }
    }
    (None, false, text)
}

impl PartialEq for TodoItem {
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text
            && self.done == other.done
            && self.time_range == other.time_range
            && self.daily == other.daily
    }
}

impl Eq for TodoItem {}

#[derive(Clone, Debug)]
pub struct TodoState {
    pub items: Vec<TodoItem>,
    pub editing: bool,
    pub adding: bool,
    pub expanded_list: bool,
    pub schedule_filter: bool,
    pub draft_title: String,
    pub draft_time: String,
    pub draft_daily: bool,
    pub draft_focus_time: bool,
    pub scroll_offset: usize,
    pub scroll_anim: f32,
    pub active_reminder_idx: Option<usize>,
    pub active_reminder_title: String,
    pub active_reminder_time: String,
    storage_path: PathBuf,
}

impl Default for TodoState {
    fn default() -> Self {
        Self::load_from(Self::resolve_path())
    }
}

impl TodoState {
    pub fn resolve_path() -> PathBuf {
        if let Ok(appdata) = env::var("APPDATA") {
            return PathBuf::from(appdata).join("eave").join("tasks.txt");
        }
        PathBuf::from("tasks.txt")
    }

    pub fn load_or_create() -> Self {
        let path = Self::resolve_path();
        if let Ok(contents) = fs::read_to_string(&path) {
            let parsed = Self::parse_str(&contents);
            if !Self::is_legacy_starter_list(&parsed) {
                return Self {
                    items: parsed,
                    editing: false,
                    adding: false,
                    expanded_list: false,
                    schedule_filter: false,
                    draft_title: String::new(),
                    draft_time: String::new(),
                    draft_daily: true,
                    draft_focus_time: false,
                    scroll_offset: 0,
                    scroll_anim: 0.0,
                    active_reminder_idx: None,
                    active_reminder_title: String::new(),
                    active_reminder_time: String::new(),
                    storage_path: path,
                };
            }
        }
        let state = Self {
            items: Vec::new(),
            editing: false,
            adding: false,
            expanded_list: false,
            schedule_filter: false,
            draft_title: String::new(),
            draft_time: String::new(),
            draft_daily: true,
            draft_focus_time: false,
            scroll_offset: 0,
            scroll_anim: 0.0,
            active_reminder_idx: None,
            active_reminder_title: String::new(),
            active_reminder_time: String::new(),
            storage_path: path,
        };
        let _ = state.save();
        state
    }

    pub fn load_from(path: impl Into<PathBuf>) -> Self {
        let storage_path = path.into();
        let items = fs::read_to_string(&storage_path)
            .map(|c| Self::parse_str(&c))
            .unwrap_or_default();
        Self {
            items,
            editing: false,
            adding: false,
            expanded_list: false,
            schedule_filter: false,
            draft_title: String::new(),
            draft_time: String::new(),
            draft_daily: true,
            draft_focus_time: false,
            scroll_offset: 0,
            scroll_anim: 0.0,
            active_reminder_idx: None,
            active_reminder_title: String::new(),
            active_reminder_time: String::new(),
            storage_path,
        }
    }

    fn is_legacy_starter_list(items: &[TodoItem]) -> bool {
        const LEGACY: [&str; 3] = [
            "Drop files onto Shelf",
            "Drink 2L of water",
            "Complete 25m focus sprint",
        ];
        items.len() == LEGACY.len()
            && items
                .iter()
                .zip(LEGACY.iter())
                .all(|(item, legacy)| item.text == *legacy)
    }

    pub fn parse_str(contents: &str) -> Vec<TodoItem> {
        let mut out = Vec::new();
        for raw_line in contents.lines() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(rest) = line.strip_prefix("[x]").or_else(|| line.strip_prefix("[X]")) {
                let text = rest.trim();
                if !text.is_empty() {
                    let (time_range, daily, clean_text) = parse_scheduled_prefix(text);
                    let row = out.len() as f32;
                    out.push(TodoItem {
                        text: clean_text.to_string(),
                        done: true,
                        done_anim: 1.0,
                        row_y_anim: row,
                        time_range,
                        daily,
                        notified_day: None,
                        remind_again_at: None,
                    });
                }
            } else if let Some(rest) = line.strip_prefix("[ ]") {
                let text = rest.trim();
                if !text.is_empty() {
                    let (time_range, daily, clean_text) = parse_scheduled_prefix(text);
                    let row = out.len() as f32;
                    out.push(TodoItem {
                        text: clean_text.to_string(),
                        done: false,
                        done_anim: 0.0,
                        row_y_anim: row,
                        time_range,
                        daily,
                        notified_day: None,
                        remind_again_at: None,
                    });
                }
            } else {
                let (time_range, daily, clean_text) = parse_scheduled_prefix(line);
                let row = out.len() as f32;
                out.push(TodoItem {
                    text: clean_text.to_string(),
                    done: false,
                    done_anim: 0.0,
                    row_y_anim: row,
                    time_range,
                    daily,
                    notified_day: None,
                    remind_again_at: None,
                });
            }
        }
        out
    }

    pub fn to_storage_string(&self) -> String {
        let mut out = String::from("# Eave tasks\n");
        for item in &self.items {
            let mark = if item.done { "x" } else { " " };
            if let Some(ref tr) = item.time_range {
                let mode = if item.daily { "daily" } else { "once" };
                out.push_str(&format!("[{}] @{}|{} {}\n", mark, tr, mode, item.text));
            } else {
                out.push_str(&format!("[{}] {}\n", mark, item.text));
            }
        }
        out
    }

    pub fn save(&self) -> io::Result<()> {
        save_text_file(&self.storage_path, &self.to_storage_string())
    }

    pub fn toggle_done(&mut self, idx: usize) -> bool {
        if let Some(item) = self.items.get_mut(idx) {
            item.done = !item.done;
            if item.done {
                item.remind_again_at = None;
            }
            let _ = self.save();
            true
        } else {
            false
        }
    }

    pub fn add_task(&mut self, text: impl Into<String>) {
        self.add_task_with_schedule(text, None, false);
    }

    pub fn add_task_with_schedule(
        &mut self,
        text: impl Into<String>,
        time_range: Option<String>,
        daily: bool,
    ) {
        let clean: String = text
            .into()
            .chars()
            .map(|c| if c == '\r' || c == '\n' { ' ' } else { c })
            .collect();
        let trimmed = clean.trim();
        if trimmed.is_empty() {
            return;
        }
        let row = self.items.len() as f32;
        self.items.push(TodoItem {
            text: trimmed.to_string(),
            done: false,
            done_anim: 0.0,
            row_y_anim: row,
            time_range,
            daily,
            notified_day: None,
            remind_again_at: None,
        });
        self.clamp_scroll();
        let _ = self.save();
    }

    pub fn visible_indices(&self) -> Vec<usize> {
        if !self.schedule_filter {
            return (0..self.items.len()).collect();
        }
        let mut sched: Vec<(u16, usize)> = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(i, it)| it.start_minutes().map(|m| (m, i)))
            .collect();
        sched.sort_by_key(|&(m, i)| (m, i));
        sched.into_iter().map(|(_, i)| i).collect()
    }

    pub fn visible_len(&self) -> usize {
        if !self.schedule_filter {
            self.items.len()
        } else {
            self.items
                .iter()
                .filter(|it| it.start_minutes().is_some())
                .count()
        }
    }

    pub fn has_scheduled_tasks(&self) -> bool {
        self.items.iter().any(|it| it.start_minutes().is_some())
    }

    pub fn toggle_schedule_filter(&mut self) {
        self.schedule_filter = !self.schedule_filter;
        self.adding = false;
        self.editing = false;
        self.scroll_offset = 0;
        self.clamp_scroll();
    }

    pub fn toggle_draft_daily(&mut self) {
        self.draft_daily = !self.draft_daily;
    }

    pub fn format_draft_time(&self, show_caret: bool) -> String {
        let digits: Vec<char> = self.draft_time.chars().collect();
        if digits.is_empty() {
            return if show_caret {
                "|__:__-__:__".to_string()
            } else {
                "__:__-__:__".to_string()
            };
        }
        let mut out = String::with_capacity(13);
        let slots = [0usize, 1, 2, 3, 4, 5, 6, 7];
        for (pos, &idx) in slots.iter().enumerate() {
            if pos == 2 || pos == 6 {
                out.push(':');
            } else if pos == 4 {
                out.push('-');
            }
            if idx < digits.len() {
                out.push(digits[idx]);
            } else {
                if show_caret && idx == digits.len() {
                    out.push('|');
                }
                out.push('_');
            }
        }
        if show_caret && digits.len() >= 8 {
            out.push('|');
        }
        out
    }

    pub fn parsed_draft_time(&self) -> Option<String> {
        let d: Vec<char> = self.draft_time.chars().collect();
        if d.len() < 4 {
            return None;
        }
        let start = format!("{}{}:{}{}", d[0], d[1], d[2], d[3]);
        parse_hhmm(&start)?;
        if d.len() >= 8 {
            let end = format!("{}{}:{}{}", d[4], d[5], d[6], d[7]);
            if parse_hhmm(&end).is_some() {
                return Some(format!("{start}-{end}"));
            }
        }
        Some(start)
    }

    pub fn check_due_reminder(
        &mut self,
        day_key: u32,
        now_minutes: u16,
        remind_start: bool,
        remind_end: bool,
        repeat_mins: u16,
    ) -> Option<(usize, String, String)> {
        if !remind_start && !remind_end {
            return None;
        }
        for (idx, item) in self.items.iter_mut().enumerate() {
            if item.done {
                continue;
            }
            if remind_start
                && item.notified_day != Some(day_key)
                && item.start_minutes() == Some(now_minutes)
            {
                item.notified_day = Some(day_key);
                let next_min = now_minutes.saturating_add(repeat_mins);
                item.remind_again_at = if next_min < 1440 {
                    Some((day_key, next_min))
                } else {
                    None
                };
                let title = item.text.clone();
                let time_label = item.time_range.clone().unwrap_or_default();
                self.active_reminder_idx = Some(idx);
                self.active_reminder_title = title.clone();
                self.active_reminder_time = time_label.clone();
                return Some((idx, title, time_label));
            }
            if remind_start && item.remind_again_at == Some((day_key, now_minutes)) {
                item.remind_again_at = None;
                let title = item.text.clone();
                let time_label = format!("Reminder · {}", item.time_range.as_deref().unwrap_or(""));
                self.active_reminder_idx = Some(idx);
                self.active_reminder_title = title.clone();
                self.active_reminder_time = time_label.clone();
                return Some((idx, title, time_label));
            }
            let end_key = day_key.saturating_add(1_000_000_000);
            if remind_end
                && item.notified_day != Some(end_key)
                && item.end_minutes() == Some(now_minutes)
            {
                item.notified_day = Some(end_key);
                item.remind_again_at = None;
                let title = item.text.clone();
                let time_label = format!("Time's up · {}", item.time_range.as_deref().unwrap_or(""));
                self.active_reminder_idx = Some(idx);
                self.active_reminder_title = title.clone();
                self.active_reminder_time = time_label.clone();
                return Some((idx, title, time_label));
            }
        }
        None
    }

    pub fn check_due_reminder_now(
        &mut self,
        remind_start: bool,
        remind_end: bool,
        repeat_mins: u16,
    ) -> Option<(usize, String, String)> {
        let mut st = SYSTEMTIME::default();
        unsafe { GetLocalTime(&mut st) };
        let day_key =
            (st.wYear as u32) * 10_000 + (st.wMonth as u32) * 100 + (st.wDay as u32);
        let now_minutes = st.wHour * 60 + st.wMinute;
        self.check_due_reminder(day_key, now_minutes, remind_start, remind_end, repeat_mins)
    }

    pub fn move_item(&mut self, from: usize, to: usize) -> bool {
        if from == to || from >= self.items.len() || to >= self.items.len() {
            return false;
        }
        let item = self.items.remove(from);
        self.items.insert(to, item);
        let _ = self.save();
        true
    }

    pub fn clamp_scroll(&mut self) {
        let max_off = self.visible_len().saturating_sub(3);
        if self.scroll_offset > max_off {
            self.scroll_offset = max_off;
        }
    }

    pub fn scroll_by(&mut self, delta: i32) -> bool {
        let max_off = self.visible_len().saturating_sub(3);
        let next = (self.scroll_offset as i32 + delta).clamp(0, max_off as i32) as usize;
        if next != self.scroll_offset {
            self.scroll_offset = next;
            true
        } else {
            false
        }
    }

    pub fn step_animations(&mut self, dt: f32) -> bool {
        let dt = dt.clamp(0.0, 0.1);
        let check_rate = 1.0 - (-10.5 * dt).exp();
        let move_rate = 1.0 - (-14.0 * dt).exp();
        let mut active = false;

        let target_scroll = self.scroll_offset as f32;
        let s_diff = target_scroll - self.scroll_anim;
        if s_diff.abs() <= 0.003 {
            self.scroll_anim = target_scroll;
        } else {
            self.scroll_anim += s_diff * move_rate;
            active = true;
        }

        let vis = self.visible_indices();
        let mut rank = vec![None; self.items.len()];
        for (pos, &idx) in vis.iter().enumerate() {
            if idx < rank.len() {
                rank[idx] = Some(pos as f32);
            }
        }

        for (idx, item) in self.items.iter_mut().enumerate() {
            let target_done = if item.done { 1.0 } else { 0.0 };
            let d_diff = target_done - item.done_anim;
            if d_diff.abs() <= 0.004 {
                item.done_anim = target_done;
            } else {
                item.done_anim = (item.done_anim + d_diff * check_rate).clamp(0.0, 1.0);
                active = true;
            }

            let target_row = rank[idx].unwrap_or(idx as f32);
            let r_diff = target_row - item.row_y_anim;
            if r_diff.abs() <= 0.003 {
                item.row_y_anim = target_row;
            } else {
                item.row_y_anim += r_diff * move_rate;
                active = true;
            }
        }
        active
    }

    pub fn add_sample_or_default(&mut self) {
        const SAMPLES: [&str; 4] = [
            "Quick stretch break",
            "Review open notes",
            "Refill water bottle",
            "Wrap up current task",
        ];
        let next_num = self.items.len() + 1;
        let label = SAMPLES
            .iter()
            .find(|s| !self.items.iter().any(|it| it.text == **s))
            .map(|s| (*s).to_string())
            .unwrap_or_else(|| format!("Quick task #{next_num}"));
        self.add_task(label);
    }

    pub fn toggle_expanded_list(&mut self) {
        if self.adding || self.items.is_empty() {
            self.expanded_list = false;
        } else {
            self.expanded_list = !self.expanded_list;
        }
    }

    pub fn start_adding(&mut self) {
        self.adding = true;
        self.editing = false;
        self.expanded_list = false;
        self.draft_title.clear();
        self.draft_time.clear();
        self.draft_daily = true;
        self.draft_focus_time = false;
    }

    pub fn cancel_adding(&mut self) {
        self.adding = false;
        self.draft_title.clear();
        self.draft_time.clear();
        self.draft_focus_time = false;
    }

    pub fn confirm_adding(&mut self) {
        let trimmed = self.draft_title.trim().to_string();
        let sched = self.parsed_draft_time();
        let daily = sched.is_some() && self.draft_daily;
        if !trimmed.is_empty() {
            self.add_task_with_schedule(trimmed, sched, daily);
        } else {
            self.add_sample_or_default();
        }
        self.adding = false;
        self.draft_title.clear();
        self.draft_time.clear();
        self.draft_focus_time = false;
    }

    pub fn toggle_editing(&mut self) {
        if self.items.is_empty() {
            self.editing = false;
        } else {
            self.editing = !self.editing;
            self.adding = false;
        }
    }

    pub fn push_draft_char(&mut self, ch: char) {
        if !self.adding || ch.is_control() {
            return;
        }
        if self.draft_focus_time {
            if ch.is_ascii_digit() && self.draft_time.len() < 8 {
                self.draft_time.push(ch);
            }
        } else if self.draft_title.chars().count() < 42 {
            self.draft_title.push(ch);
        }
    }

    pub fn pop_draft_char(&mut self) {
        if !self.adding {
            return;
        }
        if self.draft_focus_time {
            self.draft_time.pop();
        } else {
            self.draft_title.pop();
        }
    }

    pub fn remove_at(&mut self, idx: usize) -> Option<TodoItem> {
        if idx < self.items.len() {
            let removed = self.items.remove(idx);
            if self.items.is_empty() {
                self.editing = false;
            }
            self.clamp_scroll();
            let _ = self.save();
            Some(removed)
        } else {
            None
        }
    }

    pub fn clear_done(&mut self) {
        let before = self.items.len();
        self.items.retain(|it| !it.done);
        self.clamp_scroll();
        if self.items.len() != before {
            let _ = self.save();
        }
    }

    pub fn completed_count(&self) -> usize {
        self.items.iter().filter(|it| it.done).count()
    }
}

fn current_local_hour() -> u32 {
    let unix_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let intervals = (unix_secs + 11_644_473_600) * 10_000_000;
    let ft = FILETIME {
        dwLowDateTime: intervals as u32,
        dwHighDateTime: (intervals >> 32) as u32,
    };
    let mut utc_st = SYSTEMTIME::default();
    let mut local_st = SYSTEMTIME::default();
    unsafe {
        if FileTimeToSystemTime(&ft, &mut utc_st).is_ok()
            && SystemTimeToTzSpecificLocalTime(None, &utc_st, &mut local_st).is_ok()
        {
            return local_st.wHour as u32;
        }
    }
    12
}

fn save_text_file(path: &Path, content: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    fs::write(path, content)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notch_page_navigation_and_titles() {
        assert_eq!(NotchPage::ALL.len(), 6);
        for (i, page) in NotchPage::ALL.iter().enumerate() {
            assert_eq!(page.index(), i);
            assert_eq!(NotchPage::from_index(i), *page);
        }
        assert_eq!(NotchPage::from_index(99), NotchPage::Music);

        assert_eq!(NotchPage::Music.next(), NotchPage::Pomodoro);
        assert_eq!(NotchPage::Todo.next(), NotchPage::Widgets);
        assert_eq!(NotchPage::Widgets.next(), NotchPage::Music);
        assert_eq!(NotchPage::Music.prev(), NotchPage::Widgets);
        assert_eq!(NotchPage::Shelf.prev(), NotchPage::Water);

        assert_eq!(NotchPage::Music.title(), "Music");
        assert_eq!(NotchPage::Pomodoro.title(), "Pomodoro");
        assert_eq!(NotchPage::Water.title(), "Water");
        assert_eq!(NotchPage::Shelf.title(), "Shelf");
        assert_eq!(NotchPage::Todo.title(), "Todo");
        assert_eq!(NotchPage::Widgets.title(), "Widgets");
    }

    #[test]
    fn world_clock_dst_rules_eu_us_au() {
        let mk = |y, m, d, h| SYSTEMTIME {
            wYear: y,
            wMonth: m,
            wDay: d,
            wHour: h,
            wMinute: 0,
            ..Default::default()
        };
        let jan = mk(2026, 1, 15, 12);
        let jul = mk(2026, 7, 15, 12);
        assert_eq!(dst_offset_minutes(DstRule::Eu, &jan, 60), 0);
        assert_eq!(dst_offset_minutes(DstRule::Eu, &jul, 60), 60);
        assert_eq!(dst_offset_minutes(DstRule::Us, &jan, -300), 0);
        assert_eq!(dst_offset_minutes(DstRule::Us, &jul, -300), 60);
        assert_eq!(dst_offset_minutes(DstRule::Au, &jan, 600), 60);
        assert_eq!(dst_offset_minutes(DstRule::Au, &jul, 600), 0);
        assert_eq!(dst_offset_minutes(DstRule::None, &jul, 540), 0);
    }

    #[test]
    fn pomodoro_state_lifecycle() {
        let mut p = PomodoroState::new();
        assert_eq!(p.phase, PomodoroPhase::Focus);
        assert!(!p.running);
        assert_eq!(p.current_round, 1);
        assert_eq!(p.long_break_secs, 15.0 * 60.0);
        assert_eq!(p.total_round_mins(), 130);
        assert_eq!(p.formatted_remaining(), "25:00");
        assert_eq!(p.progress(), 0.0);

        assert!(!p.tick(10.0));
        assert_eq!(p.formatted_remaining(), "25:00");

        p.toggle();
        assert!(p.running);
        assert!(!p.tick(60.0));
        assert_eq!(p.formatted_remaining(), "24:00");
        assert!((p.progress() - (60.0 / 1500.0)).abs() < 1e-4);

        assert!(p.tick(1440.0));
        assert_eq!(p.phase, PomodoroPhase::Break);
        assert!(!p.running);
        assert_eq!(p.completed_sessions, 1);
        assert_eq!(p.current_round, 2);
        assert_eq!(p.formatted_remaining(), "05:00");

        p.toggle();
        assert!(!p.tick(120.0));
        assert_eq!(p.formatted_remaining(), "03:00");
        p.reset();
        assert!(!p.running);
        assert_eq!(p.formatted_remaining(), "05:00");

        for expected_round in [3, 4, 4] {
            p.toggle();
            assert!(p.tick(300.0));
            assert_eq!(p.phase, PomodoroPhase::Focus);
            p.toggle();
            assert!(p.tick(1500.0));
            assert_eq!(p.current_round, expected_round);
        }
        assert_eq!(p.completed_sessions, 4);
        assert_eq!(p.phase, PomodoroPhase::Break);
        assert_eq!(p.formatted_remaining(), "15:00");
    }

    #[test]
    fn water_state_tracking() {
        let mut w = WaterState::new();
        assert_eq!(w.taken_ml, 0);
        assert_eq!(w.goal_ml, 2000);
        assert_eq!(w.bottle_ml, 250);
        assert!(!w.reminder_enabled);
        assert_eq!(w.hourly_ml, [0; 24]);
        assert_eq!(w.progress(), 0.0);

        w.toggle_reminder();
        assert!(w.reminder_enabled);
        assert_eq!(WaterState::window_start_hour_for(14), 9);
        assert_eq!(WaterState::window_start_hour_for(15), 12);
        assert_eq!(WaterState::window_start_hour_for(22), 18);
        assert_eq!(WaterState::window_start_hour_for(1), 21);
        let (_, labels_14) = w.visible_window_for(14);
        assert_eq!(labels_14, [9, 12, 15]);
        let (_, labels_16) = w.visible_window_for(16);
        assert_eq!(labels_16, [12, 15, 18]);

        assert_eq!(w.add_drink(), 250);
        assert_eq!(w.add_drink(), 500);
        assert_eq!(w.cups_taken(), 2);
        assert!((w.progress() - 0.25).abs() < 1e-5);
        assert_eq!(w.hourly_ml.iter().sum::<u32>(), 500);

        assert_eq!(w.remove_drink(), 250);
        assert_eq!(w.hourly_ml.iter().sum::<u32>(), 250);
        w.add_amount(3000);
        assert_eq!(w.progress(), 1.0);

        w.reset_day();
        assert_eq!(w.taken_ml, 0);
        assert_eq!(w.hourly_ml, [0; 24]);
        assert_eq!(w.remove_drink(), 0);
    }

    #[test]
    fn todo_state_roundtrip() {
        let unique = std::process::id();
        let path = env::temp_dir().join(format!("eave_todo_test_{unique}.txt"));
        let _ = fs::remove_file(&path);

        let mut todo = TodoState::load_from(&path);
        assert!(todo.items.is_empty());
        assert!(!todo.adding);
        assert!(!todo.editing);
        assert!(todo.draft_title.is_empty());

        let legacy = TodoState::parse_str(
            "[ ] Drop files onto Shelf\n[ ] Drink 2L of water\n[ ] Complete 25m focus sprint\n",
        );
        assert!(TodoState::is_legacy_starter_list(&legacy));

        todo.toggle_editing();
        assert!(!todo.editing);

        todo.start_adding();
        assert!(todo.adding);
        for ch in "Buy milk!".chars() {
            todo.push_draft_char(ch);
        }
        todo.push_draft_char('\n');
        todo.pop_draft_char();
        assert_eq!(todo.draft_title, "Buy milk");
        todo.confirm_adding();
        assert!(!todo.adding);
        assert!(todo.draft_title.is_empty());
        assert_eq!(todo.items.len(), 1);
        assert_eq!(todo.items[0].text, "Buy milk");

        todo.start_adding();
        todo.push_draft_char('X');
        todo.cancel_adding();
        assert!(!todo.adding);
        assert!(todo.draft_title.is_empty());
        assert_eq!(todo.items.len(), 1);

        assert!(todo.toggle_done(0));
        assert_eq!(todo.completed_count(), 1);

        let reloaded = TodoState::load_from(&path);
        assert_eq!(reloaded.items, todo.items);

        todo.toggle_editing();
        assert!(todo.editing);
        assert!(todo.remove_at(0).is_some());
        assert!(todo.items.is_empty());
        assert!(!todo.editing);

        let _ = fs::remove_file(&path);
    }
}
