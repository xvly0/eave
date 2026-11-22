
use std::collections::HashMap;
use std::ffi::{c_char, c_int, c_void, CStr, CString, OsStr};
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime};

use windows::core::{w, Interface, PCSTR, PCWSTR, PWSTR};
use windows::Graphics::Imaging::{
    BitmapAlphaMode, BitmapDecoder, BitmapPixelFormat, BitmapTransform,
    ColorManagementMode, ExifOrientationMode,
};
use windows::Storage::Streams::{DataWriter, InMemoryRandomAccessStream};
use windows::Win32::Foundation::{ERROR_SUCCESS, HANDLE, HMODULE, SIZE};
use windows::Win32::Graphics::Gdi::{
    DeleteObject, GetDC, GetDIBits, GetObjectW, ReleaseDC, BITMAP, BITMAPINFO, BITMAPINFOHEADER,
    BI_RGB, DIB_RGB_COLORS, HBITMAP,
};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::LibraryLoader::{
    GetProcAddress as WinGetProcAddress, LoadLibraryW as WinLoadLibraryW,
};
use windows::Win32::System::Registry::{
    RegCloseKey, RegEnumKeyExW, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER,
    KEY_READ, REG_QWORD, REG_VALUE_TYPE,
};
use windows::Win32::UI::Shell::{
    IShellItem, IShellItemImageFactory, SHCreateItemFromParsingName, SIGDN_NORMALDISPLAY,
    SIIGBF_BIGGERSIZEOK, SIIGBF_ICONONLY,
};

const IOCTL_VIDEO_QUERY_DISPLAY_BRIGHTNESS: u32 = 0x0023_0498;
const OPEN_EXISTING: u32 = 3;
const FILE_SHARE_READ: u32 = 1;
const FILE_SHARE_WRITE: u32 = 2;
const GENERIC_READ: u32 = 0x8000_0000;
const QDC_ONLY_ACTIVE_PATHS: u32 = 0x0000_0002;
const DISPLAYCONFIG_DEVICE_INFO_GET_SDR_WHITE_LEVEL: i32 = 11;
const IF_TYPE_ETHERNET_CSMACD: u32 = 6;
const IF_TYPE_IEEE80211: u32 = 71;
const IF_OPER_STATUS_CONNECTED: u32 = 4;
const IF_OPER_STATUS_OPERATIONAL: u32 = 5;
const WLAN_INTF_OPCODE_CURRENT_CONNECTION: u32 = 7;

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct Luid {
    low_part: u32,
    high_part: i32,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct DisplayConfigPathSourceInfo {
    adapter_id: Luid,
    id: u32,
    mode_info_idx: u32,
    status_flags: u32,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct DisplayConfigPathTargetInfo {
    adapter_id: Luid,
    id: u32,
    mode_info_idx: u32,
    output_technology: u32,
    rotation: u32,
    scaling: u32,
    refresh_num: u32,
    refresh_den: u32,
    scan_line_ordering: u32,
    target_available: i32,
    status_flags: u32,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct DisplayConfigPathInfo {
    source_info: DisplayConfigPathSourceInfo,
    target_info: DisplayConfigPathTargetInfo,
    flags: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct DisplayConfigModeInfoRaw {
    _bytes: [u8; 64],
}

impl Default for DisplayConfigModeInfoRaw {
    fn default() -> Self {
        Self { _bytes: [0u8; 64] }
    }
}

#[repr(C)]
#[derive(Default)]
struct DisplayConfigDeviceInfoHeader {
    ty: i32,
    size: u32,
    adapter_id: Luid,
    id: u32,
}

#[repr(C)]
#[derive(Default)]
struct DisplayConfigSdrWhiteLevel {
    header: DisplayConfigDeviceInfoHeader,
    sdr_white_level: u32,
}

#[repr(C)]
#[derive(Default)]
struct DisplayBrightness {
    uc_display_policy: u8,
    uc_ac_brightness: u8,
    uc_dc_brightness: u8,
}

#[repr(C)]
#[derive(Default)]
struct SystemPowerStatus {
    ac_line_status: u8,
    battery_flag: u8,
    battery_life_percent: u8,
    system_status_flag: u8,
    battery_life_time: u32,
    battery_full_life_time: u32,
}

#[repr(C)]
struct MibIfRow {
    wsz_name: [u16; 256],
    dw_index: u32,
    dw_type: u32,
    dw_mtu: u32,
    dw_speed: u32,
    dw_phys_addr_len: u32,
    b_phys_addr: [u8; 8],
    dw_admin_status: u32,
    dw_oper_status: u32,
    dw_last_change: u32,
    dw_in_octets: u32,
    dw_in_ucast_pkts: u32,
    dw_in_nucast_pkts: u32,
    dw_in_discards: u32,
    dw_in_errors: u32,
    dw_in_unknown_protos: u32,
    dw_out_octets: u32,
    dw_out_ucast_pkts: u32,
    dw_out_nucast_pkts: u32,
    dw_out_discards: u32,
    dw_out_errors: u32,
    dw_out_qlen: u32,
    dw_descr_len: u32,
    b_descr: [u8; 256],
}

impl Default for MibIfRow {
    fn default() -> Self {
        // Safety: all-zero MibIfRow is valid.
        unsafe { std::mem::zeroed() }
    }
}

#[link(name = "kernel32")]
extern "system" {
    fn CreateFileW(
        lp_file_name: *const u16,
        dw_desired_access: u32,
        dw_share_mode: u32,
        lp_security_attributes: *const c_void,
        dw_creation_disposition: u32,
        dw_flags_and_attributes: u32,
        h_template_file: HANDLE,
    ) -> HANDLE;
    fn DeviceIoControl(
        h_device: HANDLE,
        dw_io_control_code: u32,
        lp_in_buffer: *const c_void,
        n_in_buffer_size: u32,
        lp_out_buffer: *mut c_void,
        n_out_buffer_size: u32,
        lp_bytes_returned: *mut u32,
        lp_overlapped: *mut c_void,
    ) -> i32;
    fn CloseHandle(h_object: HANDLE) -> i32;
    fn GetSystemPowerStatus(lp_system_power_status: *mut SystemPowerStatus) -> i32;
    fn LoadLibraryW(lp_lib_file_name: *const u16) -> *mut c_void;
    fn GetProcAddress(h_module: *mut c_void, lp_proc_name: *const u8) -> *const c_void;
    fn FreeLibrary(h_lib_module: *mut c_void) -> i32;
}

#[link(name = "user32")]
extern "system" {
    fn GetDisplayConfigBufferSizes(
        flags: u32,
        num_path_array_elements: *mut u32,
        num_mode_info_array_elements: *mut u32,
    ) -> i32;
    fn QueryDisplayConfig(
        flags: u32,
        num_path_array_elements: *mut u32,
        path_array: *mut DisplayConfigPathInfo,
        num_mode_info_array_elements: *mut u32,
        mode_info_array: *mut DisplayConfigModeInfoRaw,
        current_topology_id: *mut c_void,
    ) -> i32;
    fn DisplayConfigGetDeviceInfo(request_packet: *mut DisplayConfigDeviceInfoHeader) -> i32;
}

#[link(name = "iphlpapi")]
extern "system" {
    fn GetBestInterface(dw_dest_addr: u32, pdw_best_if_index: *mut u32) -> u32;
    fn GetIfEntry(p_if_row: *mut MibIfRow) -> u32;
}

#[link(name = "winmm")]
extern "system" {
    fn PlaySoundW(psz_sound: *const u16, hmod: *mut c_void, fdw_sound: u32) -> i32;
}

const SND_ASYNC: u32 = 0x0001;
const SND_NODEFAULT: u32 = 0x0002;
const SND_MEMORY: u32 = 0x0004;

pub fn build_chime_wav() -> Vec<u8> {
    const SAMPLE_RATE: u32 = 44_100;
    const DURATION_SECS: f32 = 3.15;
    let num_samples = (SAMPLE_RATE as f32 * DURATION_SECS) as usize;
    let data_bytes = (num_samples * 2) as u32;
    let mut wav = Vec::with_capacity(44 + data_bytes as usize);

    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_bytes).to_le_bytes());
    wav.extend_from_slice(b"WAVE");
    wav.extend_from_slice(b"fmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    wav.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_bytes.to_le_bytes());

    const BELL_PARTIALS: [(f32, f32, f32); 4] = [
        (1.0, 1.0, 1.0),
        (2.76, 0.45, 0.55),
        (5.40, 0.22, 0.35),
        (8.93, 0.10, 0.20),
    ];
    const BASS_PARTIALS: [(f32, f32, f32); 2] = [(1.0, 1.0, 1.0), (2.0, 0.30, 0.50)];

    let mut tones: Vec<(f32, f32, f32, f32)> = Vec::with_capacity(20);
    let arp_freqs = [523.25_f32, 659.25, 783.99, 1046.50];
    for (i, &base_f) in arp_freqs.iter().enumerate() {
        let start = i as f32 * 0.16;
        let decay = if i == arp_freqs.len() - 1 { 2.6 } else { 1.4 };
        for &(ratio, amp, life) in &BELL_PARTIALS {
            tones.push((start, base_f * ratio, 0.17 * amp, decay * life));
        }
    }
    for &(ratio, amp, life) in &BASS_PARTIALS {
        tones.push((0.0, 261.63 * ratio, 0.16 * amp, 3.0 * life));
    }

    let tau = std::f32::consts::TAU;
    let attack = 0.002_f32;
    let hold = 0.002_f32;
    let master_gain = 0.49_f32;
    let min_g = 0.0001_f32;

    for i in 0..num_samples {
        let t = i as f32 / SAMPLE_RATE as f32;
        let mut mix = 0.0_f32;
        for &(start, freq, g, rel) in &tones {
            if t < start {
                continue;
            }
            let dt = t - start;
            if dt > hold + rel {
                continue;
            }
            let env = if dt < attack {
                min_g * (g / min_g).powf(dt / attack)
            } else if dt < hold {
                g
            } else {
                let u = ((dt - hold) / rel).clamp(0.0, 1.0);
                g * (min_g / g).powf(u)
            };
            mix += (tau * freq * dt).sin() * env;
        }

        let fade_out = ((DURATION_SECS - t) / 0.025).clamp(0.0, 1.0);
        let driven = mix * master_gain * fade_out;
        let compressed = (driven * 1.15).tanh() / 1.15;
        let pcm = (compressed.clamp(-0.98, 0.98) * i16::MAX as f32) as i16;
        wav.extend_from_slice(&pcm.to_le_bytes());
    }
    wav
}

pub fn play_eave_chime() {
    static CHIME_WAV: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    let wav = CHIME_WAV.get_or_init(build_chime_wav);
    unsafe {
        let _ = PlaySoundW(
            wav.as_ptr() as *const u16,
            std::ptr::null_mut(),
            SND_MEMORY | SND_ASYNC | SND_NODEFAULT,
        );
    }
}

pub fn sdr_white_level_to_pct(sdr_white_level: u32) -> u8 {
    ((sdr_white_level.saturating_sub(1000) + 25) / 50).min(100) as u8
}

fn query_hdr_sdr_brightness_pct() -> Option<u8> {
    let mut num_paths = 0u32;
    let mut num_modes = 0u32;
    if unsafe {
        GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut num_paths, &mut num_modes)
    } != 0
        || num_paths == 0
    {
        return None;
    }
    let mut paths = vec![DisplayConfigPathInfo::default(); num_paths as usize];
    let mut modes = vec![DisplayConfigModeInfoRaw::default(); num_modes as usize];
    if unsafe {
        QueryDisplayConfig(
            QDC_ONLY_ACTIVE_PATHS,
            &mut num_paths,
            paths.as_mut_ptr(),
            &mut num_modes,
            modes.as_mut_ptr(),
            std::ptr::null_mut(),
        )
    } != 0
    {
        return None;
    }
    for path in paths.iter().take(num_paths as usize) {
        let mut req = DisplayConfigSdrWhiteLevel {
            header: DisplayConfigDeviceInfoHeader {
                ty: DISPLAYCONFIG_DEVICE_INFO_GET_SDR_WHITE_LEVEL,
                size: std::mem::size_of::<DisplayConfigSdrWhiteLevel>() as u32,
                adapter_id: path.target_info.adapter_id,
                id: path.target_info.id,
            },
            sdr_white_level: 0,
        };
        if unsafe { DisplayConfigGetDeviceInfo(&mut req.header) } == 0
            && (1000..=6000).contains(&req.sdr_white_level)
        {
            return Some(sdr_white_level_to_pct(req.sdr_white_level));
        }
    }
    None
}

pub struct BrightnessMonitor {
    lcd_handle: Option<HANDLE>,
    use_hdr_sdr: bool,
    supported: bool,
    last_pct: Option<u8>,
    last_poll: Instant,
}

impl BrightnessMonitor {
    pub fn new() -> Self {
        let handle = unsafe {
            let h = CreateFileW(
                w!("\\\\.\\LCD").as_ptr(),
                GENERIC_READ,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                std::ptr::null(),
                OPEN_EXISTING,
                0,
                HANDLE(std::ptr::null_mut()),
            );
            if h.0.is_null() || h.0 == (-1isize as *mut c_void) {
                None
            } else {
                Some(h)
            }
        };
        let (use_hdr_sdr, init_pct) = if handle.is_some() {
            (false, None)
        } else {
            match query_hdr_sdr_brightness_pct() {
                Some(p) => (true, Some(p)),
                None => (false, None),
            }
        };
        let mut mon = Self {
            supported: handle.is_some() || use_hdr_sdr,
            lcd_handle: handle,
            use_hdr_sdr,
            last_pct: init_pct,
            last_poll: Instant::now(),
        };
        if mon.lcd_handle.is_some() {
            mon.last_pct = mon.read_pct();
        }
        mon
    }

    fn read_pct(&self) -> Option<u8> {
        if let Some(h) = self.lcd_handle {
            let mut db = DisplayBrightness::default();
            let mut returned: u32 = 0;
            let ok = unsafe {
                DeviceIoControl(
                    h,
                    IOCTL_VIDEO_QUERY_DISPLAY_BRIGHTNESS,
                    std::ptr::null(),
                    0,
                    &mut db as *mut _ as *mut c_void,
                    std::mem::size_of::<DisplayBrightness>() as u32,
                    &mut returned,
                    std::ptr::null_mut(),
                )
            };
            if ok == 0 {
                return None;
            }
            let pct = match db.uc_display_policy {
                1 => db.uc_ac_brightness,
                2 => db.uc_dc_brightness,
                _ => db.uc_ac_brightness.max(db.uc_dc_brightness),
            };
            return Some(pct.min(100));
        }
        if self.use_hdr_sdr {
            return query_hdr_sdr_brightness_pct();
        }
        None
    }

    pub fn poll_changed(&mut self) -> Option<u8> {
        if !self.supported || self.last_poll.elapsed().as_millis() < 180 {
            return None;
        }
        self.last_poll = Instant::now();
        let cur = self.read_pct()?;
        if self.last_pct == Some(cur) {
            return None;
        }
        self.last_pct = Some(cur);
        Some(cur)
    }

    pub fn current_pct(&self) -> u8 {
        self.last_pct.unwrap_or(100)
    }
}

impl Drop for BrightnessMonitor {
    fn drop(&mut self) {
        if let Some(h) = self.lcd_handle.take() {
            unsafe {
                let _ = CloseHandle(h);
            }
        }
    }
}

pub struct PowerChargeMonitor {
    last_ac_online: Option<bool>,
}

impl PowerChargeMonitor {
    pub fn new() -> Self {
        let initial = Self::query().map(|(ac, _)| ac);
        Self {
            last_ac_online: initial,
        }
    }

    fn query() -> Option<(bool, u8)> {
        let mut sps = SystemPowerStatus::default();
        let ok = unsafe { GetSystemPowerStatus(&mut sps) };
        if ok == 0 {
            return None;
        }
        if (sps.battery_flag & 128) != 0 || sps.battery_life_percent > 100 {
            return None;
        }
        let ac_online = sps.ac_line_status == 1;
        Some((ac_online, sps.battery_life_percent.min(100)))
    }

    pub fn poll_changed(&mut self) -> Option<(bool, u8)> {
        let (ac, pct) = Self::query()?;
        if let Some(prev) = self.last_ac_online {
            if prev != ac {
                self.last_ac_online = Some(ac);
                return Some((ac, pct));
            }
        } else {
            self.last_ac_online = Some(ac);
        }
        None
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ToastNotification {
    pub app: String,
    pub title: String,
    pub body: String,
    pub accent_rgb: (f32, f32, f32),
    pub icon_bgra: Option<Vec<u8>>,
    pub icon_w: u32,
    pub icon_h: u32,
    pub icon_is_circle: bool,
}

#[derive(Clone, Debug)]
struct CachedAppInfo {
    display_name: String,
    accent_rgb: (f32, f32, f32),
    icon: Option<(Vec<u8>, u32, u32)>,
}

struct WinSqlite3 {
    _lib: HMODULE,
    open_v2: unsafe extern "C" fn(*const c_char, *mut *mut c_void, c_int, *const c_char) -> c_int,
    close_v2: unsafe extern "C" fn(*mut c_void) -> c_int,
    busy_timeout: unsafe extern "C" fn(*mut c_void, c_int) -> c_int,
    prepare_v2: unsafe extern "C" fn(
        *mut c_void,
        *const c_char,
        c_int,
        *mut *mut c_void,
        *mut *const c_char,
    ) -> c_int,
    step: unsafe extern "C" fn(*mut c_void) -> c_int,
    column_int64: unsafe extern "C" fn(*mut c_void, c_int) -> i64,
    column_blob: unsafe extern "C" fn(*mut c_void, c_int) -> *const c_void,
    column_bytes: unsafe extern "C" fn(*mut c_void, c_int) -> c_int,
    column_text: unsafe extern "C" fn(*mut c_void, c_int) -> *const u8,
    finalize: unsafe extern "C" fn(*mut c_void) -> c_int,
}

impl WinSqlite3 {
    fn load() -> Option<Self> {
        unsafe {
            let lib = WinLoadLibraryW(w!("winsqlite3.dll")).ok()?;
            macro_rules! sym {
                ($name:literal) => {{
                    let p = WinGetProcAddress(lib, PCSTR(concat!($name, "\0").as_ptr()))?;
                    std::mem::transmute(p)
                }};
            }
            Some(Self {
                _lib: lib,
                open_v2: sym!("sqlite3_open_v2"),
                close_v2: sym!("sqlite3_close_v2"),
                busy_timeout: sym!("sqlite3_busy_timeout"),
                prepare_v2: sym!("sqlite3_prepare_v2"),
                step: sym!("sqlite3_step"),
                column_int64: sym!("sqlite3_column_int64"),
                column_blob: sym!("sqlite3_column_blob"),
                column_bytes: sym!("sqlite3_column_bytes"),
                column_text: sym!("sqlite3_column_text"),
                finalize: sym!("sqlite3_finalize"),
            })
        }
    }

    fn query_max_toast_id(&self, db_path: &Path) -> Option<i64> {
        let path_c = CString::new(db_path.to_string_lossy().as_bytes()).ok()?;
        let sql_c = CString::new("SELECT COALESCE(MAX(Id), 0) FROM Notification WHERE Type = 'toast'").ok()?;
        unsafe {
            let mut db: *mut c_void = std::ptr::null_mut();
            // SQLITE_OPEN_READONLY = 1
            if (self.open_v2)(path_c.as_ptr(), &mut db, 1, std::ptr::null()) != 0 || db.is_null() {
                if !db.is_null() {
                    (self.close_v2)(db);
                }
                return None;
            }
            (self.busy_timeout)(db, 120);
            let mut stmt: *mut c_void = std::ptr::null_mut();
            let mut out = None;
            if (self.prepare_v2)(db, sql_c.as_ptr(), -1, &mut stmt, std::ptr::null_mut()) == 0
                && !stmt.is_null()
            {
                if (self.step)(stmt) == 100 {
                    out = Some((self.column_int64)(stmt, 0));
                }
                (self.finalize)(stmt);
            }
            (self.close_v2)(db);
            out
        }
    }

    fn query_latest_toast_after(
        &self,
        db_path: &Path,
        after_id: i64,
    ) -> Option<(i64, Vec<u8>, String)> {
        let path_c = CString::new(db_path.to_string_lossy().as_bytes()).ok()?;
        let sql = format!(
            "SELECT n.Id, n.Payload, COALESCE(h.PrimaryId, '') \
             FROM Notification n \
             LEFT JOIN NotificationHandler h ON n.HandlerId = h.RecordId \
             WHERE n.Id > {after_id} AND n.Type = 'toast' \
             ORDER BY n.Id DESC LIMIT 1"
        );
        let sql_c = CString::new(sql).ok()?;
        unsafe {
            let mut db: *mut c_void = std::ptr::null_mut();
            if (self.open_v2)(path_c.as_ptr(), &mut db, 1, std::ptr::null()) != 0 || db.is_null() {
                if !db.is_null() {
                    (self.close_v2)(db);
                }
                return None;
            }
            (self.busy_timeout)(db, 120);
            let mut stmt: *mut c_void = std::ptr::null_mut();
            let mut out = None;
            if (self.prepare_v2)(db, sql_c.as_ptr(), -1, &mut stmt, std::ptr::null_mut()) == 0
                && !stmt.is_null()
            {
                if (self.step)(stmt) == 100 {
                    let id = (self.column_int64)(stmt, 0);
                    let blob_ptr = (self.column_blob)(stmt, 1);
                    let blob_len = (self.column_bytes)(stmt, 1).max(0) as usize;
                    let payload = if !blob_ptr.is_null() && blob_len > 0 {
                        std::slice::from_raw_parts(blob_ptr as *const u8, blob_len).to_vec()
                    } else {
                        Vec::new()
                    };
                    let txt_ptr = (self.column_text)(stmt, 2);
                    let primary_id = if !txt_ptr.is_null() {
                        CStr::from_ptr(txt_ptr as *const c_char)
                            .to_string_lossy()
                            .into_owned()
                    } else {
                        String::new()
                    };
                    out = Some((id, payload, primary_id));
                }
                (self.finalize)(stmt);
            }
            (self.close_v2)(db);
            out
        }
    }
}

pub struct ToastNotificationMonitor {
    db_path: Option<PathBuf>,
    wal_path: Option<PathBuf>,
    sqlite: Option<WinSqlite3>,
    last_notif_id: i64,
    last_wal_stamp: Option<(u64, Option<SystemTime>)>,
    last_poll: Instant,
    app_icon_cache: HashMap<String, CachedAppInfo>,
}

impl ToastNotificationMonitor {
    pub fn new() -> Self {
        let base_dir = std::env::var_os("LOCALAPPDATA").map(|la| {
            PathBuf::from(la)
                .join("Microsoft")
                .join("Windows")
                .join("Notifications")
        });
        let db_path = base_dir.as_ref().map(|d| d.join("wpndatabase.db"));
        let wal_path = base_dir.as_ref().map(|d| d.join("wpndatabase.db-wal"));
        let sqlite = WinSqlite3::load();
        let last_notif_id = sqlite
            .as_ref()
            .zip(db_path.as_ref())
            .and_then(|(s, p)| s.query_max_toast_id(p))
            .unwrap_or(0);
        let last_wal_stamp = wal_path.as_ref().and_then(|p| file_stamp(p));
        Self {
            db_path,
            wal_path,
            sqlite,
            last_notif_id,
            last_wal_stamp,
            last_poll: Instant::now(),
            app_icon_cache: HashMap::new(),
        }
    }

    pub fn poll_new(&mut self) -> Option<ToastNotification> {
        if self.last_poll.elapsed().as_millis() < 450 {
            return None;
        }
        self.last_poll = Instant::now();

        let wal_stamp = self.wal_path.as_ref().and_then(|p| file_stamp(p));
        if wal_stamp.is_some() && wal_stamp == self.last_wal_stamp {
            return None;
        }
        self.last_wal_stamp = wal_stamp;

        let sqlite = self.sqlite.as_ref()?;
        let db_path = self.db_path.as_ref()?;
        let (id, payload, primary_id) = sqlite.query_latest_toast_after(db_path, self.last_notif_id)?;
        if id <= self.last_notif_id {
            return None;
        }
        self.last_notif_id = id;

        let xml = String::from_utf8_lossy(&payload);
        let (title, body) = extract_toast_texts(&xml)?;

        let app_key = if primary_id.is_empty() {
            detect_toast_app(&xml).0
        } else {
            primary_id.clone()
        };
        let info = self
            .app_icon_cache
            .entry(app_key)
            .or_insert_with(|| resolve_app_info(&primary_id, &xml))
            .clone();

        let mut icon_bgra = None;
        let mut icon_w = 0;
        let mut icon_h = 0;
        let mut icon_is_circle = false;

        // Tier 1: explicit appLogoOverride avatar/icon from XML (never hero/screenshot images).
        if let Some((img_path, is_circle)) = extract_toast_logo_override(&xml) {
            if let Some((bgra, w, h)) = decode_local_image_to_bgra(&img_path) {
                icon_bgra = Some(bgra);
                icon_w = w;
                icon_h = h;
                icon_is_circle = is_circle;
            }
        }
        // Tier 2: native Windows Shell icon from exact AUMID (NotificationHandler.PrimaryId).
        if icon_bgra.is_none() {
            if let Some((bgra, w, h)) = info.icon {
                icon_bgra = Some(bgra);
                icon_w = w;
                icon_h = h;
                icon_is_circle = false;
            }
        }

        Some(ToastNotification {
            app: info.display_name,
            title,
            body,
            accent_rgb: info.accent_rgb,
            icon_bgra,
            icon_w,
            icon_h,
            icon_is_circle,
        })
    }
}

fn file_stamp(path: &Path) -> Option<(u64, Option<SystemTime>)> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.len(), meta.modified().ok()))
}

#[cfg(test)]
pub fn extract_latest_toast_from_wal(buf: &[u8]) -> Option<ToastNotification> {
    if buf.len() < 32 {
        return None;
    }
    let magic = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]);
    if magic != 0x377F_0682 && magic != 0x377F_0683 {
        return None;
    }
    let page_size = u32::from_be_bytes([buf[8], buf[9], buf[10], buf[11]]) as usize;
    if !(512..=65536).contains(&page_size) || !page_size.is_power_of_two() {
        return None;
    }
    let wal_salt1 = u32::from_be_bytes([buf[16], buf[17], buf[18], buf[19]]);
    let wal_salt2 = u32::from_be_bytes([buf[20], buf[21], buf[22], buf[23]]);
    let frame_stride = 24 + page_size;
    let mut valid_frames: Vec<&[u8]> = Vec::new();
    let mut off = 32usize;
    while off + frame_stride <= buf.len() {
        let fh = &buf[off..off + 24];
        let s1 = u32::from_be_bytes([fh[8], fh[9], fh[10], fh[11]]);
        let s2 = u32::from_be_bytes([fh[12], fh[13], fh[14], fh[15]]);
        if s1 != wal_salt1 || s2 != wal_salt2 {
            break;
        }
        valid_frames.push(&buf[off + 24..off + frame_stride]);
        off += frame_stride;
    }
    for idx in (0..valid_frames.len()).rev() {
        if let Some(t) = extract_latest_toast_from_bytes(valid_frames[idx]) {
            return Some(t);
        }
    }
    None
}

pub fn extract_latest_toast_from_bytes(buf: &[u8]) -> Option<ToastNotification> {
    let text = String::from_utf8_lossy(buf);
    let end_idx = text.rfind("</toast>")?;
    let before = &text[..end_idx];
    let start_idx = before.rfind("<toast")?;
    let xml = &text[start_idx..end_idx + "</toast>".len()];
    let (title, body) = extract_toast_texts(xml)?;
    let context_start = start_idx.saturating_sub(260);
    let context = &text[context_start..end_idx + "</toast>".len()];
    let (app, accent_rgb) = detect_toast_app(context);

    let mut icon_bgra = None;
    let mut icon_w = 0;
    let mut icon_h = 0;
    let mut icon_is_circle = false;
    if let Some((img_path, is_circle)) = extract_toast_logo_override(xml) {
        if let Some((bgra, w, h)) = decode_local_image_to_bgra(&img_path) {
            icon_bgra = Some(bgra);
            icon_w = w;
            icon_h = h;
            icon_is_circle = is_circle;
        }
    }

    Some(ToastNotification {
        app,
        title,
        body,
        accent_rgb,
        icon_bgra,
        icon_w,
        icon_h,
        icon_is_circle,
    })
}

/// Only extracts `<image placement="appLogoOverride" ...>` (avatars/app logos),
/// never hero/inline content images like Snipping Tool screenshots.
fn extract_toast_logo_override(xml: &str) -> Option<(PathBuf, bool)> {
    let mut rest = xml;
    while let Some(open) = rest.find("<image") {
        let after = &rest[open + 6..];
        let Some(end) = after.find('>') else {
            break;
        };
        let tag = &after[..end];
        let is_logo = tag.contains("appLogoOverride");
        let is_circle = tag.contains("hint-crop=\"circle\"") || tag.contains("hint-crop='circle'");
        if is_logo {
            if let Some(raw_src) = extract_xml_attr(tag, "src") {
                let unescaped = unescape_xml(&raw_src);
                if let Some(path) = normalize_local_image_path(&unescaped) {
                    if path.exists() {
                        return Some((path, is_circle));
                    }
                }
            }
        }
        rest = &after[end + 1..];
    }
    None
}

fn extract_xml_attr(tag: &str, attr: &str) -> Option<String> {
    for quote in ['"', '\''] {
        let pat = format!("{attr}={quote}");
        if let Some(pos) = tag.find(&pat) {
            let val_start = pos + pat.len();
            if let Some(end) = tag[val_start..].find(quote) {
                return Some(tag[val_start..val_start + end].to_string());
            }
        }
    }
    None
}

fn normalize_local_image_path(src: &str) -> Option<PathBuf> {
    let trimmed = src.trim();
    if trimmed.is_empty()
        || trimmed.starts_with("http://")
        || trimmed.starts_with("https://")
        || trimmed.starts_with("ms-appx:")
    {
        return None;
    }
    let raw = if let Some(s) = trimmed.strip_prefix("file:///") {
        s
    } else if let Some(s) = trimmed.strip_prefix("file://") {
        s
    } else {
        trimmed
    };
    let decoded = percent_decode(raw);
    let cleaned = decoded.replace('/', "\\");
    let p = PathBuf::from(&cleaned);
    if p.is_absolute() {
        Some(p)
    } else {
        None
    }
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push((hi << 4) | lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn decode_local_image_to_bgra(path: &Path) -> Option<(Vec<u8>, u32, u32)> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.is_empty() || bytes.len() > 8 * 1024 * 1024 {
        return None;
    }
    let stream = InMemoryRandomAccessStream::new().ok()?;
    let writer = DataWriter::CreateDataWriter(&stream).ok()?;
    writer.WriteBytes(&bytes).ok()?;
    writer.StoreAsync().ok()?.get().ok()?;
    writer.DetachStream().ok()?;
    stream.Seek(0).ok()?;

    let decoder = BitmapDecoder::CreateAsync(&stream).ok()?.get().ok()?;
    let target = 64u32;
    let transform = BitmapTransform::new().ok()?;
    transform.SetScaledWidth(target).ok()?;
    transform.SetScaledHeight(target).ok()?;
    let pixel_provider = decoder
        .GetPixelDataTransformedAsync(
            BitmapPixelFormat::Bgra8,
            BitmapAlphaMode::Premultiplied,
            &transform,
            ExifOrientationMode::RespectExifOrientation,
            ColorManagementMode::DoNotColorManage,
        )
        .ok()?
        .get()
        .ok()?;
    let data = pixel_provider.DetachPixelData().ok()?;
    let vec = data.to_vec();
    if vec.len() == (target * target * 4) as usize {
        Some((vec, target, target))
    } else {
        None
    }
}

fn resolve_app_info(primary_id: &str, xml: &str) -> CachedAppInfo {
    let (fallback_name, mut accent_rgb) = detect_toast_app(if primary_id.is_empty() {
        xml
    } else {
        primary_id
    });

    let mut display_name = fallback_name;
    let mut icon = None;

    if !primary_id.is_empty() {
        let parsing = format!("shell:AppsFolder\\{primary_id}");
        if let Some((name_opt, icon_opt)) = load_shell_item_info(&parsing, 64) {
            if let Some(name) = name_opt {
                if !name.trim().is_empty() {
                    display_name = name;
                }
            }
            if let Some((bgra, w, h)) = icon_opt {
                if let Some(dom) = dominant_accent_from_bgra(&bgra) {
                    accent_rgb = dom;
                }
                icon = Some((bgra, w, h));
            }
        }
        // Fallback for non-packaged exe paths stored directly as PrimaryId.
        if icon.is_none() && (primary_id.contains('\\') || primary_id.ends_with(".exe")) {
            let expanded = expand_env_vars(primary_id);
            if Path::new(&expanded).exists() {
                if let Some((name_opt, icon_opt)) = load_shell_item_info(&expanded, 64) {
                    if let Some(name) = name_opt {
                        if !name.trim().is_empty() {
                            display_name = name.trim_end_matches(".exe").to_string();
                        }
                    }
                    if let Some((bgra, w, h)) = icon_opt {
                        if let Some(dom) = dominant_accent_from_bgra(&bgra) {
                            accent_rgb = dom;
                        }
                        icon = Some((bgra, w, h));
                    }
                }
            }
        }
        if display_name == "Notification" {
            if let Some(pretty) = prettify_aumid(primary_id) {
                display_name = pretty;
            }
        }
    }

    CachedAppInfo {
        display_name,
        accent_rgb,
        icon,
    }
}

fn expand_env_vars(s: &str) -> String {
    let mut out = s.to_string();
    for (k, env_k) in [
        ("{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}", "SystemRoot"),
        ("{6D809377-6AF0-444B-8957-A3773F02200E}", "ProgramFiles"),
        ("{7C5A40EF-A0FB-4BFC-874A-C0F2E0B9FA8E}", "ProgramFiles(x86)"),
    ] {
        if out.starts_with(k) {
            if let Ok(v) = std::env::var(env_k) {
                let prefix = if env_k == "SystemRoot" {
                    format!("{v}\\System32")
                } else {
                    v
                };
                out = out.replacen(k, &prefix, 1);
            }
        }
    }
    out
}

fn prettify_aumid(aumid: &str) -> Option<String> {
    let base = aumid.split('!').next().unwrap_or(aumid);
    let base = base.split('_').next().unwrap_or(base);
    let seg = base.rsplit(['.', '\\']).next().unwrap_or(base).trim();
    if seg.len() >= 2 && seg.len() <= 32 && seg.chars().all(|c| c.is_ascii_alphanumeric() || c == ' ' || c == '-') {
        Some(seg.to_string())
    } else {
        None
    }
}

fn dominant_accent_from_bgra(bgra: &[u8]) -> Option<(f32, f32, f32)> {
    let mut sr = 0.0_f32;
    let mut sg = 0.0_f32;
    let mut sb = 0.0_f32;
    let mut sw = 0.0_f32;
    for px in bgra.chunks_exact(4) {
        let a = px[3] as f32 / 255.0;
        if a < 0.35 {
            continue;
        }
        let b = (px[0] as f32 / 255.0) / a;
        let g = (px[1] as f32 / 255.0) / a;
        let r = (px[2] as f32 / 255.0) / a;
        let maxc = r.max(g).max(b);
        let minc = r.min(g).min(b);
        let sat = if maxc > 0.01 { (maxc - minc) / maxc } else { 0.0 };
        if sat > 0.25 && maxc > 0.20 {
            let w = sat * a;
            sr += r * w;
            sg += g * w;
            sb += b * w;
            sw += w;
        }
    }
    if sw > 12.0 {
        Some(((sr / sw).clamp(0.12, 1.0), (sg / sw).clamp(0.12, 1.0), (sb / sw).clamp(0.12, 1.0)))
    } else {
        None
    }
}

fn load_shell_item_info(
    parsing_name: &str,
    size_px: i32,
) -> Option<(Option<String>, Option<(Vec<u8>, u32, u32)>)> {
    let wide: Vec<u16> = OsStr::new(parsing_name)
        .encode_wide()
        .chain(Some(0))
        .collect();
    unsafe {
        let item: IShellItem = SHCreateItemFromParsingName(PCWSTR(wide.as_ptr()), None).ok()?;
        let display_name = item.GetDisplayName(SIGDN_NORMALDISPLAY).ok().map(|pw| {
            let s = pw.to_string().unwrap_or_default();
            CoTaskMemFree(Some(pw.0 as *const c_void));
            s
        });
        let icon = item.cast::<IShellItemImageFactory>().ok().and_then(|factory| {
            let hbm: HBITMAP = factory
                .GetImage(
                    SIZE {
                        cx: size_px,
                        cy: size_px,
                    },
                    SIIGBF_ICONONLY | SIIGBF_BIGGERSIZEOK,
                )
                .ok()?;
            let res = hbitmap_to_premul_bgra(hbm);
            let _ = DeleteObject(hbm);
            res
        });
        Some((display_name, icon))
    }
}

unsafe fn hbitmap_to_premul_bgra(hbm: HBITMAP) -> Option<(Vec<u8>, u32, u32)> {
    let mut bm = BITMAP::default();
    if GetObjectW(
        hbm,
        std::mem::size_of::<BITMAP>() as i32,
        Some(&mut bm as *mut BITMAP as *mut c_void),
    ) == 0
    {
        return None;
    }
    let w = bm.bmWidth.max(0) as u32;
    let h = bm.bmHeight.abs().max(0) as u32;
    if w == 0 || h == 0 || w > 256 || h > 256 {
        return None;
    }
    let mut bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w as i32,
            biHeight: -(h as i32), // top-down DIB
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut buf = vec![0u8; (w * h * 4) as usize];
    let hdc = GetDC(None);
    if hdc.is_invalid() {
        return None;
    }
    let rows = GetDIBits(
        hdc,
        hbm,
        0,
        h,
        Some(buf.as_mut_ptr() as *mut c_void),
        &mut bmi,
        DIB_RGB_COLORS,
    );
    ReleaseDC(None, hdc);
    if rows == 0 {
        return None;
    }
    // Premultiply alpha for Direct2D D2D1_ALPHA_MODE_PREMULTIPLIED.
    for px in buf.chunks_exact_mut(4) {
        let a = px[3] as u16;
        if a < 255 {
            px[0] = ((px[0] as u16 * a) / 255) as u8;
            px[1] = ((px[1] as u16 * a) / 255) as u8;
            px[2] = ((px[2] as u16 * a) / 255) as u8;
        }
    }
    Some((buf, w, h))
}

fn extract_toast_texts(xml: &str) -> Option<(String, String)> {
    let mut texts = Vec::new();
    let mut rest = xml;
    while let Some(open) = rest.find("<text") {
        let after_open = &rest[open + 5..];
        let Some(gt) = after_open.find('>') else {
            break;
        };
        let after_gt = &after_open[gt + 1..];
        let Some(close) = after_gt.find("</text>") else {
            break;
        };
        let raw = after_gt[..close].trim();
        let clean = unescape_xml(raw);
        if !clean.is_empty() {
            texts.push(clean);
            if texts.len() >= 2 {
                break;
            }
        }
        rest = &after_gt[close + 7..];
    }
    if texts.is_empty() {
        return None;
    }
    let title = texts.remove(0);
    let body = if texts.is_empty() {
        String::new()
    } else {
        texts.remove(0)
    };
    Some((title, body))
}

fn unescape_xml(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
}

fn detect_toast_app(context: &str) -> (String, (f32, f32, f32)) {
    let low = context.to_ascii_lowercase();
    let known: &[(&str, &str, (f32, f32, f32))] = &[
        ("whatsapp", "WhatsApp", (0.145, 0.827, 0.400)),
        ("discord", "Discord", (0.345, 0.396, 0.949)),
        ("telegram", "Telegram", (0.161, 0.663, 0.922)),
        ("teams", "Teams", (0.384, 0.392, 0.655)),
        ("slack", "Slack", (0.878, 0.118, 0.353)),
        ("spotify", "Spotify", (0.114, 0.725, 0.329)),
        ("outlook", "Outlook", (0.039, 0.518, 1.000)),
        ("mail", "Mail", (0.039, 0.518, 1.000)),
        ("chrome", "Chrome", (0.259, 0.522, 0.957)),
        ("msedge", "Edge", (0.047, 0.557, 0.949)),
        ("edge", "Edge", (0.047, 0.557, 0.949)),
        ("steam", "Steam", (0.200, 0.560, 0.880)),
        ("instagram", "Instagram", (0.882, 0.188, 0.424)),
        ("messenger", "Messenger", (0.000, 0.518, 1.000)),
        ("signal", "Signal", (0.227, 0.463, 0.941)),
        ("defender", "Security", (0.188, 0.745, 0.460)),
    ];
    for &(needle, label, rgb) in known {
        if low.contains(needle) {
            return (label.to_string(), rgb);
        }
    }
    ("Notification".to_string(), (0.24, 0.64, 1.0))
}

pub struct MicUsageMonitor {
    active_app: Option<String>,
    last_poll: Instant,
}

impl MicUsageMonitor {
    pub fn new() -> Self {
        Self {
            active_app: query_active_mic_app(),
            last_poll: Instant::now(),
        }
    }

    pub fn poll_changed(&mut self) -> Option<Option<String>> {
        if self.last_poll.elapsed().as_millis() < 500 {
            return None;
        }
        self.last_poll = Instant::now();
        let cur = query_active_mic_app();
        if cur != self.active_app {
            self.active_app = cur.clone();
            return Some(cur);
        }
        None
    }
}

fn query_active_mic_app() -> Option<String> {
    const BASE: &str =
        r"SOFTWARE\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\microphone";
    const NON_PACKAGED: &str =
        r"SOFTWARE\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\microphone\NonPackaged";

    let mut best: Option<(u64, String)> = None;
    scan_mic_consent_key(NON_PACKAGED, true, &mut best);
    scan_mic_consent_key(BASE, false, &mut best);
    best.map(|(_, name)| name)
}

fn scan_mic_consent_key(subkey: &str, non_packaged: bool, best: &mut Option<(u64, String)>) {
    let wide_sub: Vec<u16> = subkey.encode_utf16().chain(std::iter::once(0)).collect();
    let mut root = HKEY::default();
    let open_res = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(wide_sub.as_ptr()),
            0,
            KEY_READ,
            &mut root,
        )
    };
    if open_res != ERROR_SUCCESS {
        return;
    }

    let mut idx = 0u32;
    let mut name_buf = [0u16; 512];
    loop {
        let mut name_len = name_buf.len() as u32;
        let enum_res = unsafe {
            RegEnumKeyExW(
                root,
                idx,
                PWSTR(name_buf.as_mut_ptr()),
                &mut name_len,
                None,
                PWSTR::null(),
                None,
                None,
            )
        };
        if enum_res != ERROR_SUCCESS {
            break;
        }
        idx += 1;

        let raw_name = String::from_utf16_lossy(&name_buf[..name_len as usize]);
        if !non_packaged && raw_name.eq_ignore_ascii_case("NonPackaged") {
            continue;
        }

        let child_wide: Vec<u16> = raw_name.encode_utf16().chain(std::iter::once(0)).collect();
        let mut child = HKEY::default();
        if unsafe {
            RegOpenKeyExW(
                root,
                PCWSTR(child_wide.as_ptr()),
                0,
                KEY_READ,
                &mut child,
            )
        } == ERROR_SUCCESS
        {
            let start = reg_read_qword(child, w!("LastUsedTimeStart")).unwrap_or(0);
            let stop = reg_read_qword(child, w!("LastUsedTimeStop")).unwrap_or(0);
            unsafe {
                let _ = RegCloseKey(child);
            }
            if start > 0 && (stop == 0 || start > stop) {
                let display = format_mic_app_name(&raw_name, non_packaged);
                if !display.is_empty() && best.as_ref().map_or(true, |(ts, _)| start > *ts) {
                    *best = Some((start, display));
                }
            }
        }
    }

    unsafe {
        let _ = RegCloseKey(root);
    }
}

fn reg_read_qword(key: HKEY, value_name: PCWSTR) -> Option<u64> {
    let mut val = 0u64;
    let mut cb = std::mem::size_of::<u64>() as u32;
    let mut ty = REG_VALUE_TYPE::default();
    let res = unsafe {
        RegQueryValueExW(
            key,
            value_name,
            None,
            Some(&mut ty),
            Some(&mut val as *mut u64 as *mut u8),
            Some(&mut cb),
        )
    };
    if res == ERROR_SUCCESS && ty == REG_QWORD && cb == 8 {
        Some(val)
    } else {
        None
    }
}

pub fn format_mic_app_name(raw: &str, non_packaged: bool) -> String {
    let base = if non_packaged {
        let seg = raw.rsplit('#').next().unwrap_or(raw);
        let seg = seg.rsplit('\\').next().unwrap_or(seg);
        seg.strip_suffix(".exe")
            .or_else(|| seg.strip_suffix(".EXE"))
            .unwrap_or(seg)
    } else {
        let before_pub = raw.split('_').next().unwrap_or(raw);
        before_pub
            .strip_prefix("Microsoft.Windows")
            .or_else(|| before_pub.strip_prefix("Microsoft."))
            .unwrap_or(before_pub)
    };
    let low = base.to_ascii_lowercase();
    match low.as_str() {
        "chrome" => "Chrome".to_string(),
        "msedge" => "Edge".to_string(),
        "firefox" => "Firefox".to_string(),
        "brave" => "Brave".to_string(),
        "opera" => "Opera".to_string(),
        "discord" | "discordptb" | "discordcanary" => "Discord".to_string(),
        "teams" | "ms-teams" => "Teams".to_string(),
        "zoom" => "Zoom".to_string(),
        "obs64" | "obs32" => "OBS Studio".to_string(),
        "telegram" => "Telegram".to_string(),
        "whatsapp" => "WhatsApp".to_string(),
        "slack" => "Slack".to_string(),
        "code" => "VS Code".to_string(),
        "soundrecorder" => "Voice Recorder".to_string(),
        "audacity" => "Audacity".to_string(),
        "steamwebhelper" | "steam" => "Steam".to_string(),
        _ => {
            let trimmed = base.trim();
            let mut chars = trimmed.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_uppercase(), chars.as_str()),
                None => String::new(),
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetworkKind {
    Wifi,
    Ethernet,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkEvent {
    pub kind: NetworkKind,
    pub connected: bool,
    pub name: String,
    pub signal_pct: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum NetLinkState {
    Offline,
    Wifi { ssid: String, signal_pct: u8 },
    Ethernet,
}

pub struct NetworkMonitor {
    last_state: NetLinkState,
    last_poll: Instant,
}

impl NetworkMonitor {
    pub fn new() -> Self {
        Self {
            last_state: query_network_state(),
            last_poll: Instant::now(),
        }
    }

    pub fn poll_changed(&mut self) -> Option<NetworkEvent> {
        if self.last_poll.elapsed().as_millis() < 1200 {
            return None;
        }
        self.last_poll = Instant::now();
        let cur = query_network_state();
        if cur == self.last_state {
            return None;
        }
        let prev = std::mem::replace(&mut self.last_state, cur.clone());
        match (prev, cur) {
            (_, NetLinkState::Wifi { ssid, signal_pct }) => Some(NetworkEvent {
                kind: NetworkKind::Wifi,
                connected: true,
                name: if ssid.is_empty() {
                    "Wi-Fi".to_string()
                } else {
                    ssid
                },
                signal_pct,
            }),
            (_, NetLinkState::Ethernet) => Some(NetworkEvent {
                kind: NetworkKind::Ethernet,
                connected: true,
                name: "Ethernet".to_string(),
                signal_pct: 100,
            }),
            (NetLinkState::Wifi { ssid, .. }, NetLinkState::Offline) => Some(NetworkEvent {
                kind: NetworkKind::Wifi,
                connected: false,
                name: if ssid.is_empty() {
                    "Wi-Fi".to_string()
                } else {
                    ssid
                },
                signal_pct: 0,
            }),
            (NetLinkState::Ethernet, NetLinkState::Offline) => Some(NetworkEvent {
                kind: NetworkKind::Ethernet,
                connected: false,
                name: "Ethernet".to_string(),
                signal_pct: 0,
            }),
            (NetLinkState::Offline, NetLinkState::Offline) => None,
        }
    }
}

fn query_network_state() -> NetLinkState {
    let mut best_idx = 0u32;
    let res = unsafe { GetBestInterface(0x0808_0808, &mut best_idx) };
    if res != 0 || best_idx == 0 {
        return NetLinkState::Offline;
    }
    let mut row = MibIfRow::default();
    row.dw_index = best_idx;
    if unsafe { GetIfEntry(&mut row) } != 0 {
        return NetLinkState::Offline;
    }
    let up = row.dw_oper_status == IF_OPER_STATUS_CONNECTED
        || row.dw_oper_status == IF_OPER_STATUS_OPERATIONAL;
    if !up {
        return NetLinkState::Offline;
    }
    if row.dw_type == IF_TYPE_IEEE80211 {
        let (ssid, sig) = query_wlan_ssid_and_signal().unwrap_or_else(|| ("Wi-Fi".to_string(), 85));
        NetLinkState::Wifi {
            ssid,
            signal_pct: sig,
        }
    } else if row.dw_type == IF_TYPE_ETHERNET_CSMACD {
        NetLinkState::Ethernet
    } else {
        if let Some((ssid, sig)) = query_wlan_ssid_and_signal() {
            NetLinkState::Wifi {
                ssid,
                signal_pct: sig,
            }
        } else {
            NetLinkState::Ethernet
        }
    }
}

#[repr(C)]
struct WlanInterfaceInfo {
    interface_guid: [u8; 16],
    str_interface_description: [u16; 256],
    is_state: u32,
}

#[repr(C)]
struct WlanInterfaceInfoList {
    dw_number_of_items: u32,
    dw_index: u32,
    interface_info: [WlanInterfaceInfo; 1],
}

#[repr(C)]
struct Dot11Ssid {
    u_ssid_length: u32,
    uc_ssid: [u8; 32],
}

#[repr(C)]
struct WlanAssociationAttributes {
    dot11_ssid: Dot11Ssid,
    dot11_bss_type: u32,
    dot11_bssid: [u8; 6],
    _pad0: [u8; 2],
    dot11_phy_type: u32,
    u_dot11_phy_index: u32,
    wlan_signal_quality: u32,
    ul_rx_rate: u32,
    ul_tx_rate: u32,
}

#[repr(C)]
struct WlanConnectionAttributes {
    is_state: u32,
    wlan_connection_mode: u32,
    str_profile_name: [u16; 256],
    wlan_association_attributes: WlanAssociationAttributes,
}

fn query_wlan_ssid_and_signal() -> Option<(String, u8)> {
    type WlanOpenHandleFn =
        unsafe extern "system" fn(u32, *mut c_void, *mut u32, *mut HANDLE) -> u32;
    type WlanCloseHandleFn = unsafe extern "system" fn(HANDLE, *mut c_void) -> u32;
    type WlanEnumInterfacesFn =
        unsafe extern "system" fn(HANDLE, *mut c_void, *mut *mut WlanInterfaceInfoList) -> u32;
    type WlanQueryInterfaceFn = unsafe extern "system" fn(
        HANDLE,
        *const [u8; 16],
        u32,
        *mut c_void,
        *mut u32,
        *mut *mut c_void,
        *mut u32,
    ) -> u32;
    type WlanFreeMemoryFn = unsafe extern "system" fn(*mut c_void);

    unsafe {
        let dll = LoadLibraryW(w!("wlanapi.dll").as_ptr());
        if dll.is_null() {
            return None;
        }
        let open_p = GetProcAddress(dll, b"WlanOpenHandle\0".as_ptr());
        let close_p = GetProcAddress(dll, b"WlanCloseHandle\0".as_ptr());
        let enum_p = GetProcAddress(dll, b"WlanEnumInterfaces\0".as_ptr());
        let query_p = GetProcAddress(dll, b"WlanQueryInterface\0".as_ptr());
        let free_p = GetProcAddress(dll, b"WlanFreeMemory\0".as_ptr());
        if open_p.is_null()
            || close_p.is_null()
            || enum_p.is_null()
            || query_p.is_null()
            || free_p.is_null()
        {
            FreeLibrary(dll);
            return None;
        }
        let wlan_open: WlanOpenHandleFn = std::mem::transmute(open_p);
        let wlan_close: WlanCloseHandleFn = std::mem::transmute(close_p);
        let wlan_enum: WlanEnumInterfacesFn = std::mem::transmute(enum_p);
        let wlan_query: WlanQueryInterfaceFn = std::mem::transmute(query_p);
        let wlan_free: WlanFreeMemoryFn = std::mem::transmute(free_p);

        let mut neg_ver = 0u32;
        let mut client = HANDLE(std::ptr::null_mut());
        if wlan_open(2, std::ptr::null_mut(), &mut neg_ver, &mut client) != 0 {
            FreeLibrary(dll);
            return None;
        }

        let mut result = None;
        let mut list_ptr: *mut WlanInterfaceInfoList = std::ptr::null_mut();
        if wlan_enum(client, std::ptr::null_mut(), &mut list_ptr) == 0 && !list_ptr.is_null() {
            let count = (*list_ptr).dw_number_of_items as usize;
            let base = (*list_ptr).interface_info.as_ptr();
            for i in 0..count {
                let info = &*base.add(i);
                if info.is_state != 1 {
                    continue;
                }
                let mut data_size = 0u32;
                let mut data_ptr: *mut c_void = std::ptr::null_mut();
                let mut opcode_val_type = 0u32;
                if wlan_query(
                    client,
                    &info.interface_guid,
                    WLAN_INTF_OPCODE_CURRENT_CONNECTION,
                    std::ptr::null_mut(),
                    &mut data_size,
                    &mut data_ptr,
                    &mut opcode_val_type,
                ) == 0
                    && !data_ptr.is_null()
                    && (data_size as usize) >= std::mem::size_of::<WlanConnectionAttributes>()
                {
                    let attrs = &*(data_ptr as *const WlanConnectionAttributes);
                    let ssid_len = (attrs.wlan_association_attributes.dot11_ssid.u_ssid_length
                        as usize)
                        .min(32);
                    let ssid = String::from_utf8_lossy(
                        &attrs.wlan_association_attributes.dot11_ssid.uc_ssid[..ssid_len],
                    )
                    .trim()
                    .to_string();
                    let sig = attrs.wlan_association_attributes.wlan_signal_quality.min(100) as u8;
                    wlan_free(data_ptr);
                    result = Some((if ssid.is_empty() { "Wi-Fi".to_string() } else { ssid }, sig));
                    break;
                }
            }
            wlan_free(list_ptr as *mut c_void);
        }

        wlan_close(client, std::ptr::null_mut());
        FreeLibrary(dll);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sdr_white_level_converts_to_percentage() {
        assert_eq!(sdr_white_level_to_pct(1000), 0);
        assert_eq!(sdr_white_level_to_pct(3500), 50);
        assert_eq!(sdr_white_level_to_pct(6000), 100);
        assert_eq!(sdr_white_level_to_pct(500), 0);
        assert_eq!(sdr_white_level_to_pct(8000), 100);
    }

    #[test]
    fn chime_wav_has_valid_pcm_header() {
        let wav = build_chime_wav();
        assert!(wav.len() > 44_000);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[36..40], b"data");
    }

    #[test]
    fn parses_toast_xml_and_detects_app_brand() {
        let raw = b"junk\x00\x015319275A.WhatsAppDesktop_cv1g1gvanyjgm!App\x00<toast launch=\"msg\"><visual><binding template=\"ToastGeneric\"><text>Marco</text><text>Ci vediamo alle 9 &amp; porta il laptop!</text></binding></visual></toast>\x00tail";
        let notif = extract_latest_toast_from_bytes(raw).expect("toast parsed");
        assert_eq!(notif.app, "WhatsApp");
        assert_eq!(notif.title, "Marco");
        assert_eq!(notif.body, "Ci vediamo alle 9 & porta il laptop!");
        assert!(notif.accent_rgb.1 > 0.8);
    }

    #[test]
    fn wal_prefers_active_salt_frame_over_stale_tail_frame() {
        let page_size = 512usize;
        let mut wal = vec![0u8; 32 + 2 * (24 + page_size)];
        wal[0..4].copy_from_slice(&0x377F_0682u32.to_be_bytes());
        wal[8..12].copy_from_slice(&(page_size as u32).to_be_bytes());
        wal[16..20].copy_from_slice(&7u32.to_be_bytes());
        wal[20..24].copy_from_slice(&9u32.to_be_bytes());

        let f0 = 32;
        wal[f0 + 8..f0 + 12].copy_from_slice(&7u32.to_be_bytes());
        wal[f0 + 12..f0 + 16].copy_from_slice(&9u32.to_be_bytes());
        let new_xml = b"Discord\x00<toast><visual><binding><text>New Msg</text><text>Hello now!</text></binding></visual></toast>";
        wal[f0 + 24..f0 + 24 + new_xml.len()].copy_from_slice(new_xml);

        let f1 = 32 + 24 + page_size;
        wal[f1 + 8..f1 + 12].copy_from_slice(&6u32.to_be_bytes());
        wal[f1 + 12..f1 + 16].copy_from_slice(&9u32.to_be_bytes());
        let old_xml = b"WhatsApp\x00<toast><visual><binding><text>Old Msg</text><text>Stale</text></binding></visual></toast>";
        wal[f1 + 24..f1 + 24 + old_xml.len()].copy_from_slice(old_xml);

        let notif = extract_latest_toast_from_wal(&wal).expect("wal toast");
        assert_eq!(notif.app, "Discord");
        assert_eq!(notif.title, "New Msg");
    }

    #[test]
    fn formats_mic_app_names_cleanly() {
        assert_eq!(
            format_mic_app_name(
                "C:#Program Files#Google#Chrome#Application#chrome.exe",
                true
            ),
            "Chrome"
        );
        assert_eq!(
            format_mic_app_name("C:#Users#me#AppData#Local#Discord#app#Discord.exe", true),
            "Discord"
        );
        assert_eq!(
            format_mic_app_name("Microsoft.WindowsSoundRecorder_8wekyb3d8bbwe", false),
            "Voice Recorder"
        );
    }
}
