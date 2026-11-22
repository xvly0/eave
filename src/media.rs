use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use windows::core::{implement, Interface, GUID, PWSTR};
use windows::Foundation::{EventRegistrationToken, TypedEventHandler};
use windows::Graphics::Imaging::{
    BitmapAlphaMode, BitmapDecoder, BitmapInterpolationMode, BitmapPixelFormat, BitmapTransform,
    ColorManagementMode, ExifOrientationMode,
};
use windows::Media::Control::{
    CurrentSessionChangedEventArgs, GlobalSystemMediaTransportControlsSession,
    GlobalSystemMediaTransportControlsSessionManager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus, MediaPropertiesChangedEventArgs,
    PlaybackInfoChangedEventArgs, TimelinePropertiesChangedEventArgs,
};
use windows::Storage::Streams::{Buffer, DataReader, IRandomAccessStream};
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, S_OK, WPARAM};
use windows::Win32::Media::Audio::Endpoints::{
    IAudioEndpointVolume, IAudioEndpointVolumeCallback, IAudioEndpointVolumeCallback_Impl,
};
use windows::Win32::Media::Audio::{
    eCapture, eConsole, eRender, AudioSessionStateActive, EDataFlow, ERole,
    IAudioSessionControl2, IAudioSessionManager2, IMMDeviceEnumerator, IMMNotificationClient,
    IMMNotificationClient_Impl, MMDeviceEnumerator, AUDIO_VOLUME_NOTIFICATION_DATA,
    DEVICE_STATE_ACTIVE, DEVICE_STATE,
};
use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_ALL, STGM_READ};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::System::WinRT::{RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED};
use windows::Win32::UI::Shell::PropertiesSystem::PROPERTYKEY;
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_NULL};

const PKEY_DEVICE_FRIENDLY_NAME: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0xa45c254e_df1c_4efd_8020_67d146a850e0),
    pid: 14,
};

const EAVE_VOLUME_EVENT_CTX: GUID = GUID::from_u128(0x7c8e9b4a_51a2_4e8d_9c10_3a2b4c5d6e7f);

/// 100-nanosecond ticks per second (WinRT `TimeSpan` / `DateTime` unit).
const TICKS_PER_SEC: f64 = 10_000_000.0;
const WIN_TO_UNIX_EPOCH_TICKS: i64 = 116_444_736_000_000_000;
const MAX_THUMBNAIL_DIM: u32 = 128;

/// Decoded album art in premultiplied BGRA8 format ready for Direct2D bitmap creation.
#[derive(Clone, Debug)]
pub struct ThumbnailData {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
    pub revision: u64,
}

/// Commands sent from the UI thread to the SMTC / Core Audio worker thread.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MediaCommand {
    PlayPause,
    Next,
    Previous,
    Seek(f64),
    SetVolume(f32),
    SetMuted(bool),
}

#[derive(Clone, Debug)]
pub struct MediaSnapshot {
    pub has_session: bool,
    pub is_playing: bool,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub position_secs: f64,
    pub duration_secs: f64,
    pub thumbnail: Option<ThumbnailData>,
    pub track_key: String,
    pub volume_pct: f32,
    pub muted: bool,
    pub external_volume_seq: u64,
    pub audio_endpoint_seq: u64,
    pub audio_device_name: String,
    pub mic_muted: bool,
    pub mic_mute_seq: u64,
    pub mic_in_use_app: String,
    pub mic_in_use_seq: u64,
}

impl Default for MediaSnapshot {
    fn default() -> Self {
        Self {
            has_session: false,
            is_playing: false,
            title: String::new(),
            artist: String::new(),
            album: String::new(),
            position_secs: 0.0,
            duration_secs: 0.0,
            thumbnail: None,
            track_key: String::new(),
            volume_pct: 100.0,
            muted: false,
            external_volume_seq: 0,
            audio_endpoint_seq: 0,
            audio_device_name: String::new(),
            mic_muted: false,
            mic_mute_seq: 0,
            mic_in_use_app: String::new(),
            mic_in_use_seq: 0,
        }
    }
}

struct SharedState {
    snapshot: MediaSnapshot,
    captured_at: Option<Instant>,
    suppress_vol_until: Option<Instant>,
}

impl Default for SharedState {
    fn default() -> Self {
        Self {
            snapshot: MediaSnapshot::default(),
            captured_at: None,
            suppress_vol_until: None,
        }
    }
}

enum WorkerMsg {
    SessionChanged,
    MediaPropertiesChanged,
    PlaybackOrTimelineChanged,
    Command(MediaCommand),
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

struct SessionSubscription {
    session: GlobalSystemMediaTransportControlsSession,
    media_token: Option<EventRegistrationToken>,
    playback_token: Option<EventRegistrationToken>,
    timeline_token: Option<EventRegistrationToken>,
}

impl SessionSubscription {
    fn new(session: GlobalSystemMediaTransportControlsSession, tx: &Sender<WorkerMsg>) -> Self {
        let tx_media = tx.clone();
        let media_token = session
            .MediaPropertiesChanged(&TypedEventHandler::<
                GlobalSystemMediaTransportControlsSession,
                MediaPropertiesChangedEventArgs,
            >::new(move |_, _| {
                let _ = tx_media.send(WorkerMsg::MediaPropertiesChanged);
                Ok(())
            }))
            .ok();

        let tx_playback = tx.clone();
        let playback_token = session
            .PlaybackInfoChanged(&TypedEventHandler::<
                GlobalSystemMediaTransportControlsSession,
                PlaybackInfoChangedEventArgs,
            >::new(move |_, _| {
                let _ = tx_playback.send(WorkerMsg::PlaybackOrTimelineChanged);
                Ok(())
            }))
            .ok();

        let tx_timeline = tx.clone();
        let timeline_token = session
            .TimelinePropertiesChanged(&TypedEventHandler::<
                GlobalSystemMediaTransportControlsSession,
                TimelinePropertiesChangedEventArgs,
            >::new(move |_, _| {
                let _ = tx_timeline.send(WorkerMsg::PlaybackOrTimelineChanged);
                Ok(())
            }))
            .ok();

        Self {
            session,
            media_token,
            playback_token,
            timeline_token,
        }
    }

    fn unregister(&mut self) {
        if let Some(tok) = self.media_token.take() {
            let _ = self.session.RemoveMediaPropertiesChanged(tok);
        }
        if let Some(tok) = self.playback_token.take() {
            let _ = self.session.RemovePlaybackInfoChanged(tok);
        }
        if let Some(tok) = self.timeline_token.take() {
            let _ = self.session.RemoveTimelinePropertiesChanged(tok);
        }
    }
}

impl Drop for SessionSubscription {
    fn drop(&mut self) {
        self.unregister();
    }
}

/// Manages the background SMTC worker thread and exposes non-blocking snapshots & commands.
pub struct MediaService {
    shared: Arc<Mutex<SharedState>>,
    tx: Sender<WorkerMsg>,
    worker: Option<JoinHandle<()>>,
}

impl MediaService {
    /// Spawns the background SMTC worker thread without a UI wake HWND.
    #[allow(dead_code)]
    pub fn start() -> Self {
        Self::start_internal(0)
    }

    /// Spawns the background SMTC & Core Audio worker thread and wakes `hwnd` via `WM_NULL`
    pub fn start_with_wake(hwnd: HWND) -> Self {
        Self::start_internal(hwnd.0 as isize)
    }

    fn start_internal(wake_raw: isize) -> Self {
        let shared = Arc::new(Mutex::new(SharedState::default()));
        let wake_hwnd = Arc::new(AtomicIsize::new(wake_raw));
        let (tx, rx) = mpsc::channel::<WorkerMsg>();

        let shared_worker = Arc::clone(&shared);
        let wake_worker = Arc::clone(&wake_hwnd);
        let tx_worker = tx.clone();
        let worker = thread::Builder::new()
            .name("eave-smtc".to_string())
            .spawn(move || run_worker(shared_worker, wake_worker, tx_worker, rx))
            .ok();

        Self { shared, tx, worker }
    }

    pub fn snapshot(&self) -> MediaSnapshot {
        let Ok(guard) = self.shared.lock() else {
            return MediaSnapshot::default();
        };
        let mut snap = guard.snapshot.clone();
        if snap.has_session && snap.is_playing {
            if let Some(captured) = guard.captured_at {
                let elapsed = captured.elapsed().as_secs_f64();
                snap.position_secs += elapsed;
                if snap.duration_secs > 0.0 {
                    snap.position_secs = snap.position_secs.min(snap.duration_secs);
                }
            }
        }
        snap
    }

    pub fn send_command(&self, cmd: MediaCommand) {
        if let Ok(mut guard) = self.shared.lock() {
            match cmd {
                MediaCommand::SetVolume(pct) => {
                    guard.snapshot.volume_pct = pct.clamp(0.0, 100.0);
                }
                MediaCommand::SetMuted(muted) => {
                    guard.snapshot.muted = muted;
                }
                _ => {}
            }
        }
        let _ = self.tx.send(WorkerMsg::Command(cmd));
    }

    /// Gracefully stops the background SMTC worker thread.
    pub fn shutdown(&mut self) {
        let _ = self.tx.send(WorkerMsg::Shutdown);
        if let Some(handle) = self.worker.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for MediaService {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// state synchronously on the caller's COM thread, returning `None` if COM or endpoint access fails.
pub fn read_system_volume() -> Option<(f32, bool)> {
    let mut audio = CoreAudioEndpoint::new(eRender, None);
    if audio.endpoint.is_some() {
        let (pct, muted, _) = audio.read_volume_and_mute();
        Some((pct, muted))
    } else {
        None
    }
}

fn post_wake_hwnd(wake_hwnd: &AtomicIsize) {
    let raw = wake_hwnd.load(Ordering::Relaxed);
    if raw != 0 {
        // Safety: raw is a live top-level NotchWindow HWND; WM_NULL wakes GetMessageW with no side effects.
        unsafe {
            let _ = PostMessageW(HWND(raw as *mut core::ffi::c_void), WM_NULL, WPARAM(0), LPARAM(0));
        }
    }
}

#[implement(IAudioEndpointVolumeCallback)]
struct EndpointVolumeCallback {
    shared: Arc<Mutex<SharedState>>,
    wake_hwnd: Arc<AtomicIsize>,
}

impl IAudioEndpointVolumeCallback_Impl for EndpointVolumeCallback_Impl {
    fn OnNotify(&self, pnotify: *mut AUDIO_VOLUME_NOTIFICATION_DATA) -> windows::core::Result<()> {
        if pnotify.is_null() {
            return Ok(());
        }
        // Safety: Core Audio passes a valid AUDIO_VOLUME_NOTIFICATION_DATA pointer during OnNotify.
        let data = unsafe { &*pnotify };
        if data.guidEventContext == EAVE_VOLUME_EVENT_CTX {
            return Ok(());
        }
        let new_pct = (data.fMasterVolume * 100.0).clamp(0.0, 100.0);
        let new_muted = data.bMuted.as_bool();
        let mut changed = false;
        if let Ok(mut guard) = self.shared.lock() {
            let suppressed = guard
                .suppress_vol_until
                .is_some_and(|until| Instant::now() < until);
            let delta = (guard.snapshot.volume_pct - new_pct).abs();
            let mute_changed = guard.snapshot.muted != new_muted;
            let is_profile_jump = !mute_changed && delta >= 12.0;
            if delta > 0.25 || mute_changed {
                guard.snapshot.volume_pct = new_pct;
                guard.snapshot.muted = new_muted;
                if !suppressed && !is_profile_jump {
                    guard.snapshot.external_volume_seq =
                        guard.snapshot.external_volume_seq.wrapping_add(1);
                    changed = true;
                }
            }
        }
        if changed {
            post_wake_hwnd(&self.wake_hwnd);
        }
        Ok(())
    }
}

#[implement(IMMNotificationClient)]
struct DefaultDeviceCallback {
    shared: Arc<Mutex<SharedState>>,
}

impl IMMNotificationClient_Impl for DefaultDeviceCallback_Impl {
    fn OnDeviceStateChanged(
        &self,
        _pwstrdeviceid: &windows::core::PCWSTR,
        _dwnewstate: DEVICE_STATE,
    ) -> windows::core::Result<()> {
        if let Ok(mut guard) = self.shared.lock() {
            guard.suppress_vol_until = Some(Instant::now() + Duration::from_millis(1800));
        }
        Ok(())
    }

    fn OnDeviceAdded(&self, _pwstrdeviceid: &windows::core::PCWSTR) -> windows::core::Result<()> {
        Ok(())
    }

    fn OnDeviceRemoved(
        &self,
        _pwstrdeviceid: &windows::core::PCWSTR,
    ) -> windows::core::Result<()> {
        Ok(())
    }

    fn OnDefaultDeviceChanged(
        &self,
        _flow: EDataFlow,
        _role: ERole,
        _pwstrdefaultdeviceid: &windows::core::PCWSTR,
    ) -> windows::core::Result<()> {
        if let Ok(mut guard) = self.shared.lock() {
            guard.suppress_vol_until = Some(Instant::now() + Duration::from_millis(1800));
        }
        Ok(())
    }

    fn OnPropertyValueChanged(
        &self,
        _pwstrdeviceid: &windows::core::PCWSTR,
        _key: &PROPERTYKEY,
    ) -> windows::core::Result<()> {
        Ok(())
    }
}

#[implement(IAudioEndpointVolumeCallback)]
struct MicMuteCallback {
    shared: Arc<Mutex<SharedState>>,
    wake_hwnd: Arc<AtomicIsize>,
}

impl IAudioEndpointVolumeCallback_Impl for MicMuteCallback_Impl {
    fn OnNotify(&self, pnotify: *mut AUDIO_VOLUME_NOTIFICATION_DATA) -> windows::core::Result<()> {
        if pnotify.is_null() {
            return Ok(());
        }
        let data = unsafe { &*pnotify };
        let new_muted = data.bMuted.as_bool();
        let mut changed = false;
        if let Ok(mut guard) = self.shared.lock() {
            if guard.snapshot.mic_muted != new_muted {
                guard.snapshot.mic_muted = new_muted;
                guard.snapshot.mic_mute_seq = guard.snapshot.mic_mute_seq.wrapping_add(1);
                changed = true;
            }
        }
        if changed {
            post_wake_hwnd(&self.wake_hwnd);
        }
        Ok(())
    }
}

struct CoreAudioEndpoint {
    flow: EDataFlow,
    enumerator: Option<IMMDeviceEnumerator>,
    device_id: Option<String>,
    device_name: String,
    endpoint: Option<IAudioEndpointVolume>,
    callback: Option<IAudioEndpointVolumeCallback>,
    last_device_check: Instant,
    fallback_volume_pct: f32,
    fallback_muted: bool,
}

impl CoreAudioEndpoint {
    fn new(flow: EDataFlow, callback: Option<IAudioEndpointVolumeCallback>) -> Self {
        // Safety: CoCreateInstance creates the Core Audio MMDeviceEnumerator on an initialized COM thread.
        let enumerator: Option<IMMDeviceEnumerator> =
            unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }.ok();
        let mut ep = Self {
            flow,
            enumerator,
            device_id: None,
            device_name: String::new(),
            endpoint: None,
            callback,
            last_device_check: Instant::now() - Duration::from_secs(10),
            fallback_volume_pct: 100.0,
            fallback_muted: false,
        };
        ep.refresh_endpoint(true);
        ep
    }

    #[cfg(test)]
    fn fallback_only(volume_pct: f32, muted: bool) -> Self {
        Self {
            flow: eRender,
            enumerator: None,
            device_id: None,
            device_name: String::new(),
            endpoint: None,
            callback: None,
            last_device_check: Instant::now(),
            fallback_volume_pct: volume_pct.clamp(0.0, 100.0),
            fallback_muted: muted,
        }
    }

    fn clear_endpoint(&mut self) {
        if let (Some(ep), Some(cb)) = (&self.endpoint, &self.callback) {
            // Safety: unregisters previously registered callback before releasing endpoint.
            unsafe {
                let _ = ep.UnregisterControlChangeNotify(cb);
            }
        }
        self.device_id = None;
        self.device_name.clear();
        self.endpoint = None;
    }

    fn refresh_endpoint(&mut self, force: bool) -> bool {
        if !force
            && self.endpoint.is_some()
            && self.last_device_check.elapsed() < Duration::from_millis(800)
        {
            return false;
        }
        self.last_device_check = Instant::now();
        let Some(enumerator) = &self.enumerator else {
            return false;
        };
        // Safety: queries the default console render endpoint and its IAudioEndpointVolume interface.
        let Ok(device) = (unsafe { enumerator.GetDefaultAudioEndpoint(self.flow, eConsole) }) else {
            let had = self.device_id.is_some();
            self.clear_endpoint();
            return had;
        };
        let new_id = unsafe {
            device.GetId().ok().map(|pwstr| {
                let s = pwstr.to_string().unwrap_or_default();
                CoTaskMemFree(Some(pwstr.0 as *const core::ffi::c_void));
                s
            })
        };
        if self.endpoint.is_some() && new_id.is_some() && new_id == self.device_id {
            return false;
        }
        let changed = self.device_id.is_some() && new_id != self.device_id;
        let friendly_name = unsafe {
            device
                .OpenPropertyStore(STGM_READ)
                .ok()
                .and_then(|store| store.GetValue(&PKEY_DEVICE_FRIENDLY_NAME).ok())
                .map(|pv| pv.to_string())
                .unwrap_or_default()
        };
        self.clear_endpoint();
        // Safety: activates IAudioEndpointVolume on the current default render IMMDevice.
        if let Ok(ep) = unsafe { device.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None) } {
            if let Some(cb) = &self.callback {
                // Safety: registers callback for immediate external volume/mute notifications.
                unsafe {
                    let _ = ep.RegisterControlChangeNotify(cb);
                }
            }
            self.device_id = new_id;
            self.device_name = friendly_name;
            self.endpoint = Some(ep);
        }
        changed
    }

    fn read_volume_and_mute(&mut self) -> (f32, bool, bool) {
        let mut endpoint_changed = self.refresh_endpoint(false);
        for attempt in 0..2 {
            let Some(ep) = &self.endpoint else {
                break;
            };
            // Safety: ep is a valid IAudioEndpointVolume on this COM thread.
            let scalar_res = unsafe { ep.GetMasterVolumeLevelScalar() };
            let mute_res = unsafe { ep.GetMute() };
            match (scalar_res, mute_res) {
                (Ok(scalar), Ok(mute_bool)) => {
                    let pct = (scalar * 100.0).clamp(0.0, 100.0);
                    let muted = mute_bool.as_bool();
                    self.fallback_volume_pct = pct;
                    self.fallback_muted = muted;
                    return (pct, muted, endpoint_changed);
                }
                _ => {
                    self.clear_endpoint();
                    if attempt == 0 {
                        endpoint_changed |= self.refresh_endpoint(true);
                    }
                }
            }
        }
        (self.fallback_volume_pct, self.fallback_muted, endpoint_changed)
    }

    fn set_volume_pct(&mut self, volume_pct: f32) {
        let pct = volume_pct.clamp(0.0, 100.0);
        self.fallback_volume_pct = pct;
        let scalar = (pct / 100.0).clamp(0.0, 1.0);
        if self.endpoint.is_none() {
            self.refresh_endpoint(true);
        }
        for attempt in 0..2 {
            let Some(ep) = &self.endpoint else {
                return;
            };
            // Safety: ep is a valid IAudioEndpointVolume; tagged with Eave's event context GUID.
            if unsafe { ep.SetMasterVolumeLevelScalar(scalar, &EAVE_VOLUME_EVENT_CTX) }.is_ok() {
                return;
            }
            self.clear_endpoint();
            if attempt == 0 {
                self.refresh_endpoint(true);
            }
        }
    }

    fn set_muted(&mut self, muted: bool) {
        self.fallback_muted = muted;
        if self.endpoint.is_none() {
            self.refresh_endpoint(true);
        }
        for attempt in 0..2 {
            let Some(ep) = &self.endpoint else {
                return;
            };
            // Safety: ep is a valid IAudioEndpointVolume; tagged with Eave's event context GUID.
            if unsafe { ep.SetMute(muted, &EAVE_VOLUME_EVENT_CTX) }.is_ok() {
                return;
            }
            self.clear_endpoint();
            if attempt == 0 {
                self.refresh_endpoint(true);
            }
        }
    }
}

impl Drop for CoreAudioEndpoint {
    fn drop(&mut self) {
        self.clear_endpoint();
    }
}

fn detect_active_mic_capture_app(enumerator: &IMMDeviceEnumerator) -> Option<String> {
    let collection =
        unsafe { enumerator.EnumAudioEndpoints(eCapture, DEVICE_STATE_ACTIVE) }.ok()?;
    let dev_count = unsafe { collection.GetCount() }.ok()?;
    for d in 0..dev_count {
        let Ok(device) = (unsafe { collection.Item(d) }) else {
            continue;
        };
        let Ok(mgr) = (unsafe { device.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) })
        else {
            continue;
        };
        let Ok(sess_enum) = (unsafe { mgr.GetSessionEnumerator() }) else {
            continue;
        };
        let Ok(sess_count) = (unsafe { sess_enum.GetCount() }) else {
            continue;
        };
        for s in 0..sess_count {
            let Ok(ctrl) = (unsafe { sess_enum.GetSession(s) }) else {
                continue;
            };
            if unsafe { ctrl.GetState() }.ok() != Some(AudioSessionStateActive) {
                continue;
            }
            let Ok(ctrl2) = ctrl.cast::<IAudioSessionControl2>() else {
                continue;
            };
            if unsafe { ctrl2.IsSystemSoundsSession() } == S_OK {
                continue;
            }
            let pid = unsafe { ctrl2.GetProcessId() }.unwrap_or(0);
            let mut raw_exe = if pid > 4 {
                process_image_name_for_pid(pid)
            } else {
                None
            };
            if raw_exe.is_none() {
                if let Ok(pw) = unsafe { ctrl2.GetSessionIdentifier() } {
                    let sid = unsafe { pw.to_string().unwrap_or_default() };
                    unsafe { CoTaskMemFree(Some(pw.0 as *const core::ffi::c_void)) };
                    raw_exe = extract_exe_from_session_id(&sid);
                }
            }
            if let Some(exe_path) = raw_exe {
                let low = exe_path.to_ascii_lowercase();
                if low.ends_with("audiodg.exe")
                    || low.ends_with("svchost.exe")
                    || low.ends_with("eave.exe")
                {
                    continue;
                }
                let name = crate::osd_monitors::format_mic_app_name(&exe_path, true);
                if !name.is_empty() {
                    return Some(name);
                }
            }
            if let Ok(pw) = unsafe { ctrl.GetDisplayName() } {
                let dn = unsafe { pw.to_string().unwrap_or_default() };
                unsafe { CoTaskMemFree(Some(pw.0 as *const core::ffi::c_void)) };
                let trimmed = dn.trim();
                if !trimmed.is_empty() && !trimmed.starts_with('@') {
                    return Some(trimmed.to_string());
                }
            }
            return Some("Active".to_string());
        }
    }
    None
}

fn extract_exe_from_session_id(sid: &str) -> Option<String> {
    let after_pipe = sid.split('|').nth(1).unwrap_or(sid);
    let before_sep = after_pipe.split("%b").next().unwrap_or(after_pipe);
    let trimmed = before_sep.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_string())
}

fn process_image_name_for_pid(pid: u32) -> Option<String> {
    let hproc = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
    let mut buf = [0u16; 512];
    let mut len = buf.len() as u32;
    let ok = unsafe {
        QueryFullProcessImageNameW(
            hproc,
            PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &mut len,
        )
    };
    unsafe {
        let _ = CloseHandle(hproc);
    }
    if ok.is_ok() && len > 0 {
        Some(String::from_utf16_lossy(&buf[..len as usize]))
    } else {
        None
    }
}

struct WorkerState {
    shared: Arc<Mutex<SharedState>>,
    wake_hwnd: Arc<AtomicIsize>,
    tx: Sender<WorkerMsg>,
    current_sub: Option<SessionSubscription>,
    cached_thumb: Option<ThumbnailData>,
    last_thumb_hash: u64,
    last_track_key: String,
    thumb_revision: u64,
    timeline_start_ticks: i64,
    audio: CoreAudioEndpoint,
    mic: CoreAudioEndpoint,
    _dev_notify_client: Option<IMMNotificationClient>,
    last_mic_in_use_app: Option<String>,
    mic_idle_since: Option<Instant>,
    last_mic_session_check: Instant,
    volume_hold_until: Instant,
}

impl WorkerState {
    fn new(
        shared: Arc<Mutex<SharedState>>,
        wake_hwnd: Arc<AtomicIsize>,
        tx: Sender<WorkerMsg>,
    ) -> Self {
        let cb: IAudioEndpointVolumeCallback = EndpointVolumeCallback {
            shared: Arc::clone(&shared),
            wake_hwnd: Arc::clone(&wake_hwnd),
        }
        .into();
        let mic_cb: IAudioEndpointVolumeCallback = MicMuteCallback {
            shared: Arc::clone(&shared),
            wake_hwnd: Arc::clone(&wake_hwnd),
        }
        .into();
        let dev_notify: IMMNotificationClient = DefaultDeviceCallback {
            shared: Arc::clone(&shared),
        }
        .into();
        let audio = CoreAudioEndpoint::new(eRender, Some(cb));
        let registered_dev_notify = if let Some(en) = &audio.enumerator {
            if unsafe { en.RegisterEndpointNotificationCallback(&dev_notify) }.is_ok() {
                Some(dev_notify)
            } else {
                None
            }
        } else {
            None
        };
        let mut state = Self {
            shared,
            wake_hwnd,
            tx,
            current_sub: None,
            cached_thumb: None,
            last_thumb_hash: 0,
            last_track_key: String::new(),
            thumb_revision: 0,
            timeline_start_ticks: 0,
            audio,
            mic: CoreAudioEndpoint::new(eCapture, Some(mic_cb)),
            _dev_notify_client: registered_dev_notify,
            last_mic_in_use_app: None,
            mic_idle_since: None,
            last_mic_session_check: Instant::now() - Duration::from_secs(5),
            volume_hold_until: Instant::now(),
        };
        state.refresh_audio_volume(true);
        let (_, init_mic_muted, _) = state.mic.read_volume_and_mute();
        if let Some(en) = &state.mic.enumerator {
            state.last_mic_in_use_app = detect_active_mic_capture_app(en);
        }
        if let Ok(mut guard) = state.shared.lock() {
            guard.snapshot.mic_muted = init_mic_muted;
        }
        state
    }

    fn refresh_audio_volume(&mut self, force: bool) {
        if !force && Instant::now() < self.volume_hold_until {
            return;
        }
        let (volume_pct, muted, endpoint_changed) = self.audio.read_volume_and_mute();
        let _ = self.mic.refresh_endpoint(false);
        let mut mic_started_app: Option<String> = None;
        if self.last_mic_session_check.elapsed() >= Duration::from_millis(300) {
            self.last_mic_session_check = Instant::now();
            if let Some(en) = &self.mic.enumerator {
                let cur_mic_app = detect_active_mic_capture_app(en);
                match cur_mic_app {
                    Some(app) => {
                        self.mic_idle_since = None;
                        if self.last_mic_in_use_app.as_deref() != Some(app.as_str()) {
                            self.last_mic_in_use_app = Some(app.clone());
                            mic_started_app = Some(app);
                        }
                    }
                    None => {
                        let idle_start = *self.mic_idle_since.get_or_insert_with(Instant::now);
                        if idle_start.elapsed() >= Duration::from_millis(300) {
                            if self.last_mic_in_use_app.is_some() {
                                if let Ok(mut guard) = self.shared.lock() {
                                    guard.suppress_vol_until =
                                        Some(Instant::now() + Duration::from_millis(1200));
                                }
                            }
                            self.last_mic_in_use_app = None;
                        }
                    }
                }
            }
        }
        let mut should_wake = false;
        if let Ok(mut guard) = self.shared.lock() {
            if endpoint_changed {
                guard.snapshot.audio_device_name = self.audio.device_name.clone();
                guard.snapshot.audio_endpoint_seq =
                    guard.snapshot.audio_endpoint_seq.wrapping_add(1);
                should_wake = true;
            } else if !force
                && ((guard.snapshot.volume_pct - volume_pct).abs() > 0.25
                    || guard.snapshot.muted != muted)
            {
                guard.snapshot.external_volume_seq =
                    guard.snapshot.external_volume_seq.wrapping_add(1);
                should_wake = true;
            }
            if let Some(app) = mic_started_app {
                guard.snapshot.mic_in_use_app = app;
                guard.snapshot.mic_in_use_seq = guard.snapshot.mic_in_use_seq.wrapping_add(1);
                guard.suppress_vol_until = Some(Instant::now() + Duration::from_millis(1800));
                should_wake = true;
            }
            guard.snapshot.volume_pct = volume_pct;
            guard.snapshot.muted = muted;
        }
        if should_wake {
            post_wake_hwnd(&self.wake_hwnd);
        }
    }

    fn bind_current_session(
        &mut self,
        manager: &GlobalSystemMediaTransportControlsSessionManager,
        force_thumb: bool,
    ) {
        let new_session = manager.GetCurrentSession().ok();
        let same_session = match (&self.current_sub, &new_session) {
            (Some(old), Some(new_s)) => {
                old.session.SourceAppUserModelId().ok() == new_s.SourceAppUserModelId().ok()
                    && old.session.as_raw() == new_s.as_raw()
            }
            (None, None) => true,
            _ => false,
        };

        if !same_session {
            self.current_sub = new_session.map(|s| SessionSubscription::new(s, &self.tx));
            if self.current_sub.is_none() {
                self.cached_thumb = None;
                self.last_thumb_hash = 0;
                self.last_track_key.clear();
                self.timeline_start_ticks = 0;
                if let Ok(mut guard) = self.shared.lock() {
                    let volume_pct = guard.snapshot.volume_pct;
                    let muted = guard.snapshot.muted;
                    let external_volume_seq = guard.snapshot.external_volume_seq;
                    let audio_endpoint_seq = guard.snapshot.audio_endpoint_seq;
                    let audio_device_name = guard.snapshot.audio_device_name.clone();
                    let mic_muted = guard.snapshot.mic_muted;
                    let mic_mute_seq = guard.snapshot.mic_mute_seq;
                    let mic_in_use_app = guard.snapshot.mic_in_use_app.clone();
                    let mic_in_use_seq = guard.snapshot.mic_in_use_seq;
                    guard.snapshot = MediaSnapshot {
                        volume_pct,
                        muted,
                        external_volume_seq,
                        audio_endpoint_seq,
                        audio_device_name,
                        mic_muted,
                        mic_mute_seq,
                        mic_in_use_app,
                        mic_in_use_seq,
                        ..MediaSnapshot::default()
                    };
                    guard.captured_at = None;
                }
                return;
            }
            self.refresh_all(true);
            return;
        }

        if self.current_sub.is_some() {
            self.refresh_all(force_thumb);
        }
    }

    fn refresh_all(&mut self, force_thumb: bool) {
        self.refresh_media_properties(force_thumb);
        self.refresh_playback_and_timeline();
    }

    fn refresh_media_properties(&mut self, force_thumb: bool) {
        let Some(sub) = &self.current_sub else {
            return;
        };
        let Ok(op) = sub.session.TryGetMediaPropertiesAsync() else {
            return;
        };
        let Ok(props) = op.get() else {
            return;
        };

        let title = props.Title().map(|s| s.to_string_lossy()).unwrap_or_default();
        let artist = props
            .Artist()
            .map(|s| s.to_string_lossy())
            .unwrap_or_default();
        let album = props
            .AlbumTitle()
            .map(|s| s.to_string_lossy())
            .unwrap_or_default();

        let track_key = build_track_key(&title, &artist, &album);
        let track_changed = track_key != self.last_track_key;
        self.last_track_key = track_key.clone();

        if force_thumb || track_changed || self.cached_thumb.is_none() {
            if let Ok(thumb_ref) = props.Thumbnail() {
                match decode_thumbnail(&thumb_ref, self.last_thumb_hash, self.thumb_revision + 1) {
                    ThumbDecodeResult::Decoded(thumb, hash) => {
                        self.thumb_revision = thumb.revision;
                        self.last_thumb_hash = hash;
                        self.cached_thumb = Some(thumb);
                    }
                    ThumbDecodeResult::Unchanged => {}
                    ThumbDecodeResult::EmptyOrError => {
                        self.cached_thumb = None;
                        self.last_thumb_hash = 0;
                    }
                }
            } else {
                self.cached_thumb = None;
                self.last_thumb_hash = 0;
            }
        }

        if let Ok(mut guard) = self.shared.lock() {
            guard.snapshot.has_session = true;
            guard.snapshot.title = title;
            guard.snapshot.artist = artist;
            guard.snapshot.album = album;
            guard.snapshot.track_key = track_key;
            guard.snapshot.thumbnail = self.cached_thumb.clone();
        }
    }

    fn refresh_playback_and_timeline(&mut self) {
        let Some(sub) = &self.current_sub else {
            return;
        };

        let is_playing = sub
            .session
            .GetPlaybackInfo()
            .and_then(|info| info.PlaybackStatus())
            .map(|st| st == GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing)
            .unwrap_or(false);

        let (position_secs, duration_secs, start_ticks) =
            read_timeline(&sub.session, is_playing);
        self.timeline_start_ticks = start_ticks;

        if let Ok(mut guard) = self.shared.lock() {
            guard.snapshot.has_session = true;
            guard.snapshot.is_playing = is_playing;
            guard.snapshot.position_secs = position_secs;
            guard.snapshot.duration_secs = duration_secs;
            guard.captured_at = Some(Instant::now());
        }
    }

    fn handle_command(&mut self, cmd: MediaCommand) {
        match cmd {
            MediaCommand::SetVolume(pct) => {
                let pct = pct.clamp(0.0, 100.0);
                self.audio.set_volume_pct(pct);
                self.volume_hold_until = Instant::now() + Duration::from_millis(250);
                if let Ok(mut guard) = self.shared.lock() {
                    guard.snapshot.volume_pct = pct;
                }
                return;
            }
            MediaCommand::SetMuted(muted) => {
                self.audio.set_muted(muted);
                self.volume_hold_until = Instant::now() + Duration::from_millis(250);
                if let Ok(mut guard) = self.shared.lock() {
                    guard.snapshot.muted = muted;
                }
                return;
            }
            _ => {}
        }

        let Some(sub) = &self.current_sub else {
            return;
        };
        match cmd {
            MediaCommand::PlayPause => {
                if let Ok(op) = sub.session.TryTogglePlayPauseAsync() {
                    let _ = op.get();
                }
            }
            MediaCommand::Next => {
                if let Ok(op) = sub.session.TrySkipNextAsync() {
                    let _ = op.get();
                }
            }
            MediaCommand::Previous => {
                if let Ok(op) = sub.session.TrySkipPreviousAsync() {
                    let _ = op.get();
                }
            }
            MediaCommand::Seek(secs) => {
                let target_ticks =
                    self.timeline_start_ticks + (secs.max(0.0) * TICKS_PER_SEC) as i64;
                if let Ok(op) = sub.session.TryChangePlaybackPositionAsync(target_ticks) {
                    let _ = op.get();
                }
            }
            MediaCommand::SetVolume(_) | MediaCommand::SetMuted(_) => unreachable!(),
        }
        self.refresh_playback_and_timeline();
    }
}

fn run_worker(
    shared: Arc<Mutex<SharedState>>,
    wake_hwnd: Arc<AtomicIsize>,
    tx: Sender<WorkerMsg>,
    rx: Receiver<WorkerMsg>,
) {
    let _com = ComGuard::init_mta();
    if !_com.initialized {
        return;
    }

    let mut state = WorkerState::new(shared, wake_hwnd, tx.clone());

    let manager = GlobalSystemMediaTransportControlsSessionManager::RequestAsync()
        .and_then(|op| op.get())
        .ok();

    let mgr_token = manager.as_ref().and_then(|mgr| {
        let tx_sess = tx.clone();
        mgr.CurrentSessionChanged(&TypedEventHandler::<
            GlobalSystemMediaTransportControlsSessionManager,
            CurrentSessionChangedEventArgs,
        >::new(move |_, _| {
            let _ = tx_sess.send(WorkerMsg::SessionChanged);
            Ok(())
        }))
        .ok()
    });

    if let Some(mgr) = &manager {
        state.bind_current_session(mgr, true);
    }

    let mut last_smtc_poll = Instant::now();
    let mut last_vol_poll = Instant::now();

    loop {
        let is_playing = state
            .shared
            .lock()
            .map(|g| g.snapshot.has_session && g.snapshot.is_playing)
            .unwrap_or(false);
        let smtc_interval = if is_playing {
            Duration::from_millis(1000)
        } else {
            Duration::from_secs(3)
        };
        let vol_interval = Duration::from_millis(300);

        match rx.recv_timeout(vol_interval) {
            Ok(WorkerMsg::Shutdown) | Err(RecvTimeoutError::Disconnected) => break,
            Ok(WorkerMsg::SessionChanged) => {
                if let Some(mgr) = &manager {
                    state.bind_current_session(mgr, true);
                }
            }
            Ok(WorkerMsg::MediaPropertiesChanged) => state.refresh_media_properties(true),
            Ok(WorkerMsg::PlaybackOrTimelineChanged) => state.refresh_playback_and_timeline(),
            Ok(WorkerMsg::Command(mut cmd)) => {
                if matches!(cmd, MediaCommand::SetVolume(_)) {
                    let mut deferred = Vec::new();
                    let mut should_break = false;
                    while let Ok(next_msg) = rx.try_recv() {
                        match next_msg {
                            WorkerMsg::Command(next_cmd @ MediaCommand::SetVolume(_)) => {
                                cmd = next_cmd;
                            }
                            WorkerMsg::Shutdown => {
                                should_break = true;
                                break;
                            }
                            other => deferred.push(other),
                        }
                    }
                    state.handle_command(cmd);
                    for msg in deferred {
                        match msg {
                            WorkerMsg::Shutdown => {
                                should_break = true;
                                break;
                            }
                            WorkerMsg::SessionChanged => {
                                if let Some(mgr) = &manager {
                                    state.bind_current_session(mgr, true);
                                }
                            }
                            WorkerMsg::MediaPropertiesChanged => state.refresh_media_properties(true),
                            WorkerMsg::PlaybackOrTimelineChanged => state.refresh_playback_and_timeline(),
                            WorkerMsg::Command(c) => state.handle_command(c),
                        }
                    }
                    if should_break {
                        break;
                    }
                } else {
                    state.handle_command(cmd);
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
        }

        let now = Instant::now();
        if now.duration_since(last_vol_poll) >= vol_interval {
            state.refresh_audio_volume(false);
            last_vol_poll = now;
        }
        if now.duration_since(last_smtc_poll) >= smtc_interval {
            if let Some(mgr) = &manager {
                // Periodic safety refresh in case a media app drops an SMTC event.
                state.bind_current_session(mgr, false);
            }
            last_smtc_poll = now;
        }
    }

    if let Some(sub) = state.current_sub.take() {
        drop(sub);
    }
    if let (Some(mgr), Some(tok)) = (manager, mgr_token) {
        let _ = mgr.RemoveCurrentSessionChanged(tok);
    }
}

fn read_timeline(
    session: &GlobalSystemMediaTransportControlsSession,
    is_playing: bool,
) -> (f64, f64, i64) {
    let Ok(tl) = session.GetTimelineProperties() else {
        return (0.0, 0.0, 0);
    };

    let start = tl.StartTime().map(|t| t.Duration).unwrap_or(0);
    let end = tl.EndTime().map(|t| t.Duration).unwrap_or(0);
    let pos = tl.Position().map(|t| t.Duration).unwrap_or(0);
    let last_updated = tl.LastUpdatedTime().map(|d| d.UniversalTime).unwrap_or(0);

    let duration_ticks = (end - start).max(0);
    let mut pos_ticks = (pos - start).max(0);

    if is_playing && last_updated > WIN_TO_UNIX_EPOCH_TICKS {
        let now_ticks = current_win_filetime_ticks();
        let elapsed_ticks = (now_ticks - last_updated).max(0);
        if elapsed_ticks < 24 * 3600 * 10_000_000 {
            pos_ticks += elapsed_ticks;
        }
    }

    if duration_ticks > 0 {
        pos_ticks = pos_ticks.min(duration_ticks);
    }

    (
        pos_ticks as f64 / TICKS_PER_SEC,
        duration_ticks as f64 / TICKS_PER_SEC,
        start,
    )
}

fn current_win_filetime_ticks() -> i64 {
    let unix_ns = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0);
    WIN_TO_UNIX_EPOCH_TICKS + (unix_ns / 100)
}

enum ThumbDecodeResult {
    Unchanged,
    Decoded(ThumbnailData, u64),
    EmptyOrError,
}

fn decode_thumbnail(
    thumb_ref: &windows::Storage::Streams::IRandomAccessStreamReference,
    prev_hash: u64,
    next_revision: u64,
) -> ThumbDecodeResult {
    let Some(stream) = thumb_ref.OpenReadAsync().ok().and_then(|op| op.get().ok()) else {
        return ThumbDecodeResult::EmptyOrError;
    };
    let size = stream.Size().unwrap_or(0);
    if size == 0 || size > 16 * 1024 * 1024 {
        let _ = stream.Close();
        return ThumbDecodeResult::EmptyOrError;
    }

    let Ok(ra_stream) = stream.cast::<IRandomAccessStream>() else {
        let _ = stream.Close();
        return ThumbDecodeResult::EmptyOrError;
    };
    let Some(decoder) = BitmapDecoder::CreateAsync(&ra_stream)
        .ok()
        .and_then(|op| op.get().ok())
    else {
        let _ = stream.Close();
        return ThumbDecodeResult::EmptyOrError;
    };

    let (orig_w, orig_h) = match (decoder.PixelWidth(), decoder.PixelHeight()) {
        (Ok(w), Ok(h)) if w > 0 && h > 0 => (w, h),
        _ => {
            let _ = stream.Close();
            return ThumbDecodeResult::EmptyOrError;
        }
    };

    let (target_w, target_h) = fit_dimensions(orig_w, orig_h, MAX_THUMBNAIL_DIM);
    let Ok(transform) = BitmapTransform::new() else {
        let _ = stream.Close();
        return ThumbDecodeResult::EmptyOrError;
    };
    if target_w != orig_w || target_h != orig_h {
        if transform.SetScaledWidth(target_w).is_err()
            || transform.SetScaledHeight(target_h).is_err()
            || transform
                .SetInterpolationMode(BitmapInterpolationMode::Fant)
                .is_err()
        {
            let _ = stream.Close();
            return ThumbDecodeResult::EmptyOrError;
        }
    }

    let Some(bitmap) = decoder
        .GetSoftwareBitmapTransformedAsync(
            BitmapPixelFormat::Bgra8,
            BitmapAlphaMode::Premultiplied,
            &transform,
            ExifOrientationMode::RespectExifOrientation,
            ColorManagementMode::ColorManageToSRgb,
        )
        .ok()
        .and_then(|op| op.get().ok())
    else {
        let _ = stream.Close();
        return ThumbDecodeResult::EmptyOrError;
    };

    let w = bitmap.PixelWidth().unwrap_or(0) as u32;
    let h = bitmap.PixelHeight().unwrap_or(0) as u32;
    let Some(byte_len) = w.checked_mul(h).and_then(|px| px.checked_mul(4)) else {
        let _ = bitmap.Close();
        let _ = stream.Close();
        return ThumbDecodeResult::EmptyOrError;
    };
    if byte_len == 0 {
        let _ = bitmap.Close();
        let _ = stream.Close();
        return ThumbDecodeResult::EmptyOrError;
    }

    let Ok(buffer) = Buffer::Create(byte_len) else {
        let _ = bitmap.Close();
        let _ = stream.Close();
        return ThumbDecodeResult::EmptyOrError;
    };
    if bitmap.CopyToBuffer(&buffer).is_err() {
        let _ = bitmap.Close();
        let _ = stream.Close();
        return ThumbDecodeResult::EmptyOrError;
    }
    let _ = bitmap.Close();
    let _ = stream.Close();

    let Ok(reader) = DataReader::FromBuffer(&buffer) else {
        return ThumbDecodeResult::EmptyOrError;
    };
    let mut bgra = vec![0u8; byte_len as usize];
    if reader.ReadBytes(&mut bgra).is_err() {
        return ThumbDecodeResult::EmptyOrError;
    }

    let hash = fnv1a_64(&bgra);
    if hash == prev_hash && prev_hash != 0 {
        return ThumbDecodeResult::Unchanged;
    }

    ThumbDecodeResult::Decoded(
        ThumbnailData {
            width: w,
            height: h,
            bgra,
            revision: next_revision,
        },
        hash,
    )
}

fn fit_dimensions(w: u32, h: u32, max_dim: u32) -> (u32, u32) {
    if w <= max_dim && h <= max_dim {
        return (w, h);
    }
    if w >= h {
        let scaled_h = ((h as u64 * max_dim as u64) / w as u64).max(1) as u32;
        (max_dim, scaled_h)
    } else {
        let scaled_w = ((w as u64 * max_dim as u64) / h as u64).max(1) as u32;
        (scaled_w, max_dim)
    }
}

fn build_track_key(title: &str, artist: &str, album: &str) -> String {
    let mut key = String::with_capacity(title.len() + artist.len() + album.len() + 2);
    key.push_str(title.trim());
    key.push('|');
    key.push_str(artist.trim());
    key.push('|');
    key.push_str(album.trim());
    key
}

fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_dimensions_preserves_aspect_ratio() {
        assert_eq!(MAX_THUMBNAIL_DIM, 128);
        assert_eq!(
            fit_dimensions(96, 96, MAX_THUMBNAIL_DIM),
            (96, 96)
        );
        assert_eq!(
            fit_dimensions(1024, 512, MAX_THUMBNAIL_DIM),
            (128, 64)
        );
        assert_eq!(
            fit_dimensions(500, 1000, MAX_THUMBNAIL_DIM),
            (64, 128)
        );
    }

    #[test]
    fn track_key_trims_whitespace() {
        assert_eq!(
            build_track_key(" Song ", " Artist", "Album "),
            "Song|Artist|Album"
        );
    }

    #[test]
    fn snapshot_default_and_fallback_volume_clamp() {
        let snap = MediaSnapshot::default();
        assert_eq!(snap.volume_pct, 100.0);
        assert!(!snap.muted);

        let mut ep = CoreAudioEndpoint::fallback_only(120.0, false);
        assert_eq!(ep.read_volume_and_mute(), (100.0, false, false));
        ep.set_volume_pct(37.5);
        ep.set_muted(true);
        assert_eq!(ep.read_volume_and_mute(), (37.5, true, false));
        ep.set_volume_pct(-15.0);
        assert_eq!(ep.read_volume_and_mute(), (0.0, true, false));
    }
}
