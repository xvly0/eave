use std::cell::RefCell;
use std::mem::size_of;
use std::rc::Rc;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{
    BOOL, COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateBitmap, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject,
    EnumDisplayMonitors, GetDC, GetMonitorInfoW, ReleaseDC, ScreenToClient, SelectObject,
    BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HBITMAP, HDC, HGDIOBJ, HMONITOR,
    MONITORINFO, SRCCOPY,
};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryW};
use windows::Win32::UI::HiDpi::{
    GetDpiForMonitor, GetDpiForWindow, SetProcessDpiAwarenessContext,
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, MDT_EFFECTIVE_DPI,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetCapture, GetKeyState, ReleaseCapture, SetCapture, SetFocus, TrackMouseEvent, TME_LEAVE,
    TRACKMOUSEEVENT, VK_CAPITAL,
};
use windows::Win32::UI::Input::{
    GetRawInputData, RegisterRawInputDevices, HRAWINPUT, RAWINPUT, RAWINPUTDEVICE, RAWINPUTHEADER,
    RIDEV_INPUTSINK, RID_INPUT, RIM_TYPEKEYBOARD,
};
use windows::Win32::UI::Shell::{
    SHQueryUserNotificationState, Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD,
    NIM_DELETE, NOTIFYICONDATAW, QUNS_PRESENTATION_MODE,
    QUNS_RUNNING_D3D_FULL_SCREEN,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIconIndirect, CreatePopupMenu, CreateWindowExW, DefWindowProcW,
    DestroyIcon, DestroyMenu, DestroyWindow, DispatchMessageW, GetClassNameW, GetCursorPos,
    GetForegroundWindow, GetMessageW, GetWindowLongPtrW, GetWindowRect, IsWindow, IsWindowVisible,
    KillTimer, LoadCursorW, LoadIconW, PeekMessageW, PostMessageW, PostQuitMessage,
    RegisterClassExW, SetCursor, SetForegroundWindow, SetLayeredWindowAttributes, SetTimer,
    SetWindowLongPtrW, SetWindowPos, ShowWindow, TrackPopupMenu, TranslateMessage, CREATESTRUCTW,
    GWLP_USERDATA, GWL_EXSTYLE, HICON, HTCLIENT, HTTRANSPARENT, ICONINFO, IDC_ARROW,
    IDI_APPLICATION, LWA_ALPHA, MF_CHECKED, MF_POPUP, MF_SEPARATOR, MF_STRING, MF_UNCHECKED, MSG,
    PM_REMOVE, RI_KEY_BREAK,
    SWP_NOACTIVATE, SWP_NOREDRAW, SWP_NOZORDER, SW_HIDE, SW_SHOWNOACTIVATE, TPM_BOTTOMALIGN,
    TPM_LEFTALIGN, TPM_RIGHTBUTTON, WM_APP, WM_COMMAND, WM_CREATE, WM_DESTROY, WM_DISPLAYCHANGE,
    WM_DPICHANGED, WM_INPUT, WM_LBUTTONDOWN, WM_MOUSEMOVE, WM_NCHITTEST, WM_QUIT, WM_RBUTTONUP,
    WM_SETCURSOR, WM_TIMER, WNDCLASSEXW, WS_EX_LAYERED,
    WS_EX_NOACTIVATE, WS_EX_NOREDIRECTIONBITMAP, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_EX_TRANSPARENT, WS_POPUP,
};

const WM_TRAY_CALLBACK: u32 = WM_APP + 1;
const WM_SYNC_HITTEST: u32 = WM_APP + 2;
const WM_MOUSELEAVE: u32 = 0x02A3;
const WM_LBUTTONUP_CONST: u32 = 0x0202;
const WM_MOUSEWHEEL_CONST: u32 = 0x020A;
const WM_KEYDOWN_CONST: u32 = 0x0100;
const WM_CHAR_CONST: u32 = 0x0102;
const WM_HOTKEY_CONST: u32 = 0x0312;
const WM_ACTIVATE_CONST: u32 = 0x0006;
const AI_HOTKEY_ID: i32 = 1;
const OCR_HOTKEY_ID: i32 = 2;
const HIDE_FS_HOTKEY_ID: i32 = 3;

#[link(name = "user32")]
extern "system" {
    fn RegisterHotKey(h_wnd: HWND, id: i32, fs_modifiers: u32, vk: u32) -> i32;
    fn UnregisterHotKey(h_wnd: HWND, id: i32) -> i32;
}
const WM_CAPTURECHANGED_CONST: u32 = 0x0215;
const TRAY_ICON_ID: u32 = 1;
const MONITORINFOF_PRIMARY_FLAG: u32 = 1;

const IDM_SETTINGS: usize = 1001;
const IDM_TOGGLE_STARTUP: usize = 1002;
const IDM_TOGGLE_MASCOT: usize = 1003;
const IDM_TOGGLE_MINIMAL: usize = 1004;
const IDM_SIZE_COMPACT: usize = 1005;
const IDM_SIZE_STANDARD: usize = 1006;
const IDM_SIZE_WIDE: usize = 1007;
const IDM_EXIT: usize = 1008;

pub const IDLE_TIMER_ID: usize = 1;
const CURSOR_POLL_TIMER_ID: usize = 2;
const CURSOR_POLL_INTERVAL_MS: u32 = 16;
const CAPS_CHECK_TIMER_ID: usize = 3;
const CAPS_CHECK_DELAY_MS: u32 = 15;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TrayMenuState {
    pub start_with_windows: bool,
    pub show_mascot: bool,
    pub minimal_mode: bool,
    pub size_idx: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayAction {
    OpenSettings,
    ToggleStartup,
    ToggleMascot,
    ToggleMinimal,
    SetNotchSize(u8),
    ResetPosition,
    Exit,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WindowEvent {
    MouseMove { x_dips: f32, y_dips: f32 },
    MouseLeave,
    LeftDown { x_dips: f32, y_dips: f32 },
    LeftUp { x_dips: f32, y_dips: f32 },
    MouseWheel { delta: i32 },
    KeyChar(char),
    KeySpecial { vk: u16 },
    AiHotkey,
    OcrHotkey,
    ToggleHideFullscreenHotkey,
    FocusLost,
    CapsLockCheck,
    VolumeKey { up: bool },
    TrayAction(TrayAction),
    DpiChanged(f32),
    DisplayChanged,
    TimerTick,
    Quit,
}

#[derive(Debug, Clone, Copy)]
pub struct MonitorRect {
    pub rect: RECT,
    pub dpi: f32,
    pub is_primary: bool,
}

impl MonitorRect {
    pub fn width_px(&self) -> i32 {
        (self.rect.right - self.rect.left).max(1)
    }

    pub fn dpi_scale(&self) -> f32 {
        (self.dpi / 96.0).max(0.5)
    }
}

struct WindowShared {
    events: Vec<WindowEvent>,
    dpi: f32,
    canvas_w_dips: f32,
    pill_w_dips: f32,
    pill_h_dips: f32,
    pill_radius_dips: f32,
    ear_radius_dips: f32,
    tracking_mouse: bool,
    cursor_inside_notified: bool,
    expanded_keep_open: bool,
    extra_hit_box: Option<(f32, f32, f32)>,
    visible: bool,
    click_through: bool,
    sync_posted: bool,
    tray_icon: Option<HICON>,
    tray_added: bool,
    tray_menu: TrayMenuState,
}

impl WindowShared {
    fn dpi_scale(&self) -> f32 {
        (self.dpi / 96.0).max(0.5)
    }

    fn to_notch_local(&self, client_x_px: i32, client_y_px: i32) -> (f32, f32) {
        let scale = self.dpi_scale();
        let canvas_x_dips = client_x_px as f32 / scale;
        let pill_left = (self.canvas_w_dips * 0.5).round() - self.pill_w_dips * 0.5;
        let local_x = canvas_x_dips - pill_left;
        let local_y = client_y_px as f32 / scale;
        (local_x, local_y)
    }

    fn is_point_in_extra_box(&self, client_x_px: i32, client_y_px: i32) -> bool {
        let Some((hw, top, bot)) = self.extra_hit_box else {
            return false;
        };
        let scale = self.dpi_scale();
        let canvas_x = client_x_px as f32 / scale;
        let canvas_y = client_y_px as f32 / scale;
        let canvas_cx = (self.canvas_w_dips * 0.5).round();
        (canvas_x - canvas_cx).abs() <= hw && canvas_y >= top && canvas_y <= bot
    }
}

/// The HWND stays fixed at `(canvas_w_dips, canvas_h_dips)` so animations never call
pub struct NotchWindow {
    hwnd: HWND,
    shared: Rc<RefCell<WindowShared>>,
    last_bounds_px: (i32, i32, i32, i32),
    visible: bool,
    fullscreen_hidden: bool,
    monitor: MonitorRect,
}

impl NotchWindow {
    pub fn enable_per_monitor_dpi() {
        unsafe {
            let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        }
    }

    pub fn create(
        monitor_index: usize,
        canvas_w_dips: f32,
        canvas_h_dips: f32,
        init_w_dips: f32,
        init_h_dips: f32,
        top_offset_dips: f32,
        horizontal_offset_dips: f32,
    ) -> windows::core::Result<Self> {
        let monitor = select_monitor(monitor_index);
        let scale = monitor.dpi_scale();
        let canvas_w_px = ((canvas_w_dips * scale).ceil().max(2.0) as i32 + 1) & !1;
        let canvas_h_px = (canvas_h_dips * scale).ceil().max(2.0) as i32;
        let (x_px, y_px) = centered_top_pos(
            &monitor,
            canvas_w_px,
            top_offset_dips,
            horizontal_offset_dips,
        );

        let hinstance: HINSTANCE = unsafe { GetModuleHandleW(None)?.into() };
        let class_name = w!("EaveNotchWindow");

        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wnd_proc),
            hInstance: hinstance,
            hCursor: unsafe { LoadCursorW(None, IDC_ARROW).unwrap_or_default() },
            lpszClassName: class_name,
            ..Default::default()
        };

        unsafe {
            enable_dark_context_menus();
            let _ = RegisterClassExW(&wc);
        }

        let shared = Rc::new(RefCell::new(WindowShared {
            events: Vec::with_capacity(16),
            dpi: monitor.dpi,
            canvas_w_dips,
            pill_w_dips: init_w_dips,
            pill_h_dips: init_h_dips,
            pill_radius_dips: 14.0_f32.min(init_h_dips * 0.48),
            ear_radius_dips: 10.0,
            tracking_mouse: false,
            cursor_inside_notified: false,
            expanded_keep_open: false,
            extra_hit_box: None,
            visible: true,
            click_through: false,
            sync_posted: false,
            tray_icon: None,
            tray_added: false,
            tray_menu: TrayMenuState::default(),
        }));

        let user_ptr = Rc::into_raw(Rc::clone(&shared)) as *const std::ffi::c_void;

        let ex_style =
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_NOREDIRECTIONBITMAP;
        let hwnd = unsafe {
            CreateWindowExW(
                ex_style,
                class_name,
                w!("Eave"),
                WS_POPUP,
                x_px,
                y_px,
                canvas_w_px,
                canvas_h_px,
                None,
                None,
                hinstance,
                Some(user_ptr),
            )?
        };

        let actual_dpi = unsafe { GetDpiForWindow(hwnd) };
        if actual_dpi > 0 {
            shared.borrow_mut().dpi = actual_dpi as f32;
        }

        unsafe {
            let _ = SetTimer(hwnd, IDLE_TIMER_ID, 1000, None);
            register_raw_keyboard(hwnd);
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        }

        let win = Self {
            hwnd,
            shared,
            last_bounds_px: (x_px, y_px, canvas_w_px, canvas_h_px),
            visible: true,
            fullscreen_hidden: false,
            monitor,
        };
        win.sync_hit_test_cursor();
        Ok(win)
    }

    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    pub fn set_keyboard_focus(&self, active: bool) {
        unsafe {
            let ex = GetWindowLongPtrW(self.hwnd, GWL_EXSTYLE);
            let noact = WS_EX_NOACTIVATE.0 as isize;
            if active {
                if (ex & noact) != 0 {
                    SetWindowLongPtrW(self.hwnd, GWL_EXSTYLE, ex & !noact);
                }
                let _ = SetForegroundWindow(self.hwnd);
                let _ = SetFocus(self.hwnd);
            } else if (ex & noact) == 0 {
                SetWindowLongPtrW(self.hwnd, GWL_EXSTYLE, ex | noact);
            }
        }
    }

    pub fn dpi(&self) -> f32 {
        self.shared.borrow().dpi
    }

    pub fn dpi_scale(&self) -> f32 {
        self.shared.borrow().dpi_scale()
    }

    pub fn refresh_monitor(&mut self, monitor_index: usize) {
        self.monitor = select_monitor(monitor_index);
        let actual_dpi = unsafe { GetDpiForWindow(self.hwnd) };
        let dpi = if actual_dpi > 0 {
            actual_dpi as f32
        } else {
            self.monitor.dpi
        };
        self.shared.borrow_mut().dpi = dpi;
    }

    pub fn update_bounds(
        &mut self,
        cur_w_dips: f32,
        cur_h_dips: f32,
        radius_dips: f32,
        ear_r_dips: f32,
        _top_offset_dips: f32,
    ) {
        {
            let mut s = self.shared.borrow_mut();
            s.pill_w_dips = cur_w_dips;
            s.pill_h_dips = cur_h_dips;
            s.pill_radius_dips = radius_dips;
            s.ear_radius_dips = ear_r_dips;
        }
        sync_hit_test_cursor_shared(self.hwnd, &self.shared);
    }

    pub fn sync_hit_test_cursor(&self) {
        sync_hit_test_cursor_shared(self.hwnd, &self.shared);
    }

    pub fn set_expanded_keep_open(&self, expanded: bool) {
        self.shared.borrow_mut().expanded_keep_open = expanded;
    }

    pub fn register_ai_hotkey(&self, enabled: bool, hotkey_idx: u8) {
        unsafe {
            let _ = UnregisterHotKey(self.hwnd, AI_HOTKEY_ID);
            if enabled {
                let idx = (hotkey_idx as usize).min(crate::ai::AI_HOTKEYS.len() - 1);
                let (_, mods, vk) = crate::ai::AI_HOTKEYS[idx];
                let _ = RegisterHotKey(self.hwnd, AI_HOTKEY_ID, mods, vk);
            }
        }
    }

    pub fn register_ocr_hotkey(&self) {
        unsafe {
            let _ = UnregisterHotKey(self.hwnd, OCR_HOTKEY_ID);
            let _ = RegisterHotKey(self.hwnd, OCR_HOTKEY_ID, 0x0001 | 0x0002 | 0x4000, 0x58);
        }
    }

    pub fn register_hide_fullscreen_hotkey(&self) {
        unsafe {
            let _ = UnregisterHotKey(self.hwnd, HIDE_FS_HOTKEY_ID);
            let _ = RegisterHotKey(self.hwnd, HIDE_FS_HOTKEY_ID, 0x0001 | 0x0002 | 0x4000, 0x48);
        }
    }

    pub fn set_extra_hit_box(&self, box_dips: Option<(f32, f32, f32)>) {
        self.shared.borrow_mut().extra_hit_box = box_dips;
    }

    /// Returns `true` when the canvas HWND is currently in click-through (`WS_EX_TRANSPARENT`) mode.
    #[allow(dead_code)]
    pub fn is_click_through(&self) -> bool {
        self.shared.borrow().click_through
    }

    /// Repositions and resizes the stationary canvas HWND after DPI, display, or config changes.
    pub fn reposition_canvas(
        &mut self,
        canvas_w_dips: f32,
        canvas_h_dips: f32,
        top_offset_dips: f32,
        horizontal_offset_dips: f32,
    ) {
        self.shared.borrow_mut().canvas_w_dips = canvas_w_dips;
        let scale = self.dpi_scale();
        let canvas_w_px = ((canvas_w_dips * scale).ceil().max(2.0) as i32 + 1) & !1;
        let canvas_h_px = (canvas_h_dips * scale).ceil().max(2.0) as i32;
        let (x_px, y_px) = centered_top_pos(
            &self.monitor,
            canvas_w_px,
            top_offset_dips,
            horizontal_offset_dips,
        );

        let next = (x_px, y_px, canvas_w_px, canvas_h_px);
        if next != self.last_bounds_px {
            unsafe {
                let _ = SetWindowPos(
                    self.hwnd,
                    None,
                    x_px,
                    y_px,
                    canvas_w_px,
                    canvas_h_px,
                    SWP_NOACTIVATE | SWP_NOZORDER | SWP_NOREDRAW,
                );
            }
            self.last_bounds_px = next;
        }
        sync_hit_test_cursor_shared(self.hwnd, &self.shared);
    }

    pub fn is_fullscreen_active_now(&self, hide_fullscreen: bool) -> bool {
        if hide_fullscreen {
            is_fullscreen_active(self.hwnd, &self.monitor.rect)
        } else {
            false
        }
    }

    #[allow(dead_code)]
    pub fn update_fullscreen_visibility(&mut self, hide_fullscreen: bool) -> bool {
        let should_hide = if hide_fullscreen {
            is_fullscreen_active(self.hwnd, &self.monitor.rect)
        } else {
            false
        };

        if should_hide != self.fullscreen_hidden {
            self.fullscreen_hidden = should_hide;
            self.set_visible(!should_hide);
        }
        self.fullscreen_hidden
    }

    pub fn sync_tray_menu_state(&self, state: TrayMenuState) {
        self.shared.borrow_mut().tray_menu = state;
    }

    pub fn set_visible(&mut self, visible: bool) {
        if self.visible != visible {
            self.visible = visible;
            {
                let mut s = self.shared.borrow_mut();
                s.visible = visible;
                if !visible && (s.tracking_mouse || s.cursor_inside_notified) {
                    s.tracking_mouse = false;
                    s.cursor_inside_notified = false;
                    s.events.push(WindowEvent::MouseLeave);
                }
            }
            unsafe {
                let _ = ShowWindow(
                    self.hwnd,
                    if visible { SW_SHOWNOACTIVATE } else { SW_HIDE },
                );
            }
            sync_hit_test_cursor_shared(self.hwnd, &self.shared);
        }
    }

    pub fn pump_events(&self, out: &mut Vec<WindowEvent>) -> bool {
        let mut msg = MSG::default();
        let mut quit = false;
        unsafe {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_QUIT {
                    quit = true;
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        sync_hit_test_cursor_shared(self.hwnd, &self.shared);
        let mut s = self.shared.borrow_mut();
        out.append(&mut s.events);
        !quit
    }

    pub fn wait_and_pump(&self, out: &mut Vec<WindowEvent>) -> bool {
        loop {
            let mut msg = MSG::default();
            let mut had_non_cursor_msg;
            let mut quit = false;
            unsafe {
                let ret = GetMessageW(&mut msg, None, 0, 0);
                if ret.0 <= 0 {
                    out.push(WindowEvent::Quit);
                    return false;
                }
                had_non_cursor_msg = !is_background_filter_msg(&msg);
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);

                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    if msg.message == WM_QUIT {
                        quit = true;
                    }
                    if !is_background_filter_msg(&msg) {
                        had_non_cursor_msg = true;
                    }
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            sync_hit_test_cursor_shared(self.hwnd, &self.shared);
            {
                let mut s = self.shared.borrow_mut();
                out.append(&mut s.events);
            }
            if quit {
                return false;
            }
            if had_non_cursor_msg || !out.is_empty() {
                return true;
            }
        }
    }
}

impl Drop for NotchWindow {
    fn drop(&mut self) {
        if !self.hwnd.0.is_null() {
            unsafe {
                if IsWindow(self.hwnd).as_bool() {
                    let _ = DestroyWindow(self.hwnd);
                }
            }
            self.hwnd = HWND(std::ptr::null_mut());
        }
    }
}

pub fn is_point_in_notch(
    x: f32,
    y: f32,
    total_w: f32,
    h: f32,
    ear_r: f32,
    bottom_r: f32,
) -> bool {
    if x < 0.0 || y < 0.0 || x > total_w || y > h {
        return false;
    }
    let ear_r = ear_r.clamp(0.0, total_w * 0.25);
    let body_w = (total_w - 2.0 * ear_r).max(1.0);
    let bottom_r = bottom_r.clamp(0.0, body_w.min(h) * 0.5);

    if x < ear_r {
        if y < ear_r {
            let dy = y - ear_r;
            return x * x + dy * dy >= ear_r * ear_r;
        }
        return false;
    }
    if x > total_w - ear_r {
        if y < ear_r {
            let dx = x - total_w;
            let dy = y - ear_r;
            return dx * dx + dy * dy >= ear_r * ear_r;
        }
        return false;
    }

    if y > h - bottom_r {
        let left_cx = ear_r + bottom_r;
        let right_cx = total_w - ear_r - bottom_r;
        let cy = h - bottom_r;
        if x < left_cx {
            let dx = x - left_cx;
            let dy = y - cy;
            return dx * dx + dy * dy <= bottom_r * bottom_r;
        }
        if x > right_cx {
            let dx = x - right_cx;
            let dy = y - cy;
            return dx * dx + dy * dy <= bottom_r * bottom_r;
        }
    }
    true
}

fn centered_top_pos(
    monitor: &MonitorRect,
    canvas_w_px: i32,
    top_offset_dips: f32,
    horizontal_offset_dips: f32,
) -> (i32, i32) {
    let scale = monitor.dpi_scale();
    let x = monitor.rect.left
        + (monitor.width_px() - canvas_w_px) / 2
        + (horizontal_offset_dips * scale).round() as i32;
    let y = monitor.rect.top + (top_offset_dips * scale).round() as i32;
    (x, y)
}

pub fn select_monitor(monitor_index: usize) -> MonitorRect {
    let mut list: Vec<MonitorRect> = Vec::new();
    unsafe {
        let ptr = &mut list as *mut Vec<MonitorRect> as isize;
        let _ = EnumDisplayMonitors(None, None, Some(enum_monitor_cb), LPARAM(ptr));
    }
    list.sort_by_key(|m| if m.is_primary { 0 } else { 1 });

    if list.is_empty() {
        return MonitorRect {
            rect: RECT {
                left: 0,
                top: 0,
                right: 1920,
                bottom: 1080,
            },
            dpi: 96.0,
            is_primary: true,
        };
    }

    list.get(monitor_index).copied().unwrap_or(list[0])
}

unsafe extern "system" fn enum_monitor_cb(
    hmonitor: HMONITOR,
    _hdc: HDC,
    _rect: *mut RECT,
    lparam: LPARAM,
) -> BOOL {
    let list = &mut *(lparam.0 as *mut Vec<MonitorRect>);
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if GetMonitorInfoW(hmonitor, &mut info).as_bool() {
        let mut dpi_x: u32 = 96;
        let mut dpi_y: u32 = 96;
        let _ = GetDpiForMonitor(hmonitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
        list.push(MonitorRect {
            rect: info.rcMonitor,
            dpi: dpi_x.max(72) as f32,
            is_primary: (info.dwFlags & MONITORINFOF_PRIMARY_FLAG) != 0,
        });
    }
    BOOL(1)
}

fn is_fullscreen_active(own_hwnd: HWND, monitor_rect: &RECT) -> bool {
    unsafe {
        let fg = GetForegroundWindow();
        if fg.0.is_null()
            || fg == own_hwnd
            || !IsWindowVisible(fg).as_bool()
            || is_desktop_shell_window(fg)
        {
            return false;
        }

        if let Ok(state) = SHQueryUserNotificationState() {
            if state == QUNS_RUNNING_D3D_FULL_SCREEN || state == QUNS_PRESENTATION_MODE {
                return true;
            }
        }

        let mut fg_rect = RECT::default();
        if GetWindowRect(fg, &mut fg_rect).is_ok() {
            let covers = fg_rect.left <= monitor_rect.left
                && fg_rect.top <= monitor_rect.top
                && fg_rect.right >= monitor_rect.right
                && fg_rect.bottom >= monitor_rect.bottom;
            if covers {
                return true;
            }
        }
    }
    false
}

unsafe fn is_desktop_shell_window(hwnd: HWND) -> bool {
    let mut buf = [0u16; 64];
    let len = GetClassNameW(hwnd, &mut buf);
    if len <= 0 {
        return false;
    }
    let name = String::from_utf16_lossy(&buf[..len as usize]);
    matches!(
        name.as_str(),
        "Progman"
            | "WorkerW"
            | "Shell_TrayWnd"
            | "Shell_SecondaryTrayWnd"
            | "SHELLDLL_DefView"
            | "SysListView32"
            | "XamlExplorerHostIslandWindow"
    )
}

pub fn is_caps_lock_on() -> bool {
    unsafe { (GetKeyState(VK_CAPITAL.0 as i32) & 1) != 0 }
}

unsafe fn register_raw_keyboard(hwnd: HWND) {
    let rid = [RAWINPUTDEVICE {
        usUsagePage: 0x01,
        usUsage: 0x06,
        dwFlags: RIDEV_INPUTSINK,
        hwndTarget: hwnd,
    }];
    let _ = RegisterRawInputDevices(&rid, size_of::<RAWINPUTDEVICE>() as u32);
}

#[inline]
fn is_raw_caps_lock_keyboard(raw: &RAWINPUT) -> Option<bool> {
    if raw.header.dwType != RIM_TYPEKEYBOARD.0 {
        return None;
    }
    let kb = unsafe { raw.data.keyboard };
    if kb.VKey == VK_CAPITAL.0 {
        let is_break = (kb.Flags & RI_KEY_BREAK as u16) != 0;
        Some(!is_break)
    } else {
        None
    }
}

#[inline]
fn is_background_filter_msg(msg: &MSG) -> bool {
    (msg.message == WM_TIMER && msg.wParam.0 == CURSOR_POLL_TIMER_ID) || msg.message == WM_INPUT
}

#[inline]
fn compute_ex_style_for_click_through(current_ex: isize, click_through: bool) -> isize {
    let pass_bits = (WS_EX_LAYERED | WS_EX_TRANSPARENT).0 as isize;
    if click_through {
        current_ex | pass_bits
    } else {
        current_ex & !pass_bits
    }
}

#[inline]
#[allow(clippy::too_many_arguments)]
fn should_be_click_through(
    visible: bool,
    has_capture: bool,
    local_x: f32,
    local_y: f32,
    pill_w_dips: f32,
    pill_h_dips: f32,
    ear_radius_dips: f32,
    pill_radius_dips: f32,
) -> bool {
    if !visible {
        return true;
    }
    if has_capture {
        return false;
    }
    !is_point_in_notch(
        local_x,
        local_y,
        pill_w_dips,
        pill_h_dips,
        ear_radius_dips,
        pill_radius_dips,
    )
}

const OPEN_EDGE_MARGIN_X_DIPS: f32 = 28.0;
const OPEN_EDGE_MARGIN_BOTTOM_DIPS: f32 = 26.0;

#[inline]
fn is_point_in_open_margin(
    x_dips: f32,
    y_dips: f32,
    pill_w_dips: f32,
    pill_h_dips: f32,
    ear_radius_dips: f32,
) -> bool {
    let ear_r = ear_radius_dips.clamp(3.0, pill_w_dips * 0.22);
    let body_left = ear_r;
    let body_right = pill_w_dips - ear_r;
    x_dips >= body_left - OPEN_EDGE_MARGIN_X_DIPS
        && x_dips <= body_right + OPEN_EDGE_MARGIN_X_DIPS
        && y_dips >= -8.0
        && y_dips <= pill_h_dips + OPEN_EDGE_MARGIN_BOTTOM_DIPS
}

fn apply_click_through(hwnd: HWND, s: &mut WindowShared, click_through: bool, keep_hover: bool) {
    if s.click_through == click_through {
        return;
    }
    s.click_through = click_through;

    unsafe {
        let cur_ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let next_ex = compute_ex_style_for_click_through(cur_ex, click_through);
        if next_ex != cur_ex {
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, next_ex);
        }
        if click_through {
            // Ensure DWM treats layered window alpha as fully opaque (per-pixel alpha comes from DComp/DXGI).
            let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA);
        }
    }

    if click_through {
        if !keep_hover && (s.tracking_mouse || s.cursor_inside_notified) {
            s.tracking_mouse = false;
            s.cursor_inside_notified = false;
            s.events.push(WindowEvent::MouseLeave);
        }
        if s.visible {
            unsafe {
                let _ = SetTimer(hwnd, CURSOR_POLL_TIMER_ID, CURSOR_POLL_INTERVAL_MS, None);
            }
        }
    } else {
        unsafe {
            let _ = KillTimer(hwnd, CURSOR_POLL_TIMER_ID);
        }
    }
}

fn sync_hit_test_cursor_shared(hwnd: HWND, shared: &RefCell<WindowShared>) {
    if hwnd.0.is_null() {
        return;
    }
    let mut s = shared.borrow_mut();
    if !s.visible {
        unsafe {
            let _ = KillTimer(hwnd, CURSOR_POLL_TIMER_ID);
        }
        apply_click_through(hwnd, &mut s, true, false);
        return;
    }

    let has_capture = unsafe { GetCapture() == hwnd };
    if has_capture {
        apply_click_through(hwnd, &mut s, false, true);
        return;
    }

    let mut pt = POINT::default();
    if unsafe { GetCursorPos(&mut pt) }.is_err() {
        apply_click_through(hwnd, &mut s, true, false);
        return;
    }
    unsafe {
        let _ = ScreenToClient(hwnd, &mut pt);
    }
    let (local_x, local_y) = s.to_notch_local(pt.x, pt.y);
    let in_extra = s.is_point_in_extra_box(pt.x, pt.y);
    let next_ct = if in_extra {
        false
    } else {
        should_be_click_through(
            s.visible,
            false,
            local_x,
            local_y,
            s.pill_w_dips,
            s.pill_h_dips,
            s.ear_radius_dips,
            s.pill_radius_dips,
        )
    };
    let keep_hover = s.expanded_keep_open
        && is_point_in_open_margin(
            local_x,
            local_y,
            s.pill_w_dips,
            s.pill_h_dips,
            s.ear_radius_dips,
        );
    let was_ct = s.click_through;
    apply_click_through(hwnd, &mut s, next_ct, keep_hover);
    if next_ct && !keep_hover && (s.tracking_mouse || s.cursor_inside_notified) {
        s.tracking_mouse = false;
        s.cursor_inside_notified = false;
        s.events.push(WindowEvent::MouseLeave);
    } else if was_ct && !next_ct {
        s.cursor_inside_notified = true;
        s.events.push(WindowEvent::MouseMove {
            x_dips: local_x,
            y_dips: local_y,
        });
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_CREATE {
        let cs = &*(lparam.0 as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, cs.lpCreateParams as isize);
        let shared = &*(cs.lpCreateParams as *const RefCell<WindowShared>);
        let mut s = shared.borrow_mut();
        install_tray_icon(hwnd, &mut s);
        return LRESULT(0);
    }

    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const RefCell<WindowShared>;
    if ptr.is_null() {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }

    match msg {
        WM_SETCURSOR => {
            let hit_test = (lparam.0 & 0xFFFF) as u16 as u32;
            if hit_test == HTCLIENT {
                if let Ok(arrow) = LoadCursorW(None, IDC_ARROW) {
                    SetCursor(arrow);
                }
                return LRESULT(1);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_NCHITTEST => {
            let screen_x = (lparam.0 & 0xFFFF) as i16 as i32;
            let screen_y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
            let mut pt = POINT {
                x: screen_x,
                y: screen_y,
            };
            let _ = ScreenToClient(hwnd, &mut pt);
            let mut s = (&*ptr).borrow_mut();
            let (local_x, local_y) = s.to_notch_local(pt.x, pt.y);
            if s.is_point_in_extra_box(pt.x, pt.y)
                || is_point_in_notch(
                    local_x,
                    local_y,
                    s.pill_w_dips,
                    s.pill_h_dips,
                    s.ear_radius_dips,
                    s.pill_radius_dips,
                )
            {
                LRESULT(HTCLIENT as isize)
            } else {
                let keep_hover = s.expanded_keep_open
                    && is_point_in_open_margin(
                        local_x,
                        local_y,
                        s.pill_w_dips,
                        s.pill_h_dips,
                        s.ear_radius_dips,
                    );
                if keep_hover {
                    if s.tracking_mouse {
                        s.tracking_mouse = false;
                        s.cursor_inside_notified = true;
                        s.events.push(WindowEvent::MouseMove {
                            x_dips: local_x,
                            y_dips: local_y,
                        });
                    }
                } else if s.tracking_mouse || s.cursor_inside_notified {
                    s.tracking_mouse = false;
                    s.cursor_inside_notified = false;
                    s.events.push(WindowEvent::MouseLeave);
                }
                if !s.click_through && !s.sync_posted && GetCapture() != hwnd {
                    s.sync_posted = true;
                    let _ = PostMessageW(hwnd, WM_SYNC_HITTEST, WPARAM(0), LPARAM(0));
                }
                LRESULT(HTTRANSPARENT as isize)
            }
        }
        WM_MOUSEMOVE => {
            let px_x = (lparam.0 & 0xFFFF) as i16 as i32;
            let px_y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
            let mut s = (&*ptr).borrow_mut();
            if !s.tracking_mouse {
                let mut tme = TRACKMOUSEEVENT {
                    cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                if TrackMouseEvent(&mut tme).is_ok() {
                    s.tracking_mouse = true;
                }
            }
            let (local_x, local_y) = s.to_notch_local(px_x, px_y);
            s.cursor_inside_notified = true;
            s.events.push(WindowEvent::MouseMove {
                x_dips: local_x,
                y_dips: local_y,
            });
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            {
                let mut s = (&*ptr).borrow_mut();
                let mut pt = POINT::default();
                let keep_hover = GetCursorPos(&mut pt).is_ok() && {
                    let _ = ScreenToClient(hwnd, &mut pt);
                    s.is_point_in_extra_box(pt.x, pt.y)
                        || (s.expanded_keep_open && {
                            let (lx, ly) = s.to_notch_local(pt.x, pt.y);
                            is_point_in_open_margin(
                                lx,
                                ly,
                                s.pill_w_dips,
                                s.pill_h_dips,
                                s.ear_radius_dips,
                            )
                        })
                };
                if keep_hover {
                    s.tracking_mouse = false;
                    s.cursor_inside_notified = true;
                } else if s.tracking_mouse || s.cursor_inside_notified {
                    s.tracking_mouse = false;
                    s.cursor_inside_notified = false;
                    s.events.push(WindowEvent::MouseLeave);
                }
            }
            sync_hit_test_cursor_shared(hwnd, &*ptr);
            LRESULT(0)
        }
        WM_SYNC_HITTEST => {
            (&*ptr).borrow_mut().sync_posted = false;
            sync_hit_test_cursor_shared(hwnd, &*ptr);
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let px_x = (lparam.0 & 0xFFFF) as i16 as i32;
            let px_y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
            {
                let mut s = (&*ptr).borrow_mut();
                let (local_x, local_y) = s.to_notch_local(px_x, px_y);
                s.events.push(WindowEvent::LeftDown {
                    x_dips: local_x,
                    y_dips: local_y,
                });
            }
            SetCapture(hwnd);
            LRESULT(0)
        }
        WM_LBUTTONUP_CONST => {
            let px_x = (lparam.0 & 0xFFFF) as i16 as i32;
            let px_y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
            {
                let mut s = (&*ptr).borrow_mut();
                let (local_x, local_y) = s.to_notch_local(px_x, px_y);
                s.events.push(WindowEvent::LeftUp {
                    x_dips: local_x,
                    y_dips: local_y,
                });
            }
            let _ = ReleaseCapture();
            sync_hit_test_cursor_shared(hwnd, &*ptr);
            LRESULT(0)
        }
        WM_CAPTURECHANGED_CONST => {
            if lparam.0 != hwnd.0 as isize {
                (&*ptr).borrow_mut().events.push(WindowEvent::LeftUp {
                    x_dips: -10000.0,
                    y_dips: -10000.0,
                });
                sync_hit_test_cursor_shared(hwnd, &*ptr);
            }
            LRESULT(0)
        }
        WM_MOUSEWHEEL_CONST => {
            let delta = ((wparam.0 >> 16) as i16) as i32;
            (&*ptr)
                .borrow_mut()
                .events
                .push(WindowEvent::MouseWheel { delta });
            LRESULT(0)
        }
        WM_CHAR_CONST => {
            if let Some(ch) = char::from_u32(wparam.0 as u32) {
                (&*ptr).borrow_mut().events.push(WindowEvent::KeyChar(ch));
            }
            LRESULT(0)
        }
        WM_KEYDOWN_CONST => {
            (&*ptr).borrow_mut().events.push(WindowEvent::KeySpecial {
                vk: (wparam.0 & 0xFFFF) as u16,
            });
            LRESULT(0)
        }
        WM_HOTKEY_CONST => {
            if wparam.0 as i32 == AI_HOTKEY_ID {
                (&*ptr).borrow_mut().events.push(WindowEvent::AiHotkey);
            } else if wparam.0 as i32 == OCR_HOTKEY_ID {
                (&*ptr).borrow_mut().events.push(WindowEvent::OcrHotkey);
            } else if wparam.0 as i32 == HIDE_FS_HOTKEY_ID {
                (&*ptr)
                    .borrow_mut()
                    .events
                    .push(WindowEvent::ToggleHideFullscreenHotkey);
            }
            LRESULT(0)
        }
        WM_ACTIVATE_CONST => {
            if (wparam.0 & 0xFFFF) == 0 {
                (&*ptr).borrow_mut().events.push(WindowEvent::FocusLost);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_INPUT => {
            let mut size = size_of::<RAWINPUT>() as u32;
            let mut raw = RAWINPUT::default();
            let got = GetRawInputData(
                HRAWINPUT(lparam.0 as *mut std::ffi::c_void),
                RID_INPUT,
                Some(&mut raw as *mut RAWINPUT as *mut std::ffi::c_void),
                &mut size,
                size_of::<RAWINPUTHEADER>() as u32,
            );
            if got != u32::MAX {
                if let Some(is_down) = is_raw_caps_lock_keyboard(&raw) {
                    (&*ptr).borrow_mut().events.push(WindowEvent::CapsLockCheck);
                    if is_down {
                        let _ = SetTimer(hwnd, CAPS_CHECK_TIMER_ID, CAPS_CHECK_DELAY_MS, None);
                    }
                } else if raw.header.dwType == RIM_TYPEKEYBOARD.0 {
                    let kb = raw.data.keyboard;
                    let is_down = (kb.Flags & RI_KEY_BREAK as u16) == 0;
                    if is_down {
                        const VK_VOL_DOWN: u16 = 0xAE;
                        const VK_VOL_UP: u16 = 0xAF;
                        if kb.VKey == VK_VOL_UP {
                            (&*ptr)
                                .borrow_mut()
                                .events
                                .push(WindowEvent::VolumeKey { up: true });
                        } else if kb.VKey == VK_VOL_DOWN {
                            (&*ptr)
                                .borrow_mut()
                                .events
                                .push(WindowEvent::VolumeKey { up: false });
                        }
                    }
                }
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_DPICHANGED => {
            let new_dpi = (wparam.0 & 0xFFFF) as u16 as f32;
            if new_dpi > 0.0 {
                let mut s = (&*ptr).borrow_mut();
                s.dpi = new_dpi;
                s.events.push(WindowEvent::DpiChanged(new_dpi));
            }
            LRESULT(0)
        }
        WM_DISPLAYCHANGE => {
            (&*ptr).borrow_mut().events.push(WindowEvent::DisplayChanged);
            LRESULT(0)
        }
        WM_TIMER => {
            if wparam.0 == IDLE_TIMER_ID {
                (&*ptr).borrow_mut().events.push(WindowEvent::TimerTick);
            } else if wparam.0 == CURSOR_POLL_TIMER_ID {
                sync_hit_test_cursor_shared(hwnd, &*ptr);
            } else if wparam.0 == CAPS_CHECK_TIMER_ID {
                let _ = KillTimer(hwnd, CAPS_CHECK_TIMER_ID);
                (&*ptr).borrow_mut().events.push(WindowEvent::CapsLockCheck);
            }
            LRESULT(0)
        }
        WM_TRAY_CALLBACK => {
            let tray_msg = (lparam.0 & 0xFFFF) as u32;
            if tray_msg == WM_RBUTTONUP {
                let menu_state = (&*ptr).borrow().tray_menu;
                show_tray_menu(hwnd, menu_state);
            } else if tray_msg == WM_LBUTTONUP_CONST {
                (&*ptr)
                    .borrow_mut()
                    .events
                    .push(WindowEvent::TrayAction(TrayAction::ResetPosition));
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            let cmd = wparam.0 & 0xFFFF;
            let action = match cmd {
                IDM_SETTINGS => Some(TrayAction::OpenSettings),
                IDM_TOGGLE_STARTUP => Some(TrayAction::ToggleStartup),
                IDM_TOGGLE_MASCOT => Some(TrayAction::ToggleMascot),
                IDM_TOGGLE_MINIMAL => Some(TrayAction::ToggleMinimal),
                IDM_SIZE_COMPACT => Some(TrayAction::SetNotchSize(0)),
                IDM_SIZE_STANDARD => Some(TrayAction::SetNotchSize(1)),
                IDM_SIZE_WIDE => Some(TrayAction::SetNotchSize(2)),
                IDM_EXIT => Some(TrayAction::Exit),
                _ => None,
            };
            if let Some(a) = action {
                (&*ptr).borrow_mut().events.push(WindowEvent::TrayAction(a));
                if a == TrayAction::Exit {
                    PostQuitMessage(0);
                }
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            let _ = KillTimer(hwnd, IDLE_TIMER_ID);
            let _ = KillTimer(hwnd, CURSOR_POLL_TIMER_ID);
            let _ = KillTimer(hwnd, CAPS_CHECK_TIMER_ID);
            let mut s = (&*ptr).borrow_mut();
            remove_tray_icon(hwnd, &mut s);
            drop(s);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            let _ = Rc::from_raw(ptr);
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe fn enable_dark_context_menus() {
    if let Ok(hmod) = LoadLibraryW(w!("uxtheme.dll")) {
        if let Some(proc) = GetProcAddress(hmod, windows::core::PCSTR(135usize as *const u8)) {
            let set_mode: unsafe extern "system" fn(i32) -> i32 = std::mem::transmute(proc);
            let _ = set_mode(1);
            let _ = set_mode(2);
        }
        if let Some(proc) = GetProcAddress(hmod, windows::core::PCSTR(136usize as *const u8)) {
            let flush: unsafe extern "system" fn() = std::mem::transmute(proc);
            flush();
        }
    }
}

unsafe fn show_tray_menu(hwnd: HWND, st: TrayMenuState) {
    let Ok(hmenu) = CreatePopupMenu() else {
        return;
    };
    let check_flag = |on: bool| if on { MF_STRING | MF_CHECKED } else { MF_STRING | MF_UNCHECKED };

    let _ = AppendMenuW(hmenu, MF_STRING, IDM_SETTINGS, w!("Settings..."));
    let _ = AppendMenuW(hmenu, MF_SEPARATOR, 0, PCWSTR::null());
    let _ = AppendMenuW(
        hmenu,
        check_flag(st.start_with_windows),
        IDM_TOGGLE_STARTUP,
        w!("Start with Windows"),
    );
    let _ = AppendMenuW(
        hmenu,
        check_flag(st.show_mascot),
        IDM_TOGGLE_MASCOT,
        w!("Eave Mascot"),
    );
    let _ = AppendMenuW(
        hmenu,
        check_flag(st.minimal_mode),
        IDM_TOGGLE_MINIMAL,
        w!("Minimal notch"),
    );

    if let Ok(sub) = CreatePopupMenu() {
        let _ = AppendMenuW(sub, check_flag(st.size_idx == 0), IDM_SIZE_COMPACT, w!("Compact"));
        let _ = AppendMenuW(sub, check_flag(st.size_idx == 1), IDM_SIZE_STANDARD, w!("Standard"));
        let _ = AppendMenuW(sub, check_flag(st.size_idx == 2), IDM_SIZE_WIDE, w!("Wide"));
        let _ = AppendMenuW(hmenu, MF_POPUP, sub.0 as usize, w!("Notch size"));
    }

    let _ = AppendMenuW(hmenu, MF_SEPARATOR, 0, PCWSTR::null());
    let _ = AppendMenuW(hmenu, MF_STRING, IDM_EXIT, w!("Quit"));

    let mut pt = POINT::default();
    let _ = GetCursorPos(&mut pt);
    let _ = SetForegroundWindow(hwnd);
    let _ = TrackPopupMenu(
        hmenu,
        TPM_RIGHTBUTTON | TPM_BOTTOMALIGN | TPM_LEFTALIGN,
        pt.x,
        pt.y,
        0,
        hwnd,
        None,
    );
    let _ = DestroyMenu(hmenu);
}

unsafe fn install_tray_icon(hwnd: HWND, shared: &mut WindowShared) {
    let hinst = GetModuleHandleW(None).ok();
    let icon = hinst
        .and_then(|h| LoadIconW(h, PCWSTR(1 as *const u16)).ok())
        .or_else(|| create_pill_tray_icon())
        .or_else(|| LoadIconW(None, IDI_APPLICATION).ok());
    let mut nid = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: TRAY_ICON_ID,
        uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
        uCallbackMessage: WM_TRAY_CALLBACK,
        hIcon: icon.unwrap_or_default(),
        ..Default::default()
    };

    let tip = "Eave";
    for (dst, src) in nid.szTip.iter_mut().zip(tip.encode_utf16()) {
        *dst = src;
    }

    let ok = Shell_NotifyIconW(NIM_ADD, &nid).as_bool();
    shared.tray_icon = icon;
    shared.tray_added = ok;
}

unsafe fn remove_tray_icon(hwnd: HWND, shared: &mut WindowShared) {
    if shared.tray_added {
        let nid = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: TRAY_ICON_ID,
            ..Default::default()
        };
        let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
        shared.tray_added = false;
    }
    if let Some(icon) = shared.tray_icon.take() {
        let _ = DestroyIcon(icon);
    }
}

unsafe fn create_pill_tray_icon() -> Option<HICON> {
    const SIZE: i32 = 16;
    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: SIZE,
            biHeight: -SIZE,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };

    let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
    let color_bmp: HBITMAP =
        CreateDIBSection(None, &bmi, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;

    if bits.is_null() {
        let _ = DeleteObject(color_bmp);
        return None;
    }

    let pixels = std::slice::from_raw_parts_mut(bits as *mut u32, (SIZE * SIZE) as usize);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let fx = x as f32 + 0.5 - 8.0;
            let fy = y as f32 + 0.5 - 8.0;
            let dx = (fx.abs() - 3.0).max(0.0);
            let dist = (dx * dx + fy * fy).sqrt();
            let px = if dist <= 3.6 {
                let border = dist > 2.6;
                let eye = ((fx + 2.2).powi(2) + fy.powi(2)).sqrt() < 1.05
                    || ((fx - 2.2).powi(2) + fy.powi(2)).sqrt() < 1.05;
                if eye {
                    0xFF_F1_67_88
                } else if border {
                    0xFF_48_4A_52
                } else {
                    0xFF_00_00_00
                }
            } else {
                0x00_00_00_00
            };
            pixels[(y * SIZE + x) as usize] = px;
        }
    }

    let mask_bmp = CreateBitmap(SIZE, SIZE, 1, 1, None);
    let info = ICONINFO {
        fIcon: BOOL(1),
        xHotspot: 0,
        yHotspot: 0,
        hbmMask: mask_bmp,
        hbmColor: color_bmp,
    };
    let icon = CreateIconIndirect(&info).ok();
    let _ = DeleteObject(color_bmp);
    let _ = DeleteObject(mask_bmp);
    icon
}

pub fn is_backdrop_mostly_dark(rgb_samples: &[(u8, u8, u8)]) -> bool {
    if rgb_samples.is_empty() {
        return false;
    }
    let dark = rgb_samples
        .iter()
        .filter(|&&(r, g, b)| {
            let lum = 0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32;
            lum < 18.0 && r <= 24 && g <= 24 && b <= 24
        })
        .count();
    (dark as f32 / rgb_samples.len() as f32) >= 0.70
}

/// behind `hwnd` (excluding layered windows via `SRCCOPY` without `CAPTUREBLT`) and
pub fn detect_dark_backdrop_behind_notch(
    hwnd: HWND,
    notch_left_dips: f32,
    notch_w_dips: f32,
    notch_h_dips: f32,
    dpi: f32,
) -> bool {
    if hwnd.0.is_null() {
        return false;
    }
    let mut wr = RECT::default();
    if unsafe { GetWindowRect(hwnd, &mut wr) }.is_err() {
        return false;
    }
    let scale = (dpi / 96.0).max(0.5);
    let pad = (5.0 * scale).round().max(3.0) as i32;
    let sx = wr.left + (notch_left_dips * scale).round() as i32 - pad;
    let sy = wr.top;
    let cap_w = ((notch_w_dips * scale).round() as i32 + pad * 2).clamp(16, 1600);
    let cap_h = ((notch_h_dips * scale).round() as i32 + pad).clamp(12, 800);

    unsafe {
        let screen_dc = GetDC(HWND::default());
        if screen_dc.is_invalid() {
            return false;
        }
        let mem_dc = CreateCompatibleDC(screen_dc);
        if mem_dc.is_invalid() {
            let _ = ReleaseDC(HWND::default(), screen_dc);
            return false;
        }
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: cap_w,
                biHeight: -cap_h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut core::ffi::c_void = core::ptr::null_mut();
        let Ok(dib) = CreateDIBSection(mem_dc, &bmi, DIB_RGB_COLORS, &mut bits, None, 0) else {
            let _ = DeleteDC(mem_dc);
            let _ = ReleaseDC(HWND::default(), screen_dc);
            return false;
        };
        if dib.is_invalid() || bits.is_null() {
            let _ = DeleteObject(dib);
            let _ = DeleteDC(mem_dc);
            let _ = ReleaseDC(HWND::default(), screen_dc);
            return false;
        }
        let prev = SelectObject(mem_dc, HGDIOBJ(dib.0));
        let blt_ok = BitBlt(mem_dc, 0, 0, cap_w, cap_h, screen_dc, sx, sy, SRCCOPY).is_ok();

        let mut samples = Vec::with_capacity(28);
        if blt_ok {
            let buf = core::slice::from_raw_parts(
                bits as *const u8,
                (cap_w as usize) * (cap_h as usize) * 4,
            );
            let sample_at = |x: i32, y: i32| -> (u8, u8, u8) {
                let cx = x.clamp(0, cap_w - 1) as usize;
                let cy = y.clamp(0, cap_h - 1) as usize;
                let off = (cy * cap_w as usize + cx) * 4;
                (buf[off + 2], buf[off + 1], buf[off])
            };
            let by = cap_h - 2;
            for i in 0..16 {
                let bx = 2 + ((cap_w - 5) * i) / 15;
                samples.push(sample_at(bx, by));
            }
            for i in 0..6 {
                let ey = 2 + ((cap_h - 5) * i) / 5;
                samples.push(sample_at(1, ey));
                samples.push(sample_at(cap_w - 2, ey));
            }
        }

        let _ = SelectObject(mem_dc, prev);
        let _ = DeleteObject(dib);
        let _ = DeleteDC(mem_dc);
        let _ = ReleaseDC(HWND::default(), screen_dc);

        is_backdrop_mostly_dark(&samples)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        centered_top_pos, compute_ex_style_for_click_through, is_background_filter_msg,
        is_caps_lock_on, is_point_in_notch, is_raw_caps_lock_keyboard, should_be_click_through,
        MonitorRect, WindowShared, CAPS_CHECK_TIMER_ID, CURSOR_POLL_TIMER_ID, IDLE_TIMER_ID,
    };
    use std::mem::size_of;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    };
    use std::thread;
    use std::time::Duration;
    use windows::core::w;
    use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::Input::KeyboardAndMouse::VK_CAPITAL;
    use windows::Win32::UI::Input::{RAWINPUT, RIM_TYPEKEYBOARD, RIM_TYPEMOUSE};
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, PeekMessageW,
        RegisterClassExW, SetWindowLongPtrW, TranslateMessage, WindowFromPoint, GWL_EXSTYLE,
        HTCLIENT, MSG, PM_REMOVE, RI_KEY_BREAK, WM_INPUT, WM_MOUSEMOVE, WM_NCHITTEST, WM_TIMER,
        WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_NOREDIRECTIONBITMAP,
        WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP, WS_VISIBLE,
    };

    #[test]
    fn notch_hit_test_respects_ears_and_bottom_corners() {
        let (w, h, ear, bot) = (166.0, 30.0, 10.0, 14.0);
        assert!(is_point_in_notch(83.0, 15.0, w, h, ear, bot));
        assert!(is_point_in_notch(2.0, 0.1, w, h, ear, bot));
        assert!(!is_point_in_notch(2.0, 5.0, w, h, ear, bot));
        assert!(!is_point_in_notch(10.5, 29.5, w, h, ear, bot));
        assert!(!is_point_in_notch(-1.0, 15.0, w, h, ear, bot));
        assert!(!is_point_in_notch(167.0, 15.0, w, h, ear, bot));

        let (ew, eh, eear, ebot) = (438.0, 172.0, 22.0, 32.0);
        assert!(is_point_in_notch(219.0, 86.0, ew, eh, eear, ebot));
        assert!(is_point_in_notch(5.0, 0.3, ew, eh, eear, ebot));
        assert!(!is_point_in_notch(5.0, 10.0, ew, eh, eear, ebot));
        assert!(!is_point_in_notch(23.0, 171.0, ew, eh, eear, ebot));
    }

    #[test]
    fn centered_top_pos_applies_negative_horizontal_offset_with_dpi() {
        let mon_1x = MonitorRect {
            rect: RECT {
                left: 0,
                top: 0,
                right: 1920,
                bottom: 1080,
            },
            dpi: 96.0,
            is_primary: true,
        };
        assert_eq!(centered_top_pos(&mon_1x, 508, 0.0, -460.0), (246, 0));

        let mon_15x = MonitorRect {
            rect: RECT {
                left: 1920,
                top: 0,
                right: 4480,
                bottom: 1440,
            },
            dpi: 144.0,
            is_primary: false,
        };
        assert_eq!(centered_top_pos(&mon_15x, 762, 0.0, -460.0), (2129, 0));
    }

    #[test]
    fn click_through_policy_and_ex_style_bits() {
        assert!(!should_be_click_through(
            true, false, 83.0, 15.0, 166.0, 30.0, 10.0, 14.0
        ));
        assert!(should_be_click_through(
            true, false, 83.0, 45.0, 166.0, 30.0, 10.0, 14.0
        ));
        assert!(!should_be_click_through(
            true, true, 83.0, 45.0, 166.0, 30.0, 10.0, 14.0
        ));
        assert!(should_be_click_through(
            false, true, 83.0, 15.0, 166.0, 30.0, 10.0, 14.0
        ));

        let base =
            (WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_NOREDIRECTIONBITMAP).0
                as isize;
        let ct = compute_ex_style_for_click_through(base, true);
        assert_ne!(ct & (WS_EX_LAYERED.0 as isize), 0);
        assert_ne!(ct & (WS_EX_TRANSPARENT.0 as isize), 0);
        assert_eq!(ct & base, base);

        let interactive = compute_ex_style_for_click_through(ct, false);
        assert_eq!(interactive, base);
    }

    #[test]
    fn to_notch_local_matches_renderer_rounded_center_and_dpi() {
        let s = WindowShared {
            events: Vec::new(),
            dpi: 144.0,
            canvas_w_dips: 687.0,
            pill_w_dips: 166.0,
            pill_h_dips: 30.0,
            pill_radius_dips: 14.0,
            ear_radius_dips: 10.0,
            tracking_mouse: false,
            cursor_inside_notified: false,
            expanded_keep_open: false,
            extra_hit_box: None,
            visible: true,
            click_through: false,
            sync_posted: false,
            tray_icon: None,
            tray_added: false,
            tray_menu: super::TrayMenuState::default(),
        };
        let (lx, ly) = s.to_notch_local((344.0 * 1.5) as i32, (15.0 * 1.5) as i32);
        assert!((lx - 83.0).abs() < 0.5, "lx={lx}");
        assert!((ly - 15.0).abs() < 0.5, "ly={ly}");
    }

    #[test]
    fn noredirection_overlay_passes_through_only_with_layered_and_transparent() {
        unsafe extern "system" fn bg_proc(
            hwnd: HWND,
            msg: u32,
            wparam: WPARAM,
            lparam: LPARAM,
        ) -> LRESULT {
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        unsafe extern "system" fn fg_proc(
            hwnd: HWND,
            msg: u32,
            wparam: WPARAM,
            lparam: LPARAM,
        ) -> LRESULT {
            if msg == WM_NCHITTEST {
                return LRESULT(HTCLIENT as isize);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }

        unsafe {
            let hinst: HINSTANCE = GetModuleHandleW(None).unwrap().into();
            let bg_cls = w!("EaveHitTestBg");
            let fg_cls = w!("EaveHitTestFg");
            let _ = RegisterClassExW(&WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(bg_proc),
                hInstance: hinst,
                lpszClassName: bg_cls,
                ..Default::default()
            });
            let _ = RegisterClassExW(&WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(fg_proc),
                hInstance: hinst,
                lpszClassName: fg_cls,
                ..Default::default()
            });

            let stop = Arc::new(AtomicBool::new(false));
            let stop_bg = Arc::clone(&stop);
            let (tx, rx) = mpsc::channel::<isize>();

            let handle = thread::spawn(move || {
                let bg_hinst: HINSTANCE = GetModuleHandleW(None).unwrap().into();
                let bg = CreateWindowExW(
                    WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                    w!("EaveHitTestBg"),
                    w!("bg"),
                    WS_POPUP | WS_VISIBLE,
                    220,
                    220,
                    120,
                    120,
                    None,
                    None,
                    bg_hinst,
                    None,
                )
                .unwrap();
                let _ = tx.send(bg.0 as isize);
                let mut msg = MSG::default();
                while !stop_bg.load(Ordering::Relaxed) {
                    while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                        let _ = TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                    thread::sleep(Duration::from_millis(5));
                }
                let _ = DestroyWindow(bg);
            });

            let bg_hwnd = HWND(rx.recv().unwrap() as *mut std::ffi::c_void);
            let base_ex =
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_NOREDIRECTIONBITMAP;
            let fg_hwnd = CreateWindowExW(
                base_ex,
                fg_cls,
                w!("fg"),
                WS_POPUP | WS_VISIBLE,
                220,
                220,
                120,
                120,
                None,
                None,
                hinst,
                None,
            )
            .unwrap();

            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }

            let pt = POINT { x: 280, y: 280 };
            assert_eq!(WindowFromPoint(pt), fg_hwnd);

            let ct_style = compute_ex_style_for_click_through(base_ex.0 as isize, true);
            SetWindowLongPtrW(fg_hwnd, GWL_EXSTYLE, ct_style);
            assert_eq!(WindowFromPoint(pt), bg_hwnd);

            let active_style = compute_ex_style_for_click_through(ct_style, false);
            SetWindowLongPtrW(fg_hwnd, GWL_EXSTYLE, active_style);
            assert_eq!(WindowFromPoint(pt), fg_hwnd);

            let _ = DestroyWindow(fg_hwnd);
            stop.store(true, Ordering::Relaxed);
            let _ = handle.join();
        }
    }

    #[test]
    fn raw_caps_lock_and_background_filter_helpers() {
        let mut raw = RAWINPUT::default();
        raw.header.dwType = RIM_TYPEMOUSE.0;
        assert_eq!(is_raw_caps_lock_keyboard(&raw), None);

        raw.header.dwType = RIM_TYPEKEYBOARD.0;
        raw.data.keyboard.VKey = b'A' as u16;
        raw.data.keyboard.Flags = 0;
        assert_eq!(is_raw_caps_lock_keyboard(&raw), None);

        raw.data.keyboard.VKey = VK_CAPITAL.0;
        raw.data.keyboard.Flags = 0;
        assert_eq!(is_raw_caps_lock_keyboard(&raw), Some(true));

        raw.data.keyboard.Flags = RI_KEY_BREAK as u16;
        assert_eq!(is_raw_caps_lock_keyboard(&raw), Some(false));

        let cursor_msg = MSG {
            message: WM_TIMER,
            wParam: WPARAM(CURSOR_POLL_TIMER_ID),
            ..Default::default()
        };
        assert!(is_background_filter_msg(&cursor_msg));

        let raw_msg = MSG {
            message: WM_INPUT,
            ..Default::default()
        };
        assert!(is_background_filter_msg(&raw_msg));

        let caps_timer_msg = MSG {
            message: WM_TIMER,
            wParam: WPARAM(CAPS_CHECK_TIMER_ID),
            ..Default::default()
        };
        assert!(!is_background_filter_msg(&caps_timer_msg));

        let idle_msg = MSG {
            message: WM_TIMER,
            wParam: WPARAM(IDLE_TIMER_ID),
            ..Default::default()
        };
        assert!(!is_background_filter_msg(&idle_msg));

        let move_msg = MSG {
            message: WM_MOUSEMOVE,
            ..Default::default()
        };
        assert!(!is_background_filter_msg(&move_msg));

        let _ = is_caps_lock_on();
    }

    #[test]
    fn open_notch_edge_margin_forgives_near_edge_and_closes_far_outside() {
        let w = 440.0;
        let h = 165.0;
        let ear = 22.0;

        assert!(super::is_point_in_open_margin(ear - 18.0, 80.0, w, h, ear));
        assert!(super::is_point_in_open_margin(w - ear + 18.0, 80.0, w, h, ear));
        assert!(super::is_point_in_open_margin(w * 0.5, h + 18.0, w, h, ear));

        assert!(!super::is_point_in_open_margin(ear - 40.0, 80.0, w, h, ear));
        assert!(!super::is_point_in_open_margin(w - ear + 40.0, 80.0, w, h, ear));
        assert!(!super::is_point_in_open_margin(w * 0.5, h + 40.0, w, h, ear));
    }

    #[test]
    fn dark_backdrop_detection_requires_majority_dark_pixels() {
        assert!(!super::is_backdrop_mostly_dark(&[]));
        let mostly_black = [(0, 0, 0), (8, 8, 10), (12, 12, 14), (10, 10, 10), (240, 240, 240)];
        assert!(super::is_backdrop_mostly_dark(&mostly_black));
        let dark_gray = [(30, 30, 30), (34, 34, 36), (38, 38, 40), (32, 32, 32), (28, 28, 30)];
        assert!(!super::is_backdrop_mostly_dark(&dark_gray));
        let mostly_light = [(220, 225, 235), (180, 190, 200), (10, 10, 12), (200, 200, 200), (15, 15, 18)];
        assert!(!super::is_backdrop_mostly_dark(&mostly_light));
    }
}
