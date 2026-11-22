
use std::cell::RefCell;
use std::collections::HashMap;
use std::mem::ManuallyDrop;
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::Command;
use std::rc::Rc;
use std::time::Instant;

use windows::core::{w, Interface, PCWSTR};
use windows::Foundation::Numerics::Matrix3x2;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_IGNORE, D2D1_BEZIER_SEGMENT, D2D1_COLOR_F, D2D1_FIGURE_BEGIN_FILLED,
    D2D1_FIGURE_BEGIN_HOLLOW, D2D1_FIGURE_END_CLOSED, D2D1_FIGURE_END_OPEN, D2D1_GRADIENT_STOP,
    D2D1_PIXEL_FORMAT, D2D_POINT_2F, D2D_RECT_F, D2D_SIZE_U,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, ID2D1Brush, ID2D1Factory, ID2D1Factory1, ID2D1Geometry,
    ID2D1HwndRenderTarget, ID2D1RenderTarget, ID2D1SolidColorBrush,
    D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_CAP_STYLE_ROUND, D2D1_ELLIPSE,
    D2D1_EXTEND_MODE_CLAMP, D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_GAMMA_2_2,
    D2D1_HWND_RENDER_TARGET_PROPERTIES, D2D1_LAYER_OPTIONS_NONE, D2D1_LAYER_PARAMETERS,
    D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES, D2D1_LINE_JOIN_ROUND,
    D2D1_PRESENT_OPTIONS_IMMEDIATELY, D2D1_RADIAL_GRADIENT_BRUSH_PROPERTIES,
    D2D1_RENDER_TARGET_PROPERTIES, D2D1_ROUNDED_RECT, D2D1_STROKE_STYLE_PROPERTIES,
    D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE,
};
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, IDWriteFactory, IDWriteTextFormat, DWRITE_FACTORY_TYPE_SHARED,
    DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT_BOLD,
    DWRITE_FONT_WEIGHT_MEDIUM, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_WEIGHT_SEMI_BOLD,
    DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_LEADING,
    DWRITE_TEXT_ALIGNMENT_TRAILING, DWRITE_WORD_WRAPPING_NO_WRAP,
};
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE,
    DWMWCP_ROUND,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{
    BeginPaint, EndPaint, InvalidateRect, ScreenToClient, PAINTSTRUCT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT, VK_ESCAPE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect, GetSystemMetrics,
    GetWindowLongPtrW, KillTimer, LoadCursorW, PostMessageW, RegisterClassExW, SendMessageW,
    SetForegroundWindow, SetLayeredWindowAttributes, SetTimer, SetWindowLongPtrW, SetWindowPos,
    ShowWindow, CS_HREDRAW, CS_VREDRAW, GWL_EXSTYLE, GWLP_USERDATA, HTCAPTION, IDC_ARROW,
    LWA_ALPHA, SM_CMONITORS, SM_CXSCREEN, SM_CYSCREEN, SWP_NOZORDER, SWP_SHOWWINDOW, SW_SHOW,
    WM_CLOSE, WM_DESTROY, WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
    WM_MOUSEWHEEL, WM_NCLBUTTONDOWN, WM_PAINT, WM_SIZE, WM_TIMER, WNDCLASSEXW, WS_EX_APPWINDOW,
    WS_EX_LAYERED, WS_POPUP, WS_VISIBLE,
};

use crate::config::Config;
use crate::pages::{COUNTDOWN_TARGETS, WEATHER_CITIES, WORLD_CITIES, WidgetKind};

const SETTINGS_CLASS: PCWSTR = w!("EaveSettingsWindow");
const WIN_W_DIPS: f32 = 632.0;
const WIN_H_DIPS: f32 = 540.0;
const SIDEBAR_W: f32 = 184.0;
const ANIM_TIMER_ID: usize = 901;
const WM_MOUSELEAVE: u32 = 0x02A3;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Clone, Debug)]
pub struct SettingsSnapshot {
    pub cfg: Config,
    pub start_with_windows: bool,
    pub pomo_focus_mins: u32,
    pub pomo_break_mins: u32,
    pub pomo_long_break_mins: u32,
    pub water_goal_ml: u32,
    pub water_bottle_ml: u32,
    pub water_reminder_enabled: bool,
    pub todo_done: usize,
    pub todo_total: usize,
    pub shelf_files_count: usize,
    pub shelf_clip_count: usize,
    pub update_status: crate::updater::UpdateStatus,
}

#[derive(Clone, Debug)]
pub enum SettingsAction {
    UpdateConfig(Config),
    SetPomodoroDurations {
        focus_mins: u32,
        break_mins: u32,
        long_break_mins: u32,
    },
    SetWaterSettings {
        goal_ml: u32,
        bottle_ml: u32,
        reminder_enabled: bool,
    },
    ClearCompletedTodos,
    ClearShelfFiles,
    ClearShelfClipboard,
    OpenConfigFile,
    ResetDefaults,
    CheckForUpdates,
    OpenUpdateUrl(String),
    OpenGitHubRepo,
    TriggerSmartOcr,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
enum SettingsTab {
    General,
    Display,
    Indicators,
    EaveAi,
    Pages,
    Music,
    Widgets,
    Focus,
    Water,
    Todo,
    Shelf,
    About,
}

impl SettingsTab {
    const ALL: [Self; 12] = [
        Self::General,
        Self::Display,
        Self::Indicators,
        Self::EaveAi,
        Self::Pages,
        Self::Music,
        Self::Widgets,
        Self::Focus,
        Self::Water,
        Self::Todo,
        Self::Shelf,
        Self::About,
    ];

    const fn label(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Display => "Display",
            Self::Indicators => "Indicators",
            Self::EaveAi => "Eave AI",
            Self::Pages => "Pages",
            Self::Music => "Music & Lyrics",
            Self::Widgets => "Widgets",
            Self::Focus => "Focus",
            Self::Water => "Water",
            Self::Todo => "To-do",
            Self::Shelf => "Shelf",
            Self::About => "About",
        }
    }

    const fn badge_color(self) -> (f32, f32, f32) {
        match self {
            Self::General => (0.50, 0.50, 0.54),
            Self::Display => (0.31, 0.43, 1.0),
            Self::Indicators => (0.18, 0.78, 0.52),
            Self::EaveAi => (0.62, 0.36, 1.0),
            Self::Pages => (0.19, 0.68, 0.94),
            Self::Music => (1.0, 0.18, 0.34),
            Self::Widgets => (0.08, 0.56, 1.0),
            Self::Focus => (0.55, 0.36, 0.98),
            Self::Water => (0.14, 0.60, 1.0),
            Self::Todo => (0.20, 0.80, 0.36),
            Self::Shelf => (0.96, 0.72, 0.08),
            Self::About => (0.48, 0.48, 0.52),
        }
    }

    fn sidebar_y(self) -> f32 {
        if self == Self::About {
            WIN_H_DIPS - 44.0
        } else {
            let idx = Self::ALL.iter().position(|&t| t == self).unwrap_or(0);
            44.0 + idx as f32 * 35.0
        }
    }
}

#[derive(Clone, Debug)]
enum ControlHit {
    CloseButton,
    Tab(SettingsTab),
    Toggle(&'static str),
    Segment(&'static str, usize),
    Stepper(&'static str, i32),
    PageDrag(usize),
    PageToggle(usize),
    TabReset(SettingsTab),
    RingMode(u8),
    WidgetSlot(usize),
    WidgetPick(u8),
    ActionBtn(&'static str),
}

#[derive(Clone)]
struct HitBox {
    rect: D2D_RECT_F,
    hit: ControlHit,
}

struct GfxResources {
    #[allow(dead_code)]
    d2d_factory: ID2D1Factory1,
    rt: ID2D1HwndRenderTarget,
    brush: ID2D1SolidColorBrush,
    #[allow(dead_code)]
    dwrite: IDWriteFactory,
    fmt_title: IDWriteTextFormat,
    fmt_section: IDWriteTextFormat,
    fmt_sidebar: IDWriteTextFormat,
    fmt_row: IDWriteTextFormat,
    fmt_sub: IDWriteTextFormat,
    fmt_seg: IDWriteTextFormat,
    fmt_seg_on: IDWriteTextFormat,
    fmt_val: IDWriteTextFormat,
    fmt_preview: IDWriteTextFormat,
    eave_icon: Option<windows::Win32::Graphics::Direct2D::ID2D1Bitmap>,
}

struct SettingsInner {
    hwnd: Option<HWND>,
    dpi: f32,
    monitor_count: usize,
    snap: SettingsSnapshot,
    tab: SettingsTab,
    prev_tab: Option<SettingsTab>,
    prev_scroll_y: f32,
    scroll_y: f32,
    scroll_target: f32,
    max_scroll: f32,
    hover_pt: Option<(f32, f32)>,
    last_hover_idx: Option<usize>,
    tracking_mouse: bool,
    page_row_y: [f32; 6],
    dragging_page: Option<(usize, f32, f32)>,
    pages_card_top: f32,
    widget_edit_slot: usize,
    preview_mascot: bool,
    preview_phase: f32,
    win_opacity: f32,
    layered_active: bool,
    closing: bool,
    page_anim: f32,
    sidebar_sel_y: f32,
    anim_vals: HashMap<&'static str, f32>,
    hover_anims: RefCell<HashMap<String, (f32, f32)>>,
    last_anim_tick: Instant,
    anim_running: bool,
    hit_boxes: Vec<HitBox>,
    actions: Vec<SettingsAction>,
    gfx: Option<GfxResources>,
}

pub struct SettingsWindow {
    inner: Rc<RefCell<SettingsInner>>,
    class_registered: bool,
}

impl SettingsWindow {
    pub fn new() -> Self {
        let dummy = SettingsSnapshot {
            cfg: Config::default(),
            start_with_windows: false,
            pomo_focus_mins: 25,
            pomo_break_mins: 5,
            pomo_long_break_mins: 15,
            water_goal_ml: 2000,
            water_bottle_ml: 250,
            water_reminder_enabled: false,
            todo_done: 0,
            todo_total: 0,
            shelf_files_count: 0,
            shelf_clip_count: 0,
            update_status: crate::updater::UpdateStatus::Idle,
        };
        Self {
            inner: Rc::new(RefCell::new(SettingsInner {
                hwnd: None,
                dpi: 96.0,
                monitor_count: 1,
                snap: dummy,
                tab: SettingsTab::General,
                prev_tab: None,
                prev_scroll_y: 0.0,
                scroll_y: 0.0,
                scroll_target: 0.0,
                max_scroll: 0.0,
                hover_pt: None,
                last_hover_idx: None,
                tracking_mouse: false,
                page_row_y: [0.0, 52.0, 104.0, 156.0, 208.0, 260.0],
                dragging_page: None,
                pages_card_top: 77.0,
                widget_edit_slot: 0,
                preview_mascot: false,
                preview_phase: 0.0,
                win_opacity: 0.0,
                layered_active: false,
                closing: false,
                page_anim: 1.0,
                sidebar_sel_y: SettingsTab::General.sidebar_y(),
                anim_vals: HashMap::new(),
                hover_anims: RefCell::new(HashMap::new()),
                last_anim_tick: Instant::now(),
                anim_running: false,
                hit_boxes: Vec::new(),
                actions: Vec::new(),
                gfx: None,
            })),
            class_registered: false,
        }
    }

    pub fn is_open(&self) -> bool {
        self.inner.borrow().hwnd.is_some()
    }

    pub fn tick_and_paint(&mut self) -> bool {
        let (hwnd_opt, active, destroy_now) = {
            let Ok(mut st) = self.inner.try_borrow_mut() else {
                return false;
            };
            let Some(hwnd) = st.hwnd else {
                return false;
            };
            if !st.anim_running && st.dragging_page.is_none() {
                return false;
            }
            let (still_active, done_closing) = st.step_animations();
            if !done_closing {
                st.paint();
            }
            let hover_pending = !done_closing
                && st
                    .hover_anims
                    .borrow()
                    .values()
                    .any(|&(c, t)| (t - c).abs() >= 0.004);
            let keep_alive = still_active || hover_pending || st.dragging_page.is_some();
            if !keep_alive {
                st.stop_anim_timer();
            }
            (Some(hwnd), keep_alive, done_closing)
        };
        if destroy_now {
            if let Some(hwnd) = hwnd_opt {
                unsafe {
                    let _ = DestroyWindow(hwnd);
                }
            }
            return false;
        }
        active
    }

    pub fn sync_snapshot(&mut self, mut snap: SettingsSnapshot) {
        let hwnd_opt = {
            let mut st = self.inner.borrow_mut();
            snap.start_with_windows = st.snap.start_with_windows;
            st.snap = snap;
            st.hwnd
        };
        if let Some(hwnd) = hwnd_opt {
            unsafe {
                let _ = InvalidateRect(hwnd, None, false);
            }
        }
    }

    pub fn drain_actions(&mut self) -> Vec<SettingsAction> {
        let mut st = self.inner.borrow_mut();
        std::mem::take(&mut st.actions)
    }

    pub fn open_or_focus(&mut self, mut snap: SettingsSnapshot) {
        let startup_now = is_startup_enabled();
        let mon_cnt = unsafe { GetSystemMetrics(SM_CMONITORS).max(1) as usize };
        snap.start_with_windows = startup_now;
        {
            let mut st = self.inner.borrow_mut();
            st.snap = snap;
            st.monitor_count = mon_cnt;
            if let Some(hwnd) = st.hwnd {
                st.closing = false;
                st.ensure_anim_timer();
                unsafe {
                    let _ = ShowWindow(hwnd, SW_SHOW);
                    let _ = SetForegroundWindow(hwnd);
                    let _ = InvalidateRect(hwnd, None, false);
                }
                return;
            }
        }

        unsafe {
            let hinst = GetModuleHandleW(None).unwrap_or_default();
            if !self.class_registered {
                let app_icon = windows::Win32::UI::WindowsAndMessaging::LoadIconW(
                    hinst,
                    PCWSTR(1 as *const u16),
                )
                .unwrap_or_default();
                let wc = WNDCLASSEXW {
                    cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                    style: CS_HREDRAW | CS_VREDRAW,
                    lpfnWndProc: Some(settings_wnd_proc),
                    hInstance: hinst.into(),
                    hIcon: app_icon,
                    hIconSm: app_icon,
                    hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                    lpszClassName: SETTINGS_CLASS,
                    ..Default::default()
                };
                let _ = RegisterClassExW(&wc);
                self.class_registered = true;
            }

            let screen_w = GetSystemMetrics(SM_CXSCREEN).max(800);
            let screen_h = GetSystemMetrics(SM_CYSCREEN).max(600);
            let w_px = WIN_W_DIPS as i32;
            let h_px = WIN_H_DIPS as i32;
            let x = (screen_w - w_px) / 2;
            let y = (screen_h - h_px) / 2;

            let raw_ptr = Rc::into_raw(Rc::clone(&self.inner));
            let Ok(hwnd) = CreateWindowExW(
                WS_EX_APPWINDOW | WS_EX_LAYERED,
                SETTINGS_CLASS,
                w!("Eave Settings"),
                WS_POPUP | WS_VISIBLE,
                x,
                y,
                w_px,
                h_px,
                None,
                None,
                hinst,
                None,
            ) else {
                let _ = Rc::from_raw(raw_ptr);
                return;
            };

            let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 0, LWA_ALPHA);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, raw_ptr as isize);

            let dark: i32 = 1;
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_USE_IMMERSIVE_DARK_MODE,
                &dark as *const _ as *const _,
                std::mem::size_of::<i32>() as u32,
            );
            let corner = DWMWCP_ROUND;
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_WINDOW_CORNER_PREFERENCE,
                &corner as *const _ as *const _,
                std::mem::size_of_val(&corner) as u32,
            );

            let dpi = GetDpiForWindow(hwnd).max(96) as f32;
            let scaled_w = (WIN_W_DIPS * dpi / 96.0).round() as i32;
            let scaled_h = (WIN_H_DIPS * dpi / 96.0).round() as i32;
            let cx = (screen_w - scaled_w) / 2;
            let cy = (screen_h - scaled_h) / 2;
            let _ = SetWindowPos(
                hwnd,
                None,
                cx,
                cy,
                scaled_w,
                scaled_h,
                SWP_NOZORDER | SWP_SHOWWINDOW,
            );

            {
                let mut st = self.inner.borrow_mut();
                st.hwnd = Some(hwnd);
                st.dpi = dpi;
                st.scroll_y = 0.0;
                st.scroll_target = 0.0;
                st.win_opacity = 0.0;
                st.layered_active = true;
                st.closing = false;
                st.page_anim = 0.55;
                st.sidebar_sel_y = st.tab.sidebar_y();
                st.dragging_page = None;
                st.snap_page_row_positions();
                st.last_anim_tick = Instant::now();
                st.gfx = create_gfx(hwnd, dpi).ok();
                st.seed_anim_values();
                st.ensure_anim_timer();
            }

            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = SetForegroundWindow(hwnd);
            let _ = InvalidateRect(hwnd, None, false);
        }
    }
}

pub fn is_startup_enabled() -> bool {
    Command::new("reg")
        .args([
            "query",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
            "/v",
            "Eave",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn set_startup_enabled(enable: bool) -> bool {
    let key = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
    if enable {
        let Ok(exe) = std::env::current_exe() else {
            return false;
        };
        let cmd_val = format!("\"{}\"", exe.display());
        let _ = Command::new("reg")
            .args(["add", key, "/v", "Eave", "/t", "REG_SZ", "/d", &cmd_val, "/f"])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
    } else {
        let _ = Command::new("reg")
            .args(["delete", key, "/v", "Eave", "/f"])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
    }
    is_startup_enabled() == enable
}

fn format_local_clock_short() -> String {
    use windows::Win32::Foundation::SYSTEMTIME;
    #[link(name = "kernel32")]
    extern "system" {
        fn GetLocalTime(lpSystemTime: *mut SYSTEMTIME);
    }
    let mut st = SYSTEMTIME::default();
    unsafe { GetLocalTime(&mut st) };
    let h24 = st.wHour;
    let ampm = if h24 >= 12 { "PM" } else { "AM" };
    let h12 = match h24 % 12 {
        0 => 12,
        v => v,
    };
    format!("{}:{:02} {}", h12, st.wMinute, ampm)
}

fn create_text_format(
    dwrite: &IDWriteFactory,
    size: f32,
    weight: windows::Win32::Graphics::DirectWrite::DWRITE_FONT_WEIGHT,
    align: windows::Win32::Graphics::DirectWrite::DWRITE_TEXT_ALIGNMENT,
) -> windows::core::Result<IDWriteTextFormat> {
    unsafe {
        let fmt = dwrite.CreateTextFormat(
            w!("Segoe UI Variable Text"),
            None,
            weight,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            size,
            w!("en-US"),
        )?;
        fmt.SetTextAlignment(align)?;
        fmt.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
        fmt.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
        Ok(fmt)
    }
}

fn create_gfx(hwnd: HWND, dpi: f32) -> windows::core::Result<GfxResources> {
    unsafe {
        let d2d_factory: ID2D1Factory1 =
            D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
        let mut rc = RECT::default();
        let _ = GetClientRect(hwnd, &mut rc);
        let size = D2D_SIZE_U {
            width: (rc.right - rc.left).max(1) as u32,
            height: (rc.bottom - rc.top).max(1) as u32,
        };
        let rt_props = D2D1_RENDER_TARGET_PROPERTIES {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_IGNORE,
            },
            dpiX: dpi,
            dpiY: dpi,
            ..Default::default()
        };
        let hwnd_props = D2D1_HWND_RENDER_TARGET_PROPERTIES {
            hwnd,
            pixelSize: size,
            presentOptions: D2D1_PRESENT_OPTIONS_IMMEDIATELY,
        };
        let rt = d2d_factory.CreateHwndRenderTarget(&rt_props, &hwnd_props)?;
        rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
        let brush = rt.CreateSolidColorBrush(
            &D2D1_COLOR_F {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 1.0,
            },
            None,
        )?;
        let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
        let fmt_title =
            create_text_format(&dwrite, 19.0, DWRITE_FONT_WEIGHT_BOLD, DWRITE_TEXT_ALIGNMENT_LEADING)?;
        let fmt_section =
            create_text_format(&dwrite, 11.0, DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_TEXT_ALIGNMENT_LEADING)?;
        let fmt_sidebar =
            create_text_format(&dwrite, 12.5, DWRITE_FONT_WEIGHT_MEDIUM, DWRITE_TEXT_ALIGNMENT_LEADING)?;
        let fmt_row =
            create_text_format(&dwrite, 13.0, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_TEXT_ALIGNMENT_LEADING)?;
        let fmt_sub =
            create_text_format(&dwrite, 11.0, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_TEXT_ALIGNMENT_LEADING)?;
        let fmt_seg =
            create_text_format(&dwrite, 11.5, DWRITE_FONT_WEIGHT_MEDIUM, DWRITE_TEXT_ALIGNMENT_CENTER)?;
        let fmt_seg_on =
            create_text_format(&dwrite, 11.5, DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_TEXT_ALIGNMENT_CENTER)?;
        let fmt_val =
            create_text_format(&dwrite, 12.0, DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_TEXT_ALIGNMENT_TRAILING)?;
        let fmt_preview =
            create_text_format(&dwrite, 10.5, DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_TEXT_ALIGNMENT_CENTER)?;

        let eave_icon_bytes: &[u8] = include_bytes!("../assets/eave_icon_128.bgra");
        let eave_icon = rt
            .CreateBitmap(
                windows::Win32::Graphics::Direct2D::Common::D2D_SIZE_U {
                    width: 128,
                    height: 128,
                },
                Some(eave_icon_bytes.as_ptr() as *const std::ffi::c_void),
                128 * 4,
                &windows::Win32::Graphics::Direct2D::D2D1_BITMAP_PROPERTIES {
                    pixelFormat: windows::Win32::Graphics::Direct2D::Common::D2D1_PIXEL_FORMAT {
                        format: windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM,
                        alphaMode: windows::Win32::Graphics::Direct2D::Common::D2D1_ALPHA_MODE_PREMULTIPLIED,
                    },
                    dpiX: 96.0,
                    dpiY: 96.0,
                },
            )
            .ok();

        Ok(GfxResources {
            d2d_factory,
            rt,
            brush,
            dwrite,
            fmt_title,
            fmt_section,
            fmt_sidebar,
            fmt_row,
            fmt_sub,
            fmt_seg,
            fmt_seg_on,
            fmt_val,
            fmt_preview,
            eave_icon,
        })
    }
}

impl SettingsInner {
    fn snap_page_row_positions(&mut self) {
        for (slot, &pid) in self.snap.cfg.page_order.iter().enumerate() {
            let id = (pid as usize).min(5);
            self.page_row_y[id] = slot as f32 * 52.0;
        }
    }

    fn tab_reset_state(&self, tab: SettingsTab) -> (bool, bool) {
        let c = &self.snap.cfg;
        let d = Config::default();
        match tab {
            SettingsTab::General => (
                true,
                c.open_on_hover != d.open_on_hover
                    || c.minimal_mode != d.minimal_mode
                    || c.show_mascot != d.show_mascot
                    || c.show_clock != d.show_clock
                    || c.hide_fullscreen != d.hide_fullscreen,
            ),
            SettingsTab::Display => (
                true,
                c.clock_ring_mode != d.clock_ring_mode
                    || (c.notch_width - d.notch_width).abs() > 0.5
                    || (c.expanded_width - d.expanded_width).abs() > 0.5
                    || c.monitor_index != d.monitor_index
                    || c.horizontal_offset.abs() > 0.5
                    || (c.top_offset - d.top_offset).abs() > 0.5,
            ),
            SettingsTab::Indicators => (
                true,
                c.osd_volume != d.osd_volume
                    || c.osd_brightness != d.osd_brightness
                    || c.osd_caps_lock != d.osd_caps_lock
                    || c.osd_mic_mute != d.osd_mic_mute
                    || c.osd_mic_in_use != d.osd_mic_in_use
                    || c.osd_charging != d.osd_charging
                    || c.osd_wifi != d.osd_wifi
                    || c.osd_bluetooth != d.osd_bluetooth
                    || c.osd_audio_device != d.osd_audio_device
                    || c.osd_notifications != d.osd_notifications,
            ),
            SettingsTab::EaveAi => (
                true,
                c.ai_enabled != d.ai_enabled
                    || c.ai_model != d.ai_model
                    || c.ai_hotkey != d.ai_hotkey,
            ),
            SettingsTab::Pages => (
                true,
                c.page_order != [0, 1, 2, 3, 4, 5] || c.page_enabled != [true; 6],
            ),
            SettingsTab::Music => (true, c.show_lyrics != d.show_lyrics),
            SettingsTab::Widgets => (
                true,
                !c.page_enabled[5]
                    || c.widget_left != d.widget_left
                    || c.widget_right != d.widget_right
                    || c.widget_city_1 != d.widget_city_1
                    || c.widget_city_2 != d.widget_city_2
                    || c.widget_countdown != d.widget_countdown
                    || c.widget_weather_city != d.widget_weather_city,
            ),
            SettingsTab::Focus => (
                true,
                self.snap.pomo_focus_mins != 25
                    || self.snap.pomo_break_mins != 5
                    || self.snap.pomo_long_break_mins != 15,
            ),
            SettingsTab::Water => (
                true,
                self.snap.water_goal_ml != 2000
                    || self.snap.water_bottle_ml != 250
                    || self.snap.water_reminder_enabled,
            ),
            SettingsTab::Todo | SettingsTab::Shelf | SettingsTab::About => (false, false),
        }
    }

    fn seed_anim_values(&mut self) {
        let targets = self.desired_anim_targets();
        for (k, v) in targets {
            self.anim_vals.insert(k, v);
        }
    }

    fn desired_anim_targets(&self) -> Vec<(&'static str, f32)> {
        let c = &self.snap.cfg;
        let size_idx = c.size_preset.min(4) as f32;
        let mon_idx = c.monitor_index.min(self.monitor_count.saturating_sub(1)).min(2) as f32;
        let focus_idx = match self.snap.pomo_focus_mins {
            0..=19 => 0.0,
            20..=27 => 1.0,
            28..=37 => 2.0,
            _ => 3.0,
        };
        let break_idx = if self.snap.pomo_break_mins >= 8 { 1.0 } else { 0.0 };
        let long_idx = match self.snap.pomo_long_break_mins {
            0..=17 => 0.0,
            18..=24 => 1.0,
            _ => 2.0,
        };
        let goal_idx = match self.snap.water_goal_ml {
            0..=1750 => 0.0,
            1751..=2250 => 1.0,
            2251..=2750 => 2.0,
            _ => 3.0,
        };
        let cup_idx = match self.snap.water_bottle_ml {
            0..=225 => 0.0,
            226..=300 => 1.0,
            301..=425 => 2.0,
            _ => 3.0,
        };
        vec![
            ("startup", if self.snap.start_with_windows { 1.0 } else { 0.0 }),
            ("open_on_hover", if c.open_on_hover { 1.0 } else { 0.0 }),
            ("minimal_mode", if c.minimal_mode { 1.0 } else { 0.0 }),
            ("show_mascot", if c.show_mascot { 1.0 } else { 0.0 }),
            ("preview_mascot", if self.preview_mascot { 1.0 } else { 0.0 }),
            ("show_clock", if c.show_clock { 1.0 } else { 0.0 }),
            ("hide_fullscreen", if c.hide_fullscreen { 1.0 } else { 0.0 }),
            ("show_lyrics", if c.show_lyrics { 1.0 } else { 0.0 }),
            ("osd_volume", if c.osd_volume { 1.0 } else { 0.0 }),
            ("osd_brightness", if c.osd_brightness { 1.0 } else { 0.0 }),
            ("osd_caps_lock", if c.osd_caps_lock { 1.0 } else { 0.0 }),
            ("osd_mic_mute", if c.osd_mic_mute { 1.0 } else { 0.0 }),
            ("osd_mic_in_use", if c.osd_mic_in_use { 1.0 } else { 0.0 }),
            ("osd_charging", if c.osd_charging { 1.0 } else { 0.0 }),
            ("osd_wifi", if c.osd_wifi { 1.0 } else { 0.0 }),
            ("osd_bluetooth", if c.osd_bluetooth { 1.0 } else { 0.0 }),
            ("osd_audio_device", if c.osd_audio_device { 1.0 } else { 0.0 }),
            ("osd_notifications", if c.osd_notifications { 1.0 } else { 0.0 }),
            ("ai_enabled", if c.ai_enabled { 1.0 } else { 0.0 }),
            ("auto_check_updates", if c.auto_check_updates { 1.0 } else { 0.0 }),
            ("todo_remind_start", if c.todo_remind_start { 1.0 } else { 0.0 }),
            ("todo_remind_end", if c.todo_remind_end { 1.0 } else { 0.0 }),
            ("seg_todo_snooze", c.todo_snooze_mins.min(2) as f32),
            (
                "seg_todo_week_start",
                if c.todo_week_starts_monday { 1.0 } else { 0.0 },
            ),
            (
                "seg_todo_cal_view",
                if c.todo_calendar_default_week { 1.0 } else { 0.0 },
            ),
            (
                "seg_ai_model",
                crate::ai::model_index_for_id(&c.ai_model).unwrap_or(0) as f32,
            ),
            ("seg_ai_hotkey", c.ai_hotkey.min(3) as f32),
            ("water_reminder", if self.snap.water_reminder_enabled { 1.0 } else { 0.0 }),
            ("seg_size", size_idx),
            ("seg_monitor", mon_idx),
            ("seg_focus", focus_idx),
            ("seg_break", break_idx),
            ("seg_long", long_idx),
            ("seg_goal", goal_idx),
            ("seg_cup", cup_idx),
            ("preview_w", c.notch_width),
            ("preview_hof", c.horizontal_offset),
            ("preview_tof", c.top_offset),
            ("page_en_0", if c.page_enabled[0] { 1.0 } else { 0.0 }),
            ("page_en_1", if c.page_enabled[1] { 1.0 } else { 0.0 }),
            ("page_en_2", if c.page_enabled[2] { 1.0 } else { 0.0 }),
            ("page_en_3", if c.page_enabled[3] { 1.0 } else { 0.0 }),
            ("page_en_4", if c.page_enabled[4] { 1.0 } else { 0.0 }),
            ("page_en_5", if c.page_enabled[5] { 1.0 } else { 0.0 }),
            ("ring_sel_0", if c.clock_ring_mode == 3 { 1.0 } else { 0.0 }),
            ("ring_sel_1", if c.clock_ring_mode == 1 { 1.0 } else { 0.0 }),
            ("ring_sel_2", if c.clock_ring_mode == 2 { 1.0 } else { 0.0 }),
            ("ring_sel_3", if c.clock_ring_mode == 0 { 1.0 } else { 0.0 }),
            ("page_drag_lift", if self.dragging_page.is_some() { 1.0 } else { 0.0 }),
            ("tab_reset_dirty", if self.tab_reset_state(self.tab).1 { 1.0 } else { 0.0 }),
            ("widget_slot", self.widget_edit_slot as f32),
            ("wg_sel_0", if (if self.widget_edit_slot == 0 { c.widget_left } else { c.widget_right }) == 0 { 1.0 } else { 0.0 }),
            ("wg_sel_1", if (if self.widget_edit_slot == 0 { c.widget_left } else { c.widget_right }) == 1 { 1.0 } else { 0.0 }),
            ("wg_sel_2", if (if self.widget_edit_slot == 0 { c.widget_left } else { c.widget_right }) == 2 { 1.0 } else { 0.0 }),
            ("wg_sel_3", if (if self.widget_edit_slot == 0 { c.widget_left } else { c.widget_right }) == 3 { 1.0 } else { 0.0 }),
            ("wg_sel_4", if (if self.widget_edit_slot == 0 { c.widget_left } else { c.widget_right }) == 4 { 1.0 } else { 0.0 }),
            ("wg_sel_5", if (if self.widget_edit_slot == 0 { c.widget_left } else { c.widget_right }) == 5 { 1.0 } else { 0.0 }),
            ("wg_sel_6", if (if self.widget_edit_slot == 0 { c.widget_left } else { c.widget_right }) == 6 { 1.0 } else { 0.0 }),
            ("wg_sel_7", if (if self.widget_edit_slot == 0 { c.widget_left } else { c.widget_right }) == 7 { 1.0 } else { 0.0 }),
        ]
    }

    fn anim(&self, key: &'static str, fallback: f32) -> f32 {
        *self.anim_vals.get(key).unwrap_or(&fallback)
    }

    fn request_close(&mut self) {
        if !self.closing {
            self.closing = true;
            if let Some(hwnd) = self.hwnd {
                if !self.layered_active {
                    unsafe {
                        SetWindowLongPtrW(
                            hwnd,
                            GWL_EXSTYLE,
                            (WS_EX_APPWINDOW | WS_EX_LAYERED).0 as isize,
                        );
                        let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA);
                    }
                    self.layered_active = true;
                }
            }
            self.ensure_anim_timer();
        }
    }

    fn step_animations(&mut self) -> (bool, bool) {
        let now = Instant::now();
        let dt = now
            .duration_since(self.last_anim_tick)
            .as_secs_f32()
            .clamp(0.001, 0.05);
        self.last_anim_tick = now;

        let mut active = false;
        let exp_k = |k: f32| 1.0 - (-k * dt).exp();

        if self.layered_active {
            let op_target = if self.closing { 0.0 } else { 1.0 };
            let op_rate = exp_k(if self.closing { 22.0 } else { 18.0 });
            let op_diff = op_target - self.win_opacity;
            if op_diff.abs() < 0.015 {
                self.win_opacity = op_target;
            } else {
                self.win_opacity = (self.win_opacity + op_diff * op_rate).clamp(0.0, 1.0);
                active = true;
            }
            if let Some(hwnd) = self.hwnd {
                if !self.closing && self.win_opacity >= 0.99 {
                    unsafe {
                        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, WS_EX_APPWINDOW.0 as isize);
                    }
                    self.layered_active = false;
                } else {
                    let a = (self.win_opacity * 255.0).round().clamp(0.0, 255.0) as u8;
                    unsafe {
                        let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), a, LWA_ALPHA);
                    }
                }
            }
            if self.closing && self.win_opacity <= 0.02 {
                return (false, true);
            }
        }

        if self.page_anim < 1.0 {
            self.page_anim = (self.page_anim + dt / 0.28).min(1.0);
            if self.page_anim >= 1.0 {
                self.page_anim = 1.0;
                self.prev_tab = None;
            } else {
                active = true;
            }
        }

        let sel_target = self.tab.sidebar_y();
        let sel_diff = sel_target - self.sidebar_sel_y;
        if sel_diff.abs() < 0.25 {
            self.sidebar_sel_y = sel_target;
        } else {
            self.sidebar_sel_y += sel_diff * exp_k(22.0);
            active = true;
        }

        let sc_diff = self.scroll_target - self.scroll_y;
        if sc_diff.abs() < 0.3 {
            self.scroll_y = self.scroll_target;
        } else {
            self.scroll_y += sc_diff * exp_k(20.0);
            active = true;
        }

        let dt_ctrl = dt.min(0.016);
        let k_toggle = 1.0 - (-11.5 * dt_ctrl).exp();
        let k_ctrl = 1.0 - (-18.0 * dt_ctrl).exp();
        let targets = self.desired_anim_targets();
        for (k, tgt) in targets {
            let cur = self.anim_vals.entry(k).or_insert(tgt);
            let diff = tgt - *cur;
            let is_toggle = !k.starts_with("seg_")
                && !k.starts_with("preview_")
                && !k.starts_with("ring_sel_")
                && !k.starts_with("wg_sel_")
                && k != "widget_slot"
                && k != "page_drag_lift";
            let rate = if is_toggle { k_toggle } else { k_ctrl };
            if diff.abs() < 0.002 {
                *cur = tgt;
            } else {
                *cur += diff * rate;
                active = true;
            }
        }

        let preview_mascot_val = self.anim("preview_mascot", 0.0);
        if (self.preview_mascot || preview_mascot_val > 0.01) && self.tab == SettingsTab::General {
            self.preview_phase = (self.preview_phase + dt) % 1000.0;
            active = true;
        }

        let k_hov = exp_k(22.0);
        for (cur, tgt) in self.hover_anims.borrow_mut().values_mut() {
            let diff = *tgt - *cur;
            if diff.abs() < 0.004 {
                *cur = *tgt;
            } else {
                *cur += diff * k_hov;
                active = true;
            }
        }

        let dragged_pid = self.dragging_page.map(|(pid, _, _)| pid);
        for (slot, &pid_u8) in self.snap.cfg.page_order.iter().enumerate() {
            let pid = (pid_u8 as usize).min(5);
            if Some(pid) == dragged_pid {
                continue;
            }
            let tgt_y = slot as f32 * 52.0;
            let diff = tgt_y - self.page_row_y[pid];
            if diff.abs() < 0.3 {
                self.page_row_y[pid] = tgt_y;
            } else {
                self.page_row_y[pid] += diff * k_ctrl;
                active = true;
            }
        }
        (active, false)
    }

    fn ensure_anim_timer(&mut self) {
        if !self.anim_running {
            self.last_anim_tick = Instant::now();
            if let Some(hwnd) = self.hwnd {
                unsafe {
                    SetTimer(hwnd, ANIM_TIMER_ID, 8, None);
                }
                self.anim_running = true;
            }
        }
    }

    fn stop_anim_timer(&mut self) {
        if self.anim_running {
            if let Some(hwnd) = self.hwnd {
                unsafe {
                    let _ = KillTimer(hwnd, ANIM_TIMER_ID);
                }
            }
            self.anim_running = false;
        }
    }

    fn handle_click(&mut self, x: f32, y: f32) {
        let hit = self
            .hit_boxes
            .iter()
            .rev()
            .find(|hb| {
                x >= hb.rect.left
                    && x <= hb.rect.right
                    && y >= hb.rect.top
                    && y <= hb.rect.bottom
            })
            .map(|hb| hb.hit.clone());

        let Some(hit) = hit else {
            return;
        };

        match hit {
            ControlHit::CloseButton => {
                self.request_close();
            }
            ControlHit::Tab(t) => {
                if self.tab != t {
                    self.prev_tab = Some(self.tab);
                    self.prev_scroll_y = self.scroll_y;
                    self.tab = t;
                    self.scroll_y = 0.0;
                    self.scroll_target = 0.0;
                    self.page_anim = 0.0;
                    self.dragging_page = None;
                    self.hover_anims
                        .borrow_mut()
                        .retain(|k, _| k.starts_with("sb_") || k == "close_btn");
                    if t == SettingsTab::Pages {
                        self.snap_page_row_positions();
                    }
                    self.ensure_anim_timer();
                }
            }
            ControlHit::Toggle(k) => {
                match k {
                    "startup" => {
                        let next = !self.snap.start_with_windows;
                        if set_startup_enabled(next) {
                            self.snap.start_with_windows = next;
                        }
                    }
                    "open_on_hover" => {
                        self.snap.cfg.open_on_hover = !self.snap.cfg.open_on_hover;
                        self.actions
                            .push(SettingsAction::UpdateConfig(self.snap.cfg.clone()));
                    }
                    "minimal_mode" => {
                        self.snap.cfg.minimal_mode = !self.snap.cfg.minimal_mode;
                        self.actions
                            .push(SettingsAction::UpdateConfig(self.snap.cfg.clone()));
                    }
                    "show_mascot" => {
                        self.snap.cfg.show_mascot = !self.snap.cfg.show_mascot;
                        self.actions
                            .push(SettingsAction::UpdateConfig(self.snap.cfg.clone()));
                    }
                    "preview_mascot" => {
                        self.preview_mascot = !self.preview_mascot;
                        self.ensure_anim_timer();
                    }
                    "show_clock" => {
                        self.snap.cfg.show_clock = !self.snap.cfg.show_clock;
                        self.actions
                            .push(SettingsAction::UpdateConfig(self.snap.cfg.clone()));
                    }
                    "hide_fullscreen" => {
                        self.snap.cfg.hide_fullscreen = !self.snap.cfg.hide_fullscreen;
                        self.actions
                            .push(SettingsAction::UpdateConfig(self.snap.cfg.clone()));
                    }
                    "show_lyrics" => {
                        self.snap.cfg.show_lyrics = !self.snap.cfg.show_lyrics;
                        self.actions
                            .push(SettingsAction::UpdateConfig(self.snap.cfg.clone()));
                    }
                    "auto_check_updates" => {
                        self.snap.cfg.auto_check_updates = !self.snap.cfg.auto_check_updates;
                        self.actions
                            .push(SettingsAction::UpdateConfig(self.snap.cfg.clone()));
                    }
                    "todo_remind_start" => {
                        self.snap.cfg.todo_remind_start = !self.snap.cfg.todo_remind_start;
                        self.actions
                            .push(SettingsAction::UpdateConfig(self.snap.cfg.clone()));
                    }
                    "todo_remind_end" => {
                        self.snap.cfg.todo_remind_end = !self.snap.cfg.todo_remind_end;
                        self.actions
                            .push(SettingsAction::UpdateConfig(self.snap.cfg.clone()));
                    }
                    "osd_volume" => {
                        self.snap.cfg.osd_volume = !self.snap.cfg.osd_volume;
                        self.actions
                            .push(SettingsAction::UpdateConfig(self.snap.cfg.clone()));
                    }
                    "osd_brightness" => {
                        self.snap.cfg.osd_brightness = !self.snap.cfg.osd_brightness;
                        self.actions
                            .push(SettingsAction::UpdateConfig(self.snap.cfg.clone()));
                    }
                    "osd_caps_lock" => {
                        self.snap.cfg.osd_caps_lock = !self.snap.cfg.osd_caps_lock;
                        self.actions
                            .push(SettingsAction::UpdateConfig(self.snap.cfg.clone()));
                    }
                    "osd_mic_mute" => {
                        self.snap.cfg.osd_mic_mute = !self.snap.cfg.osd_mic_mute;
                        self.actions
                            .push(SettingsAction::UpdateConfig(self.snap.cfg.clone()));
                    }
                    "osd_mic_in_use" => {
                        self.snap.cfg.osd_mic_in_use = !self.snap.cfg.osd_mic_in_use;
                        self.actions
                            .push(SettingsAction::UpdateConfig(self.snap.cfg.clone()));
                    }
                    "osd_charging" => {
                        self.snap.cfg.osd_charging = !self.snap.cfg.osd_charging;
                        self.actions
                            .push(SettingsAction::UpdateConfig(self.snap.cfg.clone()));
                    }
                    "osd_wifi" => {
                        self.snap.cfg.osd_wifi = !self.snap.cfg.osd_wifi;
                        self.actions
                            .push(SettingsAction::UpdateConfig(self.snap.cfg.clone()));
                    }
                    "osd_bluetooth" => {
                        self.snap.cfg.osd_bluetooth = !self.snap.cfg.osd_bluetooth;
                        self.actions
                            .push(SettingsAction::UpdateConfig(self.snap.cfg.clone()));
                    }
                    "osd_audio_device" => {
                        self.snap.cfg.osd_audio_device = !self.snap.cfg.osd_audio_device;
                        self.actions
                            .push(SettingsAction::UpdateConfig(self.snap.cfg.clone()));
                    }
                    "osd_notifications" => {
                        self.snap.cfg.osd_notifications = !self.snap.cfg.osd_notifications;
                        self.actions
                            .push(SettingsAction::UpdateConfig(self.snap.cfg.clone()));
                    }
                    "ai_enabled" => {
                        self.snap.cfg.ai_enabled = !self.snap.cfg.ai_enabled;
                        self.actions
                            .push(SettingsAction::UpdateConfig(self.snap.cfg.clone()));
                    }
                    "show_widgets_page" => {
                        let cur = self.snap.cfg.page_enabled[5];
                        let enabled_cnt = self.snap.cfg.page_enabled.iter().filter(|&&b| b).count();
                        if !cur || enabled_cnt > 1 {
                            self.snap.cfg.page_enabled[5] = !cur;
                            self.actions
                                .push(SettingsAction::UpdateConfig(self.snap.cfg.clone()));
                        }
                    }
                    "water_reminder" => {
                        self.snap.water_reminder_enabled = !self.snap.water_reminder_enabled;
                        self.actions.push(SettingsAction::SetWaterSettings {
                            goal_ml: self.snap.water_goal_ml,
                            bottle_ml: self.snap.water_bottle_ml,
                            reminder_enabled: self.snap.water_reminder_enabled,
                        });
                    }
                    _ => {}
                }
                self.ensure_anim_timer();
            }
            ControlHit::Segment(k, idx) => {
                match k {
                    "seg_size" => {
                        let preset = (idx as u8).min(4);
                        self.snap.cfg.size_preset = preset;
                        let (nw, ew) = match preset {
                            0 | 1 => (166.0, 438.0),
                            2 => (184.0, 472.0),
                            3 => (202.0, 508.0),
                            _ => (224.0, 548.0),
                        };
                        self.snap.cfg.notch_width = nw;
                        self.snap.cfg.expanded_width = ew;
                        self.actions
                            .push(SettingsAction::UpdateConfig(self.snap.cfg.clone()));
                    }
                    "seg_monitor" => {
                        if idx < self.monitor_count {
                            self.snap.cfg.monitor_index = idx;
                            self.actions
                                .push(SettingsAction::UpdateConfig(self.snap.cfg.clone()));
                        }
                    }
                    "seg_ai_model" => {
                        if let Some((_, model_id)) = crate::ai::AI_MODELS.get(idx) {
                            self.snap.cfg.ai_model = (*model_id).to_string();
                            self.actions
                                .push(SettingsAction::UpdateConfig(self.snap.cfg.clone()));
                        }
                    }
                    "seg_ai_hotkey" => {
                        self.snap.cfg.ai_hotkey = (idx as u8).min(3);
                        self.actions
                            .push(SettingsAction::UpdateConfig(self.snap.cfg.clone()));
                    }
                    "seg_todo_snooze" => {
                        self.snap.cfg.todo_snooze_mins = (idx as u8).min(2);
                        self.actions
                            .push(SettingsAction::UpdateConfig(self.snap.cfg.clone()));
                    }
                    "seg_todo_week_start" => {
                        self.snap.cfg.todo_week_starts_monday = idx >= 1;
                        self.actions
                            .push(SettingsAction::UpdateConfig(self.snap.cfg.clone()));
                    }
                    "seg_todo_cal_view" => {
                        self.snap.cfg.todo_calendar_default_week = idx >= 1;
                        self.actions
                            .push(SettingsAction::UpdateConfig(self.snap.cfg.clone()));
                    }
                    "seg_focus" => {
                        self.snap.pomo_focus_mins = [15, 25, 30, 45][idx.min(3)];
                        self.actions.push(SettingsAction::SetPomodoroDurations {
                            focus_mins: self.snap.pomo_focus_mins,
                            break_mins: self.snap.pomo_break_mins,
                            long_break_mins: self.snap.pomo_long_break_mins,
                        });
                    }
                    "seg_break" => {
                        self.snap.pomo_break_mins = [5, 10][idx.min(1)];
                        self.actions.push(SettingsAction::SetPomodoroDurations {
                            focus_mins: self.snap.pomo_focus_mins,
                            break_mins: self.snap.pomo_break_mins,
                            long_break_mins: self.snap.pomo_long_break_mins,
                        });
                    }
                    "seg_long" => {
                        self.snap.pomo_long_break_mins = [15, 20, 30][idx.min(2)];
                        self.actions.push(SettingsAction::SetPomodoroDurations {
                            focus_mins: self.snap.pomo_focus_mins,
                            break_mins: self.snap.pomo_break_mins,
                            long_break_mins: self.snap.pomo_long_break_mins,
                        });
                    }
                    "seg_goal" => {
                        self.snap.water_goal_ml = [1500, 2000, 2500, 3000][idx.min(3)];
                        self.actions.push(SettingsAction::SetWaterSettings {
                            goal_ml: self.snap.water_goal_ml,
                            bottle_ml: self.snap.water_bottle_ml,
                            reminder_enabled: self.snap.water_reminder_enabled,
                        });
                    }
                    "seg_cup" => {
                        self.snap.water_bottle_ml = [200, 250, 350, 500][idx.min(3)];
                        self.actions.push(SettingsAction::SetWaterSettings {
                            goal_ml: self.snap.water_goal_ml,
                            bottle_ml: self.snap.water_bottle_ml,
                            reminder_enabled: self.snap.water_reminder_enabled,
                        });
                    }
                    _ => {}
                }
                self.ensure_anim_timer();
            }
            ControlHit::Stepper(k, delta) => {
                match k {
                    "h_offset" => {
                        if delta == 0 {
                            self.snap.cfg.horizontal_offset = 0.0;
                        } else {
                            self.snap.cfg.horizontal_offset =
                                (self.snap.cfg.horizontal_offset + delta as f32).clamp(-2000.0, 2000.0);
                        }
                        self.actions
                            .push(SettingsAction::UpdateConfig(self.snap.cfg.clone()));
                    }
                    "top_offset" => {
                        self.snap.cfg.top_offset =
                            (self.snap.cfg.top_offset + delta as f32).clamp(0.0, 120.0);
                        self.actions
                            .push(SettingsAction::UpdateConfig(self.snap.cfg.clone()));
                    }
                    "widget_city_1" => {
                        let n = WORLD_CITIES.len() as i32;
                        self.snap.cfg.widget_city_1 =
                            ((self.snap.cfg.widget_city_1 as i32 + delta).rem_euclid(n)) as u8;
                        self.actions
                            .push(SettingsAction::UpdateConfig(self.snap.cfg.clone()));
                    }
                    "widget_city_2" => {
                        let n = WORLD_CITIES.len() as i32;
                        self.snap.cfg.widget_city_2 =
                            ((self.snap.cfg.widget_city_2 as i32 + delta).rem_euclid(n)) as u8;
                        self.actions
                            .push(SettingsAction::UpdateConfig(self.snap.cfg.clone()));
                    }
                    "widget_countdown" => {
                        let n = COUNTDOWN_TARGETS.len() as i32;
                        self.snap.cfg.widget_countdown =
                            ((self.snap.cfg.widget_countdown as i32 + delta).rem_euclid(n)) as u8;
                        self.actions
                            .push(SettingsAction::UpdateConfig(self.snap.cfg.clone()));
                    }
                    "widget_weather_city" => {
                        let n = WEATHER_CITIES.len() as i32;
                        self.snap.cfg.widget_weather_city =
                            ((self.snap.cfg.widget_weather_city as i32 + delta).rem_euclid(n)) as u8;
                        self.actions
                            .push(SettingsAction::UpdateConfig(self.snap.cfg.clone()));
                    }
                    _ => {}
                }
                self.ensure_anim_timer();
            }
            ControlHit::PageDrag(_) => {}
            ControlHit::PageToggle(pid) => {
                if pid < 6 {
                    let cur = self.snap.cfg.page_enabled[pid];
                    let enabled_cnt = self.snap.cfg.page_enabled.iter().filter(|&&b| b).count();
                    if !cur || enabled_cnt > 1 {
                        self.snap.cfg.page_enabled[pid] = !cur;
                        self.actions
                            .push(SettingsAction::UpdateConfig(self.snap.cfg.clone()));
                        self.ensure_anim_timer();
                    }
                }
            }
            ControlHit::TabReset(tab) => {
                let d = Config::default();
                match tab {
                    SettingsTab::General => {
                        self.snap.cfg.open_on_hover = d.open_on_hover;
                        self.snap.cfg.minimal_mode = d.minimal_mode;
                        self.snap.cfg.show_mascot = d.show_mascot;
                        self.snap.cfg.show_clock = d.show_clock;
                        self.snap.cfg.hide_fullscreen = d.hide_fullscreen;
                        self.actions
                            .push(SettingsAction::UpdateConfig(self.snap.cfg.clone()));
                    }
                    SettingsTab::Display => {
                        self.snap.cfg.clock_ring_mode = d.clock_ring_mode;
                        self.snap.cfg.notch_width = d.notch_width;
                        self.snap.cfg.notch_height = d.notch_height;
                        self.snap.cfg.expanded_width = d.expanded_width;
                        self.snap.cfg.expanded_height = d.expanded_height;
                        self.snap.cfg.monitor_index = d.monitor_index;
                        self.snap.cfg.horizontal_offset = d.horizontal_offset;
                        self.snap.cfg.top_offset = d.top_offset;
                        self.actions
                            .push(SettingsAction::UpdateConfig(self.snap.cfg.clone()));
                    }
                    SettingsTab::Indicators => {
                        self.snap.cfg.osd_volume = d.osd_volume;
                        self.snap.cfg.osd_brightness = d.osd_brightness;
                        self.snap.cfg.osd_caps_lock = d.osd_caps_lock;
                        self.snap.cfg.osd_mic_mute = d.osd_mic_mute;
                        self.snap.cfg.osd_mic_in_use = d.osd_mic_in_use;
                        self.snap.cfg.osd_charging = d.osd_charging;
                        self.snap.cfg.osd_wifi = d.osd_wifi;
                        self.snap.cfg.osd_bluetooth = d.osd_bluetooth;
                        self.snap.cfg.osd_audio_device = d.osd_audio_device;
                        self.snap.cfg.osd_notifications = d.osd_notifications;
                        self.actions
                            .push(SettingsAction::UpdateConfig(self.snap.cfg.clone()));
                    }
                    SettingsTab::EaveAi => {
                        self.snap.cfg.ai_enabled = d.ai_enabled;
                        self.snap.cfg.ai_model = d.ai_model;
                        self.snap.cfg.ai_hotkey = d.ai_hotkey;
                        self.actions
                            .push(SettingsAction::UpdateConfig(self.snap.cfg.clone()));
                    }
                    SettingsTab::Pages => {
                        self.snap.cfg.page_order = [0, 1, 2, 3, 4, 5];
                        self.snap.cfg.page_enabled = [true; 6];
                        self.actions
                            .push(SettingsAction::UpdateConfig(self.snap.cfg.clone()));
                    }
                    SettingsTab::Music => {
                        self.snap.cfg.show_lyrics = d.show_lyrics;
                        self.actions
                            .push(SettingsAction::UpdateConfig(self.snap.cfg.clone()));
                    }
                    SettingsTab::Widgets => {
                        self.snap.cfg.page_enabled[5] = true;
                        self.snap.cfg.widget_left = d.widget_left;
                        self.snap.cfg.widget_right = d.widget_right;
                        self.snap.cfg.widget_city_1 = d.widget_city_1;
                        self.snap.cfg.widget_city_2 = d.widget_city_2;
                        self.snap.cfg.widget_countdown = d.widget_countdown;
                        self.snap.cfg.widget_weather_city = d.widget_weather_city;
                        self.actions
                            .push(SettingsAction::UpdateConfig(self.snap.cfg.clone()));
                    }
                    SettingsTab::Focus => {
                        self.snap.pomo_focus_mins = 25;
                        self.snap.pomo_break_mins = 5;
                        self.snap.pomo_long_break_mins = 15;
                        self.actions.push(SettingsAction::SetPomodoroDurations {
                            focus_mins: 25,
                            break_mins: 5,
                            long_break_mins: 15,
                        });
                    }
                    SettingsTab::Water => {
                        self.snap.water_goal_ml = 2000;
                        self.snap.water_bottle_ml = 250;
                        self.snap.water_reminder_enabled = false;
                        self.actions.push(SettingsAction::SetWaterSettings {
                            goal_ml: 2000,
                            bottle_ml: 250,
                            reminder_enabled: false,
                        });
                    }
                    _ => {}
                }
                self.ensure_anim_timer();
            }
            ControlHit::RingMode(m) => {
                self.snap.cfg.clock_ring_mode = m;
                self.actions
                    .push(SettingsAction::UpdateConfig(self.snap.cfg.clone()));
                self.ensure_anim_timer();
            }
            ControlHit::WidgetSlot(slot) => {
                self.widget_edit_slot = slot.min(1);
                self.ensure_anim_timer();
            }
            ControlHit::WidgetPick(kind) => {
                let k = kind.min(7);
                if self.widget_edit_slot == 0 {
                    if self.snap.cfg.widget_right == k {
                        self.snap.cfg.widget_right = self.snap.cfg.widget_left;
                    }
                    self.snap.cfg.widget_left = k;
                } else {
                    if self.snap.cfg.widget_left == k {
                        self.snap.cfg.widget_left = self.snap.cfg.widget_right;
                    }
                    self.snap.cfg.widget_right = k;
                }
                self.actions
                    .push(SettingsAction::UpdateConfig(self.snap.cfg.clone()));
                self.ensure_anim_timer();
            }
            ControlHit::ActionBtn(k) => match k {
                "clear_done_todos" => self.actions.push(SettingsAction::ClearCompletedTodos),
                "clear_shelf_files" => self.actions.push(SettingsAction::ClearShelfFiles),
                "clear_shelf_clip" => self.actions.push(SettingsAction::ClearShelfClipboard),
                "open_cfg" => self.actions.push(SettingsAction::OpenConfigFile),
                "check_updates" => self.actions.push(SettingsAction::CheckForUpdates),
                "download_update" => {
                    if let crate::updater::UpdateStatus::Available { ref url, .. } =
                        self.snap.update_status
                    {
                        self.actions
                            .push(SettingsAction::OpenUpdateUrl(url.clone()));
                    }
                }
                "open_github" => self.actions.push(SettingsAction::OpenGitHubRepo),
                "smart_ocr" => self.actions.push(SettingsAction::TriggerSmartOcr),
                "ai_get_key" => self.actions.push(SettingsAction::OpenUpdateUrl(
                    "https://aistudio.google.com/api-keys".to_string(),
                )),
                "ai_paste_key" => {
                    if let Some(clip) = crate::ai::read_clipboard_text() {
                        let cleaned = clip.trim().to_string();
                        if !cleaned.is_empty() {
                            self.snap.cfg.ai_api_key = cleaned;
                            self.actions
                                .push(SettingsAction::UpdateConfig(self.snap.cfg.clone()));
                        }
                    }
                }
                "ai_clear_key" => {
                    self.snap.cfg.ai_api_key.clear();
                    self.actions
                        .push(SettingsAction::UpdateConfig(self.snap.cfg.clone()));
                }
                "reset_defaults" => {
                    self.snap.cfg = Config::default();
                    self.actions.push(SettingsAction::ResetDefaults);
                    self.ensure_anim_timer();
                }
                _ => {}
            },
        }

        if let Some(hwnd) = self.hwnd {
            unsafe {
                let _ = InvalidateRect(hwnd, None, false);
            }
        }
    }

    fn paint(&mut self) {
        let Some(gfx) = self.gfx.as_ref() else {
            return;
        };
        let rt: &ID2D1RenderTarget = &gfx.rt;
        let brush = &gfx.brush;
        let brush_ref: &ID2D1Brush = brush;

        let mut new_hits: Vec<HitBox> = Vec::with_capacity(64);
        let hover = self.hover_pt;
        let is_hov = |r: D2D_RECT_F| -> bool {
            hover.is_some_and(|(hx, hy)| hx >= r.left && hx <= r.right && hy >= r.top && hy <= r.bottom)
        };
        let hov_t = |key: &str, r: D2D_RECT_F| -> f32 {
            let tgt = if is_hov(r) { 1.0_f32 } else { 0.0_f32 };
            let mut map = self.hover_anims.borrow_mut();
            if let Some((cur, t)) = map.get_mut(key) {
                *t = tgt;
                cur.clamp(0.0, 1.0)
            } else {
                map.insert(key.to_string(), (0.0, tgt));
                0.0
            }
        };
        let mut detail_passes: Vec<(SettingsTab, f32, f32, f32, bool)> = Vec::with_capacity(2);
        if self.page_anim < 0.999 {
            if let Some(prev) = self.prev_tab {
                let out_t = (1.0 - self.page_anim * 1.9).clamp(0.0, 1.0);
                if out_t > 0.01 {
                    detail_passes.push((
                        prev,
                        self.prev_scroll_y,
                        out_t * out_t,
                        -12.0 * self.page_anim,
                        false,
                    ));
                }
            }
            let in_t = ((self.page_anim - 0.12) / 0.88).clamp(0.0, 1.0);
            let ease_in = 1.0 - (1.0 - in_t).powi(3);
            detail_passes.push((
                self.tab,
                self.scroll_y,
                ease_in,
                (1.0 - ease_in) * 28.0,
                true,
            ));
        } else {
            detail_passes.push((self.tab, self.scroll_y, 1.0, 0.0, true));
        }

        unsafe {
            rt.BeginDraw();

            rt.Clear(Some(&D2D1_COLOR_F {
                r: 0.102,
                g: 0.102,
                b: 0.114,
                a: 1.0,
            }));

            set_col(brush, 0.075, 0.075, 0.084, 1.0);
            rt.FillRectangle(
                &D2D_RECT_F {
                    left: 0.0,
                    top: 0.0,
                    right: SIDEBAR_W,
                    bottom: WIN_H_DIPS,
                },
                brush_ref,
            );

            set_col(brush, 1.0, 1.0, 1.0, 0.065);
            rt.DrawLine(
                D2D_POINT_2F { x: SIDEBAR_W, y: 0.0 },
                D2D_POINT_2F { x: SIDEBAR_W, y: WIN_H_DIPS },
                brush_ref,
                1.0,
                None,
            );

            let close_rect = D2D_RECT_F {
                left: 12.0,
                top: 12.0,
                right: 28.0,
                bottom: 28.0,
            };
            let close_h = hov_t("close_btn", close_rect);
            set_col(
                brush,
                0.98 + 0.02 * close_h,
                0.37 - 0.07 * close_h,
                0.34 - 0.07 * close_h,
                1.0,
            );
            rt.FillEllipse(
                &D2D1_ELLIPSE {
                    point: D2D_POINT_2F { x: 20.0, y: 20.0 },
                    radiusX: 6.2,
                    radiusY: 6.2,
                },
                brush_ref,
            );
            if close_h > 0.01 {
                set_col(brush, 0.24, 0.04, 0.04, 0.92 * close_h);
                rt.DrawLine(
                    D2D_POINT_2F { x: 17.5, y: 17.5 },
                    D2D_POINT_2F { x: 22.5, y: 22.5 },
                    brush_ref,
                    1.35,
                    None,
                );
                rt.DrawLine(
                    D2D_POINT_2F { x: 22.5, y: 17.5 },
                    D2D_POINT_2F { x: 17.5, y: 22.5 },
                    brush_ref,
                    1.35,
                    None,
                );
            }
            new_hits.push(HitBox {
                rect: close_rect,
                hit: ControlHit::CloseButton,
            });

            let sel_pill = D2D_RECT_F {
                left: 10.0,
                top: self.sidebar_sel_y,
                right: SIDEBAR_W - 10.0,
                bottom: self.sidebar_sel_y + 30.0,
            };
            set_col(brush, 1.0, 1.0, 1.0, 0.11);
            rt.FillRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: sel_pill,
                    radiusX: 7.5,
                    radiusY: 7.5,
                },
                brush_ref,
            );

            for &t in SettingsTab::ALL.iter() {
                let y = t.sidebar_y();
                let item_r = D2D_RECT_F {
                    left: 10.0,
                    top: y,
                    right: SIDEBAR_W - 10.0,
                    bottom: y + 30.0,
                };
                let selected = self.tab == t;
                let sb_key = format!("sb_{}", t as usize);
                let sb_h = hov_t(&sb_key, item_r);
                if !selected && sb_h > 0.01 {
                    set_col(brush, 1.0, 1.0, 1.0, 0.055 * sb_h);
                    rt.FillRoundedRectangle(
                        &D2D1_ROUNDED_RECT {
                            rect: item_r,
                            radiusX: 7.5,
                            radiusY: 7.5,
                        },
                        brush_ref,
                    );
                }

                let (br, bg, bb) = t.badge_color();
                let badge = D2D_RECT_F {
                    left: item_r.left + 7.0,
                    top: item_r.top + 4.5,
                    right: item_r.left + 28.0,
                    bottom: item_r.top + 25.5,
                };
                set_col(brush, br, bg, bb, 1.0);
                rt.FillRoundedRectangle(
                    &D2D1_ROUNDED_RECT {
                        rect: badge,
                        radiusX: 5.8,
                        radiusY: 5.8,
                    },
                    brush_ref,
                );
                let sheen_stops = [
                    D2D1_GRADIENT_STOP {
                        position: 0.0,
                        color: D2D1_COLOR_F { r: 1.0, g: 1.0, b: 1.0, a: 0.22 },
                    },
                    D2D1_GRADIENT_STOP {
                        position: 0.55,
                        color: D2D1_COLOR_F { r: 1.0, g: 1.0, b: 1.0, a: 0.0 },
                    },
                    D2D1_GRADIENT_STOP {
                        position: 1.0,
                        color: D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.14 },
                    },
                ];
                if let Ok(sc) = rt.CreateGradientStopCollection(
                    &sheen_stops,
                    D2D1_GAMMA_2_2,
                    D2D1_EXTEND_MODE_CLAMP,
                ) {
                    if let Ok(lg) = rt.CreateLinearGradientBrush(
                        &D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                            startPoint: D2D_POINT_2F { x: badge.left, y: badge.top },
                            endPoint: D2D_POINT_2F { x: badge.left, y: badge.bottom },
                        },
                        None,
                        &sc,
                    ) {
                        rt.FillRoundedRectangle(
                            &D2D1_ROUNDED_RECT {
                                rect: badge,
                                radiusX: 5.8,
                                radiusY: 5.8,
                            },
                            &lg,
                        );
                    }
                }
                set_col(brush, 1.0, 1.0, 1.0, 0.14);
                rt.DrawRoundedRectangle(
                    &D2D1_ROUNDED_RECT {
                        rect: badge,
                        radiusX: 5.8,
                        radiusY: 5.8,
                    },
                    brush_ref,
                    0.75,
                    None,
                );
                draw_sidebar_glyph(
                    &gfx.d2d_factory,
                    rt,
                    brush,
                    t,
                    (br, bg, bb),
                    (badge.left + badge.right) * 0.5,
                    (badge.top + badge.bottom) * 0.5,
                );

                let lum = if selected { 1.0 } else { 0.88 };
                set_col(brush, lum, lum, lum, 1.0);
                draw_text(
                    rt,
                    t.label(),
                    &gfx.fmt_sidebar,
                    &D2D_RECT_F {
                        left: badge.right + 9.0,
                        top: item_r.top,
                        right: item_r.right - 6.0,
                        bottom: item_r.bottom,
                    },
                    brush_ref,
                );

                new_hits.push(HitBox {
                    rect: item_r,
                    hit: ControlHit::Tab(t),
                });
            }

            let pane_clip = D2D_RECT_F {
                left: SIDEBAR_W + 1.0,
                top: 0.0,
                right: WIN_W_DIPS,
                bottom: WIN_H_DIPS,
            };
            rt.PushAxisAlignedClip(&pane_clip, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);

            let dx_l = SIDEBAR_W + 22.0;
            let dx_r = WIN_W_DIPS - 22.0;
            let mut next_max_scroll = self.max_scroll;

            for &(render_tab, render_scroll_y, pa, pass_dx, is_active_pass) in &detail_passes {
                let saved_hits_len = new_hits.len();
                if pass_dx.abs() > 0.05 {
                    rt.SetTransform(&Matrix3x2::translation(pass_dx, 0.0));
                }

                let (has_tab_reset, tab_is_dirty) = self.tab_reset_state(render_tab);

                set_col(brush, 1.0, 1.0, 1.0, pa);
                draw_text(
                    rt,
                    render_tab.label(),
                    &gfx.fmt_title,
                    &D2D_RECT_F {
                        left: dx_l,
                        top: 16.0,
                        right: if has_tab_reset { dx_r - 78.0 } else { dx_r },
                        bottom: 46.0,
                    },
                    brush_ref,
                );

                if has_tab_reset {
                    let reset_r = D2D_RECT_F {
                        left: dx_r - 68.0,
                        top: 20.0,
                        right: dx_r,
                        bottom: 42.0,
                    };
                    let dirty_t = self
                        .anim("tab_reset_dirty", if tab_is_dirty { 1.0 } else { 0.0 })
                        .clamp(0.0, 1.0);
                    let rh = if is_active_pass {
                        hov_t("tab_reset", reset_r)
                    } else {
                        0.0
                    };
                    let fill_a = (0.055 + 0.055 * dirty_t + 0.08 * rh) * pa;
                    set_col(brush, 1.0, 1.0, 1.0, fill_a);
                    rt.FillRoundedRectangle(
                        &D2D1_ROUNDED_RECT {
                            rect: reset_r,
                            radiusX: 7.0,
                            radiusY: 7.0,
                        },
                        brush_ref,
                    );
                    let border_a = (0.06 + 0.08 * dirty_t + 0.06 * rh) * pa;
                    set_col(brush, 1.0, 1.0, 1.0, border_a);
                    rt.DrawRoundedRectangle(
                        &D2D1_ROUNDED_RECT {
                            rect: reset_r,
                            radiusX: 7.0,
                            radiusY: 7.0,
                        },
                        brush_ref,
                        0.85,
                        None,
                    );
                    let txt_a = (0.52 + 0.42 * dirty_t.max(rh * 0.7)) * pa;
                    set_col(brush, 0.96, 0.96, 0.98, txt_a);
                    draw_text(rt, "Reset", &gfx.fmt_seg, &reset_r, brush_ref);
                    new_hits.push(HitBox {
                        rect: reset_r,
                        hit: ControlHit::TabReset(render_tab),
                    });
                }

                let body_clip = D2D_RECT_F {
                    left: SIDEBAR_W + 1.0,
                    top: 52.0,
                    right: WIN_W_DIPS,
                    bottom: WIN_H_DIPS - 10.0,
                };
                rt.PushAxisAlignedClip(&body_clip, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);

                let mut cy = (56.0 - render_scroll_y).round();
                let start_y = cy;

                match render_tab {
                SettingsTab::General => {
                    let preview_eave_t = self.anim(
                        "preview_mascot",
                        if self.preview_mascot { 1.0 } else { 0.0 },
                    );
                    cy = draw_live_notch_preview(
                        rt,
                        brush,
                        gfx,
                        dx_l,
                        dx_r,
                        cy,
                        self.anim("preview_w", self.snap.cfg.notch_width),
                        self.anim("minimal_mode", if self.snap.cfg.minimal_mode { 1.0 } else { 0.0 }),
                        preview_eave_t,
                        self.preview_phase,
                        self.anim("show_clock", if self.snap.cfg.show_clock { 1.0 } else { 0.0 }),
                        pa,
                    );

                    cy = draw_section_header(rt, brush, &gfx.fmt_section, "Startup", dx_l, dx_r, cy, pa);
                    cy = draw_group_card(rt, brush, dx_l, dx_r, cy, 46.0, pa, |rt, b, top| {
                        draw_toggle_row(
                            rt,
                            b,
                            gfx,
                            dx_l,
                            dx_r,
                            top,
                            46.0,
                            "Start with Windows",
                            None,
                            self.anim("startup", if self.snap.start_with_windows { 1.0 } else { 0.0 }),
                            "startup",
                            pa,
                            &mut new_hits,
                        );
                    });

                    cy = draw_section_header(rt, brush, &gfx.fmt_section, "Notch", dx_l, dx_r, cy, pa);
                    cy = draw_group_card(rt, brush, dx_l, dx_r, cy, 216.0, pa, |rt, b, top| {
                        draw_toggle_row(
                            rt,
                            b,
                            gfx,
                            dx_l,
                            dx_r,
                            top,
                            54.0,
                            "Open on hover",
                            Some("Expand automatically when hovering the notch"),
                            self.anim("open_on_hover", if self.snap.cfg.open_on_hover { 1.0 } else { 0.0 }),
                            "open_on_hover",
                            pa,
                            &mut new_hits,
                        );
                        draw_row_divider(rt, b, dx_l, dx_r, top + 54.0, pa);
                        draw_toggle_row(
                            rt,
                            b,
                            gfx,
                            dx_l,
                            dx_r,
                            top + 54.0,
                            54.0,
                            "Minimal notch",
                            Some("A smaller pill when nothing is playing"),
                            self.anim("minimal_mode", if self.snap.cfg.minimal_mode { 1.0 } else { 0.0 }),
                            "minimal_mode",
                            pa,
                            &mut new_hits,
                        );
                        draw_row_divider(rt, b, dx_l, dx_r, top + 108.0, pa);
                        draw_toggle_row(
                            rt,
                            b,
                            gfx,
                            dx_l,
                            dx_r,
                            top + 108.0,
                            54.0,
                            "Eave mascot",
                            Some("Glowing eyes & aurora after 20s idle"),
                            self.anim("show_mascot", if self.snap.cfg.show_mascot { 1.0 } else { 0.0 }),
                            "show_mascot",
                            pa,
                            &mut new_hits,
                        );
                        let eye_w = 26.0;
                        let eye_h = 22.0;
                        let eye_r_x = dx_r - 14.0 - 39.0 - 9.0;
                        let eye_rect = D2D_RECT_F {
                            left: eye_r_x - eye_w,
                            top: top + 108.0 + (54.0 - eye_h) * 0.5,
                            right: eye_r_x,
                            bottom: top + 108.0 + (54.0 + eye_h) * 0.5,
                        };
                        let eye_hov = hov_t("preview_mascot_btn", eye_rect);
                        draw_eye_preview_button(
                            rt,
                            b,
                            gfx,
                            eye_rect,
                            preview_eave_t,
                            eye_hov,
                            pa,
                        );
                        new_hits.push(HitBox {
                            rect: eye_rect,
                            hit: ControlHit::Toggle("preview_mascot"),
                        });
                        draw_row_divider(rt, b, dx_l, dx_r, top + 162.0, pa);
                        draw_toggle_row(
                            rt,
                            b,
                            gfx,
                            dx_l,
                            dx_r,
                            top + 162.0,
                            54.0,
                            "Date & time",
                            Some("The clock and date beside the status ring"),
                            self.anim("show_clock", if self.snap.cfg.show_clock { 1.0 } else { 0.0 }),
                            "show_clock",
                            pa,
                            &mut new_hits,
                        );
                    });

                    cy = draw_section_header(rt, brush, &gfx.fmt_section, "Full screen", dx_l, dx_r, cy, pa);
                    cy = draw_group_card(rt, brush, dx_l, dx_r, cy, 54.0, pa, |rt, b, top| {
                        draw_toggle_row(
                            rt,
                            b,
                            gfx,
                            dx_l,
                            dx_r,
                            top,
                            54.0,
                            "Hide in full screen",
                            Some("Games, videos & slides · Toggle anywhere with Ctrl + Alt + H"),
                            self.anim("hide_fullscreen", if self.snap.cfg.hide_fullscreen { 1.0 } else { 0.0 }),
                            "hide_fullscreen",
                            pa,
                            &mut new_hits,
                        );
                    });
                }
                SettingsTab::Display => {
                    cy = draw_section_header(
                        rt,
                        brush,
                        &gfx.fmt_section,
                        "Ring next to the clock",
                        dx_l,
                        dx_r,
                        cy,
                        pa,
                    );
                    let ring_sels = [
                        self.anim("ring_sel_0", if self.snap.cfg.clock_ring_mode == 3 { 1.0 } else { 0.0 })
                            .clamp(0.0, 1.0),
                        self.anim("ring_sel_1", if self.snap.cfg.clock_ring_mode == 1 { 1.0 } else { 0.0 })
                            .clamp(0.0, 1.0),
                        self.anim("ring_sel_2", if self.snap.cfg.clock_ring_mode == 2 { 1.0 } else { 0.0 })
                            .clamp(0.0, 1.0),
                        self.anim("ring_sel_3", if self.snap.cfg.clock_ring_mode == 0 { 1.0 } else { 0.0 })
                            .clamp(0.0, 1.0),
                    ];
                    cy = draw_ring_choice_card(
                        rt,
                        brush,
                        gfx,
                        dx_l,
                        dx_r,
                        cy,
                        ring_sels,
                        pa,
                        &hov_t,
                        &mut new_hits,
                    );

                    cy = draw_section_header(rt, brush, &gfx.fmt_section, "Notch screen", dx_l, dx_r, cy, pa);
                    cy = draw_monitor_preview_card(
                        rt,
                        brush,
                        gfx,
                        dx_l,
                        dx_r,
                        cy,
                        self.snap.cfg.monitor_index,
                        self.anim("preview_hof", self.snap.cfg.horizontal_offset),
                        self.anim("preview_tof", self.snap.cfg.top_offset),
                        self.monitor_count,
                        pa,
                    );

                    cy = draw_section_header(rt, brush, &gfx.fmt_section, "Size & Monitor", dx_l, dx_r, cy, pa);
                    cy = draw_group_card(rt, brush, dx_l, dx_r, cy, 166.0, pa, |rt, b, top| {
                        draw_segmented_row(
                            rt,
                            b,
                            gfx,
                            dx_l,
                            dx_r,
                            top,
                            "Size",
                            Some("Adjust the notch scale live"),
                            &["Auto", "100%", "115%", "130%", "150%"],
                            5,
                            self.anim("seg_size", self.snap.cfg.size_preset.min(4) as f32),
                            "seg_size",
                            pa,
                            &hov_t,
                            &mut new_hits,
                        );
                        draw_row_divider(rt, b, dx_l, dx_r, top + 83.0, pa);
                        let mon_sub = if self.monitor_count <= 1 {
                            "Only 1 display connected"
                        } else {
                            "Choose which monitor anchors the notch"
                        };
                        draw_segmented_row(
                            rt,
                            b,
                            gfx,
                            dx_l,
                            dx_r,
                            top + 83.0,
                            "Target display",
                            Some(mon_sub),
                            &["Primary", "Monitor 2", "Monitor 3"],
                            self.monitor_count,
                            self.anim(
                                "seg_monitor",
                                self.snap.cfg.monitor_index.min(self.monitor_count.saturating_sub(1)).min(2) as f32,
                            ),
                            "seg_monitor",
                            pa,
                            &hov_t,
                            &mut new_hits,
                        );
                    });

                    cy = draw_section_header(rt, brush, &gfx.fmt_section, "Position", dx_l, dx_r, cy, pa);
                    cy = draw_group_card(rt, brush, dx_l, dx_r, cy, 54.0, pa, |rt, b, top| {
                        let h_str = format!("{:+.0} px", self.snap.cfg.horizontal_offset);
                        draw_stepper_row(
                            rt,
                            b,
                            gfx,
                            dx_l,
                            dx_r,
                            top,
                            54.0,
                            "Horizontal offset",
                            Some("Shift left or right from center"),
                            &h_str,
                            "h_offset",
                            50,
                            true,
                            pa,
                            &hov_t,
                            &mut new_hits,
                        );
                    });
                }
                SettingsTab::Indicators => {
                    let row_h = 44.0;
                    cy = draw_section_header(
                        rt,
                        brush,
                        &gfx.fmt_section,
                        "Indicators",
                        dx_l,
                        dx_r,
                        cy,
                        pa,
                    );
                    let ind_rows: [(&str, &str, bool); 5] = [
                        ("Volume", "osd_volume", self.snap.cfg.osd_volume),
                        ("Brightness", "osd_brightness", self.snap.cfg.osd_brightness),
                        ("Caps Lock", "osd_caps_lock", self.snap.cfg.osd_caps_lock),
                        ("Mic Mute", "osd_mic_mute", self.snap.cfg.osd_mic_mute),
                        ("Mic in use", "osd_mic_in_use", self.snap.cfg.osd_mic_in_use),
                    ];
                    cy = draw_group_card(
                        rt,
                        brush,
                        dx_l,
                        dx_r,
                        cy,
                        row_h * ind_rows.len() as f32,
                        pa,
                        |rt, b, top| {
                            for (i, &(label, key, on)) in ind_rows.iter().enumerate() {
                                let ry = top + i as f32 * row_h;
                                if i > 0 {
                                    draw_row_divider(rt, b, dx_l, dx_r, ry, pa);
                                }
                                draw_toggle_row(
                                    rt,
                                    b,
                                    gfx,
                                    dx_l,
                                    dx_r,
                                    ry,
                                    row_h,
                                    label,
                                    None,
                                    self.anim(key, if on { 1.0 } else { 0.0 }),
                                    key,
                                    pa,
                                    &mut new_hits,
                                );
                            }
                        },
                    );

                    cy = draw_section_header(
                        rt,
                        brush,
                        &gfx.fmt_section,
                        "Popups",
                        dx_l,
                        dx_r,
                        cy,
                        pa,
                    );
                    let pop_rows: [(&str, &str, bool); 5] = [
                        ("Battery", "osd_charging", self.snap.cfg.osd_charging),
                        ("Wi-Fi", "osd_wifi", self.snap.cfg.osd_wifi),
                        (
                            "Bluetooth headset",
                            "osd_bluetooth",
                            self.snap.cfg.osd_bluetooth,
                        ),
                        (
                            "Audio output",
                            "osd_audio_device",
                            self.snap.cfg.osd_audio_device,
                        ),
                        (
                            "Notifications",
                            "osd_notifications",
                            self.snap.cfg.osd_notifications,
                        ),
                    ];
                    cy = draw_group_card(
                        rt,
                        brush,
                        dx_l,
                        dx_r,
                        cy,
                        row_h * pop_rows.len() as f32,
                        pa,
                        |rt, b, top| {
                            for (i, &(label, key, on)) in pop_rows.iter().enumerate() {
                                let ry = top + i as f32 * row_h;
                                if i > 0 {
                                    draw_row_divider(rt, b, dx_l, dx_r, ry, pa);
                                }
                                draw_toggle_row(
                                    rt,
                                    b,
                                    gfx,
                                    dx_l,
                                    dx_r,
                                    ry,
                                    row_h,
                                    label,
                                    None,
                                    self.anim(key, if on { 1.0 } else { 0.0 }),
                                    key,
                                    pa,
                                    &mut new_hits,
                                );
                            }
                        },
                    );
                }
                SettingsTab::EaveAi => {
                    cy = draw_section_header(
                        rt,
                        brush,
                        &gfx.fmt_section,
                        "Assistant",
                        dx_l,
                        dx_r,
                        cy,
                        pa,
                    );
                    cy = draw_group_card(rt, brush, dx_l, dx_r, cy, 54.0, pa, |rt, b, top| {
                        draw_toggle_row(
                            rt,
                            b,
                            gfx,
                            dx_l,
                            dx_r,
                            top,
                            54.0,
                            "Enable Eave AI",
                            Some("Open Eave AI from anywhere with your global hotkey"),
                            self.anim("ai_enabled", if self.snap.cfg.ai_enabled { 1.0 } else { 0.0 }),
                            "ai_enabled",
                            pa,
                            &mut new_hits,
                        );
                    });

                    cy = draw_section_header(
                        rt,
                        brush,
                        &gfx.fmt_section,
                        "Gemini API Key",
                        dx_l,
                        dx_r,
                        cy,
                        pa,
                    );
                    let key_status = {
                        let k = self.snap.cfg.ai_api_key.trim();
                        if k.is_empty() {
                            "Not set — copy your key from aistudio.google.com".to_string()
                        } else if k.len() > 8 {
                            format!("{}••••••••{}", &k[..4], &k[k.len() - 4..])
                        } else {
                            "••••••••".to_string()
                        }
                    };
                    let has_key = !self.snap.cfg.ai_api_key.trim().is_empty();
                    let key_card_h = if has_key { 162.0 } else { 108.0 };
                    cy = draw_group_card(rt, brush, dx_l, dx_r, cy, key_card_h, pa, |rt, b, top| {
                        draw_action_row(
                            rt,
                            b,
                            gfx,
                            dx_l,
                            dx_r,
                            top,
                            54.0,
                            "API Key",
                            Some(&key_status),
                            "Paste Key",
                            "ai_paste_key",
                            false,
                            pa,
                            &hov_t,
                            &mut new_hits,
                        );
                        draw_row_divider(rt, b, dx_l, dx_r, top + 54.0, pa);
                        draw_action_row(
                            rt,
                            b,
                            gfx,
                            dx_l,
                            dx_r,
                            top + 54.0,
                            54.0,
                            "Get a free API key",
                            Some("Open Google AI Studio in your browser"),
                            "Get Key",
                            "ai_get_key",
                            false,
                            pa,
                            &hov_t,
                            &mut new_hits,
                        );
                        if has_key {
                            draw_row_divider(rt, b, dx_l, dx_r, top + 108.0, pa);
                            draw_action_row(
                                rt,
                                b,
                                gfx,
                                dx_l,
                                dx_r,
                                top + 108.0,
                                54.0,
                                "Remove saved key",
                                Some("Clears the API key from eave.cfg"),
                                "Clear",
                                "ai_clear_key",
                                true,
                                pa,
                                &hov_t,
                                &mut new_hits,
                            );
                        }
                    });

                    cy = draw_section_header(
                        rt,
                        brush,
                        &gfx.fmt_section,
                        "Shortcut",
                        dx_l,
                        dx_r,
                        cy,
                        pa,
                    );
                    let hk_labels: [&str; 4] = [
                        crate::ai::AI_HOTKEYS[0].0,
                        crate::ai::AI_HOTKEYS[1].0,
                        crate::ai::AI_HOTKEYS[2].0,
                        crate::ai::AI_HOTKEYS[3].0,
                    ];
                    cy = draw_group_card(rt, brush, dx_l, dx_r, cy, 83.0, pa, |rt, b, top| {
                        draw_segmented_row(
                            rt,
                            b,
                            gfx,
                            dx_l,
                            dx_r,
                            top,
                            "Global hotkey",
                            Some("Press anywhere on Windows to open Eave AI"),
                            &hk_labels,
                            4,
                            self.anim("seg_ai_hotkey", self.snap.cfg.ai_hotkey.min(3) as f32),
                            "seg_ai_hotkey",
                            pa,
                            &hov_t,
                            &mut new_hits,
                        );
                    });
                }
                SettingsTab::Pages => {
                    cy = draw_section_header(
                        rt,
                        brush,
                        &gfx.fmt_section,
                        "IN YOUR NOTCH",
                        dx_l,
                        dx_r,
                        cy,
                        pa,
                    );

                    const PAGES_META: [(&str, &str, (f32, f32, f32)); 6] = [
                        ("Music", "Album art, visualizer & synced lyrics", (1.0, 0.25, 0.42)),
                        ("Focus", "Focus sprints, short & long breaks", (0.55, 0.36, 0.98)),
                        ("Water", "Daily hydration tracker & hourly chart", (0.14, 0.60, 1.0)),
                        ("Shelf", "Drag-and-drop file tray & clipboard", (0.96, 0.72, 0.08)),
                        ("To-do", "Quick checklist & wide focus view", (0.20, 0.80, 0.36)),
                        ("Widgets", "Two customizable glance cards", (0.08, 0.56, 1.0)),
                    ];
                    const PAGE_ANIM_KEYS: [&str; 6] = [
                        "page_en_0",
                        "page_en_1",
                        "page_en_2",
                        "page_en_3",
                        "page_en_4",
                        "page_en_5",
                    ];
                    let order = self.snap.cfg.page_order;
                    let enabled_arr = self.snap.cfg.page_enabled;
                    let enabled_cnt = enabled_arr.iter().filter(|&&b| b).count();
                    let dragged = self.dragging_page;
                    let card_h = 6.0 * 52.0;
                    self.pages_card_top = cy;
                    let row_ys = self.page_row_y;
                    let en_anims = [
                        self.anim(PAGE_ANIM_KEYS[0], if enabled_arr[0] { 1.0 } else { 0.0 })
                            .clamp(0.0, 1.0),
                        self.anim(PAGE_ANIM_KEYS[1], if enabled_arr[1] { 1.0 } else { 0.0 })
                            .clamp(0.0, 1.0),
                        self.anim(PAGE_ANIM_KEYS[2], if enabled_arr[2] { 1.0 } else { 0.0 })
                            .clamp(0.0, 1.0),
                        self.anim(PAGE_ANIM_KEYS[3], if enabled_arr[3] { 1.0 } else { 0.0 })
                            .clamp(0.0, 1.0),
                        self.anim(PAGE_ANIM_KEYS[4], if enabled_arr[4] { 1.0 } else { 0.0 })
                            .clamp(0.0, 1.0),
                        self.anim(PAGE_ANIM_KEYS[5], if enabled_arr[5] { 1.0 } else { 0.0 })
                            .clamp(0.0, 1.0),
                    ];
                    let drag_lift = self.anim("page_drag_lift", if dragged.is_some() { 1.0 } else { 0.0 }).clamp(0.0, 1.0);
                    cy = draw_group_card(rt, brush, dx_l, dx_r, cy, card_h, pa, |rt, b, top| {
                        for s in 1..6 {
                            draw_row_divider(rt, b, dx_l, dx_r, top + s as f32 * 52.0, pa);
                        }
                        for pass in 0..2 {
                            for (slot, &page_id) in order.iter().enumerate() {
                                let pid = (page_id as usize).min(5);
                                let is_dragged = dragged.is_some_and(|(dpid, _, _)| dpid == pid);
                                if (pass == 0 && is_dragged) || (pass == 1 && !is_dragged) {
                                    continue;
                                }
                                let (name, desc, accent) = PAGES_META[pid];
                                let is_en = enabled_arr[pid];
                                let en_t = en_anims[pid];
                                let row_dim = 0.48 + 0.52 * en_t;
                                let ry = if let Some((_, _, cur_y)) = dragged.filter(|(dpid, _, _)| *dpid == pid) {
                                    cur_y.clamp(top - 4.0, top + card_h - 48.0)
                                } else {
                                    top + row_ys[pid]
                                };

                                if is_dragged && drag_lift > 0.01 {
                                    let elev_r = D2D_RECT_F {
                                        left: dx_l + 3.0,
                                        top: ry + 2.0,
                                        right: dx_r - 3.0,
                                        bottom: ry + 50.0,
                                    };
                                    set_col(b, 0.21, 0.21, 0.24, 0.98 * drag_lift * pa);
                                    rt.FillRoundedRectangle(
                                        &D2D1_ROUNDED_RECT {
                                            rect: elev_r,
                                            radiusX: 8.5,
                                            radiusY: 8.5,
                                        },
                                        b,
                                    );
                                    set_col(b, 1.0, 1.0, 1.0, 0.14 * drag_lift * pa);
                                    rt.DrawRoundedRectangle(
                                        &D2D1_ROUNDED_RECT {
                                            rect: elev_r,
                                            radiusX: 8.5,
                                            radiusY: 8.5,
                                        },
                                        b,
                                        1.0,
                                        None,
                                    );
                                }

                                set_col(b, 0.56, 0.56, 0.58, pa * row_dim);
                                let idx_s = format!("{}", slot + 1);
                                draw_text(
                                    rt,
                                    &idx_s,
                                    &gfx.fmt_seg,
                                    &D2D_RECT_F {
                                        left: dx_l + 6.0,
                                        top: ry,
                                        right: dx_l + 22.0,
                                        bottom: ry + 52.0,
                                    },
                                    b,
                                );
                                draw_page_mini_thumb(
                                    rt,
                                    b,
                                    dx_l + 26.0,
                                    ry + 14.0,
                                    pid,
                                    accent,
                                    pa * row_dim,
                                );
                                set_col(b, 0.97, 0.97, 0.98, pa * row_dim);
                                draw_text(
                                    rt,
                                    name,
                                    &gfx.fmt_row,
                                    &D2D_RECT_F {
                                        left: dx_l + 82.0,
                                        top: ry + 8.0,
                                        right: dx_r - 86.0,
                                        bottom: ry + 26.0,
                                    },
                                    b,
                                );
                                set_col(b, 0.55, 0.55, 0.57, pa * row_dim);
                                draw_text(
                                    rt,
                                    desc,
                                    &gfx.fmt_sub,
                                    &D2D_RECT_F {
                                        left: dx_l + 82.0,
                                        top: ry + 26.0,
                                        right: dx_r - 86.0,
                                        bottom: ry + 44.0,
                                    },
                                    b,
                                );

                                let handle_cx = dx_r - 68.0;
                                let handle_cy = ry + 26.0;
                                let row_drag_rect = D2D_RECT_F {
                                    left: dx_l + 4.0,
                                    top: ry + 2.0,
                                    right: dx_r - 56.0,
                                    bottom: ry + 50.0,
                                };
                                let drag_key = format!("page_drag_{pid}");
                                let drag_h = if is_dragged {
                                    1.0
                                } else {
                                    hov_t(&drag_key, row_drag_rect)
                                };
                                set_col(
                                    b,
                                    0.90,
                                    0.90,
                                    0.94,
                                    (0.44 + 0.44 * drag_h) * pa,
                                );
                                draw_mini_chevron(rt, b, handle_cx, handle_cy - 4.2, true);
                                draw_mini_chevron(rt, b, handle_cx, handle_cy + 4.2, false);
                                new_hits.push(HitBox {
                                    rect: row_drag_rect,
                                    hit: ControlHit::PageDrag(pid),
                                });

                                let locked_last = is_en && enabled_cnt <= 1;
                                let sw_w = 36.0;
                                let sw_h = 20.0;
                                let sw_r = D2D_RECT_F {
                                    left: dx_r - 14.0 - sw_w,
                                    top: ry + (52.0 - sw_h) * 0.5,
                                    right: dx_r - 14.0,
                                    bottom: ry + (52.0 + sw_h) * 0.5,
                                };
                                let sw_a = if locked_last { 0.55 * pa } else { pa };
                                set_col(
                                    b,
                                    0.24 + (0.204 - 0.24) * en_t,
                                    0.24 + (0.780 - 0.24) * en_t,
                                    0.26 + (0.349 - 0.26) * en_t,
                                    sw_a,
                                );
                                rt.FillRoundedRectangle(
                                    &D2D1_ROUNDED_RECT {
                                        rect: sw_r,
                                        radiusX: 10.0,
                                        radiusY: 10.0,
                                    },
                                    b,
                                );
                                let tcx = (sw_r.left + 10.0) + (sw_w - 20.0) * en_t;
                                let tcy = (sw_r.top + sw_r.bottom) * 0.5;
                                set_col(b, 0.99, 0.99, 1.0, sw_a);
                                rt.FillEllipse(
                                    &D2D1_ELLIPSE {
                                        point: D2D_POINT_2F { x: tcx, y: tcy },
                                        radiusX: 8.0,
                                        radiusY: 8.0,
                                    },
                                    b,
                                );
                                if !locked_last {
                                    new_hits.push(HitBox {
                                        rect: D2D_RECT_F {
                                            left: sw_r.left - 4.0,
                                            top: ry + 8.0,
                                            right: sw_r.right + 4.0,
                                            bottom: ry + 44.0,
                                        },
                                        hit: ControlHit::PageToggle(pid),
                                    });
                                }
                            }
                        }
                    });
                }
                SettingsTab::Music => {
                    cy = draw_section_header(rt, brush, &gfx.fmt_section, "Lyrics", dx_l, dx_r, cy, pa);
                    cy = draw_group_card(rt, brush, dx_l, dx_r, cy, 54.0, pa, |rt, b, top| {
                        draw_toggle_row(
                            rt,
                            b,
                            gfx,
                            dx_l,
                            dx_r,
                            top,
                            54.0,
                            "Synced lyrics",
                            Some("Fetch live time-synced lyrics via LRCLIB"),
                            self.anim("show_lyrics", if self.snap.cfg.show_lyrics { 1.0 } else { 0.0 }),
                            "show_lyrics",
                            pa,
                            &mut new_hits,
                        );
                    });

                    cy = draw_section_header(rt, brush, &gfx.fmt_section, "Playback & Audio", dx_l, dx_r, cy, pa);
                    cy = draw_group_card(rt, brush, dx_l, dx_r, cy, 54.0, pa, |rt, b, top| {
                        draw_info_row(
                            rt,
                            b,
                            gfx,
                            dx_l,
                            dx_r,
                            top,
                            54.0,
                            "Media source",
                            Some("Windows SMTC (Spotify, Browser, Apple Music)"),
                            "Automatic",
                            pa,
                        );
                    });
                }
                SettingsTab::Widgets => {
                    cy = draw_section_header(rt, brush, &gfx.fmt_section, "Widgets page", dx_l, dx_r, cy, pa);
                    cy = draw_group_card(rt, brush, dx_l, dx_r, cy, 54.0, pa, |rt, b, top| {
                        draw_toggle_row(
                            rt,
                            b,
                            gfx,
                            dx_l,
                            dx_r,
                            top,
                            54.0,
                            "Show Widgets page",
                            Some("Two widgets of your choice, one hover away"),
                            self.anim("page_en_5", if self.snap.cfg.page_enabled[5] { 1.0 } else { 0.0 }),
                            "show_widgets_page",
                            pa,
                            &mut new_hits,
                        );
                    });

                    cy = draw_section_header(rt, brush, &gfx.fmt_section, "Choose widgets", dx_l, dx_r, cy, pa);
                    let slot_f = self
                        .anim("widget_slot", self.widget_edit_slot as f32)
                        .clamp(0.0, 1.0);
                    let wg_left = self.snap.cfg.widget_left;
                    let wg_right = self.snap.cfg.widget_right;
                    let wg_sels = [
                        self.anim("wg_sel_0", if (if self.widget_edit_slot == 0 { wg_left } else { wg_right }) == 0 { 1.0 } else { 0.0 }).clamp(0.0, 1.0),
                        self.anim("wg_sel_1", if (if self.widget_edit_slot == 0 { wg_left } else { wg_right }) == 1 { 1.0 } else { 0.0 }).clamp(0.0, 1.0),
                        self.anim("wg_sel_2", if (if self.widget_edit_slot == 0 { wg_left } else { wg_right }) == 2 { 1.0 } else { 0.0 }).clamp(0.0, 1.0),
                        self.anim("wg_sel_3", if (if self.widget_edit_slot == 0 { wg_left } else { wg_right }) == 3 { 1.0 } else { 0.0 }).clamp(0.0, 1.0),
                        self.anim("wg_sel_4", if (if self.widget_edit_slot == 0 { wg_left } else { wg_right }) == 4 { 1.0 } else { 0.0 }).clamp(0.0, 1.0),
                        self.anim("wg_sel_5", if (if self.widget_edit_slot == 0 { wg_left } else { wg_right }) == 5 { 1.0 } else { 0.0 }).clamp(0.0, 1.0),
                        self.anim("wg_sel_6", if (if self.widget_edit_slot == 0 { wg_left } else { wg_right }) == 6 { 1.0 } else { 0.0 }).clamp(0.0, 1.0),
                        self.anim("wg_sel_7", if (if self.widget_edit_slot == 0 { wg_left } else { wg_right }) == 7 { 1.0 } else { 0.0 }).clamp(0.0, 1.0),
                    ];
                    cy = draw_widget_picker_card(
                        rt,
                        brush,
                        gfx,
                        dx_l,
                        dx_r,
                        cy,
                        slot_f,
                        wg_left,
                        wg_right,
                        wg_sels,
                        pa,
                        &hov_t,
                        &mut new_hits,
                    );

                    cy = draw_section_header(rt, brush, &gfx.fmt_section, "Options", dx_l, dx_r, cy, pa);
                    let city1 = WORLD_CITIES[(self.snap.cfg.widget_city_1 as usize).min(WORLD_CITIES.len() - 1)].0;
                    let city2 = WORLD_CITIES[(self.snap.cfg.widget_city_2 as usize).min(WORLD_CITIES.len() - 1)].0;
                    let ev_name = COUNTDOWN_TARGETS[(self.snap.cfg.widget_countdown as usize).min(COUNTDOWN_TARGETS.len() - 1)].0;
                    let weather_city = WEATHER_CITIES
                        [(self.snap.cfg.widget_weather_city as usize).min(WEATHER_CITIES.len() - 1)]
                        .0;
                    cy = draw_group_card(rt, brush, dx_l, dx_r, cy, 216.0, pa, |rt, b, top| {
                        draw_stepper_row(
                            rt,
                            b,
                            gfx,
                            dx_l,
                            dx_r,
                            top,
                            54.0,
                            "World clock, first city",
                            Some("Top row in the World clock widget"),
                            city1,
                            "widget_city_1",
                            1,
                            false,
                            pa,
                            &hov_t,
                            &mut new_hits,
                        );
                        draw_row_divider(rt, b, dx_l, dx_r, top + 54.0, pa);
                        draw_stepper_row(
                            rt,
                            b,
                            gfx,
                            dx_l,
                            dx_r,
                            top + 54.0,
                            54.0,
                            "World clock, second city",
                            Some("Bottom row in the World clock widget"),
                            city2,
                            "widget_city_2",
                            1,
                            false,
                            pa,
                            &hov_t,
                            &mut new_hits,
                        );
                        draw_row_divider(rt, b, dx_l, dx_r, top + 108.0, pa);
                        draw_stepper_row(
                            rt,
                            b,
                            gfx,
                            dx_l,
                            dx_r,
                            top + 108.0,
                            54.0,
                            "Countdown event",
                            Some("Target date for the Countdown widget"),
                            ev_name,
                            "widget_countdown",
                            1,
                            false,
                            pa,
                            &hov_t,
                            &mut new_hits,
                        );
                        draw_row_divider(rt, b, dx_l, dx_r, top + 162.0, pa);
                        draw_stepper_row(
                            rt,
                            b,
                            gfx,
                            dx_l,
                            dx_r,
                            top + 162.0,
                            54.0,
                            "Weather location",
                            Some("Auto-detects city or pick manually · updates every 30m"),
                            weather_city,
                            "widget_weather_city",
                            1,
                            false,
                            pa,
                            &hov_t,
                            &mut new_hits,
                        );
                    });
                }
                SettingsTab::Focus => {
                    cy = draw_section_header(rt, brush, &gfx.fmt_section, "Timer durations", dx_l, dx_r, cy, pa);
                    cy = draw_group_card(rt, brush, dx_l, dx_r, cy, 249.0, pa, |rt, b, top| {
                        draw_segmented_row(
                            rt,
                            b,
                            gfx,
                            dx_l,
                            dx_r,
                            top,
                            "Focus session",
                            Some("Length of each focus sprint"),
                            &["15 min", "25 min", "30 min", "45 min"],
                            4,
                            self.anim("seg_focus", 1.0),
                            "seg_focus",
                            pa,
                            &hov_t,
                            &mut new_hits,
                        );
                        draw_row_divider(rt, b, dx_l, dx_r, top + 83.0, pa);
                        draw_segmented_row(
                            rt,
                            b,
                            gfx,
                            dx_l,
                            dx_r,
                            top + 83.0,
                            "Short break",
                            Some("Quick pause between focus sessions"),
                            &["5 min", "10 min"],
                            2,
                            self.anim("seg_break", 0.0),
                            "seg_break",
                            pa,
                            &hov_t,
                            &mut new_hits,
                        );
                        draw_row_divider(rt, b, dx_l, dx_r, top + 166.0, pa);
                        draw_segmented_row(
                            rt,
                            b,
                            gfx,
                            dx_l,
                            dx_r,
                            top + 166.0,
                            "Long break",
                            Some("Rest period after completing round 4"),
                            &["15 min", "20 min", "30 min"],
                            3,
                            self.anim("seg_long", 0.0),
                            "seg_long",
                            pa,
                            &hov_t,
                            &mut new_hits,
                        );
                    });
                }
                SettingsTab::Water => {
                    cy = draw_section_header(rt, brush, &gfx.fmt_section, "Hydration", dx_l, dx_r, cy, pa);
                    cy = draw_group_card(rt, brush, dx_l, dx_r, cy, 166.0, pa, |rt, b, top| {
                        draw_segmented_row(
                            rt,
                            b,
                            gfx,
                            dx_l,
                            dx_r,
                            top,
                            "Daily goal",
                            Some("Target daily water intake"),
                            &["1.5 L", "2.0 L", "2.5 L", "3.0 L"],
                            4,
                            self.anim("seg_goal", 1.0),
                            "seg_goal",
                            pa,
                            &hov_t,
                            &mut new_hits,
                        );
                        draw_row_divider(rt, b, dx_l, dx_r, top + 83.0, pa);
                        draw_segmented_row(
                            rt,
                            b,
                            gfx,
                            dx_l,
                            dx_r,
                            top + 83.0,
                            "Cup size",
                            Some("Amount added per Drink click"),
                            &["200 ml", "250 ml", "350 ml", "500 ml"],
                            4,
                            self.anim("seg_cup", 1.0),
                            "seg_cup",
                            pa,
                            &hov_t,
                            &mut new_hits,
                        );
                    });

                    cy = draw_section_header(rt, brush, &gfx.fmt_section, "Reminders", dx_l, dx_r, cy, pa);
                    cy = draw_group_card(rt, brush, dx_l, dx_r, cy, 54.0, pa, |rt, b, top| {
                        draw_toggle_row(
                            rt,
                            b,
                            gfx,
                            dx_l,
                            dx_r,
                            top,
                            54.0,
                            "Water reminders",
                            Some("Spread across your active hours"),
                            self.anim("water_reminder", if self.snap.water_reminder_enabled { 1.0 } else { 0.0 }),
                            "water_reminder",
                            pa,
                            &mut new_hits,
                        );
                    });
                }
                SettingsTab::Todo => {
                    cy = draw_section_header(
                        rt,
                        brush,
                        &gfx.fmt_section,
                        "Task Alerts",
                        dx_l,
                        dx_r,
                        cy,
                        pa,
                    );
                    let rem_start_v = self.anim(
                        "todo_remind_start",
                        if self.snap.cfg.todo_remind_start { 1.0 } else { 0.0 },
                    );
                    let rem_end_v = self.anim(
                        "todo_remind_end",
                        if self.snap.cfg.todo_remind_end { 1.0 } else { 0.0 },
                    );
                    let snooze_v = self.anim(
                        "seg_todo_snooze",
                        self.snap.cfg.todo_snooze_mins.min(2) as f32,
                    );
                    cy = draw_group_card(rt, brush, dx_l, dx_r, cy, 191.0, pa, |rt, b, top| {
                        draw_toggle_row(
                            rt,
                            b,
                            gfx,
                            dx_l,
                            dx_r,
                            top,
                            54.0,
                            "Notify at start time",
                            Some("Show a notch alert when a scheduled task begins"),
                            rem_start_v,
                            "todo_remind_start",
                            pa,
                            &mut new_hits,
                        );
                        draw_row_divider(rt, b, dx_l, dx_r, top + 54.0, pa);
                        draw_toggle_row(
                            rt,
                            b,
                            gfx,
                            dx_l,
                            dx_r,
                            top + 54.0,
                            54.0,
                            "Notify at end time",
                            Some("Alert when a task's scheduled window finishes"),
                            rem_end_v,
                            "todo_remind_end",
                            pa,
                            &mut new_hits,
                        );
                        draw_row_divider(rt, b, dx_l, dx_r, top + 108.0, pa);
                        draw_segmented_row(
                            rt,
                            b,
                            gfx,
                            dx_l,
                            dx_r,
                            top + 108.0,
                            "Remind again in",
                            Some("Repeat alert if task is still unchecked"),
                            &["5 min", "10 min", "15 min"],
                            3,
                            snooze_v,
                            "seg_todo_snooze",
                            pa,
                            &hov_t,
                            &mut new_hits,
                        );
                    });

                    cy = draw_section_header(rt, brush, &gfx.fmt_section, "Storage", dx_l, dx_r, cy, pa);
                    let stats = format!("{} / {}", self.snap.todo_done, self.snap.todo_total);
                    cy = draw_group_card(rt, brush, dx_l, dx_r, cy, 108.0, pa, |rt, b, top| {
                        draw_info_row(
                            rt,
                            b,
                            gfx,
                            dx_l,
                            dx_r,
                            top,
                            54.0,
                            "Completed today",
                            Some("Saved locally on this device"),
                            &stats,
                            pa,
                        );
                        draw_row_divider(rt, b, dx_l, dx_r, top + 54.0, pa);
                        draw_action_row(
                            rt,
                            b,
                            gfx,
                            dx_l,
                            dx_r,
                            top + 54.0,
                            54.0,
                            "Clear completed tasks",
                            Some("Remove checked items from your list"),
                            "Clear",
                            "clear_done_todos",
                            false,
                            pa,
                            &hov_t,
                            &mut new_hits,
                        );
                    });
                }
                SettingsTab::Shelf => {
                    cy = draw_section_header(rt, brush, &gfx.fmt_section, "Storage", dx_l, dx_r, cy, pa);
                    let f_cnt = format!("{} items", self.snap.shelf_files_count);
                    let c_cnt = format!("{} items", self.snap.shelf_clip_count);
                    cy = draw_group_card(rt, brush, dx_l, dx_r, cy, 108.0, pa, |rt, b, top| {
                        draw_action_row(
                            rt,
                            b,
                            gfx,
                            dx_l,
                            dx_r,
                            top,
                            54.0,
                            "Shelf files",
                            Some(&f_cnt),
                            "Clear files",
                            "clear_shelf_files",
                            false,
                            pa,
                            &hov_t,
                            &mut new_hits,
                        );
                        draw_row_divider(rt, b, dx_l, dx_r, top + 54.0, pa);
                        draw_action_row(
                            rt,
                            b,
                            gfx,
                            dx_l,
                            dx_r,
                            top + 54.0,
                            54.0,
                            "Clipboard history",
                            Some(&c_cnt),
                            "Clear clips",
                            "clear_shelf_clip",
                            false,
                            pa,
                            &hov_t,
                            &mut new_hits,
                        );
                    });

                    cy = draw_section_header(
                        rt,
                        brush,
                        &gfx.fmt_section,
                        "Smart OCR",
                        dx_l,
                        dx_r,
                        cy,
                        pa,
                    );
                    cy = draw_group_card(rt, brush, dx_l, dx_r, cy, 58.0, pa, |rt, b, top| {
                        draw_action_row(
                            rt,
                            b,
                            gfx,
                            dx_l,
                            dx_r,
                            top,
                            58.0,
                            "Copy text from screen",
                            Some("Select any area and Eave copies its text"),
                            "Ctrl + Alt + X",
                            "smart_ocr",
                            false,
                            pa,
                            &hov_t,
                            &mut new_hits,
                        );
                    });
                }
                SettingsTab::About => {
                    cy = draw_group_card(rt, brush, dx_l, dx_r, cy, 102.0, pa, |rt, b, top| {
                        let b_ref: &ID2D1Brush = b;
                        let ix = dx_l + 18.0;
                        let iy = top + 23.0;
                        let icon_rect = D2D_RECT_F {
                            left: ix,
                            top: iy,
                            right: ix + 56.0,
                            bottom: iy + 56.0,
                        };
                        if let Some(ref bmp) = gfx.eave_icon {
                            rt.DrawBitmap(
                                bmp,
                                Some(&icon_rect),
                                pa,
                                windows::Win32::Graphics::Direct2D::D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                                None,
                            );
                        }

                        let tx = ix + 70.0;
                        set_col(b, 0.96, 0.97, 0.99, 0.98 * pa);
                        draw_text(
                            rt,
                            crate::updater::APP_NAME,
                            &gfx.fmt_title,
                            &D2D_RECT_F {
                                left: tx,
                                top: top + 18.0,
                                right: tx + 76.0,
                                bottom: top + 44.0,
                            },
                            b_ref,
                        );

                        let ver_badge_text = match &self.snap.update_status {
                            crate::updater::UpdateStatus::Available { version, .. } => {
                                format!("v{} -> v{}", crate::updater::CURRENT_VERSION, version)
                            }
                            _ => format!("v{}", crate::updater::CURRENT_VERSION),
                        };
                        let is_avail = matches!(
                            self.snap.update_status,
                            crate::updater::UpdateStatus::Available { .. }
                        );
                        let badge_w = if is_avail { 104.0 } else { 48.0 };
                        let badge_rect = D2D_RECT_F {
                            left: tx + 56.0,
                            top: top + 22.0,
                            right: tx + 56.0 + badge_w,
                            bottom: top + 40.0,
                        };
                        let badge_rr = D2D1_ROUNDED_RECT {
                            rect: badge_rect,
                            radiusX: 9.0,
                            radiusY: 9.0,
                        };
                        if is_avail {
                            set_col(b, 0.12, 0.42, 0.92, 0.34 * pa);
                        } else {
                            set_col(b, 1.0, 1.0, 1.0, 0.09 * pa);
                        }
                        rt.FillRoundedRectangle(&badge_rr, b_ref);
                        if is_avail {
                            set_col(b, 0.32, 0.70, 1.0, 0.55 * pa);
                        } else {
                            set_col(b, 1.0, 1.0, 1.0, 0.14 * pa);
                        }
                        rt.DrawRoundedRectangle(&badge_rr, b_ref, 0.85, None);
                        if is_avail {
                            set_col(b, 0.58, 0.85, 1.0, 0.98 * pa);
                        } else {
                            set_col(b, 0.86, 0.88, 0.92, 0.92 * pa);
                        }
                        draw_text(rt, &ver_badge_text, &gfx.fmt_seg_on, &badge_rect, b_ref);

                        set_col(b, 0.84, 0.86, 0.90, 0.92 * pa);
                        draw_text(
                            rt,
                            "Dynamic Island companion for Windows",
                            &gfx.fmt_row,
                            &D2D_RECT_F {
                                left: tx,
                                top: top + 48.0,
                                right: dx_r - 16.0,
                                bottom: top + 66.0,
                            },
                            b_ref,
                        );
                        set_col(b, 0.56, 0.58, 0.64, 0.92 * pa);
                        draw_text(
                            rt,
                            "Pure Rust · DirectComposition + Direct2D · 0% idle CPU",
                            &gfx.fmt_sub,
                            &D2D_RECT_F {
                                left: tx,
                                top: top + 68.0,
                                right: dx_r - 16.0,
                                bottom: top + 86.0,
                            },
                            b_ref,
                        );
                    });

                    cy = draw_section_header(
                        rt,
                        brush,
                        &gfx.fmt_section,
                        "Software Update",
                        dx_l,
                        dx_r,
                        cy,
                        pa,
                    );
                    let auto_upd_val = self.anim(
                        "auto_check_updates",
                        if self.snap.cfg.auto_check_updates { 1.0 } else { 0.0 },
                    );
                    let (upd_title, upd_sub, upd_btn_lbl, upd_btn_id, dot_col) =
                        match &self.snap.update_status {
                            crate::updater::UpdateStatus::Idle => (
                                format!("Current version v{}", crate::updater::CURRENT_VERSION),
                                "Check GitHub Releases for the latest build".to_string(),
                                "Check now",
                                "check_updates",
                                (0.56, 0.60, 0.66),
                            ),
                            crate::updater::UpdateStatus::Checking => (
                                "Checking for updates...".to_string(),
                                "Contacting GitHub Releases API".to_string(),
                                "Checking...",
                                "check_updates",
                                (0.30, 0.68, 1.0),
                            ),
                            crate::updater::UpdateStatus::UpToDate { checked_at } => (
                                "Eave is up to date".to_string(),
                                format!(
                                    "v{} is the latest version · Checked at {}",
                                    crate::updater::CURRENT_VERSION,
                                    checked_at
                                ),
                                "Check again",
                                "check_updates",
                                (0.22, 0.86, 0.44),
                            ),
                            crate::updater::UpdateStatus::NoReleases { checked_at } => (
                                format!("Running v{}", crate::updater::CURRENT_VERSION),
                                format!("No GitHub Releases published yet · Checked at {checked_at}"),
                                "Check again",
                                "check_updates",
                                (0.22, 0.86, 0.44),
                            ),
                            crate::updater::UpdateStatus::Available { version, .. } => (
                                format!("Update available: v{version}"),
                                "A newer release is ready on GitHub".to_string(),
                                "Download",
                                "download_update",
                                (0.24, 0.66, 1.0),
                            ),
                            crate::updater::UpdateStatus::Error(err) => (
                                "Update check failed".to_string(),
                                err.clone(),
                                "Retry",
                                "check_updates",
                                (0.98, 0.68, 0.24),
                            ),
                        };

                    cy = draw_group_card(rt, brush, dx_l, dx_r, cy, 112.0, pa, |rt, b, top| {
                        draw_toggle_row(
                            rt,
                            b,
                            gfx,
                            dx_l,
                            dx_r,
                            top,
                            54.0,
                            "Automatic update checks",
                            Some("Check GitHub Releases automatically on startup"),
                            auto_upd_val,
                            "auto_check_updates",
                            pa,
                            &mut new_hits,
                        );
                        draw_row_divider(rt, b, dx_l, dx_r, top + 54.0, pa);

                        let b_ref: &ID2D1Brush = b;
                        let dot_x = dx_l + 20.0;
                        let dot_y = top + 54.0 + 20.0;
                        set_col(b, dot_col.0, dot_col.1, dot_col.2, 0.96 * pa);
                        rt.FillEllipse(
                            &D2D1_ELLIPSE {
                                point: D2D_POINT_2F { x: dot_x, y: dot_y },
                                radiusX: 3.6,
                                radiusY: 3.6,
                            },
                            b_ref,
                        );
                        draw_action_row(
                            rt,
                            b,
                            gfx,
                            dx_l + 14.0,
                            dx_r,
                            top + 54.0,
                            58.0,
                            &upd_title,
                            Some(&upd_sub),
                            upd_btn_lbl,
                            upd_btn_id,
                            false,
                            pa,
                            &hov_t,
                            &mut new_hits,
                        );
                    });

                    cy = draw_section_header(
                        rt,
                        brush,
                        &gfx.fmt_section,
                        "Project & Configuration",
                        dx_l,
                        dx_r,
                        cy,
                        pa,
                    );
                    let repo_sub = format!(
                        "github.com/{}",
                        crate::updater::sanitize_repo(&self.snap.cfg.update_repo)
                    );
                    cy = draw_group_card(rt, brush, dx_l, dx_r, cy, 162.0, pa, |rt, b, top| {
                        draw_action_row(
                            rt,
                            b,
                            gfx,
                            dx_l,
                            dx_r,
                            top,
                            54.0,
                            "GitHub Repository",
                            Some(&repo_sub),
                            "Open",
                            "open_github",
                            false,
                            pa,
                            &hov_t,
                            &mut new_hits,
                        );
                        draw_row_divider(rt, b, dx_l, dx_r, top + 54.0, pa);
                        draw_action_row(
                            rt,
                            b,
                            gfx,
                            dx_l,
                            dx_r,
                            top + 54.0,
                            54.0,
                            "Config file",
                            Some("Open eave.cfg in Notepad for raw editing"),
                            "Open file",
                            "open_cfg",
                            false,
                            pa,
                            &hov_t,
                            &mut new_hits,
                        );
                        draw_row_divider(rt, b, dx_l, dx_r, top + 108.0, pa);
                        draw_action_row(
                            rt,
                            b,
                            gfx,
                            dx_l,
                            dx_r,
                            top + 108.0,
                            54.0,
                            "Reset settings",
                            Some("Restore default notch size, position and toggles"),
                            "Reset",
                            "reset_defaults",
                            true,
                            pa,
                            &hov_t,
                            &mut new_hits,
                        );
                    });
                }
            }

                if is_active_pass {
                    let content_h = (cy - start_y) + 18.0;
                    let view_h = (body_clip.bottom - body_clip.top).max(1.0);
                    next_max_scroll = (content_h - view_h).max(0.0);
                } else {
                    new_hits.truncate(saved_hits_len);
                }

                rt.PopAxisAlignedClip();
                if pass_dx.abs() > 0.05 {
                    rt.SetTransform(&Matrix3x2::identity());
                }
            }

            self.max_scroll = next_max_scroll;
            self.scroll_target = self.scroll_target.clamp(0.0, self.max_scroll);
            self.scroll_y = self.scroll_y.clamp(0.0, self.max_scroll);

            rt.PopAxisAlignedClip();

            set_col(brush, 1.0, 1.0, 1.0, 0.09);
            rt.DrawRectangle(
                &D2D_RECT_F {
                    left: 0.5,
                    top: 0.5,
                    right: WIN_W_DIPS - 0.5,
                    bottom: WIN_H_DIPS - 0.5,
                },
                brush_ref,
                1.0,
                None,
            );

            let _ = rt.EndDraw(None, None);
        }

        self.hit_boxes = new_hits;
    }
}

#[inline]
unsafe fn set_col(brush: &ID2D1SolidColorBrush, r: f32, g: f32, b: f32, a: f32) {
    brush.SetColor(&D2D1_COLOR_F { r, g, b, a });
}

unsafe fn draw_text(
    rt: &ID2D1RenderTarget,
    text: &str,
    fmt: &IDWriteTextFormat,
    rect: &D2D_RECT_F,
    brush: &ID2D1Brush,
) {
    let snapped = D2D_RECT_F {
        left: rect.left.round(),
        top: rect.top.round(),
        right: rect.right.round(),
        bottom: rect.bottom.round(),
    };
    let wide: Vec<u16> = text.encode_utf16().collect();
    rt.DrawText(
        &wide,
        fmt,
        &snapped,
        brush,
        windows::Win32::Graphics::Direct2D::D2D1_DRAW_TEXT_OPTIONS_CLIP,
        windows::Win32::Graphics::DirectWrite::DWRITE_MEASURING_MODE_NATURAL,
    );
}

#[allow(clippy::too_many_arguments)]
unsafe fn draw_eye_preview_button(
    rt: &ID2D1RenderTarget,
    brush: &ID2D1SolidColorBrush,
    gfx: &GfxResources,
    rect: D2D_RECT_F,
    active_t: f32,
    hov_t: f32,
    pa: f32,
) {
    let at = active_t.clamp(0.0, 1.0);
    let ht = hov_t.clamp(0.0, 1.0);
    let rrect = D2D1_ROUNDED_RECT {
        rect,
        radiusX: 7.0,
        radiusY: 7.0,
    };
    let bg_r = 1.0 + (0.12 - 1.0) * at;
    let bg_g = 1.0 + (0.68 - 1.0) * at;
    let bg_b = 1.0 + (1.00 - 1.0) * at;
    let bg_a = (0.07 + 0.08 * ht + (0.17 + 0.06 * ht) * at) * pa;
    set_col(brush, bg_r, bg_g, bg_b, bg_a);
    rt.FillRoundedRectangle(&rrect, brush);

    let bd_r = 1.0 + (0.28 - 1.0) * at;
    let bd_g = 1.0 + (0.78 - 1.0) * at;
    let bd_b = 1.0 + (1.00 - 1.0) * at;
    let bd_a = (0.12 + 0.14 * ht + 0.42 * at) * pa;
    set_col(brush, bd_r, bd_g, bd_b, bd_a);
    rt.DrawRoundedRectangle(&rrect, brush, 1.0, None);

    let cx = (rect.left + rect.right) * 0.5;
    let cy = (rect.top + rect.bottom) * 0.5;
    let ew = 6.2;
    let eh = 4.2;
    let fg_r = 0.72 + (0.48 - 0.72) * at + 0.18 * ht * (1.0 - at);
    let fg_g = 0.74 + (0.90 - 0.74) * at + 0.16 * ht * (1.0 - at);
    let fg_b = 0.78 + (1.00 - 0.78) * at + 0.14 * ht * (1.0 - at);
    set_col(brush, fg_r, fg_g, fg_b, (0.85 + 0.15 * at) * pa);

    if let Ok(eye_geo) = gfx.d2d_factory.CreatePathGeometry() {
        if let Ok(sink) = eye_geo.Open() {
            sink.BeginFigure(D2D_POINT_2F { x: cx - ew, y: cy }, D2D1_FIGURE_BEGIN_HOLLOW);
            sink.AddBezier(&D2D1_BEZIER_SEGMENT {
                point1: D2D_POINT_2F { x: cx - ew * 0.48, y: cy - eh },
                point2: D2D_POINT_2F { x: cx + ew * 0.48, y: cy - eh },
                point3: D2D_POINT_2F { x: cx + ew, y: cy },
            });
            sink.EndFigure(D2D1_FIGURE_END_OPEN);
            sink.BeginFigure(D2D_POINT_2F { x: cx - ew, y: cy }, D2D1_FIGURE_BEGIN_HOLLOW);
            sink.AddBezier(&D2D1_BEZIER_SEGMENT {
                point1: D2D_POINT_2F { x: cx - ew * 0.48, y: cy + eh },
                point2: D2D_POINT_2F { x: cx + ew * 0.48, y: cy + eh },
                point3: D2D_POINT_2F { x: cx + ew, y: cy },
            });
            sink.EndFigure(D2D1_FIGURE_END_OPEN);
            let _ = sink.Close();
            rt.DrawGeometry(&eye_geo, brush, 1.35, None);
        }
    }
    rt.FillEllipse(
        &D2D1_ELLIPSE {
            point: D2D_POINT_2F { x: cx, y: cy },
            radiusX: 2.1,
            radiusY: 2.1,
        },
        brush,
    );
}

#[allow(clippy::too_many_arguments)]
unsafe fn draw_live_notch_preview(
    rt: &ID2D1RenderTarget,
    brush: &ID2D1SolidColorBrush,
    gfx: &GfxResources,
    l: f32,
    r: f32,
    y: f32,
    notch_w_anim: f32,
    min_t: f32,
    preview_eave_t: f32,
    preview_phase: f32,
    clock_t: f32,
    pa: f32,
) -> f32 {
    let h = 92.0;
    let box_r = D2D_RECT_F {
        left: l,
        top: y,
        right: r,
        bottom: y + h,
    };
    let rrect = D2D1_ROUNDED_RECT {
        rect: box_r,
        radiusX: 11.0,
        radiusY: 11.0,
    };

    let stops = [
        D2D1_GRADIENT_STOP {
            position: 0.0,
            color: D2D1_COLOR_F {
                r: 0.36,
                g: 0.80,
                b: 0.99,
                a: pa,
            },
        },
        D2D1_GRADIENT_STOP {
            position: 0.55,
            color: D2D1_COLOR_F {
                r: 0.16,
                g: 0.60,
                b: 0.96,
                a: pa,
            },
        },
        D2D1_GRADIENT_STOP {
            position: 1.0,
            color: D2D1_COLOR_F {
                r: 0.24,
                g: 0.44,
                b: 0.94,
                a: pa,
            },
        },
    ];
    if let Ok(sc) =
        rt.CreateGradientStopCollection(&stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP)
    {
        let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
            startPoint: D2D_POINT_2F { x: l, y },
            endPoint: D2D_POINT_2F { x: r, y: y + h },
        };
        if let Ok(lin) = rt.CreateLinearGradientBrush(&props, None, &sc) {
            rt.FillRoundedRectangle(&rrect, &lin);
        } else {
            set_col(brush, 0.22, 0.66, 0.96, pa);
            rt.FillRoundedRectangle(&rrect, brush);
        }
    } else {
        set_col(brush, 0.22, 0.66, 0.96, pa);
        rt.FillRoundedRectangle(&rrect, brush);
    }

    let scale_k = (notch_w_anim / 166.0).clamp(0.86, 1.18);
    let full_w = 152.0 * scale_k;
    let min_w = 88.0 * scale_k;
    let body_w = full_w + (min_w - full_w) * min_t;
    let ph = 28.0 + (19.0 - 28.0) * min_t;
    let ear_r = 9.0 + (6.0 - 9.0) * min_t;
    let bot_r = 12.0 + (8.5 - 12.0) * min_t;
    let cx = (l + r) * 0.5;
    let bl = cx - body_w * 0.5;
    let br = cx + body_w * 0.5;
    let top_y = y;
    let bot_y = y + ph;
    const K: f32 = 0.552_284_8;

    if let Ok(geo) = gfx.d2d_factory.CreatePathGeometry() {
        if let Ok(sink) = geo.Open() {
            sink.BeginFigure(
                D2D_POINT_2F {
                    x: bl - ear_r,
                    y: top_y,
                },
                D2D1_FIGURE_BEGIN_FILLED,
            );
            sink.AddBezier(&D2D1_BEZIER_SEGMENT {
                point1: D2D_POINT_2F {
                    x: bl - ear_r * (1.0 - K),
                    y: top_y,
                },
                point2: D2D_POINT_2F {
                    x: bl,
                    y: top_y + ear_r * (1.0 - K),
                },
                point3: D2D_POINT_2F {
                    x: bl,
                    y: top_y + ear_r,
                },
            });
            sink.AddLine(D2D_POINT_2F {
                x: bl,
                y: bot_y - bot_r,
            });
            sink.AddBezier(&D2D1_BEZIER_SEGMENT {
                point1: D2D_POINT_2F {
                    x: bl,
                    y: bot_y - bot_r * (1.0 - K),
                },
                point2: D2D_POINT_2F {
                    x: bl + bot_r * (1.0 - K),
                    y: bot_y,
                },
                point3: D2D_POINT_2F {
                    x: bl + bot_r,
                    y: bot_y,
                },
            });
            sink.AddLine(D2D_POINT_2F {
                x: br - bot_r,
                y: bot_y,
            });
            sink.AddBezier(&D2D1_BEZIER_SEGMENT {
                point1: D2D_POINT_2F {
                    x: br - bot_r * (1.0 - K),
                    y: bot_y,
                },
                point2: D2D_POINT_2F {
                    x: br,
                    y: bot_y - bot_r * (1.0 - K),
                },
                point3: D2D_POINT_2F {
                    x: br,
                    y: bot_y - bot_r,
                },
            });
            sink.AddLine(D2D_POINT_2F {
                x: br,
                y: top_y + ear_r,
            });
            sink.AddBezier(&D2D1_BEZIER_SEGMENT {
                point1: D2D_POINT_2F {
                    x: br,
                    y: top_y + ear_r * (1.0 - K),
                },
                point2: D2D_POINT_2F {
                    x: br + ear_r * (1.0 - K),
                    y: top_y,
                },
                point3: D2D_POINT_2F {
                    x: br + ear_r,
                    y: top_y,
                },
            });
            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
            let _ = sink.Close();
            set_col(brush, 0.01, 0.01, 0.015, pa);
            rt.FillGeometry(&geo, brush, None);

            let eave_a = preview_eave_t.clamp(0.0, 1.0) * pa;
            if eave_a > 0.01 {
                let flow = preview_phase;
                let stop1 = (0.34 + 0.11 * (flow * 0.52).sin()).clamp(0.18, 0.52);
                let stop2 = (0.68 + 0.11 * (flow * 0.41 + 1.2).cos()).clamp(0.54, 0.86);
                let aurora_a = 0.88 * eave_a;
                let h_stops = [
                    D2D1_GRADIENT_STOP {
                        position: 0.0,
                        color: D2D1_COLOR_F { r: 0.00, g: 0.75, b: 0.93, a: aurora_a },
                    },
                    D2D1_GRADIENT_STOP {
                        position: stop1,
                        color: D2D1_COLOR_F { r: 0.07, g: 0.56, b: 0.99, a: aurora_a },
                    },
                    D2D1_GRADIENT_STOP {
                        position: stop2,
                        color: D2D1_COLOR_F { r: 0.21, g: 0.37, b: 1.00, a: aurora_a },
                    },
                    D2D1_GRADIENT_STOP {
                        position: 1.0,
                        color: D2D1_COLOR_F { r: 0.45, g: 0.28, b: 0.98, a: aurora_a },
                    },
                ];
                let geo_mask: Option<ID2D1Geometry> = geo.cast().ok();
                if let (Ok(layer), Some(mask)) = (rt.CreateLayer(None), geo_mask) {
                    let mut lp = D2D1_LAYER_PARAMETERS {
                        contentBounds: D2D_RECT_F {
                            left: bl - ear_r,
                            top: top_y,
                            right: br + ear_r,
                            bottom: bot_y + 1.0,
                        },
                        geometricMask: ManuallyDrop::new(Some(mask)),
                        maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                        maskTransform: Matrix3x2::identity(),
                        opacity: 1.0,
                        opacityBrush: ManuallyDrop::new(None),
                        layerOptions: D2D1_LAYER_OPTIONS_NONE,
                    };
                    rt.PushLayer(&lp, &layer);
                    if let Ok(sc) = rt.CreateGradientStopCollection(
                        &h_stops,
                        D2D1_GAMMA_2_2,
                        D2D1_EXTEND_MODE_CLAMP,
                    ) {
                        let hp = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                            startPoint: D2D_POINT_2F { x: bl, y: bot_y },
                            endPoint: D2D_POINT_2F { x: br, y: bot_y },
                        };
                        if let Ok(hb) = rt.CreateLinearGradientBrush(&hp, None, &sc) {
                            rt.FillRectangle(
                                &D2D_RECT_F {
                                    left: bl - ear_r,
                                    top: top_y,
                                    right: br + ear_r,
                                    bottom: bot_y,
                                },
                                &hb,
                            );
                        }
                    }

                    let draw_blob = |bx: f32, by: f32, br_rad: f32, col: (f32, f32, f32), ba: f32| {
                        let bst = [
                            D2D1_GRADIENT_STOP {
                                position: 0.0,
                                color: D2D1_COLOR_F { r: col.0, g: col.1, b: col.2, a: ba },
                            },
                            D2D1_GRADIENT_STOP {
                                position: 1.0,
                                color: D2D1_COLOR_F { r: col.0, g: col.1, b: col.2, a: 0.0 },
                            },
                        ];
                        if let Ok(sc) = rt.CreateGradientStopCollection(
                            &bst,
                            D2D1_GAMMA_2_2,
                            D2D1_EXTEND_MODE_CLAMP,
                        ) {
                            let rp = D2D1_RADIAL_GRADIENT_BRUSH_PROPERTIES {
                                center: D2D_POINT_2F { x: bx, y: by },
                                gradientOriginOffset: D2D_POINT_2F { x: 0.0, y: 0.0 },
                                radiusX: br_rad,
                                radiusY: br_rad * 0.72,
                            };
                            if let Ok(rb) = rt.CreateRadialGradientBrush(&rp, None, &sc) {
                                rt.FillEllipse(
                                    &D2D1_ELLIPSE {
                                        point: D2D_POINT_2F { x: bx, y: by },
                                        radiusX: br_rad,
                                        radiusY: br_rad * 0.72,
                                    },
                                    &rb,
                                );
                            }
                        }
                    };
                    let ba = 0.46 * eave_a;
                    draw_blob(
                        bl + body_w * (0.24 + 0.20 * (flow * 0.55).sin()),
                        top_y + ph * 0.88,
                        body_w * 0.36,
                        (0.00, 0.88, 0.98),
                        ba,
                    );
                    draw_blob(
                        bl + body_w * (0.52 + 0.22 * (flow * 0.42 + 2.0).cos()),
                        top_y + ph * 0.90,
                        body_w * 0.38,
                        (0.12, 0.54, 1.00),
                        ba * 0.85,
                    );
                    draw_blob(
                        bl + body_w * (0.78 - 0.20 * (flow * 0.48 + 0.9).sin()),
                        top_y + ph * 0.86,
                        body_w * 0.36,
                        (0.58, 0.28, 1.00),
                        ba,
                    );

                    let v_stops = [
                        D2D1_GRADIENT_STOP {
                            position: 0.0,
                            color: D2D1_COLOR_F { r: 0.01, g: 0.01, b: 0.015, a: 1.0 },
                        },
                        D2D1_GRADIENT_STOP {
                            position: 0.26,
                            color: D2D1_COLOR_F { r: 0.01, g: 0.01, b: 0.015, a: 0.92 },
                        },
                        D2D1_GRADIENT_STOP {
                            position: 0.72,
                            color: D2D1_COLOR_F { r: 0.01, g: 0.01, b: 0.015, a: 0.34 },
                        },
                        D2D1_GRADIENT_STOP {
                            position: 1.0,
                            color: D2D1_COLOR_F { r: 0.01, g: 0.01, b: 0.015, a: 0.0 },
                        },
                    ];
                    if let Ok(sc) = rt.CreateGradientStopCollection(
                        &v_stops,
                        D2D1_GAMMA_2_2,
                        D2D1_EXTEND_MODE_CLAMP,
                    ) {
                        let vp = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                            startPoint: D2D_POINT_2F { x: cx, y: top_y },
                            endPoint: D2D_POINT_2F { x: cx, y: bot_y },
                        };
                        if let Ok(vb) = rt.CreateLinearGradientBrush(&vp, None, &sc) {
                            rt.FillRectangle(
                                &D2D_RECT_F {
                                    left: bl - ear_r,
                                    top: top_y,
                                    right: br + ear_r,
                                    bottom: bot_y,
                                },
                                &vb,
                            );
                        }
                    }

                    let esc = (ph / 32.0).clamp(0.60, 1.05);
                    let look_x = ((flow * 0.75).sin() * 2.4 + (flow * 0.31).cos() * 1.1) * esc;
                    let blink_cyc = flow % 5.8;
                    let blink = if blink_cyc < 0.14 {
                        ((blink_cyc / 0.14) * std::f32::consts::PI).sin()
                    } else {
                        0.0
                    };
                    let eye_sep = 15.5 * esc;
                    let ex_rx = 3.5 * esc;
                    let ex_ry = (1.0 + 4.9 * (1.0 - blink)) * esc;
                    let ey = top_y + ph * 0.48;
                    for side in [-0.5_f32, 0.5_f32] {
                        let ex = cx + look_x + side * eye_sep;
                        draw_blob(ex, ey, 11.5 * esc, (0.56, 0.80, 1.0), 0.045 * eave_a);
                        set_col(brush, 0.94, 0.94, 0.91, 0.96 * eave_a);
                        rt.FillEllipse(
                            &D2D1_ELLIPSE {
                                point: D2D_POINT_2F { x: ex, y: ey },
                                radiusX: ex_rx,
                                radiusY: ex_ry,
                            },
                            brush,
                        );
                    }

                    rt.PopLayer();
                    ManuallyDrop::drop(&mut lp.geometricMask);
                    ManuallyDrop::drop(&mut lp.opacityBrush);
                }
            }
        }
    }

    let vis = (1.0 - min_t).clamp(0.0, 1.0) * (1.0 - preview_eave_t * 1.35).clamp(0.0, 1.0) * pa;
    if vis > 0.02 {
        if clock_t > 0.02 {
            let now_str = format_local_clock_short();
            set_col(brush, 1.0, 1.0, 1.0, clock_t * vis);
            draw_text(
                rt,
                &now_str,
                &gfx.fmt_preview,
                &D2D_RECT_F {
                    left: bl + 14.0,
                    top: y + 1.0,
                    right: br - 14.0,
                    bottom: y + ph,
                },
                brush,
            );
        }
        set_col(brush, 0.24, 0.88, 0.52, vis);
        rt.DrawEllipse(
            &D2D1_ELLIPSE {
                point: D2D_POINT_2F {
                    x: br - 14.0,
                    y: y + ph * 0.52,
                },
                radiusX: 4.1,
                radiusY: 4.1,
            },
            brush,
            1.6,
            None,
        );
    }

    set_col(brush, 1.0, 1.0, 1.0, 0.88 * pa);
    draw_text(
        rt,
        "Live preview of your notch",
        &gfx.fmt_preview,
        &D2D_RECT_F {
            left: l,
            top: y + h - 22.0,
            right: r,
            bottom: y + h - 4.0,
        },
        brush,
    );

    set_col(brush, 1.0, 1.0, 1.0, 0.14 * pa);
    rt.DrawRoundedRectangle(&rrect, brush, 0.9, None);

    y + h + 14.0
}

unsafe fn draw_mini_chevron(
    rt: &ID2D1RenderTarget,
    brush: &ID2D1SolidColorBrush,
    cx: f32,
    cy: f32,
    up: bool,
) {
    let dy = if up { -2.2 } else { 2.2 };
    let tip = D2D_POINT_2F { x: cx, y: cy + dy };
    let left = D2D_POINT_2F {
        x: cx - 3.6,
        y: cy - dy,
    };
    let right = D2D_POINT_2F {
        x: cx + 3.6,
        y: cy - dy,
    };
    rt.DrawLine(left, tip, brush, 1.45, None);
    rt.DrawLine(tip, right, brush, 1.45, None);
}

#[allow(clippy::too_many_arguments)]
unsafe fn draw_ring_choice_card<F>(
    rt: &ID2D1RenderTarget,
    brush: &ID2D1SolidColorBrush,
    gfx: &GfxResources,
    l: f32,
    r: f32,
    y: f32,
    sel_anims: [f32; 4],
    pa: f32,
    hov_t: &F,
    hits: &mut Vec<HitBox>,
) -> f32
where
    F: Fn(&str, D2D_RECT_F) -> f32,
{
    let h = 118.0;
    const CHOICES: [(u8, &str, &str, (f32, f32, f32)); 4] = [
        (3, "Auto", "Active status", (0.22, 0.86, 0.44)),
        (1, "Water", "Today's water", (0.20, 0.72, 1.0)),
        (2, "To-do", "Tasks done", (0.22, 0.86, 0.44)),
        (0, "Off", "No ring", (0.50, 0.50, 0.54)),
    ];
    draw_group_card(rt, brush, l, r, y, h, pa, |rt, b, top| {
        let pad = 10.0;
        let gap = 8.0;
        let cw = ((r - l) - pad * 2.0 - gap * 3.0) / 4.0;
        let ch = h - pad * 2.0;
        for (i, &(mode, title, sub, col)) in CHOICES.iter().enumerate() {
            let cx_l = l + pad + i as f32 * (cw + gap);
            let card_r = D2D_RECT_F {
                left: cx_l,
                top: top + pad,
                right: cx_l + cw,
                bottom: top + pad + ch,
            };
            let sel_t = sel_anims[i].clamp(0.0, 1.0);
            let h_key = format!("ring_card_{i}");
            let h_t = hov_t(&h_key, card_r);
            let rr = D2D1_ROUNDED_RECT {
                rect: card_r,
                radiusX: 8.5,
                radiusY: 8.5,
            };
            let bg_r = 0.16 + 0.02 * h_t * (1.0 - sel_t) + 0.03 * sel_t;
            let bg_g = 0.16 + 0.02 * h_t * (1.0 - sel_t) + 0.04 * sel_t;
            let bg_b = 0.18 + 0.02 * h_t * (1.0 - sel_t) + 0.05 * sel_t;
            set_col(b, bg_r, bg_g, bg_b, pa);
            rt.FillRoundedRectangle(&rr, b);
            if sel_t < 0.99 {
                set_col(b, 1.0, 1.0, 1.0, (0.06 + 0.08 * h_t) * (1.0 - sel_t) * pa);
                rt.DrawRoundedRectangle(&rr, b, 0.85, None);
            }
            if sel_t > 0.01 {
                set_col(b, 0.08, 0.54, 1.0, sel_t * pa);
                rt.DrawRoundedRectangle(&rr, b, 0.85 + 0.75 * sel_t, None);
            }

            let pv_r = D2D_RECT_F {
                left: card_r.left + 7.0,
                top: card_r.top + 7.0,
                right: card_r.right - 7.0,
                bottom: card_r.top + 48.0,
            };
            set_col(b, 0.18, 0.20, 0.28, pa);
            rt.FillRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: pv_r,
                    radiusX: 6.0,
                    radiusY: 6.0,
                },
                b,
            );

            let pill_w = (pv_r.right - pv_r.left - 8.0).max(46.0);
            let pmx = (pv_r.left + pv_r.right) * 0.5;
            let pill = D2D_RECT_F {
                left: pmx - pill_w * 0.5,
                top: pv_r.top + 3.0,
                right: pmx + pill_w * 0.5,
                bottom: pv_r.top + 18.0,
            };
            set_col(b, 0.02, 0.02, 0.03, pa);
            rt.FillRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: pill,
                    radiusX: 7.5,
                    radiusY: 7.5,
                },
                b,
            );

            if mode != 0 {
                let rx = pill.left + 9.0;
                let ry = (pill.top + pill.bottom) * 0.5;
                set_col(b, col.0, col.1, col.2, pa);
                rt.DrawEllipse(
                    &D2D1_ELLIPSE {
                        point: D2D_POINT_2F { x: rx, y: ry },
                        radiusX: 3.6,
                        radiusY: 3.6,
                    },
                    b,
                    1.5,
                    None,
                );
                if mode == 1 {
                    set_col(b, 0.88, 0.96, 1.0, pa);
                    rt.FillEllipse(
                        &D2D1_ELLIPSE {
                            point: D2D_POINT_2F { x: rx, y: ry },
                            radiusX: 1.2,
                            radiusY: 1.4,
                        },
                        b,
                    );
                } else if mode == 2 {
                    rt.DrawLine(
                        D2D_POINT_2F { x: rx - 1.5, y: ry + 0.2 },
                        D2D_POINT_2F { x: rx - 0.2, y: ry + 1.5 },
                        b,
                        1.1,
                        None,
                    );
                    rt.DrawLine(
                        D2D_POINT_2F { x: rx - 0.2, y: ry + 1.5 },
                        D2D_POINT_2F { x: rx + 1.8, y: ry - 1.2 },
                        b,
                        1.1,
                        None,
                    );
                }
            }

            set_col(b, 0.96, 0.96, 0.98, pa);
            draw_text(
                rt,
                "10:08",
                &gfx.fmt_preview,
                &D2D_RECT_F {
                    left: pill.left + if mode != 0 { 13.0 } else { 2.0 },
                    top: pill.top,
                    right: pill.right - 2.0,
                    bottom: pill.bottom,
                },
                b,
            );

            set_col(b, 0.97, 0.97, 0.98, pa);
            draw_text(
                rt,
                title,
                &gfx.fmt_sidebar,
                &D2D_RECT_F {
                    left: card_r.left + 8.0,
                    top: card_r.top + 54.0,
                    right: card_r.right - 22.0,
                    bottom: card_r.top + 72.0,
                },
                b,
            );
            if sel_t > 0.01 {
                let ck_x = card_r.right - 13.0;
                let ck_y = card_r.top + 63.0;
                let sc = 0.65 + 0.35 * sel_t;
                set_col(b, 0.08, 0.54, 1.0, sel_t * pa);
                rt.FillEllipse(
                    &D2D1_ELLIPSE {
                        point: D2D_POINT_2F { x: ck_x, y: ck_y },
                        radiusX: 5.8 * sc,
                        radiusY: 5.8 * sc,
                    },
                    b,
                );
                set_col(b, 1.0, 1.0, 1.0, sel_t * pa);
                rt.DrawLine(
                    D2D_POINT_2F { x: ck_x - 2.4 * sc, y: ck_y + 0.2 * sc },
                    D2D_POINT_2F { x: ck_x - 0.6 * sc, y: ck_y + 2.0 * sc },
                    b,
                    1.4,
                    None,
                );
                rt.DrawLine(
                    D2D_POINT_2F { x: ck_x - 0.6 * sc, y: ck_y + 2.0 * sc },
                    D2D_POINT_2F { x: ck_x + 2.6 * sc, y: ck_y - 1.8 * sc },
                    b,
                    1.4,
                    None,
                );
            }

            set_col(b, 0.56, 0.56, 0.58, pa);
            draw_text(
                rt,
                sub,
                &gfx.fmt_sub,
                &D2D_RECT_F {
                    left: card_r.left + 8.0,
                    top: card_r.top + 73.0,
                    right: card_r.right - 6.0,
                    bottom: card_r.top + 91.0,
                },
                b,
            );

            hits.push(HitBox {
                rect: card_r,
                hit: ControlHit::RingMode(mode),
            });
        }
    })
}

#[allow(clippy::too_many_arguments)]
unsafe fn draw_widget_picker_card<F>(
    rt: &ID2D1RenderTarget,
    brush: &ID2D1SolidColorBrush,
    gfx: &GfxResources,
    l: f32,
    r: f32,
    y: f32,
    slot_f: f32,
    wg_left: u8,
    wg_right: u8,
    wg_sels: [f32; 8],
    pa: f32,
    hov_t: &F,
    hits: &mut Vec<HitBox>,
) -> f32
where
    F: Fn(&str, D2D_RECT_F) -> f32,
{
    let h = 238.0;
    draw_group_card(rt, brush, l, r, y, h, pa, |rt, b, top| {
        let pad = 12.0;
        let pill_w = 58.0;
        let pill_h = 24.0;
        let pill_gap = 6.0;
        let p_left = D2D_RECT_F {
            left: l + pad,
            top: top + 10.0,
            right: l + pad + pill_w,
            bottom: top + 10.0 + pill_h,
        };
        let p_right = D2D_RECT_F {
            left: p_left.right + pill_gap,
            top: p_left.top,
            right: p_left.right + pill_gap + pill_w,
            bottom: p_left.bottom,
        };
        for (i, pr) in [p_left, p_right].iter().enumerate() {
            let hk = format!("wg_slot_{i}");
            let ht = hov_t(&hk, *pr);
            set_col(b, 1.0, 1.0, 1.0, (0.08 + 0.06 * ht) * pa);
            rt.FillRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: *pr,
                    radiusX: 12.0,
                    radiusY: 12.0,
                },
                b,
            );
        }
        let thumb_x = p_left.left + (p_right.left - p_left.left) * slot_f;
        set_col(b, 0.08, 0.50, 0.96, 0.92 * pa);
        rt.FillRoundedRectangle(
            &D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F {
                    left: thumb_x,
                    top: p_left.top,
                    right: thumb_x + pill_w,
                    bottom: p_left.bottom,
                },
                radiusX: 12.0,
                radiusY: 12.0,
            },
            b,
        );
        let l_lum = 0.72 + 0.28 * (1.0 - slot_f);
        let r_lum = 0.72 + 0.28 * slot_f;
        set_col(b, l_lum, l_lum, l_lum, pa);
        draw_text(rt, "Left", &gfx.fmt_seg_on, &p_left, b);
        set_col(b, r_lum, r_lum, r_lum, pa);
        draw_text(rt, "Right", &gfx.fmt_seg_on, &p_right, b);
        hits.push(HitBox {
            rect: p_left,
            hit: ControlHit::WidgetSlot(0),
        });
        hits.push(HitBox {
            rect: p_right,
            hit: ControlHit::WidgetSlot(1),
        });

        let gap = 8.0;
        let cw = ((r - l) - pad * 2.0 - gap * 3.0) / 4.0;
        let ch = 88.0;
        let grid_top = top + 44.0;
        for (i, &kind) in WidgetKind::ALL.iter().enumerate() {
            let col = (i % 4) as f32;
            let row = (i / 4) as f32;
            let cx_l = l + pad + col * (cw + gap);
            let cy_t = grid_top + row * (ch + gap);
            let card_r = D2D_RECT_F {
                left: cx_l,
                top: cy_t,
                right: cx_l + cw,
                bottom: cy_t + ch,
            };
            let sel_t = wg_sels[i].clamp(0.0, 1.0);
            let hk = format!("wg_card_{i}");
            let ht = hov_t(&hk, card_r);
            let rr = D2D1_ROUNDED_RECT {
                rect: card_r,
                radiusX: 8.5,
                radiusY: 8.5,
            };
            let bg_r = 0.16 + 0.02 * ht * (1.0 - sel_t) + 0.03 * sel_t;
            let bg_g = 0.16 + 0.02 * ht * (1.0 - sel_t) + 0.04 * sel_t;
            let bg_b = 0.18 + 0.02 * ht * (1.0 - sel_t) + 0.05 * sel_t;
            set_col(b, bg_r, bg_g, bg_b, pa);
            rt.FillRoundedRectangle(&rr, b);
            if sel_t < 0.99 {
                set_col(b, 1.0, 1.0, 1.0, (0.06 + 0.08 * ht) * (1.0 - sel_t) * pa);
                rt.DrawRoundedRectangle(&rr, b, 0.85, None);
            }
            if sel_t > 0.01 {
                set_col(b, 0.08, 0.54, 1.0, sel_t * pa);
                rt.DrawRoundedRectangle(&rr, b, 0.85 + 0.75 * sel_t, None);
            }

            let pv = D2D_RECT_F {
                left: card_r.left + 6.0,
                top: card_r.top + 6.0,
                right: card_r.right - 6.0,
                bottom: card_r.top + 56.0,
            };
            let pv_rr = D2D1_ROUNDED_RECT {
                rect: pv,
                radiusX: 7.5,
                radiusY: 7.5,
            };
            let pv_accent: (f32, f32, f32) = match kind {
                WidgetKind::ThisWeek => (0.04, 0.52, 1.0),
                WidgetKind::Battery => (0.19, 0.82, 0.35),
                WidgetKind::Water => (0.12, 0.64, 1.0),
                WidgetKind::FocusToday => (0.62, 0.45, 1.0),
                WidgetKind::WorldClock => (0.24, 0.72, 0.98),
                WidgetKind::Moon => (0.76, 0.78, 0.96),
                WidgetKind::Countdown => (1.0, 0.64, 0.14),
                WidgetKind::Weather => (0.20, 0.54, 0.92),
            };
            if kind == WidgetKind::Weather {
                let stops = [
                    D2D1_GRADIENT_STOP {
                        position: 0.0,
                        color: D2D1_COLOR_F { r: 0.09, g: 0.30, b: 0.60, a: pa },
                    },
                    D2D1_GRADIENT_STOP {
                        position: 1.0,
                        color: D2D1_COLOR_F { r: 0.22, g: 0.54, b: 0.85, a: pa },
                    },
                ];
                if let Ok(sc) = rt.CreateGradientStopCollection(&stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP) {
                    let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                        startPoint: D2D_POINT_2F { x: pv.left, y: pv.top },
                        endPoint: D2D_POINT_2F { x: pv.left, y: pv.bottom },
                    };
                    if let Ok(lin) = rt.CreateLinearGradientBrush(&props, None, &sc) {
                        rt.FillRoundedRectangle(&pv_rr, &lin);
                    }
                }
            } else {
                let stops = [
                    D2D1_GRADIENT_STOP {
                        position: 0.0,
                        color: D2D1_COLOR_F {
                            r: 0.09 + pv_accent.0 * 0.09,
                            g: 0.09 + pv_accent.1 * 0.09,
                            b: 0.11 + pv_accent.2 * 0.09,
                            a: pa,
                        },
                    },
                    D2D1_GRADIENT_STOP {
                        position: 1.0,
                        color: D2D1_COLOR_F { r: 0.06, g: 0.06, b: 0.08, a: pa },
                    },
                ];
                if let Ok(sc) = rt.CreateGradientStopCollection(&stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP) {
                    let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                        startPoint: D2D_POINT_2F { x: pv.left, y: pv.top },
                        endPoint: D2D_POINT_2F { x: pv.right, y: pv.bottom },
                    };
                    if let Ok(lin) = rt.CreateLinearGradientBrush(&props, None, &sc) {
                        rt.FillRoundedRectangle(&pv_rr, &lin);
                    }
                }
            }
            set_col(b, 1.0, 1.0, 1.0, 0.12 * pa);
            rt.DrawRoundedRectangle(&pv_rr, b, 0.75, None);

            let pw = pv.right - pv.left;
            let pmx = (pv.left + pv.right) * 0.5;
            let pmy = (pv.top + pv.bottom) * 0.5;
            let pill = |rt: &ID2D1RenderTarget, b: &ID2D1SolidColorBrush, l: f32, t: f32, r: f32, bt: f32| {
                let rad = ((bt - t) * 0.5).max(1.0);
                rt.FillRoundedRectangle(
                    &D2D1_ROUNDED_RECT {
                        rect: D2D_RECT_F { left: l, top: t, right: r, bottom: bt },
                        radiusX: rad,
                        radiusY: rad,
                    },
                    b,
                );
            };
            match kind {
                WidgetKind::ThisWeek => {
                    set_col(b, 0.65, 0.68, 0.74, 0.65 * pa);
                    pill(rt, b, pv.left + 6.0, pv.top + 5.5, pv.left + 26.0, pv.top + 8.0);
                    set_col(b, 0.04, 0.52, 1.0, 0.35 * pa);
                    pill(rt, b, pv.right - 18.0, pv.top + 4.5, pv.right - 5.5, pv.top + 9.5);
                    for d in 0..7 {
                        let dx = pv.left + 8.0 + d as f32 * ((pw - 16.0) / 6.0);
                        if d == 4 {
                            set_col(b, 0.04, 0.52, 1.0, 0.28 * pa);
                            rt.FillEllipse(
                                &D2D1_ELLIPSE {
                                    point: D2D_POINT_2F { x: dx, y: pv.top + 21.5 },
                                    radiusX: 6.2,
                                    radiusY: 6.2,
                                },
                                b,
                            );
                            set_col(b, 0.04, 0.52, 1.0, pa);
                            rt.FillEllipse(
                                &D2D1_ELLIPSE {
                                    point: D2D_POINT_2F { x: dx, y: pv.top + 21.5 },
                                    radiusX: 4.3,
                                    radiusY: 4.3,
                                },
                                b,
                            );
                            set_col(b, 0.22, 0.88, 0.42, pa);
                            rt.FillEllipse(
                                &D2D1_ELLIPSE {
                                    point: D2D_POINT_2F { x: dx, y: pv.top + 27.2 },
                                    radiusX: 1.2,
                                    radiusY: 1.2,
                                },
                                b,
                            );
                        } else {
                            set_col(b, 0.78, 0.80, 0.86, 0.65 * pa);
                            rt.FillEllipse(
                                &D2D1_ELLIPSE {
                                    point: D2D_POINT_2F { x: dx, y: pv.top + 21.5 },
                                    radiusX: 1.7,
                                    radiusY: 1.7,
                                },
                                b,
                            );
                        }
                    }
                    set_col(b, 1.0, 1.0, 1.0, 0.08 * pa);
                    rt.DrawLine(
                        D2D_POINT_2F { x: pv.left + 6.0, y: pv.bottom - 16.5 },
                        D2D_POINT_2F { x: pv.right - 6.0, y: pv.bottom - 16.5 },
                        b,
                        0.7,
                        None,
                    );
                    set_col(b, 0.22, 0.88, 0.42, pa);
                    rt.FillEllipse(
                        &D2D1_ELLIPSE {
                            point: D2D_POINT_2F { x: pv.left + 9.0, y: pv.bottom - 9.0 },
                            radiusX: 2.2,
                            radiusY: 2.2,
                        },
                        b,
                    );
                    set_col(b, 0.92, 0.94, 0.98, 0.90 * pa);
                    pill(rt, b, pv.left + 15.0, pv.bottom - 11.2, pv.right - 16.0, pv.bottom - 8.2);
                    set_col(b, 0.56, 0.60, 0.68, 0.75 * pa);
                    pill(rt, b, pv.left + 15.0, pv.bottom - 6.2, pv.right - 26.0, pv.bottom - 4.0);
                }
                WidgetKind::Battery => {
                    set_col(b, 0.65, 0.68, 0.74, 0.65 * pa);
                    pill(rt, b, pv.left + 6.0, pv.top + 5.5, pv.left + 26.0, pv.top + 8.0);
                    set_col(b, 1.0, 1.0, 1.0, 0.85 * pa);
                    pill(rt, b, pv.right - 15.0, pv.top + 5.5, pv.right - 9.5, pv.top + 8.0);
                    set_col(b, 1.0, 1.0, 1.0, 0.30 * pa);
                    rt.FillEllipse(
                        &D2D1_ELLIPSE {
                            point: D2D_POINT_2F { x: pv.right - 6.5, y: pv.top + 6.75 },
                            radiusX: 1.3,
                            radiusY: 1.3,
                        },
                        b,
                    );
                    let bx = pv.left + 21.0;
                    let by = pmy + 3.5;
                    set_col(b, 0.19, 0.82, 0.35, 0.16 * pa);
                    rt.FillEllipse(
                        &D2D1_ELLIPSE {
                            point: D2D_POINT_2F { x: bx, y: by },
                            radiusX: 14.0,
                            radiusY: 14.0,
                        },
                        b,
                    );
                    set_col(b, 0.19, 0.82, 0.35, 0.25 * pa);
                    rt.DrawEllipse(
                        &D2D1_ELLIPSE {
                            point: D2D_POINT_2F { x: bx, y: by },
                            radiusX: 10.5,
                            radiusY: 10.5,
                        },
                        b,
                        2.8,
                        None,
                    );
                    set_col(b, 0.19, 0.82, 0.35, pa);
                    rt.DrawEllipse(
                        &D2D1_ELLIPSE {
                            point: D2D_POINT_2F { x: bx, y: by },
                            radiusX: 10.5,
                            radiusY: 10.5,
                        },
                        b,
                        2.3,
                        None,
                    );
                    set_col(b, 1.0, 1.0, 1.0, 0.95 * pa);
                    rt.DrawLine(D2D_POINT_2F { x: bx + 0.8, y: by - 4.2 }, D2D_POINT_2F { x: bx - 2.0, y: by + 0.4 }, b, 1.5, None);
                    rt.DrawLine(D2D_POINT_2F { x: bx - 2.0, y: by + 0.4 }, D2D_POINT_2F { x: bx + 2.0, y: by + 0.4 }, b, 1.5, None);
                    rt.DrawLine(D2D_POINT_2F { x: bx + 2.0, y: by + 0.4 }, D2D_POINT_2F { x: bx - 0.8, y: by + 4.2 }, b, 1.5, None);

                    set_col(b, 0.96, 0.97, 1.0, pa);
                    pill(rt, b, bx + 16.0, by - 9.0, pv.right - 14.0, by - 3.5);
                    set_col(b, 0.19, 0.82, 0.35, 0.95 * pa);
                    pill(rt, b, bx + 16.0, by - 0.5, pv.right - 10.0, by + 2.5);
                    set_col(b, 0.56, 0.60, 0.68, 0.75 * pa);
                    pill(rt, b, bx + 16.0, by + 5.2, pv.right - 18.0, by + 7.6);
                }
                WidgetKind::Water => {
                    set_col(b, 0.65, 0.68, 0.74, 0.65 * pa);
                    pill(rt, b, pv.left + 6.0, pv.top + 5.5, pv.left + 22.0, pv.top + 8.0);
                    set_col(b, 0.12, 0.64, 1.0, 0.34 * pa);
                    pill(rt, b, pv.right - 20.0, pv.top + 4.5, pv.right - 5.5, pv.top + 9.5);
                    let cup = D2D_RECT_F {
                        left: pv.left + 7.5,
                        top: pv.top + 13.0,
                        right: pv.left + 24.5,
                        bottom: pv.bottom - 5.5,
                    };
                    set_col(b, 1.0, 1.0, 1.0, 0.08 * pa);
                    rt.FillRoundedRectangle(
                        &D2D1_ROUNDED_RECT { rect: cup, radiusX: 4.5, radiusY: 4.5 },
                        b,
                    );
                    let liq = D2D_RECT_F {
                        left: cup.left + 1.5,
                        top: cup.top + 11.5,
                        right: cup.right - 1.5,
                        bottom: cup.bottom - 1.5,
                    };
                    set_col(b, 0.12, 0.66, 1.0, 0.95 * pa);
                    rt.FillRoundedRectangle(
                        &D2D1_ROUNDED_RECT { rect: liq, radiusX: 3.2, radiusY: 3.2 },
                        b,
                    );
                    set_col(b, 0.36, 0.86, 1.0, 0.85 * pa);
                    pill(rt, b, liq.left + 1.0, liq.top, liq.right - 1.0, liq.top + 2.5);
                    set_col(b, 1.0, 1.0, 1.0, 0.22 * pa);
                    rt.DrawRoundedRectangle(
                        &D2D1_ROUNDED_RECT { rect: cup, radiusX: 4.5, radiusY: 4.5 },
                        b,
                        0.85,
                        None,
                    );
                    set_col(b, 0.96, 0.97, 1.0, pa);
                    pill(rt, b, cup.right + 7.0, pv.top + 15.0, pv.right - 14.0, pv.top + 20.5);
                    set_col(b, 0.58, 0.66, 0.76, 0.80 * pa);
                    pill(rt, b, cup.right + 7.0, pv.top + 24.0, pv.right - 10.0, pv.top + 26.8);
                    for d in 0..6 {
                        let px = cup.right + 7.0 + d as f32 * 6.0;
                        set_col(b, 0.14, 0.68, 1.0, if d < 4 { pa } else { 0.22 * pa });
                        pill(rt, b, px, pv.bottom - 10.5, px + 4.0, pv.bottom - 6.8);
                    }
                }
                WidgetKind::FocusToday => {
                    set_col(b, 0.65, 0.68, 0.74, 0.65 * pa);
                    pill(rt, b, pv.left + 6.0, pv.top + 5.5, pv.left + 30.0, pv.top + 8.0);
                    set_col(b, 0.60, 0.42, 1.0, 0.36 * pa);
                    pill(rt, b, pv.right - 22.0, pv.top + 4.5, pv.right - 5.5, pv.top + 9.5);
                    set_col(b, 0.96, 0.97, 1.0, pa);
                    pill(rt, b, pv.left + 6.0, pv.top + 14.5, pv.left + 34.0, pv.top + 20.5);
                    set_col(b, 0.68, 0.54, 1.0, 0.92 * pa);
                    pill(rt, b, pv.left + 6.0, pv.top + 25.0, pv.left + 32.0, pv.top + 28.0);
                    set_col(b, 0.54, 0.58, 0.66, 0.75 * pa);
                    pill(rt, b, pv.left + 6.0, pv.top + 31.5, pv.left + 26.0, pv.top + 34.0);
                    for k in 0..6 {
                        let bh = [7.0, 12.0, 8.0, 15.0, 10.0, 19.0][k];
                        let bx = pv.right - 34.0 + k as f32 * 4.8;
                        if k == 5 {
                            set_col(b, 0.62, 0.44, 1.0, 0.32 * pa);
                            pill(rt, b, bx - 1.2, pv.bottom - 6.0 - bh - 1.2, bx + 4.2, pv.bottom - 5.0);
                            set_col(b, 0.68, 0.50, 1.0, pa);
                        } else {
                            set_col(b, 0.78, 0.72, 1.0, 0.24 * pa);
                        }
                        pill(rt, b, bx, pv.bottom - 6.0 - bh, bx + 3.0, pv.bottom - 6.0);
                    }
                }
                WidgetKind::WorldClock => {
                    set_col(b, 0.65, 0.68, 0.74, 0.65 * pa);
                    pill(rt, b, pv.left + 6.0, pv.top + 5.5, pv.left + 32.0, pv.top + 8.0);
                    set_col(b, 1.0, 1.0, 1.0, 0.08 * pa);
                    rt.DrawLine(
                        D2D_POINT_2F { x: pv.left + 6.0, y: pv.top + 28.5 },
                        D2D_POINT_2F { x: pv.right - 6.0, y: pv.top + 28.5 },
                        b,
                        0.7,
                        None,
                    );
                    for row in 0..2 {
                        let ry = pv.top + 18.0 + row as f32 * 19.0;
                        if row == 0 {
                            set_col(b, 0.76, 0.84, 1.0, pa);
                            rt.FillEllipse(&D2D1_ELLIPSE { point: D2D_POINT_2F { x: pv.left + 10.0, y: ry }, radiusX: 2.8, radiusY: 2.8 }, b);
                            set_col(b, 0.09, 0.11, 0.14, pa);
                            rt.FillEllipse(&D2D1_ELLIPSE { point: D2D_POINT_2F { x: pv.left + 11.4, y: ry - 0.9 }, radiusX: 2.2, radiusY: 2.2 }, b);
                        } else {
                            set_col(b, 1.0, 0.82, 0.30, pa);
                            rt.FillEllipse(&D2D1_ELLIPSE { point: D2D_POINT_2F { x: pv.left + 10.0, y: ry }, radiusX: 2.6, radiusY: 2.6 }, b);
                        }
                        set_col(b, 0.92, 0.94, 0.98, 0.90 * pa);
                        pill(rt, b, pv.left + 16.0, ry - 3.2, pmx + 2.0, ry - 0.4);
                        set_col(b, 0.56, 0.60, 0.68, 0.75 * pa);
                        pill(rt, b, pv.left + 16.0, ry + 1.4, pmx - 4.0, ry + 3.6);
                        set_col(b, 0.98, 0.98, 1.0, pa);
                        pill(rt, b, pv.right - 22.0, ry - 2.2, pv.right - 6.0, ry + 2.0);
                    }
                }
                WidgetKind::Moon => {
                    set_col(b, 0.65, 0.68, 0.74, 0.65 * pa);
                    pill(rt, b, pv.left + 6.0, pv.top + 5.5, pv.left + 20.0, pv.top + 8.0);
                    set_col(b, 0.90, 0.94, 1.0, 0.55 * pa);
                    rt.FillEllipse(&D2D1_ELLIPSE { point: D2D_POINT_2F { x: pv.right - 10.0, y: pv.top + 7.5 }, radiusX: 0.9, radiusY: 0.9 }, b);
                    rt.FillEllipse(&D2D1_ELLIPSE { point: D2D_POINT_2F { x: pv.right - 16.0, y: pv.top + 10.5 }, radiusX: 0.7, radiusY: 0.7 }, b);
                    let mx = pv.left + 20.5;
                    let my = pmy + 3.5;
                    set_col(b, 0.84, 0.88, 1.0, 0.14 * pa);
                    rt.FillEllipse(&D2D1_ELLIPSE { point: D2D_POINT_2F { x: mx, y: my }, radiusX: 13.5, radiusY: 13.5 }, b);
                    set_col(b, 0.15, 0.16, 0.22, pa);
                    rt.FillEllipse(&D2D1_ELLIPSE { point: D2D_POINT_2F { x: mx, y: my }, radiusX: 10.0, radiusY: 10.0 }, b);
                    let clip_r = D2D_RECT_F { left: mx, top: my - 10.5, right: mx + 10.5, bottom: my + 10.5 };
                    rt.PushAxisAlignedClip(
                        &clip_r,
                        windows::Win32::Graphics::Direct2D::D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                    );
                    set_col(b, 0.95, 0.93, 0.86, pa);
                    rt.FillEllipse(&D2D1_ELLIPSE { point: D2D_POINT_2F { x: mx, y: my }, radiusX: 10.0, radiusY: 10.0 }, b);
                    rt.PopAxisAlignedClip();
                    set_col(b, 0.95, 0.93, 0.86, pa);
                    rt.FillEllipse(&D2D1_ELLIPSE { point: D2D_POINT_2F { x: mx, y: my }, radiusX: 5.5, radiusY: 10.0 }, b);
                    set_col(b, 0.96, 0.97, 1.0, pa);
                    pill(rt, b, mx + 15.0, my - 7.5, pv.right - 7.0, my - 4.2);
                    set_col(b, 0.94, 0.90, 0.76, 0.90 * pa);
                    pill(rt, b, mx + 15.0, my - 0.5, pv.right - 16.0, my + 2.5);
                    set_col(b, 0.56, 0.60, 0.68, 0.75 * pa);
                    pill(rt, b, mx + 15.0, my + 5.5, pv.right - 12.0, my + 7.8);
                }
                WidgetKind::Countdown => {
                    set_col(b, 0.65, 0.68, 0.74, 0.65 * pa);
                    pill(rt, b, pv.left + 6.0, pv.top + 5.5, pv.left + 32.0, pv.top + 8.0);
                    set_col(b, 1.0, 0.64, 0.14, 0.32 * pa);
                    pill(rt, b, pv.right - 19.0, pv.top + 4.5, pv.right - 5.5, pv.top + 9.5);
                    set_col(b, 0.98, 0.98, 1.0, pa);
                    pill(rt, b, pv.left + 6.5, pv.top + 16.0, pv.left + 21.0, pv.top + 23.0);
                    set_col(b, 0.90, 0.92, 0.96, 0.90 * pa);
                    pill(rt, b, pv.left + 25.0, pv.top + 16.0, pv.right - 12.0, pv.top + 19.0);
                    set_col(b, 0.60, 0.64, 0.70, 0.80 * pa);
                    pill(rt, b, pv.left + 25.0, pv.top + 21.2, pv.right - 18.0, pv.top + 23.6);
                    set_col(b, 1.0, 1.0, 1.0, 0.14 * pa);
                    pill(rt, b, pv.left + 6.5, pv.bottom - 11.0, pv.right - 6.5, pv.bottom - 6.2);
                    let fill_x = pv.left + pw * 0.72;
                    set_col(b, 1.0, 0.68, 0.16, pa);
                    pill(rt, b, pv.left + 6.5, pv.bottom - 11.0, fill_x, pv.bottom - 6.2);
                    set_col(b, 1.0, 0.86, 0.40, 0.48 * pa);
                    rt.FillEllipse(
                        &D2D1_ELLIPSE {
                            point: D2D_POINT_2F { x: fill_x - 2.0, y: pv.bottom - 8.6 },
                            radiusX: 4.5,
                            radiusY: 4.5,
                        },
                        b,
                    );
                }
                WidgetKind::Weather => {
                    let sx = pv.right - 13.5;
                    let sy = pv.top + 13.5;
                    set_col(b, 1.0, 0.88, 0.42, 0.28 * pa);
                    rt.FillEllipse(&D2D1_ELLIPSE { point: D2D_POINT_2F { x: sx, y: sy }, radiusX: 8.5, radiusY: 8.5 }, b);
                    set_col(b, 1.0, 0.85, 0.28, pa);
                    rt.FillEllipse(&D2D1_ELLIPSE { point: D2D_POINT_2F { x: sx, y: sy }, radiusX: 4.8, radiusY: 4.8 }, b);
                    set_col(b, 0.96, 0.98, 1.0, 0.96 * pa);
                    rt.FillEllipse(&D2D1_ELLIPSE { point: D2D_POINT_2F { x: sx - 5.0, y: sy + 4.0 }, radiusX: 3.6, radiusY: 3.0 }, b);
                    rt.FillEllipse(&D2D1_ELLIPSE { point: D2D_POINT_2F { x: sx - 0.5, y: sy + 3.2 }, radiusX: 4.2, radiusY: 3.4 }, b);
                    pill(rt, b, sx - 8.5, sy + 3.2, sx + 4.0, sy + 7.2);
                    set_col(b, 1.0, 1.0, 1.0, pa);
                    pill(rt, b, pv.left + 7.0, pv.top + 8.5, pv.left + 26.0, pv.top + 15.5);
                    set_col(b, 0.96, 0.98, 1.0, 0.94 * pa);
                    pill(rt, b, pv.left + 7.0, pv.bottom - 16.5, pv.left + 32.0, pv.bottom - 13.2);
                    set_col(b, 0.80, 0.88, 1.0, 0.85 * pa);
                    pill(rt, b, pv.left + 7.0, pv.bottom - 10.5, pv.left + 40.0, pv.bottom - 8.0);
                }
            }

            let is_l = wg_left == i as u8;
            let is_r = wg_right == i as u8;
            if is_l || is_r {
                let bx = card_r.right - 12.0;
                let by = card_r.top + 12.0;
                set_col(b, 0.08, 0.50, 0.96, pa);
                rt.FillEllipse(
                    &D2D1_ELLIPSE {
                        point: D2D_POINT_2F { x: bx, y: by },
                        radiusX: 7.2,
                        radiusY: 7.2,
                    },
                    b,
                );
                set_col(b, 1.0, 1.0, 1.0, pa);
                draw_text(
                    rt,
                    if is_l { "L" } else { "R" },
                    &gfx.fmt_preview,
                    &D2D_RECT_F {
                        left: bx - 7.0,
                        top: by - 7.5,
                        right: bx + 7.0,
                        bottom: by + 7.5,
                    },
                    b,
                );
            }

            set_col(b, 0.92 + 0.08 * sel_t, 0.92 + 0.08 * sel_t, 0.94 + 0.06 * sel_t, pa);
            draw_text(
                rt,
                kind.label(),
                &gfx.fmt_seg,
                &D2D_RECT_F {
                    left: card_r.left + 4.0,
                    top: card_r.top + 60.0,
                    right: card_r.right - 4.0,
                    bottom: card_r.bottom - 4.0,
                },
                b,
            );

            hits.push(HitBox {
                rect: card_r,
                hit: ControlHit::WidgetPick(i as u8),
            });
        }
    })
}

#[allow(clippy::too_many_arguments)]
unsafe fn draw_monitor_preview_card(
    rt: &ID2D1RenderTarget,
    brush: &ID2D1SolidColorBrush,
    gfx: &GfxResources,
    l: f32,
    r: f32,
    y: f32,
    monitor_index: usize,
    h_offset: f32,
    top_offset: f32,
    monitor_count: usize,
    pa: f32,
) -> f32 {
    let h = 122.0;
    let sw = GetSystemMetrics(SM_CXSCREEN).max(1);
    let sh = GetSystemMetrics(SM_CYSCREEN).max(1);
    draw_group_card(rt, brush, l, r, y, h, pa, |rt, b, top| {
        let mw = 160.0;
        let mh = 86.0;
        let cx = (l + r) * 0.5;
        let mon_r = D2D_RECT_F {
            left: cx - mw * 0.5,
            top: top + 12.0,
            right: cx + mw * 0.5,
            bottom: top + 12.0 + mh,
        };
        let mon_rr = D2D1_ROUNDED_RECT {
            rect: mon_r,
            radiusX: 7.0,
            radiusY: 7.0,
        };
        set_col(b, 0.085, 0.105, 0.095, pa);
        rt.FillRoundedRectangle(&mon_rr, b);
        set_col(b, 0.24, 0.72, 0.40, 0.72 * pa);
        rt.DrawRoundedRectangle(&mon_rr, b, 1.3, None);

        set_col(b, 0.34, 0.36, 0.38, 0.70 * pa);
        rt.FillRectangle(
            &D2D_RECT_F {
                left: cx - 7.0,
                top: mon_r.bottom + 1.0,
                right: cx + 7.0,
                bottom: mon_r.bottom + 8.0,
            },
            b,
        );
        rt.FillRoundedRectangle(
            &D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F {
                    left: cx - 22.0,
                    top: mon_r.bottom + 7.0,
                    right: cx + 22.0,
                    bottom: mon_r.bottom + 10.5,
                },
                radiusX: 1.8,
                radiusY: 1.8,
            },
            b,
        );

        let _ = (h_offset, top_offset);
        let active_mon = monitor_index.min(monitor_count.saturating_sub(1));
        let num_s = format!("{}", active_mon + 1);
        let res_s = format!(
            "{sw}×{sh} · {}",
            if active_mon == 0 { "Main" } else { "Display" }
        );
        set_col(b, 0.90, 0.92, 0.92, pa);
        draw_text(
            rt,
            &num_s,
            &gfx.fmt_seg_on,
            &D2D_RECT_F {
                left: mon_r.left,
                top: mon_r.top + 28.0,
                right: mon_r.right,
                bottom: mon_r.top + 48.0,
            },
            b,
        );
        set_col(b, 0.56, 0.62, 0.58, pa);
        draw_text(
            rt,
            &res_s,
            &gfx.fmt_preview,
            &D2D_RECT_F {
                left: mon_r.left,
                top: mon_r.top + 46.0,
                right: mon_r.right,
                bottom: mon_r.top + 64.0,
            },
            b,
        );
    })
}

unsafe fn draw_page_mini_thumb(
    rt: &ID2D1RenderTarget,
    brush: &ID2D1SolidColorBrush,
    x: f32,
    y: f32,
    page_idx: usize,
    accent: (f32, f32, f32),
    pa: f32,
) {
    let outer = D2D_RECT_F {
        left: x,
        top: y,
        right: x + 48.0,
        bottom: y + 24.0,
    };
    set_col(brush, 0.30, 0.30, 0.40, 0.42 * pa);
    rt.FillRoundedRectangle(
        &D2D1_ROUNDED_RECT {
            rect: outer,
            radiusX: 5.5,
            radiusY: 5.5,
        },
        brush,
    );
    let inner = D2D_RECT_F {
        left: x + 3.0,
        top: y + 2.0,
        right: x + 45.0,
        bottom: y + 22.0,
    };
    set_col(brush, 0.02, 0.02, 0.03, pa);
    rt.FillRoundedRectangle(
        &D2D1_ROUNDED_RECT {
            rect: inner,
            radiusX: 4.5,
            radiusY: 4.5,
        },
        brush,
    );

    match page_idx {
        0 => {
            set_col(brush, accent.0, accent.1, accent.2, pa);
            rt.FillRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: inner.left + 3.5,
                        top: inner.top + 4.0,
                        right: inner.left + 13.5,
                        bottom: inner.bottom - 4.0,
                    },
                    radiusX: 2.0,
                    radiusY: 2.0,
                },
                brush,
            );
            set_col(brush, 0.85, 0.85, 0.88, 0.85 * pa);
            rt.DrawLine(
                D2D_POINT_2F { x: inner.left + 17.0, y: inner.top + 8.0 },
                D2D_POINT_2F { x: inner.right - 5.0, y: inner.top + 8.0 },
                brush,
                1.6,
                None,
            );
            set_col(brush, 0.55, 0.55, 0.60, 0.7 * pa);
            rt.DrawLine(
                D2D_POINT_2F { x: inner.left + 17.0, y: inner.top + 13.0 },
                D2D_POINT_2F { x: inner.right - 10.0, y: inner.top + 13.0 },
                brush,
                1.4,
                None,
            );
        }
        1 => {
            set_col(brush, 0.92, 0.92, 0.95, pa);
            rt.DrawLine(
                D2D_POINT_2F { x: inner.left + 4.0, y: inner.top + 10.0 },
                D2D_POINT_2F { x: inner.left + 15.0, y: inner.top + 10.0 },
                brush,
                2.4,
                None,
            );
            for k in 0..3 {
                set_col(brush, accent.0, accent.1, accent.2, if k < 2 { pa } else { 0.35 * pa });
                rt.FillRoundedRectangle(
                    &D2D1_ROUNDED_RECT {
                        rect: D2D_RECT_F {
                            left: inner.left + 19.0 + k as f32 * 7.0,
                            top: inner.top + 9.0,
                            right: inner.left + 24.0 + k as f32 * 7.0,
                            bottom: inner.top + 12.0,
                        },
                        radiusX: 1.2,
                        radiusY: 1.2,
                    },
                    brush,
                );
            }
        }
        2 => {
            set_col(brush, accent.0, accent.1, accent.2, pa);
            rt.DrawEllipse(
                &D2D1_ELLIPSE {
                    point: D2D_POINT_2F {
                        x: inner.left + 10.5,
                        y: inner.top + 10.0,
                    },
                    radiusX: 4.5,
                    radiusY: 4.5,
                },
                brush,
                1.6,
                None,
            );
            for (k, bh) in [5.0, 9.0, 6.5, 11.0].iter().enumerate() {
                rt.FillRectangle(
                    &D2D_RECT_F {
                        left: inner.left + 20.0 + k as f32 * 4.5,
                        top: inner.bottom - 4.0 - bh,
                        right: inner.left + 22.5 + k as f32 * 4.5,
                        bottom: inner.bottom - 4.0,
                    },
                    brush,
                );
            }
        }
        3 => {
            for k in 0..3 {
                set_col(
                    brush,
                    if k == 0 { accent.0 } else { 0.45 },
                    if k == 0 { accent.1 } else { 0.45 },
                    if k == 0 { accent.2 } else { 0.50 },
                    0.85 * pa,
                );
                rt.FillRoundedRectangle(
                    &D2D1_ROUNDED_RECT {
                        rect: D2D_RECT_F {
                            left: inner.left + 5.0 + k as f32 * 11.5,
                            top: inner.top + 5.5,
                            right: inner.left + 13.5 + k as f32 * 11.5,
                            bottom: inner.bottom - 5.5,
                        },
                        radiusX: 2.0,
                        radiusY: 2.0,
                    },
                    brush,
                );
            }
        }
        4 => {
            for k in 0..2 {
                let ry = inner.top + 6.5 + k as f32 * 7.0;
                set_col(brush, accent.0, accent.1, accent.2, if k == 0 { pa } else { 0.4 * pa });
                rt.FillEllipse(
                    &D2D1_ELLIPSE {
                        point: D2D_POINT_2F { x: inner.left + 7.5, y: ry },
                        radiusX: 2.0,
                        radiusY: 2.0,
                    },
                    brush,
                );
                set_col(brush, 0.82, 0.84, 0.86, 0.85 * pa);
                rt.DrawLine(
                    D2D_POINT_2F { x: inner.left + 12.5, y: ry },
                    D2D_POINT_2F { x: inner.right - 6.0 - k as f32 * 5.0, y: ry },
                    brush,
                    1.5,
                    None,
                );
            }
        }
        _ => {
            let mid = (inner.left + inner.right) * 0.5;
            set_col(brush, 1.0, 1.0, 1.0, 0.14 * pa);
            rt.FillRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: inner.left + 3.5,
                        top: inner.top + 4.5,
                        right: mid - 1.5,
                        bottom: inner.bottom - 4.0,
                    },
                    radiusX: 2.5,
                    radiusY: 2.5,
                },
                brush,
            );
            rt.FillRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: mid + 1.5,
                        top: inner.top + 4.5,
                        right: inner.right - 3.5,
                        bottom: inner.bottom - 4.0,
                    },
                    radiusX: 2.5,
                    radiusY: 2.5,
                },
                brush,
            );
            set_col(brush, 0.14, 0.60, 1.0, pa);
            rt.FillEllipse(
                &D2D1_ELLIPSE {
                    point: D2D_POINT_2F {
                        x: inner.left + 8.0,
                        y: inner.top + 11.5,
                    },
                    radiusX: 1.8,
                    radiusY: 1.8,
                },
                brush,
            );
            set_col(brush, 0.19, 0.82, 0.35, pa);
            rt.DrawEllipse(
                &D2D1_ELLIPSE {
                    point: D2D_POINT_2F {
                        x: mid + 7.5,
                        y: inner.top + 10.2,
                    },
                    radiusX: 3.3,
                    radiusY: 3.3,
                },
                brush,
                1.4,
                None,
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
unsafe fn draw_section_header(
    rt: &ID2D1RenderTarget,
    brush: &ID2D1SolidColorBrush,
    fmt: &IDWriteTextFormat,
    title: &str,
    l: f32,
    r: f32,
    y: f32,
    pa: f32,
) -> f32 {
    set_col(brush, 0.54, 0.54, 0.56, pa);
    draw_text(
        rt,
        title,
        fmt,
        &D2D_RECT_F {
            left: l + 4.0,
            top: y,
            right: r,
            bottom: y + 17.0,
        },
        brush,
    );
    y + 21.0
}

#[allow(clippy::too_many_arguments)]
unsafe fn draw_group_card<F>(
    rt: &ID2D1RenderTarget,
    brush: &ID2D1SolidColorBrush,
    l: f32,
    r: f32,
    y: f32,
    h: f32,
    pa: f32,
    body: F,
) -> f32
where
    F: FnOnce(&ID2D1RenderTarget, &ID2D1SolidColorBrush, f32),
{
    let card = D2D1_ROUNDED_RECT {
        rect: D2D_RECT_F {
            left: l,
            top: y,
            right: r,
            bottom: y + h,
        },
        radiusX: 10.5,
        radiusY: 10.5,
    };
    set_col(brush, 0.137, 0.137, 0.153, pa);
    rt.FillRoundedRectangle(&card, brush);
    set_col(brush, 1.0, 1.0, 1.0, 0.06 * pa);
    rt.DrawRoundedRectangle(&card, brush, 0.9, None);
    body(rt, brush, y);
    y + h + 14.0
}

unsafe fn draw_row_divider(
    rt: &ID2D1RenderTarget,
    brush: &ID2D1SolidColorBrush,
    l: f32,
    r: f32,
    y: f32,
    pa: f32,
) {
    set_col(brush, 1.0, 1.0, 1.0, 0.065 * pa);
    rt.DrawLine(
        D2D_POINT_2F { x: l + 14.0, y },
        D2D_POINT_2F { x: r - 14.0, y },
        brush,
        0.9,
        None,
    );
}

#[allow(clippy::too_many_arguments)]
unsafe fn draw_toggle_row(
    rt: &ID2D1RenderTarget,
    brush: &ID2D1SolidColorBrush,
    gfx: &GfxResources,
    l: f32,
    r: f32,
    top: f32,
    h: f32,
    title: &str,
    sub: Option<&str>,
    t: f32,
    key: &'static str,
    pa: f32,
    hits: &mut Vec<HitBox>,
) {
    let t = t.clamp(0.0, 1.0);
    set_col(brush, 0.97, 0.97, 0.98, pa);
    if let Some(sub_txt) = sub {
        draw_text(
            rt,
            title,
            &gfx.fmt_row,
            &D2D_RECT_F {
                left: l + 14.0,
                top: top + 8.0,
                right: r - 66.0,
                bottom: top + 27.0,
            },
            brush,
        );
        set_col(brush, 0.55, 0.55, 0.57, pa);
        draw_text(
            rt,
            sub_txt,
            &gfx.fmt_sub,
            &D2D_RECT_F {
                left: l + 14.0,
                top: top + 27.0,
                right: r - 66.0,
                bottom: top + 45.0,
            },
            brush,
        );
    } else {
        draw_text(
            rt,
            title,
            &gfx.fmt_row,
            &D2D_RECT_F {
                left: l + 14.0,
                top,
                right: r - 66.0,
                bottom: top + h,
            },
            brush,
        );
    }

    let sw_w = 39.0;
    let sw_h = 22.0;
    let sw_r = D2D_RECT_F {
        left: r - 14.0 - sw_w,
        top: top + (h - sw_h) * 0.5,
        right: r - 14.0,
        bottom: top + (h + sw_h) * 0.5,
    };
    let e = t * t * (3.0 - 2.0 * t);
    let mid_stretch = 1.0 - (2.0 * e - 1.0).abs();
    set_col(
        brush,
        0.245 + (0.204 - 0.245) * e,
        0.245 + (0.780 - 0.245) * e,
        0.270 + (0.349 - 0.270) * e,
        pa,
    );
    rt.FillRoundedRectangle(
        &D2D1_ROUNDED_RECT {
            rect: sw_r,
            radiusX: 11.0,
            radiusY: 11.0,
        },
        brush,
    );
    set_col(brush, 1.0, 1.0, 1.0, (0.06 + 0.04 * e) * pa);
    rt.DrawRoundedRectangle(
        &D2D1_ROUNDED_RECT {
            rect: sw_r,
            radiusX: 11.0,
            radiusY: 11.0,
        },
        brush,
        0.75,
        None,
    );
    let thumb_rx = 8.8 + 1.45 * mid_stretch;
    let thumb_ry = 8.8;
    let thumb_cx = (sw_r.left + 11.0) + (sw_w - 22.0) * e;
    let thumb_cy = (sw_r.top + sw_r.bottom) * 0.5;
    set_col(brush, 0.0, 0.0, 0.0, 0.22 * pa);
    rt.FillRoundedRectangle(
        &D2D1_ROUNDED_RECT {
            rect: D2D_RECT_F {
                left: thumb_cx - thumb_rx,
                top: thumb_cy - thumb_ry + 0.9,
                right: thumb_cx + thumb_rx,
                bottom: thumb_cy + thumb_ry + 0.9,
            },
            radiusX: thumb_ry,
            radiusY: thumb_ry,
        },
        brush,
    );
    set_col(brush, 0.99, 0.99, 1.0, pa);
    rt.FillRoundedRectangle(
        &D2D1_ROUNDED_RECT {
            rect: D2D_RECT_F {
                left: thumb_cx - thumb_rx,
                top: thumb_cy - thumb_ry,
                right: thumb_cx + thumb_rx,
                bottom: thumb_cy + thumb_ry,
            },
            radiusX: thumb_ry,
            radiusY: thumb_ry,
        },
        brush,
    );

    hits.push(HitBox {
        rect: D2D_RECT_F {
            left: l,
            top,
            right: r,
            bottom: top + h,
        },
        hit: ControlHit::Toggle(key),
    });
}

#[allow(clippy::too_many_arguments)]
unsafe fn draw_segmented_row<F>(
    rt: &ID2D1RenderTarget,
    brush: &ID2D1SolidColorBrush,
    gfx: &GfxResources,
    l: f32,
    r: f32,
    top: f32,
    title: &str,
    sub: Option<&str>,
    options: &[&str],
    enabled_count: usize,
    active_f: f32,
    key: &'static str,
    pa: f32,
    hov_t: &F,
    hits: &mut Vec<HitBox>,
) where
    F: Fn(&str, D2D_RECT_F) -> f32,
{
    set_col(brush, 0.97, 0.97, 0.98, pa);
    draw_text(
        rt,
        title,
        &gfx.fmt_row,
        &D2D_RECT_F {
            left: l + 14.0,
            top: top + 8.0,
            right: r - 14.0,
            bottom: top + 26.0,
        },
        brush,
    );
    if let Some(sub_txt) = sub {
        set_col(brush, 0.55, 0.55, 0.57, pa);
        draw_text(
            rt,
            sub_txt,
            &gfx.fmt_sub,
            &D2D_RECT_F {
                left: l + 14.0,
                top: top + 26.0,
                right: r - 14.0,
                bottom: top + 41.0,
            },
            brush,
        );
    }

    let track = D2D_RECT_F {
        left: l + 14.0,
        top: top + 46.0,
        right: r - 14.0,
        bottom: top + 73.0,
    };
    set_col(brush, 0.082, 0.082, 0.092, pa);
    rt.FillRoundedRectangle(
        &D2D1_ROUNDED_RECT {
            rect: track,
            radiusX: 7.5,
            radiusY: 7.5,
        },
        brush,
    );

    let n = options.len().max(1);
    let inner_w = (track.right - track.left - 4.0).max(1.0);
    let seg_w = inner_w / n as f32;
    let clamped = active_f.clamp(0.0, (n - 1) as f32);

    for i in 0..n {
        if i >= enabled_count {
            continue;
        }
        let cell = D2D_RECT_F {
            left: track.left + 2.0 + i as f32 * seg_w,
            top: track.top + 2.0,
            right: track.left + 2.0 + (i + 1) as f32 * seg_w,
            bottom: track.bottom - 2.0,
        };
        let seg_on = (1.0 - (clamped - i as f32).abs()).clamp(0.0, 1.0);
        let h_key = format!("seg_{key}_{i}");
        let h_t = hov_t(&h_key, cell);
        let hov_a = 0.065 * h_t * (1.0 - seg_on);
        if hov_a > 0.004 {
            set_col(brush, 1.0, 1.0, 1.0, hov_a * pa);
            rt.FillRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: cell,
                    radiusX: 6.0,
                    radiusY: 6.0,
                },
                brush,
            );
        }
    }

    let thumb = D2D_RECT_F {
        left: track.left + 2.0 + clamped * seg_w,
        top: track.top + 2.0,
        right: track.left + 2.0 + (clamped + 1.0) * seg_w,
        bottom: track.bottom - 2.0,
    };
    set_col(brush, 0.28, 0.28, 0.32, pa);
    rt.FillRoundedRectangle(
        &D2D1_ROUNDED_RECT {
            rect: thumb,
            radiusX: 6.0,
            radiusY: 6.0,
        },
        brush,
    );

    let active_idx = clamped.round() as usize;
    for (i, &opt) in options.iter().enumerate() {
        let cell = D2D_RECT_F {
            left: track.left + 2.0 + i as f32 * seg_w,
            top: track.top,
            right: track.left + 2.0 + (i + 1) as f32 * seg_w,
            bottom: track.bottom,
        };
        let enabled = i < enabled_count;
        let seg_on = if enabled {
            (1.0 - (clamped - i as f32).abs()).clamp(0.0, 1.0)
        } else {
            0.0
        };
        if enabled {
            let lum = 0.68 + 0.32 * seg_on;
            set_col(brush, lum, lum, lum + 0.02 * (1.0 - seg_on), pa);
        } else {
            set_col(brush, 0.42, 0.42, 0.44, 0.45 * pa);
        }
        draw_text(
            rt,
            opt,
            if i == active_idx && enabled { &gfx.fmt_seg_on } else { &gfx.fmt_seg },
            &cell,
            brush,
        );
        if enabled {
            hits.push(HitBox {
                rect: cell,
                hit: ControlHit::Segment(key, i),
            });
        }
    }
}

#[allow(clippy::too_many_arguments)]
unsafe fn draw_stepper_row<F>(
    rt: &ID2D1RenderTarget,
    brush: &ID2D1SolidColorBrush,
    gfx: &GfxResources,
    l: f32,
    r: f32,
    top: f32,
    h: f32,
    title: &str,
    sub: Option<&str>,
    val_txt: &str,
    key: &'static str,
    step: i32,
    show_center: bool,
    pa: f32,
    hov_t: &F,
    hits: &mut Vec<HitBox>,
) where
    F: Fn(&str, D2D_RECT_F) -> f32,
{
    set_col(brush, 0.97, 0.97, 0.98, pa);
    draw_text(
        rt,
        title,
        &gfx.fmt_row,
        &D2D_RECT_F {
            left: l + 14.0,
            top: top + 8.0,
            right: r - 180.0,
            bottom: top + 27.0,
        },
        brush,
    );
    if let Some(sub_txt) = sub {
        set_col(brush, 0.55, 0.55, 0.57, pa);
        draw_text(
            rt,
            sub_txt,
            &gfx.fmt_sub,
            &D2D_RECT_F {
                left: l + 14.0,
                top: top + 27.0,
                right: r - 180.0,
                bottom: top + 45.0,
            },
            brush,
        );
    }

    let cy = top + h * 0.5;
    let plus_r = D2D_RECT_F {
        left: r - 40.0,
        top: cy - 12.5,
        right: r - 14.0,
        bottom: cy + 12.5,
    };
    let minus_r = D2D_RECT_F {
        left: r - 70.0,
        top: cy - 12.5,
        right: r - 44.0,
        bottom: cy + 12.5,
    };

    for (btn_r, sym, delta, suffix) in [(minus_r, "−", -step, "m"), (plus_r, "+", step, "p")] {
        let h_key = format!("step_{key}_{suffix}");
        let h_t = hov_t(&h_key, btn_r);
        set_col(brush, 1.0, 1.0, 1.0, (0.10 + 0.09 * h_t) * pa);
        rt.FillRoundedRectangle(
            &D2D1_ROUNDED_RECT {
                rect: btn_r,
                radiusX: 6.5,
                radiusY: 6.5,
            },
            brush,
        );
        set_col(brush, 0.96, 0.96, 0.98, pa);
        draw_text(rt, sym, &gfx.fmt_seg_on, &btn_r, brush);
        hits.push(HitBox {
            rect: btn_r,
            hit: ControlHit::Stepper(key, delta),
        });
    }

    let mut val_right = minus_r.left - 8.0;
    if show_center {
        let ctr_r = D2D_RECT_F {
            left: minus_r.left - 52.0,
            top: cy - 12.5,
            right: minus_r.left - 6.0,
            bottom: cy + 12.5,
        };
        let h_key = format!("step_{key}_c");
        let h_t = hov_t(&h_key, ctr_r);
        set_col(brush, 1.0, 1.0, 1.0, (0.08 + 0.09 * h_t) * pa);
        rt.FillRoundedRectangle(
            &D2D1_ROUNDED_RECT {
                rect: ctr_r,
                radiusX: 6.5,
                radiusY: 6.5,
            },
            brush,
        );
        set_col(brush, 0.86, 0.86, 0.88, pa);
        draw_text(rt, "Center", &gfx.fmt_seg, &ctr_r, brush);
        hits.push(HitBox {
            rect: ctr_r,
            hit: ControlHit::Stepper(key, 0),
        });
        val_right = ctr_r.left - 8.0;
    }

    set_col(brush, 0.82, 0.82, 0.84, pa);
    draw_text(
        rt,
        val_txt,
        &gfx.fmt_val,
        &D2D_RECT_F {
            left: val_right - 68.0,
            top,
            right: val_right,
            bottom: top + h,
        },
        brush,
    );
}

#[allow(clippy::too_many_arguments)]
unsafe fn draw_info_row(
    rt: &ID2D1RenderTarget,
    brush: &ID2D1SolidColorBrush,
    gfx: &GfxResources,
    l: f32,
    r: f32,
    top: f32,
    h: f32,
    title: &str,
    sub: Option<&str>,
    val_txt: &str,
    pa: f32,
) {
    set_col(brush, 0.97, 0.97, 0.98, pa);
    draw_text(
        rt,
        title,
        &gfx.fmt_row,
        &D2D_RECT_F {
            left: l + 14.0,
            top: top + 8.0,
            right: r - 102.0,
            bottom: top + 27.0,
        },
        brush,
    );
    if let Some(sub_txt) = sub {
        set_col(brush, 0.55, 0.55, 0.57, pa);
        draw_text(
            rt,
            sub_txt,
            &gfx.fmt_sub,
            &D2D_RECT_F {
                left: l + 14.0,
                top: top + 27.0,
                right: r - 102.0,
                bottom: top + 45.0,
            },
            brush,
        );
    }
    set_col(brush, 0.68, 0.68, 0.70, pa);
    draw_text(
        rt,
        val_txt,
        &gfx.fmt_val,
        &D2D_RECT_F {
            left: r - 102.0,
            top,
            right: r - 14.0,
            bottom: top + h,
        },
        brush,
    );
}

#[allow(clippy::too_many_arguments)]
unsafe fn draw_action_row<F>(
    rt: &ID2D1RenderTarget,
    brush: &ID2D1SolidColorBrush,
    gfx: &GfxResources,
    l: f32,
    r: f32,
    top: f32,
    h: f32,
    title: &str,
    sub: Option<&str>,
    btn_label: &str,
    key: &'static str,
    destructive: bool,
    pa: f32,
    hov_t: &F,
    hits: &mut Vec<HitBox>,
) where
    F: Fn(&str, D2D_RECT_F) -> f32,
{
    set_col(brush, 0.97, 0.97, 0.98, pa);
    draw_text(
        rt,
        title,
        &gfx.fmt_row,
        &D2D_RECT_F {
            left: l + 14.0,
            top: top + 8.0,
            right: r - 106.0,
            bottom: top + 27.0,
        },
        brush,
    );
    if let Some(sub_txt) = sub {
        set_col(brush, 0.55, 0.55, 0.57, pa);
        draw_text(
            rt,
            sub_txt,
            &gfx.fmt_sub,
            &D2D_RECT_F {
                left: l + 14.0,
                top: top + 27.0,
                right: r - 106.0,
                bottom: top + 45.0,
            },
            brush,
        );
    }

    let cy = top + h * 0.5;
    let btn_r = D2D_RECT_F {
        left: r - 96.0,
        top: cy - 13.0,
        right: r - 14.0,
        bottom: cy + 13.0,
    };
    let h_key = format!("act_{key}");
    let h_t = hov_t(&h_key, btn_r);
    if destructive {
        set_col(brush, 0.95, 0.28, 0.25, (0.20 + 0.16 * h_t) * pa);
    } else {
        set_col(brush, 1.0, 1.0, 1.0, (0.10 + 0.09 * h_t) * pa);
    }
    rt.FillRoundedRectangle(
        &D2D1_ROUNDED_RECT {
            rect: btn_r,
            radiusX: 7.0,
            radiusY: 7.0,
        },
        brush,
    );
    if destructive {
        set_col(brush, 1.0, 0.44, 0.42, pa);
    } else {
        set_col(brush, 0.95, 0.95, 0.97, pa);
    }
    draw_text(rt, btn_label, &gfx.fmt_seg_on, &btn_r, brush);

    hits.push(HitBox {
        rect: btn_r,
        hit: ControlHit::ActionBtn(key),
    });
}

unsafe fn draw_sidebar_glyph(
    factory: &ID2D1Factory1,
    rt: &ID2D1RenderTarget,
    brush: &ID2D1SolidColorBrush,
    tab: SettingsTab,
    badge_rgb: (f32, f32, f32),
    cx: f32,
    cy: f32,
) {
    let round_st = ID2D1Factory::CreateStrokeStyle(
        factory,
        &D2D1_STROKE_STYLE_PROPERTIES {
            startCap: D2D1_CAP_STYLE_ROUND,
            endCap: D2D1_CAP_STYLE_ROUND,
            lineJoin: D2D1_LINE_JOIN_ROUND,
            ..Default::default()
        },
        None,
    )
    .ok();
    let (br, bg, bb) = badge_rgb;
    set_col(brush, 1.0, 1.0, 1.0, 0.98);
    match tab {
        SettingsTab::General => {
            for k in 0..8 {
                let ang = (k as f32) * std::f32::consts::FRAC_PI_4;
                let (s, c) = ang.sin_cos();
                rt.DrawLine(
                    D2D_POINT_2F { x: cx + c * 3.0, y: cy + s * 3.0 },
                    D2D_POINT_2F { x: cx + c * 5.7, y: cy + s * 5.7 },
                    brush,
                    2.2,
                    round_st.as_ref(),
                );
            }
            rt.FillEllipse(
                &D2D1_ELLIPSE {
                    point: D2D_POINT_2F { x: cx, y: cy },
                    radiusX: 4.5,
                    radiusY: 4.5,
                },
                brush,
            );
            set_col(brush, br, bg, bb, 1.0);
            rt.FillEllipse(
                &D2D1_ELLIPSE {
                    point: D2D_POINT_2F { x: cx, y: cy },
                    radiusX: 2.0,
                    radiusY: 2.0,
                },
                brush,
            );
        }
        SettingsTab::Display => {
            rt.FillRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: cx - 5.9,
                        top: cy - 4.7,
                        right: cx + 5.9,
                        bottom: cy + 2.5,
                    },
                    radiusX: 1.6,
                    radiusY: 1.6,
                },
                brush,
            );
            rt.FillRectangle(
                &D2D_RECT_F {
                    left: cx - 1.3,
                    top: cy + 2.2,
                    right: cx + 1.3,
                    bottom: cy + 4.7,
                },
                brush,
            );
            rt.DrawLine(
                D2D_POINT_2F { x: cx - 3.2, y: cy + 4.9 },
                D2D_POINT_2F { x: cx + 3.2, y: cy + 4.9 },
                brush,
                1.5,
                round_st.as_ref(),
            );
        }
        SettingsTab::Indicators => {
            if let Ok(geo) = factory.CreatePathGeometry() {
                if let Ok(sink) = geo.Open() {
                    sink.BeginFigure(
                        D2D_POINT_2F { x: cx - 4.8, y: cy + 3.0 },
                        D2D1_FIGURE_BEGIN_FILLED,
                    );
                    sink.AddBezier(&D2D1_BEZIER_SEGMENT {
                        point1: D2D_POINT_2F { x: cx - 3.6, y: cy + 2.0 },
                        point2: D2D_POINT_2F { x: cx - 3.8, y: cy - 4.3 },
                        point3: D2D_POINT_2F { x: cx, y: cy - 4.5 },
                    });
                    sink.AddBezier(&D2D1_BEZIER_SEGMENT {
                        point1: D2D_POINT_2F { x: cx + 3.8, y: cy - 4.3 },
                        point2: D2D_POINT_2F { x: cx + 3.6, y: cy + 2.0 },
                        point3: D2D_POINT_2F { x: cx + 4.8, y: cy + 3.0 },
                    });
                    sink.EndFigure(D2D1_FIGURE_END_CLOSED);
                    if sink.Close().is_ok() {
                        rt.FillGeometry(&geo, brush, None);
                    }
                }
            }
            rt.FillEllipse(
                &D2D1_ELLIPSE {
                    point: D2D_POINT_2F { x: cx, y: cy - 5.1 },
                    radiusX: 1.0,
                    radiusY: 1.0,
                },
                brush,
            );
            rt.FillEllipse(
                &D2D1_ELLIPSE {
                    point: D2D_POINT_2F { x: cx, y: cy + 4.6 },
                    radiusX: 1.35,
                    radiusY: 1.25,
                },
                brush,
            );
        }
        SettingsTab::EaveAi => {
            if let Ok(geo) = factory.CreatePathGeometry() {
                if let Ok(sink) = geo.Open() {
                    let sx = cx - 0.8;
                    let sy = cy + 0.6;
                    let ro = 5.4;
                    let ri = 1.45;
                    sink.BeginFigure(
                        D2D_POINT_2F { x: sx, y: sy - ro },
                        D2D1_FIGURE_BEGIN_FILLED,
                    );
                    sink.AddLine(D2D_POINT_2F { x: sx + ri, y: sy - ri });
                    sink.AddLine(D2D_POINT_2F { x: sx + ro, y: sy });
                    sink.AddLine(D2D_POINT_2F { x: sx + ri, y: sy + ri });
                    sink.AddLine(D2D_POINT_2F { x: sx, y: sy + ro });
                    sink.AddLine(D2D_POINT_2F { x: sx - ri, y: sy + ri });
                    sink.AddLine(D2D_POINT_2F { x: sx - ro, y: sy });
                    sink.AddLine(D2D_POINT_2F { x: sx - ri, y: sy - ri });
                    sink.EndFigure(D2D1_FIGURE_END_CLOSED);
                    if sink.Close().is_ok() {
                        rt.FillGeometry(&geo, brush, None);
                    }
                }
            }
            if let Ok(geo2) = factory.CreatePathGeometry() {
                if let Ok(sink) = geo2.Open() {
                    let sx = cx + 4.0;
                    let sy = cy - 3.9;
                    let ro = 2.1;
                    let ri = 0.6;
                    sink.BeginFigure(
                        D2D_POINT_2F { x: sx, y: sy - ro },
                        D2D1_FIGURE_BEGIN_FILLED,
                    );
                    sink.AddLine(D2D_POINT_2F { x: sx + ri, y: sy - ri });
                    sink.AddLine(D2D_POINT_2F { x: sx + ro, y: sy });
                    sink.AddLine(D2D_POINT_2F { x: sx + ri, y: sy + ri });
                    sink.AddLine(D2D_POINT_2F { x: sx, y: sy + ro });
                    sink.AddLine(D2D_POINT_2F { x: sx - ri, y: sy + ri });
                    sink.AddLine(D2D_POINT_2F { x: sx - ro, y: sy });
                    sink.AddLine(D2D_POINT_2F { x: sx - ri, y: sy - ri });
                    sink.EndFigure(D2D1_FIGURE_END_CLOSED);
                    if sink.Close().is_ok() {
                        rt.FillGeometry(&geo2, brush, None);
                    }
                }
            }
        }
        SettingsTab::Pages => {
            for dy in [-3.4, 0.0, 3.4] {
                rt.FillEllipse(
                    &D2D1_ELLIPSE {
                        point: D2D_POINT_2F { x: cx - 4.3, y: cy + dy },
                        radiusX: 1.05,
                        radiusY: 1.05,
                    },
                    brush,
                );
                rt.DrawLine(
                    D2D_POINT_2F { x: cx - 1.6, y: cy + dy },
                    D2D_POINT_2F { x: cx + 4.8, y: cy + dy },
                    brush,
                    1.75,
                    round_st.as_ref(),
                );
            }
        }
        SettingsTab::Music => {
            rt.DrawLine(
                D2D_POINT_2F { x: cx - 1.7, y: cy + 3.5 },
                D2D_POINT_2F { x: cx - 1.7, y: cy - 3.9 },
                brush,
                1.55,
                round_st.as_ref(),
            );
            rt.DrawLine(
                D2D_POINT_2F { x: cx - 1.7, y: cy - 3.9 },
                D2D_POINT_2F { x: cx + 4.1, y: cy - 5.1 },
                brush,
                2.1,
                round_st.as_ref(),
            );
            rt.DrawLine(
                D2D_POINT_2F { x: cx + 4.1, y: cy - 5.1 },
                D2D_POINT_2F { x: cx + 4.1, y: cy + 2.2 },
                brush,
                1.55,
                round_st.as_ref(),
            );
            rt.FillEllipse(
                &D2D1_ELLIPSE {
                    point: D2D_POINT_2F { x: cx - 3.1, y: cy + 3.7 },
                    radiusX: 2.05,
                    radiusY: 1.65,
                },
                brush,
            );
            rt.FillEllipse(
                &D2D1_ELLIPSE {
                    point: D2D_POINT_2F { x: cx + 2.7, y: cy + 2.4 },
                    radiusX: 2.05,
                    radiusY: 1.65,
                },
                brush,
            );
        }
        SettingsTab::Widgets => {
            for &(dx, dy) in &[(-2.8, -2.8), (2.8, -2.8), (-2.8, 2.8), (2.8, 2.8)] {
                rt.FillRoundedRectangle(
                    &D2D1_ROUNDED_RECT {
                        rect: D2D_RECT_F {
                            left: cx + dx - 2.15,
                            top: cy + dy - 2.15,
                            right: cx + dx + 2.15,
                            bottom: cy + dy + 2.15,
                        },
                        radiusX: 1.15,
                        radiusY: 1.15,
                    },
                    brush,
                );
            }
        }
        SettingsTab::Focus => {
            rt.DrawLine(
                D2D_POINT_2F { x: cx - 1.6, y: cy - 5.5 },
                D2D_POINT_2F { x: cx + 1.6, y: cy - 5.5 },
                brush,
                1.45,
                round_st.as_ref(),
            );
            rt.DrawLine(
                D2D_POINT_2F { x: cx, y: cy - 5.4 },
                D2D_POINT_2F { x: cx, y: cy - 3.6 },
                brush,
                1.4,
                round_st.as_ref(),
            );
            rt.FillEllipse(
                &D2D1_ELLIPSE {
                    point: D2D_POINT_2F { x: cx, y: cy + 0.9 },
                    radiusX: 4.8,
                    radiusY: 4.8,
                },
                brush,
            );
            set_col(brush, br, bg, bb, 1.0);
            rt.DrawLine(
                D2D_POINT_2F { x: cx, y: cy + 0.9 },
                D2D_POINT_2F { x: cx, y: cy - 1.8 },
                brush,
                1.45,
                round_st.as_ref(),
            );
            rt.DrawLine(
                D2D_POINT_2F { x: cx, y: cy + 0.9 },
                D2D_POINT_2F { x: cx + 2.2, y: cy + 2.0 },
                brush,
                1.45,
                round_st.as_ref(),
            );
        }
        SettingsTab::Water => {
            if let Ok(geo) = factory.CreatePathGeometry() {
                if let Ok(sink) = geo.Open() {
                    sink.BeginFigure(
                        D2D_POINT_2F { x: cx, y: cy - 5.6 },
                        D2D1_FIGURE_BEGIN_FILLED,
                    );
                    sink.AddBezier(&D2D1_BEZIER_SEGMENT {
                        point1: D2D_POINT_2F { x: cx + 1.6, y: cy - 2.6 },
                        point2: D2D_POINT_2F { x: cx + 4.4, y: cy - 0.1 },
                        point3: D2D_POINT_2F { x: cx + 4.3, y: cy + 1.8 },
                    });
                    sink.AddBezier(&D2D1_BEZIER_SEGMENT {
                        point1: D2D_POINT_2F { x: cx + 4.2, y: cy + 4.4 },
                        point2: D2D_POINT_2F { x: cx + 2.2, y: cy + 5.7 },
                        point3: D2D_POINT_2F { x: cx, y: cy + 5.7 },
                    });
                    sink.AddBezier(&D2D1_BEZIER_SEGMENT {
                        point1: D2D_POINT_2F { x: cx - 2.2, y: cy + 5.7 },
                        point2: D2D_POINT_2F { x: cx - 4.2, y: cy + 4.4 },
                        point3: D2D_POINT_2F { x: cx - 4.3, y: cy + 1.8 },
                    });
                    sink.AddBezier(&D2D1_BEZIER_SEGMENT {
                        point1: D2D_POINT_2F { x: cx - 4.4, y: cy - 0.1 },
                        point2: D2D_POINT_2F { x: cx - 1.6, y: cy - 2.6 },
                        point3: D2D_POINT_2F { x: cx, y: cy - 5.6 },
                    });
                    sink.EndFigure(D2D1_FIGURE_END_CLOSED);
                    if sink.Close().is_ok() {
                        rt.FillGeometry(&geo, brush, None);
                    }
                }
            }
            set_col(brush, br, bg, bb, 0.72);
            rt.DrawLine(
                D2D_POINT_2F { x: cx - 1.9, y: cy + 1.6 },
                D2D_POINT_2F { x: cx - 1.1, y: cy + 3.2 },
                brush,
                1.15,
                round_st.as_ref(),
            );
        }
        SettingsTab::Todo => {
            rt.FillEllipse(
                &D2D1_ELLIPSE {
                    point: D2D_POINT_2F { x: cx, y: cy },
                    radiusX: 5.4,
                    radiusY: 5.4,
                },
                brush,
            );
            set_col(brush, br, bg, bb, 1.0);
            rt.DrawLine(
                D2D_POINT_2F { x: cx - 2.7, y: cy + 0.1 },
                D2D_POINT_2F { x: cx - 0.7, y: cy + 2.2 },
                brush,
                1.75,
                round_st.as_ref(),
            );
            rt.DrawLine(
                D2D_POINT_2F { x: cx - 0.7, y: cy + 2.2 },
                D2D_POINT_2F { x: cx + 2.9, y: cy - 2.0 },
                brush,
                1.75,
                round_st.as_ref(),
            );
        }
        SettingsTab::Shelf => {
            rt.FillRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: cx - 5.5,
                        top: cy - 4.7,
                        right: cx - 0.4,
                        bottom: cy - 1.8,
                    },
                    radiusX: 1.2,
                    radiusY: 1.2,
                },
                brush,
            );
            rt.FillRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: cx - 5.5,
                        top: cy - 3.0,
                        right: cx + 5.5,
                        bottom: cy + 4.6,
                    },
                    radiusX: 1.7,
                    radiusY: 1.7,
                },
                brush,
            );
        }
        SettingsTab::About => {
            rt.FillEllipse(
                &D2D1_ELLIPSE {
                    point: D2D_POINT_2F { x: cx, y: cy },
                    radiusX: 5.4,
                    radiusY: 5.4,
                },
                brush,
            );
            set_col(brush, br, bg, bb, 1.0);
            rt.FillEllipse(
                &D2D1_ELLIPSE {
                    point: D2D_POINT_2F { x: cx, y: cy - 2.5 },
                    radiusX: 0.95,
                    radiusY: 0.95,
                },
                brush,
            );
            rt.DrawLine(
                D2D_POINT_2F { x: cx, y: cy - 0.4 },
                D2D_POINT_2F { x: cx, y: cy + 2.8 },
                brush,
                1.65,
                round_st.as_ref(),
            );
        }
    }
}

unsafe extern "system" fn settings_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const RefCell<SettingsInner>;
    if ptr.is_null() {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    let cell = &*ptr;

    match msg {
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let _ = BeginPaint(hwnd, &mut ps);
            if let Ok(mut st) = cell.try_borrow_mut() {
                st.paint();
            }
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_SIZE => {
            if let Ok(st) = cell.try_borrow() {
                let w = (lparam.0 & 0xFFFF) as u32;
                let h = ((lparam.0 >> 16) & 0xFFFF) as u32;
                if let Some(gfx) = st.gfx.as_ref() {
                    let _ = gfx.rt.Resize(&D2D_SIZE_U {
                        width: w.max(1),
                        height: h.max(1),
                    });
                }
            }
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let x_px = (lparam.0 & 0xFFFF) as i16 as f32;
            let y_px = ((lparam.0 >> 16) & 0xFFFF) as i16 as f32;
            let mut need_repaint = false;
            if let Ok(mut st) = cell.try_borrow_mut() {
                let scale = (st.dpi / 96.0).max(0.5);
                let x = x_px / scale;
                let y = y_px / scale;
                st.hover_pt = Some((x, y));
                if !st.tracking_mouse {
                    let mut tme = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    let _ = TrackMouseEvent(&mut tme);
                    st.tracking_mouse = true;
                }
                if let Some((dpid, grab_dy, _)) = st.dragging_page {
                    let cur_top = (y - grab_dy).clamp(st.pages_card_top - 4.0, st.pages_card_top + 264.0);
                    st.dragging_page = Some((dpid, grab_dy, cur_top));
                    st.page_row_y[dpid] = cur_top - st.pages_card_top;
                    let target_slot = (((cur_top - st.pages_card_top + 26.0) / 52.0).floor() as i32)
                        .clamp(0, 5) as usize;
                    if let Some(cur_slot) = st
                        .snap
                        .cfg
                        .page_order
                        .iter()
                        .position(|&p| p as usize == dpid)
                    {
                        if cur_slot != target_slot {
                            let mut v = st.snap.cfg.page_order.to_vec();
                            let item = v.remove(cur_slot);
                            v.insert(target_slot, item);
                            st.snap.cfg.page_order.copy_from_slice(&v);
                            let cfg_clone = st.snap.cfg.clone();
                            st.actions.push(SettingsAction::UpdateConfig(cfg_clone));
                        }
                    }
                    st.ensure_anim_timer();
                    need_repaint = true;
                } else {
                    let cur_hov = st.hit_boxes.iter().rposition(|hb| {
                        x >= hb.rect.left
                            && x <= hb.rect.right
                            && y >= hb.rect.top
                            && y <= hb.rect.bottom
                    });
                    if cur_hov != st.last_hover_idx {
                        st.last_hover_idx = cur_hov;
                        st.ensure_anim_timer();
                        need_repaint = true;
                    }
                }
            }
            if need_repaint {
                let _ = InvalidateRect(hwnd, None, false);
            }
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            if let Ok(mut st) = cell.try_borrow_mut() {
                st.hover_pt = None;
                st.last_hover_idx = None;
                st.tracking_mouse = false;
                st.ensure_anim_timer();
            }
            let _ = InvalidateRect(hwnd, None, false);
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let x_px = (lparam.0 & 0xFFFF) as i16 as f32;
            let y_px = ((lparam.0 >> 16) & 0xFFFF) as i16 as f32;
            let (drag_window, started_page_drag) = if let Ok(mut st) = cell.try_borrow_mut() {
                let scale = (st.dpi / 96.0).max(0.5);
                let x = x_px / scale;
                let y = y_px / scale;
                let hit = st
                    .hit_boxes
                    .iter()
                    .rev()
                    .find(|hb| {
                        x >= hb.rect.left
                            && x <= hb.rect.right
                            && y >= hb.rect.top
                            && y <= hb.rect.bottom
                    })
                    .map(|hb| hb.hit.clone());
                if y <= 44.0 && hit.is_none() {
                    (true, false)
                } else if let Some(ControlHit::PageDrag(pid)) = hit {
                    let row_top = st.pages_card_top + st.page_row_y[pid];
                    st.dragging_page = Some((pid, y - row_top, row_top));
                    st.ensure_anim_timer();
                    (false, true)
                } else {
                    (false, false)
                }
            } else {
                (false, false)
            };
            if drag_window {
                let _ = ReleaseCapture();
                let _ = SendMessageW(hwnd, WM_NCLBUTTONDOWN, WPARAM(HTCAPTION as usize), LPARAM(0));
                return LRESULT(0);
            }
            if started_page_drag {
                let _ = SetCapture(hwnd);
                let _ = InvalidateRect(hwnd, None, false);
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let mut pt = POINT {
                x: (lparam.0 & 0xFFFF) as i16 as i32,
                y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32,
            };
            let _ = ScreenToClient(hwnd, &mut pt);
            let x_px = (lparam.0 & 0xFFFF) as i16 as f32;
            let y_px = ((lparam.0 >> 16) & 0xFFFF) as i16 as f32;
            let was_drag = if let Ok(mut st) = cell.try_borrow_mut() {
                if st.dragging_page.take().is_some() {
                    st.ensure_anim_timer();
                    true
                } else {
                    let scale = (st.dpi / 96.0).max(0.5);
                    st.handle_click(x_px / scale, y_px / scale);
                    false
                }
            } else {
                false
            };
            if was_drag {
                let _ = ReleaseCapture();
                let _ = InvalidateRect(hwnd, None, false);
            }
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            let delta = ((wparam.0 >> 16) as i16) as f32;
            if let Ok(mut st) = cell.try_borrow_mut() {
                if st.max_scroll > 0.0 {
                    st.scroll_target = (st.scroll_target - delta * 0.34).clamp(0.0, st.max_scroll);
                    st.ensure_anim_timer();
                }
            }
            LRESULT(0)
        }
        WM_TIMER => {
            if wparam.0 == ANIM_TIMER_ID {
                let destroy_now = if let Ok(mut st) = cell.try_borrow_mut() {
                    let (active, done_closing) = st.step_animations();
                    if !active {
                        st.stop_anim_timer();
                    }
                    done_closing
                } else {
                    false
                };
                if destroy_now {
                    let _ = DestroyWindow(hwnd);
                } else {
                    let _ = InvalidateRect(hwnd, None, false);
                }
            }
            LRESULT(0)
        }
        WM_KEYDOWN => {
            if wparam.0 as u16 == VK_ESCAPE.0 {
                let _ = PostMessageW(hwnd, WM_CLOSE, WPARAM(0), LPARAM(0));
                return LRESULT(0);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_CLOSE => {
            if let Ok(mut st) = cell.try_borrow_mut() {
                if st.win_opacity <= 0.05 {
                    let _ = DestroyWindow(hwnd);
                } else {
                    st.request_close();
                }
            } else {
                let _ = DestroyWindow(hwnd);
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            if let Ok(mut st) = cell.try_borrow_mut() {
                st.stop_anim_timer();
                st.gfx = None;
                st.hwnd = None;
                st.hit_boxes.clear();
                st.closing = false;
                st.layered_active = false;
            }
            let _ = Rc::from_raw(ptr);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

#[allow(dead_code)]
pub fn _settings_path_hint(_p: &Path) {}
