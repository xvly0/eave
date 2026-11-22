use std::mem::ManuallyDrop;
use std::time::{SystemTime, UNIX_EPOCH};

use windows::core::{w, Interface, Result, HSTRING};
use windows::Foundation::Numerics::Matrix3x2;
use windows::Win32::Foundation::{BOOL, FILETIME, HMODULE, SYSTEMTIME};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_IGNORE, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F,
    D2D1_FIGURE_BEGIN_FILLED, D2D1_FIGURE_END_CLOSED, D2D1_GRADIENT_STOP, D2D1_PIXEL_FORMAT,
    D2D_POINT_2F, D2D_RECT_F, D2D_SIZE_F, D2D_SIZE_U,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, ID2D1Bitmap1, ID2D1BitmapBrush1, ID2D1Brush, ID2D1Device,
    ID2D1DeviceContext, ID2D1Factory, ID2D1Factory1, ID2D1Geometry, ID2D1PathGeometry,
    ID2D1RadialGradientBrush, ID2D1RenderTarget, ID2D1SolidColorBrush, ID2D1StrokeStyle, D2D1_ANTIALIAS_MODE_ALIASED,
    D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_ARC_SEGMENT, D2D1_ARC_SIZE_SMALL,
    D2D1_BITMAP_BRUSH_PROPERTIES1, D2D1_BITMAP_OPTIONS_CANNOT_DRAW, D2D1_BITMAP_OPTIONS_TARGET,
    D2D1_BITMAP_PROPERTIES1,
    D2D1_BRUSH_PROPERTIES, D2D1_CAP_STYLE_ROUND, D2D1_DASH_STYLE_CUSTOM,
    D2D1_DEVICE_CONTEXT_OPTIONS_NONE, D2D1_DRAW_TEXT_OPTIONS_CLIP, D2D1_DRAW_TEXT_OPTIONS_NO_SNAP,
    D2D1_ELLIPSE, D2D1_EXTEND_MODE_CLAMP, D2D1_EXTEND_MODE_WRAP,
    D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_GAMMA_2_2, D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
    D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR, D2D1_LAYER_OPTIONS1_NONE, D2D1_LAYER_PARAMETERS1,
    D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES, D2D1_LINE_JOIN_ROUND,
    D2D1_RADIAL_GRADIENT_BRUSH_PROPERTIES, D2D1_ROUNDED_RECT, D2D1_STROKE_STYLE_PROPERTIES,
    D2D1_SWEEP_DIRECTION_CLOCKWISE, D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE,
    D2D1_TEXT_ANTIALIAS_MODE_CLEARTYPE, D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE,
};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP, D3D_FEATURE_LEVEL_10_0,
    D3D_FEATURE_LEVEL_10_1, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION,
};
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, IDWriteFactory, IDWriteFontCollection, IDWriteRenderingParams,
    IDWriteTextFormat, IDWriteTextLayout, DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL,
    DWRITE_FONT_STYLE_ITALIC, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT_BOLD,
    DWRITE_FONT_WEIGHT_MEDIUM, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_WEIGHT_SEMI_BOLD,
    DWRITE_HIT_TEST_METRICS, DWRITE_LINE_SPACING_METHOD_UNIFORM, DWRITE_MEASURING_MODE_NATURAL,
    DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_PARAGRAPH_ALIGNMENT_NEAR,
    DWRITE_RENDERING_MODE_NATURAL_SYMMETRIC, DWRITE_TEXT_ALIGNMENT_CENTER,
    DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_TEXT_ALIGNMENT_TRAILING, DWRITE_TEXT_METRICS,
    DWRITE_TEXT_RANGE, DWRITE_TRIMMING, DWRITE_TRIMMING_GRANULARITY_CHARACTER,
    DWRITE_WORD_WRAPPING_EMERGENCY_BREAK, DWRITE_WORD_WRAPPING_NO_WRAP, DWRITE_WORD_WRAPPING_WRAP,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_PREMULTIPLIED, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    IDXGIDevice, IDXGIFactory2, IDXGISurface, IDXGISwapChain1, DXGI_PRESENT, DXGI_SCALING_STRETCH,
    DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_CHAIN_FLAG, DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
    DXGI_USAGE_RENDER_TARGET_OUTPUT,
};
use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};

use crate::config::Config;
use crate::icons;
use crate::lyrics;
use crate::pages::{
    NotchPage, PomodoroPhase, PomodoroState, TodoState, WaterState, WidgetKind, WidgetState,
};
use crate::physics::NotchSprings;
use crate::shelf::{ShelfItem, ShelfStore, ShelfTab};

/// Zero-copy borrowed album art slice (premultiplied BGRA8) for the renderer.
#[derive(Debug, Clone, Copy)]
pub struct AlbumArtView<'a> {
    pub width: u32,
    pub height: u32,
    pub bgra: &'a [u8],
    pub revision: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CollapsedOsdKind {
    #[default]
    None,
    Volume,
    Brightness,
    CapsLock { on: bool },
    MicMute { muted: bool },
    MicInUse,
    AudioDevice,
    Network,
    Bluetooth,
    Charging { charging: bool, pct: u8 },
    Notification,
    FocusAlert {
        next_is_focus: bool,
        is_long_break: bool,
        next_mins: u32,
    },
    WaterReminder,
    TodoReminder,
    UpdateAvailable,
    UpdateInstalled,
}

pub fn interactive_osd_button_rects_at(body_right: f32, notch_h: f32) -> (D2D_RECT_F, D2D_RECT_F) {
    let cy = (notch_h - 3.0) * 0.5;
    let btn_h = 30.0;
    let cta_w = 72.0;
    let cta_r = body_right - 14.0;
    let cta_l = cta_r - cta_w;
    let dismiss_r = cta_l - 8.0;
    let dismiss_l = dismiss_r - btn_h;
    (
        D2D_RECT_F {
            left: dismiss_l,
            top: cy - btn_h * 0.5,
            right: dismiss_r,
            bottom: cy + btn_h * 0.5,
        },
        D2D_RECT_F {
            left: cta_l,
            top: cy - btn_h * 0.5,
            right: cta_r,
            bottom: cy + btn_h * 0.5,
        },
    )
}

pub fn interactive_osd_button_rects(
    notch_w: f32,
    notch_h: f32,
    ear_r: f32,
) -> (D2D_RECT_F, D2D_RECT_F) {
    interactive_osd_button_rects_at(ear_r + notch_w, notch_h)
}

pub fn ai_card_rects(
    canvas_cx: f32,
    notch_h: f32,
    card_h: f32,
) -> (D2D_RECT_F, D2D_RECT_F, D2D_RECT_F, D2D_RECT_F) {
    let card_w = 564.0;
    let left = (canvas_cx - card_w * 0.5).round();
    let right = left + card_w;
    let top = notch_h + 12.0;
    let bottom = top + card_h;
    let close_r = right - 14.0;
    let close_l = close_r - 24.0;
    let copy_r = close_l - 4.0;
    let copy_l = copy_r - 24.0;
    let hdr_cy = top + 24.0;
    (
        D2D_RECT_F {
            left,
            top,
            right,
            bottom,
        },
        D2D_RECT_F {
            left: copy_l,
            top: hdr_cy - 12.0,
            right: copy_r,
            bottom: hdr_cy + 12.0,
        },
        D2D_RECT_F {
            left: close_l,
            top: hdr_cy - 12.0,
            right: close_r,
            bottom: hdr_cy + 12.0,
        },
        D2D_RECT_F {
            left: left + 12.0,
            top: bottom - 36.0,
            right: right - 12.0,
            bottom: bottom - 6.0,
        },
    )
}

pub fn ai_clear_rect(copy_r: D2D_RECT_F) -> D2D_RECT_F {
    let right = copy_r.left - 4.0;
    D2D_RECT_F {
        left: right - 24.0,
        top: copy_r.top,
        right,
        bottom: copy_r.bottom,
    }
}

pub fn ai_input_enter_rect(pill_left: f32, cur_w: f32, cur_h: f32, ear_r: f32) -> D2D_RECT_F {
    let body_right = pill_left + cur_w - ear_r;
    let cy = cur_h * 0.5;
    D2D_RECT_F {
        left: body_right - 42.0,
        top: cy - 11.5,
        right: body_right - 16.0,
        bottom: cy + 11.5,
    }
}

pub fn ai_input_toggle_rect(pill_left: f32, cur_w: f32, cur_h: f32, ear_r: f32) -> D2D_RECT_F {
    let enter_r = ai_input_enter_rect(pill_left, cur_w, cur_h, ear_r);
    D2D_RECT_F {
        left: enter_r.left - 32.0,
        top: enter_r.top,
        right: enter_r.left - 6.0,
        bottom: enter_r.bottom,
    }
}

pub fn ai_turn_dot_rects(card_r: D2D_RECT_F, total_turns: usize) -> Vec<D2D_RECT_F> {
    if total_turns < 2 {
        return Vec::new();
    }
    let avail_h = (card_r.bottom - card_r.top - 34.0).max(24.0);
    let step = (avail_h / (total_turns - 1) as f32).clamp(9.5, 14.0);
    let total_h = (total_turns - 1) as f32 * step;
    let cx = card_r.left - 16.0;
    let start_y = (card_r.top + card_r.bottom) * 0.5 - total_h * 0.5;
    let half_h = (step * 0.5).max(5.0);
    (0..total_turns)
        .map(|i| {
            let cy = start_y + i as f32 * step;
            D2D_RECT_F {
                left: cx - 9.0,
                top: cy - half_h,
                right: cx + 9.0,
                bottom: cy + half_h,
            }
        })
        .collect()
}

#[derive(Debug, Clone, Copy)]
pub struct MediaViewState<'a> {
    pub has_session: bool,
    pub is_playing: bool,
    pub title: &'a str,
    pub artist: &'a str,
    pub album: &'a str,
    pub position_secs: f64,
    pub duration_secs: f64,
    pub thumbnail: Option<AlbumArtView<'a>>,
    pub lyric_prev_prev: &'a str,
    pub lyric_prev: &'a str,
    pub lyric_current: &'a str,
    pub lyric_next: &'a str,
    pub lyric_next_after: &'a str,
    pub lyric_started: bool,
    #[allow(dead_code)]
    pub lyric_prev_time: Option<f64>,
    #[allow(dead_code)]
    pub lyric_current_time: Option<f64>,
    #[allow(dead_code)]
    pub lyric_next_time: Option<f64>,
    #[allow(dead_code)]
    pub ambient_rgb: Option<(f32, f32, f32)>,
}

impl Default for MediaViewState<'_> {
    fn default() -> Self {
        Self {
            has_session: false,
            is_playing: false,
            title: "",
            artist: "",
            album: "",
            position_secs: 0.0,
            duration_secs: 0.0,
            thumbnail: None,
            lyric_prev_prev: "",
            lyric_prev: "",
            lyric_current: "",
            lyric_next: "",
            lyric_next_after: "",
            lyric_started: true,
            lyric_prev_time: None,
            lyric_current_time: None,
            lyric_next_time: None,
            ambient_rgb: None,
        }
    }
}

pub struct EaveViewState<'a> {
    pub page: NotchPage,
    pub prev_page: NotchPage,
    pub media: MediaViewState<'a>,
    pub pomodoro: &'a PomodoroState,
    pub water: &'a WaterState,
    pub shelf: &'a ShelfStore,
    pub todo: &'a TodoState,
    pub widgets: &'a WidgetState,
    #[allow(dead_code)]
    pub lyrics_open: bool,
    pub muted: bool,
    pub has_notification: bool,
    pub collapsed_osd: CollapsedOsdKind,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HitTarget {
    None,
    Body,
    ChevronLeft,
    ChevronRight,
    BellButton,
    DockZone,
    DockPage(NotchPage),
    DockSettings,
    PrevButton,
    PlayPauseButton,
    NextButton,
    ProgressBar(f64),
    LyricsButton,
    LyricSeek(usize),
    SpeakerButton,
    VolumeSlider(f64),
    PomodoroToggle,
    PomodoroReset,
    PomodoroSwitch,
    PomodoroSegment(usize),
    WaterAdd,
    WaterReset,
    WaterToggleReminder,
    WaterSwitchView,
    ShelfDropZone,
    ShelfTabFiles,
    ShelfTabClipboard,
    ShelfChip(usize),
    #[allow(dead_code)]
    ShelfRemove(usize),
    ShelfSelectToggle,
    ShelfClear,
    ClipboardCard(usize),
    ClipboardRemove(usize),
    TodoToggle(usize),
    TodoPanelEmpty,
    TodoAdd,
    TodoEditToggle,
    TodoCalendar,
    TodoConfirmAdd,
    TodoCancelAdd,
    TodoFormTitle,
    TodoFormTime,
    TodoFormDaily,
    #[allow(dead_code)]
    TodoClearDone,
    WidgetWeekDay(usize),
    WidgetCardLeft,
    WidgetCardRight,
}

#[derive(Debug, Clone, Copy)]
struct ExpandedLayout {
    cx: f32,
    body_left: f32,
    body_right: f32,
    base_h: f32,
    page_title_rect: D2D_RECT_F,
    header_clock_rect: D2D_RECT_F,
    bell_center: D2D_POINT_2F,
    chevron_left_center: D2D_POINT_2F,
    chevron_right_center: D2D_POINT_2F,
    title_rect: D2D_RECT_F,
    artist_rect: D2D_RECT_F,
    #[allow(dead_code)]
    lyric_rect: D2D_RECT_F,
    bar_hit_rect: D2D_RECT_F,
    lyrics_btn_center: D2D_POINT_2F,
    prev_center: D2D_POINT_2F,
    play_center: D2D_POINT_2F,
    next_center: D2D_POINT_2F,
    speaker_btn_center: D2D_POINT_2F,
    shelf_zone_rect: D2D_RECT_F,
    dock_w: f32,
    dock_top: f32,
    dock_item_centers: [D2D_POINT_2F; 6],
    dock_divider_x: f32,
}

impl ExpandedLayout {
    fn compute(canvas_cx: f32, body_w: f32, base_h: f32) -> Self {
        let body_left = (canvas_cx - body_w * 0.5).round();
        let body_right = body_left + body_w;
        let cx = canvas_cx;

        let page_title_rect = D2D_RECT_F {
            left: body_left + 32.0,
            top: 18.0,
            right: body_left + 185.0,
            bottom: 38.0,
        };
        let header_clock_rect = D2D_RECT_F {
            left: cx - 125.0,
            top: 18.0,
            right: cx + 125.0,
            bottom: 38.0,
        };
        let bell_center = D2D_POINT_2F {
            x: body_right - 31.0,
            y: 28.0,
        };

        let chevron_left_center = D2D_POINT_2F {
            x: body_left + 13.0,
            y: 94.0,
        };
        let chevron_right_center = D2D_POINT_2F {
            x: body_right - 13.0,
            y: 94.0,
        };

        let text_left = body_left + 109.0;
        let title_rect = D2D_RECT_F {
            left: text_left,
            top: 44.0,
            right: body_right - 31.0,
            bottom: 66.0,
        };
        let artist_rect = D2D_RECT_F {
            left: text_left,
            top: 63.0,
            right: body_right - 31.0,
            bottom: 80.0,
        };
        let lyric_rect = D2D_RECT_F {
            left: text_left,
            top: 80.0,
            right: body_right - 31.0,
            bottom: 95.0,
        };

        let prog_cy = 106.5;
        let bar_left = body_left + 76.0;
        let bar_right = (body_right - 79.0).max(bar_left + 32.0);
        let bar_hit_rect = D2D_RECT_F {
            left: bar_left,
            top: prog_cy - 9.0,
            right: bar_right,
            bottom: prog_cy + 9.0,
        };

        let ctrl_y = 136.5;
        let lyrics_btn_center = D2D_POINT_2F {
            x: body_left + 61.5,
            y: ctrl_y,
        };
        let prev_center = D2D_POINT_2F {
            x: cx - 48.0,
            y: ctrl_y,
        };
        let play_center = D2D_POINT_2F { x: cx, y: ctrl_y };
        let next_center = D2D_POINT_2F {
            x: cx + 48.0,
            y: ctrl_y,
        };
        let speaker_btn_center = D2D_POINT_2F {
            x: body_right - 41.0,
            y: ctrl_y,
        };

        let shelf_zone_rect = D2D_RECT_F {
            left: body_left + 31.0,
            top: 52.0,
            right: body_right - 31.0,
            bottom: 148.0_f32.min(base_h - 24.0),
        };

        let dock_w = 194.0;
        let dock_top = base_h - 4.0;
        let dock_cy = dock_top + 11.5;
        let dock_left = cx - dock_w * 0.5;
        let slot_step = 27.0;
        let start_x = dock_left + 20.0;
        let dock_divider_x = start_x + 4.5 * slot_step + 2.5;
        let dock_item_centers = [
            D2D_POINT_2F { x: start_x + 0.0 * slot_step, y: dock_cy },
            D2D_POINT_2F { x: start_x + 1.0 * slot_step, y: dock_cy },
            D2D_POINT_2F { x: start_x + 2.0 * slot_step, y: dock_cy },
            D2D_POINT_2F { x: start_x + 3.0 * slot_step, y: dock_cy },
            D2D_POINT_2F { x: start_x + 4.0 * slot_step, y: dock_cy },
            D2D_POINT_2F { x: dock_divider_x + 17.0, y: dock_cy },
        ];

        Self {
            cx,
            body_left,
            body_right,
            base_h,
            page_title_rect,
            header_clock_rect,
            bell_center,
            chevron_left_center,
            chevron_right_center,
            title_rect,
            artist_rect,
            lyric_rect,
            bar_hit_rect,
            lyrics_btn_center,
            prev_center,
            play_center,
            next_center,
            speaker_btn_center,
            shelf_zone_rect,
            dock_w,
            dock_top,
            dock_item_centers,
            dock_divider_x,
        }
    }
}

pub fn dock_hovered_index(target: HitTarget) -> Option<usize> {
    match target {
        HitTarget::DockPage(p) => Some(p.index()),
        HitTarget::DockSettings => Some(6),
        _ => None,
    }
}

pub fn hit_target_slot(target: HitTarget) -> Option<usize> {
    match target {
        HitTarget::BellButton => Some(0),
        HitTarget::ChevronLeft => Some(1),
        HitTarget::ChevronRight => Some(2),
        HitTarget::PrevButton => Some(3),
        HitTarget::PlayPauseButton => Some(4),
        HitTarget::NextButton => Some(5),
        HitTarget::LyricsButton => Some(6),
        HitTarget::LyricSeek(i) => Some(39 + i.min(2)),
        HitTarget::SpeakerButton => Some(7),
        HitTarget::VolumeSlider(_) => Some(42),
        HitTarget::ProgressBar(_) => Some(8),
        HitTarget::PomodoroToggle => Some(9),
        HitTarget::PomodoroReset => Some(10),
        HitTarget::PomodoroSwitch => Some(11),
        HitTarget::WaterAdd => Some(12),
        HitTarget::WaterReset => Some(13),
        HitTarget::WaterToggleReminder => Some(14),
        HitTarget::WaterSwitchView => Some(15),
        HitTarget::ShelfDropZone => Some(15),
        HitTarget::ShelfClear => Some(16),
        HitTarget::ShelfTabFiles => Some(17),
        HitTarget::ShelfTabClipboard => Some(18),
        HitTarget::ShelfChip(i) => Some(19 + i.min(4)),
        HitTarget::ShelfRemove(i) => Some(24 + i.min(4)),
        HitTarget::ClipboardCard(i) => Some(29 + i.min(3)),
        HitTarget::ClipboardRemove(i) => Some(43 + i.min(3)),
        HitTarget::ShelfSelectToggle => Some(38),
        HitTarget::TodoToggle(i) => Some(29 + i.min(2)),
        HitTarget::TodoAdd => Some(33),
        HitTarget::TodoEditToggle | HitTarget::TodoClearDone => Some(34),
        HitTarget::TodoCalendar => Some(35),
        HitTarget::TodoConfirmAdd => Some(36),
        HitTarget::TodoCancelAdd => Some(37),
        HitTarget::None
        | HitTarget::Body
        | HitTarget::DockZone
        | HitTarget::DockPage(_)
        | HitTarget::DockSettings
        | HitTarget::PomodoroSegment(_)
        | HitTarget::TodoPanelEmpty
        | HitTarget::TodoFormTitle
        | HitTarget::TodoFormTime
        | HitTarget::TodoFormDaily
        | HitTarget::WidgetWeekDay(_)
        | HitTarget::WidgetCardLeft
        | HitTarget::WidgetCardRight => None,
    }
}

pub fn is_in_dock_zone(
    x_dips: f32,
    y_dips: f32,
    cur_w_dips: f32,
    base_h_dips: f32,
    cur_h_dips: f32,
    dock_hover: f32,
) -> bool {
    let cx = cur_w_dips * 0.5;
    if dock_hover < 0.15 {
        (x_dips - cx).abs() <= 64.0 && y_dips >= (base_h_dips - 18.0) && y_dips <= base_h_dips + 2.0
    } else {
        (x_dips - cx).abs() <= 148.0
            && y_dips >= (base_h_dips - 20.0)
            && y_dips <= cur_h_dips + 4.0
    }
}

#[allow(dead_code)]
pub fn is_in_shelf_zone(
    x_dips: f32,
    y_dips: f32,
    total_w_dips: f32,
    base_h_dips: f32,
    ear_r_dips: f32,
) -> bool {
    let body_w = (total_w_dips - 2.0 * ear_r_dips).max(160.0);
    let layout = ExpandedLayout::compute(total_w_dips * 0.5, body_w, base_h_dips);
    point_in_rect(x_dips, y_dips, layout.shelf_zone_rect)
}

fn compute_enabled_dock_layout(
    cx: f32,
    dock_cy: f32,
    page_order: [u8; 6],
    page_enabled: [bool; 6],
) -> (Vec<usize>, Vec<D2D_POINT_2F>, f32, D2D_POINT_2F, f32) {
    let mut ids: Vec<usize> = page_order
        .iter()
        .copied()
        .filter_map(|p| {
            let idx = p as usize;
            if idx < 6 && page_enabled[idx] {
                Some(idx)
            } else {
                None
            }
        })
        .collect();
    if ids.is_empty() {
        ids.push(0);
    }
    let n = ids.len();
    let slot_step = 30.0_f32;
    let dock_w = (n as f32 * slot_step + 48.0).max(88.0);
    let start_x = cx - dock_w * 0.5 + 18.5;
    let centers: Vec<D2D_POINT_2F> = (0..n)
        .map(|k| D2D_POINT_2F {
            x: start_x + k as f32 * slot_step,
            y: dock_cy,
        })
        .collect();
    let divider_x = start_x + (n as f32 - 0.5) * slot_step + 2.5;
    let gear_center = D2D_POINT_2F {
        x: divider_x + 17.0,
        y: dock_cy,
    };
    (ids, centers, divider_x, gear_center, dock_w)
}

fn widget_card_rects(body_left: f32, body_right: f32) -> (D2D_RECT_F, D2D_RECT_F) {
    let inner_l = body_left + 26.0;
    let inner_r = body_right - 26.0;
    let gap = 10.0;
    let card_w = ((inner_r - inner_l - gap) * 0.5).max(120.0);
    let left = D2D_RECT_F {
        left: inner_l,
        top: 44.0,
        right: inner_l + card_w,
        bottom: 152.0,
    };
    let right = D2D_RECT_F {
        left: left.right + gap,
        top: 44.0,
        right: inner_r,
        bottom: 152.0,
    };
    (left, right)
}

#[allow(clippy::too_many_arguments, dead_code)]
pub fn hit_test_notch(
    x_dips: f32,
    y_dips: f32,
    total_w_dips: f32,
    base_h_dips: f32,
    cur_h_dips: f32,
    ear_r_dips: f32,
    expand: f32,
    dock_hover: f32,
    has_notification: bool,
    page: NotchPage,
    shelf_tab: ShelfTab,
    shelf_len: usize,
    clip_len: usize,
    todo_len: usize,
    todo_adding: bool,
    lyrics_open: bool,
    volume_slider_open: f32,
) -> HitTarget {
    hit_test_notch_ex(
        x_dips,
        y_dips,
        total_w_dips,
        base_h_dips,
        cur_h_dips,
        ear_r_dips,
        expand,
        dock_hover,
        has_notification,
        page,
        shelf_tab,
        shelf_len,
        clip_len,
        todo_len,
        todo_adding,
        lyrics_open,
        volume_slider_open,
        0.0,
        [0, 1, 2, 3, 4, 5],
        [true, true, true, true, true, false],
        0,
        1,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn hit_test_notch_ex(
    x_dips: f32,
    y_dips: f32,
    total_w_dips: f32,
    base_h_dips: f32,
    cur_h_dips: f32,
    ear_r_dips: f32,
    expand: f32,
    dock_hover: f32,
    has_notification: bool,
    page: NotchPage,
    shelf_tab: ShelfTab,
    shelf_len: usize,
    clip_len: usize,
    todo_len: usize,
    todo_adding: bool,
    lyrics_open: bool,
    volume_slider_open: f32,
    todo_list_expanded: f32,
    page_order: [u8; 6],
    page_enabled: [bool; 6],
    widget_left: u8,
    widget_right: u8,
) -> HitTarget {
    if x_dips < 0.0 || y_dips < 0.0 || x_dips > total_w_dips || y_dips > cur_h_dips {
        return HitTarget::None;
    }
    if expand < 0.65 {
        return HitTarget::Body;
    }

    let body_w = (total_w_dips - 2.0 * ear_r_dips).max(160.0);
    let layout = ExpandedLayout::compute(total_w_dips * 0.5, body_w, base_h_dips);
    let lyrics_locked = page == NotchPage::Music && lyrics_open;

    if !lyrics_locked
        && is_in_dock_zone(
            x_dips,
            y_dips,
            total_w_dips,
            base_h_dips,
            cur_h_dips,
            dock_hover,
        )
    {
        if dock_hover > 0.35 || (y_dips >= layout.dock_top && y_dips <= layout.dock_top + 23.0) {
            let dock_cy = layout.dock_item_centers[0].y;
            let (enabled_ids, centers, _div_x, gear_c, _dock_w) =
                compute_enabled_dock_layout(layout.cx, dock_cy, page_order, page_enabled);
            for (slot, center) in centers.iter().enumerate() {
                if dist_sq(x_dips, y_dips, *center) <= 13.0_f32.powi(2) {
                    return HitTarget::DockPage(NotchPage::from_index(enabled_ids[slot]));
                }
            }
            if dist_sq(x_dips, y_dips, gear_c) <= 13.0_f32.powi(2) {
                return HitTarget::DockSettings;
            }
        }
        return HitTarget::DockZone;
    }

    if !lyrics_locked
        && has_notification
        && dist_sq(x_dips, y_dips, layout.bell_center) <= 13.0_f32.powi(2)
    {
        return HitTarget::BellButton;
    }
    if !lyrics_locked && dist_sq(x_dips, y_dips, layout.chevron_left_center) <= 18.5_f32.powi(2) {
        return HitTarget::ChevronLeft;
    }
    if !lyrics_locked && dist_sq(x_dips, y_dips, layout.chevron_right_center) <= 18.5_f32.powi(2) {
        return HitTarget::ChevronRight;
    }

    match page {
        NotchPage::Music => {
            if lyrics_open {
                let bl = layout.body_left;
                let br = layout.body_right;
                let ctrl_y = 136.5;
                if dist_sq(x_dips, y_dips, D2D_POINT_2F { x: bl + 131.0, y: ctrl_y }) <= 16.0_f32.powi(2) {
                    return HitTarget::PlayPauseButton;
                }
                if dist_sq(x_dips, y_dips, D2D_POINT_2F { x: bl + 91.0, y: ctrl_y }) <= 14.0_f32.powi(2) {
                    return HitTarget::PrevButton;
                }
                if dist_sq(x_dips, y_dips, D2D_POINT_2F { x: bl + 171.0, y: ctrl_y }) <= 14.0_f32.powi(2) {
                    return HitTarget::NextButton;
                }
                if dist_sq(x_dips, y_dips, D2D_POINT_2F { x: bl + 53.0, y: ctrl_y }) <= 14.0_f32.powi(2) {
                    return HitTarget::LyricsButton;
                }
                let bar_hit = D2D_RECT_F {
                    left: bl + 69.0,
                    top: 98.0,
                    right: bl + 188.0,
                    bottom: 116.0,
                };
                if point_in_rect(x_dips, y_dips, bar_hit) {
                    let width = (bar_hit.right - bar_hit.left).max(1.0);
                    let ratio = ((x_dips - bar_hit.left) / width).clamp(0.0, 1.0) as f64;
                    return HitTarget::ProgressBar(ratio);
                }
                if x_dips >= bl + 260.0 && x_dips <= br - 12.0 {
                    if (38.0..72.0).contains(&y_dips) {
                        return HitTarget::LyricSeek(0);
                    }
                    if (72.0..122.0).contains(&y_dips) {
                        return HitTarget::LyricSeek(1);
                    }
                    if (122.0..=154.0).contains(&y_dips) {
                        return HitTarget::LyricSeek(2);
                    }
                }
            } else {
                if dist_sq(x_dips, y_dips, layout.play_center) <= 16.0_f32.powi(2) {
                    return HitTarget::PlayPauseButton;
                }
                if dist_sq(x_dips, y_dips, layout.prev_center) <= 14.0_f32.powi(2) {
                    return HitTarget::PrevButton;
                }
                if dist_sq(x_dips, y_dips, layout.next_center) <= 14.0_f32.powi(2) {
                    return HitTarget::NextButton;
                }
                if dist_sq(x_dips, y_dips, layout.lyrics_btn_center) <= 14.0_f32.powi(2) {
                    return HitTarget::LyricsButton;
                }
                if dist_sq(x_dips, y_dips, layout.speaker_btn_center) <= 14.0_f32.powi(2) {
                    return HitTarget::SpeakerButton;
                }
                if volume_slider_open > 0.25 {
                    let (full_rect, _, hit_rect) =
                        music_volume_slider_rects(layout.speaker_btn_center, volume_slider_open);
                    if point_in_rect(x_dips, y_dips, hit_rect) {
                        let full_w = (full_rect.right - full_rect.left).max(1.0);
                        let ratio = ((x_dips - full_rect.left) / full_w).clamp(0.0, 1.0) as f64;
                        return HitTarget::VolumeSlider(ratio);
                    }
                }
                if point_in_rect(x_dips, y_dips, layout.bar_hit_rect) {
                    let width = (layout.bar_hit_rect.right - layout.bar_hit_rect.left).max(1.0);
                    let ratio = ((x_dips - layout.bar_hit_rect.left) / width).clamp(0.0, 1.0) as f64;
                    return HitTarget::ProgressBar(ratio);
                }
            }
        }
        NotchPage::Pomodoro => {
            let (btn_toggle, reset_c, switch_rect) = pomodoro_controls(layout.body_left);
            if point_in_rect(x_dips, y_dips, btn_toggle) {
                return HitTarget::PomodoroToggle;
            }
            if dist_sq(x_dips, y_dips, reset_c) <= 14.0_f32.powi(2) {
                return HitTarget::PomodoroReset;
            }
            if point_in_rect(x_dips, y_dips, switch_rect) {
                return HitTarget::PomodoroSwitch;
            }
            const POMO_SEG_W: [f32; 8] = [30.0, 6.0, 30.0, 6.0, 30.0, 6.0, 30.0, 18.0];
            const POMO_SEG_GAP: f32 = 3.0;
            let mut sx = layout.body_left + 190.0;
            for (idx, &sw) in POMO_SEG_W.iter().enumerate() {
                let seg_hit = D2D_RECT_F {
                    left: sx - 1.5,
                    top: 90.0,
                    right: sx + sw + 1.5,
                    bottom: 123.0,
                };
                if point_in_rect(x_dips, y_dips, seg_hit) {
                    return HitTarget::PomodoroSegment(idx);
                }
                sx += sw + POMO_SEG_GAP;
            }
        }
        NotchPage::Water => {
            let (switch_rect, btn_drink, btn_reset) = water_controls(layout.body_left);
            if point_in_rect(x_dips, y_dips, switch_rect) {
                return HitTarget::WaterToggleReminder;
            }
            if point_in_rect(x_dips, y_dips, btn_drink) {
                return HitTarget::WaterAdd;
            }
            if point_in_rect(x_dips, y_dips, btn_reset) {
                return HitTarget::WaterReset;
            }
            let water_view_rect = D2D_RECT_F {
                left: layout.body_left + 200.0,
                top: 44.0,
                right: layout.body_right - 12.0,
                bottom: 145.0,
            };
            if point_in_rect(x_dips, y_dips, water_view_rect) {
                return HitTarget::WaterSwitchView;
            }
        }
        NotchPage::Shelf => {
            let (_pill_rect, files_rect, clip_rect) = shelf_switcher_rects(layout.body_right);
            if point_in_rect(x_dips, y_dips, files_rect) {
                return HitTarget::ShelfTabFiles;
            }
            if point_in_rect(x_dips, y_dips, clip_rect) {
                return HitTarget::ShelfTabClipboard;
            }
            let (select_rect, clear_rect) =
                shelf_bottom_button_rects(layout.body_left, layout.body_right);
            match shelf_tab {
                ShelfTab::Files => {
                    if shelf_len > 0 {
                        if point_in_rect(x_dips, y_dips, select_rect) {
                            return HitTarget::ShelfSelectToggle;
                        }
                        if point_in_rect(x_dips, y_dips, clear_rect) {
                            return HitTarget::ShelfClear;
                        }
                        let visible = shelf_len.min(5);
                        for idx in 0..visible {
                            let card_rect = shelf_card_rect(layout.body_left, idx);
                            let del_c = D2D_POINT_2F {
                                x: card_rect.right - 7.5,
                                y: card_rect.top + 7.5,
                            };
                            if dist_sq(x_dips, y_dips, del_c) <= 8.5_f32.powi(2) {
                                return HitTarget::ShelfRemove(idx);
                            }
                            if point_in_rect(x_dips, y_dips, card_rect) {
                                return HitTarget::ShelfChip(idx);
                            }
                        }
                    } else if point_in_rect(x_dips, y_dips, layout.shelf_zone_rect) {
                        return HitTarget::ShelfDropZone;
                    }
                }
                ShelfTab::Clipboard => {
                    if clip_len > 0 && point_in_rect(x_dips, y_dips, clear_rect) {
                        return HitTarget::ShelfClear;
                    }
                    let visible = clip_len.min(4);
                    for idx in 0..visible {
                        let card_rect = cb_card_rect(layout.body_left, idx);
                        let del_c = D2D_POINT_2F {
                            x: card_rect.right - 10.0,
                            y: card_rect.top + 10.0,
                        };
                        if dist_sq(x_dips, y_dips, del_c) <= 8.5_f32.powi(2) {
                            return HitTarget::ClipboardRemove(idx);
                        }
                        if point_in_rect(x_dips, y_dips, card_rect) {
                            return HitTarget::ClipboardCard(idx);
                        }
                    }
                }
            }
        }
        NotchPage::Todo => {
            let exp_t = todo_list_expanded.clamp(0.0, 1.0);
            if exp_t < 0.5 {
                let cal_c = todo_calendar_center(layout.body_left);
                if dist_sq(x_dips, y_dips, cal_c) <= 15.0_f32.powi(2) {
                    return HitTarget::TodoCalendar;
                }
            }
            let (add_rect, edit_rect) = todo_button_rects_morphed(layout.body_left, exp_t);
            if point_in_rect(x_dips, y_dips, add_rect) {
                return HitTarget::TodoAdd;
            }
            if point_in_rect(x_dips, y_dips, edit_rect) {
                return HitTarget::TodoEditToggle;
            }
            if todo_adding {
                let (panel, input_rect, cancel_rect, confirm_rect) =
                    todo_inline_form_rects(layout.body_left);
                if point_in_rect(x_dips, y_dips, cancel_rect) {
                    return HitTarget::TodoCancelAdd;
                }
                if point_in_rect(x_dips, y_dips, confirm_rect) {
                    return HitTarget::TodoConfirmAdd;
                }
                if point_in_rect(x_dips, y_dips, input_rect) {
                    return HitTarget::TodoFormTitle;
                }
                let time_bar = D2D_RECT_F {
                    left: panel.left + 8.0,
                    top: panel.top + 38.5,
                    right: panel.right - 8.0,
                    bottom: panel.top + 60.5,
                };
                let daily_pill = D2D_RECT_F {
                    left: time_bar.right - 37.0,
                    top: time_bar.top + 3.0,
                    right: time_bar.right - 4.0,
                    bottom: time_bar.bottom - 3.0,
                };
                if point_in_rect(x_dips, y_dips, daily_pill) {
                    return HitTarget::TodoFormDaily;
                }
                if point_in_rect(x_dips, y_dips, time_bar) {
                    return HitTarget::TodoFormTime;
                }
            } else {
                let visible = todo_len.min(3);
                for idx in 0..visible {
                    let row = todo_card_rect_morphed(layout.body_left, idx, exp_t);
                    if point_in_rect(x_dips, y_dips, row) {
                        return HitTarget::TodoToggle(idx);
                    }
                }
                if todo_len > 0 {
                    let panel = todo_panel_rect_morphed(layout.body_left, exp_t);
                    if point_in_rect(x_dips, y_dips, panel) {
                        return HitTarget::TodoPanelEmpty;
                    }
                }
            }
        }
        NotchPage::Widgets => {
            let (left_r, right_r) = widget_card_rects(layout.body_left, layout.body_right);
            for (card_r, kind, card_hit) in [
                (left_r, widget_left, HitTarget::WidgetCardLeft),
                (right_r, widget_right, HitTarget::WidgetCardRight),
            ] {
                if point_in_rect(x_dips, y_dips, card_r) {
                    if kind == 0 {
                        let pad_x = 8.0;
                        let col_w = (card_r.right - card_r.left - pad_x * 2.0) / 7.0;
                        let row_cy = card_r.top + 50.0;
                        if (y_dips - row_cy).abs() <= 14.0 {
                            let rel_x = x_dips - (card_r.left + pad_x);
                            if rel_x >= 0.0 && rel_x <= col_w * 7.0 {
                                let d = (rel_x / col_w).floor().clamp(0.0, 6.0) as usize;
                                return HitTarget::WidgetWeekDay(d);
                            }
                        }
                    }
                    return card_hit;
                }
            }
        }
    }

    HitTarget::Body
}

fn music_volume_slider_rects(
    speaker_center: D2D_POINT_2F,
    open_t: f32,
) -> (D2D_RECT_F, D2D_RECT_F, D2D_RECT_F) {
    let t = open_t.clamp(0.0, 1.0);
    let full_w = 65.0_f32;
    let right = speaker_center.x - 15.0;
    let full_left = right - full_w;
    let cur_w = full_w * t;
    let anim_left = right - cur_w;
    let half_h = 3.0;
    let full_rect = D2D_RECT_F {
        left: full_left,
        top: speaker_center.y - half_h,
        right,
        bottom: speaker_center.y + half_h,
    };
    let anim_rect = D2D_RECT_F {
        left: anim_left,
        top: speaker_center.y - half_h,
        right,
        bottom: speaker_center.y + half_h,
    };
    let hit_rect = D2D_RECT_F {
        left: (anim_left - 2.0).min(right - 14.0),
        top: speaker_center.y - 11.0,
        right: speaker_center.x - 12.0,
        bottom: speaker_center.y + 11.0,
    };
    (full_rect, anim_rect, hit_rect)
}

fn pomodoro_controls(body_left: f32) -> (D2D_RECT_F, D2D_POINT_2F, D2D_RECT_F) {
    let toggle = D2D_RECT_F {
        left: body_left + 281.0,
        top: 55.0,
        right: body_left + 335.0,
        bottom: 73.0,
    };
    let reset_center = D2D_POINT_2F {
        x: body_left + 353.0,
        y: 64.0,
    };
    let switch = D2D_RECT_F {
        left: body_left + 33.0,
        top: 48.0,
        right: body_left + 150.0,
        bottom: 66.0,
    };
    (toggle, reset_center, switch)
}

fn water_controls(body_left: f32) -> (D2D_RECT_F, D2D_RECT_F, D2D_RECT_F) {
    let switch = D2D_RECT_F {
        left: body_left + 150.0,
        top: 57.0,
        right: body_left + 184.0,
        bottom: 77.0,
    };
    let drink = D2D_RECT_F {
        left: body_left + 33.0,
        top: 122.0,
        right: body_left + 104.0,
        bottom: 147.0,
    };
    let reset = D2D_RECT_F {
        left: body_left + 111.0,
        top: 122.0,
        right: body_left + 184.0,
        bottom: 147.0,
    };
    (switch, drink, reset)
}

fn shelf_bottom_button_rects(body_left: f32, body_right: f32) -> (D2D_RECT_F, D2D_RECT_F) {
    let select = D2D_RECT_F {
        left: body_left + 32.0,
        top: 138.0,
        right: body_left + 84.0,
        bottom: 159.0,
    };
    let clear = D2D_RECT_F {
        left: body_right - 92.0,
        top: 138.0,
        right: body_right - 31.0,
        bottom: 159.0,
    };
    (select, clear)
}

fn shelf_switcher_rects(body_right: f32) -> (D2D_RECT_F, D2D_RECT_F, D2D_RECT_F) {
    let pill = D2D_RECT_F {
        left: body_right - 95.0,
        top: 19.0,
        right: body_right - 31.0,
        bottom: 39.0,
    };
    let files = D2D_RECT_F {
        left: pill.left,
        top: pill.top,
        right: pill.left + 32.0,
        bottom: pill.bottom,
    };
    let clip = D2D_RECT_F {
        left: pill.left + 32.0,
        top: pill.top,
        right: pill.right,
        bottom: pill.bottom,
    };
    (pill, files, clip)
}

fn cb_card_rect(body_left: f32, idx: usize) -> D2D_RECT_F {
    let left = body_left + 32.0 + idx as f32 * 83.0;
    D2D_RECT_F {
        left,
        top: 53.0,
        right: left + 77.0,
        bottom: 134.0,
    }
}

fn shelf_card_rect(body_left: f32, idx: usize) -> D2D_RECT_F {
    let cx = body_left + 59.5 + idx as f32 * 68.5;
    D2D_RECT_F {
        left: cx - 28.0,
        top: 53.0,
        right: cx + 28.0,
        bottom: 134.0,
    }
}

fn todo_calendar_center(body_left: f32) -> D2D_POINT_2F {
    D2D_POINT_2F {
        x: body_left + 174.0,
        y: 72.5,
    }
}

fn todo_right_panel_rect(body_left: f32) -> D2D_RECT_F {
    D2D_RECT_F {
        left: body_left + 198.0,
        top: 50.0,
        right: body_left + 368.0,
        bottom: 147.0,
    }
}

#[inline]
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn todo_panel_rect_morphed(body_left: f32, exp_t: f32) -> D2D_RECT_F {
    let t = exp_t.clamp(0.0, 1.0);
    let r0 = todo_right_panel_rect(body_left);
    D2D_RECT_F {
        left: lerp(r0.left, body_left + 68.0, t),
        top: r0.top,
        right: r0.right,
        bottom: r0.bottom,
    }
}

#[allow(dead_code)]
fn todo_card_rect(body_left: f32, idx: usize) -> D2D_RECT_F {
    todo_card_rect_morphed(body_left, idx, 0.0)
}

fn todo_card_rect_morphed(body_left: f32, idx: usize, exp_t: f32) -> D2D_RECT_F {
    let panel = todo_panel_rect_morphed(body_left, exp_t);
    let y = panel.top + 6.0 + idx as f32 * 30.0;
    D2D_RECT_F {
        left: panel.left + 6.0,
        top: y,
        right: panel.right - 6.0,
        bottom: y + 26.5,
    }
}

fn todo_button_rects(body_left: f32) -> (D2D_RECT_F, D2D_RECT_F) {
    let add = D2D_RECT_F {
        left: body_left + 28.0,
        top: 118.0,
        right: body_left + 106.0,
        bottom: 146.0,
    };
    let edit = D2D_RECT_F {
        left: body_left + 112.0,
        top: 118.0,
        right: body_left + 190.0,
        bottom: 146.0,
    };
    (add, edit)
}

fn todo_button_rects_morphed(body_left: f32, exp_t: f32) -> (D2D_RECT_F, D2D_RECT_F) {
    let t = exp_t.clamp(0.0, 1.0);
    let (add0, edit0) = todo_button_rects(body_left);
    let add1 = D2D_RECT_F {
        left: body_left + 28.0,
        top: 62.0,
        right: body_left + 58.0,
        bottom: 92.0,
    };
    let edit1 = D2D_RECT_F {
        left: body_left + 28.0,
        top: 102.0,
        right: body_left + 58.0,
        bottom: 132.0,
    };
    (
        D2D_RECT_F {
            left: lerp(add0.left, add1.left, t),
            top: lerp(add0.top, add1.top, t),
            right: lerp(add0.right, add1.right, t),
            bottom: lerp(add0.bottom, add1.bottom, t),
        },
        D2D_RECT_F {
            left: lerp(edit0.left, edit1.left, t),
            top: lerp(edit0.top, edit1.top, t),
            right: lerp(edit0.right, edit1.right, t),
            bottom: lerp(edit0.bottom, edit1.bottom, t),
        },
    )
}

fn todo_inline_form_rects(body_left: f32) -> (D2D_RECT_F, D2D_RECT_F, D2D_RECT_F, D2D_RECT_F) {
    let panel = todo_right_panel_rect(body_left);
    let input = D2D_RECT_F {
        left: panel.left + 8.0,
        top: panel.top + 8.0,
        right: panel.right - 8.0,
        bottom: panel.top + 34.0,
    };
    let cancel = D2D_RECT_F {
        left: panel.left + 8.0,
        top: panel.bottom - 27.0,
        right: panel.left + 82.0,
        bottom: panel.bottom - 6.0,
    };
    let confirm = D2D_RECT_F {
        left: panel.left + 88.0,
        top: panel.bottom - 27.0,
        right: panel.right - 8.0,
        bottom: panel.bottom - 6.0,
    };
    (panel, input, cancel, confirm)
}

fn point_in_rect(x: f32, y: f32, r: D2D_RECT_F) -> bool {
    x >= r.left && x <= r.right && y >= r.top && y <= r.bottom
}

fn scale_rect(r: D2D_RECT_F, s: f32) -> D2D_RECT_F {
    if (s - 1.0).abs() <= 0.0005 {
        return r;
    }
    let cx = (r.left + r.right) * 0.5;
    let cy = (r.top + r.bottom) * 0.5;
    let hw = (r.right - r.left) * 0.5 * s;
    let hh = (r.bottom - r.top) * 0.5 * s;
    D2D_RECT_F {
        left: cx - hw,
        top: cy - hh,
        right: cx + hw,
        bottom: cy + hh,
    }
}

fn dist_sq(x: f32, y: f32, c: D2D_POINT_2F) -> f32 {
    let dx = x - c.x;
    let dy = y - c.y;
    dx * dx + dy * dy
}

/// D3D11 + DXGI SwapChain + Direct2D + DirectWrite renderer for the Eave Dynamic Island notch.
pub struct NotchRenderer {
    _d3d_device: ID3D11Device,
    swap_chain: IDXGISwapChain1,
    d2d_factory: ID2D1Factory1,
    d2d_context: ID2D1DeviceContext,
    _d2d_device: ID2D1Device,
    dwrite: IDWriteFactory,
    solid_brush: ID2D1SolidColorBrush,
    alt_brush: ID2D1SolidColorBrush,
    dashed_stroke: Option<ID2D1StrokeStyle>,
    dotted_stroke: Option<ID2D1StrokeStyle>,

    fmt_header_date: IDWriteTextFormat,
    fmt_header_time: IDWriteTextFormat,
    fmt_header_ampm: IDWriteTextFormat,
    fmt_page_title: IDWriteTextFormat,
    fmt_title: IDWriteTextFormat,
    fmt_artist: IDWriteTextFormat,
    fmt_music_artist: IDWriteTextFormat,
    #[allow(dead_code)]
    fmt_lyric: IDWriteTextFormat,
    fmt_closed_lyric: IDWriteTextFormat,
    fmt_lyric_line_dim: IDWriteTextFormat,
    fmt_lyric_line_active: IDWriteTextFormat,
    fmt_time_left: IDWriteTextFormat,
    fmt_time_right: IDWriteTextFormat,
    fmt_center_md: IDWriteTextFormat,
    fmt_center_sm: IDWriteTextFormat,
    fmt_timer_lg: IDWriteTextFormat,
    fmt_pomo_until: IDWriteTextFormat,
    fmt_label_xs: IDWriteTextFormat,
    fmt_label_xs_center: IDWriteTextFormat,
    fmt_pill_btn: IDWriteTextFormat,
    fmt_pill_sm: IDWriteTextFormat,
    fmt_card_name: IDWriteTextFormat,
    fmt_card_size: IDWriteTextFormat,
    fmt_clip_text: IDWriteTextFormat,
    fmt_clip_time: IDWriteTextFormat,
    fmt_todo_ratio: IDWriteTextFormat,
    fmt_gauge_pct: IDWriteTextFormat,
    fmt_osd_label: IDWriteTextFormat,
    fmt_osd_state: IDWriteTextFormat,
    fmt_wg_label: IDWriteTextFormat,
    fmt_wg_big: IDWriteTextFormat,
    fmt_wg_val: IDWriteTextFormat,
    fmt_wg_sub: IDWriteTextFormat,
    fmt_wg_time: IDWriteTextFormat,
    fmt_wg_day_hdr: IDWriteTextFormat,
    fmt_wg_day_num: IDWriteTextFormat,
    fmt_wg_pill: IDWriteTextFormat,
    fmt_ai_input: IDWriteTextFormat,
    fmt_ai_body: IDWriteTextFormat,
    fmt_ai_code: IDWriteTextFormat,
    fmt_ai_math: IDWriteTextFormat,

    max_w_dips: f32,
    max_h_dips: f32,
    dpi: f32,

    opaque_surface: Option<ID2D1Bitmap1>,
    opaque_brush: Option<ID2D1BitmapBrush1>,
    icon_atlas_2x: Option<ID2D1Bitmap1>,
    speaker_2x_bmp: Option<ID2D1Bitmap1>,
    cached_speaker_key: Option<(u16, u8)>,
    ambient_grad_bmp: Option<ID2D1Bitmap1>,
    ambient_grad_brush: Option<ID2D1BitmapBrush1>,
    cached_ambient_key: Option<(u32, u8, u8, u8, u8, u8)>,
    ambient_aurora_bmp: Option<ID2D1Bitmap1>,
    cached_aurora_key: Option<(u32, u8, u8, u8, u8, u8)>,
    clock_2x_bmp: Option<ID2D1Bitmap1>,
    cached_clock_key: Option<(String, String, u32)>,
    cached_clock_cell_w: f32,
    cached_hdr_dt: Option<(String, String, String, f32, f32, f32)>,
    swap_chain_target: Option<ID2D1Bitmap1>,
    cached_notch_geo: Option<([i32; 5], ID2D1PathGeometry)>,
    eave_mask_brush: Option<(u32, ID2D1Brush)>,
    eave_radial_brushes: [Option<ID2D1RadialGradientBrush>; 4],

    cached_art_revision: u64,
    cached_art_bitmap: Option<ID2D1Bitmap1>,
    prev_art_bitmap: Option<ID2D1Bitmap1>,
    shell_icon_cache: Vec<(u64, ID2D1Bitmap1)>,
    toast_icon_bmp: Option<(u64, ID2D1Bitmap1)>,
    ai_wave_bgra: Vec<u8>,
    ai_wave_bitmap: Option<ID2D1Bitmap1>,
}

const BAYER_8X8: [[u8; 8]; 8] = [
    [0, 48, 12, 60, 3, 51, 15, 63],
    [32, 16, 44, 28, 35, 19, 47, 31],
    [8, 56, 4, 52, 11, 59, 7, 55],
    [40, 24, 36, 20, 43, 27, 39, 23],
    [2, 50, 14, 62, 1, 49, 13, 61],
    [34, 18, 46, 30, 33, 17, 45, 29],
    [10, 58, 6, 54, 9, 57, 5, 53],
    [42, 26, 38, 22, 41, 25, 37, 21],
];

const ATLAS_COLS: usize = 5;
const ATLAS_ROWS: usize = 2;
const ATLAS_CELL_DIPS: f32 = 32.0;
const ATLAS_SLOT_PLAY: usize = 0;
const ATLAS_SLOT_PAUSE: usize = 1;
const ATLAS_SLOT_PREV: usize = 2;
const ATLAS_SLOT_NEXT: usize = 3;
const ATLAS_SLOT_LYRICS_STROKED: usize = 4;
const ATLAS_SLOT_LYRICS_FILLED: usize = 5;
const ATLAS_SLOT_MUSIC_NOTE: usize = 6;
const ATLAS_SLOT_CAPS_ON: usize = 7;
const ATLAS_SLOT_CAPS_OFF: usize = 8;
const ATLAS_SLOT_BELL: usize = 9;

const ATLAS_BAKE_SIZES: [f32; 10] = [
    20.0,
    20.0,
    20.0,
    20.0,
    20.0,
    20.0,
    20.0,
    20.0,
    20.0,
    18.0,
];

impl NotchRenderer {
    /// Creates the D3D11 device, stationary DXGI swapchain, Direct2D context, and DirectWrite formats.
    pub fn new(max_w_dips: f32, max_h_dips: f32, dpi: f32) -> Result<Self> {
        let (d3d_device, dxgi_factory) = create_d3d11_and_factory()?;

        let scale = (dpi / 96.0).max(0.5);
        let w_px = (((max_w_dips * scale).ceil().max(2.0) as u32) + 1) & !1;
        let h_px = (max_h_dips * scale).ceil().max(2.0) as u32;

        let sc_desc = DXGI_SWAP_CHAIN_DESC1 {
            Width: w_px,
            Height: h_px,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            Stereo: BOOL(0),
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 2,
            Scaling: DXGI_SCALING_STRETCH,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
            AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
            Flags: 0,
        };
        let swap_chain = unsafe {
            dxgi_factory.CreateSwapChainForComposition(&d3d_device, &sc_desc, None)?
        };

        let d2d_factory: ID2D1Factory1 =
            unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)? };
        let dxgi_device: IDXGIDevice = d3d_device.cast()?;
        let d2d_device = unsafe { d2d_factory.CreateDevice(&dxgi_device)? };
        let d2d_context =
            unsafe { d2d_device.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)? };

        unsafe {
            d2d_context.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
        }
        let _ = create_swap_chain_target(&d2d_context, &swap_chain, dpi)?;

        let solid_brush = unsafe {
            d2d_context.CreateSolidColorBrush(
                &D2D1_COLOR_F {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: 1.0,
                },
                None,
            )?
        };
        let alt_brush = unsafe {
            d2d_context.CreateSolidColorBrush(
                &D2D1_COLOR_F {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: 1.0,
                },
                None,
            )?
        };

        let dash_props = D2D1_STROKE_STYLE_PROPERTIES {
            startCap: D2D1_CAP_STYLE_ROUND,
            endCap: D2D1_CAP_STYLE_ROUND,
            dashCap: D2D1_CAP_STYLE_ROUND,
            lineJoin: D2D1_LINE_JOIN_ROUND,
            miterLimit: 10.0,
            dashStyle: D2D1_DASH_STYLE_CUSTOM,
            dashOffset: 0.0,
        };
        let dashed_stroke = unsafe {
            ID2D1Factory::CreateStrokeStyle(
                &d2d_factory,
                &dash_props,
                Some(&[3.5_f32, 3.5_f32]),
            )
            .ok()
        };
        let dotted_stroke = unsafe {
            ID2D1Factory::CreateStrokeStyle(
                &d2d_factory,
                &dash_props,
                Some(&[1.4_f32, 2.8_f32]),
            )
            .ok()
        };

        let dwrite: IDWriteFactory = unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)? };
        if let Ok(base_rp) = unsafe { dwrite.CreateRenderingParams() } {
            let gamma = unsafe { base_rp.GetGamma() };
            let contrast = unsafe { base_rp.GetEnhancedContrast() }.max(0.5);
            let ct_level = unsafe { base_rp.GetClearTypeLevel() }.max(1.0);
            let geom = unsafe { base_rp.GetPixelGeometry() };
            if let Ok(custom_rp) = unsafe {
                let rp: IDWriteRenderingParams = dwrite.CreateCustomRenderingParams(
                    gamma,
                    contrast,
                    ct_level,
                    geom,
                    DWRITE_RENDERING_MODE_NATURAL_SYMMETRIC,
                )?;
                Ok::<IDWriteRenderingParams, windows::core::Error>(rp)
            } {
                unsafe {
                    d2d_context.SetTextRenderingParams(&custom_rp);
                }
            }
        }
        let font_family = pick_ui_font_family(&dwrite);

        let fmt_header_date = create_text_format(
            &dwrite,
            &font_family,
            12.5,
            DWRITE_FONT_WEIGHT_NORMAL,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_TEXT_ALIGNMENT_LEADING,
            false,
        )?;
        let fmt_header_time = create_text_format(
            &dwrite,
            &font_family,
            12.5,
            DWRITE_FONT_WEIGHT_BOLD,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_TEXT_ALIGNMENT_LEADING,
            false,
        )?;
        let fmt_header_ampm = create_text_format(
            &dwrite,
            &font_family,
            10.5,
            DWRITE_FONT_WEIGHT_SEMI_BOLD,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_TEXT_ALIGNMENT_LEADING,
            false,
        )?;
        let fmt_page_title = create_text_format(
            &dwrite,
            &font_family,
            14.2,
            DWRITE_FONT_WEIGHT_SEMI_BOLD,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_TEXT_ALIGNMENT_LEADING,
            true,
        )?;
        let fmt_title = create_text_format(
            &dwrite,
            &font_family,
            15.5,
            DWRITE_FONT_WEIGHT_SEMI_BOLD,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_TEXT_ALIGNMENT_LEADING,
            true,
        )?;
        let fmt_artist = create_text_format(
            &dwrite,
            &font_family,
            12.0,
            DWRITE_FONT_WEIGHT_NORMAL,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_TEXT_ALIGNMENT_LEADING,
            true,
        )?;
        let fmt_music_artist = create_text_format(
            &dwrite,
            &font_family,
            12.5,
            DWRITE_FONT_WEIGHT_NORMAL,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_TEXT_ALIGNMENT_LEADING,
            true,
        )?;
        let fmt_lyric = create_text_format(
            &dwrite,
            &font_family,
            11.5,
            DWRITE_FONT_WEIGHT_MEDIUM,
            DWRITE_FONT_STYLE_ITALIC,
            DWRITE_TEXT_ALIGNMENT_LEADING,
            true,
        )?;
        let fmt_closed_lyric = create_text_format(
            &dwrite,
            &font_family,
            12.8,
            DWRITE_FONT_WEIGHT_MEDIUM,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_TEXT_ALIGNMENT_CENTER,
            true,
        )?;
        let fmt_lyric_line_dim = create_text_format(
            &dwrite,
            &font_family,
            15.2,
            DWRITE_FONT_WEIGHT_BOLD,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_TEXT_ALIGNMENT_LEADING,
            false,
        )?;
        let fmt_lyric_line_active = create_text_format(
            &dwrite,
            &font_family,
            15.2,
            DWRITE_FONT_WEIGHT_BOLD,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_TEXT_ALIGNMENT_LEADING,
            false,
        )?;
        unsafe {
            fmt_lyric_line_dim.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_NEAR)?;
            fmt_lyric_line_dim.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP)?;
            fmt_lyric_line_active.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_NEAR)?;
            fmt_lyric_line_active.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP)?;
        }
        let fmt_time_left = create_text_format(
            &dwrite,
            &font_family,
            13.0,
            DWRITE_FONT_WEIGHT_NORMAL,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_TEXT_ALIGNMENT_LEADING,
            false,
        )?;
        let fmt_time_right = create_text_format(
            &dwrite,
            &font_family,
            13.0,
            DWRITE_FONT_WEIGHT_NORMAL,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_TEXT_ALIGNMENT_TRAILING,
            false,
        )?;
        let fmt_center_md = create_text_format(
            &dwrite,
            &font_family,
            14.0,
            DWRITE_FONT_WEIGHT_MEDIUM,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_TEXT_ALIGNMENT_CENTER,
            true,
        )?;
        let fmt_center_sm = create_text_format(
            &dwrite,
            &font_family,
            11.5,
            DWRITE_FONT_WEIGHT_NORMAL,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_TEXT_ALIGNMENT_CENTER,
            true,
        )?;
        let fmt_timer_lg = create_text_format(
            &dwrite,
            &font_family,
            31.5,
            DWRITE_FONT_WEIGHT_BOLD,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_TEXT_ALIGNMENT_LEADING,
            false,
        )?;
        let fmt_pomo_until = create_text_format(
            &dwrite,
            &font_family,
            13.0,
            DWRITE_FONT_WEIGHT_SEMI_BOLD,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_TEXT_ALIGNMENT_LEADING,
            false,
        )?;
        let fmt_label_xs = create_text_format(
            &dwrite,
            &font_family,
            10.0,
            DWRITE_FONT_WEIGHT_NORMAL,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_TEXT_ALIGNMENT_LEADING,
            false,
        )?;
        let fmt_label_xs_center = create_text_format(
            &dwrite,
            &font_family,
            9.2,
            DWRITE_FONT_WEIGHT_SEMI_BOLD,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_TEXT_ALIGNMENT_CENTER,
            false,
        )?;
        let fmt_pill_btn = create_text_format(
            &dwrite,
            &font_family,
            11.0,
            DWRITE_FONT_WEIGHT_SEMI_BOLD,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_TEXT_ALIGNMENT_CENTER,
            false,
        )?;
        let fmt_pill_sm = create_text_format(
            &dwrite,
            &font_family,
            9.5,
            DWRITE_FONT_WEIGHT_SEMI_BOLD,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_TEXT_ALIGNMENT_CENTER,
            false,
        )?;
        let fmt_card_name = create_text_format(
            &dwrite,
            &font_family,
            10.0,
            DWRITE_FONT_WEIGHT_SEMI_BOLD,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_TEXT_ALIGNMENT_CENTER,
            true,
        )?;
        let fmt_card_size = create_text_format(
            &dwrite,
            &font_family,
            8.8,
            DWRITE_FONT_WEIGHT_NORMAL,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_TEXT_ALIGNMENT_CENTER,
            true,
        )?;
        let fmt_clip_text = create_text_format(
            &dwrite,
            &font_family,
            10.0,
            DWRITE_FONT_WEIGHT_NORMAL,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_TEXT_ALIGNMENT_LEADING,
            true,
        )?;
        unsafe {
            fmt_clip_text.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_NEAR)?;
            fmt_clip_text.SetWordWrapping(DWRITE_WORD_WRAPPING_EMERGENCY_BREAK)?;
        }
        let fmt_clip_time = create_text_format(
            &dwrite,
            &font_family,
            8.5,
            DWRITE_FONT_WEIGHT_NORMAL,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_TEXT_ALIGNMENT_LEADING,
            true,
        )?;
        let fmt_todo_ratio = create_text_format(
            &dwrite,
            &font_family,
            32.5,
            DWRITE_FONT_WEIGHT_BOLD,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_TEXT_ALIGNMENT_LEADING,
            false,
        )?;
        let fmt_gauge_pct = create_text_format(
            &dwrite,
            &font_family,
            23.0,
            DWRITE_FONT_WEIGHT_BOLD,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_TEXT_ALIGNMENT_CENTER,
            false,
        )?;
        let fmt_osd_label = create_text_format(
            &dwrite,
            &font_family,
            13.0,
            DWRITE_FONT_WEIGHT_SEMI_BOLD,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_TEXT_ALIGNMENT_LEADING,
            true,
        )?;
        let fmt_osd_state = create_text_format(
            &dwrite,
            &font_family,
            13.0,
            DWRITE_FONT_WEIGHT_SEMI_BOLD,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_TEXT_ALIGNMENT_TRAILING,
            false,
        )?;
        let fmt_wg_label = create_text_format(
            &dwrite,
            &font_family,
            9.5,
            DWRITE_FONT_WEIGHT_BOLD,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_TEXT_ALIGNMENT_LEADING,
            true,
        )?;
        let fmt_wg_big = create_text_format(
            &dwrite,
            &font_family,
            21.0,
            DWRITE_FONT_WEIGHT_BOLD,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_TEXT_ALIGNMENT_LEADING,
            true,
        )?;
        let fmt_wg_val = create_text_format(
            &dwrite,
            &font_family,
            11.8,
            DWRITE_FONT_WEIGHT_SEMI_BOLD,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_TEXT_ALIGNMENT_LEADING,
            true,
        )?;
        let fmt_wg_sub = create_text_format(
            &dwrite,
            &font_family,
            10.5,
            DWRITE_FONT_WEIGHT_NORMAL,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_TEXT_ALIGNMENT_LEADING,
            true,
        )?;
        let fmt_wg_time = create_text_format(
            &dwrite,
            &font_family,
            15.0,
            DWRITE_FONT_WEIGHT_BOLD,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_TEXT_ALIGNMENT_TRAILING,
            false,
        )?;
        let fmt_wg_day_hdr = create_text_format(
            &dwrite,
            &font_family,
            9.6,
            DWRITE_FONT_WEIGHT_SEMI_BOLD,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_TEXT_ALIGNMENT_CENTER,
            false,
        )?;
        let fmt_wg_day_num = create_text_format(
            &dwrite,
            &font_family,
            11.2,
            DWRITE_FONT_WEIGHT_BOLD,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_TEXT_ALIGNMENT_CENTER,
            false,
        )?;
        let fmt_wg_pill = create_text_format(
            &dwrite,
            &font_family,
            8.8,
            DWRITE_FONT_WEIGHT_BOLD,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_TEXT_ALIGNMENT_CENTER,
            false,
        )?;
        let fmt_ai_input = create_text_format(
            &dwrite,
            &font_family,
            15.6,
            DWRITE_FONT_WEIGHT_NORMAL,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_TEXT_ALIGNMENT_LEADING,
            false,
        )?;
        let fmt_ai_body = create_text_format(
            &dwrite,
            &font_family,
            14.6,
            DWRITE_FONT_WEIGHT_NORMAL,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_TEXT_ALIGNMENT_LEADING,
            false,
        )?;
        unsafe {
            let _ = fmt_ai_body.SetWordWrapping(DWRITE_WORD_WRAPPING_EMERGENCY_BREAK);
            let _ = fmt_ai_body.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_NEAR);
            let _ = fmt_ai_body.SetLineSpacing(DWRITE_LINE_SPACING_METHOD_UNIFORM, 22.5, 17.8);
        }
        let code_family = HSTRING::from("Consolas");
        let fmt_ai_code = create_text_format(
            &dwrite,
            &code_family,
            12.5,
            DWRITE_FONT_WEIGHT_NORMAL,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_TEXT_ALIGNMENT_LEADING,
            false,
        )?;
        unsafe {
            let _ = fmt_ai_code.SetWordWrapping(DWRITE_WORD_WRAPPING_EMERGENCY_BREAK);
            let _ = fmt_ai_code.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_NEAR);
            let _ = fmt_ai_code.SetLineSpacing(DWRITE_LINE_SPACING_METHOD_UNIFORM, 18.5, 14.6);
        }
        let math_family = HSTRING::from("Cambria Math");
        let fmt_ai_math = create_text_format(
            &dwrite,
            &math_family,
            16.0,
            DWRITE_FONT_WEIGHT_NORMAL,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_TEXT_ALIGNMENT_CENTER,
            false,
        )?;
        unsafe {
            let _ = fmt_ai_math.SetWordWrapping(DWRITE_WORD_WRAPPING_EMERGENCY_BREAK);
            let _ = fmt_ai_math.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_NEAR);
            let _ = fmt_ai_math.SetLineSpacing(DWRITE_LINE_SPACING_METHOD_UNIFORM, 24.0, 19.0);
        }

        let (opaque_surface, opaque_brush) =
            match create_opaque_surface(&d2d_context, w_px, h_px, dpi) {
                Some((bmp, br)) => (Some(bmp), Some(br)),
                None => (None, None),
            };
        let icon_atlas_2x = create_and_bake_icon_atlas_2x(
            &d2d_context,
            &solid_brush,
            &alt_brush,
            dpi,
        );
        let speaker_2x_bmp = create_speaker_2x_bmp(&d2d_context, dpi);
        let swap_chain_target = Some(create_swap_chain_target(&d2d_context, &swap_chain, dpi)?);
        let eave_radial_brushes = create_eave_radial_brushes(&d2d_context);

        Ok(Self {
            _d3d_device: d3d_device,
            swap_chain,
            d2d_factory,
            d2d_context,
            _d2d_device: d2d_device,
            dwrite,
            solid_brush,
            alt_brush,
            dashed_stroke,
            dotted_stroke,
            fmt_header_date,
            fmt_header_time,
            fmt_header_ampm,
            fmt_page_title,
            fmt_title,
            fmt_artist,
            fmt_music_artist,
            fmt_lyric,
            fmt_closed_lyric,
            fmt_lyric_line_dim,
            fmt_lyric_line_active,
            fmt_time_left,
            fmt_time_right,
            fmt_center_md,
            fmt_center_sm,
            fmt_timer_lg,
            fmt_pomo_until,
            fmt_label_xs,
            fmt_label_xs_center,
            fmt_pill_btn,
            fmt_pill_sm,
            fmt_card_name,
            fmt_card_size,
            fmt_clip_text,
            fmt_clip_time,
            fmt_todo_ratio,
            fmt_gauge_pct,
            fmt_osd_label,
            fmt_osd_state,
            fmt_wg_label,
            fmt_wg_big,
            fmt_wg_val,
            fmt_wg_sub,
            fmt_wg_time,
            fmt_wg_day_hdr,
            fmt_wg_day_num,
            fmt_wg_pill,
            fmt_ai_input,
            fmt_ai_body,
            fmt_ai_code,
            fmt_ai_math,
            max_w_dips,
            max_h_dips,
            dpi,
            opaque_surface,
            opaque_brush,
            icon_atlas_2x,
            speaker_2x_bmp,
            cached_speaker_key: None,
            ambient_grad_bmp: None,
            ambient_grad_brush: None,
            cached_ambient_key: None,
            ambient_aurora_bmp: None,
            cached_aurora_key: None,
            clock_2x_bmp: None,
            cached_clock_key: None,
            cached_clock_cell_w: 0.0,
            cached_hdr_dt: None,
            swap_chain_target,
            cached_notch_geo: None,
            eave_mask_brush: None,
            eave_radial_brushes,
            cached_art_revision: 0,
            cached_art_bitmap: None,
            prev_art_bitmap: None,
            shell_icon_cache: Vec::new(),
            toast_icon_bmp: None,
            ai_wave_bgra: vec![0u8; (crate::ai::AI_WAVE_W * crate::ai::AI_WAVE_H * 4) as usize],
            ai_wave_bitmap: None,
        })
    }

    pub fn swap_chain(&self) -> &IDXGISwapChain1 {
        &self.swap_chain
    }

    pub fn resize(&mut self, max_w_dips: f32, max_h_dips: f32, dpi: f32) -> Result<()> {
        let scale = (dpi / 96.0).max(0.5);
        let prev_scale = (self.dpi / 96.0).max(0.5);
        let old_px = (
            (((self.max_w_dips * prev_scale).ceil().max(2.0) as u32) + 1) & !1,
            (self.max_h_dips * prev_scale).ceil().max(2.0) as u32,
        );
        let new_px = (
            (((max_w_dips * scale).ceil().max(2.0) as u32) + 1) & !1,
            (max_h_dips * scale).ceil().max(2.0) as u32,
        );
        let dpi_changed = (self.dpi - dpi).abs() > 0.01;

        self.max_w_dips = max_w_dips;
        self.max_h_dips = max_h_dips;
        self.dpi = dpi;

        if old_px != new_px {
            unsafe {
                self.d2d_context.SetTarget(None);
                self.swap_chain_target = None;
                self.swap_chain.ResizeBuffers(
                    2,
                    new_px.0,
                    new_px.1,
                    DXGI_FORMAT_B8G8R8A8_UNORM,
                    DXGI_SWAP_CHAIN_FLAG(0),
                )?;
            }
            self.cached_notch_geo = None;
        }
        if old_px != new_px || dpi_changed {
            let (op_s, op_b) = match create_opaque_surface(&self.d2d_context, new_px.0, new_px.1, dpi) {
                Some((bmp, br)) => (Some(bmp), Some(br)),
                None => (None, None),
            };
            self.opaque_surface = op_s;
            self.opaque_brush = op_b;
        }
        if dpi_changed {
            self.icon_atlas_2x = create_and_bake_icon_atlas_2x(
                &self.d2d_context,
                &self.solid_brush,
                &self.alt_brush,
                dpi,
            );
            self.speaker_2x_bmp = create_speaker_2x_bmp(&self.d2d_context, dpi);
            self.cached_speaker_key = None;
            self.ambient_grad_bmp = None;
            self.ambient_grad_brush = None;
            self.cached_ambient_key = None;
            self.clock_2x_bmp = None;
            self.cached_clock_key = None;
            self.cached_hdr_dt = None;
            self.eave_mask_brush = None;
        }

        self.swap_chain_target = Some(create_swap_chain_target(
            &self.d2d_context,
            &self.swap_chain,
            dpi,
        )?);
        Ok(())
    }

    pub fn render(
        &mut self,
        cfg: &Config,
        springs: &NotchSprings,
        state: &EaveViewState<'_>,
        hover_target: HitTarget,
    ) -> Result<()> {
        self.sync_album_art(
            state.media.thumbnail,
            springs.collapsed_art_mix.value() > 0.01,
        );
        self.sync_shelf_icons(state.shelf);
        self.sync_toast_icon(state.widgets.active_toast.as_ref());
        self.sync_dynamic_icons(springs, state);
        self.sync_clock_bitmap(cfg);
        if springs.mascot_idle_presence.value() > 0.01 {
            self.sync_eave_mask_brush(springs.height.value.max(24.0));
        }
        self.sync_ai_wave(springs);

        let base_w = springs.width.value.max(60.0);
        let expand = springs.expand.value().clamp(0.0, 1.0);
        let vol_bounce = if state.collapsed_osd == CollapsedOsdKind::Volume {
            springs.osd_vol_bounce() * springs.osd_open() * (1.0 - expand)
        } else {
            0.0
        };
        let cur_w = (base_w + vol_bounce.abs()).max(60.0);
        let cur_h = (springs.height.value - (vol_bounce.abs() * 0.07).min(1.0)).max(22.0);
        let ear_r = springs.ear_radius.value.clamp(3.0, cur_w * 0.22);
        let body_w = (cur_w - ear_r * 2.0).max(24.0);
        let bottom_r = springs.radius.value.clamp(4.0, body_w.min(cur_h) * 0.48);
        self.sync_ambient_gradient(springs, state, cur_h, expand);
        let canvas_cx = (self.max_w_dips * 0.5).round();
        let pill_left =
            (canvas_cx - base_w * 0.5) + if vol_bounce < 0.0 { vol_bounce } else { 0.0 };
        let geo_key = [
            (pill_left * 20.0).round() as i32,
            (cur_w * 20.0).round() as i32,
            (cur_h * 20.0).round() as i32,
            (ear_r * 20.0).round() as i32,
            (bottom_r * 20.0).round() as i32,
        ];
        if self.cached_notch_geo.as_ref().map(|(k, _)| *k) != Some(geo_key) {
            self.cached_notch_geo = create_notch_geometry(
                &self.d2d_factory,
                pill_left,
                cur_w,
                cur_h,
                ear_r,
                bottom_r,
            )
            .map(|g| (geo_key, g));
        }
        let notch_geo = self.cached_notch_geo.as_ref().map(|(_, g)| g.clone());

        if springs.mascot_idle_presence.value() > 0.01 {
            self.sync_eave_mask_brush(cur_h.max(24.0));
        }

        let simple_collapsed = expand < 0.01
            && springs.lyrics_mode.value() < 0.01
            && springs.osd_open() < 0.01
            && springs.ai_active.value() < 0.01
            && springs.ai_phase == crate::ai::AiPhase::Idle;
        let use_opaque_cleartype = cfg.glass_tint[3] == 255
            && !simple_collapsed
            && self.opaque_surface.is_some()
            && self.opaque_brush.is_some();

        let reveal = springs.notch_reveal.value().max(0.0);
        let has_slide = (reveal - 1.0).abs() > 0.001;
        let (sx, sy, y_slide) = if reveal < 1.0 {
            let t = reveal;
            (
                1.0 - 0.06 * (1.0 - t),
                0.90 + 0.10 * t,
                -(1.0 - t) * (cur_h + 16.0),
            )
        } else {
            let over = (reveal - 1.0).min(0.24);
            (1.0 - over * 0.28, 1.0 + over * 0.78, 0.0)
        };
        let reveal_xf = Matrix3x2 {
            M11: sx,
            M12: 0.0,
            M21: 0.0,
            M22: sy,
            M31: canvas_cx * (1.0 - sx),
            M32: y_slide,
        };

        if self.swap_chain_target.is_none() {
            self.bind_cached_swap_chain_target()?;
        }

        unsafe {
            if reveal <= 0.004 {
                self.bind_cached_swap_chain_target()?;
                self.d2d_context.BeginDraw();
                self.d2d_context.Clear(Some(&D2D1_COLOR_F {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                    a: 0.0,
                }));
                self.d2d_context.EndDraw(None, None)?;
                self.swap_chain.Present(1, DXGI_PRESENT(0)).ok()?;
                return Ok(());
            }

            if use_opaque_cleartype {
                if let (Some(opaque_surf), Some(opaque_brush), Some(sc_target)) = (
                    &self.opaque_surface,
                    &self.opaque_brush,
                    &self.swap_chain_target,
                ) {
                    self.d2d_context.SetTarget(opaque_surf);
                    self.d2d_context.SetDpi(self.dpi, self.dpi);
                    self.d2d_context
                        .SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_CLEARTYPE);
                    self.d2d_context.BeginDraw();
                    self.d2d_context.Clear(Some(&D2D1_COLOR_F {
                        r: 0.0,
                        g: 0.0,
                        b: 0.0,
                        a: 1.0,
                    }));
                    if let Some(geo) = &notch_geo {
                        self.draw_notch_interior(
                            cfg, springs, state, hover_target, cur_w, cur_h, ear_r, expand,
                            canvas_cx, pill_left, geo, true,
                        );
                    }
                    self.d2d_context.EndDraw(None, None)?;

                    self.d2d_context.SetTarget(sc_target);
                    self.d2d_context.SetDpi(self.dpi, self.dpi);
                    self.d2d_context.BeginDraw();
                    self.d2d_context.Clear(Some(&D2D1_COLOR_F {
                        r: 0.0,
                        g: 0.0,
                        b: 0.0,
                        a: 0.0,
                    }));
                    if has_slide {
                        self.d2d_context.SetTransform(&reveal_xf);
                    }
                    self.draw_drop_shadow(pill_left, cur_w, cur_h, ear_r, bottom_r, expand);
                    if let Some(geo) = &notch_geo {
                        self.d2d_context.FillGeometry(geo, opaque_brush, None);
                        self.draw_dark_backdrop_highlight(geo, cur_h, springs.dark_bg_border());
                    }
                    if springs.ai_card_open.value() > 0.01 {
                        self.draw_ai_response_card(springs, state, cur_h, canvas_cx);
                    }
                    if has_slide {
                        self.d2d_context.SetTransform(&Matrix3x2::identity());
                    }
                    self.d2d_context.EndDraw(None, None)?;
                    self.swap_chain.Present(1, DXGI_PRESENT(0)).ok()?;
                    return Ok(());
                }
            }

            self.bind_cached_swap_chain_target()?;
            self.d2d_context
                .SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            self.d2d_context.BeginDraw();
            self.d2d_context.Clear(Some(&D2D1_COLOR_F {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.0,
            }));
            if has_slide {
                self.d2d_context.SetTransform(&reveal_xf);
            }
            self.draw_scene_translucent(
                cfg,
                springs,
                state,
                hover_target,
                cur_w,
                cur_h,
                ear_r,
                bottom_r,
                expand,
                canvas_cx,
                pill_left,
                notch_geo.as_ref(),
            )?;
            if springs.ai_card_open.value() > 0.01 {
                self.draw_ai_response_card(springs, state, cur_h, canvas_cx);
            }
            if has_slide {
                self.d2d_context.SetTransform(&Matrix3x2::identity());
            }
            self.d2d_context.EndDraw(None, None)?;
            self.swap_chain.Present(1, DXGI_PRESENT(0)).ok()?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn draw_drop_shadow(
        &self,
        pill_left: f32,
        cur_w: f32,
        cur_h: f32,
        ear_r: f32,
        bottom_r: f32,
        expand: f32,
    ) {
        self.d2d_context.PushAxisAlignedClip(
            &D2D_RECT_F {
                left: 0.0,
                top: 0.0,
                right: self.max_w_dips,
                bottom: self.max_h_dips,
            },
            D2D1_ANTIALIAS_MODE_ALIASED,
        );
        for i in 0..10 {
            let t = i as f32 / 10.0;
            let spread = (3.0 + 36.0 * t) * (0.45 + 0.55 * expand);
            let down = (1.0 - t) + 5.0 * t;
            let layer_alpha = (1.0 - t).powf(1.4) * (0.032 + 0.048 * expand);
            let shadow_rrect = D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F {
                    left: pill_left + ear_r - spread,
                    top: 0.0,
                    right: pill_left + cur_w - ear_r + spread,
                    bottom: cur_h + spread + down,
                },
                radiusX: bottom_r + spread,
                radiusY: bottom_r + spread,
            };
            self.set_color(0.0, 0.0, 0.0, layer_alpha);
            self.d2d_context
                .FillRoundedRectangle(&shadow_rrect, &self.solid_brush);
        }
        self.d2d_context.PopAxisAlignedClip();
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn draw_notch_interior(
        &self,
        cfg: &Config,
        springs: &NotchSprings,
        state: &EaveViewState<'_>,
        _hover_target: HitTarget,
        cur_w: f32,
        cur_h: f32,
        ear_r: f32,
        expand: f32,
        canvas_cx: f32,
        pill_left: f32,
        geo: &ID2D1PathGeometry,
        cleartype_active: bool,
    ) {
        if state.page == NotchPage::Music && springs.lyrics_mode.value() > 0.01 {
            self.draw_song_ambient_glow(springs, canvas_cx, pill_left, cur_w, cur_h, expand);
        }

        let ai_active = springs.ai_active.value().clamp(0.0, 1.0);
        let ai_interior_a = springs
            .ai_input_open
            .value()
            .max(springs.ai_wave_open.value())
            .clamp(0.0, 1.0);
        let non_ai_a = (1.0 - ai_active.max(ai_interior_a) * 1.45).clamp(0.0, 1.0);

        let collapsed_alpha = (1.0 - expand * 2.4).clamp(0.0, 1.0) * non_ai_a;
        if collapsed_alpha > 0.01 {
            self.draw_collapsed_notch(
                cfg,
                springs,
                state,
                canvas_cx,
                pill_left,
                cur_w,
                cur_h,
                ear_r,
                collapsed_alpha,
            );
        }

        let expanded_alpha = ((expand - 0.16) / 0.84).clamp(0.0, 1.0) * non_ai_a;
        if expanded_alpha > 0.01 {
            let bloom_s = (0.86 + 0.14 * springs.expand_bloom.value).clamp(0.86, 1.06);
            if (bloom_s - 1.0).abs() > 0.001 {
                let bloom_xf = Matrix3x2 {
                    M11: bloom_s,
                    M12: 0.0,
                    M21: 0.0,
                    M22: bloom_s,
                    M31: canvas_cx * (1.0 - bloom_s),
                    M32: 0.0,
                };
                self.d2d_context.SetTransform(&bloom_xf);
            }
            self.draw_expanded_notch(
                cfg,
                springs,
                state,
                canvas_cx,
                expanded_alpha,
                geo,
                cleartype_active,
            );
            if (bloom_s - 1.0).abs() > 0.001 {
                self.d2d_context.SetTransform(&Matrix3x2::identity());
            }
        }

        if non_ai_a > 0.01 {
            self.draw_shared_clock_header(
                cfg, springs, state, canvas_cx, pill_left, cur_w, ear_r, expand, non_ai_a,
            );
        }

        if ai_interior_a > 0.01 {
            self.draw_ai_notch_layers(springs, state, pill_left, cur_w, cur_h, ear_r);
        }
    }

    fn sync_ai_wave(&mut self, springs: &NotchSprings) {
        if springs.ai_wave_open.value() <= 0.01 {
            return;
        }
        crate::ai::render_siri_wave_frame(
            &mut self.ai_wave_bgra,
            crate::ai::AI_WAVE_W,
            crate::ai::AI_WAVE_H,
            springs.ai_wave_phase(),
            springs.ai_wave_energy.value().clamp(0.0, 1.0),
        );
        let pitch = crate::ai::AI_WAVE_W * 4;
        if let Some(bmp) = &self.ai_wave_bitmap {
            unsafe {
                let _ = bmp.CopyFromMemory(
                    None,
                    self.ai_wave_bgra.as_ptr() as *const std::ffi::c_void,
                    pitch,
                );
            }
            return;
        }
        let props = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            bitmapOptions: Default::default(),
            ..Default::default()
        };
        let size = D2D_SIZE_U {
            width: crate::ai::AI_WAVE_W,
            height: crate::ai::AI_WAVE_H,
        };
        self.ai_wave_bitmap = unsafe {
            self.d2d_context
                .CreateBitmap(
                    size,
                    Some(self.ai_wave_bgra.as_ptr() as *const std::ffi::c_void),
                    pitch,
                    &props,
                )
                .ok()
        };
    }

    unsafe fn build_md_text_layout(
        &self,
        rich: &crate::markdown::MdRichText,
        max_w: f32,
        font_size: f32,
        default_bold: bool,
        default_italic: bool,
    ) -> Option<IDWriteTextLayout> {
        let display = if rich.plain.is_empty() {
            " "
        } else {
            rich.plain.as_str()
        };
        let wide: Vec<u16> = display.encode_utf16().collect();
        let layout = self
            .dwrite
            .CreateTextLayout(&wide, &self.fmt_ai_body, max_w.max(32.0), 4000.0)
            .ok()?;
        let full_range = DWRITE_TEXT_RANGE {
            startPosition: 0,
            length: wide.len() as u32,
        };
        if (font_size - 14.6).abs() > 0.05 {
            let _ = layout.SetFontSize(font_size, full_range);
        }
        if default_bold {
            let _ = layout.SetFontWeight(DWRITE_FONT_WEIGHT_BOLD, full_range);
        }
        if default_italic {
            let _ = layout.SetFontStyle(DWRITE_FONT_STYLE_ITALIC, full_range);
        }
        let mono_family = HSTRING::from("Consolas");
        let math_family = HSTRING::from("Cambria Math");
        for sp in &rich.spans {
            if sp.len_u16 == 0 {
                continue;
            }
            let r = DWRITE_TEXT_RANGE {
                startPosition: sp.start_u16,
                length: sp.len_u16,
            };
            if sp.bold {
                let _ = layout.SetFontWeight(DWRITE_FONT_WEIGHT_BOLD, r);
            }
            if sp.italic {
                let _ = layout.SetFontStyle(DWRITE_FONT_STYLE_ITALIC, r);
            }
            if sp.strike {
                let _ = layout.SetStrikethrough(true, r);
            }
            if sp.link_url.is_some() {
                let _ = layout.SetUnderline(true, r);
            }
            if sp.code {
                let _ = layout.SetFontFamilyName(&mono_family, r);
                let _ = layout.SetFontSize((font_size - 1.6).max(11.5), r);
            }
            if sp.math {
                let _ = layout.SetFontFamilyName(&math_family, r);
                let _ = layout.SetFontSize(font_size + 0.6, r);
            }
        }
        Some(layout)
    }

    unsafe fn measure_layout_height(layout: &IDWriteTextLayout) -> f32 {
        let mut m = DWRITE_TEXT_METRICS::default();
        layout
            .GetMetrics(&mut m)
            .ok()
            .map(|()| m.height.max(18.0))
            .unwrap_or(18.0)
    }

    unsafe fn measure_md_blocks(&self, blocks: &[crate::markdown::MdBlock], max_w: f32) -> f32 {
        use crate::markdown::MdBlock;
        if blocks.is_empty() {
            return 24.0;
        }
        let mut y = 0.0_f32;
        for (idx, block) in blocks.iter().enumerate() {
            let is_last = idx + 1 == blocks.len();
            match block {
                MdBlock::Heading { level, text } => {
                    if idx > 0 {
                        y += 4.0;
                    }
                    let sz = match level {
                        1 => 17.5,
                        2 => 16.0,
                        3 => 15.0,
                        _ => 14.6,
                    };
                    let h = self
                        .build_md_text_layout(text, max_w, sz, true, false)
                        .map(|l| Self::measure_layout_height(&l))
                        .unwrap_or(22.0);
                    y += h + if is_last { 2.0 } else { 6.0 };
                }
                MdBlock::Paragraph(text) => {
                    let h = self
                        .build_md_text_layout(text, max_w, 14.6, false, false)
                        .map(|l| Self::measure_layout_height(&l))
                        .unwrap_or(22.0);
                    y += h + if is_last { 2.0 } else { 8.0 };
                }
                MdBlock::CodeBlock { lang, code } => {
                    let display = if code.is_empty() { " " } else { code.as_str() };
                    let wide: Vec<u16> = display.encode_utf16().collect();
                    let code_h = self
                        .dwrite
                        .CreateTextLayout(&wide, &self.fmt_ai_code, (max_w - 24.0).max(40.0), 4000.0)
                        .ok()
                        .map(|l| Self::measure_layout_height(&l))
                        .unwrap_or(19.0);
                    let lang_hdr = if lang.is_empty() { 0.0 } else { 18.0 };
                    y += lang_hdr + code_h + 18.0 + if is_last { 2.0 } else { 9.0 };
                }
                MdBlock::MathBlock(expr) => {
                    let display = if expr.is_empty() { " " } else { expr.as_str() };
                    let wide: Vec<u16> = display.encode_utf16().collect();
                    let math_h = self
                        .dwrite
                        .CreateTextLayout(&wide, &self.fmt_ai_math, (max_w - 24.0).max(40.0), 2000.0)
                        .ok()
                        .map(|l| Self::measure_layout_height(&l))
                        .unwrap_or(24.0);
                    y += math_h + 18.0 + if is_last { 2.0 } else { 9.0 };
                }
                MdBlock::BulletItem { indent, text } => {
                    let left_pad = 16.0 + (*indent as f32) * 14.0;
                    let h = self
                        .build_md_text_layout(text, (max_w - left_pad).max(40.0), 14.6, false, false)
                        .map(|l| Self::measure_layout_height(&l))
                        .unwrap_or(22.0);
                    y += h + if is_last { 2.0 } else { 4.5 };
                }
                MdBlock::NumberedItem { indent, text, .. } => {
                    let left_pad = 24.0 + (*indent as f32) * 14.0;
                    let h = self
                        .build_md_text_layout(text, (max_w - left_pad).max(40.0), 14.6, false, false)
                        .map(|l| Self::measure_layout_height(&l))
                        .unwrap_or(22.0);
                    y += h + if is_last { 2.0 } else { 4.5 };
                }
                MdBlock::Blockquote(text) => {
                    let h = self
                        .build_md_text_layout(text, (max_w - 16.0).max(40.0), 14.2, false, true)
                        .map(|l| Self::measure_layout_height(&l))
                        .unwrap_or(22.0);
                    y += h + 4.0 + if is_last { 2.0 } else { 8.0 };
                }
                MdBlock::HorizontalRule => {
                    y += 14.0;
                }
                MdBlock::Table { headers, rows } => {
                    let cols = headers.len().max(1);
                    let col_w = ((max_w - 16.0) / cols as f32).max(36.0);
                    let mut hdr_h = 22.0_f32;
                    for hcell in headers {
                        if let Some(l) =
                            self.build_md_text_layout(hcell, (col_w - 10.0).max(24.0), 13.0, true, false)
                        {
                            hdr_h = hdr_h.max(Self::measure_layout_height(&l) + 8.0);
                        }
                    }
                    let mut total_h = hdr_h;
                    for row in rows {
                        let mut rh = 20.0_f32;
                        for cell in row {
                            if let Some(l) = self.build_md_text_layout(
                                cell,
                                (col_w - 10.0).max(24.0),
                                13.0,
                                false,
                                false,
                            ) {
                                rh = rh.max(Self::measure_layout_height(&l) + 8.0);
                            }
                        }
                        total_h += rh;
                    }
                    y += total_h + 4.0 + if is_last { 2.0 } else { 9.0 };
                }
            }
        }
        y.max(24.0)
    }

    pub fn measure_ai_card_height(&self, text: &str, show_followup: bool) -> (f32, f32) {
        let blocks = crate::markdown::parse_markdown(text);
        let content_h = unsafe { self.measure_md_blocks(&blocks, 510.0) };
        let extra_footer = if show_followup { 38.0 } else { 0.0 };
        let card_h = (content_h + 58.0 + extra_footer).clamp(90.0, 310.0);
        (card_h, content_h)
    }

    pub fn hit_test_ai_link(
        &self,
        text: &str,
        card_r: D2D_RECT_F,
        scroll: f32,
        show_followup: bool,
        canvas_x: f32,
        canvas_y: f32,
    ) -> Option<String> {
        use crate::markdown::MdBlock;
        let body_bottom = if show_followup {
            card_r.bottom - 42.0
        } else {
            card_r.bottom - 16.0
        };
        let body_clip = D2D_RECT_F {
            left: card_r.left + 20.0,
            top: card_r.top + 38.0,
            right: card_r.right - 20.0,
            bottom: body_bottom,
        };
        if canvas_x < body_clip.left
            || canvas_x > body_clip.right
            || canvas_y < body_clip.top
            || canvas_y > body_clip.bottom
        {
            return None;
        }
        let blocks = crate::markdown::parse_markdown(text);
        let md_max_w = 510.0_f32;
        let origin_x = body_clip.left;
        let mut cur_y = body_clip.top - scroll;

        unsafe {
            let check_rich = |layout: &IDWriteTextLayout,
                              rich: &crate::markdown::MdRichText,
                              dx: f32,
                              dy: f32|
             -> Option<String> {
                for sp in &rich.spans {
                    let Some(url) = sp.link_url.as_ref() else {
                        continue;
                    };
                    if sp.len_u16 == 0 {
                        continue;
                    }
                    let mut htm = [DWRITE_HIT_TEST_METRICS::default(); 6];
                    let mut cnt = 0u32;
                    let _ = layout.HitTestTextRange(
                        sp.start_u16,
                        sp.len_u16,
                        dx,
                        dy,
                        Some(&mut htm),
                        &mut cnt,
                    );
                    let n = (cnt as usize).min(htm.len());
                    for m in &htm[..n] {
                        if canvas_x >= m.left - 2.0
                            && canvas_x <= m.left + m.width + 2.0
                            && canvas_y >= m.top - 1.5
                            && canvas_y <= m.top + m.height + 1.5
                        {
                            return Some(url.clone());
                        }
                    }
                }
                None
            };

            for (idx, block) in blocks.iter().enumerate() {
                let is_last = idx + 1 == blocks.len();
                match block {
                    MdBlock::Heading { level, text } => {
                        if idx > 0 {
                            cur_y += 4.0;
                        }
                        let sz = match level {
                            1 => 17.5,
                            2 => 16.0,
                            3 => 15.0,
                            _ => 14.6,
                        };
                        if let Some(layout) =
                            self.build_md_text_layout(text, md_max_w, sz, true, false)
                        {
                            if let Some(u) = check_rich(&layout, text, origin_x, cur_y) {
                                return Some(u);
                            }
                            cur_y +=
                                Self::measure_layout_height(&layout) + if is_last { 2.0 } else { 6.0 };
                        }
                    }
                    MdBlock::Paragraph(text) => {
                        if let Some(layout) =
                            self.build_md_text_layout(text, md_max_w, 14.6, false, false)
                        {
                            if let Some(u) = check_rich(&layout, text, origin_x, cur_y) {
                                return Some(u);
                            }
                            cur_y +=
                                Self::measure_layout_height(&layout) + if is_last { 2.0 } else { 8.0 };
                        }
                    }
                    MdBlock::CodeBlock { lang, code } => {
                        let display = if code.is_empty() { " " } else { code.as_str() };
                        let wide: Vec<u16> = display.encode_utf16().collect();
                        let code_h = self
                            .dwrite
                            .CreateTextLayout(
                                &wide,
                                &self.fmt_ai_code,
                                (md_max_w - 24.0).max(40.0),
                                4000.0,
                            )
                            .ok()
                            .map(|l| Self::measure_layout_height(&l))
                            .unwrap_or(19.0);
                        let lang_hdr = if lang.is_empty() { 0.0 } else { 18.0 };
                        cur_y += lang_hdr + code_h + 18.0 + if is_last { 2.0 } else { 9.0 };
                    }
                    MdBlock::MathBlock(expr) => {
                        let display = if expr.is_empty() { " " } else { expr.as_str() };
                        let wide: Vec<u16> = display.encode_utf16().collect();
                        let math_h = self
                            .dwrite
                            .CreateTextLayout(
                                &wide,
                                &self.fmt_ai_math,
                                (md_max_w - 24.0).max(40.0),
                                2000.0,
                            )
                            .ok()
                            .map(|l| Self::measure_layout_height(&l))
                            .unwrap_or(24.0);
                        cur_y += math_h + 18.0 + if is_last { 2.0 } else { 9.0 };
                    }
                    MdBlock::BulletItem { indent, text } => {
                        let left_pad = 16.0 + (*indent as f32) * 14.0;
                        if let Some(layout) = self.build_md_text_layout(
                            text,
                            (md_max_w - left_pad).max(40.0),
                            14.6,
                            false,
                            false,
                        ) {
                            if let Some(u) = check_rich(&layout, text, origin_x + left_pad, cur_y) {
                                return Some(u);
                            }
                            cur_y += Self::measure_layout_height(&layout)
                                + if is_last { 2.0 } else { 4.5 };
                        }
                    }
                    MdBlock::NumberedItem { indent, text, .. } => {
                        let left_pad = 24.0 + (*indent as f32) * 14.0;
                        if let Some(layout) = self.build_md_text_layout(
                            text,
                            (md_max_w - left_pad).max(40.0),
                            14.6,
                            false,
                            false,
                        ) {
                            if let Some(u) = check_rich(&layout, text, origin_x + left_pad, cur_y) {
                                return Some(u);
                            }
                            cur_y += Self::measure_layout_height(&layout)
                                + if is_last { 2.0 } else { 4.5 };
                        }
                    }
                    MdBlock::Blockquote(text) => {
                        if let Some(layout) = self.build_md_text_layout(
                            text,
                            (md_max_w - 16.0).max(40.0),
                            14.2,
                            false,
                            true,
                        ) {
                            if let Some(u) =
                                check_rich(&layout, text, origin_x + 14.0, cur_y + 2.0)
                            {
                                return Some(u);
                            }
                            cur_y += Self::measure_layout_height(&layout)
                                + 4.0
                                + if is_last { 2.0 } else { 8.0 };
                        }
                    }
                    MdBlock::HorizontalRule => {
                        cur_y += 14.0;
                    }
                    MdBlock::Table { headers, rows } => {
                        let cols = headers.len().max(1);
                        let col_w = ((md_max_w - 16.0) / cols as f32).max(36.0);
                        let mut hdr_h = 22.0_f32;
                        for (c, hcell) in headers.iter().enumerate() {
                            if let Some(l) = self.build_md_text_layout(
                                hcell,
                                (col_w - 10.0).max(24.0),
                                13.0,
                                true,
                                false,
                            ) {
                                if let Some(u) = check_rich(
                                    &l,
                                    hcell,
                                    origin_x + 8.0 + c as f32 * col_w,
                                    cur_y + 4.0,
                                ) {
                                    return Some(u);
                                }
                                hdr_h = hdr_h.max(Self::measure_layout_height(&l) + 8.0);
                            }
                        }
                        let mut ry = cur_y + hdr_h;
                        for row in rows {
                            let mut rh = 20.0_f32;
                            for (c, cell) in row.iter().enumerate() {
                                if let Some(l) = self.build_md_text_layout(
                                    cell,
                                    (col_w - 10.0).max(24.0),
                                    13.0,
                                    false,
                                    false,
                                ) {
                                    if let Some(u) = check_rich(
                                        &l,
                                        cell,
                                        origin_x + 8.0 + c as f32 * col_w,
                                        ry + 4.0,
                                    ) {
                                        return Some(u);
                                    }
                                    rh = rh.max(Self::measure_layout_height(&l) + 8.0);
                                }
                            }
                            ry += rh;
                        }
                        cur_y = ry + 4.0 + if is_last { 2.0 } else { 9.0 };
                    }
                }
            }
        }
        None
    }

    unsafe fn collect_md_segments(
        &self,
        text: &str,
        origin_x: f32,
        origin_y: f32,
    ) -> Vec<(IDWriteTextLayout, String, f32, f32, f32)> {
        use crate::markdown::MdBlock;
        let blocks = crate::markdown::parse_markdown(text);
        let md_max_w = 510.0_f32;
        let mut cur_y = origin_y;
        let mut out = Vec::new();

        for (idx, block) in blocks.iter().enumerate() {
            let is_last = idx + 1 == blocks.len();
            match block {
                MdBlock::Heading { level, text } => {
                    if idx > 0 {
                        cur_y += 4.0;
                    }
                    let sz = match level {
                        1 => 17.5,
                        2 => 16.0,
                        3 => 15.0,
                        _ => 14.6,
                    };
                    if let Some(layout) = self.build_md_text_layout(text, md_max_w, sz, true, false)
                    {
                        let h = Self::measure_layout_height(&layout);
                        out.push((layout, text.plain.clone(), origin_x, cur_y, h));
                        cur_y += h + if is_last { 2.0 } else { 6.0 };
                    }
                }
                MdBlock::Paragraph(text) => {
                    if let Some(layout) =
                        self.build_md_text_layout(text, md_max_w, 14.6, false, false)
                    {
                        let h = Self::measure_layout_height(&layout);
                        out.push((layout, text.plain.clone(), origin_x, cur_y, h));
                        cur_y += h + if is_last { 2.0 } else { 8.0 };
                    }
                }
                MdBlock::CodeBlock { lang, code } => {
                    let display = if code.is_empty() { " " } else { code.as_str() };
                    let wide: Vec<u16> = display.encode_utf16().collect();
                    if let Ok(layout) = self.dwrite.CreateTextLayout(
                        &wide,
                        &self.fmt_ai_code,
                        (md_max_w - 24.0).max(40.0),
                        4000.0,
                    ) {
                        let code_h = Self::measure_layout_height(&layout);
                        let lang_hdr = if lang.is_empty() { 0.0 } else { 18.0 };
                        let code_y = cur_y + lang_hdr + 9.0;
                        out.push((layout, code.clone(), origin_x + 12.0, code_y, code_h));
                        cur_y += lang_hdr + code_h + 18.0 + if is_last { 2.0 } else { 9.0 };
                    }
                }
                MdBlock::MathBlock(expr) => {
                    let display = if expr.is_empty() { " " } else { expr.as_str() };
                    let wide: Vec<u16> = display.encode_utf16().collect();
                    if let Ok(layout) = self.dwrite.CreateTextLayout(
                        &wide,
                        &self.fmt_ai_math,
                        (md_max_w - 24.0).max(40.0),
                        2000.0,
                    ) {
                        let math_h = Self::measure_layout_height(&layout);
                        let math_y = cur_y + 9.0;
                        out.push((layout, expr.clone(), origin_x + 12.0, math_y, math_h));
                        cur_y += math_h + 18.0 + if is_last { 2.0 } else { 9.0 };
                    }
                }
                MdBlock::BulletItem { indent, text } => {
                    let left_pad = 16.0 + (*indent as f32) * 14.0;
                    if let Some(layout) = self.build_md_text_layout(
                        text,
                        (md_max_w - left_pad).max(40.0),
                        14.6,
                        false,
                        false,
                    ) {
                        let h = Self::measure_layout_height(&layout);
                        out.push((layout, text.plain.clone(), origin_x + left_pad, cur_y, h));
                        cur_y += h + if is_last { 2.0 } else { 4.5 };
                    }
                }
                MdBlock::NumberedItem { indent, text, .. } => {
                    let left_pad = 24.0 + (*indent as f32) * 14.0;
                    if let Some(layout) = self.build_md_text_layout(
                        text,
                        (md_max_w - left_pad).max(40.0),
                        14.6,
                        false,
                        false,
                    ) {
                        let h = Self::measure_layout_height(&layout);
                        out.push((layout, text.plain.clone(), origin_x + left_pad, cur_y, h));
                        cur_y += h + if is_last { 2.0 } else { 4.5 };
                    }
                }
                MdBlock::Blockquote(text) => {
                    if let Some(layout) = self.build_md_text_layout(
                        text,
                        (md_max_w - 16.0).max(40.0),
                        14.2,
                        false,
                        true,
                    ) {
                        let h = Self::measure_layout_height(&layout);
                        out.push((layout, text.plain.clone(), origin_x + 14.0, cur_y + 2.0, h));
                        cur_y += h + 4.0 + if is_last { 2.0 } else { 8.0 };
                    }
                }
                MdBlock::HorizontalRule => {
                    cur_y += 14.0;
                }
                MdBlock::Table { headers, rows } => {
                    let cols = headers.len().max(1);
                    let col_w = ((md_max_w - 16.0) / cols as f32).max(36.0);
                    let mut hdr_h = 22.0_f32;
                    for (c, hcell) in headers.iter().enumerate() {
                        if let Some(l) = self.build_md_text_layout(
                            hcell,
                            (col_w - 10.0).max(24.0),
                            13.0,
                            true,
                            false,
                        ) {
                            let h = Self::measure_layout_height(&l);
                            hdr_h = hdr_h.max(h + 8.0);
                            out.push((
                                l,
                                hcell.plain.clone(),
                                origin_x + 8.0 + c as f32 * col_w,
                                cur_y + 4.0,
                                h,
                            ));
                        }
                    }
                    let mut ry = cur_y + hdr_h;
                    for row in rows {
                        let mut rh = 20.0_f32;
                        for (c, cell) in row.iter().enumerate() {
                            if let Some(l) = self.build_md_text_layout(
                                cell,
                                (col_w - 10.0).max(24.0),
                                13.0,
                                false,
                                false,
                            ) {
                                let h = Self::measure_layout_height(&l);
                                rh = rh.max(h + 8.0);
                                out.push((
                                    l,
                                    cell.plain.clone(),
                                    origin_x + 8.0 + c as f32 * col_w,
                                    ry + 4.0,
                                    h,
                                ));
                            }
                        }
                        ry += rh;
                    }
                    cur_y = ry + 4.0 + if is_last { 2.0 } else { 9.0 };
                }
            }
        }
        out
    }

    pub fn hit_test_ai_text_pos(
        &self,
        text: &str,
        card_r: D2D_RECT_F,
        scroll: f32,
        canvas_x: f32,
        canvas_y: f32,
    ) -> Option<(usize, u32)> {
        if text.is_empty() {
            return None;
        }
        let origin_x = card_r.left + 20.0;
        let origin_y = card_r.top + 38.0 - scroll;
        unsafe {
            let segs = self.collect_md_segments(text, origin_x, origin_y);
            if segs.is_empty() {
                return None;
            }
            let mut best_idx = 0usize;
            let mut best_dist = f32::INFINITY;
            for (i, (_, _, _, sy, sh)) in segs.iter().enumerate() {
                let d = if canvas_y < *sy {
                    *sy - canvas_y
                } else if canvas_y > *sy + *sh {
                    canvas_y - (*sy + *sh)
                } else {
                    0.0
                };
                if d < best_dist {
                    best_dist = d;
                    best_idx = i;
                }
            }
            let (layout, plain, sx, sy, sh) = &segs[best_idx];
            let clamped_y = (canvas_y - *sy).clamp(0.0, (*sh - 1.0).max(0.0));
            let mut is_trailing = BOOL(0);
            let mut is_inside = BOOL(0);
            let mut htm = DWRITE_HIT_TEST_METRICS::default();
            if layout
                .HitTestPoint(
                    canvas_x - *sx,
                    clamped_y,
                    &mut is_trailing,
                    &mut is_inside,
                    &mut htm,
                )
                .is_ok()
            {
                let max_u16 = plain.encode_utf16().count() as u32;
                let pos = (htm.textPosition
                    + if is_trailing.as_bool() { htm.length } else { 0 })
                .min(max_u16);
                return Some((best_idx, pos));
            }
        }
        None
    }

    pub fn ai_select_all_end(&self, text: &str) -> Option<(usize, u32)> {
        if text.is_empty() {
            return None;
        }
        unsafe {
            let segs = self.collect_md_segments(text, 0.0, 0.0);
            let last_idx = segs.len().checked_sub(1)?;
            let last_u16 = segs[last_idx].1.encode_utf16().count() as u32;
            Some((last_idx, last_u16))
        }
    }

    pub fn extract_ai_selected_text(
        &self,
        text: &str,
        start: (usize, u32),
        end: (usize, u32),
    ) -> String {
        if text.is_empty() || start >= end {
            return String::new();
        }
        let segs = unsafe { self.collect_md_segments(text, 0.0, 0.0) };
        let mut parts = Vec::new();
        for (i, (_, plain, _, _, _)) in segs.iter().enumerate() {
            if i < start.0 || i > end.0 {
                continue;
            }
            let units: Vec<u16> = plain.encode_utf16().collect();
            let s = if i == start.0 {
                (start.1 as usize).min(units.len())
            } else {
                0
            };
            let e = if i == end.0 {
                (end.1 as usize).min(units.len())
            } else {
                units.len()
            };
            if e > s {
                parts.push(String::from_utf16_lossy(&units[s..e]));
            }
        }
        parts.join("\n")
    }

    unsafe fn draw_ai_sparkle_stroked(
        &self,
        cx: f32,
        cy: f32,
        radius: f32,
        stroke_w: f32,
        rgba: (f32, f32, f32, f32),
    ) {
        if let Ok(geo) = self.d2d_factory.CreatePathGeometry() {
            if let Ok(sink) = geo.Open() {
                let r_out = radius;
                let r_in = radius * 0.257;
                sink.BeginFigure(
                    D2D_POINT_2F { x: cx, y: cy - r_out },
                    windows::Win32::Graphics::Direct2D::Common::D2D1_FIGURE_BEGIN_HOLLOW,
                );
                sink.AddLine(D2D_POINT_2F { x: cx + r_in, y: cy - r_in });
                sink.AddLine(D2D_POINT_2F { x: cx + r_out, y: cy });
                sink.AddLine(D2D_POINT_2F { x: cx + r_in, y: cy + r_in });
                sink.AddLine(D2D_POINT_2F { x: cx, y: cy + r_out });
                sink.AddLine(D2D_POINT_2F { x: cx - r_in, y: cy + r_in });
                sink.AddLine(D2D_POINT_2F { x: cx - r_out, y: cy });
                sink.AddLine(D2D_POINT_2F { x: cx - r_in, y: cy - r_in });
                sink.EndFigure(D2D1_FIGURE_END_CLOSED);
                if sink.Close().is_ok() {
                    let round_st = ID2D1Factory::CreateStrokeStyle(
                        &self.d2d_factory,
                        &D2D1_STROKE_STYLE_PROPERTIES {
                            startCap: D2D1_CAP_STYLE_ROUND,
                            endCap: D2D1_CAP_STYLE_ROUND,
                            lineJoin: D2D1_LINE_JOIN_ROUND,
                            ..Default::default()
                        },
                        None,
                    )
                    .ok();
                    self.set_color(rgba.0, rgba.1, rgba.2, rgba.3);
                    self.d2d_context.DrawGeometry(
                        &geo,
                        &self.solid_brush,
                        stroke_w,
                        round_st.as_ref(),
                    );
                }
            }
        }
    }

    unsafe fn draw_return_arrow_icon(
        &self,
        cx: f32,
        cy: f32,
        scale: f32,
        stroke_w: f32,
        rgba: (f32, f32, f32, f32),
    ) {
        let round_st = ID2D1Factory::CreateStrokeStyle(
            &self.d2d_factory,
            &D2D1_STROKE_STYLE_PROPERTIES {
                startCap: D2D1_CAP_STYLE_ROUND,
                endCap: D2D1_CAP_STYLE_ROUND,
                lineJoin: D2D1_LINE_JOIN_ROUND,
                ..Default::default()
            },
            None,
        )
        .ok();
        self.set_color(rgba.0, rgba.1, rgba.2, rgba.3);
        let rx = cx + 3.6 * scale;
        let lx = cx - 3.6 * scale;
        let ty = cy - 2.6 * scale;
        let by = cy + 1.4 * scale;
        self.d2d_context.DrawLine(
            D2D_POINT_2F { x: rx, y: ty },
            D2D_POINT_2F { x: rx, y: by },
            &self.solid_brush,
            stroke_w,
            round_st.as_ref(),
        );
        self.d2d_context.DrawLine(
            D2D_POINT_2F { x: rx, y: by },
            D2D_POINT_2F { x: lx, y: by },
            &self.solid_brush,
            stroke_w,
            round_st.as_ref(),
        );
        self.d2d_context.DrawLine(
            D2D_POINT_2F { x: lx + 2.3 * scale, y: by - 2.2 * scale },
            D2D_POINT_2F { x: lx, y: by },
            &self.solid_brush,
            stroke_w,
            round_st.as_ref(),
        );
        self.d2d_context.DrawLine(
            D2D_POINT_2F { x: lx + 2.3 * scale, y: by + 2.2 * scale },
            D2D_POINT_2F { x: lx, y: by },
            &self.solid_brush,
            stroke_w,
            round_st.as_ref(),
        );
    }

    unsafe fn draw_ai_notch_layers(
        &self,
        springs: &NotchSprings,
        state: &EaveViewState<'_>,
        pill_left: f32,
        cur_w: f32,
        cur_h: f32,
        ear_r: f32,
    ) {
        let body_left = pill_left + ear_r;
        let in_a = springs.ai_input_open.value().clamp(0.0, 1.0);
        let wave_a = springs.ai_wave_open.value().clamp(0.0, 1.0);

        if in_a > 0.01 {
            let shake = springs.ai_shake.value;
            let cy = cur_h * 0.5;
            let cx = pill_left + cur_w * 0.5;
            let prev_xf = {
                let mut m = Matrix3x2::identity();
                self.d2d_context.GetTransform(&mut m);
                m
            };
            let s = 0.96 + 0.04 * in_a;
            if (s - 1.0).abs() > 0.001 {
                let xf = Matrix3x2 {
                    M11: s,
                    M12: 0.0,
                    M21: 0.0,
                    M22: s,
                    M31: cx * (1.0 - s),
                    M32: cy * (1.0 - s),
                };
                self.d2d_context.SetTransform(&mul_matrix3x2(xf, prev_xf));
            }

            let sx = body_left + 28.0 + shake;
            self.draw_ai_sparkle_stroked(sx, cy, 7.6, 1.75, (1.0, 1.0, 1.0, 0.62 * in_a));

            let kbd_r = ai_input_enter_rect(pill_left, cur_w, cur_h, ear_r);
            let enter_hov = springs.ai_btn_hover[4].value().clamp(0.0, 1.0);
            let has_text = !state.widgets.ai.prompt.trim().is_empty();
            self.set_color(1.0, 1.0, 1.0, (0.14 + 0.08 * enter_hov) * in_a);
            self.d2d_context.FillRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: kbd_r.left,
                        top: kbd_r.top + 1.0,
                        right: kbd_r.right,
                        bottom: kbd_r.bottom + 1.4,
                    },
                    radiusX: 6.0,
                    radiusY: 6.0,
                },
                &self.solid_brush,
            );
            self.set_color(
                0.110 + 0.045 * enter_hov,
                0.110 + 0.045 * enter_hov,
                0.133 + 0.055 * enter_hov,
                0.98 * in_a,
            );
            self.d2d_context.FillRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: kbd_r,
                    radiusX: 6.0,
                    radiusY: 6.0,
                },
                &self.solid_brush,
            );
            self.set_color(1.0, 1.0, 1.0, (0.11 + 0.12 * enter_hov) * in_a);
            self.d2d_context.DrawRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: kbd_r,
                    radiusX: 6.0,
                    radiusY: 6.0,
                },
                &self.solid_brush,
                0.9,
                None,
            );
            let enter_arrow_a =
                (if has_text { 0.72 } else { 0.56 } + 0.28 * enter_hov).min(0.98) * in_a;
            self.draw_return_arrow_icon(
                (kbd_r.left + kbd_r.right) * 0.5,
                (kbd_r.top + kbd_r.bottom) * 0.5,
                1.0 + 0.10 * enter_hov,
                1.45,
                (1.0, 1.0, 1.0, enter_arrow_a),
            );

            let has_saved_chat =
                !state.widgets.ai.turns.is_empty() || !state.widgets.ai.response.is_empty();
            let toggle_r = ai_input_toggle_rect(pill_left, cur_w, cur_h, ear_r);
            if has_saved_chat {
                let open_t = springs.ai_card_open.value().clamp(0.0, 1.0);
                let hov = springs.ai_btn_hover[3].value().clamp(0.0, 1.0);
                self.set_color(1.0, 1.0, 1.0, (0.14 + 0.08 * hov) * in_a);
                self.d2d_context.FillRoundedRectangle(
                    &D2D1_ROUNDED_RECT {
                        rect: D2D_RECT_F {
                            left: toggle_r.left,
                            top: toggle_r.top + 1.0,
                            right: toggle_r.right,
                            bottom: toggle_r.bottom + 1.4,
                        },
                        radiusX: 6.0,
                        radiusY: 6.0,
                    },
                    &self.solid_brush,
                );
                self.set_color(
                    0.110 + 0.045 * hov,
                    0.110 + 0.045 * hov,
                    0.133 + 0.055 * hov,
                    0.98 * in_a,
                );
                self.d2d_context.FillRoundedRectangle(
                    &D2D1_ROUNDED_RECT {
                        rect: toggle_r,
                        radiusX: 6.0,
                        radiusY: 6.0,
                    },
                    &self.solid_brush,
                );
                self.set_color(1.0, 1.0, 1.0, (0.11 + 0.12 * hov) * in_a);
                self.d2d_context.DrawRoundedRectangle(
                    &D2D1_ROUNDED_RECT {
                        rect: toggle_r,
                        radiusX: 6.0,
                        radiusY: 6.0,
                    },
                    &self.solid_brush,
                    0.9,
                    None,
                );
                let tcx = (toggle_r.left + toggle_r.right) * 0.5;
                let tcy = (toggle_r.top + toggle_r.bottom) * 0.5;
                let cs = 1.0 + 0.12 * hov;
                let dir = 1.0 - 2.0 * open_t;
                let tip_y = tcy + 2.1 * dir * cs;
                let wing_y = tcy - 1.9 * dir * cs;
                let round_st = ID2D1Factory::CreateStrokeStyle(
                    &self.d2d_factory,
                    &D2D1_STROKE_STYLE_PROPERTIES {
                        startCap: D2D1_CAP_STYLE_ROUND,
                        endCap: D2D1_CAP_STYLE_ROUND,
                        lineJoin: D2D1_LINE_JOIN_ROUND,
                        ..Default::default()
                    },
                    None,
                )
                .ok();
                self.set_color(1.0, 1.0, 1.0, (0.56 + 0.36 * hov) * in_a);
                self.d2d_context.DrawLine(
                    D2D_POINT_2F { x: tcx - 3.8 * cs, y: wing_y },
                    D2D_POINT_2F { x: tcx, y: tip_y },
                    &self.solid_brush,
                    1.45,
                    round_st.as_ref(),
                );
                self.d2d_context.DrawLine(
                    D2D_POINT_2F { x: tcx, y: tip_y },
                    D2D_POINT_2F { x: tcx + 3.8 * cs, y: wing_y },
                    &self.solid_brush,
                    1.45,
                    round_st.as_ref(),
                );
            }

            let txt_l = body_left + 48.0 + shake;
            let right_limit = if has_saved_chat {
                toggle_r.left - 10.0
            } else {
                kbd_r.left - 12.0
            };
            let txt_r = right_limit.max(txt_l + 60.0);
            let txt_clip = D2D_RECT_F {
                left: txt_l,
                top: cy - 16.0,
                right: txt_r,
                bottom: cy + 16.0,
            };
            self.d2d_context
                .PushAxisAlignedClip(&txt_clip, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);

            let prompt = state.widgets.ai.prompt.as_str();
            if prompt.is_empty() {
                let ph = if !state.widgets.ai.has_api_key {
                    "Inserisci la Gemini API Key in Impostazioni → Eave AI"
                } else if !state.widgets.ai.history.is_empty() {
                    "Continua la conversazione…"
                } else {
                    "Chiedi qualcosa…"
                };
                self.draw_text(
                    ph,
                    &self.fmt_ai_input,
                    &txt_clip,
                    (1.0, 1.0, 1.0, 0.40 * in_a),
                );
                if ((springs.phase_secs() * 1.9) as i32) % 2 == 0 {
                    self.set_color(0.75, 0.35, 0.95, 0.92 * in_a);
                    self.d2d_context.FillRoundedRectangle(
                        &D2D1_ROUNDED_RECT {
                            rect: D2D_RECT_F {
                                left: txt_l,
                                top: cy - 9.5,
                                right: txt_l + 2.0,
                                bottom: cy + 9.5,
                            },
                            radiusX: 1.0,
                            radiusY: 1.0,
                        },
                        &self.solid_brush,
                    );
                }
            } else {
                let text_w = self.measure_width(prompt, &self.fmt_ai_input);
                let avail_w = (txt_r - txt_l - 6.0).max(20.0);
                let scroll_x = (text_w - avail_w).max(0.0);
                let draw_r = D2D_RECT_F {
                    left: txt_l - scroll_x,
                    top: txt_clip.top,
                    right: txt_l - scroll_x + text_w + 16.0,
                    bottom: txt_clip.bottom,
                };
                self.draw_text(
                    prompt,
                    &self.fmt_ai_input,
                    &draw_r,
                    (0.95, 0.95, 0.96, 0.98 * in_a),
                );
                if ((springs.phase_secs() * 1.9) as i32) % 2 == 0 {
                    let caret_x = (txt_l - scroll_x + text_w + 1.5).min(txt_r - 2.0);
                    self.set_color(0.75, 0.35, 0.95, 0.96 * in_a);
                    self.d2d_context.FillRoundedRectangle(
                        &D2D1_ROUNDED_RECT {
                            rect: D2D_RECT_F {
                                left: caret_x,
                                top: cy - 9.5,
                                right: caret_x + 2.0,
                                bottom: cy + 9.5,
                            },
                            radiusX: 1.0,
                            radiusY: 1.0,
                        },
                        &self.solid_brush,
                    );
                }
            }
            self.d2d_context.PopAxisAlignedClip();
            if (s - 1.0).abs() > 0.001 {
                self.d2d_context.SetTransform(&prev_xf);
            }
        }

        if wave_a > 0.01 {
            if let Some(bmp) = &self.ai_wave_bitmap {
                let dest = D2D_RECT_F {
                    left: pill_left,
                    top: 0.0,
                    right: pill_left + cur_w,
                    bottom: cur_h,
                };
                self.d2d_context.DrawBitmap(
                    bmp,
                    Some(&dest),
                    wave_a,
                    windows::Win32::Graphics::Direct2D::D2D1_INTERPOLATION_MODE_LINEAR,
                    None,
                    None,
                );
            }
        }
    }

    unsafe fn draw_ai_response_card(
        &self,
        springs: &NotchSprings,
        state: &EaveViewState<'_>,
        cur_h: f32,
        canvas_cx: f32,
    ) {
        let raw_open = springs.ai_card_open.value();
        let card_a = raw_open.clamp(0.0, 1.0);
        if card_a <= 0.01 {
            return;
        }
        let ai = &state.widgets.ai;
        let (display_q, body_text, is_live_turn) = ai.displayed_turn();
        let is_err = is_live_turn && ai.error.is_some();
        let is_busy = is_live_turn && ai.phase == crate::ai::AiPhase::Busy;
        let show_followup = ai.phase == crate::ai::AiPhase::Done && !is_err;
        let (target_card_h, content_h) = self.measure_ai_card_height(body_text, show_followup);
        let card_h = if springs.ai_card_h.value > 20.0 {
            springs.ai_card_h.value.clamp(90.0, 310.0)
        } else {
            target_card_h
        };
        let (card_r, copy_r, close_r, followup_r) = ai_card_rects(canvas_cx, cur_h, card_h);

        let prev_xf = {
            let mut m = Matrix3x2::identity();
            self.d2d_context.GetTransform(&mut m);
            m
        };
        let scale = (0.92 + 0.08 * raw_open).clamp(0.90, 1.03);
        let slide_y = (1.0 - raw_open) * -14.0;
        if (scale - 1.0).abs() > 0.001 || slide_y.abs() > 0.05 {
            let xf = Matrix3x2 {
                M11: scale,
                M12: 0.0,
                M21: 0.0,
                M22: scale,
                M31: canvas_cx * (1.0 - scale),
                M32: card_r.top * (1.0 - scale) + slide_y,
            };
            self.d2d_context.SetTransform(&mul_matrix3x2(xf, prev_xf));
        }

        const SHADOW_STEPS: usize = 28;
        for i in 0..SHADOW_STEPS {
            let t = i as f32 / (SHADOW_STEPS - 1) as f32;
            let spread = 0.6 + 25.0 * t;
            let down = 1.5 + 22.0 * t;
            let layer_a = (-3.4 * t * t).exp() * 0.040 * card_a;
            self.set_color(0.0, 0.0, 0.0, layer_a);
            self.d2d_context.FillRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: card_r.left - spread * 0.72,
                        top: card_r.top + down * 0.24 - spread * 0.14,
                        right: card_r.right + spread * 0.72,
                        bottom: card_r.bottom + down,
                    },
                    radiusX: 24.0 + spread * 0.42,
                    radiusY: 24.0 + spread * 0.42,
                },
                &self.solid_brush,
            );
        }

        self.set_color(0.078, 0.078, 0.094, 0.97 * card_a);
        self.d2d_context.FillRoundedRectangle(
            &D2D1_ROUNDED_RECT {
                rect: card_r,
                radiusX: 24.0,
                radiusY: 24.0,
            },
            &self.solid_brush,
        );
        self.set_color(1.0, 1.0, 1.0, 0.10 * card_a);
        self.d2d_context.DrawRoundedRectangle(
            &D2D1_ROUNDED_RECT {
                rect: card_r,
                radiusX: 24.0,
                radiusY: 24.0,
            },
            &self.solid_brush,
            1.0,
            None,
        );

        let clear_r = ai_clear_rect(copy_r);
        let hdr_l = card_r.left + 20.0;
        let hdr_r = (clear_r.left - 10.0).max(hdr_l + 60.0);
        self.draw_text(
            display_q,
            &self.fmt_artist,
            &D2D_RECT_F {
                left: hdr_l,
                top: card_r.top + 14.0,
                right: hdr_r,
                bottom: card_r.top + 33.0,
            },
            (0.541, 0.541, 0.580, 0.96 * card_a),
        );

        let round_st = ID2D1Factory::CreateStrokeStyle(
            &self.d2d_factory,
            &D2D1_STROKE_STYLE_PROPERTIES {
                startCap: D2D1_CAP_STYLE_ROUND,
                endCap: D2D1_CAP_STYLE_ROUND,
                lineJoin: D2D1_LINE_JOIN_ROUND,
                ..Default::default()
            },
            None,
        )
        .ok();

        let tr_cx = (clear_r.left + clear_r.right) * 0.5;
        let tr_cy = (clear_r.top + clear_r.bottom) * 0.5;
        let tr_h = springs.ai_btn_hover[0].value().clamp(0.0, 1.0);
        if tr_h > 0.01 {
            let ph = 11.0 * (0.80 + 0.20 * tr_h);
            self.set_color(1.0, 0.36, 0.36, 0.15 * tr_h * card_a);
            self.d2d_context.FillRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: tr_cx - ph,
                        top: tr_cy - ph,
                        right: tr_cx + ph,
                        bottom: tr_cy + ph,
                    },
                    radiusX: 7.0,
                    radiusY: 7.0,
                },
                &self.solid_brush,
            );
        }
        let ts = 1.0 + 0.14 * tr_h;
        self.set_color(
            0.76 + 0.24 * tr_h,
            0.78 - 0.30 * tr_h,
            0.84 - 0.36 * tr_h,
            (0.55 + 0.42 * tr_h) * card_a,
        );
        if let Ok(handle_geo) = self.d2d_factory.CreatePathGeometry() {
            if let Ok(sink) = handle_geo.Open() {
                sink.BeginFigure(
                    D2D_POINT_2F { x: tr_cx - 1.9 * ts, y: tr_cy - 3.3 * ts },
                    windows::Win32::Graphics::Direct2D::Common::D2D1_FIGURE_BEGIN_HOLLOW,
                );
                sink.AddLine(D2D_POINT_2F { x: tr_cx - 1.5 * ts, y: tr_cy - 4.9 * ts });
                sink.AddLine(D2D_POINT_2F { x: tr_cx + 1.5 * ts, y: tr_cy - 4.9 * ts });
                sink.AddLine(D2D_POINT_2F { x: tr_cx + 1.9 * ts, y: tr_cy - 3.3 * ts });
                sink.EndFigure(windows::Win32::Graphics::Direct2D::Common::D2D1_FIGURE_END_OPEN);
                if sink.Close().is_ok() {
                    self.d2d_context.DrawGeometry(
                        &handle_geo,
                        &self.solid_brush,
                        1.2,
                        round_st.as_ref(),
                    );
                }
            }
        }
        self.d2d_context.DrawLine(
            D2D_POINT_2F { x: tr_cx - 5.1 * ts, y: tr_cy - 3.1 * ts },
            D2D_POINT_2F { x: tr_cx + 5.1 * ts, y: tr_cy - 3.1 * ts },
            &self.solid_brush,
            1.25,
            round_st.as_ref(),
        );
        if let Ok(bin_geo) = self.d2d_factory.CreatePathGeometry() {
            if let Ok(sink) = bin_geo.Open() {
                sink.BeginFigure(
                    D2D_POINT_2F { x: tr_cx - 3.8 * ts, y: tr_cy - 3.0 * ts },
                    windows::Win32::Graphics::Direct2D::Common::D2D1_FIGURE_BEGIN_HOLLOW,
                );
                sink.AddLine(D2D_POINT_2F { x: tr_cx - 3.1 * ts, y: tr_cy + 4.3 * ts });
                sink.AddLine(D2D_POINT_2F { x: tr_cx - 2.0 * ts, y: tr_cy + 5.0 * ts });
                sink.AddLine(D2D_POINT_2F { x: tr_cx + 2.0 * ts, y: tr_cy + 5.0 * ts });
                sink.AddLine(D2D_POINT_2F { x: tr_cx + 3.1 * ts, y: tr_cy + 4.3 * ts });
                sink.AddLine(D2D_POINT_2F { x: tr_cx + 3.8 * ts, y: tr_cy - 3.0 * ts });
                sink.EndFigure(windows::Win32::Graphics::Direct2D::Common::D2D1_FIGURE_END_OPEN);
                if sink.Close().is_ok() {
                    self.d2d_context.DrawGeometry(
                        &bin_geo,
                        &self.solid_brush,
                        1.2,
                        round_st.as_ref(),
                    );
                }
            }
        }
        self.d2d_context.DrawLine(
            D2D_POINT_2F { x: tr_cx - 1.35 * ts, y: tr_cy - 0.9 * ts },
            D2D_POINT_2F { x: tr_cx - 1.15 * ts, y: tr_cy + 2.9 * ts },
            &self.solid_brush,
            1.1,
            round_st.as_ref(),
        );
        self.d2d_context.DrawLine(
            D2D_POINT_2F { x: tr_cx + 1.35 * ts, y: tr_cy - 0.9 * ts },
            D2D_POINT_2F { x: tr_cx + 1.15 * ts, y: tr_cy + 2.9 * ts },
            &self.solid_brush,
            1.1,
            round_st.as_ref(),
        );

        let cp_cx = (copy_r.left + copy_r.right) * 0.5;
        let cp_cy = (copy_r.top + copy_r.bottom) * 0.5;
        let copy_t = springs.ai_copy_anim.value().clamp(0.0, 1.0);
        let cp_h = springs.ai_btn_hover[1].value().clamp(0.0, 1.0);
        if copy_t > 0.01 || cp_h > 0.01 {
            let mix_p = cp_h.max(copy_t);
            let ph = 11.0 * (0.80 + 0.20 * mix_p);
            let r = 1.0 * (1.0 - copy_t) + 0.24 * copy_t;
            let g = 1.0 * (1.0 - copy_t) + 0.86 * copy_t;
            let b = 1.0 * (1.0 - copy_t) + 0.48 * copy_t;
            let pill_a = (0.11 * cp_h).max(0.15 * copy_t) * card_a;
            self.set_color(r, g, b, pill_a);
            self.d2d_context.FillRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: cp_cx - ph,
                        top: cp_cy - ph,
                        right: cp_cx + ph,
                        bottom: cp_cy + ph,
                    },
                    radiusX: 7.0,
                    radiusY: 7.0,
                },
                &self.solid_brush,
            );
        }
        let icon_a = (1.0 - copy_t * 1.45).clamp(0.0, 1.0);
        if icon_a > 0.01 {
            let s = (1.0 - 0.25 * copy_t) * (1.0 + 0.14 * cp_h);
            self.set_color(
                0.76 + 0.22 * cp_h,
                0.78 + 0.20 * cp_h,
                0.84 + 0.16 * cp_h,
                (0.55 + 0.40 * cp_h) * icon_a * card_a,
            );
            self.d2d_context.DrawRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: cp_cx - 4.6 * s,
                        top: cp_cy - 4.6 * s,
                        right: cp_cx + 1.8 * s,
                        bottom: cp_cy + 1.8 * s,
                    },
                    radiusX: 1.5 * s,
                    radiusY: 1.5 * s,
                },
                &self.solid_brush,
                1.2,
                None,
            );
            self.d2d_context.DrawRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: cp_cx - 1.8 * s,
                        top: cp_cy - 1.8 * s,
                        right: cp_cx + 4.6 * s,
                        bottom: cp_cy + 4.6 * s,
                    },
                    radiusX: 1.5 * s,
                    radiusY: 1.5 * s,
                },
                &self.solid_brush,
                1.2,
                None,
            );
        }
        let check_a = ((copy_t - 0.16) / 0.84).clamp(0.0, 1.0);
        if check_a > 0.01 {
            let s = (0.72 + 0.28 * check_a) * (1.0 + 0.10 * cp_h);
            self.set_color(0.28, 0.88, 0.50, check_a * card_a);
            let p0 = D2D_POINT_2F {
                x: cp_cx - 4.2 * s,
                y: cp_cy + 0.3 * s,
            };
            let p1 = D2D_POINT_2F {
                x: cp_cx - 1.2 * s,
                y: cp_cy + 3.4 * s,
            };
            let p2 = D2D_POINT_2F {
                x: cp_cx + 4.4 * s,
                y: cp_cy - 3.2 * s,
            };
            let seg1 = (check_a * 2.2).clamp(0.0, 1.0);
            let seg2 = ((check_a - 0.35) / 0.65).clamp(0.0, 1.0);
            if seg1 > 0.01 {
                self.d2d_context.DrawLine(
                    p0,
                    D2D_POINT_2F {
                        x: p0.x + (p1.x - p0.x) * seg1,
                        y: p0.y + (p1.y - p0.y) * seg1,
                    },
                    &self.solid_brush,
                    1.65,
                    round_st.as_ref(),
                );
            }
            if seg2 > 0.01 {
                self.d2d_context.DrawLine(
                    p1,
                    D2D_POINT_2F {
                        x: p1.x + (p2.x - p1.x) * seg2,
                        y: p1.y + (p2.y - p1.y) * seg2,
                    },
                    &self.solid_brush,
                    1.65,
                    round_st.as_ref(),
                );
            }
        }

        let c_cx = (close_r.left + close_r.right) * 0.5;
        let c_cy = (close_r.top + close_r.bottom) * 0.5;
        let cl_h = springs.ai_btn_hover[2].value().clamp(0.0, 1.0);
        if cl_h > 0.01 {
            let ph = 11.0 * (0.80 + 0.20 * cl_h);
            self.set_color(1.0, 1.0, 1.0, 0.11 * cl_h * card_a);
            self.d2d_context.FillRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: c_cx - ph,
                        top: c_cy - ph,
                        right: c_cx + ph,
                        bottom: c_cy + ph,
                    },
                    radiusX: 7.0,
                    radiusY: 7.0,
                },
                &self.solid_brush,
            );
        }
        let cs = 1.0 + 0.14 * cl_h;
        self.set_color(
            0.76 + 0.22 * cl_h,
            0.78 + 0.20 * cl_h,
            0.84 + 0.16 * cl_h,
            (0.55 + 0.42 * cl_h) * card_a,
        );
        self.d2d_context.DrawLine(
            D2D_POINT_2F { x: c_cx - 3.8 * cs, y: c_cy - 3.8 * cs },
            D2D_POINT_2F { x: c_cx + 3.8 * cs, y: c_cy + 3.8 * cs },
            &self.solid_brush,
            1.4,
            round_st.as_ref(),
        );
        self.d2d_context.DrawLine(
            D2D_POINT_2F { x: c_cx + 3.8 * cs, y: c_cy - 3.8 * cs },
            D2D_POINT_2F { x: c_cx - 3.8 * cs, y: c_cy + 3.8 * cs },
            &self.solid_brush,
            1.4,
            round_st.as_ref(),
        );

        let fu = springs.ai_followup_open.value().clamp(0.0, 1.0);
        let body_bottom = card_r.bottom - (16.0 + 26.0 * fu);
        let body_clip = D2D_RECT_F {
            left: card_r.left + 20.0,
            top: card_r.top + 38.0,
            right: card_r.right - 20.0,
            bottom: body_bottom,
        };
        let md_max_w = 510.0_f32;
        let visible_h = (body_clip.bottom - body_clip.top).max(24.0);
        let max_scroll = (content_h - visible_h).max(0.0);
        let scroll = springs.ai_scroll.value.clamp(0.0, max_scroll);
        self.d2d_context
            .PushAxisAlignedClip(&body_clip, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);

        let origin_x = body_clip.left;
        let origin_y = body_clip.top - scroll;
        let base_col = if is_err {
            (1.0, 0.48, 0.48)
        } else {
            (0.949, 0.949, 0.961)
        };

        if body_text.is_empty() && is_busy {
            if ((springs.phase_secs() * 2.0) as i32) % 2 == 0 {
                self.set_color(0.749, 0.353, 0.949, 0.95 * card_a);
                self.d2d_context.FillRoundedRectangle(
                    &D2D1_ROUNDED_RECT {
                        rect: D2D_RECT_F {
                            left: origin_x,
                            top: origin_y + 3.0,
                            right: origin_x + 7.0,
                            bottom: origin_y + 19.0,
                        },
                        radiusX: 2.0,
                        radiusY: 2.0,
                    },
                    &self.solid_brush,
                );
            }
        } else if !body_text.is_empty() {
            use crate::markdown::MdBlock;
            let blocks = crate::markdown::parse_markdown(body_text);
            let mut cur_y = origin_y;
            let mut caret_pos: Option<(f32, f32)> = None;

            let phase = springs.phase_secs();
            const BLUR_DUR: f32 = 0.38;
            let newest_age = if is_live_turn && !is_err {
                ai.chunks
                    .last()
                    .map(|&(_, _, birth)| (phase - birth).max(0.0))
                    .unwrap_or(BLUR_DUR)
            } else {
                BLUR_DUR
            };
            let tail_blur_active = newest_age < BLUR_DUR;

            let draw_inline_code_pills =
                |layout: &IDWriteTextLayout, rich: &crate::markdown::MdRichText, dx: f32, dy: f32| {
                    for sp in &rich.spans {
                        if sp.len_u16 == 0 || (!sp.code && sp.link_url.is_none()) {
                            continue;
                        }
                        let mut htm = [DWRITE_HIT_TEST_METRICS::default(); 6];
                        let mut cnt = 0u32;
                        let _ = layout.HitTestTextRange(
                            sp.start_u16,
                            sp.len_u16,
                            dx,
                            dy,
                            Some(&mut htm),
                            &mut cnt,
                        );
                        let n = (cnt as usize).min(htm.len());
                        if sp.code {
                            self.set_color(1.0, 1.0, 1.0, 0.095 * card_a);
                            for m in &htm[..n] {
                                self.d2d_context.FillRoundedRectangle(
                                    &D2D1_ROUNDED_RECT {
                                        rect: D2D_RECT_F {
                                            left: m.left - 2.8,
                                            top: m.top + 1.0,
                                            right: m.left + m.width + 2.8,
                                            bottom: m.top + m.height - 1.0,
                                        },
                                        radiusX: 4.5,
                                        radiusY: 4.5,
                                    },
                                    &self.solid_brush,
                                );
                            }
                        } else if sp.link_url.is_some() {
                            self.set_color(0.42, 0.72, 1.0, 0.14 * card_a);
                            for m in &htm[..n] {
                                self.d2d_context.FillRoundedRectangle(
                                    &D2D1_ROUNDED_RECT {
                                        rect: D2D_RECT_F {
                                            left: m.left - 2.2,
                                            top: m.top + 1.5,
                                            right: m.left + m.width + 2.2,
                                            bottom: m.top + m.height - 1.0,
                                        },
                                        radiusX: 4.0,
                                        radiusY: 4.0,
                                    },
                                    &self.solid_brush,
                                );
                            }
                        }
                    }
                };

            let sel_range = ai.normalized_selection();
            let draw_selection_bg =
                |layout: &IDWriteTextLayout, plain_u16: u32, seg_i: usize, dx: f32, dy: f32| {
                    let Some((sel_s, sel_e)) = sel_range else {
                        return;
                    };
                    if seg_i < sel_s.0 || seg_i > sel_e.0 || plain_u16 == 0 {
                        return;
                    }
                    let u_start = if seg_i == sel_s.0 {
                        sel_s.1.min(plain_u16)
                    } else {
                        0
                    };
                    let u_end = if seg_i == sel_e.0 {
                        sel_e.1.min(plain_u16)
                    } else {
                        plain_u16
                    };
                    if u_end <= u_start {
                        return;
                    }
                    let mut htm = [DWRITE_HIT_TEST_METRICS::default(); 32];
                    let mut cnt = 0u32;
                    let _ = layout.HitTestTextRange(
                        u_start,
                        u_end - u_start,
                        dx,
                        dy,
                        Some(&mut htm),
                        &mut cnt,
                    );
                    let n = (cnt as usize).min(htm.len());
                    self.set_color(0.56, 0.42, 0.98, 0.36 * card_a);
                    for m in &htm[..n] {
                        if m.width <= 0.2 {
                            continue;
                        }
                        self.d2d_context.FillRoundedRectangle(
                            &D2D1_ROUNDED_RECT {
                                rect: D2D_RECT_F {
                                    left: m.left - 1.0,
                                    top: m.top + 1.0,
                                    right: m.left + m.width + 1.0,
                                    bottom: m.top + m.height - 1.0,
                                },
                                radiusX: 3.5,
                                radiusY: 3.5,
                            },
                            &self.solid_brush,
                        );
                    }
                };
            let mut seg_idx = 0usize;

            for (idx, block) in blocks.iter().enumerate() {
                let is_last = idx + 1 == blocks.len();
                match block {
                    MdBlock::Heading { level, text } => {
                        if idx > 0 {
                            cur_y += 4.0;
                        }
                        let sz = match level {
                            1 => 17.5,
                            2 => 16.0,
                            3 => 15.0,
                            _ => 14.6,
                        };
                        if let Some(layout) =
                            self.build_md_text_layout(text, md_max_w, sz, true, false)
                        {
                            let h = Self::measure_layout_height(&layout);
                            let u16_len = text.plain.encode_utf16().count() as u32;
                            if cur_y + h >= body_clip.top - 20.0 && cur_y <= body_clip.bottom + 20.0
                            {
                                draw_inline_code_pills(&layout, text, origin_x, cur_y);
                                draw_selection_bg(&layout, u16_len, seg_idx, origin_x, cur_y);
                                self.set_color(1.0, 1.0, 1.0, 0.99 * card_a);
                                self.d2d_context.DrawTextLayout(
                                    D2D_POINT_2F { x: origin_x, y: cur_y },
                                    &layout,
                                    &self.solid_brush,
                                    D2D1_DRAW_TEXT_OPTIONS_NO_SNAP,
                                );
                            }
                            seg_idx += 1;
                            if is_last {
                                let mut px = 0.0;
                                let mut py = 0.0;
                                let mut htm = DWRITE_HIT_TEST_METRICS::default();
                                if layout
                                    .HitTestTextPosition(
                                        u16_len.saturating_sub(1),
                                        true,
                                        &mut px,
                                        &mut py,
                                        &mut htm,
                                    )
                                    .is_ok()
                                {
                                    caret_pos = Some((origin_x + px, cur_y + py));
                                }
                            }
                            cur_y += h + if is_last { 2.0 } else { 6.0 };
                        }
                    }
                    MdBlock::Paragraph(text) => {
                        if let Some(layout) =
                            self.build_md_text_layout(text, md_max_w, 14.6, false, false)
                        {
                            let h = Self::measure_layout_height(&layout);
                            let u16_len = text.plain.encode_utf16().count() as u32;
                            if cur_y + h >= body_clip.top - 20.0 && cur_y <= body_clip.bottom + 20.0
                            {
                                draw_inline_code_pills(&layout, text, origin_x, cur_y);
                                draw_selection_bg(&layout, u16_len, seg_idx, origin_x, cur_y);
                                if is_last && tail_blur_active {
                                    let t = (newest_age / BLUR_DUR).clamp(0.0, 1.0);
                                    let ease = 1.0 - (1.0 - t).powi(3);
                                    let blur_r = (1.0 - ease) * 1.8;
                                    if blur_r > 0.18 {
                                        let halo_a = (1.0 - ease) * ease * 0.20 * card_a;
                                        self.set_color(base_col.0, base_col.1, base_col.2, halo_a);
                                        for &(ox, oy) in &[(-blur_r, 0.0), (blur_r, 0.0)] {
                                            self.d2d_context.DrawTextLayout(
                                                D2D_POINT_2F {
                                                    x: origin_x + ox,
                                                    y: cur_y + oy,
                                                },
                                                &layout,
                                                &self.solid_brush,
                                                D2D1_DRAW_TEXT_OPTIONS_NO_SNAP,
                                            );
                                        }
                                    }
                                }
                                self.set_color(base_col.0, base_col.1, base_col.2, 0.98 * card_a);
                                self.d2d_context.DrawTextLayout(
                                    D2D_POINT_2F { x: origin_x, y: cur_y },
                                    &layout,
                                    &self.solid_brush,
                                    D2D1_DRAW_TEXT_OPTIONS_NO_SNAP,
                                );
                            }
                            seg_idx += 1;
                            if is_last {
                                let mut px = 0.0;
                                let mut py = 0.0;
                                let mut htm = DWRITE_HIT_TEST_METRICS::default();
                                if layout
                                    .HitTestTextPosition(
                                        u16_len.saturating_sub(1),
                                        true,
                                        &mut px,
                                        &mut py,
                                        &mut htm,
                                    )
                                    .is_ok()
                                {
                                    caret_pos = Some((origin_x + px, cur_y + py));
                                }
                            }
                            cur_y += h + if is_last { 2.0 } else { 8.0 };
                        }
                    }
                    MdBlock::CodeBlock { lang, code } => {
                        let display = if code.is_empty() { " " } else { code.as_str() };
                        let wide: Vec<u16> = display.encode_utf16().collect();
                        if let Ok(layout) = self.dwrite.CreateTextLayout(
                            &wide,
                            &self.fmt_ai_code,
                            (md_max_w - 24.0).max(40.0),
                            4000.0,
                        ) {
                            let code_h = Self::measure_layout_height(&layout);
                            let lang_hdr = if lang.is_empty() { 0.0 } else { 18.0 };
                            let box_h = lang_hdr + code_h + 18.0;
                            if cur_y + box_h >= body_clip.top - 20.0
                                && cur_y <= body_clip.bottom + 20.0
                            {
                                let box_r = D2D_RECT_F {
                                    left: origin_x,
                                    top: cur_y,
                                    right: origin_x + md_max_w,
                                    bottom: cur_y + box_h,
                                };
                                self.set_color(1.0, 1.0, 1.0, 0.055 * card_a);
                                self.d2d_context.FillRoundedRectangle(
                                    &D2D1_ROUNDED_RECT {
                                        rect: box_r,
                                        radiusX: 10.0,
                                        radiusY: 10.0,
                                    },
                                    &self.solid_brush,
                                );
                                self.set_color(1.0, 1.0, 1.0, 0.09 * card_a);
                                self.d2d_context.DrawRoundedRectangle(
                                    &D2D1_ROUNDED_RECT {
                                        rect: box_r,
                                        radiusX: 10.0,
                                        radiusY: 10.0,
                                    },
                                    &self.solid_brush,
                                    0.9,
                                    None,
                                );
                                if !lang.is_empty() {
                                    self.draw_text(
                                        lang,
                                        &self.fmt_label_xs,
                                        &D2D_RECT_F {
                                            left: box_r.left + 12.0,
                                            top: box_r.top + 5.0,
                                            right: box_r.right - 12.0,
                                            bottom: box_r.top + 19.0,
                                        },
                                        (0.68, 0.70, 0.78, 0.72 * card_a),
                                    );
                                }
                                let code_y = cur_y + lang_hdr + 9.0;
                                draw_selection_bg(
                                    &layout,
                                    code.encode_utf16().count() as u32,
                                    seg_idx,
                                    origin_x + 12.0,
                                    code_y,
                                );
                                self.set_color(0.88, 0.91, 0.98, 0.96 * card_a);
                                self.d2d_context.DrawTextLayout(
                                    D2D_POINT_2F {
                                        x: origin_x + 12.0,
                                        y: code_y,
                                    },
                                    &layout,
                                    &self.solid_brush,
                                    D2D1_DRAW_TEXT_OPTIONS_NO_SNAP,
                                );
                                if is_last {
                                    let mut px = 0.0;
                                    let mut py = 0.0;
                                    let mut htm = DWRITE_HIT_TEST_METRICS::default();
                                    if layout
                                        .HitTestTextPosition(
                                            wide.len().saturating_sub(1) as u32,
                                            true,
                                            &mut px,
                                            &mut py,
                                            &mut htm,
                                        )
                                        .is_ok()
                                    {
                                        caret_pos = Some((origin_x + 12.0 + px, code_y + py));
                                    }
                                }
                            }
                            seg_idx += 1;
                            cur_y += box_h + if is_last { 2.0 } else { 9.0 };
                        }
                    }
                    MdBlock::MathBlock(expr) => {
                        let display = if expr.is_empty() { " " } else { expr.as_str() };
                        let wide: Vec<u16> = display.encode_utf16().collect();
                        if let Ok(layout) = self.dwrite.CreateTextLayout(
                            &wide,
                            &self.fmt_ai_math,
                            (md_max_w - 24.0).max(40.0),
                            2000.0,
                        ) {
                            let math_h = Self::measure_layout_height(&layout);
                            let box_h = math_h + 18.0;
                            if cur_y + box_h >= body_clip.top - 20.0
                                && cur_y <= body_clip.bottom + 20.0
                            {
                                let box_r = D2D_RECT_F {
                                    left: origin_x,
                                    top: cur_y,
                                    right: origin_x + md_max_w,
                                    bottom: cur_y + box_h,
                                };
                                self.set_color(1.0, 1.0, 1.0, 0.042 * card_a);
                                self.d2d_context.FillRoundedRectangle(
                                    &D2D1_ROUNDED_RECT {
                                        rect: box_r,
                                        radiusX: 10.0,
                                        radiusY: 10.0,
                                    },
                                    &self.solid_brush,
                                );
                                self.set_color(1.0, 1.0, 1.0, 0.075 * card_a);
                                self.d2d_context.DrawRoundedRectangle(
                                    &D2D1_ROUNDED_RECT {
                                        rect: box_r,
                                        radiusX: 10.0,
                                        radiusY: 10.0,
                                    },
                                    &self.solid_brush,
                                    0.85,
                                    None,
                                );
                                let math_y = cur_y + 9.0;
                                draw_selection_bg(
                                    &layout,
                                    expr.encode_utf16().count() as u32,
                                    seg_idx,
                                    origin_x + 12.0,
                                    math_y,
                                );
                                self.set_color(0.97, 0.97, 1.0, 0.98 * card_a);
                                self.d2d_context.DrawTextLayout(
                                    D2D_POINT_2F {
                                        x: origin_x + 12.0,
                                        y: math_y,
                                    },
                                    &layout,
                                    &self.solid_brush,
                                    D2D1_DRAW_TEXT_OPTIONS_NO_SNAP,
                                );
                                if is_last {
                                    let mut px = 0.0;
                                    let mut py = 0.0;
                                    let mut htm = DWRITE_HIT_TEST_METRICS::default();
                                    if layout
                                        .HitTestTextPosition(
                                            wide.len().saturating_sub(1) as u32,
                                            true,
                                            &mut px,
                                            &mut py,
                                            &mut htm,
                                        )
                                        .is_ok()
                                    {
                                        caret_pos = Some((origin_x + 12.0 + px, math_y + py));
                                    }
                                }
                            }
                            seg_idx += 1;
                            cur_y += box_h + if is_last { 2.0 } else { 9.0 };
                        }
                    }
                    MdBlock::BulletItem { indent, text } => {
                        let left_pad = 16.0 + (*indent as f32) * 14.0;
                        if let Some(layout) = self.build_md_text_layout(
                            text,
                            (md_max_w - left_pad).max(40.0),
                            14.6,
                            false,
                            false,
                        ) {
                            let h = Self::measure_layout_height(&layout);
                            let u16_len = text.plain.encode_utf16().count() as u32;
                            if cur_y + h >= body_clip.top - 20.0 && cur_y <= body_clip.bottom + 20.0
                            {
                                self.set_color(0.75, 0.76, 0.84, 0.78 * card_a);
                                self.d2d_context.FillEllipse(
                                    &D2D1_ELLIPSE {
                                        point: D2D_POINT_2F {
                                            x: origin_x + left_pad - 9.5,
                                            y: cur_y + 11.2,
                                        },
                                        radiusX: 2.2,
                                        radiusY: 2.2,
                                    },
                                    &self.solid_brush,
                                );
                                draw_inline_code_pills(&layout, text, origin_x + left_pad, cur_y);
                                draw_selection_bg(
                                    &layout,
                                    u16_len,
                                    seg_idx,
                                    origin_x + left_pad,
                                    cur_y,
                                );
                                self.set_color(base_col.0, base_col.1, base_col.2, 0.98 * card_a);
                                self.d2d_context.DrawTextLayout(
                                    D2D_POINT_2F {
                                        x: origin_x + left_pad,
                                        y: cur_y,
                                    },
                                    &layout,
                                    &self.solid_brush,
                                    D2D1_DRAW_TEXT_OPTIONS_NO_SNAP,
                                );
                            }
                            seg_idx += 1;
                            if is_last {
                                let mut px = 0.0;
                                let mut py = 0.0;
                                let mut htm = DWRITE_HIT_TEST_METRICS::default();
                                if layout
                                    .HitTestTextPosition(
                                        u16_len.saturating_sub(1),
                                        true,
                                        &mut px,
                                        &mut py,
                                        &mut htm,
                                    )
                                    .is_ok()
                                {
                                    caret_pos = Some((origin_x + left_pad + px, cur_y + py));
                                }
                            }
                            cur_y += h + if is_last { 2.0 } else { 4.5 };
                        }
                    }
                    MdBlock::NumberedItem {
                        indent,
                        number,
                        text,
                    } => {
                        let left_pad = 24.0 + (*indent as f32) * 14.0;
                        if let Some(layout) = self.build_md_text_layout(
                            text,
                            (md_max_w - left_pad).max(40.0),
                            14.6,
                            false,
                            false,
                        ) {
                            let h = Self::measure_layout_height(&layout);
                            let u16_len = text.plain.encode_utf16().count() as u32;
                            if cur_y + h >= body_clip.top - 20.0 && cur_y <= body_clip.bottom + 20.0
                            {
                                self.draw_text(
                                    number,
                                    &self.fmt_artist,
                                    &D2D_RECT_F {
                                        left: origin_x + (*indent as f32) * 14.0,
                                        top: cur_y + 2.5,
                                        right: origin_x + left_pad - 4.0,
                                        bottom: cur_y + 21.0,
                                    },
                                    (0.68, 0.70, 0.78, 0.85 * card_a),
                                );
                                draw_inline_code_pills(&layout, text, origin_x + left_pad, cur_y);
                                draw_selection_bg(
                                    &layout,
                                    u16_len,
                                    seg_idx,
                                    origin_x + left_pad,
                                    cur_y,
                                );
                                self.set_color(base_col.0, base_col.1, base_col.2, 0.98 * card_a);
                                self.d2d_context.DrawTextLayout(
                                    D2D_POINT_2F {
                                        x: origin_x + left_pad,
                                        y: cur_y,
                                    },
                                    &layout,
                                    &self.solid_brush,
                                    D2D1_DRAW_TEXT_OPTIONS_NO_SNAP,
                                );
                            }
                            seg_idx += 1;
                            if is_last {
                                let mut px = 0.0;
                                let mut py = 0.0;
                                let mut htm = DWRITE_HIT_TEST_METRICS::default();
                                if layout
                                    .HitTestTextPosition(
                                        u16_len.saturating_sub(1),
                                        true,
                                        &mut px,
                                        &mut py,
                                        &mut htm,
                                    )
                                    .is_ok()
                                {
                                    caret_pos = Some((origin_x + left_pad + px, cur_y + py));
                                }
                            }
                            cur_y += h + if is_last { 2.0 } else { 4.5 };
                        }
                    }
                    MdBlock::Blockquote(text) => {
                        if let Some(layout) = self.build_md_text_layout(
                            text,
                            (md_max_w - 16.0).max(40.0),
                            14.2,
                            false,
                            true,
                        ) {
                            let h = Self::measure_layout_height(&layout);
                            let u16_len = text.plain.encode_utf16().count() as u32;
                            if cur_y + h + 4.0 >= body_clip.top - 20.0
                                && cur_y <= body_clip.bottom + 20.0
                            {
                                self.set_color(0.749, 0.353, 0.949, 0.58 * card_a);
                                self.d2d_context.FillRoundedRectangle(
                                    &D2D1_ROUNDED_RECT {
                                        rect: D2D_RECT_F {
                                            left: origin_x + 2.0,
                                            top: cur_y + 2.0,
                                            right: origin_x + 5.0,
                                            bottom: cur_y + h + 2.0,
                                        },
                                        radiusX: 1.5,
                                        radiusY: 1.5,
                                    },
                                    &self.solid_brush,
                                );
                                draw_inline_code_pills(&layout, text, origin_x + 14.0, cur_y + 2.0);
                                draw_selection_bg(
                                    &layout,
                                    u16_len,
                                    seg_idx,
                                    origin_x + 14.0,
                                    cur_y + 2.0,
                                );
                                self.set_color(0.82, 0.83, 0.88, 0.92 * card_a);
                                self.d2d_context.DrawTextLayout(
                                    D2D_POINT_2F {
                                        x: origin_x + 14.0,
                                        y: cur_y + 2.0,
                                    },
                                    &layout,
                                    &self.solid_brush,
                                    D2D1_DRAW_TEXT_OPTIONS_NO_SNAP,
                                );
                            }
                            seg_idx += 1;
                            cur_y += h + 4.0 + if is_last { 2.0 } else { 8.0 };
                        }
                    }
                    MdBlock::HorizontalRule => {
                        if cur_y + 14.0 >= body_clip.top - 10.0 && cur_y <= body_clip.bottom + 10.0
                        {
                            self.set_color(1.0, 1.0, 1.0, 0.12 * card_a);
                            self.d2d_context.DrawLine(
                                D2D_POINT_2F {
                                    x: origin_x,
                                    y: cur_y + 7.0,
                                },
                                D2D_POINT_2F {
                                    x: origin_x + md_max_w,
                                    y: cur_y + 7.0,
                                },
                                &self.solid_brush,
                                1.0,
                                None,
                            );
                        }
                        cur_y += 14.0;
                    }
                    MdBlock::Table { headers, rows } => {
                        let cols = headers.len().max(1);
                        let col_w = ((md_max_w - 16.0) / cols as f32).max(36.0);
                        let mut hdr_layouts = Vec::with_capacity(cols);
                        let mut hdr_h = 22.0_f32;
                        for hcell in headers {
                            let l = self.build_md_text_layout(
                                hcell,
                                (col_w - 10.0).max(24.0),
                                13.0,
                                true,
                                false,
                            );
                            if let Some(ref lay) = l {
                                hdr_h = hdr_h.max(Self::measure_layout_height(lay) + 8.0);
                            }
                            hdr_layouts.push(l);
                        }
                        let mut row_layouts = Vec::with_capacity(rows.len());
                        let mut total_h = hdr_h;
                        for row in rows {
                            let mut rh = 20.0_f32;
                            let mut lays = Vec::with_capacity(cols);
                            for cell in row {
                                let l = self.build_md_text_layout(
                                    cell,
                                    (col_w - 10.0).max(24.0),
                                    13.0,
                                    false,
                                    false,
                                );
                                if let Some(ref lay) = l {
                                    rh = rh.max(Self::measure_layout_height(lay) + 8.0);
                                }
                                lays.push(l);
                            }
                            total_h += rh;
                            row_layouts.push((rh, lays));
                        }
                        let box_h = total_h + 4.0;
                        if cur_y + box_h >= body_clip.top - 20.0
                            && cur_y <= body_clip.bottom + 20.0
                        {
                            let tbl_r = D2D_RECT_F {
                                left: origin_x,
                                top: cur_y,
                                right: origin_x + md_max_w,
                                bottom: cur_y + box_h,
                            };
                            self.set_color(1.0, 1.0, 1.0, 0.045 * card_a);
                            self.d2d_context.FillRoundedRectangle(
                                &D2D1_ROUNDED_RECT {
                                    rect: tbl_r,
                                    radiusX: 8.0,
                                    radiusY: 8.0,
                                },
                                &self.solid_brush,
                            );
                            self.set_color(1.0, 1.0, 1.0, 0.09 * card_a);
                            self.d2d_context.DrawRoundedRectangle(
                                &D2D1_ROUNDED_RECT {
                                    rect: tbl_r,
                                    radiusX: 8.0,
                                    radiusY: 8.0,
                                },
                                &self.solid_brush,
                                0.85,
                                None,
                            );
                            for (c, lay_opt) in hdr_layouts.iter().enumerate() {
                                if let Some(lay) = lay_opt {
                                    let u16_len = headers
                                        .get(c)
                                        .map(|h| h.plain.encode_utf16().count() as u32)
                                        .unwrap_or(0);
                                    draw_selection_bg(
                                        lay,
                                        u16_len,
                                        seg_idx,
                                        origin_x + 8.0 + c as f32 * col_w,
                                        cur_y + 4.0,
                                    );
                                    self.set_color(0.98, 0.98, 1.0, 0.96 * card_a);
                                    self.d2d_context.DrawTextLayout(
                                        D2D_POINT_2F {
                                            x: origin_x + 8.0 + c as f32 * col_w,
                                            y: cur_y + 4.0,
                                        },
                                        lay,
                                        &self.solid_brush,
                                        D2D1_DRAW_TEXT_OPTIONS_NO_SNAP,
                                    );
                                    seg_idx += 1;
                                }
                            }
                            let mut ry = cur_y + hdr_h;
                            self.set_color(1.0, 1.0, 1.0, 0.11 * card_a);
                            self.d2d_context.DrawLine(
                                D2D_POINT_2F {
                                    x: tbl_r.left + 1.0,
                                    y: ry,
                                },
                                D2D_POINT_2F {
                                    x: tbl_r.right - 1.0,
                                    y: ry,
                                },
                                &self.solid_brush,
                                0.9,
                                None,
                            );
                            for (r_idx, (rh, lays)) in row_layouts.iter().enumerate() {
                                for (c, lay_opt) in lays.iter().enumerate() {
                                    if let Some(lay) = lay_opt {
                                        let u16_len = rows
                                            .get(r_idx)
                                            .and_then(|r| r.get(c))
                                            .map(|cell| cell.plain.encode_utf16().count() as u32)
                                            .unwrap_or(0);
                                        draw_selection_bg(
                                            lay,
                                            u16_len,
                                            seg_idx,
                                            origin_x + 8.0 + c as f32 * col_w,
                                            ry + 4.0,
                                        );
                                        self.set_color(
                                            base_col.0,
                                            base_col.1,
                                            base_col.2,
                                            0.94 * card_a,
                                        );
                                        self.d2d_context.DrawTextLayout(
                                            D2D_POINT_2F {
                                                x: origin_x + 8.0 + c as f32 * col_w,
                                                y: ry + 4.0,
                                            },
                                            lay,
                                            &self.solid_brush,
                                            D2D1_DRAW_TEXT_OPTIONS_NO_SNAP,
                                        );
                                        seg_idx += 1;
                                    }
                                }
                                ry += *rh;
                                if r_idx + 1 < row_layouts.len() {
                                    self.set_color(1.0, 1.0, 1.0, 0.06 * card_a);
                                    self.d2d_context.DrawLine(
                                        D2D_POINT_2F {
                                            x: tbl_r.left + 6.0,
                                            y: ry,
                                        },
                                        D2D_POINT_2F {
                                            x: tbl_r.right - 6.0,
                                            y: ry,
                                        },
                                        &self.solid_brush,
                                        0.75,
                                        None,
                                    );
                                }
                            }
                        } else {
                            seg_idx += hdr_layouts.iter().filter(|l| l.is_some()).count();
                            for (_, lays) in &row_layouts {
                                seg_idx += lays.iter().filter(|l| l.is_some()).count();
                            }
                        }
                        cur_y += box_h + if is_last { 2.0 } else { 9.0 };
                    }
                }
            }

            if is_busy && !is_err && ((springs.phase_secs() * 2.0) as i32) % 2 == 0 {
                if let Some((cx, cy)) = caret_pos {
                    self.set_color(0.749, 0.353, 0.949, 0.95 * card_a);
                    self.d2d_context.FillRoundedRectangle(
                        &D2D1_ROUNDED_RECT {
                            rect: D2D_RECT_F {
                                left: cx + 3.0,
                                top: cy + 2.5,
                                right: cx + 9.5,
                                bottom: cy + 18.0,
                            },
                            radiusX: 2.0,
                            radiusY: 2.0,
                        },
                        &self.solid_brush,
                    );
                }
            }
        }
        if scroll > 2.0 {
            let fade_h = 18.0;
            let stops = [
                D2D1_GRADIENT_STOP {
                    position: 0.0,
                    color: D2D1_COLOR_F { r: 0.078, g: 0.078, b: 0.094, a: 0.96 * card_a },
                },
                D2D1_GRADIENT_STOP {
                    position: 1.0,
                    color: D2D1_COLOR_F { r: 0.078, g: 0.078, b: 0.094, a: 0.0 },
                },
            ];
            if let Ok(sc) = ID2D1RenderTarget::CreateGradientStopCollection(
                &self.d2d_context,
                &stops,
                D2D1_GAMMA_2_2,
                D2D1_EXTEND_MODE_CLAMP,
            ) {
                if let Ok(lg) = self.d2d_context.CreateLinearGradientBrush(
                    &D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                        startPoint: D2D_POINT_2F { x: body_clip.left, y: body_clip.top },
                        endPoint: D2D_POINT_2F { x: body_clip.left, y: body_clip.top + fade_h },
                    },
                    None,
                    &sc,
                ) {
                    self.d2d_context.FillRectangle(
                        &D2D_RECT_F {
                            left: body_clip.left,
                            top: body_clip.top,
                            right: body_clip.right,
                            bottom: body_clip.top + fade_h,
                        },
                        &lg,
                    );
                }
            }
        }
        if max_scroll - scroll > 2.0 {
            let fade_h = 18.0;
            let stops = [
                D2D1_GRADIENT_STOP {
                    position: 0.0,
                    color: D2D1_COLOR_F { r: 0.078, g: 0.078, b: 0.094, a: 0.0 },
                },
                D2D1_GRADIENT_STOP {
                    position: 1.0,
                    color: D2D1_COLOR_F { r: 0.078, g: 0.078, b: 0.094, a: 0.96 * card_a },
                },
            ];
            if let Ok(sc) = ID2D1RenderTarget::CreateGradientStopCollection(
                &self.d2d_context,
                &stops,
                D2D1_GAMMA_2_2,
                D2D1_EXTEND_MODE_CLAMP,
            ) {
                if let Ok(lg) = self.d2d_context.CreateLinearGradientBrush(
                    &D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                        startPoint: D2D_POINT_2F { x: body_clip.left, y: body_clip.bottom - fade_h },
                        endPoint: D2D_POINT_2F { x: body_clip.left, y: body_clip.bottom },
                    },
                    None,
                    &sc,
                ) {
                    self.d2d_context.FillRectangle(
                        &D2D_RECT_F {
                            left: body_clip.left,
                            top: body_clip.bottom - fade_h,
                            right: body_clip.right,
                            bottom: body_clip.bottom,
                        },
                        &lg,
                    );
                }
            }
        }
        self.d2d_context.PopAxisAlignedClip();

        let sb_alpha = springs.ai_scrollbar_vis.value().clamp(0.0, 1.0);
        let smooth_content_h = if springs.ai_content_h.value > 20.0 {
            springs.ai_content_h.value.max(visible_h)
        } else {
            content_h.max(visible_h)
        };
        let smooth_max_scroll = (smooth_content_h - visible_h).max(0.0);
        if sb_alpha > 0.01 {
            let track_top = body_clip.top + 3.0;
            let track_bot = body_clip.bottom - 3.0;
            let track_h = (track_bot - track_top).max(20.0);
            let sb_x = card_r.right - 11.5;
            let sb_w = 3.5_f32;
            self.set_color(1.0, 1.0, 1.0, 0.065 * sb_alpha * card_a);
            self.d2d_context.FillRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: sb_x,
                        top: track_top,
                        right: sb_x + sb_w,
                        bottom: track_bot,
                    },
                    radiusX: sb_w * 0.5,
                    radiusY: sb_w * 0.5,
                },
                &self.solid_brush,
            );
            let ratio = (visible_h / smooth_content_h.max(1.0)).clamp(0.08, 1.0);
            let thumb_h = (track_h * ratio).clamp(24.0, track_h);
            let frac = if smooth_max_scroll > 0.5 {
                (scroll / smooth_max_scroll).clamp(0.0, 1.0)
            } else {
                1.0
            };
            let thumb_top = track_top + frac * (track_h - thumb_h);
            self.set_color(1.0, 1.0, 1.0, 0.38 * sb_alpha * card_a);
            self.d2d_context.FillRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: sb_x,
                        top: thumb_top,
                        right: sb_x + sb_w,
                        bottom: thumb_top + thumb_h,
                    },
                    radiusX: sb_w * 0.5,
                    radiusY: sb_w * 0.5,
                },
                &self.solid_brush,
            );
        }

        let total_turns = ai.total_turns();
        if total_turns >= 2 {
            let active_idx = ai.active_turn_index();
            let dot_rects = ai_turn_dot_rects(card_r, total_turns);
            if let (Some(first), Some(last)) = (dot_rects.first(), dot_rects.last()) {
                let track_cx = (first.left + first.right) * 0.5;
                let track_r = D2D_RECT_F {
                    left: track_cx - 6.5,
                    top: (first.top + first.bottom) * 0.5 - 10.0,
                    right: track_cx + 6.5,
                    bottom: (last.top + last.bottom) * 0.5 + 10.0,
                };
                self.set_color(0.078, 0.078, 0.094, 0.86 * card_a);
                self.d2d_context.FillRoundedRectangle(
                    &D2D1_ROUNDED_RECT {
                        rect: track_r,
                        radiusX: 6.5,
                        radiusY: 6.5,
                    },
                    &self.solid_brush,
                );
                self.set_color(1.0, 1.0, 1.0, 0.09 * card_a);
                self.d2d_context.DrawRoundedRectangle(
                    &D2D1_ROUNDED_RECT {
                        rect: track_r,
                        radiusX: 6.5,
                        radiusY: 6.5,
                    },
                    &self.solid_brush,
                    0.85,
                    None,
                );
            }
            for (i, dr) in dot_rects.iter().enumerate() {
                let dcx = (dr.left + dr.right) * 0.5;
                let dcy = (dr.top + dr.bottom) * 0.5;
                if i == active_idx {
                    self.set_color(0.95, 0.96, 0.98, 0.96 * card_a);
                    self.d2d_context.FillRoundedRectangle(
                        &D2D1_ROUNDED_RECT {
                            rect: D2D_RECT_F {
                                left: dcx - 2.3,
                                top: dcy - 5.2,
                                right: dcx + 2.3,
                                bottom: dcy + 5.2,
                            },
                            radiusX: 2.3,
                            radiusY: 2.3,
                        },
                        &self.solid_brush,
                    );
                } else {
                    self.set_color(1.0, 1.0, 1.0, 0.34 * card_a);
                    self.d2d_context.FillEllipse(
                        &D2D1_ELLIPSE {
                            point: D2D_POINT_2F { x: dcx, y: dcy },
                            radiusX: 2.3,
                            radiusY: 2.3,
                        },
                        &self.solid_brush,
                    );
                }
            }
        }

        if fu > 0.02 {
            let fu_clip = D2D_RECT_F {
                left: card_r.left,
                top: body_bottom.max(card_r.top + 38.0),
                right: card_r.right,
                bottom: card_r.bottom - 2.0,
            };
            if fu_clip.bottom > fu_clip.top + 1.0 {
                self.d2d_context
                    .PushAxisAlignedClip(&fu_clip, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
                let slide_fu = (1.0 - fu) * 26.0;
                let fu_fade = if show_followup && springs.ai_card_open.target() > 0.5 {
                    fu
                } else {
                    fu.powf(2.6)
                };
                let fu_a = fu_fade * card_a;
                let fy = followup_r.top + slide_fu;
                self.set_color(1.0, 1.0, 1.0, 0.065 * fu_a);
                self.d2d_context.DrawLine(
                    D2D_POINT_2F { x: card_r.left + 20.0, y: fy },
                    D2D_POINT_2F { x: card_r.right - 20.0, y: fy },
                    &self.solid_brush,
                    0.85,
                    None,
                );
                let f_cy = (followup_r.top + followup_r.bottom) * 0.5 + 1.0 + slide_fu;
                self.draw_ai_sparkle_stroked(
                    card_r.left + 28.0,
                    f_cy,
                    5.4,
                    1.35,
                    (1.0, 1.0, 1.0, 0.42 * fu_a),
                );
                self.draw_text(
                    "Scrivi per continuare la conversazione…",
                    &self.fmt_artist,
                    &D2D_RECT_F {
                        left: card_r.left + 42.0,
                        top: followup_r.top + 2.0 + slide_fu,
                        right: card_r.right - 46.0,
                        bottom: followup_r.bottom + slide_fu,
                    },
                    (0.56, 0.58, 0.64, 0.84 * fu_a),
                );
                self.draw_return_arrow_icon(
                    card_r.right - 26.0,
                    f_cy,
                    0.85,
                    1.3,
                    (1.0, 1.0, 1.0, 0.36 * fu_a),
                );
                self.d2d_context.PopAxisAlignedClip();
            }
        }

        if (scale - 1.0).abs() > 0.001 || slide_y.abs() > 0.05 {
            self.d2d_context.SetTransform(&prev_xf);
        }
    }

    unsafe fn draw_dark_backdrop_highlight(
        &self,
        geo: &ID2D1PathGeometry,
        cur_h: f32,
        amount: f32,
    ) {
        if amount <= 0.005 {
            return;
        }
        let clip = D2D_RECT_F {
            left: 0.0,
            top: 0.85,
            right: self.max_w_dips,
            bottom: self.max_h_dips,
        };
        self.d2d_context
            .PushAxisAlignedClip(&clip, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
        let stops = [
            D2D1_GRADIENT_STOP {
                position: 0.0,
                color: D2D1_COLOR_F {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: 0.02 * amount,
                },
            },
            D2D1_GRADIENT_STOP {
                position: 0.35,
                color: D2D1_COLOR_F {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: 0.075 * amount,
                },
            },
            D2D1_GRADIENT_STOP {
                position: 1.0,
                color: D2D1_COLOR_F {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: 0.115 * amount,
                },
            },
        ];
        let drawn = ID2D1RenderTarget::CreateGradientStopCollection(
            &self.d2d_context,
            &stops,
            D2D1_GAMMA_2_2,
            D2D1_EXTEND_MODE_CLAMP,
        )
        .ok()
        .and_then(|sc| {
            let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                startPoint: D2D_POINT_2F { x: 0.0, y: 0.0 },
                endPoint: D2D_POINT_2F {
                    x: 0.0,
                    y: cur_h.max(16.0),
                },
            };
            self.d2d_context
                .CreateLinearGradientBrush(&props, None, &sc)
                .ok()
        });
        if let Some(grad) = drawn {
            self.d2d_context.DrawGeometry(geo, &grad, 0.9, None);
        } else {
            self.set_color(1.0, 1.0, 1.0, 0.095 * amount);
            self.d2d_context
                .DrawGeometry(geo, &self.solid_brush, 0.9, None);
        }
        self.d2d_context.PopAxisAlignedClip();
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn draw_scene_translucent(
        &self,
        cfg: &Config,
        springs: &NotchSprings,
        state: &EaveViewState<'_>,
        hover_target: HitTarget,
        cur_w: f32,
        cur_h: f32,
        ear_r: f32,
        bottom_r: f32,
        expand: f32,
        canvas_cx: f32,
        pill_left: f32,
        notch_geo: Option<&ID2D1PathGeometry>,
    ) -> Result<()> {
        self.draw_drop_shadow(pill_left, cur_w, cur_h, ear_r, bottom_r, expand);

        if let Some(geo) = notch_geo {
            if cfg.glass_tint == [0, 0, 0, 255] {
                self.set_color(0.0, 0.0, 0.0, 1.0);
            } else {
                let tint = cfg.glass_tint_f32();
                self.set_color(tint[0], tint[1], tint[2], tint[3]);
            }
            self.d2d_context.FillGeometry(geo, &self.solid_brush, None);

            let mask_geom: ID2D1Geometry = geo.cast()?;
            let mut layer_params = D2D1_LAYER_PARAMETERS1 {
                contentBounds: D2D_RECT_F {
                    left: 0.0,
                    top: 0.0,
                    right: self.max_w_dips,
                    bottom: self.max_h_dips,
                },
                geometricMask: ManuallyDrop::new(Some(mask_geom)),
                maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                maskTransform: Matrix3x2::identity(),
                opacity: 1.0,
                opacityBrush: ManuallyDrop::new(None),
                layerOptions: D2D1_LAYER_OPTIONS1_NONE,
            };
            self.d2d_context.PushLayer(&layer_params, None);
            self.draw_notch_interior(
                cfg,
                springs,
                state,
                hover_target,
                cur_w,
                cur_h,
                ear_r,
                expand,
                canvas_cx,
                pill_left,
                geo,
                false,
            );
            self.d2d_context.PopLayer();
            self.draw_dark_backdrop_highlight(geo, cur_h, springs.dark_bg_border());
            ManuallyDrop::drop(&mut layer_params.geometricMask);
        }

        Ok(())
    }

    fn sync_album_art(&mut self, thumbnail: Option<AlbumArtView<'_>>, keep_for_fade: bool) {
        match thumbnail {
            Some(art)
                if art.width > 0
                    && art.height > 0
                    && art.bgra.len() >= (art.width * art.height * 4) as usize =>
            {
                if art.revision != self.cached_art_revision || self.cached_art_bitmap.is_none() {
                    let props = D2D1_BITMAP_PROPERTIES1 {
                        pixelFormat: D2D1_PIXEL_FORMAT {
                            format: DXGI_FORMAT_B8G8R8A8_UNORM,
                            alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                        },
                        dpiX: 96.0,
                        dpiY: 96.0,
                        bitmapOptions: Default::default(),
                        ..Default::default()
                    };
                    let size = D2D_SIZE_U {
                        width: art.width,
                        height: art.height,
                    };
                    let pitch = art.width * 4;
                    let bmp = unsafe {
                        self.d2d_context
                            .CreateBitmap(size, Some(art.bgra.as_ptr() as *const _), pitch, &props)
                            .ok()
                    };
                    if self.cached_art_revision != 0 && self.cached_art_bitmap.is_some() {
                        self.prev_art_bitmap = self.cached_art_bitmap.take();
                    }
                    self.cached_art_bitmap = bmp;
                    self.cached_art_revision = art.revision;
                }
            }
            _ if !keep_for_fade => {
                self.cached_art_bitmap = None;
                self.prev_art_bitmap = None;
                self.cached_art_revision = 0;
            }
            _ => {}
        }
    }

    fn sync_shelf_icons(&mut self, shelf: &ShelfStore) {
        let props = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            bitmapOptions: Default::default(),
            ..Default::default()
        };
        for item in &shelf.items {
            let Some(bgra) = item.icon_bgra.as_ref() else {
                continue;
            };
            if item.icon_width == 0
                || item.icon_height == 0
                || bgra.len() < (item.icon_width * item.icon_height * 4) as usize
            {
                continue;
            }
            let key = shelf_icon_hash(item);
            if self.shell_icon_cache.iter().any(|(k, _)| *k == key) {
                continue;
            }
            let size = D2D_SIZE_U {
                width: item.icon_width,
                height: item.icon_height,
            };
            let pitch = item.icon_width * 4;
            let bmp = unsafe {
                self.d2d_context
                    .CreateBitmap(size, Some(bgra.as_ptr() as *const _), pitch, &props)
                    .ok()
            };
            if let Some(bmp) = bmp {
                if self.shell_icon_cache.len() >= 64 {
                    self.shell_icon_cache.remove(0);
                }
                self.shell_icon_cache.push((key, bmp));
            }
        }
    }

    fn cached_shelf_icon(&self, item: &ShelfItem) -> Option<&ID2D1Bitmap1> {
        item.icon_bgra.as_ref()?;
        let key = shelf_icon_hash(item);
        self.shell_icon_cache
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, bmp)| bmp)
    }

    fn sync_toast_icon(&mut self, toast: Option<&crate::osd_monitors::ToastNotification>) {
        let Some(t) = toast else {
            return;
        };
        let Some(bgra) = t.icon_bgra.as_ref() else {
            self.toast_icon_bmp = None;
            return;
        };
        if t.icon_w == 0 || t.icon_h == 0 || bgra.len() < (t.icon_w * t.icon_h * 4) as usize {
            self.toast_icon_bmp = None;
            return;
        }
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in t.app.as_bytes().iter().chain(t.title.as_bytes()).chain(bgra.iter().take(32)) {
            h ^= *b as u64;
            h = h.wrapping_mul(0x1000_0000_01b3);
        }
        if self.toast_icon_bmp.as_ref().is_some_and(|(k, _)| *k == h) {
            return;
        }
        let props = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            bitmapOptions: Default::default(),
            colorContext: ManuallyDrop::new(None),
        };
        let size = D2D_SIZE_U {
            width: t.icon_w,
            height: t.icon_h,
        };
        let pitch = t.icon_w * 4;
        self.toast_icon_bmp = unsafe {
            self.d2d_context
                .CreateBitmap(size, Some(bgra.as_ptr() as *const _), pitch, &props)
                .ok()
                .map(|bmp| (h, bmp))
        };
    }

    fn sync_dynamic_icons(&mut self, springs: &NotchSprings, state: &EaveViewState<'_>) {
        let raw_vol = springs.speaker_volume().clamp(0.0, 100.0);
        let vol = if (state.muted || springs.speaker_volume.target <= 0.05) && raw_vol < 0.25 {
            0.0
        } else {
            raw_vol
        };
        let max_bars =
            if springs.osd_open() > 0.05 && state.collapsed_osd == CollapsedOsdKind::Volume {
                3
            } else {
                2
            };
        let key = ((vol * 4.0).round() as u16, max_bars);
        if self.cached_speaker_key == Some(key) && self.speaker_2x_bmp.is_some() {
            return;
        }
        let bake_vol = if key.0 == 0 { 0.0 } else { key.0 as f32 * 0.25 };
        let Some(bmp) = &self.speaker_2x_bmp else {
            return;
        };
        let (_, atlas_dpi) = atlas_cell_metrics(self.dpi);
        unsafe {
            self.d2d_context.SetTarget(bmp);
            self.d2d_context.SetDpi(atlas_dpi, atlas_dpi);
            self.d2d_context.BeginDraw();
            self.d2d_context.Clear(Some(&D2D1_COLOR_F {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.0,
            }));
            self.set_color(1.0, 1.0, 1.0, 1.0);
            let brush: &ID2D1Brush = &self.solid_brush;
            icons::draw_speaker_volume_bars(
                &self.d2d_context,
                ATLAS_CELL_DIPS * 0.5,
                ATLAS_CELL_DIPS * 0.5,
                20.0,
                brush,
                bake_vol,
                max_bars,
            );
            if self.d2d_context.EndDraw(None, None).is_ok() {
                self.cached_speaker_key = Some(key);
            }
        }
    }

    unsafe fn draw_ss_icon(
        &self,
        slot: usize,
        cx: f32,
        cy: f32,
        size: f32,
        alpha: f32,
    ) -> bool {
        if size <= 0.0 || alpha <= 0.004 {
            return true;
        }
        let Some(atlas) = &self.icon_atlas_2x else {
            return false;
        };
        let Some(&bake_size) = ATLAS_BAKE_SIZES.get(slot) else {
            return false;
        };
        let col = (slot % ATLAS_COLS) as f32;
        let row = (slot / ATLAS_COLS) as f32;
        let cell_left = col * ATLAS_CELL_DIPS;
        let cell_top = row * ATLAS_CELL_DIPS;
        let src_rect = D2D_RECT_F {
            left: cell_left + 0.5,
            top: cell_top + 0.5,
            right: cell_left + ATLAS_CELL_DIPS - 0.5,
            bottom: cell_top + ATLAS_CELL_DIPS - 0.5,
        };
        let draw_span = (ATLAS_CELL_DIPS - 1.0) * (size / bake_size);
        let half = draw_span * 0.5;
        let dst_rect = D2D_RECT_F {
            left: cx - half,
            top: cy - half,
            right: cx + half,
            bottom: cy + half,
        };
        self.d2d_context.DrawBitmap(
            atlas,
            Some(&dst_rect),
            alpha,
            D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
            Some(&src_rect),
            None,
        );
        true
    }

    unsafe fn draw_ss_speaker(
        &self,
        cx: f32,
        cy: f32,
        size: f32,
        alpha: f32,
        vol: f32,
        max_bars: u8,
    ) {
        if size <= 0.0 || alpha <= 0.004 {
            return;
        }
        if let Some(bmp) = &self.speaker_2x_bmp {
            if self.cached_speaker_key.is_some_and(|(_, b)| b == max_bars) {
                let src_rect = D2D_RECT_F {
                    left: 0.5,
                    top: 0.5,
                    right: ATLAS_CELL_DIPS - 0.5,
                    bottom: ATLAS_CELL_DIPS - 0.5,
                };
                let draw_span = (ATLAS_CELL_DIPS - 1.0) * (size / 20.0);
                let half = draw_span * 0.5;
                let dst_rect = D2D_RECT_F {
                    left: cx - half,
                    top: cy - half,
                    right: cx + half,
                    bottom: cy + half,
                };
                self.d2d_context.DrawBitmap(
                    bmp,
                    Some(&dst_rect),
                    alpha,
                    D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
                    Some(&src_rect),
                    None,
                );
                return;
            }
        }
        self.set_color(1.0, 1.0, 1.0, alpha);
        let brush: &ID2D1Brush = &self.solid_brush;
        icons::draw_speaker_volume_bars(&self.d2d_context, cx, cy, size, brush, vol, max_bars);
    }

    unsafe fn set_color(&self, r: f32, g: f32, b: f32, a: f32) {
        self.solid_brush.SetColor(&D2D1_COLOR_F { r, g, b, a });
    }

    unsafe fn set_alt_color(&self, r: f32, g: f32, b: f32, a: f32) {
        self.alt_brush.SetColor(&D2D1_COLOR_F { r, g, b, a });
    }

    #[allow(dead_code)]
    pub fn measure_closed_lyric_target_width(&self, text: &str) -> f32 {
        let text_w = unsafe { self.measure_width(text, &self.fmt_closed_lyric) };
        let body_w = (20.0 + 21.5 + 12.0 + text_w + 20.0).clamp(300.0, 520.0);
        body_w + 22.0
    }

    unsafe fn measure_multiline_height(
        &self,
        text: &str,
        format: &IDWriteTextFormat,
        max_w: f32,
    ) -> f32 {
        let wide: Vec<u16> = text.encode_utf16().collect();
        if let Ok(layout) = self.dwrite.CreateTextLayout(&wide, format, max_w, 160.0) {
            let mut metrics = DWRITE_TEXT_METRICS::default();
            if layout.GetMetrics(&mut metrics).is_ok() {
                return metrics.height;
            }
        }
        22.0
    }

    unsafe fn measure_width(&self, text: &str, format: &IDWriteTextFormat) -> f32 {
        let wide: Vec<u16> = text.encode_utf16().collect();
        if let Ok(layout) = self.dwrite.CreateTextLayout(&wide, format, 600.0, 48.0) {
            let mut metrics = DWRITE_TEXT_METRICS::default();
            if layout.GetMetrics(&mut metrics).is_ok() {
                return metrics.widthIncludingTrailingWhitespace;
            }
        }
        text.len() as f32 * 7.5
    }

    fn bind_cached_swap_chain_target(&mut self) -> Result<()> {
        if self.swap_chain_target.is_none() {
            self.swap_chain_target = Some(create_swap_chain_target(
                &self.d2d_context,
                &self.swap_chain,
                self.dpi,
            )?);
        }
        if let Some(target) = &self.swap_chain_target {
            unsafe {
                self.d2d_context.SetTarget(target);
                self.d2d_context.SetDpi(self.dpi, self.dpi);
            }
        }
        Ok(())
    }

    fn sync_eave_mask_brush(&mut self, idle_h: f32) {
        let key = (idle_h * 10.0).round() as u32;
        if self.eave_mask_brush.as_ref().map(|(k, _)| *k) == Some(key) {
            return;
        }
        let mask_stops = [
            D2D1_GRADIENT_STOP {
                position: 0.0,
                color: D2D1_COLOR_F { r: 1.0, g: 1.0, b: 1.0, a: 0.0 },
            },
            D2D1_GRADIENT_STOP {
                position: 0.24,
                color: D2D1_COLOR_F { r: 1.0, g: 1.0, b: 1.0, a: 0.04 },
            },
            D2D1_GRADIENT_STOP {
                position: 0.66,
                color: D2D1_COLOR_F { r: 1.0, g: 1.0, b: 1.0, a: 0.52 },
            },
            D2D1_GRADIENT_STOP {
                position: 1.0,
                color: D2D1_COLOR_F { r: 1.0, g: 1.0, b: 1.0, a: 0.96 },
            },
        ];
        let brush_opt: Option<ID2D1Brush> = unsafe {
            ID2D1RenderTarget::CreateGradientStopCollection(
                &self.d2d_context,
                &mask_stops,
                D2D1_GAMMA_2_2,
                D2D1_EXTEND_MODE_CLAMP,
            )
            .ok()
            .and_then(|stops| {
                let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                    startPoint: D2D_POINT_2F { x: 0.0, y: 0.0 },
                    endPoint: D2D_POINT_2F { x: 0.0, y: idle_h },
                };
                self.d2d_context
                    .CreateLinearGradientBrush(&props, None, &stops)
                    .ok()
            })
            .and_then(|b| b.cast().ok())
        };
        self.eave_mask_brush = brush_opt.map(|b| (key, b));
    }

    fn sync_clock_bitmap(&mut self, _cfg: &Config) {
        let (date_str, time_num, ampm) = format_header_datetime();
        let dpi_key = (self.dpi * 100.0).round() as u32;
        if let Some((ref kt, ref ka, kd)) = self.cached_clock_key {
            if *kt == time_num
                && *ka == ampm
                && kd == dpi_key
                && self.clock_2x_bmp.is_some()
                && self.cached_hdr_dt.is_some()
            {
                return;
            }
        }

        let date_part = format!("{date_str} ");
        let ampm_part = format!(" {ampm}");
        let (w_date, w_time, w_ampm) = unsafe {
            (
                self.measure_width(&date_part, &self.fmt_header_date),
                self.measure_width(&time_num, &self.fmt_header_time),
                self.measure_width(&ampm_part, &self.fmt_header_ampm),
            )
        };
        self.cached_hdr_dt = Some((
            date_part,
            time_num.clone(),
            ampm_part.clone(),
            w_date,
            w_time,
            w_ampm,
        ));
        let total_w = w_time + w_ampm;
        let cell_w_dips = (total_w + 8.0).ceil().max(16.0);
        const CELL_H_DIPS: f32 = 22.0;

        let scale = (self.dpi / 96.0).max(0.5);
        let bmp_w_px = (cell_w_dips * scale * 2.0).ceil().max(16.0) as u32;
        let bmp_h_px = (CELL_H_DIPS * scale * 2.0).ceil().max(16.0) as u32;
        let bmp_dpi = self.dpi * 2.0;

        let props = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: bmp_dpi,
            dpiY: bmp_dpi,
            bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET,
            ..Default::default()
        };

        unsafe {
            if let Ok(bmp) = self.d2d_context.CreateBitmap(
                D2D_SIZE_U {
                    width: bmp_w_px,
                    height: bmp_h_px,
                },
                None,
                0,
                &props,
            ) {
                let prev_target = self.d2d_context.GetTarget().ok();
                self.d2d_context.SetTarget(&bmp);
                self.d2d_context.SetDpi(bmp_dpi, bmp_dpi);
                self.d2d_context
                    .SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
                self.d2d_context.BeginDraw();
                self.d2d_context.Clear(Some(&D2D1_COLOR_F {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                    a: 0.0,
                }));
                let x0 = (cell_w_dips - total_w) * 0.5;
                self.draw_text_smooth(
                    &time_num,
                    &self.fmt_header_time,
                    &D2D_RECT_F {
                        left: x0,
                        top: 1.0,
                        right: x0 + w_time + 4.0,
                        bottom: 21.0,
                    },
                    (1.0, 1.0, 1.0, 1.0),
                );
                self.draw_text_smooth(
                    &ampm_part,
                    &self.fmt_header_ampm,
                    &D2D_RECT_F {
                        left: x0 + w_time,
                        top: 1.8,
                        right: x0 + total_w + 6.0,
                        bottom: 21.0,
                    },
                    (1.0, 1.0, 1.0, 1.0),
                );
                let ok = self.d2d_context.EndDraw(None, None).is_ok();
                if let Some(prev) = prev_target {
                    self.d2d_context.SetTarget(&prev);
                } else {
                    self.d2d_context.SetTarget(None);
                }
                self.d2d_context.SetDpi(self.dpi, self.dpi);
                if ok {
                    self.clock_2x_bmp = Some(bmp);
                    self.cached_clock_cell_w = cell_w_dips;
                    self.cached_clock_key = Some((time_num, ampm.to_string(), dpi_key));
                }
            }
        }
    }

    fn sync_ambient_gradient(
        &mut self,
        springs: &NotchSprings,
        state: &EaveViewState<'_>,
        cur_h: f32,
        expand: f32,
    ) {
        if state.page != NotchPage::Music {
            return;
        }
        let lyr_t = springs.lyrics_mode.value().clamp(0.0, 1.0);
        let t = ((expand - 0.15) / 0.35).clamp(0.0, 1.0);
        let osd_t = springs.osd_open().clamp(0.0, 1.0);
        let osd_suppress = (1.0 - t) * (osd_t * 1.45).clamp(0.0, 1.0);
        let glow_t = lyr_t * (1.0 - osd_suppress);
        if glow_t <= 0.01 {
            return;
        }

        let scale = (self.dpi / 96.0).max(0.5);
        let h_px = (cur_h * scale).ceil().max(4.0) as u32;
        let r = springs.song_color_r.value().clamp(0.0, 1.0);
        let g = springs.song_color_g.value().clamp(0.0, 1.0);
        let b = springs.song_color_b.value().clamp(0.0, 1.0);

        let key = (
            h_px,
            (r * 255.0).round() as u8,
            (g * 255.0).round() as u8,
            (b * 255.0).round() as u8,
            (t * 255.0).round() as u8,
            (glow_t * 255.0).round() as u8,
        );

        let start_p = 0.07 + (0.23 - 0.07) * t;
        let power = 1.70 + (2.68 - 1.70) * t;
        let peak = (0.68 + (0.60 - 0.68) * t) * glow_t;
        let inv_span = 1.0 / (1.0 - start_p).max(0.01);

        if self.cached_ambient_key != Some(key) || self.ambient_grad_brush.is_none() {
            let mut buf = vec![0u8; (8 * h_px * 4) as usize];
            for y in 0..h_px {
                let y_norm = (y as f32 + 0.5) / h_px as f32;
                let u = ((y_norm - start_p) * inv_span).clamp(0.0, 1.0);
                let a = peak * u.powf(power);
                if a <= 1e-5 {
                    continue;
                }
                let a_f = (a * 255.0).clamp(0.0, 255.0);
                let r_f = r * a_f;
                let g_f = g * a_f;
                let b_f = b * a_f;
                let row_bayer = &BAYER_8X8[(y as usize) & 7];
                let row_off = (y as usize) * 8 * 4;
                for x in 0..8 {
                    let d = (row_bayer[x] as f32 + 0.5) / 64.0 - 0.5;
                    let a_u8 = (a_f + d).clamp(0.0, 255.0).round() as u8;
                    let b_u8 = ((b_f + d).clamp(0.0, 255.0).round() as u8).min(a_u8);
                    let g_u8 = ((g_f + d).clamp(0.0, 255.0).round() as u8).min(a_u8);
                    let r_u8 = ((r_f + d).clamp(0.0, 255.0).round() as u8).min(a_u8);
                    let idx = row_off + x * 4;
                    buf[idx] = b_u8;
                    buf[idx + 1] = g_u8;
                    buf[idx + 2] = r_u8;
                    buf[idx + 3] = a_u8;
                }
            }

            let size = D2D_SIZE_U {
                width: 8,
                height: h_px,
            };
            let bmp_props = D2D1_BITMAP_PROPERTIES1 {
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: self.dpi,
                dpiY: self.dpi,
                bitmapOptions: Default::default(),
                ..Default::default()
            };
            let bb_props = D2D1_BITMAP_BRUSH_PROPERTIES1 {
                extendModeX: D2D1_EXTEND_MODE_WRAP,
                extendModeY: D2D1_EXTEND_MODE_CLAMP,
                interpolationMode: D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
            };
            let brush_props = D2D1_BRUSH_PROPERTIES {
                opacity: 1.0,
                transform: Matrix3x2::identity(),
            };

            unsafe {
                if let Ok(bmp) = self.d2d_context.CreateBitmap(
                    size,
                    Some(buf.as_ptr() as *const _),
                    8 * 4,
                    &bmp_props,
                ) {
                    if let Ok(brush) = self.d2d_context.CreateBitmapBrush(
                        &bmp,
                        Some(&bb_props),
                        Some(&brush_props),
                    ) {
                        self.ambient_grad_bmp = Some(bmp);
                        self.ambient_grad_brush = Some(brush);
                        self.cached_ambient_key = Some(key);
                    }
                }
            }
        }

        const AURORA_W: usize = 64;
        const AURORA_H: usize = 24;
        let phase = springs.phase_secs();
        let phase_bucket = (phase * 30.0).floor() as u32;
        let aurora_key = (
            phase_bucket,
            key.1,
            key.2,
            key.3,
            key.4,
            key.5,
        );
        if self.cached_aurora_key == Some(aurora_key) && self.ambient_aurora_bmp.is_some() {
            return;
        }

        let (h, s, l) = rgb_to_hsl(r, g, b);
        let is_mono = s < 0.08;
        let (r1, g1, b1) = hsl_to_rgb(
            (h + 0.075).rem_euclid(1.0),
            if is_mono { 0.0 } else { (s * 1.18 + 0.08).clamp(0.32, 0.94) },
            (l + 0.10).clamp(0.34, 0.72),
        );
        let (r2, g2, b2) = hsl_to_rgb(
            (h - 0.068).rem_euclid(1.0),
            if is_mono { 0.0 } else { (s * 1.12 + 0.05).clamp(0.28, 0.90) },
            (l + 0.03).clamp(0.26, 0.62),
        );

        let cx1 = 0.50 + 0.34 * (phase * 0.24).sin() + 0.08 * (phase * 0.41 + 1.1).cos();
        let cy1 = 0.88 + 0.11 * (phase * 0.19 + 0.7).sin();
        let rx1 = 0.34 + 0.09 * (phase * 0.29 + 0.4).sin();
        let ry1 = 0.30 + 0.07 * (phase * 0.23 + 2.0).cos();

        let cx2 = 0.50 - 0.32 * (phase * 0.18 + 1.9).cos() + 0.07 * (phase * 0.31).sin();
        let cy2 = 0.90 + 0.10 * (phase * 0.22 + 2.8).cos();
        let rx2 = 0.38 + 0.10 * (phase * 0.21 + 1.5).cos();
        let ry2 = 0.28 + 0.07 * (phase * 0.27 + 0.9).sin();

        let amp1 = (0.34 + 0.06 * (phase * 0.17).sin()) * glow_t;
        let amp2 = (0.28 + 0.05 * (phase * 0.15 + 1.3).cos()) * glow_t;
        let env_power = power * 0.92;

        let mut aurora_buf = [0u8; AURORA_W * AURORA_H * 4];
        for y in 0..AURORA_H {
            let u_y = (y as f32 + 0.5) / AURORA_H as f32;
            let u_env = ((u_y - start_p) * inv_span).clamp(0.0, 1.0);
            let env = u_env.powf(env_power);
            if env <= 1e-4 {
                continue;
            }
            let row_off = y * AURORA_W * 4;
            for x in 0..AURORA_W {
                let u_x = (x as f32 + 0.5) / AURORA_W as f32;
                let warp1 = 0.08 * (u_x * 4.2 + phase * 0.33).sin()
                    + 0.05 * (u_x * 2.5 - phase * 0.21).cos();
                let dx1 = (u_x - cx1) / rx1;
                let dy1 = (u_y - cy1 + warp1) / ry1;
                let w1 = (-(dx1 * dx1 + dy1 * dy1) * 1.45).exp();

                let warp2 = 0.07 * (u_x * 3.6 - phase * 0.27 + 1.4).sin();
                let dx2 = (u_x - cx2) / rx2;
                let dy2 = (u_y - cy2 + warp2) / ry2;
                let w2 = (-(dx2 * dx2 + dy2 * dy2) * 1.45).exp();

                let a1 = (w1 * env * amp1).clamp(0.0, 0.50);
                let a2 = (w2 * env * amp2).clamp(0.0, 0.45);
                let inv_a1 = 1.0 - a1;
                let a_tot = (a1 + a2 * inv_a1).clamp(0.0, 0.58);
                if a_tot <= 1e-4 {
                    continue;
                }
                let r_pm = (r1 * a1 + r2 * a2 * inv_a1).clamp(0.0, a_tot);
                let g_pm = (g1 * a1 + g2 * a2 * inv_a1).clamp(0.0, a_tot);
                let b_pm = (b1 * a1 + b2 * a2 * inv_a1).clamp(0.0, a_tot);

                let a_u8 = (a_tot * 255.0).round().clamp(0.0, 255.0) as u8;
                let idx = row_off + x * 4;
                aurora_buf[idx] = ((b_pm * 255.0).round().clamp(0.0, 255.0) as u8).min(a_u8);
                aurora_buf[idx + 1] = ((g_pm * 255.0).round().clamp(0.0, 255.0) as u8).min(a_u8);
                aurora_buf[idx + 2] = ((r_pm * 255.0).round().clamp(0.0, 255.0) as u8).min(a_u8);
                aurora_buf[idx + 3] = a_u8;
            }
        }

        unsafe {
            if let Some(bmp) = &self.ambient_aurora_bmp {
                if bmp
                    .CopyFromMemory(None, aurora_buf.as_ptr() as *const _, (AURORA_W * 4) as u32)
                    .is_ok()
                {
                    self.cached_aurora_key = Some(aurora_key);
                }
            } else {
                let size = D2D_SIZE_U {
                    width: AURORA_W as u32,
                    height: AURORA_H as u32,
                };
                let props = D2D1_BITMAP_PROPERTIES1 {
                    pixelFormat: D2D1_PIXEL_FORMAT {
                        format: DXGI_FORMAT_B8G8R8A8_UNORM,
                        alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                    },
                    dpiX: 96.0,
                    dpiY: 96.0,
                    bitmapOptions: Default::default(),
                    ..Default::default()
                };
                if let Ok(bmp) = self.d2d_context.CreateBitmap(
                    size,
                    Some(aurora_buf.as_ptr() as *const _),
                    (AURORA_W * 4) as u32,
                    &props,
                ) {
                    self.ambient_aurora_bmp = Some(bmp);
                    self.cached_aurora_key = Some(aurora_key);
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn draw_song_ambient_glow(
        &self,
        springs: &NotchSprings,
        _canvas_cx: f32,
        pill_left: f32,
        cur_w: f32,
        cur_h: f32,
        expand: f32,
    ) {
        let lyr_t = springs.lyrics_mode.value().clamp(0.0, 1.0);
        let t = ((expand - 0.15) / 0.35).clamp(0.0, 1.0);
        let osd_t = springs.osd_open().clamp(0.0, 1.0);
        let osd_suppress = (1.0 - t) * (osd_t * 1.45).clamp(0.0, 1.0);
        let glow_t = lyr_t * (1.0 - osd_suppress);
        if glow_t <= 0.01 {
            return;
        }
        let rect = D2D_RECT_F {
            left: pill_left,
            top: 0.0,
            right: pill_left + cur_w,
            bottom: cur_h,
        };
        if let Some(brush) = &self.ambient_grad_brush {
            self.d2d_context.FillRectangle(&rect, brush);
        }
        if let Some(aurora_bmp) = &self.ambient_aurora_bmp {
            self.d2d_context.DrawBitmap(
                aurora_bmp,
                Some(&rect),
                1.0,
                D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
                None,
                None,
            );
        }
    }

    unsafe fn draw_mascot_idle_companion(
        &self,
        springs: &NotchSprings,
        pill_left: f32,
        cur_w: f32,
        idle_h: f32,
        ear_r: f32,
        alpha: f32,
    ) {
        let body_left = pill_left + ear_r;
        let body_right = pill_left + cur_w - ear_r;
        let sleep_t = springs.mascot_idle_sleep.value().clamp(0.0, 1.0);
        let phase = springs.phase_secs();
        let flow = springs.eave_aurora_phase();
        let breathe = 0.5 + 0.5 * (flow * 1.65).sin();
        let grad_dim = (1.0 - 0.60 * sleep_t) * (0.94 + 0.06 * breathe);
        let glow_rect = D2D_RECT_F {
            left: pill_left,
            top: 0.0,
            right: pill_left + cur_w,
            bottom: idle_h + 2.0,
        };

        // 1. Reuse cached vertical opacity mask (zero COM allocations per frame).
        let opacity_brush: Option<ID2D1Brush> =
            self.eave_mask_brush.as_ref().map(|(_, b)| b.clone());

        let stop1 = (0.34 + 0.11 * (flow * 0.52).sin()).clamp(0.18, 0.52);
        let stop2 = (0.68 + 0.11 * (flow * 0.41 + 1.2).cos()).clamp(0.54, 0.86);
        let aurora_a = (0.86 * grad_dim * alpha).clamp(0.0, 1.0);
        let aurora_stops = [
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
        if let Ok(stops) = ID2D1RenderTarget::CreateGradientStopCollection(
            &self.d2d_context,
            &aurora_stops,
            D2D1_GAMMA_2_2,
            D2D1_EXTEND_MODE_CLAMP,
        ) {
            let h_props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                startPoint: D2D_POINT_2F { x: body_left, y: idle_h },
                endPoint: D2D_POINT_2F { x: body_right, y: idle_h },
            };
            if let Ok(h_brush) = self
                .d2d_context
                .CreateLinearGradientBrush(&h_props, None, &stops)
            {
                let mut layer_params = D2D1_LAYER_PARAMETERS1 {
                    contentBounds: glow_rect,
                    geometricMask: ManuallyDrop::new(None),
                    maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                    maskTransform: Matrix3x2::identity(),
                    opacity: 1.0,
                    opacityBrush: ManuallyDrop::new(opacity_brush),
                    layerOptions: D2D1_LAYER_OPTIONS1_NONE,
                };
                self.d2d_context.PushLayer(&layer_params, None);
                self.d2d_context.FillRectangle(&glow_rect, &h_brush);

                let body_w = (body_right - body_left).max(40.0);
                let blob_base_a = (0.48 * grad_dim * alpha).clamp(0.0, 1.0);
                let draw_cached_blob = |slot: usize, x: f32, y: f32, r: f32, a: f32| {
                    if a <= 0.005 {
                        return;
                    }
                    if let Some(rb) = &self.eave_radial_brushes[slot] {
                        rb.SetCenter(D2D_POINT_2F { x, y });
                        rb.SetRadiusX(r);
                        rb.SetRadiusY(r);
                        rb.SetOpacity(a);
                        self.d2d_context.FillEllipse(
                            &D2D1_ELLIPSE {
                                point: D2D_POINT_2F { x, y },
                                radiusX: r,
                                radiusY: r,
                            },
                            rb,
                        );
                    }
                };
                let b1_x = body_left + body_w * (0.24 + 0.22 * (flow * 0.47).sin() + 0.06 * (flow * 0.83 + 1.1).cos());
                let b1_y = idle_h * (0.86 + 0.14 * (flow * 0.39 + 0.6).sin());
                let b1_r = body_w * (0.34 + 0.06 * (flow * 0.58 + 0.4).sin());
                draw_cached_blob(0, b1_x, b1_y, b1_r, blob_base_a * 0.92);

                let b2_x = body_left + body_w * (0.52 + 0.24 * (flow * 0.36 + 2.1).cos());
                let b2_y = idle_h * (0.90 + 0.12 * (flow * 0.44 + 2.4).cos());
                let b2_r = body_w * (0.38 + 0.07 * (flow * 0.49 + 1.7).cos());
                draw_cached_blob(1, b2_x, b2_y, b2_r, blob_base_a * 0.80);

                let b3_x = body_left + body_w * (0.78 - 0.22 * (flow * 0.42 + 0.9).sin() + 0.05 * (flow * 0.74).cos());
                let b3_y = idle_h * (0.85 + 0.14 * (flow * 0.35 + 1.5).sin());
                let b3_r = body_w * (0.35 + 0.06 * (flow * 0.53 + 2.8).sin());
                draw_cached_blob(2, b3_x, b3_y, b3_r, blob_base_a * 0.95);

                self.d2d_context.PopLayer();
                ManuallyDrop::drop(&mut layer_params.opacityBrush);

                let rim_rect = D2D_RECT_F {
                    left: body_left,
                    top: (idle_h - 2.0).max(0.0),
                    right: body_right,
                    bottom: idle_h + 1.0,
                };
                h_brush.SetOpacity((0.46 * grad_dim * alpha).clamp(0.0, 1.0));
                self.d2d_context.FillRectangle(&rim_rect, &h_brush);
            }
        }

        let scale = (idle_h / 32.0).clamp(0.75, 1.35);
        let cx = (body_left + body_right) * 0.5;
        let cy = idle_h * 0.47;
        let waking = springs.eave_is_waking();
        let look_amp = if waking { 4.6 } else { 2.4 };
        let look_x = springs.mascot_look_x.value() * look_amp * scale * (1.0 - sleep_t);
        let wake_lift = if waking {
            -0.55 * springs.mascot_look_x.value().abs() * (1.0 - sleep_t)
        } else {
            0.0
        };
        let look_y =
            ((phase * 0.42).sin() * 0.45 * (1.0 - sleep_t) + wake_lift + 1.4 * sleep_t) * scale;
        let blink = springs.mascot_blink() * (1.0 - sleep_t);
        let openness = ((1.0 - blink) * (1.0 - sleep_t)).clamp(0.0, 1.0);
        let eye_sep = 16.5 * scale;
        let rx = (3.7 + 1.15 * sleep_t) * scale;
        let ry = (1.05 + 5.15 * openness) * scale;
        let eye_luma_a = ((0.96 - 0.36 * sleep_t) * alpha).clamp(0.0, 1.0);
        let halo_a = (0.045 * (1.0 - 0.65 * sleep_t) * alpha).clamp(0.0, 1.0);

        for side in [-0.5_f32, 0.5_f32] {
            let ex = cx + look_x + side * eye_sep;
            let ey = cy + look_y;
            if halo_a > 0.005 {
                if let Some(rb) = &self.eave_radial_brushes[3] {
                    let hr = 13.5 * scale;
                    rb.SetCenter(D2D_POINT_2F { x: ex, y: ey });
                    rb.SetRadiusX(hr);
                    rb.SetRadiusY(hr);
                    rb.SetOpacity(halo_a);
                    self.d2d_context.FillEllipse(
                        &D2D1_ELLIPSE {
                            point: D2D_POINT_2F { x: ex, y: ey },
                            radiusX: hr,
                            radiusY: hr,
                        },
                        rb,
                    );
                }
            }
            self.set_color(0.94, 0.94, 0.91, eye_luma_a);
            self.d2d_context.FillEllipse(
                &D2D1_ELLIPSE {
                    point: D2D_POINT_2F { x: ex, y: ey },
                    radiusX: rx,
                    radiusY: ry,
                },
                &self.solid_brush,
            );
        }

        if let Some(z_elapsed) = springs.eave_sleep_z_elapsed() {
            let fade_in = (z_elapsed / 0.25).clamp(0.0, 1.0);
            let fade_out = ((3.0 - z_elapsed) / 0.45).clamp(0.0, 1.0);
            let z_a = (fade_in * fade_out * (0.35 + 0.65 * sleep_t) * 0.82 * alpha).clamp(0.0, 1.0);
            if z_a > 0.01 {
                let tick = (z_elapsed.floor() as i32).clamp(0, 2) as f32;
                let zx = cx + eye_sep * 0.5 + (9.5 + tick * 1.4) * scale;
                let zy = cy + (0.2 - tick * 3.0) * scale;
                let zw = 4.6 * scale;
                let zh = 5.4 * scale;
                let stroke = 1.35 * scale;
                self.set_color(0.76, 0.89, 1.0, z_a);
                let tl = D2D_POINT_2F { x: zx, y: zy - zh * 0.5 };
                let tr = D2D_POINT_2F { x: zx + zw, y: zy - zh * 0.5 };
                let bl = D2D_POINT_2F { x: zx, y: zy + zh * 0.5 };
                let br = D2D_POINT_2F { x: zx + zw, y: zy + zh * 0.5 };
                self.d2d_context
                    .DrawLine(tl, tr, &self.solid_brush, stroke, None);
                self.d2d_context
                    .DrawLine(tr, bl, &self.solid_brush, stroke, None);
                self.d2d_context
                    .DrawLine(bl, br, &self.solid_brush, stroke, None);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn draw_collapsed_notch(
        &self,
        cfg: &Config,
        springs: &NotchSprings,
        state: &EaveViewState<'_>,
        _canvas_cx: f32,
        pill_left: f32,
        cur_w: f32,
        cur_h: f32,
        ear_r: f32,
        alpha: f32,
    ) {
        let media = &state.media;
        let body_left = pill_left + ear_r;
        let body_right = pill_left + cur_w - ear_r;
        let idle_h = cur_h.max(24.0);
        let cy = idle_h * 0.5;
        let brush: &ID2D1Brush = &self.solid_brush;

        let lyr_t = springs.lyrics_mode.value().clamp(0.0, 1.0);
        let osd_t = springs.osd_open().clamp(0.0, 1.0);
        let eave_t = springs.mascot_idle_presence.value().clamp(0.0, 1.0);
        let osd_fade_out = (1.0 - osd_t * 1.45).clamp(0.0, 1.0);
        let eave_fade_out = (1.0 - eave_t * 1.35).clamp(0.0, 1.0);
        let idle_a = alpha * (1.0 - lyr_t * 1.35).clamp(0.0, 1.0) * osd_fade_out * eave_fade_out;
        let eave_a = alpha * (1.0 - lyr_t * 1.35).clamp(0.0, 1.0) * osd_fade_out * eave_t;

        if eave_a > 0.01 {
            self.draw_mascot_idle_companion(springs, pill_left, cur_w, idle_h, ear_r, eave_a);
        }

        if idle_a > 0.01 {
            let art_mix = springs.collapsed_art_mix.value().clamp(0.0, 1.0);
            let art_a = idle_a * art_mix;
            if self.cached_art_bitmap.is_some() && art_a > 0.01 {
                let art_s = (cfg.notch_height.max(24.0) - 10.0).clamp(14.0, 20.0);
                let art_rect = D2D_RECT_F {
                    left: body_left + 10.0,
                    top: cy - art_s * 0.5,
                    right: body_left + 10.0 + art_s,
                    bottom: cy + art_s * 0.5,
                };
                self.draw_rounded_art(&art_rect, 4.0, art_a, springs.album_art_fade.value());
            }

            let audio_vis_mix = springs.audio_vis_mix.value().clamp(0.0, 1.0);
            let vis_a = idle_a * audio_vis_mix;
            if vis_a > 0.01 {
                let vis_size = (cfg.notch_height.max(24.0) - 8.0).clamp(15.0, 22.0)
                    * (0.76 + 0.24 * audio_vis_mix);
                let vis_cx = body_right - 10.0 - vis_size * 0.44;
                let levels = springs.visualizer_levels();
                let sr = springs.song_color_r.value().clamp(0.0, 1.0);
                let sg = springs.song_color_g.value().clamp(0.0, 1.0);
                let sb = springs.song_color_b.value().clamp(0.0, 1.0);
                let (h, s, l) = rgb_to_hsl(sr, sg, sb);
                let sat = if s < 0.08 {
                    s
                } else {
                    (s * 1.18 + 0.14).clamp(0.48, 0.94)
                };
                let (r1, g1, b1) = hsl_to_rgb(
                    (h + 0.055).rem_euclid(1.0),
                    sat,
                    (l + 0.18).clamp(0.62, 0.82),
                );
                let (r2, g2, b2) = hsl_to_rgb(
                    (h - 0.055).rem_euclid(1.0),
                    sat,
                    (l + 0.04).clamp(0.48, 0.68),
                );
                let bar_a = 0.96 * vis_a;
                let stops = [
                    D2D1_GRADIENT_STOP {
                        position: 0.0,
                        color: D2D1_COLOR_F {
                            r: r1,
                            g: g1,
                            b: b1,
                            a: bar_a,
                        },
                    },
                    D2D1_GRADIENT_STOP {
                        position: 1.0,
                        color: D2D1_COLOR_F {
                            r: r2,
                            g: g2,
                            b: b2,
                            a: bar_a,
                        },
                    },
                ];
                let grad_brush = ID2D1RenderTarget::CreateGradientStopCollection(
                    &self.d2d_context,
                    &stops,
                    D2D1_GAMMA_2_2,
                    D2D1_EXTEND_MODE_CLAMP,
                )
                .ok()
                .and_then(|sc| {
                    let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                        startPoint: D2D_POINT_2F {
                            x: vis_cx - vis_size * 0.5,
                            y: cy - vis_size * 0.42,
                        },
                        endPoint: D2D_POINT_2F {
                            x: vis_cx + vis_size * 0.5,
                            y: cy + vis_size * 0.42,
                        },
                    };
                    self.d2d_context
                        .CreateLinearGradientBrush(&props, None, &sc)
                        .ok()
                });
                if let Some(gb) = grad_brush {
                    let gbrush: &ID2D1Brush = &gb;
                    icons::draw_audio_bars(
                        &self.d2d_context,
                        vis_cx,
                        cy,
                        vis_size,
                        gbrush,
                        Some(levels),
                    );
                } else {
                    self.set_color(r1, g1, b1, bar_a);
                    icons::draw_audio_bars(
                        &self.d2d_context,
                        vis_cx,
                        cy,
                        vis_size,
                        brush,
                        Some(levels),
                    );
                }
            }
        }

        let started_mix = if !media.lyric_started && springs.lyric_started_mix.is_settled() {
            0.0
        } else {
            springs.lyric_started_mix().clamp(0.0, 1.0)
        };
        let lyr_a = alpha * ((lyr_t - 0.15) / 0.85).clamp(0.0, 1.0) * osd_fade_out;
        if lyr_a > 0.01 {
            let text = if !media.lyric_current.is_empty() {
                media.lyric_current
            } else if !media.lyric_next.is_empty() {
                media.lyric_next
            } else if !media.title.is_empty() {
                media.title
            } else {
                "Playing..."
            };
            let line_bright = 0.44 + (0.98 - 0.44) * started_mix;
            let line_t = springs.lyric_line_change.value().clamp(0.0, 1.0);

            let note_bright = 0.52 + (0.88 - 0.52) * started_mix;
            self.draw_ss_icon(
                ATLAS_SLOT_MUSIC_NOTE,
                body_left + 20.0,
                cy + 0.5,
                21.5,
                0.82 * note_bright * lyr_a,
            );

            let text_left = body_left + 36.0;
            let text_right = (body_right - 14.0).max(text_left + 40.0);
            let clip = D2D_RECT_F {
                left: text_left,
                top: 2.0,
                right: text_right,
                bottom: (idle_h - 2.0).max(4.0),
            };
            self.d2d_context
                .PushAxisAlignedClip(&clip, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);

            let lyr_moving = line_t < 0.999;
            if line_t < 0.995 && !media.lyric_prev.is_empty() {
                let dy_prev = -line_t * 13.0;
                let a_prev = lyr_a * (1.0 - line_t).powf(1.3);
                if a_prev > 0.01 {
                    if lyrics::is_instrumental_text(media.lyric_prev) {
                        self.draw_ss_icon(
                            ATLAS_SLOT_MUSIC_NOTE,
                            (text_left + text_right) * 0.5,
                            cy + dy_prev,
                            18.0,
                            0.98 * a_prev,
                        );
                    } else {
                        let prev_rect = D2D_RECT_F {
                            left: text_left,
                            top: dy_prev,
                            right: text_right,
                            bottom: idle_h + dy_prev,
                        };
                        if lyr_moving {
                            self.draw_text_smooth(
                                media.lyric_prev,
                                &self.fmt_closed_lyric,
                                &prev_rect,
                                (1.0, 1.0, 1.0, 1.0 * a_prev),
                            );
                        } else {
                            self.draw_text(
                                media.lyric_prev,
                                &self.fmt_closed_lyric,
                                &prev_rect,
                                (1.0, 1.0, 1.0, 1.0 * a_prev),
                            );
                        }
                    }
                }
            }

            let dy_curr = if !media.lyric_prev.is_empty() {
                (1.0 - line_t) * 13.0
            } else {
                (1.0 - line_t) * 8.0
            };
            let a_curr = lyr_a * line_t.powf(0.85);
            if a_curr > 0.01 {
                if lyrics::is_instrumental_text(text) {
                    self.draw_ss_icon(
                        ATLAS_SLOT_MUSIC_NOTE,
                        (text_left + text_right) * 0.5,
                        cy + dy_curr,
                        18.0,
                        line_bright * a_curr,
                    );
                } else {
                    let curr_rect = D2D_RECT_F {
                        left: text_left,
                        top: dy_curr,
                        right: text_right,
                        bottom: idle_h + dy_curr,
                    };
                    if lyr_moving {
                        self.draw_text_smooth(
                            text,
                            &self.fmt_closed_lyric,
                            &curr_rect,
                            (1.0, 1.0, 1.0, line_bright * a_curr),
                        );
                    } else {
                        self.draw_text(
                            text,
                            &self.fmt_closed_lyric,
                            &curr_rect,
                            (1.0, 1.0, 1.0, line_bright * a_curr),
                        );
                    }
                }
            }

            self.d2d_context.PopAxisAlignedClip();
        }

        let osd_a = alpha * ((osd_t - 0.12) / 0.88).clamp(0.0, 1.0);
        if osd_a > 0.01 {
            match state.collapsed_osd {
                CollapsedOsdKind::None => {}
                CollapsedOsdKind::Volume => {
                    let raw_vol = springs.speaker_volume().clamp(0.0, 100.0);
                    let vol = if state.muted && springs.speaker_volume.is_settled() {
                        0.0
                    } else {
                        raw_vol
                    };
                    let spk_x = body_left + 21.0;
                    self.draw_ss_speaker(spk_x, cy, 18.0, 0.98 * osd_a, vol, 3);

                    let bar_left = body_left + 40.0;
                    let bar_right = (body_right - 14.0).max(bar_left + 12.0);
                    let squash = (springs.osd_vol_bounce().abs() * 0.045).min(0.55);
                    let half_h = (3.25 - squash).max(2.5);
                    let track_rect = D2D_RECT_F {
                        left: bar_left,
                        top: cy - half_h,
                        right: bar_right,
                        bottom: cy + half_h,
                    };
                    self.set_color(0.227, 0.227, 0.239, 0.96 * osd_a);
                    self.d2d_context.FillRoundedRectangle(
                        &D2D1_ROUNDED_RECT {
                            rect: track_rect,
                            radiusX: half_h,
                            radiusY: half_h,
                        },
                        &self.solid_brush,
                    );
                    let track_w = (bar_right - bar_left).max(0.0);
                    let fill_w = track_w * (vol / 100.0).clamp(0.0, 1.0);
                    if fill_w > 0.5 {
                        let fill_r = (fill_w * 0.5).min(half_h);
                        self.set_color(0.898, 0.902, 0.910, 0.98 * osd_a);
                        self.d2d_context.FillRoundedRectangle(
                            &D2D1_ROUNDED_RECT {
                                rect: D2D_RECT_F {
                                    left: bar_left,
                                    top: cy - half_h,
                                    right: bar_left + fill_w,
                                    bottom: cy + half_h,
                                },
                                radiusX: fill_r,
                                radiusY: fill_r,
                            },
                            &self.solid_brush,
                        );
                    }
                }
                CollapsedOsdKind::CapsLock { on } => {
                    let raw_caps = springs.caps_on_mix().clamp(0.0, 1.0);
                    let caps_mix = if springs.caps_on_mix.is_settled() {
                        if on && raw_caps < 0.5 {
                            1.0
                        } else if !on && raw_caps > 0.5 {
                            0.0
                        } else {
                            raw_caps
                        }
                    } else {
                        raw_caps
                    };
                    let icon_x = body_left + 21.0;
                    if 1.0 - caps_mix > 0.01 {
                        let a_off = (1.0 - caps_mix) * osd_a;
                        self.draw_ss_icon(ATLAS_SLOT_CAPS_OFF, icon_x, cy, 18.0, a_off);
                    }
                    if caps_mix > 0.01 {
                        let a_on = caps_mix * osd_a;
                        self.draw_ss_icon(ATLAS_SLOT_CAPS_ON, icon_x, cy, 18.0, 0.98 * a_on);
                    }

                    let state_left = (body_right - 56.0).max(body_left + 44.0);
                    self.draw_text(
                        "Caps Lock",
                        &self.fmt_osd_label,
                        &D2D_RECT_F {
                            left: body_left + 38.0,
                            top: 0.0,
                            right: state_left,
                            bottom: idle_h,
                        },
                        (0.96, 0.97, 0.98, 1.0 * osd_a),
                    );

                    let state_right = (body_right - 14.0).max(state_left + 10.0);
                    let clip = D2D_RECT_F {
                        left: state_left,
                        top: 3.0,
                        right: state_right,
                        bottom: (idle_h - 3.0).max(6.0),
                    };
                    self.d2d_context
                        .PushAxisAlignedClip(&clip, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);

                    let caps_moving = caps_mix > 0.005 && caps_mix < 0.995;
                    if 1.0 - caps_mix > 0.01 {
                        let dy_off = -18.0 * caps_mix;
                        let off_rect = D2D_RECT_F {
                            left: state_left,
                            top: dy_off,
                            right: state_right,
                            bottom: idle_h + dy_off,
                        };
                        let off_col = (0.54, 0.54, 0.56, 0.98 * (1.0 - caps_mix) * osd_a);
                        if caps_moving {
                            self.draw_text_smooth("Off", &self.fmt_osd_state, &off_rect, off_col);
                        } else {
                            self.draw_text("Off", &self.fmt_osd_state, &off_rect, off_col);
                        }
                    }
                    if caps_mix > 0.01 {
                        let dy_on = 18.0 * (1.0 - caps_mix);
                        let on_rect = D2D_RECT_F {
                            left: state_left,
                            top: dy_on,
                            right: state_right,
                            bottom: idle_h + dy_on,
                        };
                        let on_col = (0.180, 0.902, 0.376, 0.98 * caps_mix * osd_a);
                        if caps_moving {
                            self.draw_text_smooth("On", &self.fmt_osd_state, &on_rect, on_col);
                        } else {
                            self.draw_text("On", &self.fmt_osd_state, &on_rect, on_col);
                        }
                    }

                    self.d2d_context.PopAxisAlignedClip();
                }
                CollapsedOsdKind::Bluetooth => {
                    let connected = state.widgets.popup_bt_connected;
                    let pct = state.widgets.popup_bt_pct.min(100);
                    let name = if state.widgets.popup_bt_name.is_empty() {
                        "Bluetooth Device"
                    } else {
                        state.widgets.popup_bt_name.as_str()
                    };
                    let ring_cx = body_left + 27.0;
                    let ring_r = 14.2;
                    let stroke_w = 3.4;

                    if connected {
                        self.set_color(1.0, 1.0, 1.0, 0.22 * osd_a);
                    } else {
                        self.set_color(1.0, 0.36, 0.38, 0.32 * osd_a);
                    }
                    self.d2d_context.DrawEllipse(
                        &D2D1_ELLIPSE {
                            point: D2D_POINT_2F { x: ring_cx, y: cy },
                            radiusX: ring_r,
                            radiusY: ring_r,
                        },
                        &self.solid_brush,
                        stroke_w,
                        None,
                    );

                    if connected {
                        let fill_ratio = springs.bt_popup_arc.value().clamp(0.0, 1.0);
                        let sweep = fill_ratio * 359.5;
                        if sweep > 1.0 {
                            let a0 = (-90.0_f32).to_radians();
                            let a1 = (-90.0_f32 + sweep).to_radians();
                            if let Ok(geo) = self.d2d_factory.CreatePathGeometry() {
                                if let Ok(sink) = geo.Open() {
                                    sink.BeginFigure(
                                        D2D_POINT_2F {
                                            x: ring_cx + ring_r * a0.cos(),
                                            y: cy + ring_r * a0.sin(),
                                        },
                                        windows::Win32::Graphics::Direct2D::Common::D2D1_FIGURE_BEGIN_HOLLOW,
                                    );
                                    sink.AddArc(&D2D1_ARC_SEGMENT {
                                        point: D2D_POINT_2F {
                                            x: ring_cx + ring_r * a1.cos(),
                                            y: cy + ring_r * a1.sin(),
                                        },
                                        size: D2D_SIZE_F {
                                            width: ring_r,
                                            height: ring_r,
                                        },
                                        rotationAngle: 0.0,
                                        sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                                        arcSize: if sweep > 180.0 {
                                            windows::Win32::Graphics::Direct2D::D2D1_ARC_SIZE_LARGE
                                        } else {
                                            D2D1_ARC_SIZE_SMALL
                                        },
                                    });
                                    sink.EndFigure(
                                        windows::Win32::Graphics::Direct2D::Common::D2D1_FIGURE_END_OPEN,
                                    );
                                    if sink.Close().is_ok() {
                                        let round_st = ID2D1Factory::CreateStrokeStyle(
                                            &self.d2d_factory,
                                            &D2D1_STROKE_STYLE_PROPERTIES {
                                                startCap: D2D1_CAP_STYLE_ROUND,
                                                endCap: D2D1_CAP_STYLE_ROUND,
                                                ..Default::default()
                                            },
                                            None,
                                        )
                                        .ok();
                                        self.set_color(0.369, 0.761, 1.0, 0.98 * osd_a);
                                        self.d2d_context.DrawGeometry(
                                            &geo,
                                            &self.solid_brush,
                                            stroke_w,
                                            round_st.as_ref(),
                                        );
                                    }
                                }
                            }
                        }
                    }

                    self.set_color(
                        1.0,
                        1.0,
                        1.0,
                        (if connected { 0.98 } else { 0.68 }) * osd_a,
                    );
                    icons::draw_bluetooth_icon(
                        &self.d2d_context,
                        ring_cx,
                        cy,
                        16.5,
                        &self.solid_brush,
                        1.85,
                    );

                    let txt_l = body_left + 52.0;
                    let txt_r = (body_right - 52.0).max(txt_l + 40.0);
                    self.draw_text(
                        name,
                        &self.fmt_osd_label,
                        &D2D_RECT_F {
                            left: txt_l,
                            top: cy - 17.0,
                            right: txt_r,
                            bottom: cy + 1.0,
                        },
                        (0.98, 0.98, 0.99, osd_a),
                    );
                    let sub_s = if connected {
                        format!("Connected · {pct}%")
                    } else {
                        "Disconnected".to_string()
                    };
                    let sub_col = if connected {
                        (0.62, 0.63, 0.66, osd_a)
                    } else {
                        (1.0, 0.44, 0.44, 0.92 * osd_a)
                    };
                    self.draw_text(
                        &sub_s,
                        &self.fmt_artist,
                        &D2D_RECT_F {
                            left: txt_l,
                            top: cy + 1.0,
                            right: txt_r,
                            bottom: cy + 17.0,
                        },
                        sub_col,
                    );

                    let badge_cx = body_right - 26.0;
                    let (acc_r, acc_g, acc_b) = if connected {
                        (0.369, 0.761, 1.0)
                    } else {
                        (1.0, 0.38, 0.40)
                    };
                    let (fill_r, fill_g, fill_b) = if connected {
                        (0.051, 0.102, 0.149)
                    } else {
                        (0.149, 0.062, 0.070)
                    };
                    {
                        let p = springs.phase_secs().rem_euclid(1.0);
                        let ease = 1.0 - (1.0 - p).powi(3);
                        let ripple_r = 13.8 + 8.5 * ease;
                        let ripple_a = 0.62 * (1.0 - p).powi(2) * osd_a;
                        if ripple_a > 0.005 {
                            self.set_color(acc_r, acc_g, acc_b, ripple_a);
                            self.d2d_context.DrawEllipse(
                                &D2D1_ELLIPSE {
                                    point: D2D_POINT_2F { x: badge_cx, y: cy },
                                    radiusX: ripple_r,
                                    radiusY: ripple_r,
                                },
                                &self.solid_brush,
                                1.5 - 0.6 * ease,
                                None,
                            );
                        }
                    }
                    self.set_color(fill_r, fill_g, fill_b, 0.96 * osd_a);
                    self.d2d_context.FillEllipse(
                        &D2D1_ELLIPSE {
                            point: D2D_POINT_2F { x: badge_cx, y: cy },
                            radiusX: 13.8,
                            radiusY: 13.8,
                        },
                        &self.solid_brush,
                    );
                    self.set_color(acc_r, acc_g, acc_b, 0.56 * osd_a);
                    self.d2d_context.DrawEllipse(
                        &D2D1_ELLIPSE {
                            point: D2D_POINT_2F { x: badge_cx, y: cy },
                            radiusX: 13.8,
                            radiusY: 13.8,
                        },
                        &self.solid_brush,
                        1.25,
                        None,
                    );
                    let hp_a = (if connected { 0.96 } else { 0.68 }) * osd_a;
                    if let Ok(geo) = self.d2d_factory.CreatePathGeometry() {
                        if let Ok(sink) = geo.Open() {
                            sink.BeginFigure(
                                D2D_POINT_2F {
                                    x: badge_cx - 6.2,
                                    y: cy + 1.5,
                                },
                                windows::Win32::Graphics::Direct2D::Common::D2D1_FIGURE_BEGIN_HOLLOW,
                            );
                            sink.AddArc(&D2D1_ARC_SEGMENT {
                                point: D2D_POINT_2F {
                                    x: badge_cx + 6.2,
                                    y: cy + 1.5,
                                },
                                size: D2D_SIZE_F {
                                    width: 6.2,
                                    height: 6.8,
                                },
                                rotationAngle: 0.0,
                                sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                                arcSize: D2D1_ARC_SIZE_SMALL,
                            });
                            sink.EndFigure(
                                windows::Win32::Graphics::Direct2D::Common::D2D1_FIGURE_END_OPEN,
                            );
                            if sink.Close().is_ok() {
                                self.set_color(1.0, 1.0, 1.0, hp_a);
                                self.d2d_context.DrawGeometry(&geo, &self.solid_brush, 1.7, None);
                            }
                        }
                    }
                    self.set_color(1.0, 1.0, 1.0, hp_a);
                    for side in [-1.0_f32, 1.0_f32] {
                        let ex = badge_cx + side * 5.6;
                        self.d2d_context.FillRoundedRectangle(
                            &D2D1_ROUNDED_RECT {
                                rect: D2D_RECT_F {
                                    left: ex - 1.8,
                                    top: cy + 0.5,
                                    right: ex + 1.8,
                                    bottom: cy + 6.2,
                                },
                                radiusX: 1.5,
                                radiusY: 1.5,
                            },
                            &self.solid_brush,
                        );
                    }
                }
                CollapsedOsdKind::Brightness => {
                    let ratio = (springs.brightness_pct.value / 100.0).clamp(0.0, 1.0);
                    let sun_cx = body_left + 20.0;
                    let core_r = 3.2 + 0.9 * ratio;
                    self.draw_radial_glow(
                        sun_cx,
                        cy,
                        11.5,
                        (1.0, 0.86, 0.42),
                        (0.14 + 0.16 * ratio) * osd_a,
                    );
                    self.set_color(1.0, 0.93, 0.72, 0.96 * osd_a);
                    self.d2d_context.FillEllipse(
                        &D2D1_ELLIPSE {
                            point: D2D_POINT_2F { x: sun_cx, y: cy },
                            radiusX: core_r,
                            radiusY: core_r,
                        },
                        &self.solid_brush,
                    );
                    let ray_in = core_r + 1.8;
                    let ray_out = ray_in + 1.4 + 1.5 * ratio;
                    let rot = ratio * 0.65;
                    for i in 0..8 {
                        let ang = rot + (i as f32) * std::f32::consts::FRAC_PI_4;
                        let (s, c) = ang.sin_cos();
                        self.d2d_context.DrawLine(
                            D2D_POINT_2F { x: sun_cx + c * ray_in, y: cy + s * ray_in },
                            D2D_POINT_2F { x: sun_cx + c * ray_out, y: cy + s * ray_out },
                            &self.solid_brush,
                            1.35,
                            None,
                        );
                    }
                    let bar_l = body_left + 38.0;
                    let bar_r = (body_right - 14.0).max(bar_l + 40.0);
                    let bar_h = 5.2;
                    let track_r = D2D_RECT_F {
                        left: bar_l,
                        top: cy - bar_h * 0.5,
                        right: bar_r,
                        bottom: cy + bar_h * 0.5,
                    };
                    self.set_color(1.0, 1.0, 1.0, 0.22 * osd_a);
                    self.d2d_context.FillRoundedRectangle(
                        &D2D1_ROUNDED_RECT {
                            rect: track_r,
                            radiusX: bar_h * 0.5,
                            radiusY: bar_h * 0.5,
                        },
                        &self.solid_brush,
                    );
                    let fill_w = (bar_r - bar_l) * ratio;
                    if fill_w > 1.0 {
                        let fill_r = D2D_RECT_F {
                            left: bar_l,
                            top: track_r.top,
                            right: (bar_l + fill_w).max(bar_l + bar_h),
                            bottom: track_r.bottom,
                        };
                        self.set_color(1.0, 0.96, 0.86, 0.98 * osd_a);
                        self.d2d_context.FillRoundedRectangle(
                            &D2D1_ROUNDED_RECT {
                                rect: fill_r,
                                radiusX: bar_h * 0.5,
                                radiusY: bar_h * 0.5,
                            },
                            &self.solid_brush,
                        );
                    }
                }
                CollapsedOsdKind::MicMute { .. } => {
                    let m = springs.mic_muted_mix.value().clamp(0.0, 1.0);
                    let icx = body_left + 20.0;
                    let fg_r = 0.95 + (1.00 - 0.95) * m;
                    let fg_g = 0.96 + (0.45 - 0.96) * m;
                    let fg_b = 0.98 + (0.46 - 0.98) * m;
                    self.set_color(fg_r, fg_g, fg_b, 0.96 * osd_a);
                    self.d2d_context.FillRoundedRectangle(
                        &D2D1_ROUNDED_RECT {
                            rect: D2D_RECT_F {
                                left: icx - 2.6,
                                top: cy - 6.8,
                                right: icx + 2.6,
                                bottom: cy + 1.2,
                            },
                            radiusX: 2.6,
                            radiusY: 2.6,
                        },
                        &self.solid_brush,
                    );
                    if let Ok(geo) = self.d2d_factory.CreatePathGeometry() {
                        if let Ok(sink) = geo.Open() {
                            sink.BeginFigure(
                                D2D_POINT_2F { x: icx - 5.2, y: cy - 1.2 },
                                windows::Win32::Graphics::Direct2D::Common::D2D1_FIGURE_BEGIN_HOLLOW,
                            );
                            sink.AddArc(&D2D1_ARC_SEGMENT {
                                point: D2D_POINT_2F { x: icx + 5.2, y: cy - 1.2 },
                                size: D2D_SIZE_F { width: 5.2, height: 5.2 },
                                rotationAngle: 0.0,
                                sweepDirection: D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE,
                                arcSize: D2D1_ARC_SIZE_SMALL,
                            });
                            sink.EndFigure(
                                windows::Win32::Graphics::Direct2D::Common::D2D1_FIGURE_END_OPEN,
                            );
                            if sink.Close().is_ok() {
                                self.d2d_context.DrawGeometry(&geo, &self.solid_brush, 1.45, None);
                            }
                        }
                    }
                    self.d2d_context.DrawLine(
                        D2D_POINT_2F { x: icx, y: cy + 4.0 },
                        D2D_POINT_2F { x: icx, y: cy + 6.8 },
                        &self.solid_brush,
                        1.45,
                        None,
                    );
                    self.d2d_context.DrawLine(
                        D2D_POINT_2F { x: icx - 3.0, y: cy + 6.8 },
                        D2D_POINT_2F { x: icx + 3.0, y: cy + 6.8 },
                        &self.solid_brush,
                        1.45,
                        None,
                    );
                    if m > 0.02 {
                        let x0 = icx - 6.4;
                        let y0 = cy - 6.2;
                        let x1 = x0 + 12.8 * m;
                        let y1 = y0 + 12.6 * m;
                        self.set_color(0.0, 0.0, 0.0, 0.92 * osd_a);
                        self.d2d_context.DrawLine(
                            D2D_POINT_2F { x: x0, y: y0 + 1.2 },
                            D2D_POINT_2F { x: x1, y: y1 + 1.2 },
                            &self.solid_brush,
                            2.4,
                            None,
                        );
                        self.set_color(1.0, 0.34, 0.36, 0.98 * m * osd_a);
                        self.d2d_context.DrawLine(
                            D2D_POINT_2F { x: x0, y: y0 },
                            D2D_POINT_2F { x: x1, y: y1 },
                            &self.solid_brush,
                            1.65,
                            None,
                        );
                    }
                    let state_left = (body_right - 68.0).max(body_left + 44.0);
                    self.draw_text(
                        "Mic",
                        &self.fmt_osd_label,
                        &D2D_RECT_F {
                            left: body_left + 35.0,
                            top: 0.0,
                            right: state_left,
                            bottom: idle_h,
                        },
                        (0.96, 0.97, 0.98, osd_a),
                    );
                    let state_right = (body_right - 14.0).max(state_left + 10.0);
                    let clip = D2D_RECT_F {
                        left: state_left,
                        top: 3.0,
                        right: state_right,
                        bottom: (idle_h - 3.0).max(6.0),
                    };
                    self.d2d_context
                        .PushAxisAlignedClip(&clip, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);

                    let mic_moving = m > 0.005 && m < 0.995;
                    if 1.0 - m > 0.01 {
                        let dy_live = -18.0 * m;
                        let live_rect = D2D_RECT_F {
                            left: state_left,
                            top: dy_live,
                            right: state_right,
                            bottom: idle_h + dy_live,
                        };
                        let live_col = (0.180, 0.902, 0.376, 0.98 * (1.0 - m) * osd_a);
                        if mic_moving {
                            self.draw_text_smooth("Live", &self.fmt_osd_state, &live_rect, live_col);
                        } else {
                            self.draw_text("Live", &self.fmt_osd_state, &live_rect, live_col);
                        }
                    }
                    if m > 0.01 {
                        let dy_muted = 18.0 * (1.0 - m);
                        let muted_rect = D2D_RECT_F {
                            left: state_left,
                            top: dy_muted,
                            right: state_right,
                            bottom: idle_h + dy_muted,
                        };
                        let muted_col = (1.0, 0.36, 0.38, 0.98 * m * osd_a);
                        if mic_moving {
                            self.draw_text_smooth(
                                "Muted",
                                &self.fmt_osd_state,
                                &muted_rect,
                                muted_col,
                            );
                        } else {
                            self.draw_text("Muted", &self.fmt_osd_state, &muted_rect, muted_col);
                        }
                    }
                    self.d2d_context.PopAxisAlignedClip();
                }
                CollapsedOsdKind::MicInUse => {
                    let app_name = if state.widgets.active_mic_app.is_empty() {
                        "Active"
                    } else {
                        state.widgets.active_mic_app.as_str()
                    };
                    let icx = body_left + 20.0;
                    let pulse = 0.78 + 0.22 * (springs.phase_secs() * 3.2).sin().abs();
                    self.draw_radial_glow(
                        icx,
                        cy,
                        11.0,
                        (1.0, 0.62, 0.14),
                        0.22 * pulse * osd_a,
                    );
                    self.set_color(1.0, 0.66, 0.18, 0.98 * osd_a);
                    self.d2d_context.FillRoundedRectangle(
                        &D2D1_ROUNDED_RECT {
                            rect: D2D_RECT_F {
                                left: icx - 2.6,
                                top: cy - 6.8,
                                right: icx + 2.6,
                                bottom: cy + 1.2,
                            },
                            radiusX: 2.6,
                            radiusY: 2.6,
                        },
                        &self.solid_brush,
                    );
                    if let Ok(geo) = self.d2d_factory.CreatePathGeometry() {
                        if let Ok(sink) = geo.Open() {
                            sink.BeginFigure(
                                D2D_POINT_2F { x: icx - 5.2, y: cy - 1.2 },
                                windows::Win32::Graphics::Direct2D::Common::D2D1_FIGURE_BEGIN_HOLLOW,
                            );
                            sink.AddArc(&D2D1_ARC_SEGMENT {
                                point: D2D_POINT_2F { x: icx + 5.2, y: cy - 1.2 },
                                size: D2D_SIZE_F { width: 5.2, height: 5.2 },
                                rotationAngle: 0.0,
                                sweepDirection: D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE,
                                arcSize: D2D1_ARC_SIZE_SMALL,
                            });
                            sink.EndFigure(
                                windows::Win32::Graphics::Direct2D::Common::D2D1_FIGURE_END_OPEN,
                            );
                            if sink.Close().is_ok() {
                                self.d2d_context.DrawGeometry(&geo, &self.solid_brush, 1.45, None);
                            }
                        }
                    }
                    self.d2d_context.DrawLine(
                        D2D_POINT_2F { x: icx, y: cy + 4.0 },
                        D2D_POINT_2F { x: icx, y: cy + 6.8 },
                        &self.solid_brush,
                        1.45,
                        None,
                    );
                    self.d2d_context.DrawLine(
                        D2D_POINT_2F { x: icx - 3.0, y: cy + 6.8 },
                        D2D_POINT_2F { x: icx + 3.0, y: cy + 6.8 },
                        &self.solid_brush,
                        1.45,
                        None,
                    );
                    self.set_color(1.0, 0.58, 0.10, pulse * osd_a);
                    self.d2d_context.FillEllipse(
                        &D2D1_ELLIPSE {
                            point: D2D_POINT_2F { x: icx + 6.2, y: cy - 5.2 },
                            radiusX: 2.1,
                            radiusY: 2.1,
                        },
                        &self.solid_brush,
                    );

                    let state_left = (body_right - 98.0).max(body_left + 104.0);
                    self.draw_text(
                        "Mic in use",
                        &self.fmt_osd_label,
                        &D2D_RECT_F {
                            left: body_left + 35.0,
                            top: 0.0,
                            right: state_left,
                            bottom: idle_h,
                        },
                        (0.96, 0.97, 0.98, osd_a),
                    );
                    self.draw_text(
                        app_name,
                        &self.fmt_osd_state,
                        &D2D_RECT_F {
                            left: state_left,
                            top: 0.0,
                            right: (body_right - 14.0).max(state_left + 10.0),
                            bottom: idle_h,
                        },
                        (1.0, 0.66, 0.18, 0.98 * osd_a),
                    );
                }
                CollapsedOsdKind::AudioDevice => {
                    let dev_name = if state.widgets.active_audio_device.is_empty() {
                        "Audio Output"
                    } else {
                        state.widgets.active_audio_device.as_str()
                    };
                    let badge_cx = body_left + 29.0;
                    let badge_r = 14.5;
                    let (acc_r, acc_g, acc_b) = (0.369, 0.761, 1.0);
                    self.draw_radial_glow(badge_cx, cy, 20.0, (acc_r, acc_g, acc_b), 0.24 * osd_a);
                    self.set_color(acc_r, acc_g, acc_b, 0.18 * osd_a);
                    self.d2d_context.FillEllipse(
                        &D2D1_ELLIPSE {
                            point: D2D_POINT_2F { x: badge_cx, y: cy },
                            radiusX: badge_r,
                            radiusY: badge_r,
                        },
                        &self.solid_brush,
                    );
                    self.set_color(acc_r, acc_g, acc_b, 0.72 * osd_a);
                    self.d2d_context.DrawEllipse(
                        &D2D1_ELLIPSE {
                            point: D2D_POINT_2F { x: badge_cx, y: cy },
                            radiusX: badge_r,
                            radiusY: badge_r,
                        },
                        &self.solid_brush,
                        1.6,
                        None,
                    );
                    self.set_color(1.0, 1.0, 1.0, 0.98 * osd_a);
                    self.d2d_context.FillRoundedRectangle(
                        &D2D1_ROUNDED_RECT {
                            rect: D2D_RECT_F {
                                left: badge_cx - 2.6,
                                top: cy - 6.8,
                                right: badge_cx + 2.6,
                                bottom: cy + 1.2,
                            },
                            radiusX: 2.6,
                            radiusY: 2.6,
                        },
                        &self.solid_brush,
                    );
                    if let Ok(geo) = self.d2d_factory.CreatePathGeometry() {
                        if let Ok(sink) = geo.Open() {
                            sink.BeginFigure(
                                D2D_POINT_2F { x: badge_cx - 5.2, y: cy - 1.2 },
                                windows::Win32::Graphics::Direct2D::Common::D2D1_FIGURE_BEGIN_HOLLOW,
                            );
                            sink.AddArc(&D2D1_ARC_SEGMENT {
                                point: D2D_POINT_2F { x: badge_cx + 5.2, y: cy - 1.2 },
                                size: D2D_SIZE_F { width: 5.2, height: 5.2 },
                                rotationAngle: 0.0,
                                sweepDirection: D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE,
                                arcSize: D2D1_ARC_SIZE_SMALL,
                            });
                            sink.EndFigure(
                                windows::Win32::Graphics::Direct2D::Common::D2D1_FIGURE_END_OPEN,
                            );
                            if sink.Close().is_ok() {
                                self.d2d_context.DrawGeometry(&geo, &self.solid_brush, 1.5, None);
                            }
                        }
                    }
                    self.d2d_context.DrawLine(
                        D2D_POINT_2F { x: badge_cx, y: cy + 4.0 },
                        D2D_POINT_2F { x: badge_cx, y: cy + 6.8 },
                        &self.solid_brush,
                        1.5,
                        None,
                    );
                    self.d2d_context.DrawLine(
                        D2D_POINT_2F { x: badge_cx - 3.0, y: cy + 6.8 },
                        D2D_POINT_2F { x: badge_cx + 3.0, y: cy + 6.8 },
                        &self.solid_brush,
                        1.5,
                        None,
                    );

                    let txt_l = body_left + 54.0;
                    let txt_r = (body_right - 16.0).max(txt_l + 40.0);
                    self.draw_text(
                        dev_name,
                        &self.fmt_osd_label,
                        &D2D_RECT_F {
                            left: txt_l,
                            top: cy - 17.0,
                            right: txt_r,
                            bottom: cy + 1.0,
                        },
                        (0.98, 0.98, 0.99, osd_a),
                    );
                    self.draw_text(
                        "Audio output switched",
                        &self.fmt_artist,
                        &D2D_RECT_F {
                            left: txt_l,
                            top: cy + 1.0,
                            right: txt_r,
                            bottom: cy + 17.0,
                        },
                        (0.60, 0.62, 0.66, osd_a),
                    );
                }
                CollapsedOsdKind::Network => {
                    let (is_wifi, connected, name, sig_pct) = match &state.widgets.active_network {
                        Some(ev) => (
                            ev.kind == crate::osd_monitors::NetworkKind::Wifi,
                            ev.connected,
                            ev.name.as_str(),
                            ev.signal_pct,
                        ),
                        None => (true, true, "Wi-Fi", 85),
                    };
                    let badge_cx = body_left + 29.0;
                    let badge_r = 14.5;
                    let (acc_r, acc_g, acc_b) = if !connected {
                        (1.0, 0.36, 0.38)
                    } else if is_wifi {
                        (0.20, 0.84, 0.64)
                    } else {
                        (0.28, 0.66, 1.0)
                    };
                    self.draw_radial_glow(badge_cx, cy, 20.0, (acc_r, acc_g, acc_b), 0.24 * osd_a);
                    self.set_color(acc_r, acc_g, acc_b, 0.18 * osd_a);
                    self.d2d_context.FillEllipse(
                        &D2D1_ELLIPSE {
                            point: D2D_POINT_2F { x: badge_cx, y: cy },
                            radiusX: badge_r,
                            radiusY: badge_r,
                        },
                        &self.solid_brush,
                    );
                    self.set_color(acc_r, acc_g, acc_b, 0.76 * osd_a);
                    self.d2d_context.DrawEllipse(
                        &D2D1_ELLIPSE {
                            point: D2D_POINT_2F { x: badge_cx, y: cy },
                            radiusX: badge_r,
                            radiusY: badge_r,
                        },
                        &self.solid_brush,
                        1.6,
                        None,
                    );
                    let round_cap = ID2D1Factory::CreateStrokeStyle(
                        &self.d2d_factory,
                        &D2D1_STROKE_STYLE_PROPERTIES {
                            startCap: D2D1_CAP_STYLE_ROUND,
                            endCap: D2D1_CAP_STYLE_ROUND,
                            lineJoin: D2D1_LINE_JOIN_ROUND,
                            ..Default::default()
                        },
                        None,
                    )
                    .ok();
                    if is_wifi {
                        let dot_y = cy + 5.6;
                        self.set_color(1.0, 1.0, 1.0, (if connected { 0.98 } else { 0.52 }) * osd_a);
                        self.d2d_context.FillEllipse(
                            &D2D1_ELLIPSE {
                                point: D2D_POINT_2F { x: badge_cx, y: dot_y },
                                radiusX: 1.85,
                                radiusY: 1.85,
                            },
                            &self.solid_brush,
                        );
                        let active_arcs = if !connected {
                            0
                        } else if sig_pct >= 66 {
                            3
                        } else if sig_pct >= 33 {
                            2
                        } else {
                            1
                        };
                        for (idx, r) in [5.0_f32, 8.3, 11.6].iter().enumerate() {
                            let arc_a = if idx < active_arcs { 0.98 } else { 0.28 } * osd_a;
                            self.set_color(1.0, 1.0, 1.0, arc_a);
                            let a0 = (-132.0_f32).to_radians();
                            let a1 = (-48.0_f32).to_radians();
                            if let Ok(geo) = self.d2d_factory.CreatePathGeometry() {
                                if let Ok(sink) = geo.Open() {
                                    sink.BeginFigure(
                                        D2D_POINT_2F {
                                            x: badge_cx + r * a0.cos(),
                                            y: dot_y + r * a0.sin(),
                                        },
                                        windows::Win32::Graphics::Direct2D::Common::D2D1_FIGURE_BEGIN_HOLLOW,
                                    );
                                    sink.AddArc(&D2D1_ARC_SEGMENT {
                                        point: D2D_POINT_2F {
                                            x: badge_cx + r * a1.cos(),
                                            y: dot_y + r * a1.sin(),
                                        },
                                        size: D2D_SIZE_F { width: *r, height: *r },
                                        rotationAngle: 0.0,
                                        sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                                        arcSize: D2D1_ARC_SIZE_SMALL,
                                    });
                                    sink.EndFigure(
                                        windows::Win32::Graphics::Direct2D::Common::D2D1_FIGURE_END_OPEN,
                                    );
                                    if sink.Close().is_ok() {
                                        self.d2d_context.DrawGeometry(
                                            &geo,
                                            &self.solid_brush,
                                            2.0,
                                            round_cap.as_ref(),
                                        );
                                    }
                                }
                            }
                        }
                        if !connected {
                            self.set_color(1.0, 0.42, 0.44, 0.96 * osd_a);
                            self.d2d_context.DrawLine(
                                D2D_POINT_2F { x: badge_cx - 6.8, y: cy - 6.2 },
                                D2D_POINT_2F { x: badge_cx + 6.8, y: cy + 6.8 },
                                &self.solid_brush,
                                1.9,
                                round_cap.as_ref(),
                            );
                        }
                    } else {
                        let eth_a = (if connected { 0.98 } else { 0.56 }) * osd_a;
                        self.set_color(1.0, 1.0, 1.0, eth_a);
                        // Sleek RJ45 Ethernet connector: filled head + gold-pin cutouts + strain collar + cable
                        let head_r = D2D1_ROUNDED_RECT {
                            rect: D2D_RECT_F {
                                left: badge_cx - 6.2,
                                top: cy - 6.2,
                                right: badge_cx + 6.2,
                                bottom: cy + 1.8,
                            },
                            radiusX: 2.2,
                            radiusY: 2.2,
                        };
                        self.d2d_context.DrawRoundedRectangle(
                            &head_r,
                            &self.solid_brush,
                            1.65,
                            round_cap.as_ref(),
                        );
                        // 4 crisp contact pins along top inner edge
                        for dx in [-3.15_f32, -1.05, 1.05, 3.15] {
                            self.d2d_context.DrawLine(
                                D2D_POINT_2F { x: badge_cx + dx, y: cy - 5.4 },
                                D2D_POINT_2F { x: badge_cx + dx, y: cy - 2.3 },
                                &self.solid_brush,
                                1.25,
                                round_cap.as_ref(),
                            );
                        }
                        // Tapered strain-relief collar
                        self.d2d_context.FillRoundedRectangle(
                            &D2D1_ROUNDED_RECT {
                                rect: D2D_RECT_F {
                                    left: badge_cx - 3.0,
                                    top: cy + 1.6,
                                    right: badge_cx + 3.0,
                                    bottom: cy + 4.4,
                                },
                                radiusX: 1.1,
                                radiusY: 1.1,
                            },
                            &self.solid_brush,
                        );
                        // Cable stem
                        self.d2d_context.DrawLine(
                            D2D_POINT_2F { x: badge_cx, y: cy + 4.2 },
                            D2D_POINT_2F { x: badge_cx, y: cy + 7.4 },
                            &self.solid_brush,
                            1.9,
                            round_cap.as_ref(),
                        );
                        if !connected {
                            self.set_color(1.0, 0.42, 0.44, 0.96 * osd_a);
                            self.d2d_context.DrawLine(
                                D2D_POINT_2F { x: badge_cx - 6.8, y: cy - 6.2 },
                                D2D_POINT_2F { x: badge_cx + 6.8, y: cy + 6.8 },
                                &self.solid_brush,
                                1.9,
                                round_cap.as_ref(),
                            );
                        }
                    }

                    let txt_l = body_left + 54.0;
                    let txt_r = (body_right - 16.0).max(txt_l + 40.0);
                    let title_s = if name.is_empty() {
                        if is_wifi { "Wi-Fi" } else { "Ethernet" }
                    } else {
                        name
                    };
                    let sub_s = match (is_wifi, connected) {
                        (true, true) => "Wi-Fi connected",
                        (true, false) => "Wi-Fi disconnected",
                        (false, true) => "Ethernet connected",
                        (false, false) => "Ethernet disconnected",
                    };
                    self.draw_text(
                        title_s,
                        &self.fmt_osd_label,
                        &D2D_RECT_F {
                            left: txt_l,
                            top: cy - 17.0,
                            right: txt_r,
                            bottom: cy + 1.0,
                        },
                        (0.98, 0.98, 0.99, osd_a),
                    );
                    self.draw_text(
                        sub_s,
                        &self.fmt_artist,
                        &D2D_RECT_F {
                            left: txt_l,
                            top: cy + 1.0,
                            right: txt_r,
                            bottom: cy + 17.0,
                        },
                        if connected {
                            (0.60, 0.62, 0.66, osd_a)
                        } else {
                            (1.0, 0.44, 0.44, 0.92 * osd_a)
                        },
                    );
                }
                CollapsedOsdKind::Charging { charging, pct } => {
                    let pct = pct.min(100);
                    let ring_cx = body_left + 29.0;
                    let ring_r = 14.0;
                    let stroke_w = 4.2;
                    let (acc_r, acc_g, acc_b) = if charging {
                        (0.200, 0.878, 0.478)
                    } else if pct <= 15 {
                        (1.0, 0.27, 0.23)
                    } else {
                        (1.0, 0.72, 0.14)
                    };

                    self.set_color(acc_r, acc_g, acc_b, 0.26 * osd_a);
                    self.d2d_context.DrawEllipse(
                        &D2D1_ELLIPSE {
                            point: D2D_POINT_2F { x: ring_cx, y: cy },
                            radiusX: ring_r,
                            radiusY: ring_r,
                        },
                        &self.solid_brush,
                        stroke_w,
                        None,
                    );
                    let fill_ratio = springs.bt_popup_arc.value().clamp(0.0, 1.0);
                    let sweep = fill_ratio * 359.5;
                    if sweep > 1.0 {
                        let a0 = (-90.0_f32).to_radians();
                        let a1 = (-90.0_f32 + sweep).to_radians();
                        if let Ok(geo) = self.d2d_factory.CreatePathGeometry() {
                            if let Ok(sink) = geo.Open() {
                                sink.BeginFigure(
                                    D2D_POINT_2F {
                                        x: ring_cx + ring_r * a0.cos(),
                                        y: cy + ring_r * a0.sin(),
                                    },
                                    windows::Win32::Graphics::Direct2D::Common::D2D1_FIGURE_BEGIN_HOLLOW,
                                );
                                sink.AddArc(&D2D1_ARC_SEGMENT {
                                    point: D2D_POINT_2F {
                                        x: ring_cx + ring_r * a1.cos(),
                                        y: cy + ring_r * a1.sin(),
                                    },
                                    size: D2D_SIZE_F { width: ring_r, height: ring_r },
                                    rotationAngle: 0.0,
                                    sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                                    arcSize: if sweep > 180.0 {
                                        windows::Win32::Graphics::Direct2D::D2D1_ARC_SIZE_LARGE
                                    } else {
                                        D2D1_ARC_SIZE_SMALL
                                    },
                                });
                                sink.EndFigure(
                                    windows::Win32::Graphics::Direct2D::Common::D2D1_FIGURE_END_OPEN,
                                );
                                if sink.Close().is_ok() {
                                    let round_st = ID2D1Factory::CreateStrokeStyle(
                                        &self.d2d_factory,
                                        &D2D1_STROKE_STYLE_PROPERTIES {
                                            startCap: D2D1_CAP_STYLE_ROUND,
                                            endCap: D2D1_CAP_STYLE_ROUND,
                                            ..Default::default()
                                        },
                                        None,
                                    )
                                    .ok();
                                    self.set_color(acc_r, acc_g, acc_b, 0.98 * osd_a);
                                    self.d2d_context.DrawGeometry(
                                        &geo,
                                        &self.solid_brush,
                                        stroke_w,
                                        round_st.as_ref(),
                                    );
                                }
                            }
                        }
                    }

                    if let Ok(geo) = self.d2d_factory.CreatePathGeometry() {
                        if let Ok(sink) = geo.Open() {
                            sink.BeginFigure(
                                D2D_POINT_2F { x: ring_cx + 1.4, y: cy - 6.6 },
                                D2D1_FIGURE_BEGIN_FILLED,
                            );
                            sink.AddLine(D2D_POINT_2F { x: ring_cx - 4.0, y: cy + 0.8 });
                            sink.AddLine(D2D_POINT_2F { x: ring_cx - 0.3, y: cy + 0.8 });
                            sink.AddLine(D2D_POINT_2F { x: ring_cx - 1.5, y: cy + 6.6 });
                            sink.AddLine(D2D_POINT_2F { x: ring_cx + 4.1, y: cy - 0.9 });
                            sink.AddLine(D2D_POINT_2F { x: ring_cx + 0.4, y: cy - 0.9 });
                            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
                            if sink.Close().is_ok() {
                                self.set_color(1.0, 1.0, 1.0, 0.98 * osd_a);
                                self.d2d_context.FillGeometry(&geo, &self.solid_brush, None);
                            }
                        }
                    }

                    let pct_s = format!("{pct}%");
                    let pct_w = 64.0;
                    let txt_l = body_left + 54.0;
                    let txt_r = (body_right - 16.0 - pct_w).max(txt_l + 40.0);
                    let title_s = if charging { "Charging" } else { "On Battery" };
                    let sub_s = if charging {
                        "Charger connected"
                    } else {
                        "Charger unplugged"
                    };
                    self.draw_text(
                        title_s,
                        &self.fmt_osd_label,
                        &D2D_RECT_F {
                            left: txt_l,
                            top: cy - 17.0,
                            right: txt_r,
                            bottom: cy + 1.0,
                        },
                        (0.98, 0.98, 0.99, osd_a),
                    );
                    self.draw_text(
                        sub_s,
                        &self.fmt_artist,
                        &D2D_RECT_F {
                            left: txt_l,
                            top: cy + 1.0,
                            right: txt_r,
                            bottom: cy + 17.0,
                        },
                        (0.60, 0.62, 0.66, osd_a),
                    );

                    self.draw_text(
                        &pct_s,
                        &self.fmt_wg_big,
                        &D2D_RECT_F {
                            left: body_right - 16.0 - pct_w,
                            top: cy - 16.0,
                            right: body_right - 14.0,
                            bottom: cy + 16.0,
                        },
                        (0.98, 0.98, 1.0, osd_a),
                    );
                }
                CollapsedOsdKind::Notification => {
                    let (app, title, body, acc, is_circle) = match &state.widgets.active_toast {
                        Some(t) => (
                            t.app.as_str(),
                            t.title.as_str(),
                            t.body.as_str(),
                            t.accent_rgb,
                            t.icon_is_circle,
                        ),
                        None => ("Notification", "Eave", "New notification", (0.24, 0.64, 1.0), false),
                    };

                    let badge_s = 32.0;
                    let badge_l = body_left + 14.0;
                    let badge_cx = badge_l + badge_s * 0.5;
                    self.draw_radial_glow(badge_cx, cy, 38.0, acc, 0.18 * osd_a);

                    let badge_r = D2D_RECT_F {
                        left: badge_l,
                        top: cy - badge_s * 0.5,
                        right: badge_l + badge_s,
                        bottom: cy + badge_s * 0.5,
                    };
                    let badge_rad = if is_circle { badge_s * 0.5 } else { 9.0 };
                    let badge_rr = D2D1_ROUNDED_RECT {
                        rect: badge_r,
                        radiusX: badge_rad,
                        radiusY: badge_rad,
                    };
                    if let Some((_, ref icon_bmp)) = self.toast_icon_bmp {
                        if let Ok(clip_geo) = self.d2d_factory.CreateRoundedRectangleGeometry(&badge_rr) {
                            let layer_params = D2D1_LAYER_PARAMETERS1 {
                                contentBounds: badge_r,
                                geometricMask: ManuallyDrop::new(Some(clip_geo.into())),
                                maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                                maskTransform: Matrix3x2::identity(),
                                opacity: osd_a,
                                opacityBrush: ManuallyDrop::new(None),
                                layerOptions: D2D1_LAYER_OPTIONS1_NONE,
                            };
                            self.d2d_context.PushLayer(&layer_params, None);
                            self.d2d_context.DrawBitmap(
                                icon_bmp,
                                Some(&badge_r),
                                1.0,
                                D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
                                None,
                                None,
                            );
                            self.d2d_context.PopLayer();
                        } else {
                            self.d2d_context.DrawBitmap(
                                icon_bmp,
                                Some(&badge_r),
                                osd_a,
                                D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
                                None,
                                None,
                            );
                        }
                        self.set_color(1.0, 1.0, 1.0, 0.22 * osd_a);
                        self.d2d_context
                            .DrawRoundedRectangle(&badge_rr, &self.solid_brush, 0.9, None);
                    } else {
                        self.set_color(acc.0, acc.1, acc.2, 0.94 * osd_a);
                        self.d2d_context
                            .FillRoundedRectangle(&badge_rr, &self.solid_brush);
                        self.set_color(1.0, 1.0, 1.0, 0.22 * osd_a);
                        self.d2d_context
                            .DrawRoundedRectangle(&badge_rr, &self.solid_brush, 0.9, None);
                        self.set_color(1.0, 1.0, 1.0, 0.96 * osd_a);
                        icons::draw_bell(
                            &self.d2d_context,
                            badge_cx,
                            cy,
                            16.5,
                            &self.solid_brush,
                            None,
                        );
                    }

                    let txt_l = badge_r.right + 10.0;
                    let txt_r = (body_right - 16.0).max(txt_l + 60.0);
                    let now_w = 28.0;
                    self.draw_text(
                        app,
                        &self.fmt_wg_sub,
                        &D2D_RECT_F {
                            left: txt_l,
                            top: cy - 22.0,
                            right: txt_r - now_w - 4.0,
                            bottom: cy - 9.0,
                        },
                        (acc.0 * 0.45 + 0.55, acc.1 * 0.45 + 0.55, acc.2 * 0.45 + 0.55, 0.92 * osd_a),
                    );
                    self.draw_text(
                        "now",
                        &self.fmt_wg_sub,
                        &D2D_RECT_F {
                            left: txt_r - now_w,
                            top: cy - 22.0,
                            right: txt_r,
                            bottom: cy - 9.0,
                        },
                        (0.56, 0.58, 0.62, 0.85 * osd_a),
                    );
                    self.draw_text(
                        title,
                        &self.fmt_osd_label,
                        &D2D_RECT_F {
                            left: txt_l,
                            top: cy - 10.0,
                            right: txt_r,
                            bottom: cy + 6.0,
                        },
                        (0.98, 0.98, 1.0, osd_a),
                    );
                    if !body.is_empty() {
                        self.draw_text(
                            body,
                            &self.fmt_artist,
                            &D2D_RECT_F {
                                left: txt_l,
                                top: cy + 5.0,
                                right: txt_r,
                                bottom: cy + 21.0,
                            },
                            (0.68, 0.70, 0.75, 0.94 * osd_a),
                        );
                    }
                }
                CollapsedOsdKind::FocusAlert {
                    next_is_focus,
                    is_long_break,
                    next_mins,
                } => {
                    self.draw_interactive_alert_osd(
                        body_left,
                        body_right,
                        idle_h,
                        springs.phase_secs(),
                        osd_a,
                        state.widgets.osd_alert_progress,
                        0,
                        if next_is_focus {
                            "Break over"
                        } else {
                            "Focus complete"
                        },
                        &if next_is_focus {
                            format!("Back to focus, {next_mins} min")
                        } else if is_long_break {
                            format!("Long break, {next_mins} min")
                        } else {
                            format!("Take a {next_mins} min break")
                        },
                        if next_is_focus { "Focus" } else { "Break" },
                        (0.55, 0.38, 1.0),
                        (0.74, 0.42, 1.0),
                    );
                }
                CollapsedOsdKind::WaterReminder => {
                    let sub = format!(
                        "Drink {} ml · {}/{} ml",
                        state.water.bottle_ml, state.water.taken_ml, state.water.goal_ml
                    );
                    self.draw_interactive_alert_osd(
                        body_left,
                        body_right,
                        idle_h,
                        springs.phase_secs(),
                        osd_a,
                        state.widgets.osd_alert_progress,
                        1,
                        "Stay hydrated",
                        &sub,
                        "Drink",
                        (0.10, 0.62, 1.0),
                        (0.24, 0.82, 1.0),
                    );
                }
                CollapsedOsdKind::TodoReminder => {
                    let title = if state.todo.active_reminder_title.is_empty() {
                        "Task reminder"
                    } else {
                        &state.todo.active_reminder_title
                    };
                    let sub = if state.todo.active_reminder_time.is_empty() {
                        "Scheduled task".to_string()
                    } else {
                        format!("Scheduled · {}", state.todo.active_reminder_time)
                    };
                    self.draw_interactive_alert_osd(
                        body_left,
                        body_right,
                        idle_h,
                        springs.phase_secs(),
                        osd_a,
                        state.widgets.osd_alert_progress,
                        2,
                        title,
                        &sub,
                        "Done",
                        (0.14, 0.76, 0.38),
                        (0.26, 0.92, 0.50),
                    );
                }
                CollapsedOsdKind::UpdateAvailable => {
                    let sub = if state.widgets.update_available_ver.is_empty() {
                        "A new version of Eave is ready".to_string()
                    } else {
                        format!("Eave v{} is ready", state.widgets.update_available_ver)
                    };
                    self.draw_interactive_alert_osd(
                        body_left,
                        body_right,
                        idle_h,
                        springs.phase_secs(),
                        osd_a,
                        state.widgets.osd_alert_progress,
                        3,
                        "Update available",
                        &sub,
                        "Update",
                        (0.14, 0.56, 1.0),
                        (0.36, 0.80, 1.0),
                    );
                }
                CollapsedOsdKind::UpdateInstalled => {
                    let badge_s = 32.0;
                    let badge_l = body_left + 14.0;
                    let badge_cx = badge_l + badge_s * 0.5;
                    let badge_r = badge_s * 0.5;
                    self.draw_radial_glow(badge_cx, cy, 38.0, (0.16, 0.84, 0.44), 0.24 * osd_a);
                    self.set_color(0.14, 0.78, 0.40, 0.24 * osd_a);
                    self.d2d_context.FillEllipse(
                        &D2D1_ELLIPSE {
                            point: D2D_POINT_2F { x: badge_cx, y: cy },
                            radiusX: badge_r,
                            radiusY: badge_r,
                        },
                        &self.solid_brush,
                    );
                    self.set_color(0.24, 0.90, 0.50, 0.86 * osd_a);
                    self.d2d_context.DrawEllipse(
                        &D2D1_ELLIPSE {
                            point: D2D_POINT_2F { x: badge_cx, y: cy },
                            radiusX: badge_r,
                            radiusY: badge_r,
                        },
                        &self.solid_brush,
                        1.65,
                        None,
                    );
                    let stroke = ID2D1Factory::CreateStrokeStyle(
                        &self.d2d_factory,
                        &D2D1_STROKE_STYLE_PROPERTIES {
                            startCap: D2D1_CAP_STYLE_ROUND,
                            endCap: D2D1_CAP_STYLE_ROUND,
                            lineJoin: D2D1_LINE_JOIN_ROUND,
                            ..Default::default()
                        },
                        None,
                    )
                    .ok();
                    self.set_color(1.0, 1.0, 1.0, 0.98 * osd_a);
                    self.d2d_context.DrawLine(
                        D2D_POINT_2F { x: badge_cx - 5.2, y: cy + 0.4 },
                        D2D_POINT_2F { x: badge_cx - 1.4, y: cy + 4.4 },
                        &self.solid_brush,
                        2.3,
                        stroke.as_ref(),
                    );
                    self.d2d_context.DrawLine(
                        D2D_POINT_2F { x: badge_cx - 1.4, y: cy + 4.4 },
                        D2D_POINT_2F { x: badge_cx + 5.8, y: cy - 4.0 },
                        &self.solid_brush,
                        2.3,
                        stroke.as_ref(),
                    );

                    let txt_l = badge_l + badge_s + 11.0;
                    let txt_r = (body_right - 14.0).max(txt_l + 40.0);
                    let sub = format!("Now running Eave v{}", crate::updater::CURRENT_VERSION);
                    self.draw_text(
                        "Update installed",
                        &self.fmt_osd_label,
                        &D2D_RECT_F {
                            left: txt_l,
                            top: cy - 17.0,
                            right: txt_r,
                            bottom: cy + 1.0,
                        },
                        (0.98, 0.98, 0.99, osd_a),
                    );
                    self.draw_text(
                        &sub,
                        &self.fmt_artist,
                        &D2D_RECT_F {
                            left: txt_l,
                            top: cy + 1.0,
                            right: txt_r,
                            bottom: cy + 17.0,
                        },
                        (0.62, 0.66, 0.70, osd_a),
                    );
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn draw_interactive_alert_osd(
        &self,
        body_left: f32,
        body_right: f32,
        idle_h: f32,
        phase_secs: f32,
        osd_a: f32,
        progress: f32,
        alert_kind: u8,
        title: &str,
        subtitle: &str,
        cta_label: &str,
        grad_a: (f32, f32, f32),
        grad_b: (f32, f32, f32),
    ) {
        let cy = (idle_h - 3.0) * 0.5;
        let badge_cx = body_left + 32.0;
        let inner_r = 15.8;

        self.draw_radial_glow(badge_cx, cy, 26.0, grad_a, 0.24 * osd_a);
        for offset in [0.0_f32, 0.5_f32] {
            let p = (phase_secs * 0.85 + offset).rem_euclid(1.0);
            let ease = 1.0 - (1.0 - p).powi(3);
            let ripple_r = inner_r + 11.5 * ease;
            let ripple_a = 0.64 * (1.0 - p).powi(2) * osd_a;
            if ripple_a > 0.005 {
                self.set_color(grad_b.0, grad_b.1, grad_b.2, ripple_a);
                self.d2d_context.DrawEllipse(
                    &D2D1_ELLIPSE {
                        point: D2D_POINT_2F { x: badge_cx, y: cy },
                        radiusX: ripple_r,
                        radiusY: ripple_r,
                    },
                    &self.solid_brush,
                    1.45,
                    None,
                );
            }
        }
        self.set_color(grad_a.0, grad_a.1, grad_a.2, 0.16 * osd_a);
        self.d2d_context.FillEllipse(
            &D2D1_ELLIPSE {
                point: D2D_POINT_2F { x: badge_cx, y: cy },
                radiusX: inner_r,
                radiusY: inner_r,
            },
            &self.solid_brush,
        );
        self.set_color(grad_b.0, grad_b.1, grad_b.2, 0.68 * osd_a);
        self.d2d_context.DrawEllipse(
            &D2D1_ELLIPSE {
                point: D2D_POINT_2F { x: badge_cx, y: cy },
                radiusX: inner_r,
                radiusY: inner_r,
            },
            &self.solid_brush,
            1.6,
            None,
        );

        self.set_color(1.0, 1.0, 1.0, 0.98 * osd_a);
        if alert_kind == 0 {
            self.d2d_context.DrawEllipse(
                &D2D1_ELLIPSE {
                    point: D2D_POINT_2F { x: badge_cx, y: cy + 1.4 },
                    radiusX: 7.2,
                    radiusY: 7.2,
                },
                &self.solid_brush,
                1.8,
                None,
            );
            self.d2d_context.DrawLine(
                D2D_POINT_2F { x: badge_cx - 2.8, y: cy - 8.4 },
                D2D_POINT_2F { x: badge_cx + 2.8, y: cy - 8.4 },
                &self.solid_brush,
                1.85,
                None,
            );
            self.d2d_context.DrawLine(
                D2D_POINT_2F { x: badge_cx, y: cy - 8.4 },
                D2D_POINT_2F { x: badge_cx, y: cy - 5.8 },
                &self.solid_brush,
                1.65,
                None,
            );
            self.d2d_context.DrawLine(
                D2D_POINT_2F { x: badge_cx + 5.3, y: cy - 4.8 },
                D2D_POINT_2F { x: badge_cx + 7.0, y: cy - 6.5 },
                &self.solid_brush,
                1.65,
                None,
            );
            self.d2d_context.DrawLine(
                D2D_POINT_2F { x: badge_cx, y: cy + 1.4 },
                D2D_POINT_2F { x: badge_cx, y: cy - 2.6 },
                &self.solid_brush,
                1.65,
                None,
            );
            self.d2d_context.DrawLine(
                D2D_POINT_2F { x: badge_cx, y: cy + 1.4 },
                D2D_POINT_2F { x: badge_cx + 3.4, y: cy + 3.2 },
                &self.solid_brush,
                1.65,
                None,
            );
        } else if alert_kind == 1 {
            icons::draw_water_drop_stroked(
                &self.d2d_context,
                badge_cx,
                cy,
                18.5,
                &self.solid_brush,
                1.85,
            );
        } else if alert_kind == 2 {
            icons::draw_calendar_badge_icon(
                &self.d2d_context,
                badge_cx,
                cy,
                16.5,
                &self.solid_brush,
                1.5,
            );
        } else {
            // Crisp download arrow + tray bar for UpdateAvailable
            let stroke = ID2D1Factory::CreateStrokeStyle(
                &self.d2d_factory,
                &D2D1_STROKE_STYLE_PROPERTIES {
                    startCap: D2D1_CAP_STYLE_ROUND,
                    endCap: D2D1_CAP_STYLE_ROUND,
                    lineJoin: D2D1_LINE_JOIN_ROUND,
                    ..Default::default()
                },
                None,
            )
            .ok();
            self.d2d_context.DrawLine(
                D2D_POINT_2F { x: badge_cx, y: cy - 6.8 },
                D2D_POINT_2F { x: badge_cx, y: cy + 2.8 },
                &self.solid_brush,
                2.0,
                stroke.as_ref(),
            );
            self.d2d_context.DrawLine(
                D2D_POINT_2F { x: badge_cx - 4.2, y: cy - 1.2 },
                D2D_POINT_2F { x: badge_cx, y: cy + 3.0 },
                &self.solid_brush,
                2.0,
                stroke.as_ref(),
            );
            self.d2d_context.DrawLine(
                D2D_POINT_2F { x: badge_cx + 4.2, y: cy - 1.2 },
                D2D_POINT_2F { x: badge_cx, y: cy + 3.0 },
                &self.solid_brush,
                2.0,
                stroke.as_ref(),
            );
            self.d2d_context.DrawLine(
                D2D_POINT_2F { x: badge_cx - 5.4, y: cy + 6.4 },
                D2D_POINT_2F { x: badge_cx + 5.4, y: cy + 6.4 },
                &self.solid_brush,
                2.0,
                stroke.as_ref(),
            );
        }

        let (dismiss_rect, cta_rect) = interactive_osd_button_rects_at(body_right, idle_h);
        let d_cx = (dismiss_rect.left + dismiss_rect.right) * 0.5;
        let d_cy = (dismiss_rect.top + dismiss_rect.bottom) * 0.5;
        let d_r = (dismiss_rect.right - dismiss_rect.left) * 0.5;
        self.set_color(1.0, 1.0, 1.0, 0.16 * osd_a);
        self.d2d_context.FillEllipse(
            &D2D1_ELLIPSE {
                point: D2D_POINT_2F { x: d_cx, y: d_cy },
                radiusX: d_r,
                radiusY: d_r,
            },
            &self.solid_brush,
        );
        self.set_color(0.95, 0.96, 0.98, 0.94 * osd_a);
        self.d2d_context.DrawLine(
            D2D_POINT_2F { x: d_cx - 4.2, y: d_cy - 4.2 },
            D2D_POINT_2F { x: d_cx + 4.2, y: d_cy + 4.2 },
            &self.solid_brush,
            1.7,
            None,
        );
        self.d2d_context.DrawLine(
            D2D_POINT_2F { x: d_cx + 4.2, y: d_cy - 4.2 },
            D2D_POINT_2F { x: d_cx - 4.2, y: d_cy + 4.2 },
            &self.solid_brush,
            1.7,
            None,
        );

        let cta_r_rad = (cta_rect.bottom - cta_rect.top) * 0.5;
        let stops = [
            D2D1_GRADIENT_STOP {
                position: 0.0,
                color: D2D1_COLOR_F {
                    r: grad_a.0,
                    g: grad_a.1,
                    b: grad_a.2,
                    a: 0.98 * osd_a,
                },
            },
            D2D1_GRADIENT_STOP {
                position: 1.0,
                color: D2D1_COLOR_F {
                    r: grad_b.0,
                    g: grad_b.1,
                    b: grad_b.2,
                    a: 0.98 * osd_a,
                },
            },
        ];
        if let Ok(sc) = ID2D1RenderTarget::CreateGradientStopCollection(
            &self.d2d_context,
            &stops,
            D2D1_GAMMA_2_2,
            D2D1_EXTEND_MODE_CLAMP,
        ) {
            if let Ok(gb) = self.d2d_context.CreateLinearGradientBrush(
                &D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                    startPoint: D2D_POINT_2F { x: cta_rect.left, y: cta_rect.top },
                    endPoint: D2D_POINT_2F { x: cta_rect.right, y: cta_rect.bottom },
                },
                None,
                &sc,
            ) {
                self.d2d_context.FillRoundedRectangle(
                    &D2D1_ROUNDED_RECT {
                        rect: cta_rect,
                        radiusX: cta_r_rad,
                        radiusY: cta_r_rad,
                    },
                    &gb,
                );
            }
        }
        self.draw_text(
            cta_label,
            &self.fmt_pill_btn,
            &cta_rect,
            (1.0, 1.0, 1.0, 0.99 * osd_a),
        );

        let txt_l = body_left + 60.0;
        let txt_r = (dismiss_rect.left - 8.0).max(txt_l + 40.0);
        self.draw_text(
            title,
            &self.fmt_title,
            &D2D_RECT_F {
                left: txt_l,
                top: cy - 19.0,
                right: txt_r,
                bottom: cy + 1.0,
            },
            (0.98, 0.98, 1.0, osd_a),
        );
        self.draw_text(
            subtitle,
            &self.fmt_artist,
            &D2D_RECT_F {
                left: txt_l,
                top: cy + 1.5,
                right: txt_r,
                bottom: cy + 19.0,
            },
            (0.66, 0.68, 0.74, osd_a),
        );

        let bar_h = 3.0;
        let bar_l = body_left;
        let bar_r = body_right;
        let bar_top = idle_h - bar_h;
        self.set_color(1.0, 1.0, 1.0, 0.10 * osd_a);
        self.d2d_context.FillRectangle(
            &D2D_RECT_F {
                left: bar_l,
                top: bar_top,
                right: bar_r,
                bottom: idle_h,
            },
            &self.solid_brush,
        );
        let fill_w = (bar_r - bar_l) * progress.clamp(0.0, 1.0);
        if fill_w > 1.0 {
            let fill_rect = D2D_RECT_F {
                left: bar_l,
                top: bar_top,
                right: bar_l + fill_w,
                bottom: idle_h,
            };
            if let Ok(sc) = ID2D1RenderTarget::CreateGradientStopCollection(
                &self.d2d_context,
                &stops,
                D2D1_GAMMA_2_2,
                D2D1_EXTEND_MODE_CLAMP,
            ) {
                if let Ok(gb) = self.d2d_context.CreateLinearGradientBrush(
                    &D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                        startPoint: D2D_POINT_2F { x: bar_l, y: bar_top },
                        endPoint: D2D_POINT_2F { x: bar_r, y: bar_top },
                    },
                    None,
                    &sc,
                ) {
                    self.d2d_context.FillRectangle(&fill_rect, &gb);
                }
            }
            self.draw_radial_glow(bar_l + fill_w, idle_h - 1.5, 11.0, grad_b, 0.50 * osd_a);
        }
    }

    unsafe fn header_clock_x0_for_page(
        &self,
        page: NotchPage,
        layout: &ExpandedLayout,
        canvas_cx: f32,
        total_w: f32,
        bell_reserved_x: f32,
    ) -> f32 {
        let title_end_x = match page {
            NotchPage::Music => layout.body_left + 12.0,
            NotchPage::Pomodoro => {
                layout.page_title_rect.left + self.measure_width("Focus", &self.fmt_page_title)
            }
            NotchPage::Water => {
                layout.page_title_rect.left + self.measure_width("Water", &self.fmt_page_title)
            }
            NotchPage::Shelf => {
                layout.page_title_rect.left + self.measure_width("Shelf", &self.fmt_page_title)
            }
            NotchPage::Todo => {
                layout.page_title_rect.left
                    + self.measure_width("Todo", &self.fmt_page_title)
                    + self.measure_width("List", &self.fmt_page_title)
            }
            NotchPage::Widgets => {
                layout.page_title_rect.left + self.measure_width("Widgets", &self.fmt_page_title)
            }
        };
        let right_limit = if page == NotchPage::Shelf {
            layout.body_right - 101.0
        } else {
            bell_reserved_x
        };
        let max_x0 = (right_limit - total_w).max(title_end_x + 8.0);
        (canvas_cx - total_w * 0.5)
            .max(title_end_x + 12.0)
            .min(max_x0)
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn draw_shared_clock_header(
        &self,
        cfg: &Config,
        springs: &NotchSprings,
        state: &EaveViewState<'_>,
        canvas_cx: f32,
        _pill_left: f32,
        _cur_w: f32,
        _ear_r: f32,
        expand: f32,
        non_ai_a: f32,
    ) {
        let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
        let lyr_t_global = springs.lyrics_mode.value().clamp(0.0, 1.0);
        let lyr_t_expanded = if state.page == NotchPage::Music {
            lyr_t_global
        } else {
            0.0
        };
        let osd_t = springs.osd_open().clamp(0.0, 1.0);
        let eave_t = springs.mascot_idle_presence.value().clamp(0.0, 1.0);
        let ai_fade = non_ai_a.clamp(0.0, 1.0);
        let collapsed_vis = (1.0 - lyr_t_global * 1.35).clamp(0.0, 1.0)
            * (1.0 - osd_t * 1.45).clamp(0.0, 1.0)
            * (1.0 - eave_t * 1.35).clamp(0.0, 1.0)
            * ai_fade;
        let expanded_vis = (1.0 - lyr_t_expanded * 1.35).clamp(0.0, 1.0) * ai_fade;

        let base_body_w = (cfg.expanded_width - 44.0).max(160.0);
        let body_w = base_body_w + (520.0 - base_body_w) * lyr_t_expanded;
        let layout = ExpandedLayout::compute(canvas_cx, body_w, cfg.expanded_height);
        let collapsed_cy = (cfg.notch_height.max(24.0) + 4.5 * springs.hover.value()) * 0.5;
        let expanded_cy = (layout.header_clock_rect.top + layout.header_clock_rect.bottom) * 0.5;
        let cy = lerp(collapsed_cy, expanded_cy, expand);
        let page_t = springs.page_transition.value().clamp(0.0, 1.0);

        let pomo_run_t = springs.pomo_running_mix.value().clamp(0.0, 1.0);
        if pomo_run_t > 0.01 {
            let audio_vis_mix = springs.audio_vis_mix.value().clamp(0.0, 1.0);
            let vis_block = (1.0 - audio_vis_mix).clamp(0.0, 1.0);
            let ring_collapsed_a = collapsed_vis * vis_block;
            let shelf_curr = if state.page == NotchPage::Shelf { 0.0 } else { 1.0 };
            let shelf_prev = if state.prev_page == NotchPage::Shelf { 0.0 } else { 1.0 };
            let non_shelf_vis = if state.prev_page != state.page && page_t < 0.999 {
                lerp(shelf_prev, shelf_curr, page_t)
            } else {
                shelf_curr
            };
            let ring_expanded_a = expanded_vis * non_shelf_vis;
            let ring_alpha = lerp(ring_collapsed_a, ring_expanded_a, expand) * pomo_run_t;
            if ring_alpha > 0.01 {
                let collapsed_right_rx = canvas_cx + cfg.notch_width * 0.5 - 26.0;
                let expanded_right_rx = layout.body_right - 31.0;
                let rx = lerp(collapsed_right_rx, expanded_right_rx, expand);
                let ry = cy;
                let rr = lerp(6.2, 6.8, expand);
                let is_focus = state.pomodoro.phase == PomodoroPhase::Focus;
                let is_long_break = !is_focus && state.pomodoro.current_round >= 4;
                self.draw_pomodoro_ring(
                    rx,
                    ry,
                    rr,
                    is_focus,
                    is_long_break,
                    springs.phase_secs(),
                    ring_alpha,
                );
            }
        }

        let clock_cfg_a = springs.clock_visible.value().clamp(0.0, 1.0);
        if clock_cfg_a <= 0.01 {
            return;
        }
        let clock_collapsed_vis = if cfg.minimal_mode { 0.0 } else { collapsed_vis };
        let time_alpha = lerp(clock_collapsed_vis, expanded_vis, expand) * clock_cfg_a;
        if time_alpha <= 0.01 {
            return;
        }

        let (date_part, time_num, ampm_part, w_date, w_time, w_ampm) =
            if let Some(cached) = &self.cached_hdr_dt {
                (
                    cached.0.as_str(),
                    cached.1.as_str(),
                    cached.2.as_str(),
                    cached.3,
                    cached.4,
                    cached.5,
                )
            } else {
                return;
            };
        let time_total_w = w_time + w_ampm;
        let full_total_w = w_date + time_total_w;

        let bell_vis = springs.bell_visible.value().clamp(0.0, 1.0);
        let right_icon_vis = bell_vis.max(pomo_run_t);
        let bell_reserved_x = lerp(layout.body_right - 16.0, layout.bell_center.x - 16.0, right_icon_vis);

        let x0_curr = self.header_clock_x0_for_page(
            state.page,
            &layout,
            canvas_cx,
            full_total_w,
            bell_reserved_x,
        );
        let expanded_x0 = if state.prev_page != state.page && page_t < 0.999 {
            let x0_prev = self.header_clock_x0_for_page(
                state.prev_page,
                &layout,
                canvas_cx,
                full_total_w,
                bell_reserved_x,
            );
            lerp(x0_prev, x0_curr, page_t)
        } else {
            x0_curr
        };

        let expanded_time_cx = expanded_x0 + w_date + time_total_w * 0.5;
        let time_cx = lerp(canvas_cx, expanded_time_cx, expand);
        let time_scale = lerp(1.16, 1.0, expand);

        let top = cy - 10.0;
        let bottom = cy + 10.0;

        let scaled_time_left = time_cx - (time_total_w * 0.5) * time_scale;
        let date_x = scaled_time_left - w_date;
        let left_ring_vis = springs.left_ring_visible.value().clamp(0.0, 1.0);
        let ring_rx = if left_ring_vis > 0.01 {
            let collapsed_rx = canvas_cx - cfg.notch_width * 0.5 + 26.0;
            let collapsed_gap = ((canvas_cx - (time_total_w * 0.5) * 1.16) - collapsed_rx).max(16.0);
            Some(scaled_time_left - lerp(collapsed_gap, w_date + 13.5, expand))
        } else {
            None
        };

        let date_a = ((expand - 0.22) / 0.78).clamp(0.0, 1.0) * expanded_vis;
        if date_a > 0.01 {
            let clip_date = ring_rx.filter(|_| expand < 0.99);
            if let Some(rx) = clip_date {
                self.d2d_context.PushAxisAlignedClip(
                    &D2D_RECT_F {
                        left: (rx + 10.5).min(scaled_time_left),
                        top: top - 2.0,
                        right: scaled_time_left + 4.0,
                        bottom: bottom + 2.0,
                    },
                    D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                );
            }
            self.draw_text(
                &date_part,
                &self.fmt_header_date,
                &D2D_RECT_F {
                    left: date_x,
                    top,
                    right: date_x + w_date + 4.0,
                    bottom,
                },
                (0.86, 0.86, 0.86, 1.0 * date_a),
            );
            if clip_date.is_some() {
                self.d2d_context.PopAxisAlignedClip();
            }
        }

        if let Some(rx) = ring_rx {
            let art_mix = springs.collapsed_art_mix.value().clamp(0.0, 1.0);
            let ring_col_a = clock_collapsed_vis * (1.0 - art_mix);
            let ring_a = lerp(ring_col_a, expanded_vis, expand) * left_ring_vis;
            if ring_a > 0.01 {
                let rr = lerp(7.8, 7.2, expand);
                let sw = springs.left_ring_switch.value().clamp(0.0, 1.0);
                if sw < 0.99 && springs.left_ring_prev_mode != 0 {
                    let prev_a = ring_a * (1.0 - sw);
                    if prev_a > 0.01 {
                        self.draw_collapsed_left_ring(
                            springs.left_ring_prev_mode,
                            springs,
                            state,
                            rx,
                            cy,
                            rr,
                            prev_a,
                        );
                    }
                }
                if springs.left_ring_mode != 0 {
                    let curr_a = ring_a * sw;
                    if curr_a > 0.01 {
                        self.draw_collapsed_left_ring(
                            springs.left_ring_mode,
                            springs,
                            state,
                            rx,
                            cy,
                            rr,
                            curr_a,
                        );
                    }
                }
            }
        }

        if let Some(clock_bmp) = &self.clock_2x_bmp {
            let draw_w = self.cached_clock_cell_w * time_scale;
            let draw_h = 22.0 * time_scale;
            let dst_rect = D2D_RECT_F {
                left: time_cx - draw_w * 0.5,
                top: cy - draw_h * 0.5,
                right: time_cx + draw_w * 0.5,
                bottom: cy + draw_h * 0.5,
            };
            self.d2d_context.DrawBitmap(
                clock_bmp,
                Some(&dst_rect),
                time_alpha,
                D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
                None,
                None,
            );
        } else {
            let time_x = time_cx - time_total_w * 0.5;
            self.draw_text(
                &time_num,
                &self.fmt_header_time,
                &D2D_RECT_F {
                    left: time_x,
                    top,
                    right: time_x + w_time + 4.0,
                    bottom,
                },
                (1.0, 1.0, 1.0, 1.0 * time_alpha),
            );
            self.draw_text(
                &ampm_part,
                &self.fmt_header_ampm,
                &D2D_RECT_F {
                    left: time_x + w_time,
                    top: top + 0.8,
                    right: time_x + time_total_w + 6.0,
                    bottom,
                },
                (1.0, 1.0, 1.0, 1.0 * time_alpha),
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn draw_expanded_notch(
        &self,
        cfg: &Config,
        springs: &NotchSprings,
        state: &EaveViewState<'_>,
        canvas_cx: f32,
        alpha: f32,
        notch_geo: &ID2D1PathGeometry,
        cleartype_active: bool,
    ) {
        let lyr_t = if state.page == NotchPage::Music {
            springs.lyrics_mode.value().clamp(0.0, 1.0)
        } else {
            0.0
        };
        let base_body_w = (cfg.expanded_width - 44.0).max(160.0);
        let body_w = base_body_w + (520.0 - base_body_w) * lyr_t;
        let base_h = cfg.expanded_height;
        let layout = ExpandedLayout::compute(canvas_cx, body_w, base_h);
        let chrome_fade = (1.0 - lyr_t * 1.35).clamp(0.0, 1.0);

        let page_t = springs.page_transition.value().clamp(0.0, 1.0);
        let transitioning = page_t < 0.995 && state.prev_page != state.page;
        let out_alpha = alpha * (1.0 - page_t).powi(2);
        let in_alpha = if transitioning {
            alpha * page_t.powf(0.72).clamp(0.0, 1.0)
        } else {
            alpha
        };
        let out_dx = -page_t * 44.0 * springs.page_dir;
        let in_dx = if transitioning {
            (1.0 - page_t) * 44.0 * springs.page_dir
        } else {
            0.0
        };

        if transitioning && out_alpha > 0.01 {
            let _ = self.draw_page_header(
                state.prev_page,
                springs,
                state,
                &layout,
                out_dx * 0.45,
                out_alpha,
            );
        }
        let _ = self.draw_page_header(
            state.page,
            springs,
            state,
            &layout,
            in_dx * 0.45,
            in_alpha,
        );

        let bell_vis = springs.bell_visible.value().clamp(0.0, 1.15);
        let pomo_ring_active = springs.pomo_running_mix.value() > 0.5 && state.page != NotchPage::Shelf;

        if bell_vis * chrome_fade > 0.01 && !pomo_ring_active {
            let bell_h = springs.slot_hover(0);
            let bell_scale =
                (0.65 + 0.35 * bell_vis.min(1.0)) * springs.slot_scale(0, 0.08, 0.15);
            let bell_alpha = bell_vis.clamp(0.0, 1.0) * alpha * chrome_fade;
            self.draw_ss_icon(
                ATLAS_SLOT_BELL,
                layout.bell_center.x,
                layout.bell_center.y,
                16.0 * bell_scale,
                (0.90 + 0.10 * bell_h) * bell_alpha,
            );
            if state.has_notification {
                let dot_r = 2.4 * (0.65 + 0.35 * bell_vis.min(1.0));
                self.set_color(0.945, 0.404, 0.533, 0.98 * bell_alpha);
                self.d2d_context.FillEllipse(
                    &D2D1_ELLIPSE {
                        point: D2D_POINT_2F {
                            x: layout.bell_center.x + 5.2 * bell_scale,
                            y: layout.bell_center.y - 5.2 * bell_scale,
                        },
                        radiusX: dot_r,
                        radiusY: dot_r,
                    },
                    &self.solid_brush,
                );
            }
        }

        if transitioning && out_alpha > 0.01 {
            self.draw_single_page(
                state.prev_page,
                cfg,
                springs,
                state,
                &layout,
                out_dx,
                out_alpha,
                notch_geo,
                cleartype_active,
            );
        }
        if in_alpha > 0.01 {
            self.draw_single_page(
                state.page,
                cfg,
                springs,
                state,
                &layout,
                in_dx,
                in_alpha,
                notch_geo,
                cleartype_active,
            );
        }

        if chrome_fade > 0.01 {
            self.draw_side_chevrons(springs, &layout, alpha * chrome_fade);
        }

        if chrome_fade > 0.01 {
            self.draw_bottom_dock(cfg, springs, &layout, alpha * chrome_fade);
        }
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn draw_page_header(
        &self,
        page: NotchPage,
        springs: &NotchSprings,
        _state: &EaveViewState<'_>,
        layout: &ExpandedLayout,
        dx: f32,
        alpha: f32,
    ) -> f32 {
        let mut title_end_x = layout.body_left + 12.0;
        let mut title_rect = layout.page_title_rect;
        title_rect.left += dx;
        title_rect.right += dx;

        match page {
            NotchPage::Music => {}
            NotchPage::Pomodoro => {
                let lbl = "Focus";
                self.draw_text(
                    lbl,
                    &self.fmt_page_title,
                    &title_rect,
                    (0.957, 0.957, 0.965, 1.0 * alpha),
                );
                title_end_x = title_rect.left + self.measure_width(lbl, &self.fmt_page_title);
            }
            NotchPage::Water => {
                let lbl = "Water";
                self.draw_text(
                    lbl,
                    &self.fmt_page_title,
                    &title_rect,
                    (0.957, 0.957, 0.965, 1.0 * alpha),
                );
                title_end_x = title_rect.left + self.measure_width(lbl, &self.fmt_page_title);
            }
            NotchPage::Shelf => {
                let lbl = "Shelf";
                self.draw_text(
                    lbl,
                    &self.fmt_page_title,
                    &title_rect,
                    (0.957, 0.957, 0.965, 1.0 * alpha),
                );
                title_end_x = title_rect.left + self.measure_width(lbl, &self.fmt_page_title);
                self.draw_shelf_switcher(springs, layout, alpha);
            }
            NotchPage::Todo => {
                let w_todo = self.measure_width("Todo", &self.fmt_page_title);
                let w_list = self.measure_width("List", &self.fmt_page_title);
                self.draw_text(
                    "Todo",
                    &self.fmt_page_title,
                    &title_rect,
                    (0.957, 0.957, 0.965, 1.0 * alpha),
                );
                self.draw_text(
                    "List",
                    &self.fmt_page_title,
                    &D2D_RECT_F {
                        left: title_rect.left + w_todo,
                        top: title_rect.top,
                        right: title_rect.right + 16.0,
                        bottom: title_rect.bottom,
                    },
                    (0.24, 0.88, 0.46, 1.0 * alpha),
                );
                title_end_x = title_rect.left + w_todo + w_list;
            }
            NotchPage::Widgets => {
                self.draw_text(
                    "Widgets",
                    &self.fmt_page_title,
                    &title_rect,
                    (0.957, 0.957, 0.965, 1.0 * alpha),
                );
                title_end_x = title_rect.left + self.measure_width("Widgets", &self.fmt_page_title);
            }
        }
        title_end_x
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn draw_single_page(
        &self,
        page: NotchPage,
        cfg: &Config,
        springs: &NotchSprings,
        state: &EaveViewState<'_>,
        layout: &ExpandedLayout,
        dx: f32,
        alpha: f32,
        notch_geo: &ID2D1PathGeometry,
        cleartype_active: bool,
    ) {
        match page {
            NotchPage::Music => {
                self.draw_music_page(cfg, springs, state, layout, dx, alpha, cleartype_active);
            }
            NotchPage::Pomodoro => {
                self.draw_pomodoro_page(springs, state.pomodoro, layout, dx, alpha);
            }
            NotchPage::Water => {
                self.draw_water_page(springs, state.water, layout, dx, alpha);
            }
            NotchPage::Shelf => {
                self.draw_shelf_page(
                    state.shelf,
                    springs,
                    layout,
                    dx,
                    springs.drag_over.value(),
                    alpha,
                    Some(notch_geo),
                );
            }
            NotchPage::Todo => {
                self.draw_todo_page(springs, state.todo, layout, dx, alpha);
            }
            NotchPage::Widgets => {
                self.draw_widgets_page(cfg, springs, state, layout, dx, alpha);
            }
        }
    }

    unsafe fn draw_side_chevrons(
        &self,
        springs: &NotchSprings,
        layout: &ExpandedLayout,
        alpha: f32,
    ) {
        let brush: &ID2D1Brush = &self.solid_brush;

        let draw_mag_chevron = |base: D2D_POINT_2F,
                                mag: crate::physics::MagneticArrowState,
                                slot: usize,
                                is_left: bool| {
            let hov = springs.slot_hover(slot);
            let press_scale = springs.slot_scale(slot, 0.0, 0.10);
            let h = (mag.s * 1.4).min(1.0).max(hov);
            let center = D2D_POINT_2F {
                x: base.x + mag.x,
                y: base.y + mag.y,
            };
            if h > 0.01 {
                let halo_r = 11.5 * (0.82 + 0.18 * h) * press_scale;
                let ell = D2D1_ELLIPSE {
                    point: center,
                    radiusX: halo_r,
                    radiusY: halo_r,
                };
                self.set_color(
                    0.145 + 0.055 * hov,
                    0.145 + 0.055 * hov,
                    0.160 + 0.060 * hov,
                    0.94 * h * alpha,
                );
                self.d2d_context.FillEllipse(&ell, &self.solid_brush);
                self.set_color(1.0, 1.0, 1.0, (0.16 + 0.14 * hov) * h * alpha);
                self.d2d_context
                    .DrawEllipse(&ell, &self.solid_brush, 0.8, None);
            }
            let fg_v = 0.62 + (0.957 - 0.62) * h;
            self.set_color(fg_v, fg_v, fg_v + 0.015, (0.44 + 0.54 * h) * alpha);
            let icon_s = (9.8 + 1.0 * h) * press_scale;
            let nudge = if is_left { -0.5 * hov } else { 0.5 * hov };
            if is_left {
                icons::draw_chevron_left_stroked(
                    &self.d2d_context,
                    center.x + nudge,
                    center.y,
                    icon_s,
                    brush,
                    1.55,
                );
            } else {
                icons::draw_chevron_right_stroked(
                    &self.d2d_context,
                    center.x + nudge,
                    center.y,
                    icon_s,
                    brush,
                    1.55,
                );
            }
        };

        draw_mag_chevron(layout.chevron_left_center, springs.mag_left, 1, true);
        draw_mag_chevron(layout.chevron_right_center, springs.mag_right, 2, false);
    }

    unsafe fn draw_shelf_switcher(
        &self,
        springs: &NotchSprings,
        layout: &ExpandedLayout,
        alpha: f32,
    ) {
        let brush: &ID2D1Brush = &self.solid_brush;
        let (pill, files_rect, clip_rect) = shelf_switcher_rects(layout.body_right);
        let cb_t = springs.shelf_tab_clipboard.value().clamp(0.0, 1.0);
        let h_files = springs.slot_hover(17);
        let h_clip = springs.slot_hover(18);

        let pill_rrect = D2D1_ROUNDED_RECT {
            rect: pill,
            radiusX: 10.0,
            radiusY: 10.0,
        };
        let hov_any = h_files.max(h_clip);
        self.set_color(
            0.110 + 0.025 * hov_any,
            0.110 + 0.025 * hov_any,
            0.118 + 0.025 * hov_any,
            0.96 * alpha,
        );
        self.d2d_context
            .FillRoundedRectangle(&pill_rrect, &self.solid_brush);
        self.set_color(1.0, 1.0, 1.0, (0.08 + 0.04 * hov_any) * alpha);
        self.d2d_context
            .DrawRoundedRectangle(&pill_rrect, &self.solid_brush, 0.85, None);

        let thumb_x = (pill.left + 2.0) + 31.0 * cb_t;
        let thumb_rect = D2D_RECT_F {
            left: thumb_x,
            top: pill.top + 2.0,
            right: thumb_x + 29.0,
            bottom: pill.bottom - 2.0,
        };
        let thumb_rrect = D2D1_ROUNDED_RECT {
            rect: thumb_rect,
            radiusX: 8.0,
            radiusY: 8.0,
        };
        self.set_color(0.220, 0.220, 0.235, 0.98 * alpha);
        self.d2d_context
            .FillRoundedRectangle(&thumb_rrect, &self.solid_brush);
        self.set_color(1.0, 1.0, 1.0, 0.12 * alpha);
        self.d2d_context
            .DrawRoundedRectangle(&thumb_rrect, &self.solid_brush, 0.75, None);

        let fx = (files_rect.left + files_rect.right) * 0.5;
        let fy = (files_rect.top + files_rect.bottom) * 0.5;
        let f_alpha = (0.56 + 0.42 * (1.0 - cb_t).max(h_files * 0.7)) * alpha;
        self.set_color(0.957, 0.957, 0.965, f_alpha);
        icons::draw_folder_outline(&self.d2d_context, fx, fy, 12.6, brush, 1.4);

        let cx = (clip_rect.left + clip_rect.right) * 0.5;
        let cy = (clip_rect.top + clip_rect.bottom) * 0.5;
        let c_alpha = (0.56 + 0.42 * cb_t.max(h_clip * 0.7)) * alpha;
        self.set_color(0.957, 0.957, 0.965, c_alpha);
        self.d2d_context.DrawRoundedRectangle(
            &D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F {
                    left: cx - 4.1,
                    top: cy - 3.9,
                    right: cx + 4.1,
                    bottom: cy + 5.0,
                },
                radiusX: 1.4,
                radiusY: 1.4,
            },
            &self.solid_brush,
            1.35,
            None,
        );
        self.d2d_context.FillRoundedRectangle(
            &D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F {
                    left: cx - 2.1,
                    top: cy - 5.5,
                    right: cx + 2.1,
                    bottom: cy - 3.4,
                },
                radiusX: 0.7,
                radiusY: 0.7,
            },
            &self.solid_brush,
        );
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn draw_music_page(
        &self,
        _cfg: &Config,
        springs: &NotchSprings,
        state: &EaveViewState<'_>,
        layout: &ExpandedLayout,
        dx: f32,
        alpha: f32,
        cleartype_active: bool,
    ) {
        let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
        let media = &state.media;
        let lyr_t = springs.lyrics_mode.value().clamp(0.0, 1.0);
        let chrome_fade = (1.0 - lyr_t * 1.35).clamp(0.0, 1.0);
        let bl = layout.body_left + dx;
        let br = layout.body_right + dx;

        let art_left = bl + lerp(31.0, 22.0, lyr_t);
        let art_rect = D2D_RECT_F {
            left: art_left,
            top: 24.0,
            right: art_left + 62.0,
            bottom: 86.0,
        };

        if media.has_session && self.cached_art_bitmap.is_some() {
            self.draw_rounded_art(&art_rect, 14.0, alpha, springs.album_art_fade.value());
        } else {
            let ph = D2D1_ROUNDED_RECT {
                rect: art_rect,
                radiusX: 14.0,
                radiusY: 14.0,
            };
            self.set_color(0.043, 0.043, 0.051, 0.98 * alpha);
            self.d2d_context.FillRoundedRectangle(&ph, &self.solid_brush);

            let mcx = (art_rect.left + art_rect.right) * 0.5;
            let mcy = (art_rect.top + art_rect.bottom) * 0.5;
            self.set_color(0.38, 0.40, 0.45, 0.94 * alpha);
            let round_st = ID2D1Factory::CreateStrokeStyle(
                &self.d2d_factory,
                &D2D1_STROKE_STYLE_PROPERTIES {
                    startCap: D2D1_CAP_STYLE_ROUND,
                    endCap: D2D1_CAP_STYLE_ROUND,
                    lineJoin: D2D1_LINE_JOIN_ROUND,
                    ..Default::default()
                },
                None,
            )
            .ok();
            if let Ok(beam_geo) = self.d2d_factory.CreatePathGeometry() {
                if let Ok(sink) = beam_geo.Open() {
                    sink.BeginFigure(
                        D2D_POINT_2F { x: mcx - 4.2, y: mcy - 10.6 },
                        D2D1_FIGURE_BEGIN_FILLED,
                    );
                    sink.AddLine(D2D_POINT_2F { x: mcx + 11.4, y: mcy - 13.8 });
                    sink.AddLine(D2D_POINT_2F { x: mcx + 11.4, y: mcy - 8.6 });
                    sink.AddLine(D2D_POINT_2F { x: mcx - 4.2, y: mcy - 5.4 });
                    sink.EndFigure(D2D1_FIGURE_END_CLOSED);
                    if sink.Close().is_ok() {
                        self.d2d_context
                            .FillGeometry(&beam_geo, &self.solid_brush, None);
                    }
                }
            }
            self.d2d_context.DrawLine(
                D2D_POINT_2F { x: mcx - 3.1, y: mcy - 9.4 },
                D2D_POINT_2F { x: mcx - 3.1, y: mcy + 9.2 },
                &self.solid_brush,
                2.5,
                round_st.as_ref(),
            );
            self.d2d_context.DrawLine(
                D2D_POINT_2F { x: mcx + 10.2, y: mcy - 12.2 },
                D2D_POINT_2F { x: mcx + 10.2, y: mcy + 6.0 },
                &self.solid_brush,
                2.5,
                round_st.as_ref(),
            );
            let prev_xf = {
                let mut m = Matrix3x2::identity();
                self.d2d_context.GetTransform(&mut m);
                m
            };
            for &(nx, ny) in &[(mcx - 6.8, mcy + 9.6), (mcx + 6.5, mcy + 6.4)] {
                let tilt = Matrix3x2::rotation(-16.0, nx, ny);
                self.d2d_context.SetTransform(&mul_matrix3x2(tilt, prev_xf));
                self.d2d_context.FillEllipse(
                    &D2D1_ELLIPSE {
                        point: D2D_POINT_2F { x: nx, y: ny },
                        radiusX: 5.0,
                        radiusY: 3.9,
                    },
                    &self.solid_brush,
                );
            }
            self.d2d_context.SetTransform(&prev_xf);
        }

        let text_left = bl + lerp(109.0, 100.0, lyr_t);
        let text_right = lerp(br - 31.0, bl + 242.0, lyr_t);
        let title_shift = lerp(3.0, 0.0, lyr_t);
        let title_rect = D2D_RECT_F {
            left: text_left,
            top: layout.title_rect.top + title_shift,
            right: text_right,
            bottom: layout.title_rect.bottom + title_shift,
        };
        let artist_shift = lerp(4.0, 0.0, lyr_t);
        let artist_rect = D2D_RECT_F {
            left: text_left,
            top: layout.artist_rect.top + artist_shift,
            right: text_right,
            bottom: layout.artist_rect.bottom + artist_shift + lerp(3.0, 0.0, lyr_t),
        };

        if !media.has_session {
            self.draw_text(
                "Not Playing",
                &self.fmt_title,
                &title_rect,
                (0.957, 0.957, 0.965, 1.0 * alpha),
            );
        } else {
            let title = if media.title.is_empty() {
                "Unknown Track"
            } else {
                media.title
            };
            self.draw_text(
                title,
                &self.fmt_title,
                &title_rect,
                (0.957, 0.957, 0.965, 1.0 * alpha),
            );
            let sep = if lyr_t > 0.5 { " - " } else { " • " };
            let subtitle = match (media.artist.is_empty(), media.album.is_empty()) {
                (false, false) => Some(format!("{}{sep}{}", media.artist, media.album)),
                (false, true) => Some(media.artist.to_string()),
                (true, false) => Some(media.album.to_string()),
                (true, true) => None,
            };
            if let Some(sub) = subtitle {
                let artist_fmt = if lyr_t < 0.5 {
                    &self.fmt_music_artist
                } else {
                    &self.fmt_artist
                };
                self.draw_text(
                    &sub,
                    artist_fmt,
                    &artist_rect,
                    (0.60, 0.60, 0.60, 1.0 * alpha),
                );
            }
        }

        let prog_cy = lerp(106.5, 107.0, lyr_t);
        let tl_left = bl + lerp(31.0, 24.0, lyr_t);
        let tl_rect = D2D_RECT_F {
            left: tl_left,
            top: prog_cy - 9.0,
            right: tl_left + 52.0,
            bottom: prog_cy + 9.0,
        };
        let tr_rect = D2D_RECT_F {
            left: lerp(br - 83.0, bl + 188.0, lyr_t),
            top: prog_cy - 9.0,
            right: lerp(br - 31.0, bl + 240.0, lyr_t),
            bottom: prog_cy + 9.0,
        };

        let ratio = if media.has_session && media.duration_secs > 0.1 {
            (media.position_secs / media.duration_secs).clamp(0.0, 1.0) as f32
        } else {
            0.0
        };
        let pos_str = if media.has_session {
            format_mm_ss(media.position_secs)
        } else {
            "00:00".to_string()
        };
        let rem_str = if media.has_session {
            format_remaining_mm_ss(media.duration_secs, media.position_secs)
        } else {
            "-00:00".to_string()
        };

        let time_col = if media.has_session {
            (0.58, 0.58, 0.60, 0.96 * alpha)
        } else {
            (0.42, 0.42, 0.44, 0.96 * alpha)
        };
        self.draw_text(&pos_str, &self.fmt_time_left, &tl_rect, time_col);
        self.draw_text(&rem_str, &self.fmt_time_right, &tr_rect, time_col);

        let bar_h = if media.has_session {
            springs.slot_hover(8)
        } else {
            0.0
        };
        let half_h = 2.0 + 0.8 * bar_h;
        let track_rect = D2D_RECT_F {
            left: bl + lerp(76.0, 69.0, lyr_t),
            top: prog_cy - half_h,
            right: lerp(br - 79.0, bl + 188.0, lyr_t),
            bottom: prog_cy + half_h,
        };
        let track_a = if media.has_session {
            0.24 + 0.08 * bar_h
        } else {
            0.12
        };
        self.set_color(1.0, 1.0, 1.0, track_a * alpha);
        self.d2d_context.FillRoundedRectangle(
            &D2D1_ROUNDED_RECT {
                rect: track_rect,
                radiusX: half_h,
                radiusY: half_h,
            },
            &self.solid_brush,
        );

        let bar_w = (track_rect.right - track_rect.left).max(0.0);
        let fill_w = bar_w * ratio;
        if fill_w > 1.0 {
            let fill_rect = D2D_RECT_F {
                left: track_rect.left,
                top: prog_cy - half_h,
                right: track_rect.left + fill_w,
                bottom: prog_cy + half_h,
            };
            self.set_color(0.96, 0.97, 0.98, 0.98 * alpha);
            self.d2d_context.FillRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: fill_rect,
                    radiusX: half_h,
                    radiusY: half_h,
                },
                &self.solid_brush,
            );
        }

        let ctrl_y = 136.5;
        let lyr_x = bl + lerp(61.5, 53.0, lyr_t);
        let lyr_h = if media.has_session {
            springs.slot_hover(6)
        } else {
            0.0
        };
        let lyr_scale = if media.has_session {
            springs.slot_scale(6, 0.09, 0.15)
        } else {
            1.0
        };
        let lyr_pop = 1.0 + 0.06 * (lyr_t * std::f32::consts::PI).sin();
        let lyr_size = 19.5 * lyr_scale * lyr_pop;
        let outline_a = (1.0 - lyr_t) * alpha;
        if outline_a > 0.01 {
            let lyr_alpha = if media.has_session {
                (0.72 + (1.0 - 0.72) * lyr_h) * outline_a
            } else {
                0.30 * outline_a
            };
            self.draw_ss_icon(ATLAS_SLOT_LYRICS_STROKED, lyr_x, ctrl_y, lyr_size, lyr_alpha);
        }
        let filled_a = lyr_t * alpha;
        if filled_a > 0.01 {
            self.draw_ss_icon(
                ATLAS_SLOT_LYRICS_FILLED,
                lyr_x,
                ctrl_y,
                lyr_size,
                (0.95 + 0.05 * lyr_h) * filled_a,
            );
        }

        let base_a = if media.has_session { 0.96 } else { 0.30 };

        let prev_x = lerp(layout.cx + dx - 48.0, bl + 91.0, lyr_t);
        let prev_h = if media.has_session {
            springs.slot_hover(3)
        } else {
            0.0
        };
        let prev_scale = if media.has_session {
            springs.slot_scale(3, 0.09, 0.15)
        } else {
            1.0
        };
        self.draw_ss_icon(
            ATLAS_SLOT_PREV,
            prev_x,
            ctrl_y,
            19.5 * prev_scale,
            (base_a + (1.0 - base_a) * prev_h) * alpha,
        );

        let play_x = lerp(layout.cx + dx, bl + 131.0, lyr_t);
        let play_h = if media.has_session {
            springs.slot_hover(4)
        } else {
            0.0
        };
        let play_scale = if media.has_session {
            springs.slot_scale(4, 0.09, 0.15)
        } else {
            1.0
        };
        let play_t = springs.play_mix().clamp(0.0, 1.0);
        let play_pulse = 1.0 + 0.08 * (play_t * std::f32::consts::PI).sin();
        let base_play_size = 20.5 * play_scale * play_pulse;
        let play_a = (base_a + (1.0 - base_a) * play_h) * alpha;
        if 1.0 - play_t > 0.01 {
            let a = (1.0 - play_t) * play_a;
            let s = 0.76 + 0.24 * (1.0 - play_t);
            self.draw_ss_icon(ATLAS_SLOT_PLAY, play_x, ctrl_y, base_play_size * s, a);
        }
        if play_t > 0.01 {
            let a = play_t * play_a;
            let s = 0.76 + 0.24 * play_t;
            self.draw_ss_icon(ATLAS_SLOT_PAUSE, play_x, ctrl_y, base_play_size * s, a);
        }

        let next_x = lerp(layout.cx + dx + 48.0, bl + 171.0, lyr_t);
        let next_h = if media.has_session {
            springs.slot_hover(5)
        } else {
            0.0
        };
        let next_scale = if media.has_session {
            springs.slot_scale(5, 0.09, 0.15)
        } else {
            1.0
        };
        self.draw_ss_icon(
            ATLAS_SLOT_NEXT,
            next_x,
            ctrl_y,
            19.5 * next_scale,
            (base_a + (1.0 - base_a) * next_h) * alpha,
        );

        let spk_fade = alpha * chrome_fade;
        if spk_fade > 0.01 {
            let spk_h = springs.slot_hover(7);
            let spk_scale = springs.slot_scale(7, 0.09, 0.15);
            let raw_vol = springs.speaker_volume().clamp(0.0, 100.0);
            let vol = if state.muted && springs.speaker_volume.is_settled() {
                0.0
            } else {
                raw_vol
            };

            let vol_t = springs.volume_slider_open().clamp(0.0, 1.0);
            let slider_a = spk_fade * ((vol_t - 0.05) / 0.95).clamp(0.0, 1.0);
            if slider_a > 0.01 {
                let mut spk_center = layout.speaker_btn_center;
                spk_center.x += dx;
                let (_, anim_rect, _) = music_volume_slider_rects(spk_center, vol_t);
                let slider_h = springs.slot_hover(42);
                let half_h = 3.0 + 0.6 * slider_h;
                let track_rect = D2D_RECT_F {
                    left: anim_rect.left,
                    top: spk_center.y - half_h,
                    right: anim_rect.right,
                    bottom: spk_center.y + half_h,
                };
                let track_rrect = D2D1_ROUNDED_RECT {
                    rect: track_rect,
                    radiusX: half_h,
                    radiusY: half_h,
                };
                self.set_color(0.227, 0.227, 0.239, 0.96 * slider_a);
                self.d2d_context
                    .FillRoundedRectangle(&track_rrect, &self.solid_brush);
                self.set_color(1.0, 1.0, 1.0, (0.06 + 0.08 * slider_h) * slider_a);
                self.d2d_context
                    .DrawRoundedRectangle(&track_rrect, &self.solid_brush, 0.75, None);

                let track_w = (track_rect.right - track_rect.left).max(0.0);
                let fill_w = track_w * (vol / 100.0).clamp(0.0, 1.0);
                if fill_w > 0.5 {
                    let fill_r = (fill_w * 0.5).min(half_h);
                    let fill_rect = D2D_RECT_F {
                        left: track_rect.left,
                        top: spk_center.y - half_h,
                        right: track_rect.left + fill_w,
                        bottom: spk_center.y + half_h,
                    };
                    self.set_color(
                        0.898 + 0.05 * slider_h,
                        0.902 + 0.05 * slider_h,
                        0.910 + 0.05 * slider_h,
                        0.98 * slider_a,
                    );
                    self.d2d_context.FillRoundedRectangle(
                        &D2D1_ROUNDED_RECT {
                            rect: fill_rect,
                            radiusX: fill_r,
                            radiusY: fill_r,
                        },
                        &self.solid_brush,
                    );
                }
            }

            let spk_alpha = (0.92 + 0.08 * spk_h) * spk_fade;
            self.draw_ss_speaker(
                layout.speaker_btn_center.x + dx,
                layout.speaker_btn_center.y,
                18.5 * spk_scale,
                spk_alpha,
                vol,
                2,
            );
        }

        let pane_a = alpha * ((lyr_t - 0.18) / 0.82).clamp(0.0, 1.0);
        if pane_a > 0.01 {
            let lyrics_bl = (layout.cx - 260.0).round() + dx;
            let pane_left = lyrics_bl + 265.0;
            let pane_right = lyrics_bl + 501.0;

            self.set_color(1.0, 1.0, 1.0, 0.13 * pane_a);
            self.d2d_context.DrawLine(
                D2D_POINT_2F { x: lyrics_bl + 252.0, y: 32.0 },
                D2D_POINT_2F { x: lyrics_bl + 252.0, y: 148.0 },
                &self.solid_brush,
                1.0,
                None,
            );

            let clip_rect = D2D_RECT_F {
                left: lyrics_bl + 258.0,
                top: 20.0,
                right: lyrics_bl + 506.0,
                bottom: 160.0,
            };
            let mask_stops = [
                D2D1_GRADIENT_STOP {
                    position: 0.0,
                    color: D2D1_COLOR_F { r: 1.0, g: 1.0, b: 1.0, a: 0.0 },
                },
                D2D1_GRADIENT_STOP {
                    position: 0.18,
                    color: D2D1_COLOR_F { r: 1.0, g: 1.0, b: 1.0, a: 1.0 },
                },
                D2D1_GRADIENT_STOP {
                    position: 0.78,
                    color: D2D1_COLOR_F { r: 1.0, g: 1.0, b: 1.0, a: 1.0 },
                },
                D2D1_GRADIENT_STOP {
                    position: 1.0,
                    color: D2D1_COLOR_F { r: 1.0, g: 1.0, b: 1.0, a: 0.0 },
                },
            ];
            let mask_brush: Option<ID2D1Brush> = ID2D1RenderTarget::CreateGradientStopCollection(
                &self.d2d_context,
                &mask_stops,
                D2D1_GAMMA_2_2,
                D2D1_EXTEND_MODE_CLAMP,
            )
            .ok()
            .and_then(|sc| {
                let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                    startPoint: D2D_POINT_2F { x: pane_left, y: 20.0 },
                    endPoint: D2D_POINT_2F { x: pane_left, y: 160.0 },
                };
                self.d2d_context
                    .CreateLinearGradientBrush(&props, None, &sc)
                    .ok()
                    .and_then(|b| b.cast().ok())
            });
            let mut layer_params = D2D1_LAYER_PARAMETERS1 {
                contentBounds: clip_rect,
                geometricMask: ManuallyDrop::new(None),
                maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                maskTransform: Matrix3x2::identity(),
                opacity: 1.0,
                opacityBrush: ManuallyDrop::new(mask_brush),
                layerOptions: D2D1_LAYER_OPTIONS1_NONE,
            };
            let pushed_layer = layer_params.opacityBrush.is_some();
            if pushed_layer {
                self.d2d_context.PushLayer(&layer_params, None);
                self.d2d_context
                    .SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            }
            self.d2d_context
                .PushAxisAlignedClip(&clip_rect, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);

            if media.lyric_prev_prev.is_empty()
                && media.lyric_prev.is_empty()
                && media.lyric_current.is_empty()
                && media.lyric_next.is_empty()
            {
                self.draw_text(
                    "No synced lyrics available",
                    &self.fmt_lyric_line_dim,
                    &D2D_RECT_F {
                        left: pane_left,
                        top: 78.0,
                        right: pane_right,
                        bottom: 120.0,
                    },
                    (0.50, 0.50, 0.52, 1.0 * pane_a),
                );
            } else {
                let line_t = springs.lyric_line_change.value().clamp(0.0, 1.0);
                let measure_line_h = |txt: &str| -> f32 {
                    if txt.is_empty() {
                        22.0
                    } else if lyrics::is_instrumental_text(txt) {
                        24.0
                    } else {
                        self.measure_multiline_height(txt, &self.fmt_lyric_line_active, 236.0)
                            .clamp(19.0, 64.0)
                    }
                };
                let h_pp = measure_line_h(media.lyric_prev_prev);
                let h_p = measure_line_h(media.lyric_prev);
                let h_c = measure_line_h(media.lyric_current);
                let _h_n = measure_line_h(media.lyric_next);

                let center_y = 92.5;
                let gap = 11.0;
                let curr_rest_top = center_y - h_c * 0.5;
                let prev_rest_top = curr_rest_top - gap - h_p * 0.93;
                let next_rest_top = curr_rest_top + h_c + gap;
                let old_curr_top = center_y - h_p * 0.5;
                let old_prev_top = old_curr_top - gap - h_pp * 0.93;
                let old_next_top = old_curr_top + h_p + gap;

                let mut prev_xf = Matrix3x2::identity();
                self.d2d_context.GetTransform(&mut prev_xf);

                let draw_scaled_line = |text: &str,
                                        fmt: &IDWriteTextFormat,
                                        top: f32,
                                        h: f32,
                                        scale: f32,
                                        col: (f32, f32, f32, f32)| {
                    if col.3 <= 0.005 || text.is_empty() {
                        return;
                    }
                    if (scale - 1.0).abs() > 0.001 {
                        let sxf = Matrix3x2 {
                            M11: scale,
                            M12: 0.0,
                            M21: 0.0,
                            M22: scale,
                            M31: pane_left * (1.0 - scale),
                            M32: top * (1.0 - scale),
                        };
                        let combined = mul_matrix3x2(sxf, prev_xf);
                        self.d2d_context.SetTransform(&combined);
                    }
                    if lyrics::is_instrumental_text(text) {
                        self.draw_ss_icon(
                            ATLAS_SLOT_MUSIC_NOTE,
                            pane_left + 12.0,
                            top + h * 0.5,
                            20.0,
                            col.3,
                        );
                    } else {
                        let rect = D2D_RECT_F {
                            left: pane_left,
                            top,
                            right: pane_right,
                            bottom: top + h + 4.0,
                        };
                        if line_t < 0.999 || (scale - 1.0).abs() > 0.001 {
                            self.draw_text_smooth(text, fmt, &rect, col);
                        } else {
                            self.draw_text(text, fmt, &rect, col);
                        }
                    }
                    if (scale - 1.0).abs() > 0.001 {
                        self.d2d_context.SetTransform(&prev_xf);
                    }
                };

                let h0 = springs.slot_hover(39);
                let h2 = springs.slot_hover(41);
                let going_back = springs.lyric_scroll_dir < 0.0 && !media.lyric_current.is_empty();

                if going_back {
                    let old_curr_back_top = center_y - _h_n * 0.5;
                    let old_prev_back_top = old_curr_back_top - gap - h_c * 0.93;

                    if !media.lyric_prev.is_empty() {
                        let prev_y = lerp(prev_rest_top - gap - h_p * 0.93, prev_rest_top, line_t);
                        let prev_a = line_t * (0.36 + 0.25 * h0) * pane_a;
                        draw_scaled_line(
                            media.lyric_prev,
                            &self.fmt_lyric_line_dim,
                            prev_y,
                            h_p,
                            0.93,
                            (1.0, 1.0, 1.0, prev_a),
                        );
                    }

                    let cur_y = lerp(old_prev_back_top, curr_rest_top, line_t);
                    let cur_s = lerp(0.93, 1.0, line_t);
                    let cur_a = lerp(0.36, 0.98, line_t) * pane_a;
                    draw_scaled_line(
                        media.lyric_current,
                        &self.fmt_lyric_line_active,
                        cur_y,
                        h_c,
                        cur_s,
                        (1.0, 1.0, 1.0, cur_a),
                    );

                    if !media.lyric_next.is_empty() {
                        let nxt_y = lerp(old_curr_back_top, next_rest_top, line_t);
                        let nxt_s = lerp(1.0, 0.93, line_t);
                        let nxt_a = lerp(0.98, 0.36 + 0.25 * h2, line_t) * pane_a;
                        draw_scaled_line(
                            media.lyric_next,
                            &self.fmt_lyric_line_dim,
                            nxt_y,
                            _h_n,
                            nxt_s,
                            (1.0, 1.0, 1.0, nxt_a),
                        );
                    }
                } else if !media.lyric_prev.is_empty() {
                    if !media.lyric_prev_prev.is_empty() {
                        let pp_y = lerp(old_prev_top, prev_rest_top - gap - h_pp * 0.93, line_t);
                        let pp_a = lerp(0.36, 0.0, line_t) * pane_a;
                        draw_scaled_line(
                            media.lyric_prev_prev,
                            &self.fmt_lyric_line_dim,
                            pp_y,
                            h_pp,
                            0.93,
                            (1.0, 1.0, 1.0, pp_a),
                        );
                    }

                    let prev_y = lerp(old_curr_top, prev_rest_top, line_t);
                    let prev_s = lerp(1.0, 0.93, line_t);
                    let prev_a = lerp(0.98, 0.36 + 0.25 * h0, line_t) * pane_a;
                    draw_scaled_line(
                        media.lyric_prev,
                        &self.fmt_lyric_line_dim,
                        prev_y,
                        h_p,
                        prev_s,
                        (1.0, 1.0, 1.0, prev_a),
                    );

                    if !media.lyric_current.is_empty() {
                        let cur_y = lerp(old_next_top, curr_rest_top, line_t);
                        let cur_s = lerp(0.93, 1.0, line_t);
                        let cur_a = lerp(0.36, 0.98, line_t) * pane_a;
                        draw_scaled_line(
                            media.lyric_current,
                            &self.fmt_lyric_line_active,
                            cur_y,
                            h_c,
                            cur_s,
                            (1.0, 1.0, 1.0, cur_a),
                        );
                    }

                    if !media.lyric_next.is_empty() {
                        let nxt_y = lerp(next_rest_top + h_c * 0.65 + gap, next_rest_top, line_t);
                        let nxt_a = line_t * (0.36 + 0.25 * h2) * pane_a;
                        draw_scaled_line(
                            media.lyric_next,
                            &self.fmt_lyric_line_dim,
                            nxt_y,
                            _h_n,
                            0.93,
                            (1.0, 1.0, 1.0, nxt_a),
                        );
                    }
                } else {
                    let intro_dy = (1.0 - line_t) * 14.0;
                    let started_mix =
                        if !media.lyric_started && springs.lyric_started_mix.is_settled() {
                            0.0
                        } else {
                            springs.lyric_started_mix().clamp(0.0, 1.0)
                        };
                    if media.lyric_current.is_empty() && !media.lyric_next.is_empty() {
                        let h_0 = measure_line_h(media.lyric_next);
                        let line0_top = center_y - h_0 * 0.5 + intro_dy;
                        let line0_a = lerp(0.38, 0.98, started_mix) * line_t * pane_a;
                        let line0_scale = lerp(0.95, 1.0, started_mix);
                        let line0_fmt = if started_mix > 0.5 {
                            &self.fmt_lyric_line_active
                        } else {
                            &self.fmt_lyric_line_dim
                        };
                        draw_scaled_line(
                            media.lyric_next,
                            line0_fmt,
                            line0_top,
                            h_0,
                            line0_scale,
                            (1.0, 1.0, 1.0, line0_a),
                        );
                        if !media.lyric_next_after.is_empty() {
                            let h_1 = measure_line_h(media.lyric_next_after);
                            let line1_top = line0_top + h_0 + gap;
                            draw_scaled_line(
                                media.lyric_next_after,
                                &self.fmt_lyric_line_dim,
                                line1_top,
                                h_1,
                                0.93,
                                (1.0, 1.0, 1.0, (0.28 + 0.18 * h2) * line_t * pane_a),
                            );
                        }
                    } else {
                        if !media.lyric_current.is_empty() {
                            let cur_a = lerp(0.38, 0.98, started_mix) * line_t * pane_a;
                            let cur_s = lerp(0.95, 1.0, started_mix);
                            let cur_fmt = if started_mix > 0.5 {
                                &self.fmt_lyric_line_active
                            } else {
                                &self.fmt_lyric_line_dim
                            };
                            draw_scaled_line(
                                media.lyric_current,
                                cur_fmt,
                                curr_rest_top + intro_dy,
                                h_c,
                                cur_s,
                                (1.0, 1.0, 1.0, cur_a),
                            );
                        }
                        if !media.lyric_next.is_empty() {
                            draw_scaled_line(
                                media.lyric_next,
                                &self.fmt_lyric_line_dim,
                                next_rest_top + intro_dy,
                                _h_n,
                                0.93,
                                (1.0, 1.0, 1.0, (0.36 + 0.25 * h2) * line_t * pane_a),
                            );
                        }
                    }
                }
            }

            self.d2d_context.PopAxisAlignedClip();
            if pushed_layer {
                self.d2d_context.PopLayer();
                if cleartype_active {
                    self.d2d_context
                        .SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_CLEARTYPE);
                }
            }
            ManuallyDrop::drop(&mut layer_params.opacityBrush);
        }
    }

    unsafe fn draw_collapsed_left_ring(
        &self,
        mode: u8,
        springs: &NotchSprings,
        state: &EaveViewState<'_>,
        cx: f32,
        cy: f32,
        r: f32,
        alpha: f32,
    ) {
        let effective = if mode == 3 {
            if state.pomodoro.running {
                4
            } else if state.water.taken_ml > 0 {
                1
            } else {
                2
            }
        } else {
            mode
        };

        let (prog, col) = match effective {
            1 => (
                springs.widget_water_fill.value().clamp(0.0, 1.0),
                (0.04, 0.52, 1.0),
            ),
            2 => (
                springs.left_ring_todo_prog.value().clamp(0.0, 1.0),
                (0.19, 0.82, 0.35),
            ),
            4 => {
                let p = springs.widget_focus_prog.value().clamp(0.0, 1.0);
                let is_focus = state.pomodoro.phase == PomodoroPhase::Focus;
                let is_long = !is_focus && state.pomodoro.current_round >= 4;
                let c = if is_focus {
                    (0.58, 0.42, 1.0)
                } else if is_long {
                    (1.0, 0.58, 0.54)
                } else {
                    (0.18, 0.86, 0.78)
                };
                (p, c)
            }
            _ => return,
        };

        let stroke_w = (r * 0.32).clamp(2.2, 2.6);

        self.set_color(col.0, col.1, col.2, 0.24 * alpha);
        self.d2d_context.DrawEllipse(
            &D2D1_ELLIPSE {
                point: D2D_POINT_2F { x: cx, y: cy },
                radiusX: r,
                radiusY: r,
            },
            &self.solid_brush,
            stroke_w,
            None,
        );

        let sweep_deg = (prog * 359.5).max(if prog > 0.005 { 22.0 } else { 0.0 });
        if sweep_deg > 1.0 {
            let a0 = (-90.0_f32).to_radians();
            let a1 = (-90.0_f32 + sweep_deg).to_radians();
            if let Ok(geo) = self.d2d_factory.CreatePathGeometry() {
                if let Ok(sink) = geo.Open() {
                    let p0 = D2D_POINT_2F {
                        x: cx + r * a0.cos(),
                        y: cy + r * a0.sin(),
                    };
                    let p1 = D2D_POINT_2F {
                        x: cx + r * a1.cos(),
                        y: cy + r * a1.sin(),
                    };
                    sink.BeginFigure(
                        p0,
                        windows::Win32::Graphics::Direct2D::Common::D2D1_FIGURE_BEGIN_HOLLOW,
                    );
                    sink.AddArc(&D2D1_ARC_SEGMENT {
                        point: p1,
                        size: D2D_SIZE_F { width: r, height: r },
                        rotationAngle: 0.0,
                        sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                        arcSize: if sweep_deg > 180.0 {
                            windows::Win32::Graphics::Direct2D::D2D1_ARC_SIZE_LARGE
                        } else {
                            D2D1_ARC_SIZE_SMALL
                        },
                    });
                    sink.EndFigure(
                        windows::Win32::Graphics::Direct2D::Common::D2D1_FIGURE_END_OPEN,
                    );
                    if sink.Close().is_ok() {
                        let round_stroke = ID2D1Factory::CreateStrokeStyle(
                            &self.d2d_factory,
                            &D2D1_STROKE_STYLE_PROPERTIES {
                                startCap: D2D1_CAP_STYLE_ROUND,
                                endCap: D2D1_CAP_STYLE_ROUND,
                                ..Default::default()
                            },
                            None,
                        )
                        .ok();
                        self.set_color(col.0, col.1, col.2, alpha);
                        self.d2d_context.DrawGeometry(
                            &geo,
                            &self.solid_brush,
                            stroke_w,
                            round_stroke.as_ref(),
                        );
                    }
                }
            }
        }

        let brush: &ID2D1Brush = &self.solid_brush;
        if effective == 1 {
            self.set_color(1.0, 1.0, 1.0, 0.96 * alpha);
            icons::draw_water_drop_filled(&self.d2d_context, cx, cy + 0.2, r * 1.18, brush);
        } else if effective == 2 {
            self.set_color(1.0, 1.0, 1.0, 0.96 * alpha);
            icons::draw_checkmark_animated(&self.d2d_context, cx, cy, r * 1.10, brush, 1.75, 1.0);
        }
    }

    unsafe fn draw_pomodoro_ring(
        &self,
        cx: f32,
        cy: f32,
        radius: f32,
        is_focus: bool,
        is_long_break: bool,
        phase_secs: f32,
        alpha: f32,
    ) {
        if alpha <= 0.01 {
            return;
        }
        let angle = phase_secs * 0.85 + 0.42 * (phase_secs * 0.43).sin();
        let (sin_a, cos_a) = angle.sin_cos();

        let (c1, c_mid, c2) = if is_focus {
            (
                (0.64, 0.42, 1.0),
                (0.54, 0.44, 0.99),
                (0.45, 0.48, 0.98),
            )
        } else if is_long_break {
            (
                (1.0, 0.76, 0.48),
                (1.0, 0.58, 0.54),
                (1.0, 0.42, 0.62),
            )
        } else {
            (
                (0.14, 0.88, 0.96),
                (0.18, 0.86, 0.78),
                (0.22, 0.90, 0.56),
            )
        };

        let ring_stops = [
            D2D1_GRADIENT_STOP {
                position: 0.0,
                color: D2D1_COLOR_F {
                    r: c1.0,
                    g: c1.1,
                    b: c1.2,
                    a: 0.98 * alpha,
                },
            },
            D2D1_GRADIENT_STOP {
                position: 0.5,
                color: D2D1_COLOR_F {
                    r: c_mid.0,
                    g: c_mid.1,
                    b: c_mid.2,
                    a: 0.98 * alpha,
                },
            },
            D2D1_GRADIENT_STOP {
                position: 1.0,
                color: D2D1_COLOR_F {
                    r: c2.0,
                    g: c2.1,
                    b: c2.2,
                    a: 0.98 * alpha,
                },
            },
        ];
        let ell = D2D1_ELLIPSE {
            point: D2D_POINT_2F { x: cx, y: cy },
            radiusX: radius,
            radiusY: radius,
        };
        if let Ok(sc) = ID2D1RenderTarget::CreateGradientStopCollection(
            &self.d2d_context,
            &ring_stops,
            D2D1_GAMMA_2_2,
            D2D1_EXTEND_MODE_CLAMP,
        ) {
            let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                startPoint: D2D_POINT_2F {
                    x: cx - cos_a * radius,
                    y: cy - sin_a * radius,
                },
                endPoint: D2D_POINT_2F {
                    x: cx + cos_a * radius,
                    y: cy + sin_a * radius,
                },
            };
            if let Ok(lin) = self.d2d_context.CreateLinearGradientBrush(&props, None, &sc) {
                self.d2d_context.DrawEllipse(&ell, &lin, 2.0, None);
                return;
            }
        }
        self.set_color(c1.0, c1.1, c1.2, 0.98 * alpha);
        self.d2d_context.DrawEllipse(&ell, &self.solid_brush, 2.0, None);
    }

    unsafe fn draw_pomodoro_page(
        &self,
        springs: &NotchSprings,
        pomo: &PomodoroState,
        layout: &ExpandedLayout,
        dx: f32,
        alpha: f32,
    ) {
        let brush: &ID2D1Brush = &self.solid_brush;
        let bl = layout.body_left + dx;
        let is_focus = pomo.phase == PomodoroPhase::Focus;
        let is_long_break = !is_focus && pomo.current_round >= 4;

        let phase_label = format!("{} · {} of 4", pomo.display_phase_label(), pomo.current_round);
        let switch_h = springs.slot_hover(11);
        self.draw_text(
            &phase_label,
            &self.fmt_artist,
            &D2D_RECT_F {
                left: bl + 33.0,
                top: 44.5,
                right: bl + 174.0,
                bottom: 62.5,
            },
            (
                (0.60 + 0.32 * switch_h).min(1.0),
                (0.60 + 0.32 * switch_h).min(1.0),
                (0.60 + 0.32 * switch_h).min(1.0),
                1.0 * alpha,
            ),
        );

        let time_str = pomo.formatted_remaining();
        self.draw_text(
            &time_str,
            &self.fmt_timer_lg,
            &D2D_RECT_F {
                left: bl + 31.0,
                top: 62.0,
                right: bl + 178.0,
                bottom: 103.0,
            },
            (0.957, 0.957, 0.965, 1.0 * alpha),
        );

        let accent_status_col = if is_focus {
            (0.635, 0.455, 1.0, 0.98 * alpha)
        } else if is_long_break {
            (1.0, 0.420, 0.616, 0.98 * alpha)
        } else {
            (0.133, 0.827, 0.933, 0.98 * alpha)
        };

        if !pomo.running && pomo.progress() <= 0.001 {
            let mins_part = format!("{} min ", (pomo.remaining_secs / 60.0).round() as u32);
            let mw = self.measure_width(&mins_part, &self.fmt_pomo_until);
            self.draw_text(
                &mins_part,
                &self.fmt_pomo_until,
                &D2D_RECT_F {
                    left: bl + 33.0,
                    top: 107.0,
                    right: bl + 33.0 + mw + 4.0,
                    bottom: 124.0,
                },
                (0.96, 0.96, 0.98, 1.0 * alpha),
            );
            self.draw_text(
                "Ready",
                &self.fmt_pomo_until,
                &D2D_RECT_F {
                    left: bl + 33.0 + mw,
                    top: 107.0,
                    right: bl + 175.0,
                    bottom: 124.0,
                },
                accent_status_col,
            );
        } else {
            let finish_clock = format_future_clock_24h(pomo.remaining_secs);
            let finish_part = format!("{finish_clock} ");
            let fw = self.measure_width(&finish_part, &self.fmt_pomo_until);
            self.draw_text(
                &finish_part,
                &self.fmt_pomo_until,
                &D2D_RECT_F {
                    left: bl + 33.0,
                    top: 107.0,
                    right: bl + 33.0 + fw + 4.0,
                    bottom: 124.0,
                },
                (1.0, 1.0, 1.0, 1.0 * alpha),
            );
            self.draw_text(
                "until done",
                &self.fmt_pomo_until,
                &D2D_RECT_F {
                    left: bl + 33.0 + fw,
                    top: 107.0,
                    right: bl + 175.0,
                    bottom: 124.0,
                },
                accent_status_col,
            );
        }

        let left_bar = D2D_RECT_F {
            left: bl + 33.0,
            top: 133.0,
            right: bl + 161.0,
            bottom: 137.0,
        };
        self.set_color(1.0, 1.0, 1.0, 0.14 * alpha);
        self.d2d_context.FillRoundedRectangle(
            &D2D1_ROUNDED_RECT {
                rect: left_bar,
                radiusX: 2.0,
                radiusY: 2.0,
            },
            &self.solid_brush,
        );
        let p_smooth = springs.widget_focus_prog.value().clamp(0.0, 1.0);
        let p_fill_w = (128.0 * p_smooth).max(if p_smooth > 0.004 { 4.0 } else { 0.0 });
        if p_fill_w > 0.5 {
            let fill_rect = D2D_RECT_F {
                left: left_bar.left,
                top: left_bar.top,
                right: left_bar.left + p_fill_w,
                bottom: left_bar.bottom,
            };
            let (bar_c0, bar_c1) = if is_focus {
                ((0.357, 0.424, 1.0, 0.98 * alpha), (0.659, 0.333, 0.969, 0.98 * alpha))
            } else if is_long_break {
                ((1.0, 0.745, 0.463, 0.98 * alpha), (1.0, 0.420, 0.616, 0.98 * alpha))
            } else {
                ((0.133, 0.827, 0.933, 0.98 * alpha), (0.204, 0.827, 0.600, 0.98 * alpha))
            };
            self.fill_linear_gradient_rrect(
                &D2D1_ROUNDED_RECT {
                    rect: fill_rect,
                    radiusX: 2.0,
                    radiusY: 2.0,
                },
                bar_c0,
                bar_c1,
            );
        }

        self.set_color(1.0, 1.0, 1.0, 0.10 * alpha);
        self.d2d_context.DrawLine(
            D2D_POINT_2F { x: bl + 179.0, y: 53.0 },
            D2D_POINT_2F { x: bl + 179.0, y: 141.0 },
            &self.solid_brush,
            1.0,
            None,
        );

        self.draw_text(
            "This round",
            &self.fmt_artist,
            &D2D_RECT_F {
                left: bl + 192.0,
                top: 55.0,
                right: bl + 278.0,
                bottom: 73.0,
            },
            (0.60, 0.60, 0.60, 1.0 * alpha),
        );

        let (btn_toggle, reset_c, _) = pomodoro_controls(bl);
        let toggle_h = springs.slot_hover(9);
        let toggle_rect = scale_rect(btn_toggle, springs.slot_scale(9, 0.04, 0.08));
        let toggle_rrect = D2D1_ROUNDED_RECT {
            rect: toggle_rect,
            radiusX: 9.0,
            radiusY: 9.0,
        };
        let run_t = springs.pomo_running_mix.value().clamp(0.0, 1.0);
        if (1.0 - run_t) > 0.005 {
            let (g_start, g_end) = if is_focus {
                (
                    (
                        (0.255 + 0.05 * toggle_h).min(1.0),
                        (0.365 + 0.05 * toggle_h).min(1.0),
                        1.0,
                        0.98 * (1.0 - run_t) * alpha,
                    ),
                    (
                        (0.745 + 0.05 * toggle_h).min(1.0),
                        (0.275 + 0.05 * toggle_h).min(1.0),
                        1.0,
                        0.98 * (1.0 - run_t) * alpha,
                    ),
                )
            } else if is_long_break {
                (
                    (
                        1.0,
                        (0.745 + 0.05 * toggle_h).min(1.0),
                        (0.463 + 0.05 * toggle_h).min(1.0),
                        0.98 * (1.0 - run_t) * alpha,
                    ),
                    (
                        1.0,
                        (0.420 + 0.05 * toggle_h).min(1.0),
                        (0.616 + 0.05 * toggle_h).min(1.0),
                        0.98 * (1.0 - run_t) * alpha,
                    ),
                )
            } else {
                (
                    (
                        (0.055 + 0.04 * toggle_h).min(1.0),
                        (0.755 + 0.05 * toggle_h).min(1.0),
                        (0.865 + 0.04 * toggle_h).min(1.0),
                        0.98 * (1.0 - run_t) * alpha,
                    ),
                    (
                        (0.155 + 0.04 * toggle_h).min(1.0),
                        (0.880 + 0.05 * toggle_h).min(1.0),
                        (0.515 + 0.05 * toggle_h).min(1.0),
                        0.98 * (1.0 - run_t) * alpha,
                    ),
                )
            };
            self.fill_linear_gradient_rrect(&toggle_rrect, g_start, g_end);
        }
        if run_t > 0.005 {
            self.set_color(
                0.072 + 0.032 * toggle_h,
                0.072 + 0.032 * toggle_h,
                0.072 + 0.032 * toggle_h,
                0.98 * run_t * alpha,
            );
            self.d2d_context.FillRoundedRectangle(&toggle_rrect, &self.solid_brush);
            self.set_color(1.0, 1.0, 1.0, (0.08 + 0.07 * toggle_h) * run_t * alpha);
            self.d2d_context
                .DrawRoundedRectangle(&toggle_rrect, &self.solid_brush, 0.85, None);
        }
        let toggle_text_rect = D2D_RECT_F {
            left: toggle_rect.left,
            top: toggle_rect.top - 0.8,
            right: toggle_rect.right,
            bottom: toggle_rect.bottom - 0.8,
        };
        let start_a = (1.0 - run_t).clamp(0.0, 1.0);
        let pause_a = run_t.clamp(0.0, 1.0);
        if start_a > 0.01 {
            let fg = if is_long_break {
                (0.09, 0.09, 0.11, 0.98 * start_a * alpha)
            } else {
                (1.0, 1.0, 1.0, 0.98 * start_a * alpha)
            };
            self.draw_text("Start", &self.fmt_pill_sm, &toggle_text_rect, fg);
        }
        if pause_a > 0.01 {
            self.draw_text(
                "Pause",
                &self.fmt_pill_sm,
                &toggle_text_rect,
                (1.0, 1.0, 1.0, 0.98 * pause_a * alpha),
            );
        }

        let reset_h = springs.slot_hover(10);
        let reset_scale = springs.slot_scale(10, 0.08, 0.14);
        let reset_ell = D2D1_ELLIPSE {
            point: reset_c,
            radiusX: 9.5 * reset_scale,
            radiusY: 9.5 * reset_scale,
        };
        self.set_color(
            0.072 + 0.032 * reset_h,
            0.072 + 0.032 * reset_h,
            0.072 + 0.032 * reset_h,
            0.98 * alpha,
        );
        self.d2d_context.FillEllipse(&reset_ell, &self.solid_brush);
        self.set_color(1.0, 1.0, 1.0, (0.08 + 0.07 * reset_h) * alpha);
        self.d2d_context
            .DrawEllipse(&reset_ell, &self.solid_brush, 0.85, None);
        self.set_color(1.0, 1.0, 1.0, (0.90 + 0.10 * reset_h) * alpha);
        icons::draw_reset_ccw(
            &self.d2d_context,
            reset_c.x,
            reset_c.y,
            14.5 * reset_scale,
            brush,
        );

        const SEG_W: [f32; 8] = [30.0, 6.0, 30.0, 6.0, 30.0, 6.0, 30.0, 18.0];
        const SEG_GAP: f32 = 3.0;
        let active_seg = match pomo.phase {
            PomodoroPhase::Focus => ((pomo.current_round.clamp(1, 4) - 1) * 2) as usize,
            PomodoroPhase::Break => {
                if pomo.current_round >= 4 {
                    7
                } else {
                    (pomo.current_round.clamp(1, 3) * 2 - 1) as usize
                }
            }
        };

        let mut sx = bl + 190.0;
        let sy = 98.0;
        let mut orb_x: Option<f32> = None;
        let mut label_xs = [0.0_f32; 5];

        for (idx, &sw) in SEG_W.iter().enumerate() {
            if idx % 2 == 0 {
                label_xs[idx / 2] = sx + sw * 0.5;
            } else if idx == 7 {
                label_xs[4] = sx + sw * 0.5;
            }

            let seg_rect = D2D_RECT_F {
                left: sx,
                top: sy,
                right: sx + sw,
                bottom: sy + 6.0,
            };
            self.set_color(1.0, 1.0, 1.0, 0.12 * alpha);
            self.d2d_context.FillRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: seg_rect,
                    radiusX: 3.0,
                    radiusY: 3.0,
                },
                &self.solid_brush,
            );

            let is_seg_focus = idx % 2 == 0;
            let is_seg_long = idx == 7;
            if idx < active_seg {
                let rrect = D2D1_ROUNDED_RECT {
                    rect: seg_rect,
                    radiusX: 3.0,
                    radiusY: 3.0,
                };
                if is_seg_focus {
                    self.fill_linear_gradient_rrect(
                        &rrect,
                        (0.36, 0.44, 1.0, 0.98 * alpha),
                        (0.68, 0.34, 1.0, 0.98 * alpha),
                    );
                } else if is_seg_long {
                    self.fill_linear_gradient_rrect(
                        &rrect,
                        (1.0, 0.745, 0.463, 0.98 * alpha),
                        (1.0, 0.420, 0.616, 0.98 * alpha),
                    );
                } else {
                    self.set_color(0.16, 0.86, 0.80, 0.98 * alpha);
                    self.d2d_context.FillRoundedRectangle(&rrect, &self.solid_brush);
                }
            } else if idx == active_seg {
                let p = springs.widget_focus_prog.value().clamp(0.0, 1.0);
                let fw = (sw * p).max(0.0);
                if fw > 0.5 {
                    let rrect = D2D1_ROUNDED_RECT {
                        rect: D2D_RECT_F {
                            left: sx,
                            top: sy,
                            right: sx + fw,
                            bottom: sy + 6.0,
                        },
                        radiusX: 3.0,
                        radiusY: 3.0,
                    };
                    if is_seg_focus {
                        self.fill_linear_gradient_rrect(
                            &rrect,
                            (0.36, 0.44, 1.0, 0.98 * alpha),
                            (0.68, 0.34, 1.0, 0.98 * alpha),
                        );
                    } else if is_seg_long {
                        self.fill_linear_gradient_rrect(
                            &rrect,
                            (1.0, 0.745, 0.463, 0.98 * alpha),
                            (1.0, 0.420, 0.616, 0.98 * alpha),
                        );
                    } else {
                        self.set_color(0.16, 0.86, 0.80, 0.98 * alpha);
                        self.d2d_context.FillRoundedRectangle(&rrect, &self.solid_brush);
                    }
                }
                orb_x = Some(sx + fw);
            }

            sx += sw + SEG_GAP;
        }

        let orb_alpha = alpha * run_t.max(if pomo.progress() > 0.001 { 1.0 } else { 0.0 });
        if orb_alpha > 0.01 {
            if let Some(ox) = orb_x {
                let oy = sy + 3.0;
                let (gr, gg, gb) = if is_focus {
                    (0.66, 0.34, 1.0)
                } else if is_long_break {
                    (1.0, 0.48, 0.56)
                } else {
                    (0.16, 0.88, 0.72)
                };
                let bloom_r = 12.5;
                let bloom_stops = [
                    D2D1_GRADIENT_STOP {
                        position: 0.0,
                        color: D2D1_COLOR_F {
                            r: gr,
                            g: gg,
                            b: gb,
                            a: 0.82 * orb_alpha,
                        },
                    },
                    D2D1_GRADIENT_STOP {
                        position: 0.42,
                        color: D2D1_COLOR_F {
                            r: gr,
                            g: gg,
                            b: gb,
                            a: 0.38 * orb_alpha,
                        },
                    },
                    D2D1_GRADIENT_STOP {
                        position: 1.0,
                        color: D2D1_COLOR_F {
                            r: gr,
                            g: gg,
                            b: gb,
                            a: 0.0,
                        },
                    },
                ];
                if let Ok(sc) = ID2D1RenderTarget::CreateGradientStopCollection(
                    &self.d2d_context,
                    &bloom_stops,
                    D2D1_GAMMA_2_2,
                    D2D1_EXTEND_MODE_CLAMP,
                ) {
                    let rprops = D2D1_RADIAL_GRADIENT_BRUSH_PROPERTIES {
                        center: D2D_POINT_2F { x: ox, y: oy },
                        gradientOriginOffset: D2D_POINT_2F { x: 0.0, y: 0.0 },
                        radiusX: bloom_r,
                        radiusY: bloom_r,
                    };
                    if let Ok(rad) = self.d2d_context.CreateRadialGradientBrush(&rprops, None, &sc) {
                        self.d2d_context.FillEllipse(
                            &D2D1_ELLIPSE {
                                point: D2D_POINT_2F { x: ox, y: oy },
                                radiusX: bloom_r,
                                radiusY: bloom_r,
                            },
                            &rad,
                        );
                    }
                }
                self.set_color(1.0, 1.0, 1.0, 0.98 * orb_alpha);
                self.d2d_context.FillEllipse(
                    &D2D1_ELLIPSE {
                        point: D2D_POINT_2F { x: ox, y: oy },
                        radiusX: 3.4,
                        radiusY: 3.4,
                    },
                    &self.solid_brush,
                );
            }
        }

        const LABELS: [&str; 5] = ["1", "2", "3", "4", "Long"];
        for (i, &lx) in label_xs.iter().enumerate() {
            let is_on = (i < 4 && active_seg == i * 2) || (i == 4 && active_seg == 7);
            let is_past = (i < 4 && i * 2 < active_seg) || (i == 4 && active_seg > 7);
            let col = if is_on {
                (1.0, 1.0, 1.0, 0.98 * alpha)
            } else if is_past {
                (0.78, 0.76, 0.92, 0.82 * alpha)
            } else {
                (0.58, 0.58, 0.58, 0.95 * alpha)
            };
            self.draw_text(
                LABELS[i],
                &self.fmt_label_xs_center,
                &D2D_RECT_F {
                    left: lx - 14.0,
                    top: 109.0,
                    right: lx + 14.0,
                    bottom: 123.0,
                },
                col,
            );
        }

        let total_mins = pomo.total_round_mins();
        let dur_txt = format!(
            "{} h {} min · 4 sessions",
            total_mins / 60,
            total_mins % 60
        );
        self.draw_text(
            &dur_txt,
            &self.fmt_label_xs,
            &D2D_RECT_F {
                left: bl + 193.0,
                top: 126.0,
                right: bl + 365.0,
                bottom: 140.0,
            },
            (0.54, 0.54, 0.54, 1.0 * alpha),
        );
    }

    unsafe fn draw_water_page(
        &self,
        springs: &NotchSprings,
        water: &WaterState,
        layout: &ExpandedLayout,
        dx: f32,
        alpha: f32,
    ) {
        let brush: &ID2D1Brush = &self.solid_brush;
        let bl = layout.body_left + dx;

        let taken_l = format!("{:.1}", water.taken_ml as f32 / 1000.0);
        let goal_l = format!(" / {:.1} L", water.goal_ml as f32 / 1000.0);
        let taken_w = self.measure_width(&taken_l, &self.fmt_timer_lg);
        self.draw_text(
            &taken_l,
            &self.fmt_timer_lg,
            &D2D_RECT_F {
                left: bl + 32.0,
                top: 46.0,
                right: bl + 32.0 + taken_w + 6.0,
                bottom: 86.0,
            },
            (0.957, 0.957, 0.965, 1.0 * alpha),
        );
        self.draw_text(
            &goal_l,
            &self.fmt_artist,
            &D2D_RECT_F {
                left: bl + 32.0 + taken_w,
                top: 59.0,
                right: bl + 148.0,
                bottom: 83.0,
            },
            (0.60, 0.60, 0.60, 1.0 * alpha),
        );

        let (switch_rect, btn_drink_raw, btn_reset_raw) = water_controls(bl);
        let sw_t = springs.water_switch_on.value().clamp(0.0, 1.0);
        let sw_h = springs.slot_hover(14);
        let sw_scale = 1.0_f32;
        let switch_scaled = switch_rect;
        let sw_rrect = D2D1_ROUNDED_RECT {
            rect: switch_scaled,
            radiusX: 10.0 * sw_scale,
            radiusY: 10.0 * sw_scale,
        };
        self.set_color(
            (0.227 + (0.149 - 0.227) * sw_t + 0.04 * sw_h).min(1.0),
            (0.227 + (0.600 - 0.227) * sw_t + 0.04 * sw_h).min(1.0),
            (0.251 + (1.0 - 0.251) * sw_t + 0.04 * sw_h).min(1.0),
            alpha,
        );
        self.d2d_context.FillRoundedRectangle(&sw_rrect, &self.solid_brush);

        let thumb_cx =
            (switch_scaled.left + 10.0 * sw_scale)
                + (switch_scaled.right - switch_scaled.left - 20.0 * sw_scale) * sw_t;
        let thumb_cy = (switch_scaled.top + switch_scaled.bottom) * 0.5;
        self.set_color(1.0, 1.0, 1.0, 0.98 * alpha);
        self.d2d_context.FillEllipse(
            &D2D1_ELLIPSE {
                point: D2D_POINT_2F { x: thumb_cx, y: thumb_cy },
                radiusX: 8.2 * sw_scale,
                radiusY: 8.2 * sw_scale,
            },
            &self.solid_brush,
        );
        self.set_color(
            0.38 + (0.149 - 0.38) * sw_t,
            0.38 + (0.600 - 0.38) * sw_t,
            0.40 + (1.0 - 0.40) * sw_t,
            0.96 * alpha,
        );
        icons::draw_bell(&self.d2d_context, thumb_cx, thumb_cy, 12.2 * sw_scale, brush, None);

        let pct = ((water.progress() * 100.0).round() as u32).min(999);
        let status_suffix = if !water.reminder_enabled {
            " · Reminders off"
        } else if pct >= 100 {
            " · Goal reached"
        } else {
            " · Stay hydrated"
        };
        let prefix = "Today · ";
        let pct_str = format!("{pct}%");
        let w_pre = self.measure_width(prefix, &self.fmt_artist);
        let w_pct = self.measure_width(&pct_str, &self.fmt_artist);
        self.draw_text(
            prefix,
            &self.fmt_artist,
            &D2D_RECT_F {
                left: bl + 33.0,
                top: 86.0,
                right: bl + 33.0 + w_pre + 4.0,
                bottom: 103.0,
            },
            (0.60, 0.60, 0.60, 1.0 * alpha),
        );
        let pct_col = (
            0.70 + (0.310 - 0.70) * sw_t,
            0.70 + (0.753 - 0.70) * sw_t,
            0.73 + (1.0 - 0.73) * sw_t,
            0.98 * alpha,
        );
        self.draw_text(
            &pct_str,
            &self.fmt_artist,
            &D2D_RECT_F {
                left: bl + 33.0 + w_pre,
                top: 86.0,
                right: bl + 33.0 + w_pre + w_pct + 4.0,
                bottom: 103.0,
            },
            pct_col,
        );
        self.draw_text(
            status_suffix,
            &self.fmt_artist,
            &D2D_RECT_F {
                left: bl + 33.0 + w_pre + w_pct,
                top: 86.0,
                right: bl + 194.0,
                bottom: 103.0,
            },
            (0.60, 0.60, 0.60, 1.0 * alpha),
        );

        let bar_rect = D2D_RECT_F {
            left: bl + 33.0,
            top: 108.0,
            right: bl + 183.0,
            bottom: 113.0,
        };
        self.set_color(1.0, 1.0, 1.0, 0.14 * alpha);
        self.d2d_context.FillRoundedRectangle(
            &D2D1_ROUNDED_RECT {
                rect: bar_rect,
                radiusX: 2.5,
                radiusY: 2.5,
            },
            &self.solid_brush,
        );
        let w_fill = 150.0 * springs.widget_water_fill.value().clamp(0.0, 1.0);
        if w_fill > 1.0 {
            self.fill_linear_gradient_rrect(
                &D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: bar_rect.left,
                        top: bar_rect.top,
                        right: bar_rect.left + w_fill,
                        bottom: bar_rect.bottom,
                    },
                    radiusX: 2.5,
                    radiusY: 2.5,
                },
                (
                    0.50 + (0.400 - 0.50) * sw_t,
                    0.50 + (0.859 - 0.50) * sw_t,
                    0.52 + (1.0 - 0.52) * sw_t,
                    0.98 * alpha,
                ),
                (
                    0.36 + (0.176 - 0.36) * sw_t,
                    0.36 + (0.420 - 0.36) * sw_t,
                    0.38 + (1.0 - 0.38) * sw_t,
                    0.98 * alpha,
                ),
            );
        }

        let drink_h = springs.slot_hover(12);
        let drink_scale = springs.slot_scale(12, 0.04, 0.07);
        let btn_drink = scale_rect(btn_drink_raw, drink_scale);
        let drink_rrect = D2D1_ROUNDED_RECT {
            rect: btn_drink,
            radiusX: 13.0 * drink_scale,
            radiusY: 13.0 * drink_scale,
        };
        self.set_color(
            (0.072 + (0.055 - 0.072) * sw_t + 0.032 * drink_h).min(1.0),
            (0.072 + (0.285 - 0.072) * sw_t + (0.032 + 0.04 * sw_t) * drink_h).min(1.0),
            (0.072 + (0.495 - 0.072) * sw_t + (0.032 + 0.06 * sw_t) * drink_h).min(1.0),
            0.98 * alpha,
        );
        self.d2d_context
            .FillRoundedRectangle(&drink_rrect, &self.solid_brush);
        self.set_color(
            1.0 + (0.220 - 1.0) * sw_t,
            1.0 + (0.741 - 1.0) * sw_t,
            1.0 + (0.973 - 1.0) * sw_t,
            (0.08 + (0.28 - 0.08) * sw_t + 0.08 * drink_h) * alpha,
        );
        self.d2d_context
            .DrawRoundedRectangle(&drink_rrect, &self.solid_brush, 0.9, None);
        self.set_color(
            0.48 + (0.35 - 0.48) * sw_t,
            0.48 + (0.80 - 0.48) * sw_t,
            0.50 + (1.0 - 0.50) * sw_t,
            0.98 * alpha,
        );
        icons::draw_water_drop_filled(
            &self.d2d_context,
            btn_drink.left + 15.5 * drink_scale,
            (btn_drink.top + btn_drink.bottom) * 0.5,
            15.5 * drink_scale,
            brush,
        );
        self.draw_text(
            "Drink",
            &self.fmt_pill_btn,
            &D2D_RECT_F {
                left: btn_drink.left + 22.5 * drink_scale,
                top: btn_drink.top,
                right: btn_drink.right - 4.0,
                bottom: btn_drink.bottom,
            },
            (
                0.68 + (0.957 - 0.68) * sw_t,
                0.68 + (0.957 - 0.68) * sw_t,
                0.71 + (0.965 - 0.71) * sw_t,
                1.0 * alpha,
            ),
        );

        let reset_h = springs.slot_hover(13);
        let reset_scale = springs.slot_scale(13, 0.0, 0.05);
        let btn_reset = scale_rect(btn_reset_raw, reset_scale);
        let reset_rrect = D2D1_ROUNDED_RECT {
            rect: btn_reset,
            radiusX: 13.0 * reset_scale,
            radiusY: 13.0 * reset_scale,
        };
        self.set_color(
            0.072 + 0.032 * reset_h,
            0.072 + 0.032 * reset_h,
            0.072 + 0.032 * reset_h,
            0.98 * alpha,
        );
        self.d2d_context
            .FillRoundedRectangle(&reset_rrect, &self.solid_brush);
        self.set_color(1.0, 1.0, 1.0, (0.08 + 0.07 * reset_h) * alpha);
        self.d2d_context
            .DrawRoundedRectangle(&reset_rrect, &self.solid_brush, 0.85, None);
        self.set_color(0.957, 0.957, 0.965, 0.94 * alpha);
        icons::draw_sliders_h(
            &self.d2d_context,
            btn_reset.left + 16.0 * reset_scale,
            (btn_reset.top + btn_reset.bottom) * 0.5,
            15.0 * reset_scale,
            brush,
        );
        self.draw_text(
            "Reset",
            &self.fmt_pill_btn,
            &D2D_RECT_F {
                left: btn_reset.left + 23.5 * reset_scale,
                top: btn_reset.top,
                right: btn_reset.right - 4.0,
                bottom: btn_reset.bottom,
            },
            (0.957, 0.957, 0.965, 1.0 * alpha),
        );

        self.set_color(1.0, 1.0, 1.0, 0.12 * alpha);
        self.d2d_context.DrawLine(
            D2D_POINT_2F { x: bl + 195.0, y: 52.0 },
            D2D_POINT_2F { x: bl + 195.0, y: 143.0 },
            &self.solid_brush,
            1.0,
            None,
        );

        let chart_left = bl + 208.0;
        let chart_right = bl + 322.0;
        let chart_top = 66.0;
        let chart_bottom = 122.0;
        let chart_h = chart_bottom - chart_top;
        let gauge_t = springs.water_view_gauge.value().clamp(0.0, 1.0);
        let bars_a = alpha * (1.0 - gauge_t);
        let gauge_a = alpha * gauge_t;

        let dot_left_r = 1.8 + 0.6 * gauge_t;
        let dot_right_r = 2.4 - 0.6 * gauge_t;
        self.set_color(1.0, 1.0, 1.0, (0.30 + 0.64 * gauge_t) * alpha);
        self.d2d_context.FillEllipse(
            &D2D1_ELLIPSE {
                point: D2D_POINT_2F {
                    x: chart_right + 16.0,
                    y: 52.0,
                },
                radiusX: dot_left_r,
                radiusY: dot_left_r,
            },
            &self.solid_brush,
        );
        self.set_color(1.0, 1.0, 1.0, (0.94 - 0.64 * gauge_t) * alpha);
        self.d2d_context.FillEllipse(
            &D2D1_ELLIPSE {
                point: D2D_POINT_2F {
                    x: chart_right + 24.0,
                    y: 52.0,
                },
                radiusX: dot_right_r,
                radiusY: dot_right_r,
            },
            &self.solid_brush,
        );

        if bars_a > 0.01 {
            let b_dx = 10.0 * gauge_t;
            let cl = chart_left + b_dx;
            let cr = chart_right + b_dx;

            self.set_color(1.0, 1.0, 1.0, 0.22 * bars_a);
            for gy in [chart_top, chart_top + chart_h * 0.5] {
                self.d2d_context.DrawLine(
                    D2D_POINT_2F { x: cl, y: gy },
                    D2D_POINT_2F { x: cr + 4.0, y: gy },
                    &self.solid_brush,
                    1.1,
                    self.dotted_stroke.as_ref(),
                );
            }

            self.set_color(
                0.40 + (0.314 - 0.40) * sw_t,
                0.40 + (0.706 - 0.40) * sw_t,
                0.42 + (1.0 - 0.42) * sw_t,
                (0.32 + 0.13 * sw_t) * bars_a,
            );
            self.d2d_context.DrawLine(
                D2D_POINT_2F { x: cl - 2.0, y: chart_bottom },
                D2D_POINT_2F { x: cr + 6.0, y: chart_bottom },
                &self.solid_brush,
                1.0,
                None,
            );
            self.set_color(1.0, 1.0, 1.0, 0.18 * bars_a);
            self.d2d_context.DrawLine(
                D2D_POINT_2F { x: cr + 6.0, y: chart_top - 4.0 },
                D2D_POINT_2F { x: cr + 6.0, y: chart_bottom },
                &self.solid_brush,
                1.0,
                None,
            );

            let (visible_bars, label_hours) = water.visible_window();
            let max_ml = if visible_bars.iter().copied().max().unwrap_or(0) > 400 {
                800.0_f32
            } else {
                400.0_f32
            };
            let top_lbl = if max_ml > 400.0 { "800" } else { "400" };
            let mid_lbl = if max_ml > 400.0 { "400" } else { "200" };
            for (lbl, ly) in [
                (top_lbl, chart_top),
                (mid_lbl, chart_top + chart_h * 0.5),
                ("0 ml", chart_bottom),
            ] {
                self.draw_text(
                    lbl,
                    &self.fmt_label_xs,
                    &D2D_RECT_F {
                        left: cr + 11.0,
                        top: ly - 7.0,
                        right: cr + 42.0,
                        bottom: ly + 7.0,
                    },
                    (0.54, 0.54, 0.54, 1.0 * bars_a),
                );
            }

            for (i, &ml) in visible_bars.iter().enumerate() {
                let bx = cl + 4.0 + i as f32 * 14.0;
                let track_rrect = D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: bx,
                        top: chart_top,
                        right: bx + 7.5,
                        bottom: chart_bottom - 1.5,
                    },
                    radiusX: 3.75,
                    radiusY: 3.75,
                };
                self.set_color(0.135, 0.135, 0.145, 0.62 * bars_a);
                self.d2d_context
                    .FillRoundedRectangle(&track_rrect, &self.solid_brush);

                let bar_t = springs.water_hourly[i].value().clamp(0.0, 1.0);
                let bh = (bar_t * (chart_h - 1.5))
                    .max(if bar_t > 0.004 || ml > 0 { 5.0 * (bar_t * 6.0).min(1.0) } else { 0.0 });
                if bh > 0.5 {
                    let rrect = D2D1_ROUNDED_RECT {
                        rect: D2D_RECT_F {
                            left: bx,
                            top: chart_bottom - 1.5 - bh,
                            right: bx + 7.5,
                            bottom: chart_bottom - 1.5,
                        },
                        radiusX: 3.75,
                        radiusY: 3.75,
                    };
                    self.fill_linear_gradient_rrect(
                        &rrect,
                        (
                            0.48 + (0.36 - 0.48) * sw_t,
                            0.48 + (0.80 - 0.48) * sw_t,
                            0.50 + (1.0 - 0.50) * sw_t,
                            0.72 * bars_a,
                        ),
                        (
                            0.32 + (0.18 - 0.32) * sw_t,
                            0.32 + (0.46 - 0.32) * sw_t,
                            0.34 + (1.0 - 0.34) * sw_t,
                            0.70 * bars_a,
                        ),
                    );
                }
            }

            for (hour, idx) in [
                (label_hours[0], 0usize),
                (label_hours[1], 3usize),
                (label_hours[2], 6usize),
            ] {
                let lbl = format!("{hour:02}");
                let cx = cl + 7.5 + idx as f32 * 14.0;
                self.draw_text(
                    &lbl,
                    &self.fmt_label_xs_center,
                    &D2D_RECT_F {
                        left: cx - 12.0,
                        top: chart_bottom + 4.0,
                        right: cx + 12.0,
                        bottom: chart_bottom + 18.0,
                    },
                    (0.54, 0.54, 0.54, 1.0 * bars_a),
                );
            }
        }

        if gauge_a > 0.01 {
            let g_dx = -10.0 * (1.0 - gauge_t);
            let gcx = bl + 282.0 + g_dx;
            let gcy = 98.0;
            let gr = 33.5 * (0.94 + 0.06 * gauge_t);
            let stroke_w = 5.2;
            let start_deg = 135.0_f32;
            let total_sweep = 270.0_f32;

            let round_stroke = ID2D1Factory::CreateStrokeStyle(
                &self.d2d_factory,
                &D2D1_STROKE_STYLE_PROPERTIES {
                    startCap: D2D1_CAP_STYLE_ROUND,
                    endCap: D2D1_CAP_STYLE_ROUND,
                    lineJoin: D2D1_LINE_JOIN_ROUND,
                    ..Default::default()
                },
                None,
            )
            .ok();

            let build_arc = |sweep_deg: f32| -> Option<(ID2D1PathGeometry, D2D_POINT_2F)> {
                let sweep = sweep_deg.clamp(0.5, 359.0);
                let a0 = start_deg.to_radians();
                let a1 = (start_deg + sweep).to_radians();
                let p0 = D2D_POINT_2F {
                    x: gcx + gr * a0.cos(),
                    y: gcy + gr * a0.sin(),
                };
                let p1 = D2D_POINT_2F {
                    x: gcx + gr * a1.cos(),
                    y: gcy + gr * a1.sin(),
                };
                let geo = self.d2d_factory.CreatePathGeometry().ok()?;
                let sink = geo.Open().ok()?;
                sink.BeginFigure(p0, windows::Win32::Graphics::Direct2D::Common::D2D1_FIGURE_BEGIN_HOLLOW);
                sink.AddArc(&D2D1_ARC_SEGMENT {
                    point: p1,
                    size: D2D_SIZE_F {
                        width: gr,
                        height: gr,
                    },
                    rotationAngle: 0.0,
                    sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                    arcSize: if sweep > 180.0 {
                        windows::Win32::Graphics::Direct2D::D2D1_ARC_SIZE_LARGE
                    } else {
                        D2D1_ARC_SIZE_SMALL
                    },
                });
                sink.EndFigure(windows::Win32::Graphics::Direct2D::Common::D2D1_FIGURE_END_OPEN);
                sink.Close().ok()?;
                Some((geo.into(), p1))
            };

            if let Some((track_geo, _)) = build_arc(total_sweep) {
                self.set_color(0.095, 0.095, 0.098, 0.96 * gauge_a);
                self.d2d_context.DrawGeometry(
                    &track_geo,
                    &self.solid_brush,
                    stroke_w,
                    round_stroke.as_ref(),
                );
            }

            let prog = springs.widget_water_fill.value().clamp(0.0, 1.0);
            if prog > 0.005 {
                let sweep = (total_sweep * prog).max(2.0);
                if let Some((fill_geo, end_pt)) = build_arc(sweep) {
                    let c_left = (
                        0.52 + (0.42 - 0.52) * sw_t,
                        0.52 + (0.90 - 0.52) * sw_t,
                        0.54 + (1.0 - 0.54) * sw_t,
                        0.98 * gauge_a,
                    );
                    let c_right = (
                        0.36 + (0.14 - 0.36) * sw_t,
                        0.36 + (0.48 - 0.36) * sw_t,
                        0.38 + (1.0 - 0.38) * sw_t,
                        0.98 * gauge_a,
                    );
                    let stops = [
                        D2D1_GRADIENT_STOP {
                            position: 0.0,
                            color: D2D1_COLOR_F {
                                r: c_left.0,
                                g: c_left.1,
                                b: c_left.2,
                                a: c_left.3,
                            },
                        },
                        D2D1_GRADIENT_STOP {
                            position: 1.0,
                            color: D2D1_COLOR_F {
                                r: c_right.0,
                                g: c_right.1,
                                b: c_right.2,
                                a: c_right.3,
                            },
                        },
                    ];
                    if let Ok(sc) = ID2D1RenderTarget::CreateGradientStopCollection(
                        &self.d2d_context,
                        &stops,
                        D2D1_GAMMA_2_2,
                        D2D1_EXTEND_MODE_CLAMP,
                    ) {
                        let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                            startPoint: D2D_POINT_2F { x: gcx - gr, y: gcy - gr * 0.5 },
                            endPoint: D2D_POINT_2F { x: gcx + gr, y: gcy + gr * 0.5 },
                        };
                        if let Ok(lin) =
                            self.d2d_context.CreateLinearGradientBrush(&props, None, &sc)
                        {
                            self.d2d_context.DrawGeometry(
                                &fill_geo,
                                &lin,
                                stroke_w,
                                round_stroke.as_ref(),
                            );
                        }
                    }

                    self.set_color(0.05, 0.07, 0.10, 0.96 * gauge_a);
                    self.d2d_context.FillEllipse(
                        &D2D1_ELLIPSE {
                            point: end_pt,
                            radiusX: 4.4,
                            radiusY: 4.4,
                        },
                        &self.solid_brush,
                    );
                    self.set_color(
                        0.55 + (0.40 - 0.55) * sw_t,
                        0.55 + (0.86 - 0.55) * sw_t,
                        0.57 + (1.0 - 0.57) * sw_t,
                        0.98 * gauge_a,
                    );
                    self.d2d_context.FillEllipse(
                        &D2D1_ELLIPSE {
                            point: end_pt,
                            radiusX: 3.1,
                            radiusY: 3.1,
                        },
                        &self.solid_brush,
                    );
                }
            }

            let num_str = format!("{pct}");
            self.draw_text(
                &num_str,
                &self.fmt_gauge_pct,
                &D2D_RECT_F {
                    left: gcx - 32.0,
                    top: gcy - 18.0,
                    right: gcx + 32.0,
                    bottom: gcy + 10.0,
                },
                (0.96, 0.96, 0.98, 1.0 * gauge_a),
            );
            self.set_color(
                0.50 + (0.36 - 0.50) * sw_t,
                0.50 + (0.82 - 0.50) * sw_t,
                0.52 + (1.0 - 0.52) * sw_t,
                0.98 * gauge_a,
            );
            icons::draw_water_drop_filled(&self.d2d_context, gcx - 6.5, gcy + 16.5, 9.5, brush);
            self.draw_text(
                "%",
                &self.fmt_artist,
                &D2D_RECT_F {
                    left: gcx + 0.5,
                    top: gcy + 9.0,
                    right: gcx + 18.0,
                    bottom: gcy + 24.0,
                },
                (0.86, 0.86, 0.88, 1.0 * gauge_a),
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn draw_shelf_page(
        &self,
        shelf: &ShelfStore,
        springs: &NotchSprings,
        layout: &ExpandedLayout,
        dx: f32,
        drag_over_amt: f32,
        alpha: f32,
        notch_geo: Option<&ID2D1PathGeometry>,
    ) {
        let drop_t = springs.shelf_drop_t;
        let sh_hov = springs.shelf_hover.value().max(drag_over_amt).clamp(0.0, 1.05);
        let px_local = if drop_t >= 0.0 {
            springs.shelf_drop_x
        } else {
            springs.cursor_x.value
        };
        let px = px_local + dx;

        if sh_hov > 0.005 || drop_t >= 0.0 {
            let rise = if drop_t >= 0.0 && drop_t < 1.1 {
                ((drop_t / 1.1) * std::f32::consts::PI).sin().max(0.0)
            } else {
                0.0
            };
            let fade = if drop_t >= 0.0 {
                if drop_t < 0.9 {
                    1.0
                } else {
                    let k = ((drop_t - 0.9) / 0.6).clamp(0.0, 1.0);
                    1.0 - k * k * (3.0 - 2.0 * k)
                }
            } else {
                1.0
            };
            let base_glow = (0.26 * sh_hov + 0.34 * rise) * fade * alpha;
            let pool_glow = (0.42 * sh_hov + 0.28 * rise) * fade * alpha;
            if base_glow > 0.004 || pool_glow > 0.004 {
                self.draw_shelf_ambient_glow(
                    layout.cx + dx,
                    px,
                    layout.base_h,
                    base_glow,
                    pool_glow,
                    rise,
                );
            }
        }

        let cb_t = springs.shelf_tab_clipboard.value().clamp(0.0, 1.0);
        let files_alpha = alpha * (1.0 - cb_t).clamp(0.0, 1.0);
        if files_alpha > 0.01 {
            self.draw_shelf_files_tab(
                shelf,
                springs,
                layout,
                dx - 18.0 * cb_t,
                drag_over_amt,
                sh_hov,
                files_alpha,
            );
        }

        let clip_alpha = alpha * cb_t;
        if clip_alpha > 0.01 {
            self.draw_shelf_clipboard_tab(
                shelf,
                springs,
                layout,
                dx + 18.0 * (1.0 - cb_t),
                clip_alpha,
            );
        }

        if (0.0..1.45).contains(&drop_t) {
            self.draw_shelf_drop_beam(px, layout.base_h, drop_t, alpha, notch_geo);
        }
    }

    unsafe fn draw_centered_x_badge(
        &self,
        center: D2D_POINT_2F,
        radius: f32,
        arm: f32,
        stroke_w: f32,
        card_hover: f32,
        del_hover: f32,
        item_alpha: f32,
    ) {
        let a = card_hover * item_alpha;
        let bg_r = 0.173 + (0.953 - 0.173) * del_hover;
        let bg_g = 0.173 + (0.255 - 0.173) * del_hover;
        let bg_b = 0.188 + (0.227 - 0.188) * del_hover;
        let bg_a = (0.90 + 0.08 * del_hover) * a;
        let ell = D2D1_ELLIPSE {
            point: center,
            radiusX: radius,
            radiusY: radius,
        };
        self.set_color(bg_r, bg_g, bg_b, bg_a);
        self.d2d_context.FillEllipse(&ell, &self.solid_brush);
        self.set_color(
            1.0,
            1.0,
            1.0,
            (0.18 * (1.0 - del_hover) + 0.10 * del_hover) * a,
        );
        self.d2d_context
            .DrawEllipse(&ell, &self.solid_brush, 0.75, None);
        let round_stroke = ID2D1Factory::CreateStrokeStyle(
            &self.d2d_factory,
            &D2D1_STROKE_STYLE_PROPERTIES {
                startCap: D2D1_CAP_STYLE_ROUND,
                endCap: D2D1_CAP_STYLE_ROUND,
                lineJoin: D2D1_LINE_JOIN_ROUND,
                ..Default::default()
            },
            None,
        )
        .ok();
        let fg_v = 0.84 + 0.16 * del_hover;
        self.set_color(fg_v, fg_v, fg_v + 0.02 * (1.0 - del_hover), 0.96 * a);
        self.d2d_context.DrawLine(
            D2D_POINT_2F {
                x: center.x - arm,
                y: center.y - arm,
            },
            D2D_POINT_2F {
                x: center.x + arm,
                y: center.y + arm,
            },
            &self.solid_brush,
            stroke_w,
            round_stroke.as_ref(),
        );
        self.d2d_context.DrawLine(
            D2D_POINT_2F {
                x: center.x + arm,
                y: center.y - arm,
            },
            D2D_POINT_2F {
                x: center.x - arm,
                y: center.y + arm,
            },
            &self.solid_brush,
            stroke_w,
            round_stroke.as_ref(),
        );
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn draw_shelf_files_tab(
        &self,
        shelf: &ShelfStore,
        springs: &NotchSprings,
        layout: &ExpandedLayout,
        dx: f32,
        drag_over_amt: f32,
        sh_hov: f32,
        alpha: f32,
    ) {
        let brush: &ID2D1Brush = &self.solid_brush;
        let empty_t = springs.shelf_empty_reveal.value().clamp(0.0, 1.0);
        let items_fade = (1.0 - empty_t * 0.85).clamp(0.0, 1.0);
        let bl = layout.body_left + dx;
        let br = layout.body_right + dx;
        let mut prev_xf = Matrix3x2::identity();
        self.d2d_context.GetTransform(&mut prev_xf);

        if !shelf.items.is_empty() && items_fade > 0.01 {
            let items_alpha = alpha * items_fade;
            let btn_fade = springs.shelf_clear_overall_progress().map_or(1.0, |ov| {
                if ov > 0.75 {
                    ((1.0 - ov) / 0.25).clamp(0.0, 1.0)
                } else {
                    1.0
                }
            });

            let files_clip = D2D_RECT_F {
                left: bl + 24.0,
                top: 48.0,
                right: br - 24.0,
                bottom: 139.0,
            };
            self.d2d_context
                .PushAxisAlignedClip(&files_clip, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);

            let mut active_file_idx = 0usize;
            for (idx, item) in shelf.items.iter().enumerate() {
                let slot_pos = if item.slot_x_anim >= 0.0 {
                    item.slot_x_anim
                } else {
                    active_file_idx as f32
                };
                let cur_active_idx = active_file_idx;
                if !item.removing {
                    active_file_idx += 1;
                }
                let rel = slot_pos - shelf.files_scroll_anim;
                if !(-1.15..=5.25).contains(&rel) {
                    continue;
                }
                let cx = bl + 59.5 + rel * 68.5;
                let card_raw = D2D_RECT_F {
                    left: cx - 28.0,
                    top: 53.0,
                    right: cx + 28.0,
                    bottom: 134.0,
                };
                let in_vis = !item.removing
                    && cur_active_idx >= shelf.files_scroll_offset
                    && cur_active_idx < shelf.files_scroll_offset + 5;
                let vis_slot = (cur_active_idx as isize - shelf.files_scroll_offset as isize)
                    .clamp(0, 4) as usize;
                let del_h = if in_vis { springs.slot_hover(24 + vis_slot) } else { 0.0 };
                let card_h = if in_vis {
                    springs.slot_hover(19 + vis_slot).max(del_h)
                } else {
                    0.0
                };
                let card_scale = 1.0_f32;
                let spawn_v = springs.card_spawn[idx.min(4)].value;
                let spawn_y = -(1.0 - spawn_v) * 32.0;
                let spawn_alpha = (spawn_v * 1.4).clamp(0.0, 1.0);
                if spawn_alpha <= 0.005 {
                    continue;
                }

                let rem_t = if item.removing {
                    item.remove_anim.clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let rem_fade = (1.0 - rem_t).powf(1.35);
                let clear_p = springs.shelf_item_clear_progress(vis_slot);
                let item_alpha = if let Some(p) = clear_p {
                    let clear_fade = if p < 0.72 {
                        1.0
                    } else {
                        ((1.0 - p) / 0.28).clamp(0.0, 1.0)
                    };
                    items_alpha * spawn_alpha * clear_fade * rem_fade
                } else {
                    items_alpha * spawn_alpha * rem_fade
                };
                if item_alpha <= 0.005 {
                    continue;
                }

                let mut pushed_xf = false;
                if let Some(p) = clear_p {
                    let reach = (p / 0.92).clamp(0.0, 1.0);
                    let pull = reach * reach * (0.22 + 0.78 * reach);
                    let arc_y = -10.0 * 4.0 * reach * (1.0 - reach);
                    let s = (1.0 - 0.90 * p.powf(1.05)).clamp(0.08, 1.0);
                    let base_cx = (card_raw.left + card_raw.right) * 0.5;
                    let base_cy = (card_raw.top + card_raw.bottom) * 0.5 + spawn_y;
                    let target_x = br - 58.0;
                    let target_y = 148.0;
                    let dst_cx = base_cx + (target_x - base_cx) * pull;
                    let dst_cy = base_cy + (target_y - base_cy) * pull + arc_y;
                    let suction = Matrix3x2 {
                        M11: s,
                        M12: 0.0,
                        M21: 0.0,
                        M22: s,
                        M31: dst_cx - base_cx * s,
                        M32: dst_cy - base_cy * s,
                    };
                    let combined = mul_matrix3x2(suction, prev_xf);
                    self.d2d_context.SetTransform(&combined);
                    pushed_xf = true;
                } else if rem_t > 0.001 {
                    let s = 1.0 - 0.26 * rem_t;
                    let base_cx = (card_raw.left + card_raw.right) * 0.5;
                    let base_cy = (card_raw.top + card_raw.bottom) * 0.5 + spawn_y;
                    let dy_rem = -5.0 * rem_t;
                    let xf = Matrix3x2 {
                        M11: s,
                        M12: 0.0,
                        M21: 0.0,
                        M22: s,
                        M31: base_cx - base_cx * s,
                        M32: (base_cy + dy_rem) - base_cy * s,
                    };
                    let combined = mul_matrix3x2(xf, prev_xf);
                    self.d2d_context.SetTransform(&combined);
                    pushed_xf = true;
                }

                let mut card = scale_rect(card_raw, card_scale);
                card.top += spawn_y;
                card.bottom += spawn_y;

                if card_h > 0.01 {
                    self.set_color(1.0, 1.0, 1.0, 0.07 * card_h * item_alpha);
                    self.d2d_context.FillRoundedRectangle(
                        &D2D1_ROUNDED_RECT {
                            rect: card,
                            radiusX: 10.0,
                            radiusY: 10.0,
                        },
                        &self.solid_brush,
                    );
                }

                let lift_y = -2.0 * card_h;
                let icon_cx = (card_raw.left + card_raw.right) * 0.5;
                let icon_cy = card_raw.top + 24.0 + lift_y + spawn_y;
                let half = 21.0 * card_scale;
                let icon_rect = D2D_RECT_F {
                    left: icon_cx - half,
                    top: icon_cy - half,
                    right: icon_cx + half,
                    bottom: icon_cy + half,
                };

                if let Some(bmp) = self.cached_shelf_icon(item) {
                    self.d2d_context.DrawBitmap(
                        bmp,
                        Some(&icon_rect),
                        item_alpha,
                        D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
                        None,
                        None,
                    );
                } else if item.is_dir {
                    icons::draw_folder_thumb(&self.d2d_context, icon_rect, item_alpha);
                } else {
                    icons::draw_file_thumb(
                        &self.d2d_context,
                        icon_rect,
                        is_image_filename(&item.name),
                        item_alpha,
                    );
                }

                if !shelf.select_mode && card_h > 0.02 {
                    let del_c = D2D_POINT_2F {
                        x: card.right - 7.5,
                        y: card.top + 7.5,
                    };
                    let bs = (0.72 + 0.28 * card_h) * (1.0 + 0.08 * del_h);
                    self.draw_centered_x_badge(
                        del_c,
                        6.0 * bs,
                        2.15 * bs,
                        1.35,
                        card_h,
                        del_h,
                        item_alpha,
                    );
                }

                let sel_mode_t = springs.shelf_select_mode.value().clamp(0.0, 1.0);
                if sel_mode_t > 0.01 {
                    let bx = icon_rect.right - 5.0;
                    let by = icon_rect.top + 5.0;
                    let s_rad = 6.5 * (0.65 + 0.35 * sel_mode_t);
                    let sa = item_alpha * sel_mode_t;
                    if shelf.is_item_selected(idx) {
                        self.set_color(0.231, 0.510, 0.965, 0.98 * sa);
                        self.d2d_context.FillEllipse(
                            &D2D1_ELLIPSE {
                                point: D2D_POINT_2F { x: bx, y: by },
                                radiusX: s_rad,
                                radiusY: s_rad,
                            },
                            &self.solid_brush,
                        );
                        self.set_color(1.0, 1.0, 1.0, 0.98 * sa);
                        icons::draw_checkmark(&self.d2d_context, bx, by, 7.5 * (0.65 + 0.35 * sel_mode_t), brush, 1.6);
                    } else {
                        self.set_color(0.0, 0.0, 0.0, 0.45 * sa);
                        self.d2d_context.FillEllipse(
                            &D2D1_ELLIPSE {
                                point: D2D_POINT_2F { x: bx, y: by },
                                radiusX: s_rad,
                                radiusY: s_rad,
                            },
                            &self.solid_brush,
                        );
                        self.set_color(1.0, 1.0, 1.0, 0.65 * sa);
                        self.d2d_context.DrawEllipse(
                            &D2D1_ELLIPSE {
                                point: D2D_POINT_2F { x: bx, y: by },
                                radiusX: s_rad,
                                radiusY: s_rad,
                            },
                            &self.solid_brush,
                            1.2,
                            None,
                        );
                    }
                }

                self.draw_text(
                    &item.name,
                    &self.fmt_card_name,
                    &D2D_RECT_F {
                        left: card.left - 3.0,
                        top: card_raw.top + 52.0 + spawn_y,
                        right: card.right + 3.0,
                        bottom: card_raw.top + 67.0 + spawn_y,
                    },
                    (0.953, 0.957, 0.965, 1.0 * item_alpha),
                );
                let sz = item.format_size();
                self.draw_text(
                    &sz,
                    &self.fmt_card_size,
                    &D2D_RECT_F {
                        left: card.left - 2.0,
                        top: card_raw.top + 68.0 + spawn_y,
                        right: card.right + 2.0,
                        bottom: card_raw.top + 81.0 + spawn_y,
                    },
                    (0.52, 0.52, 0.54, 1.0 * item_alpha),
                );

                if pushed_xf {
                    self.d2d_context.SetTransform(&prev_xf);
                }
            }

            self.d2d_context.PopAxisAlignedClip();

            if shelf.items.len() > 5 {
                let track_l = bl + 130.0;
                let track_w = 134.0_f32;
                let max_off = (shelf.items.len() - 5) as f32;
                let thumb_w = (track_w * (5.0 / shelf.items.len() as f32)).clamp(24.0, 96.0);
                let frac = (shelf.files_scroll_anim / max_off).clamp(0.0, 1.0);
                let thumb_l = track_l + (track_w - thumb_w) * frac;
                self.set_color(1.0, 1.0, 1.0, 0.12 * items_alpha * btn_fade);
                self.d2d_context.FillRoundedRectangle(
                    &D2D1_ROUNDED_RECT {
                        rect: D2D_RECT_F {
                            left: track_l,
                            top: 146.5,
                            right: track_l + track_w,
                            bottom: 149.5,
                        },
                        radiusX: 1.5,
                        radiusY: 1.5,
                    },
                    &self.solid_brush,
                );
                self.set_color(1.0, 1.0, 1.0, 0.48 * items_alpha * btn_fade);
                self.d2d_context.FillRoundedRectangle(
                    &D2D1_ROUNDED_RECT {
                        rect: D2D_RECT_F {
                            left: thumb_l,
                            top: 146.5,
                            right: thumb_l + thumb_w,
                            bottom: 149.5,
                        },
                        radiusX: 1.5,
                        radiusY: 1.5,
                    },
                    &self.solid_brush,
                );
            }

            let pill_alpha = items_alpha * btn_fade;
            if pill_alpha > 0.01 {
                let (sel_raw, clear_raw) = shelf_bottom_button_rects(bl, br);
                let sel_rect = scale_rect(sel_raw, springs.slot_scale(38, 0.0, 0.05));
                self.draw_shelf_select_pill(
                    sel_rect,
                    springs.shelf_select_mode.value().clamp(0.0, 1.0),
                    springs.slot_hover(38),
                    pill_alpha,
                );
                let clear_rect = scale_rect(clear_raw, springs.slot_scale(16, 0.0, 0.05));
                self.draw_shelf_clear_all_pill(clear_rect, springs.slot_hover(16), pill_alpha);
            }
        }

        if empty_t > 0.01 {
            let zone_alpha = alpha * empty_t;
            let mut zone_rect = layout.shelf_zone_rect;
            zone_rect.left += dx;
            zone_rect.right += dx;

            let hot = sh_hov.max(springs.slot_hover(15) * 0.85).clamp(0.0, 1.05);
            let cx = (zone_rect.left + zone_rect.right) * 0.5;
            let cy = (zone_rect.top + zone_rect.bottom) * 0.5;
            let zone_scale = (0.95 + 0.05 * empty_t) * (1.0 + 0.02 * hot.min(1.0));
            let mut prev_zone_xf = Matrix3x2::identity();
            self.d2d_context.GetTransform(&mut prev_zone_xf);
            let scaled_zone = (zone_scale - 1.0).abs() > 0.0005;
            if scaled_zone {
                let zxf = Matrix3x2 {
                    M11: zone_scale,
                    M12: 0.0,
                    M21: 0.0,
                    M22: zone_scale,
                    M31: cx * (1.0 - zone_scale),
                    M32: cy * (1.0 - zone_scale),
                };
                let combined = mul_matrix3x2(zxf, prev_zone_xf);
                self.d2d_context.SetTransform(&combined);
            }

            let zone = D2D1_ROUNDED_RECT {
                rect: zone_rect,
                radiusX: 14.0,
                radiusY: 14.0,
            };

            let ht = hot.clamp(0.0, 1.0);
            self.set_color(1.0, 1.0, 1.0, (0.028 + 0.032 * ht) * zone_alpha);
            self.d2d_context.FillRoundedRectangle(&zone, &self.solid_brush);

            let cur_x = (springs.cursor_x.value + dx).clamp(zone_rect.left, zone_rect.right);
            let cur_y = springs.cursor_y.value.clamp(zone_rect.top, zone_rect.bottom);
            if ht > 0.005 {
                let spot_stops = [
                    D2D1_GRADIENT_STOP {
                        position: 0.0,
                        color: D2D1_COLOR_F {
                            r: 1.0,
                            g: 1.0,
                            b: 1.0,
                            a: 0.18 * ht * zone_alpha,
                        },
                    },
                    D2D1_GRADIENT_STOP {
                        position: 0.55,
                        color: D2D1_COLOR_F {
                            r: 1.0,
                            g: 1.0,
                            b: 1.0,
                            a: 0.05 * ht * zone_alpha,
                        },
                    },
                    D2D1_GRADIENT_STOP {
                        position: 1.0,
                        color: D2D1_COLOR_F {
                            r: 1.0,
                            g: 1.0,
                            b: 1.0,
                            a: 0.0,
                        },
                    },
                ];
                if let Ok(sc) = ID2D1RenderTarget::CreateGradientStopCollection(
                    &self.d2d_context,
                    &spot_stops,
                    D2D1_GAMMA_2_2,
                    D2D1_EXTEND_MODE_CLAMP,
                ) {
                    let rprops = D2D1_RADIAL_GRADIENT_BRUSH_PROPERTIES {
                        center: D2D_POINT_2F { x: cur_x, y: cur_y },
                        gradientOriginOffset: D2D_POINT_2F { x: 0.0, y: 0.0 },
                        radiusX: 82.0,
                        radiusY: 82.0,
                    };
                    if let Ok(rad) = self.d2d_context.CreateRadialGradientBrush(&rprops, None, &sc) {
                        self.d2d_context.FillRoundedRectangle(&zone, &rad);
                    }
                }
            }

            if ht > 0.005 {
                let b_stops = [
                    D2D1_GRADIENT_STOP {
                        position: 0.0,
                        color: D2D1_COLOR_F {
                            r: 1.0,
                            g: 1.0,
                            b: 1.0,
                            a: (0.30 + 0.65 * ht) * zone_alpha,
                        },
                    },
                    D2D1_GRADIENT_STOP {
                        position: 0.55,
                        color: D2D1_COLOR_F {
                            r: 0.62,
                            g: 0.82,
                            b: 1.0,
                            a: (0.30 + 0.30 * ht) * zone_alpha,
                        },
                    },
                    D2D1_GRADIENT_STOP {
                        position: 1.0,
                        color: D2D1_COLOR_F {
                            r: 1.0,
                            g: 1.0,
                            b: 1.0,
                            a: 0.30 * zone_alpha,
                        },
                    },
                ];
                if let Ok(sc) = ID2D1RenderTarget::CreateGradientStopCollection(
                    &self.d2d_context,
                    &b_stops,
                    D2D1_GAMMA_2_2,
                    D2D1_EXTEND_MODE_CLAMP,
                ) {
                    let rprops = D2D1_RADIAL_GRADIENT_BRUSH_PROPERTIES {
                        center: D2D_POINT_2F { x: cur_x, y: cur_y },
                        gradientOriginOffset: D2D_POINT_2F { x: 0.0, y: 0.0 },
                        radiusX: 96.0,
                        radiusY: 96.0,
                    };
                    if let Ok(rad) = self.d2d_context.CreateRadialGradientBrush(&rprops, None, &sc) {
                        self.d2d_context.DrawRoundedRectangle(
                            &zone,
                            &rad,
                            1.25,
                            self.dashed_stroke.as_ref(),
                        );
                    }
                } else {
                    self.set_color(1.0, 1.0, 1.0, 0.30 * zone_alpha);
                    self.d2d_context.DrawRoundedRectangle(
                        &zone,
                        &self.solid_brush,
                        1.25,
                        self.dashed_stroke.as_ref(),
                    );
                }
            } else {
                self.set_color(1.0, 1.0, 1.0, 0.30 * zone_alpha);
                self.d2d_context.DrawRoundedRectangle(
                    &zone,
                    &self.solid_brush,
                    1.25,
                    self.dashed_stroke.as_ref(),
                );
            }

            let bob_y = if ht > 0.02 {
                (springs.phase_secs() * 3.8).sin() * 2.4 * ht
            } else {
                0.0
            };
            self.set_color(
                0.90 + (0.494 - 0.90) * ht,
                0.91 + (0.698 - 0.91) * ht,
                0.94 + (1.0 - 0.94) * ht,
                0.96 * zone_alpha,
            );
            icons::draw_shelf_drop_hero(&self.d2d_context, cx, cy - 16.0, 28.0, brush, bob_y);

            let release_mode = drag_over_amt > 0.2;
            self.draw_text(
                if release_mode {
                    "Release to add"
                } else {
                    "Drop files here"
                },
                &self.fmt_center_md,
                &D2D_RECT_F {
                    left: zone_rect.left + 12.0,
                    top: cy + 7.0,
                    right: zone_rect.right - 12.0,
                    bottom: cy + 27.0,
                },
                if release_mode {
                    (0.494, 0.698, 1.0, 1.0 * zone_alpha)
                } else {
                    (0.96, 0.97, 0.98, 1.0 * zone_alpha)
                },
            );
            self.draw_text(
                "Drag out anytime · click to open or copy",
                &self.fmt_center_sm,
                &D2D_RECT_F {
                    left: zone_rect.left + 12.0,
                    top: cy + 25.0,
                    right: zone_rect.right - 12.0,
                    bottom: cy + 42.0,
                },
                (0.50, 0.50, 0.52, 1.0 * zone_alpha),
            );
            if scaled_zone {
                self.d2d_context.SetTransform(&prev_zone_xf);
            }
        }
    }

    fn truncate_clip_text_4lines<'a>(&self, text: &'a str, max_w: f32) -> std::borrow::Cow<'a, str> {
        let wide: Vec<u16> = text.encode_utf16().collect();
        if wide.is_empty() {
            return std::borrow::Cow::Borrowed(text);
        }
        unsafe {
            let Ok(layout) = self.dwrite.CreateTextLayout(
                &wide,
                &self.fmt_clip_text,
                max_w,
                1000.0,
            ) else {
                return std::borrow::Cow::Borrowed(text);
            };
            let mut line_count = 0u32;
            let _ = layout.GetLineMetrics(None, &mut line_count);
            if line_count <= 4 {
                return std::borrow::Cow::Borrowed(text);
            }
            let mut metrics = vec![
                windows::Win32::Graphics::DirectWrite::DWRITE_LINE_METRICS::default();
                line_count as usize
            ];
            let mut actual = 0u32;
            if layout.GetLineMetrics(Some(&mut metrics), &mut actual).is_err() || actual <= 4 {
                return std::borrow::Cow::Borrowed(text);
            }
            let take_u16: usize = metrics[..4].iter().map(|m| m.length as usize).sum();
            let capped = take_u16.min(wide.len());
            let mut prefix = String::from_utf16_lossy(&wide[..capped]);
            while prefix.ends_with(|c: char| c.is_whitespace()) {
                prefix.pop();
            }
            for _ in 0..3 {
                if prefix.len() > 4 {
                    prefix.pop();
                }
            }
            let trimmed = prefix.trim_end();
            std::borrow::Cow::Owned(format!("{trimmed}..."))
        }
    }

    unsafe fn draw_shelf_clipboard_tab(
        &self,
        shelf: &ShelfStore,
        springs: &NotchSprings,
        layout: &ExpandedLayout,
        dx: f32,
        alpha: f32,
    ) {
        let bl = layout.body_left + dx;
        let br = layout.body_right + dx;
        let cb_empty_t = springs.clipboard_empty_reveal.value().clamp(0.0, 1.0);
        let items_fade = (1.0 - cb_empty_t * 0.85).clamp(0.0, 1.0);

        if cb_empty_t > 0.01 {
            let s = 0.95 + 0.05 * cb_empty_t;
            let title_rect = scale_rect(
                D2D_RECT_F {
                    left: bl + 24.0,
                    top: 74.0,
                    right: br - 24.0,
                    bottom: 94.0,
                },
                s,
            );
            let sub_rect = scale_rect(
                D2D_RECT_F {
                    left: bl + 24.0,
                    top: 94.0,
                    right: br - 24.0,
                    bottom: 112.0,
                },
                s,
            );
            self.draw_text(
                "Copy some text",
                &self.fmt_center_md,
                &title_rect,
                (0.94, 0.94, 0.96, 1.0 * alpha * cb_empty_t),
            );
            self.draw_text(
                "It will wait here, ready to paste again",
                &self.fmt_center_sm,
                &sub_rect,
                (0.56, 0.56, 0.59, 1.0 * alpha * cb_empty_t),
            );
        }

        if shelf.clipboard_items.is_empty() || items_fade <= 0.01 {
            return;
        }

        let items_alpha = alpha * items_fade;
        let btn_fade = springs.shelf_clear_overall_progress().map_or(1.0, |ov| {
            if ov > 0.75 {
                ((1.0 - ov) / 0.25).clamp(0.0, 1.0)
            } else {
                1.0
            }
        });
        let alt_brush: &ID2D1Brush = &self.alt_brush;
        let mut prev_xf = Matrix3x2::identity();
        self.d2d_context.GetTransform(&mut prev_xf);

        let cb_clip = D2D_RECT_F {
            left: bl + 24.0,
            top: 48.0,
            right: br - 24.0,
            bottom: 139.0,
        };
        self.d2d_context
            .PushAxisAlignedClip(&cb_clip, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);

        let mut active_clip_idx = 0usize;
        for (idx, item) in shelf.clipboard_items.iter().enumerate() {
            let cur_active_idx = active_clip_idx;
            if !item.removing {
                active_clip_idx += 1;
            }
            let rel = item.slot_x_anim - shelf.clip_scroll_anim;
            if !(-1.15..=4.25).contains(&rel) {
                continue;
            }
            let left = bl + 32.0 + rel * 83.0;
            let card_raw = D2D_RECT_F {
                left,
                top: 53.0,
                right: left + 77.0,
                bottom: 134.0,
            };
            let in_vis = !item.removing
                && cur_active_idx >= shelf.clip_scroll_offset
                && cur_active_idx < shelf.clip_scroll_offset + 4;
            let vis_slot = (cur_active_idx as isize - shelf.clip_scroll_offset as isize)
                .clamp(0, 3) as usize;
            let enter_t = item.enter_anim.clamp(0.0, 1.0);
            let rem_t = if item.removing {
                item.remove_anim.clamp(0.0, 1.0)
            } else {
                0.0
            };
            let anim_fade = enter_t.powf(0.85) * (1.0 - rem_t).powf(1.35);
            let clear_p = springs.shelf_item_clear_progress(vis_slot);
            let item_alpha = if let Some(p) = clear_p {
                let clear_fade = if p < 0.72 {
                    1.0
                } else {
                    ((1.0 - p) / 0.28).clamp(0.0, 1.0)
                };
                items_alpha * clear_fade * anim_fade
            } else {
                items_alpha * anim_fade
            };
            if item_alpha <= 0.005 {
                continue;
            }

            let mut pushed_xf = false;
            if let Some(p) = clear_p {
                let reach = (p / 0.92).clamp(0.0, 1.0);
                let pull = reach * reach * (0.22 + 0.78 * reach);
                let arc_y = -10.0 * 4.0 * reach * (1.0 - reach);
                let s = (1.0 - 0.90 * p.powf(1.05)).clamp(0.08, 1.0);
                let base_cx = (card_raw.left + card_raw.right) * 0.5;
                let base_cy = (card_raw.top + card_raw.bottom) * 0.5;
                let target_x = br - 58.0;
                let target_y = 148.0;
                let dst_cx = base_cx + (target_x - base_cx) * pull;
                let dst_cy = base_cy + (target_y - base_cy) * pull + arc_y;
                let suction = Matrix3x2 {
                    M11: s,
                    M12: 0.0,
                    M21: 0.0,
                    M22: s,
                    M31: dst_cx - base_cx * s,
                    M32: dst_cy - base_cy * s,
                };
                let combined = mul_matrix3x2(suction, prev_xf);
                self.d2d_context.SetTransform(&combined);
                pushed_xf = true;
            } else if enter_t < 0.999 || rem_t > 0.001 {
                let s = (0.84 + 0.16 * enter_t) * (1.0 - 0.26 * rem_t);
                let dy_anim = -(1.0 - enter_t) * 11.0 - rem_t * 5.0;
                let base_cx = (card_raw.left + card_raw.right) * 0.5;
                let base_cy = (card_raw.top + card_raw.bottom) * 0.5;
                let xf = Matrix3x2 {
                    M11: s,
                    M12: 0.0,
                    M21: 0.0,
                    M22: s,
                    M31: base_cx - base_cx * s,
                    M32: (base_cy + dy_anim) - base_cy * s,
                };
                let combined = mul_matrix3x2(xf, prev_xf);
                self.d2d_context.SetTransform(&combined);
                pushed_xf = true;
            }

            let del_h = if in_vis { springs.slot_hover(43 + vis_slot) } else { 0.0 };
            let ch = if in_vis {
                springs.slot_hover(29 + vis_slot).max(del_h)
            } else {
                0.0
            };
            let copy_t = shelf.copied_flash_t(idx);
            let lift = -1.4 * ch;
            let card = D2D_RECT_F {
                left: card_raw.left,
                top: card_raw.top + lift,
                right: card_raw.right,
                bottom: card_raw.bottom + lift,
            };

            let rrect = D2D1_ROUNDED_RECT {
                rect: card,
                radiusX: 12.5,
                radiusY: 12.5,
            };

            self.set_color(
                (0.072 + 0.032 * ch) * (1.0 - copy_t) + 0.068 * copy_t,
                (0.072 + 0.032 * ch) * (1.0 - copy_t) + 0.145 * copy_t,
                (0.072 + 0.032 * ch) * (1.0 - copy_t) + 0.078 * copy_t,
                0.98 * item_alpha,
            );
            self.d2d_context.FillRoundedRectangle(&rrect, &self.solid_brush);

            if ch > 0.02 && copy_t < 0.85 {
                let sheen_a = 0.055 * ch * (1.0 - copy_t) * item_alpha;
                let spot_stops = [
                    D2D1_GRADIENT_STOP {
                        position: 0.0,
                        color: D2D1_COLOR_F {
                            r: 1.0,
                            g: 1.0,
                            b: 1.0,
                            a: sheen_a,
                        },
                    },
                    D2D1_GRADIENT_STOP {
                        position: 1.0,
                        color: D2D1_COLOR_F {
                            r: 1.0,
                            g: 1.0,
                            b: 1.0,
                            a: 0.0,
                        },
                    },
                ];
                if let Ok(sc) = ID2D1RenderTarget::CreateGradientStopCollection(
                    &self.d2d_context,
                    &spot_stops,
                    D2D1_GAMMA_2_2,
                    D2D1_EXTEND_MODE_CLAMP,
                ) {
                    let rprops = D2D1_RADIAL_GRADIENT_BRUSH_PROPERTIES {
                        center: D2D_POINT_2F {
                            x: (card.left + card.right) * 0.5,
                            y: card.top + 8.0,
                        },
                        gradientOriginOffset: D2D_POINT_2F { x: 0.0, y: 0.0 },
                        radiusX: 52.0,
                        radiusY: 42.0,
                    };
                    if let Ok(rad) = self.d2d_context.CreateRadialGradientBrush(&rprops, None, &sc) {
                        self.d2d_context.FillRoundedRectangle(&rrect, &rad);
                    }
                }
            }

            let border_r = 1.0 + (0.200 - 1.0) * copy_t;
            let border_g = 1.0 + (0.878 - 1.0) * copy_t;
            let border_b = 1.0 + (0.478 - 1.0) * copy_t;
            let border_a = ((0.07 + 0.14 * ch) * (1.0 - copy_t) + 0.85 * copy_t) * item_alpha;
            self.set_color(border_r, border_g, border_b, border_a);
            self.d2d_context.DrawRoundedRectangle(
                &rrect,
                &self.solid_brush,
                0.85 + 0.45 * copy_t,
                None,
            );

            if (1.0 - copy_t) > 0.02 {
                self.set_alt_color(
                    0.66,
                    0.66,
                    0.70,
                    (0.68 + 0.24 * ch) * (1.0 - copy_t) * item_alpha,
                );
                icons::draw_text_lines_icon(
                    &self.d2d_context,
                    card.left + 8.5,
                    card.top + 8.0,
                    8.5,
                    alt_brush,
                );
            }
            if copy_t > 0.02 {
                self.set_color(0.200, 0.878, 0.478, 0.98 * copy_t * item_alpha);
                icons::draw_checkmark_animated(
                    &self.d2d_context,
                    card.left + 12.5,
                    card.top + 11.5,
                    9.0,
                    &self.solid_brush,
                    1.6,
                    copy_t,
                );
            }

            let meta_rect = D2D_RECT_F {
                left: card.left + 21.5,
                top: card.top + 5.5,
                right: card.right - 7.0 - 13.0 * ch,
                bottom: card.top + 17.5,
            };
            if (1.0 - copy_t) > 0.02 {
                self.draw_text(
                    &item.time_label,
                    &self.fmt_clip_time,
                    &meta_rect,
                    (0.54, 0.54, 0.54, (0.88 + 0.10 * ch) * (1.0 - copy_t) * item_alpha),
                );
            }
            if copy_t > 0.02 {
                self.draw_text(
                    "Copied",
                    &self.fmt_clip_time,
                    &meta_rect,
                    (0.200, 0.878, 0.478, 1.0 * copy_t * item_alpha),
                );
            }

            if ch > 0.02 {
                let del_c = D2D_POINT_2F {
                    x: card.right - 10.0,
                    y: card.top + 10.0,
                };
                let bs = (0.72 + 0.28 * ch) * (1.0 + 0.08 * del_h);
                self.draw_centered_x_badge(
                    del_c,
                    6.0 * bs,
                    2.15 * bs,
                    1.35,
                    ch,
                    del_h,
                    item_alpha,
                );
            }

            let clip_text_rect = D2D_RECT_F {
                left: card.left + 8.5,
                top: card.top + 20.5,
                right: card.right - 8.0,
                bottom: card.bottom - 6.0,
            };
            let display_text = self.truncate_clip_text_4lines(
                &item.text,
                (clip_text_rect.right - clip_text_rect.left).max(16.0),
            );
            self.draw_text(
                &display_text,
                &self.fmt_clip_text,
                &clip_text_rect,
                (
                    0.91 + 0.07 * ch,
                    0.91 + 0.07 * ch,
                    0.93 + 0.05 * ch,
                    1.0 * item_alpha,
                ),
            );

            if pushed_xf {
                self.d2d_context.SetTransform(&prev_xf);
            }
        }

        self.d2d_context.PopAxisAlignedClip();

        if shelf.clipboard_items.len() > 4 {
            let track_l = bl + 130.0;
            let track_w = 134.0_f32;
            let max_off = (shelf.clipboard_items.len() - 4) as f32;
            let thumb_w = (track_w * (4.0 / shelf.clipboard_items.len() as f32)).clamp(24.0, 96.0);
            let frac = (shelf.clip_scroll_anim / max_off).clamp(0.0, 1.0);
            let thumb_l = track_l + (track_w - thumb_w) * frac;
            self.set_color(1.0, 1.0, 1.0, 0.12 * items_alpha * btn_fade);
            self.d2d_context.FillRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: track_l,
                        top: 146.5,
                        right: track_l + track_w,
                        bottom: 149.5,
                    },
                    radiusX: 1.5,
                    radiusY: 1.5,
                },
                &self.solid_brush,
            );
            self.set_color(1.0, 1.0, 1.0, 0.48 * items_alpha * btn_fade);
            self.d2d_context.FillRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: thumb_l,
                        top: 146.5,
                        right: thumb_l + thumb_w,
                        bottom: 149.5,
                    },
                    radiusX: 1.5,
                    radiusY: 1.5,
                },
                &self.solid_brush,
            );
        }

        let pill_alpha = items_alpha * btn_fade;
        if pill_alpha > 0.01 {
            let (_sel_raw, clear_raw) = shelf_bottom_button_rects(bl, br);
            let clear_rect = scale_rect(clear_raw, springs.slot_scale(16, 0.0, 0.05));
            self.draw_shelf_clear_all_pill(clear_rect, springs.slot_hover(16), pill_alpha);
        }
    }

    unsafe fn draw_todo_page(
        &self,
        springs: &NotchSprings,
        todo: &TodoState,
        layout: &ExpandedLayout,
        dx: f32,
        alpha: f32,
    ) {
        let brush: &ID2D1Brush = &self.solid_brush;
        let bl = layout.body_left + dx;
        let form_t = springs.todo_form_open.value().clamp(0.0, 1.0);
        let edit_t = springs.todo_edit_mode.value().clamp(0.0, 1.0);
        let empty_todo_t = springs.todo_empty_reveal.value().clamp(0.0, 1.0);
        let exp_t = springs.todo_list_expanded.value().clamp(0.0, 1.0) * (1.0 - form_t);

        let stats_a = (1.0 - exp_t * 1.65).clamp(0.0, 1.0) * alpha;
        if stats_a > 0.01 {
            let stats_dx = -12.0 * exp_t;
            let ratio_str = format!("{}/{}", todo.completed_count(), todo.items.len());
            self.draw_text(
                &ratio_str,
                &self.fmt_todo_ratio,
                &D2D_RECT_F {
                    left: bl + 28.0 + stats_dx,
                    top: 46.0,
                    right: bl + 114.0 + stats_dx,
                    bottom: 90.0,
                },
                (0.94, 0.94, 0.96, 1.0 * stats_a),
            );

            let mut cal_c = todo_calendar_center(bl);
            cal_c.x += stats_dx;
            let cal_h = springs.slot_hover(35);
            let cal_s = springs.slot_scale(35, 0.08, 0.14);
            let cal_ell = D2D1_ELLIPSE {
                point: cal_c,
                radiusX: 14.0 * cal_s,
                radiusY: 14.0 * cal_s,
            };
            if todo.schedule_filter {
                self.set_color(
                    0.065 + 0.025 * cal_h,
                    0.155 + 0.030 * cal_h,
                    0.285 + 0.040 * cal_h,
                    0.98 * stats_a,
                );
                self.d2d_context.FillEllipse(&cal_ell, &self.solid_brush);
                self.set_color(0.24, 0.62, 1.0, (0.68 + 0.20 * cal_h) * stats_a);
                self.d2d_context
                    .DrawEllipse(&cal_ell, &self.solid_brush, 0.95, None);
                self.set_color(0.60, 0.85, 1.0, (0.94 + 0.06 * cal_h) * stats_a);
            } else {
                self.set_color(
                    0.072 + 0.032 * cal_h,
                    0.072 + 0.032 * cal_h,
                    0.072 + 0.032 * cal_h,
                    0.98 * stats_a,
                );
                self.d2d_context.FillEllipse(&cal_ell, &self.solid_brush);
                self.set_color(1.0, 1.0, 1.0, (0.08 + 0.07 * cal_h) * stats_a);
                self.d2d_context
                    .DrawEllipse(&cal_ell, &self.solid_brush, 0.85, None);
                self.set_color(0.92, 0.92, 0.94, (0.92 + 0.08 * cal_h) * stats_a);
            }
            icons::draw_calendar_badge_icon(
                &self.d2d_context,
                cal_c.x,
                cal_c.y,
                15.0 * cal_s,
                brush,
                1.35,
            );

            self.draw_text(
                "Task Done Today",
                &self.fmt_artist,
                &D2D_RECT_F {
                    left: bl + 28.0 + stats_dx,
                    top: 92.0,
                    right: bl + 180.0 + stats_dx,
                    bottom: 110.0,
                },
                (0.60, 0.60, 0.60, 1.0 * stats_a),
            );
        }

        let (add_raw, edit_raw) = todo_button_rects_morphed(bl, exp_t);
        let btn_label_a = (1.0 - exp_t * 2.2).clamp(0.0, 1.0);
        let add_h = springs.slot_hover(33);
        let add_s = springs.slot_scale(33, 0.0, 0.05);
        let add_rect = scale_rect(add_raw, add_s);
        let add_rad = lerp(14.0, 15.0, exp_t) * add_s;
        let add_rrect = D2D1_ROUNDED_RECT {
            rect: add_rect,
            radiusX: add_rad,
            radiusY: add_rad,
        };
        if (1.0 - form_t) > 0.005 {
            self.set_color(
                0.072 + 0.032 * add_h,
                0.072 + 0.032 * add_h,
                0.072 + 0.032 * add_h,
                0.98 * (1.0 - form_t) * alpha,
            );
            self.d2d_context
                .FillRoundedRectangle(&add_rrect, &self.solid_brush);
            self.set_color(1.0, 1.0, 1.0, (0.08 + 0.07 * add_h) * (1.0 - form_t) * alpha);
            self.d2d_context
                .DrawRoundedRectangle(&add_rrect, &self.solid_brush, 0.85, None);
        }
        if form_t > 0.005 {
            self.set_color(
                0.068 + 0.020 * add_h,
                0.145 + 0.025 * add_h,
                0.078 + 0.020 * add_h,
                0.98 * form_t * alpha,
            );
            self.d2d_context
                .FillRoundedRectangle(&add_rrect, &self.solid_brush);
        }
        let add_fg = (
            1.0 + (0.260 - 1.0) * form_t,
            1.0 + (0.910 - 1.0) * form_t,
            1.0 + (0.475 - 1.0) * form_t,
            0.96 * alpha,
        );
        self.set_color(add_fg.0, add_fg.1, add_fg.2, add_fg.3);
        let add_icon_x = lerp(
            add_rect.left + 15.5 * add_s,
            (add_rect.left + add_rect.right) * 0.5,
            exp_t,
        );
        icons::draw_plus_circle_filled(
            &self.d2d_context,
            add_icon_x,
            (add_rect.top + add_rect.bottom) * 0.5,
            15.6 * add_s,
            brush,
        );
        if btn_label_a > 0.01 {
            self.draw_text(
                "Add Task",
                &self.fmt_pill_sm,
                &D2D_RECT_F {
                    left: add_rect.left + 22.5 * add_s,
                    top: add_rect.top,
                    right: add_rect.right - 5.0,
                    bottom: add_rect.bottom,
                },
                (add_fg.0, add_fg.1, add_fg.2, add_fg.3 * btn_label_a),
            );
        }

        let edit_h = springs.slot_hover(34);
        let edit_s = springs.slot_scale(34, 0.0, 0.05);
        let edit_rect = scale_rect(edit_raw, edit_s);
        let edit_rad = lerp(14.0, 15.0, exp_t) * edit_s;
        let edit_rrect = D2D1_ROUNDED_RECT {
            rect: edit_rect,
            radiusX: edit_rad,
            radiusY: edit_rad,
        };
        if (1.0 - edit_t) > 0.005 {
            self.set_color(
                0.072 + 0.032 * edit_h,
                0.072 + 0.032 * edit_h,
                0.072 + 0.032 * edit_h,
                0.98 * (1.0 - edit_t) * alpha,
            );
            self.d2d_context
                .FillRoundedRectangle(&edit_rrect, &self.solid_brush);
            self.set_color(1.0, 1.0, 1.0, (0.08 + 0.07 * edit_h) * (1.0 - edit_t) * alpha);
            self.d2d_context
                .DrawRoundedRectangle(&edit_rrect, &self.solid_brush, 0.85, None);
        }
        if edit_t > 0.005 {
            self.set_color(
                0.068 + 0.020 * edit_h,
                0.145 + 0.025 * edit_h,
                0.078 + 0.020 * edit_h,
                0.98 * edit_t * alpha,
            );
            self.d2d_context
                .FillRoundedRectangle(&edit_rrect, &self.solid_brush);
        }
        let edit_fg_a = if todo.items.is_empty() && !todo.editing {
            0.46 * alpha
        } else {
            0.96 * alpha
        };
        let edit_fg = (
            1.0 + (0.260 - 1.0) * edit_t,
            1.0 + (0.910 - 1.0) * edit_t,
            1.0 + (0.475 - 1.0) * edit_t,
            edit_fg_a,
        );
        self.set_color(edit_fg.0, edit_fg.1, edit_fg.2, edit_fg.3);
        let edit_icon_x = lerp(
            edit_rect.left + 15.5 * edit_s,
            (edit_rect.left + edit_rect.right) * 0.5,
            exp_t,
        );
        icons::draw_pencil_edit(
            &self.d2d_context,
            edit_icon_x,
            (edit_rect.top + edit_rect.bottom) * 0.5,
            14.8 * edit_s,
            brush,
        );
        if btn_label_a > 0.01 {
            self.draw_text(
                "Edit Task",
                &self.fmt_pill_sm,
                &D2D_RECT_F {
                    left: edit_rect.left + 22.5 * edit_s,
                    top: edit_rect.top,
                    right: edit_rect.right - 5.0,
                    bottom: edit_rect.bottom,
                },
                (edit_fg.0, edit_fg.1, edit_fg.2, edit_fg.3 * btn_label_a),
            );
        }

        let empty_mix = empty_todo_t * (1.0 - form_t);
        if empty_mix > 0.01 {
            let ea = alpha * empty_mix;
            let panel = scale_rect(todo_right_panel_rect(bl), 0.96 + 0.04 * empty_mix);
            let panel_rrect = D2D1_ROUNDED_RECT {
                rect: panel,
                radiusX: 16.0,
                radiusY: 16.0,
            };
            self.set_color(0.072, 0.072, 0.072, 0.98 * ea);
            self.d2d_context
                .FillRoundedRectangle(&panel_rrect, &self.solid_brush);
            self.set_color(1.0, 1.0, 1.0, 0.08 * ea);
            self.d2d_context
                .DrawRoundedRectangle(&panel_rrect, &self.solid_brush, 0.85, None);
            let pcx = (panel.left + panel.right) * 0.5;
            self.set_color(0.30, 0.30, 0.33, 0.96 * ea);
            self.set_alt_color(0.072, 0.072, 0.072, 0.98 * ea);
            icons::draw_no_tasks_badge(
                &self.d2d_context,
                pcx,
                84.0,
                33.0,
                &self.solid_brush,
                &self.alt_brush,
            );
            let empty_label = if todo.schedule_filter {
                "No scheduled tasks."
            } else {
                "No tasks."
            };
            self.draw_text(
                empty_label,
                &self.fmt_center_sm,
                &D2D_RECT_F {
                    left: panel.left + 8.0,
                    top: 108.0,
                    right: panel.right - 8.0,
                    bottom: 130.0,
                },
                (0.54, 0.54, 0.54, 1.0 * ea),
            );
        }

        let vis_indices = todo.visible_indices();
        let list_mix = (1.0 - form_t) * (1.0 - empty_todo_t * 0.85);
        if !vis_indices.is_empty() && list_mix > 0.01 {
            let la = alpha * list_mix;
            let viewport = todo_panel_rect_morphed(bl, exp_t);
            let panel_rrect = D2D1_ROUNDED_RECT {
                rect: viewport,
                radiusX: 15.0,
                radiusY: 15.0,
            };

            self.set_color(0.072, 0.072, 0.072, 0.98 * la);
            self.d2d_context
                .FillRoundedRectangle(&panel_rrect, &self.solid_brush);
            self.set_color(1.0, 1.0, 1.0, 0.08 * la);
            self.d2d_context
                .DrawRoundedRectangle(&panel_rrect, &self.solid_brush, 0.85, None);

            let clip_rect = D2D_RECT_F {
                left: viewport.left + 2.0,
                top: viewport.top + 2.0,
                right: viewport.right - 2.0,
                bottom: viewport.bottom - 2.0,
            };
            self.d2d_context
                .PushAxisAlignedClip(&clip_rect, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);

            let total_items = vis_indices.len();
            for (vis_pos, &actual_idx) in vis_indices.iter().enumerate() {
                let item = &todo.items[actual_idx];
                let idx = vis_pos;
                let rel_row = item.row_y_anim - todo.scroll_anim;
                if !(-1.1..=3.2).contains(&rel_row) {
                    continue;
                }
                let y = viewport.top + 6.0 + rel_row * 30.0;
                let base_card = D2D_RECT_F {
                    left: viewport.left + 6.0,
                    top: y,
                    right: viewport.right - 6.0,
                    bottom: y + 26.5,
                };
                let vis_slot = (idx as isize - todo.scroll_offset as isize).clamp(0, 2) as usize;
                let in_vis = idx >= todo.scroll_offset && idx < todo.scroll_offset + 3;
                let row_slot = 29 + vis_slot;
                let row_h = if in_vis { springs.slot_hover(row_slot) } else { 0.0 };
                let card = base_card;
                let card_rrect = D2D1_ROUNDED_RECT {
                    rect: card,
                    radiusX: 9.5,
                    radiusY: 9.5,
                };

                let dt = item.done_anim.clamp(0.0, 1.0);
                if (1.0 - dt) * row_h > 0.01 {
                    self.set_color(1.0, 1.0, 1.0, 0.05 * row_h * (1.0 - dt) * la);
                    self.d2d_context
                        .FillRoundedRectangle(&card_rrect, &self.solid_brush);
                }
                if dt > 0.005 {
                    self.set_color(
                        0.068 + 0.015 * row_h,
                        0.145 + 0.025 * row_h,
                        0.078 + 0.015 * row_h,
                        0.98 * dt * la,
                    );
                    self.d2d_context
                        .FillRoundedRectangle(&card_rrect, &self.solid_brush);
                }

                if edit_t > 0.01 {
                    self.set_color(1.0, 1.0, 1.0, (0.14 + 0.06 * row_h) * edit_t * la);
                    self.d2d_context
                        .DrawRoundedRectangle(&card_rrect, &self.solid_brush, 1.0, None);
                }

                let cy_row = (card.top + card.bottom) * 0.5;
                let check_c = D2D_POINT_2F {
                    x: card.left + 13.0,
                    y: cy_row,
                };
                if (1.0 - dt) > 0.01 {
                    self.set_color(1.0, 1.0, 1.0, 0.52 * (1.0 - dt) * la);
                    self.d2d_context.DrawEllipse(
                        &D2D1_ELLIPSE {
                            point: check_c,
                            radiusX: 7.0,
                            radiusY: 7.0,
                        },
                        &self.solid_brush,
                        1.3,
                        None,
                    );
                }
                if dt > 0.01 {
                    let pop = 0.72 + 0.28 * dt + 0.12 * (dt * std::f32::consts::PI).sin();
                    self.set_color(0.212, 0.847, 0.384, (dt * 1.4).min(1.0) * 0.98 * la);
                    self.d2d_context.FillEllipse(
                        &D2D1_ELLIPSE {
                            point: check_c,
                            radiusX: 7.0 * pop.min(1.08),
                            radiusY: 7.0 * pop.min(1.08),
                        },
                        &self.solid_brush,
                    );
                    let chk_prog = ((dt - 0.15) / 0.85).clamp(0.0, 1.0);
                    if chk_prog > 0.01 {
                        self.set_color(1.0, 1.0, 1.0, 0.98 * la);
                        icons::draw_checkmark_animated(
                            &self.d2d_context,
                            check_c.x,
                            check_c.y,
                            8.6 * pop.min(1.05),
                            brush,
                            1.8,
                            chk_prog,
                        );
                    }
                }

                let time_badge_w = if item.time_range.is_some() && edit_t < 0.5 {
                    let tr = item.time_range.as_deref().unwrap_or("");
                    (self.measure_width(tr, &self.fmt_label_xs) + 6.0).min(68.0) * (1.0 - edit_t)
                } else {
                    0.0
                };
                let text_left = card.left + 25.5;
                let text_right =
                    (card.right - 8.0 - 52.0 * edit_t - time_badge_w).max(text_left + 12.0);
                self.draw_text(
                    &item.text,
                    &self.fmt_artist,
                    &D2D_RECT_F {
                        left: text_left,
                        top: card.top,
                        right: text_right,
                        bottom: card.bottom,
                    },
                    (0.957, 0.957, 0.965, 0.96 * la),
                );
                if let Some(ref tr) = item.time_range {
                    if (1.0 - edit_t) > 0.05 {
                        let time_col = if dt > 0.5 {
                            (0.68, 0.88, 0.72, 0.72 * (1.0 - edit_t) * la)
                        } else {
                            (0.45, 0.74, 1.0, 0.88 * (1.0 - edit_t) * la)
                        };
                        self.draw_text(
                            tr,
                            &self.fmt_label_xs,
                            &D2D_RECT_F {
                                left: (card.right - 6.0 - time_badge_w).max(text_left + 14.0),
                                top: card.top,
                                right: card.right - 6.0,
                                bottom: card.bottom,
                            },
                            time_col,
                        );
                    }
                }

                if dt > 0.02 {
                    let max_w = (text_right - text_left).max(0.0);
                    let text_w = self.measure_width(&item.text, &self.fmt_artist).min(max_w);
                    let strike_w = text_w * dt;
                    if strike_w > 0.5 {
                        self.set_color(0.94, 0.96, 0.95, 0.88 * dt * la);
                        self.d2d_context.DrawLine(
                            D2D_POINT_2F {
                                x: text_left,
                                y: cy_row + 0.5,
                            },
                            D2D_POINT_2F {
                                x: text_left + strike_w,
                                y: cy_row + 0.5,
                            },
                            &self.solid_brush,
                            1.25,
                            None,
                        );
                    }
                }

                if edit_t > 0.02 {
                    let badge_s = 0.55 + 0.45 * edit_t;
                    let r_btn = 7.4 * badge_s;

                    let up_c = D2D_POINT_2F {
                        x: card.right - 47.0,
                        y: cy_row,
                    };
                    let can_up = idx > 0;
                    self.set_color(0.18, 0.18, 0.20, (if can_up { 0.94 } else { 0.52 }) * edit_t * la);
                    self.d2d_context.FillEllipse(
                        &D2D1_ELLIPSE {
                            point: up_c,
                            radiusX: r_btn,
                            radiusY: r_btn,
                        },
                        &self.solid_brush,
                    );
                    self.set_color(0.84, 0.84, 0.86, (if can_up { 0.94 } else { 0.34 }) * edit_t * la);
                    let cw = 2.6 * badge_s;
                    let ch = 1.6 * badge_s;
                    self.d2d_context.DrawLine(
                        D2D_POINT_2F { x: up_c.x - cw, y: up_c.y + ch * 0.7 },
                        D2D_POINT_2F { x: up_c.x, y: up_c.y - ch * 0.9 },
                        &self.solid_brush,
                        1.35,
                        None,
                    );
                    self.d2d_context.DrawLine(
                        D2D_POINT_2F { x: up_c.x, y: up_c.y - ch * 0.9 },
                        D2D_POINT_2F { x: up_c.x + cw, y: up_c.y + ch * 0.7 },
                        &self.solid_brush,
                        1.35,
                        None,
                    );

                    let dn_c = D2D_POINT_2F {
                        x: card.right - 30.0,
                        y: cy_row,
                    };
                    let can_dn = idx + 1 < total_items;
                    self.set_color(0.18, 0.18, 0.20, (if can_dn { 0.94 } else { 0.52 }) * edit_t * la);
                    self.d2d_context.FillEllipse(
                        &D2D1_ELLIPSE {
                            point: dn_c,
                            radiusX: r_btn,
                            radiusY: r_btn,
                        },
                        &self.solid_brush,
                    );
                    self.set_color(0.84, 0.84, 0.86, (if can_dn { 0.94 } else { 0.34 }) * edit_t * la);
                    self.d2d_context.DrawLine(
                        D2D_POINT_2F { x: dn_c.x - cw, y: dn_c.y - ch * 0.7 },
                        D2D_POINT_2F { x: dn_c.x, y: dn_c.y + ch * 0.9 },
                        &self.solid_brush,
                        1.35,
                        None,
                    );
                    self.d2d_context.DrawLine(
                        D2D_POINT_2F { x: dn_c.x, y: dn_c.y + ch * 0.9 },
                        D2D_POINT_2F { x: dn_c.x + cw, y: dn_c.y - ch * 0.7 },
                        &self.solid_brush,
                        1.35,
                        None,
                    );

                    let del_c = D2D_POINT_2F {
                        x: card.right - 13.0,
                        y: cy_row,
                    };
                    self.set_color(1.0, 0.255, 0.220, 0.96 * edit_t * la);
                    self.d2d_context.FillEllipse(
                        &D2D1_ELLIPSE {
                            point: del_c,
                            radiusX: r_btn,
                            radiusY: r_btn,
                        },
                        &self.solid_brush,
                    );
                    let arm = 2.75 * badge_s;
                    self.set_color(1.0, 1.0, 1.0, 0.98 * edit_t * la);
                    self.d2d_context.DrawLine(
                        D2D_POINT_2F {
                            x: del_c.x - arm,
                            y: del_c.y - arm,
                        },
                        D2D_POINT_2F {
                            x: del_c.x + arm,
                            y: del_c.y + arm,
                        },
                        &self.solid_brush,
                        1.5,
                        None,
                    );
                    self.d2d_context.DrawLine(
                        D2D_POINT_2F {
                            x: del_c.x + arm,
                            y: del_c.y - arm,
                        },
                        D2D_POINT_2F {
                            x: del_c.x - arm,
                            y: del_c.y + arm,
                        },
                        &self.solid_brush,
                        1.5,
                        None,
                    );
                }
            }

            self.d2d_context.PopAxisAlignedClip();

            if total_items > 3 {
                let track_top = 55.0_f32;
                let track_h = 90.0_f32;
                let max_off = (total_items - 3) as f32;
                let thumb_h = (track_h * (3.0 / total_items as f32)).clamp(20.0, 62.0);
                let frac = (todo.scroll_anim / max_off).clamp(0.0, 1.0);
                let thumb_y = track_top + (track_h - thumb_h) * frac;
                self.set_color(1.0, 1.0, 1.0, 0.42 * la);
                self.d2d_context.FillRoundedRectangle(
                    &D2D1_ROUNDED_RECT {
                        rect: D2D_RECT_F {
                            left: bl + 372.0,
                            top: thumb_y,
                            right: bl + 375.0,
                            bottom: thumb_y + thumb_h,
                        },
                        radiusX: 1.5,
                        radiusY: 1.5,
                    },
                    &self.solid_brush,
                );
            }
        }

        if form_t > 0.01 {
            let fa = alpha * form_t;
            let (panel, input_rect, cancel_raw, confirm_raw) = todo_inline_form_rects(bl);
            let mut prev_xf = Matrix3x2::identity();
            self.d2d_context.GetTransform(&mut prev_xf);
            let fs = 0.96 + 0.04 * form_t;
            if (fs - 1.0).abs() > 0.001 {
                let pcx = (panel.left + panel.right) * 0.5;
                let pcy = (panel.top + panel.bottom) * 0.5;
                let form_xf = Matrix3x2 {
                    M11: fs,
                    M12: 0.0,
                    M21: 0.0,
                    M22: fs,
                    M31: pcx * (1.0 - fs),
                    M32: pcy * (1.0 - fs),
                };
                let combined = mul_matrix3x2(form_xf, prev_xf);
                self.d2d_context.SetTransform(&combined);
            }

            let panel_rrect = D2D1_ROUNDED_RECT {
                rect: panel,
                radiusX: 18.0,
                radiusY: 18.0,
            };
            self.set_color(0.072, 0.072, 0.072, 0.98 * fa);
            self.d2d_context
                .FillRoundedRectangle(&panel_rrect, &self.solid_brush);
            self.set_color(1.0, 1.0, 1.0, 0.08 * fa);
            self.d2d_context
                .DrawRoundedRectangle(&panel_rrect, &self.solid_brush, 0.85, None);

            let input_rrect = D2D1_ROUNDED_RECT {
                rect: input_rect,
                radiusX: 9.0,
                radiusY: 9.0,
            };
            self.set_color(0.048, 0.048, 0.050, 0.98 * fa);
            self.d2d_context
                .FillRoundedRectangle(&input_rrect, &self.solid_brush);
            let title_border_a = if !todo.draft_focus_time { 0.82 } else { 0.16 };
            self.set_color(0.86, 0.86, 0.88, title_border_a * fa);
            self.d2d_context
                .DrawRoundedRectangle(&input_rrect, &self.solid_brush, 1.1, None);

            let dot_cy = (input_rect.top + input_rect.bottom) * 0.5;
            self.set_color(0.220, 0.875, 0.427, 0.98 * fa);
            self.d2d_context.FillEllipse(
                &D2D1_ELLIPSE {
                    point: D2D_POINT_2F {
                        x: input_rect.left + 12.0,
                        y: dot_cy,
                    },
                    radiusX: 4.2,
                    radiusY: 4.2,
                },
                &self.solid_brush,
            );

            let text_box = D2D_RECT_F {
                left: input_rect.left + 22.0,
                top: input_rect.top,
                right: input_rect.right - 8.0,
                bottom: input_rect.bottom,
            };
            if todo.draft_title.is_empty() {
                self.draw_text(
                    if !todo.draft_focus_time { "|Task title" } else { "Task title" },
                    &self.fmt_artist,
                    &text_box,
                    (0.50, 0.50, 0.52, 1.0 * fa),
                );
            } else {
                let typed = if !todo.draft_focus_time {
                    format!("{}|", todo.draft_title)
                } else {
                    todo.draft_title.clone()
                };
                self.draw_text(
                    &typed,
                    &self.fmt_artist,
                    &text_box,
                    (1.0, 1.0, 1.0, 1.0 * fa),
                );
            }

            let time_bar = D2D_RECT_F {
                left: panel.left + 8.0,
                top: panel.top + 38.5,
                right: panel.right - 8.0,
                bottom: panel.top + 60.5,
            };
            let time_rrect = D2D1_ROUNDED_RECT {
                rect: time_bar,
                radiusX: 8.5,
                radiusY: 8.5,
            };
            self.set_color(
                if todo.draft_focus_time { 0.048 } else { 0.100 },
                if todo.draft_focus_time { 0.048 } else { 0.100 },
                if todo.draft_focus_time { 0.050 } else { 0.102 },
                0.98 * fa,
            );
            self.d2d_context
                .FillRoundedRectangle(&time_rrect, &self.solid_brush);
            if todo.draft_focus_time {
                self.set_color(0.86, 0.86, 0.88, 0.82 * fa);
                self.d2d_context
                    .DrawRoundedRectangle(&time_rrect, &self.solid_brush, 1.1, None);
            }

            let clk_cx = time_bar.left + 11.5;
            let clk_cy = (time_bar.top + time_bar.bottom) * 0.5;
            self.set_color(
                if todo.draft_focus_time { 0.45 } else { 0.86 },
                if todo.draft_focus_time { 0.78 } else { 0.86 },
                if todo.draft_focus_time { 1.0 } else { 0.88 },
                0.92 * fa,
            );
            self.d2d_context.DrawEllipse(
                &D2D1_ELLIPSE {
                    point: D2D_POINT_2F {
                        x: clk_cx,
                        y: clk_cy,
                    },
                    radiusX: 5.5,
                    radiusY: 5.5,
                },
                &self.solid_brush,
                1.15,
                None,
            );
            self.d2d_context.DrawLine(
                D2D_POINT_2F {
                    x: clk_cx,
                    y: clk_cy,
                },
                D2D_POINT_2F {
                    x: clk_cx,
                    y: clk_cy - 3.0,
                },
                &self.solid_brush,
                1.15,
                None,
            );
            self.d2d_context.DrawLine(
                D2D_POINT_2F {
                    x: clk_cx,
                    y: clk_cy,
                },
                D2D_POINT_2F {
                    x: clk_cx + 2.4,
                    y: clk_cy + 1.2,
                },
                &self.solid_brush,
                1.15,
                None,
            );

            let time_disp = todo.format_draft_time(todo.draft_focus_time);
            let time_txt_col = if !todo.draft_time.is_empty() || todo.draft_focus_time {
                (0.95, 0.96, 0.98, 1.0 * fa)
            } else {
                (0.44, 0.44, 0.46, 1.0 * fa)
            };
            self.draw_text(
                &time_disp,
                &self.fmt_label_xs,
                &D2D_RECT_F {
                    left: time_bar.left + 22.0,
                    top: time_bar.top,
                    right: time_bar.right - 42.0,
                    bottom: time_bar.bottom,
                },
                time_txt_col,
            );

            let daily_pill = D2D_RECT_F {
                left: time_bar.right - 37.0,
                top: time_bar.top + 3.0,
                right: time_bar.right - 4.0,
                bottom: time_bar.bottom - 3.0,
            };
            if todo.draft_daily {
                self.set_color(0.08, 0.22, 0.38, 0.98 * fa);
            } else {
                self.set_color(0.135, 0.135, 0.138, 0.98 * fa);
            }
            self.d2d_context.FillRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: daily_pill,
                    radiusX: 7.5,
                    radiusY: 7.5,
                },
                &self.solid_brush,
            );
            self.draw_text(
                if todo.draft_daily { "Daily" } else { "Once" },
                &self.fmt_label_xs_center,
                &daily_pill,
                if todo.draft_daily {
                    (0.58, 0.85, 1.0, 1.0 * fa)
                } else {
                    (0.76, 0.76, 0.78, 1.0 * fa)
                },
            );

            let cancel_h = springs.slot_hover(37);
            let cancel_s = springs.slot_scale(37, 0.0, 0.05);
            let cancel_rect = scale_rect(cancel_raw, cancel_s);
            self.set_color(
                0.110 + 0.030 * cancel_h,
                0.110 + 0.030 * cancel_h,
                0.110 + 0.030 * cancel_h,
                0.98 * fa,
            );
            self.d2d_context.FillRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: cancel_rect,
                    radiusX: 10.5 * cancel_s,
                    radiusY: 10.5 * cancel_s,
                },
                &self.solid_brush,
            );
            self.draw_text(
                "Cancel",
                &self.fmt_pill_sm,
                &cancel_rect,
                (0.96, 0.96, 0.98, 1.0 * fa),
            );

            let confirm_h = springs.slot_hover(36);
            let confirm_s = springs.slot_scale(36, 0.0, 0.05);
            let confirm_rect = scale_rect(confirm_raw, confirm_s);
            self.set_color(0.220, 0.855, 0.415, (0.90 + 0.10 * confirm_h) * fa);
            self.d2d_context.FillRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: confirm_rect,
                    radiusX: 10.5 * confirm_s,
                    radiusY: 10.5 * confirm_s,
                },
                &self.solid_brush,
            );
            self.draw_text(
                "Save",
                &self.fmt_pill_sm,
                &confirm_rect,
                (0.05, 0.09, 0.06, 1.0 * fa),
            );

            if (fs - 1.0).abs() > 0.001 {
                self.d2d_context.SetTransform(&prev_xf);
            }
        }
    }

    unsafe fn draw_radial_glow(
        &self,
        cx: f32,
        cy: f32,
        radius: f32,
        rgb: (f32, f32, f32),
        center_alpha: f32,
    ) {
        if center_alpha <= 0.004 || radius <= 0.5 {
            return;
        }
        let stops = [
            D2D1_GRADIENT_STOP {
                position: 0.0,
                color: D2D1_COLOR_F {
                    r: rgb.0,
                    g: rgb.1,
                    b: rgb.2,
                    a: center_alpha,
                },
            },
            D2D1_GRADIENT_STOP {
                position: 0.55,
                color: D2D1_COLOR_F {
                    r: rgb.0,
                    g: rgb.1,
                    b: rgb.2,
                    a: center_alpha * 0.35,
                },
            },
            D2D1_GRADIENT_STOP {
                position: 1.0,
                color: D2D1_COLOR_F {
                    r: rgb.0,
                    g: rgb.1,
                    b: rgb.2,
                    a: 0.0,
                },
            },
        ];
        if let Ok(sc) = ID2D1RenderTarget::CreateGradientStopCollection(
            &self.d2d_context,
            &stops,
            D2D1_GAMMA_2_2,
            D2D1_EXTEND_MODE_CLAMP,
        ) {
            let props = D2D1_RADIAL_GRADIENT_BRUSH_PROPERTIES {
                center: D2D_POINT_2F { x: cx, y: cy },
                gradientOriginOffset: D2D_POINT_2F { x: 0.0, y: 0.0 },
                radiusX: radius,
                radiusY: radius,
            };
            if let Ok(rg) = self.d2d_context.CreateRadialGradientBrush(&props, None, &sc) {
                self.d2d_context.FillEllipse(
                    &D2D1_ELLIPSE {
                        point: D2D_POINT_2F { x: cx, y: cy },
                        radiusX: radius,
                        radiusY: radius,
                    },
                    &rg,
                );
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn draw_widgets_page(
        &self,
        cfg: &Config,
        springs: &NotchSprings,
        state: &EaveViewState<'_>,
        layout: &ExpandedLayout,
        dx: f32,
        alpha: f32,
    ) {
        let (left_raw, right_raw) = widget_card_rects(layout.body_left, layout.body_right);

        let left_r = D2D_RECT_F {
            left: left_raw.left + dx,
            top: left_raw.top,
            right: left_raw.right + dx,
            bottom: left_raw.bottom,
        };
        let right_r = D2D_RECT_F {
            left: right_raw.left + dx,
            top: right_raw.top,
            right: right_raw.right + dx,
            bottom: right_raw.bottom,
        };
        let lh = springs.widget_left_hover.value().clamp(0.0, 1.0);
        let rh = springs.widget_right_hover.value().clamp(0.0, 1.0);
        let lp = springs.widget_left_press.value().clamp(0.0, 1.0);
        let rp = springs.widget_right_press.value().clamp(0.0, 1.0);
        self.draw_widget_card(
            WidgetKind::from_u8(cfg.widget_left),
            cfg,
            springs,
            state,
            left_r,
            lh,
            lp,
            1.0,
            alpha,
        );
        self.draw_widget_card(
            WidgetKind::from_u8(cfg.widget_right),
            cfg,
            springs,
            state,
            right_r,
            rh,
            rp,
            1.0,
            alpha,
        );
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn draw_widget_card(
        &self,
        kind: WidgetKind,
        cfg: &Config,
        springs: &NotchSprings,
        state: &EaveViewState<'_>,
        rect: D2D_RECT_F,
        hov: f32,
        press: f32,
        enter: f32,
        alpha: f32,
    ) {
        let card_alpha = alpha * (0.25 + 0.75 * enter);
        if card_alpha <= 0.005 {
            return;
        }
        let cw = rect.right - rect.left;
        let ch = rect.bottom - rect.top;
        let card_cx = rect.left + cw * 0.5;
        let card_cy = rect.top + ch * 0.5;
        let phase = springs.phase_secs();

        let _ = press;
        let enter_dy = (1.0 - enter) * 6.5;
        let scale = 0.96 + 0.04 * enter;

        let mut prev_xf = Matrix3x2::identity();
        self.d2d_context.GetTransform(&mut prev_xf);
        if (scale - 1.0).abs() > 0.0005 || enter_dy.abs() > 0.01 {
            let sxf = Matrix3x2 {
                M11: scale,
                M12: 0.0,
                M21: 0.0,
                M22: scale,
                M31: card_cx * (1.0 - scale),
                M32: card_cy * (1.0 - scale) + enter_dy,
            };
            let combined = mul_matrix3x2(sxf, prev_xf);
            self.d2d_context.SetTransform(&combined);
        }

        let rrect = D2D1_ROUNDED_RECT {
            rect,
            radiusX: 15.0,
            radiusY: 15.0,
        };

        let accent: (f32, f32, f32) = match kind {
            WidgetKind::ThisWeek => (0.76, 0.78, 0.84),
            WidgetKind::Battery => springs.widget_bat_rgb(),
            WidgetKind::Water => (0.12, 0.64, 1.0),
            WidgetKind::FocusToday => (0.62, 0.45, 1.0),
            WidgetKind::WorldClock => (0.24, 0.72, 0.98),
            WidgetKind::Moon => (0.76, 0.78, 0.96),
            WidgetKind::Countdown => (1.0, 0.64, 0.14),
            WidgetKind::Weather => (0.42, 0.66, 1.0),
        };

        if kind == WidgetKind::Weather {
            let (_, _, hour) = WidgetState::local_weather_date();
            let w = &state.widgets.weather;
            let (c0, c1, c2) = if w.valid {
                if !w.is_day {
                    if matches!(w.wmo_code, 51..=67 | 80..=82 | 95..=99) {
                        ((0.06, 0.08, 0.15), (0.10, 0.13, 0.22), (0.15, 0.19, 0.30))
                    } else {
                        ((0.055, 0.075, 0.165), (0.095, 0.130, 0.275), (0.150, 0.195, 0.385))
                    }
                } else if matches!(w.wmo_code, 95..=99) {
                    ((0.10, 0.13, 0.22), (0.16, 0.20, 0.32), (0.24, 0.28, 0.40))
                } else if matches!(w.wmo_code, 51..=67 | 80..=82) {
                    ((0.10, 0.20, 0.36), (0.15, 0.29, 0.48), (0.22, 0.38, 0.56))
                } else if matches!(w.wmo_code, 71..=77 | 85..=86) {
                    ((0.16, 0.30, 0.50), (0.24, 0.42, 0.64), (0.36, 0.54, 0.74))
                } else if matches!(w.wmo_code, 3 | 45 | 48) {
                    ((0.15, 0.22, 0.34), (0.22, 0.30, 0.44), (0.30, 0.39, 0.52))
                } else {
                    ((0.08, 0.28, 0.56), (0.13, 0.42, 0.74), (0.22, 0.54, 0.84))
                }
            } else if (6..11).contains(&hour) {
                ((0.11, 0.22, 0.44), (0.18, 0.36, 0.62), (0.32, 0.46, 0.64))
            } else if (11..17).contains(&hour) {
                ((0.08, 0.28, 0.56), (0.13, 0.42, 0.74), (0.22, 0.54, 0.84))
            } else if (17..20).contains(&hour) {
                ((0.13, 0.15, 0.34), (0.26, 0.22, 0.44), (0.46, 0.30, 0.38))
            } else {
                ((0.055, 0.075, 0.165), (0.095, 0.130, 0.275), (0.150, 0.195, 0.385))
            };
            let stops = [
                D2D1_GRADIENT_STOP {
                    position: 0.0,
                    color: D2D1_COLOR_F {
                        r: (c0.0 + 0.03 * hov).min(1.0),
                        g: (c0.1 + 0.03 * hov).min(1.0),
                        b: (c0.2 + 0.04 * hov).min(1.0),
                        a: 0.98 * card_alpha,
                    },
                },
                D2D1_GRADIENT_STOP {
                    position: 0.55,
                    color: D2D1_COLOR_F {
                        r: (c1.0 + 0.03 * hov).min(1.0),
                        g: (c1.1 + 0.03 * hov).min(1.0),
                        b: (c1.2 + 0.04 * hov).min(1.0),
                        a: 0.98 * card_alpha,
                    },
                },
                D2D1_GRADIENT_STOP {
                    position: 1.0,
                    color: D2D1_COLOR_F {
                        r: (c2.0 + 0.04 * hov).min(1.0),
                        g: (c2.1 + 0.04 * hov).min(1.0),
                        b: (c2.2 + 0.04 * hov).min(1.0),
                        a: 0.98 * card_alpha,
                    },
                },
            ];
            if let Ok(sc) = ID2D1RenderTarget::CreateGradientStopCollection(
                &self.d2d_context,
                &stops,
                D2D1_GAMMA_2_2,
                D2D1_EXTEND_MODE_CLAMP,
            ) {
                let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                    startPoint: D2D_POINT_2F { x: rect.left, y: rect.top },
                    endPoint: D2D_POINT_2F { x: rect.left, y: rect.bottom },
                };
                if let Ok(lg) = self.d2d_context.CreateLinearGradientBrush(&props, None, &sc) {
                    self.d2d_context.FillRoundedRectangle(&rrect, &lg);
                }
            }
        } else {
            let glass_stops = [
                D2D1_GRADIENT_STOP {
                    position: 0.0,
                    color: D2D1_COLOR_F {
                        r: 1.0,
                        g: 1.0,
                        b: 1.0,
                        a: (0.095 + 0.038 * hov) * card_alpha,
                    },
                },
                D2D1_GRADIENT_STOP {
                    position: 1.0,
                    color: D2D1_COLOR_F {
                        r: 1.0,
                        g: 1.0,
                        b: 1.0,
                        a: (0.048 + 0.024 * hov) * card_alpha,
                    },
                },
            ];
            if let Ok(sc) = ID2D1RenderTarget::CreateGradientStopCollection(
                &self.d2d_context,
                &glass_stops,
                D2D1_GAMMA_2_2,
                D2D1_EXTEND_MODE_CLAMP,
            ) {
                let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                    startPoint: D2D_POINT_2F { x: rect.left, y: rect.top },
                    endPoint: D2D_POINT_2F { x: rect.left, y: rect.bottom },
                };
                if let Ok(lg) = self.d2d_context.CreateLinearGradientBrush(&props, None, &sc) {
                    self.d2d_context.FillRoundedRectangle(&rrect, &lg);
                }
            }

            let focus_boost = if kind == WidgetKind::FocusToday {
                springs.widget_focus_active.value().clamp(0.0, 1.0) * 0.08
            } else {
                0.0
            };
            let tint_stops = [
                D2D1_GRADIENT_STOP {
                    position: 0.0,
                    color: D2D1_COLOR_F {
                        r: accent.0,
                        g: accent.1,
                        b: accent.2,
                        a: (0.065 + 0.055 * hov + focus_boost) * card_alpha,
                    },
                },
                D2D1_GRADIENT_STOP {
                    position: 0.72,
                    color: D2D1_COLOR_F {
                        r: accent.0,
                        g: accent.1,
                        b: accent.2,
                        a: 0.0,
                    },
                },
            ];
            if let Ok(sc) = ID2D1RenderTarget::CreateGradientStopCollection(
                &self.d2d_context,
                &tint_stops,
                D2D1_GAMMA_2_2,
                D2D1_EXTEND_MODE_CLAMP,
            ) {
                let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                    startPoint: D2D_POINT_2F { x: rect.left, y: rect.top },
                    endPoint: D2D_POINT_2F { x: rect.right, y: rect.bottom },
                };
                if let Ok(lg) = self.d2d_context.CreateLinearGradientBrush(&props, None, &sc) {
                    self.d2d_context.FillRoundedRectangle(&rrect, &lg);
                }
            }
        }

        let br_r = 1.0 - 0.35 * hov + accent.0 * 0.35 * hov;
        let br_g = 1.0 - 0.35 * hov + accent.1 * 0.35 * hov;
        let br_b = 1.0 - 0.35 * hov + accent.2 * 0.35 * hov;
        let border_stops = [
            D2D1_GRADIENT_STOP {
                position: 0.0,
                color: D2D1_COLOR_F {
                    r: (br_r + 0.15).min(1.0),
                    g: (br_g + 0.15).min(1.0),
                    b: (br_b + 0.15).min(1.0),
                    a: (0.20 + 0.15 * hov) * card_alpha,
                },
            },
            D2D1_GRADIENT_STOP {
                position: 0.24,
                color: D2D1_COLOR_F {
                    r: br_r,
                    g: br_g,
                    b: br_b,
                    a: (0.10 + 0.13 * hov) * card_alpha,
                },
            },
            D2D1_GRADIENT_STOP {
                position: 1.0,
                color: D2D1_COLOR_F {
                    r: br_r,
                    g: br_g,
                    b: br_b,
                    a: (0.07 + 0.10 * hov) * card_alpha,
                },
            },
        ];
        if let Ok(sc) = ID2D1RenderTarget::CreateGradientStopCollection(
            &self.d2d_context,
            &border_stops,
            D2D1_GAMMA_2_2,
            D2D1_EXTEND_MODE_CLAMP,
        ) {
            let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                startPoint: D2D_POINT_2F { x: rect.left, y: rect.top },
                endPoint: D2D_POINT_2F { x: rect.left, y: rect.bottom },
            };
            if let Ok(lg) = self.d2d_context.CreateLinearGradientBrush(&props, None, &sc) {
                self.d2d_context
                    .DrawRoundedRectangle(&rrect, &lg, 1.0, None);
            }
        } else {
            self.set_color(br_r, br_g, br_b, (0.10 + 0.14 * hov) * card_alpha);
            self.d2d_context
                .DrawRoundedRectangle(&rrect, &self.solid_brush, 1.0, None);
        }

        self.d2d_context
            .PushAxisAlignedClip(&rect, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);

        let label_r = D2D_RECT_F {
            left: rect.left + 12.0,
            top: rect.top + 8.0,
            right: rect.right - 12.0,
            bottom: rect.top + 22.0,
        };
        let label_col = (0.68, 0.70, 0.76, 0.94 * card_alpha);

        match kind {
            WidgetKind::ThisWeek => {
                self.draw_text("THIS WEEK", &self.fmt_wg_label, &label_r, label_col);

                let (date_s, _, _) = WidgetState::local_weather_date();
                let mon_upper = date_s
                    .split_whitespace()
                    .next()
                    .unwrap_or("NOW")
                    .to_ascii_uppercase();
                let pill_r = D2D_RECT_F {
                    left: rect.right - 40.0,
                    top: rect.top + 7.5,
                    right: rect.right - 10.0,
                    bottom: rect.top + 20.5,
                };
                self.set_color(0.039, 0.518, 1.0, (0.18 + 0.10 * hov) * card_alpha);
                self.d2d_context.FillRoundedRectangle(
                    &D2D1_ROUNDED_RECT {
                        rect: pill_r,
                        radiusX: 6.5,
                        radiusY: 6.5,
                    },
                    &self.solid_brush,
                );
                self.draw_text(
                    &mon_upper,
                    &self.fmt_wg_pill,
                    &pill_r,
                    (0.45, 0.76, 1.0, 0.96 * card_alpha),
                );

                let (days, today_idx) = WidgetState::current_week_days();
                const LETTERS: [&str; 7] = ["M", "T", "W", "T", "F", "S", "S"];
                let pad_x = 8.0;
                let col_w = (cw - pad_x * 2.0) / 7.0;
                let day_cy = rect.top + 50.0;

                let sel_f = springs.widget_day_idx.value.clamp(0.0, 6.0);
                let pop = springs.widget_day_pop.value().clamp(0.0, 1.0);
                let sel_cx = rect.left + pad_x + (sel_f + 0.5) * col_w;
                let pill_rad = 10.0 + 1.6 * pop;
                self.draw_radial_glow(
                    sel_cx,
                    day_cy,
                    pill_rad + 6.5,
                    (0.039, 0.518, 1.0),
                    (0.42 + 0.18 * pop) * card_alpha,
                );
                self.set_color(0.039, 0.518, 1.0, 0.98 * card_alpha);
                self.d2d_context.FillEllipse(
                    &D2D1_ELLIPSE {
                        point: D2D_POINT_2F { x: sel_cx, y: day_cy },
                        radiusX: pill_rad,
                        radiusY: pill_rad,
                    },
                    &self.solid_brush,
                );

                for i in 0..7 {
                    let cx_i = rect.left + pad_x + (i as f32 + 0.5) * col_w;
                    let is_weekend = i >= 5;
                    let hdr_col = if i == today_idx {
                        (0.45, 0.76, 1.0, 0.96 * card_alpha)
                    } else if is_weekend {
                        (0.54, 0.56, 0.62, 0.85 * card_alpha)
                    } else {
                        (0.64, 0.66, 0.72, 0.92 * card_alpha)
                    };
                    self.draw_text(
                        LETTERS[i],
                        &self.fmt_wg_day_hdr,
                        &D2D_RECT_F {
                            left: cx_i - col_w * 0.5,
                            top: rect.top + 23.5,
                            right: cx_i + col_w * 0.5,
                            bottom: rect.top + 36.5,
                        },
                        hdr_col,
                    );
                    let proximity = (1.0 - (sel_f - i as f32).abs()).clamp(0.0, 1.0);
                    let lum = 0.78 + 0.22 * proximity;
                    let d_str = format!("{}", days[i]);
                    self.draw_text(
                        &d_str,
                        &self.fmt_wg_day_num,
                        &D2D_RECT_F {
                            left: cx_i - col_w * 0.5,
                            top: day_cy - 8.5,
                            right: cx_i + col_w * 0.5,
                            bottom: day_cy + 8.5,
                        },
                        (lum, lum, lum, card_alpha),
                    );
                    if i == today_idx {
                        self.set_color(0.22, 0.88, 0.42, 0.98 * card_alpha);
                        self.d2d_context.FillEllipse(
                            &D2D1_ELLIPSE {
                                point: D2D_POINT_2F {
                                    x: cx_i,
                                    y: day_cy + 13.0,
                                },
                                radiusX: 2.1,
                                radiusY: 2.1,
                            },
                            &self.solid_brush,
                        );
                    }
                }

                self.set_color(1.0, 1.0, 1.0, 0.08 * card_alpha);
                self.d2d_context.DrawLine(
                    D2D_POINT_2F {
                        x: rect.left + 12.0,
                        y: rect.bottom - 38.5,
                    },
                    D2D_POINT_2F {
                        x: rect.right - 12.0,
                        y: rect.bottom - 38.5,
                    },
                    &self.solid_brush,
                    0.8,
                    None,
                );

                let sel_day = state.widgets.selected_week_day.min(6);
                let (_, sel_wk, sel_mon, sel_num, _) = WidgetState::week_day_info(sel_day);
                let dsw = springs.widget_day_switch.value().clamp(0.0, 1.0);
                let dsw_dy = (1.0 - dsw) * 4.5;
                let dsw_a = (0.25 + 0.75 * dsw) * card_alpha;
                let is_today = sel_day == today_idx;
                let dot_col = if is_today {
                    (0.22, 0.88, 0.42)
                } else {
                    (0.24, 0.68, 1.0)
                };
                self.set_color(dot_col.0, dot_col.1, dot_col.2, 0.24 * dsw_a);
                self.d2d_context.FillEllipse(
                    &D2D1_ELLIPSE {
                        point: D2D_POINT_2F {
                            x: rect.left + 15.5,
                            y: rect.bottom - 27.5 + dsw_dy,
                        },
                        radiusX: 5.2,
                        radiusY: 5.2,
                    },
                    &self.solid_brush,
                );
                self.set_color(dot_col.0, dot_col.1, dot_col.2, 0.98 * dsw_a);
                self.d2d_context.FillEllipse(
                    &D2D1_ELLIPSE {
                        point: D2D_POINT_2F {
                            x: rect.left + 15.5,
                            y: rect.bottom - 27.5 + dsw_dy,
                        },
                        radiusX: 2.8,
                        radiusY: 2.8,
                    },
                    &self.solid_brush,
                );
                let open_tasks = state.todo.items.len().saturating_sub(state.todo.completed_count());
                let (summary, sub) = if is_today {
                    let s = if open_tasks == 0 {
                        "All caught up".to_string()
                    } else if open_tasks == 1 {
                        "1 task today".to_string()
                    } else {
                        format!("{open_tasks} tasks today")
                    };
                    let b = if let Some(first) = state.todo.items.iter().find(|t| !t.done) {
                        format!("Next: {}", first.text)
                    } else {
                        "Nothing scheduled".to_string()
                    };
                    (s, b)
                } else {
                    let diff = sel_day as i32 - today_idx as i32;
                    let rel = match diff {
                        -1 => "Yesterday".to_string(),
                        1 => "Tomorrow".to_string(),
                        d if d < 0 => format!("{} days ago", -d),
                        d => format!("In {d} days"),
                    };
                    (format!("{sel_wk}, {sel_mon} {sel_num}"), rel)
                };
                self.draw_text(
                    &summary,
                    &self.fmt_wg_val,
                    &D2D_RECT_F {
                        left: rect.left + 24.0,
                        top: rect.bottom - 35.5 + dsw_dy,
                        right: rect.right - 10.0,
                        bottom: rect.bottom - 18.5 + dsw_dy,
                    },
                    (0.97, 0.97, 0.99, dsw_a),
                );
                self.draw_text(
                    &sub,
                    &self.fmt_wg_sub,
                    &D2D_RECT_F {
                        left: rect.left + 24.0,
                        top: rect.bottom - 19.5 + dsw_dy,
                        right: rect.right - 10.0,
                        bottom: rect.bottom - 4.5 + dsw_dy,
                    },
                    (0.64, 0.66, 0.72, dsw_a),
                );
            }
            WidgetKind::Battery => {
                self.draw_text("BATTERY", &self.fmt_wg_label, &label_r, label_col);

                let Some(dev) = state.widgets.active_battery() else {
                    let cx = rect.left + 37.0;
                    let cy = rect.top + ch * 0.58;
                    let r = 21.5;
                    self.set_color(1.0, 1.0, 1.0, 0.12 * card_alpha);
                    self.d2d_context.DrawEllipse(
                        &D2D1_ELLIPSE {
                            point: D2D_POINT_2F { x: cx, y: cy },
                            radiusX: r,
                            radiusY: r,
                        },
                        &self.solid_brush,
                        4.8,
                        None,
                    );
                    self.set_color(1.0, 1.0, 1.0, 0.28 * card_alpha);
                    icons::draw_bluetooth_icon(
                        &self.d2d_context,
                        cx,
                        cy,
                        19.0,
                        &self.solid_brush,
                        1.8,
                    );
                    self.draw_text(
                        "—",
                        &self.fmt_wg_big,
                        &D2D_RECT_F {
                            left: rect.left + 68.0,
                            top: rect.top + 26.0,
                            right: rect.right - 8.0,
                            bottom: rect.top + 54.0,
                        },
                        (0.46, 0.48, 0.52, card_alpha),
                    );
                    self.draw_text(
                        "No battery",
                        &self.fmt_wg_val,
                        &D2D_RECT_F {
                            left: rect.left + 68.0,
                            top: rect.top + 56.0,
                            right: rect.right - 8.0,
                            bottom: rect.top + 73.0,
                        },
                        (0.54, 0.56, 0.60, card_alpha),
                    );
                    self.draw_text(
                        "Connect a BT device",
                        &self.fmt_wg_sub,
                        &D2D_RECT_F {
                            left: rect.left + 68.0,
                            top: rect.top + 74.0,
                            right: rect.right - 8.0,
                            bottom: rect.top + 91.0,
                        },
                        (0.44, 0.46, 0.50, card_alpha),
                    );
                    self.d2d_context.PopAxisAlignedClip();
                    self.d2d_context.SetTransform(&prev_xf);
                    return;
                };

                let dev_cnt = state.widgets.battery_devices.len();
                let active_idx = state.widgets.selected_battery_idx;
                let sw = springs.widget_bat_switch.value().clamp(0.0, 1.0);
                let sw_dy = (1.0 - sw) * 5.0;
                let sw_a = (0.30 + 0.70 * sw) * card_alpha;

                if dev_cnt > 1 {
                    let shown = dev_cnt.min(5);
                    let mut x_cur = rect.right - 11.0;
                    for i in (0..shown).rev() {
                        let is_cur = i == active_idx;
                        let pw = if is_cur { 9.5 } else { 3.8 };
                        let left_x = x_cur - pw;
                        self.set_color(
                            1.0,
                            1.0,
                            1.0,
                            (if is_cur { 0.92 } else { 0.28 }) * card_alpha,
                        );
                        self.d2d_context.FillRoundedRectangle(
                            &D2D1_ROUNDED_RECT {
                                rect: D2D_RECT_F {
                                    left: left_x,
                                    top: rect.top + 12.2,
                                    right: x_cur,
                                    bottom: rect.top + 16.0,
                                },
                                radiusX: 1.9,
                                radiusY: 1.9,
                            },
                            &self.solid_brush,
                        );
                        x_cur = left_x - 3.6;
                    }
                }

                let cx = rect.left + 37.0;
                let cy = rect.top + ch * 0.58;
                let r = 21.8;
                let prog = (springs.widget_bat_arc.value().clamp(0.0, 1.0)) * (0.25 + 0.75 * enter);
                let ring_col = if dev.is_bluetooth {
                    if dev.pct <= 15 {
                        (1.0, 0.271, 0.227)
                    } else if dev.pct <= 25 {
                        (1.0, 0.624, 0.039)
                    } else {
                        (0.24, 0.68, 1.0)
                    }
                } else if dev.charging || dev.is_ac_only || dev.pct > 25 {
                    (0.188, 0.820, 0.349)
                } else if dev.pct > 15 {
                    (1.0, 0.624, 0.039)
                } else {
                    (1.0, 0.271, 0.227)
                };

                self.draw_radial_glow(
                    cx,
                    cy,
                    r + 12.0,
                    ring_col,
                    (0.22 + 0.10 * hov) * card_alpha,
                );

                if dev.charging {
                    let t = (phase * 0.62) % 1.0;
                    let rip_r = r + 10.5 * t;
                    let rip_a = (1.0 - t).powi(2) * 0.36 * card_alpha;
                    if rip_a > 0.01 {
                        self.set_color(ring_col.0, ring_col.1, ring_col.2, rip_a);
                        self.d2d_context.DrawEllipse(
                            &D2D1_ELLIPSE {
                                point: D2D_POINT_2F { x: cx, y: cy },
                                radiusX: rip_r,
                                radiusY: rip_r,
                            },
                            &self.solid_brush,
                            2.2 * (1.0 - 0.45 * t),
                            None,
                        );
                    }
                }

                self.set_color(ring_col.0, ring_col.1, ring_col.2, 0.20 * card_alpha);
                self.d2d_context.DrawEllipse(
                    &D2D1_ELLIPSE {
                        point: D2D_POINT_2F { x: cx, y: cy },
                        radiusX: r,
                        radiusY: r,
                    },
                    &self.solid_brush,
                    5.0,
                    None,
                );

                let sweep = (prog * 359.5).max(8.0);
                let a0 = (-90.0_f32).to_radians();
                let a1 = (-90.0_f32 + sweep).to_radians();
                if let Ok(geo) = self.d2d_factory.CreatePathGeometry() {
                    if let Ok(sink) = geo.Open() {
                        sink.BeginFigure(
                            D2D_POINT_2F {
                                x: cx + r * a0.cos(),
                                y: cy + r * a0.sin(),
                            },
                            windows::Win32::Graphics::Direct2D::Common::D2D1_FIGURE_BEGIN_HOLLOW,
                        );
                        sink.AddArc(&D2D1_ARC_SEGMENT {
                            point: D2D_POINT_2F {
                                x: cx + r * a1.cos(),
                                y: cy + r * a1.sin(),
                            },
                            size: D2D_SIZE_F { width: r, height: r },
                            rotationAngle: 0.0,
                            sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                            arcSize: if sweep > 180.0 {
                                windows::Win32::Graphics::Direct2D::D2D1_ARC_SIZE_LARGE
                            } else {
                                D2D1_ARC_SIZE_SMALL
                            },
                        });
                        sink.EndFigure(
                            windows::Win32::Graphics::Direct2D::Common::D2D1_FIGURE_END_OPEN,
                        );
                        if sink.Close().is_ok() {
                            let round_st = ID2D1Factory::CreateStrokeStyle(
                                &self.d2d_factory,
                                &D2D1_STROKE_STYLE_PROPERTIES {
                                    startCap: D2D1_CAP_STYLE_ROUND,
                                    endCap: D2D1_CAP_STYLE_ROUND,
                                    ..Default::default()
                                },
                                None,
                            )
                            .ok();
                            self.set_color(ring_col.0, ring_col.1, ring_col.2, 0.24 * card_alpha);
                            self.d2d_context.DrawGeometry(
                                &geo,
                                &self.solid_brush,
                                8.2,
                                round_st.as_ref(),
                            );
                            self.set_color(ring_col.0, ring_col.1, ring_col.2, card_alpha);
                            self.d2d_context.DrawGeometry(
                                &geo,
                                &self.solid_brush,
                                5.0,
                                round_st.as_ref(),
                            );
                        }
                    }
                }

                if dev.is_bluetooth {
                    self.set_color(1.0, 1.0, 1.0, 0.96 * sw_a);
                    icons::draw_bluetooth_icon(
                        &self.d2d_context,
                        cx,
                        cy,
                        19.5,
                        &self.solid_brush,
                        1.95,
                    );
                } else if dev.charging || dev.is_ac_only {
                    if let Ok(geo) = self.d2d_factory.CreatePathGeometry() {
                        if let Ok(sink) = geo.Open() {
                            sink.BeginFigure(
                                D2D_POINT_2F { x: cx + 1.5, y: cy - 8.2 },
                                windows::Win32::Graphics::Direct2D::Common::D2D1_FIGURE_BEGIN_FILLED,
                            );
                            sink.AddLine(D2D_POINT_2F { x: cx - 5.0, y: cy + 0.8 });
                            sink.AddLine(D2D_POINT_2F { x: cx - 0.5, y: cy + 0.8 });
                            sink.AddLine(D2D_POINT_2F { x: cx - 1.7, y: cy + 8.2 });
                            sink.AddLine(D2D_POINT_2F { x: cx + 5.0, y: cy - 0.8 });
                            sink.AddLine(D2D_POINT_2F { x: cx + 0.5, y: cy - 0.8 });
                            sink.EndFigure(
                                windows::Win32::Graphics::Direct2D::Common::D2D1_FIGURE_END_CLOSED,
                            );
                            if sink.Close().is_ok() {
                                self.set_color(1.0, 1.0, 1.0, 0.96 * sw_a);
                                self.d2d_context.FillGeometry(&geo, &self.solid_brush, None);
                            }
                        }
                    }
                } else {
                    self.set_color(1.0, 1.0, 1.0, 0.95 * sw_a);
                    self.d2d_context.DrawRoundedRectangle(
                        &D2D1_ROUNDED_RECT {
                            rect: D2D_RECT_F {
                                left: cx - 6.6,
                                top: cy - 4.8,
                                right: cx + 6.6,
                                bottom: cy + 3.2,
                            },
                            radiusX: 1.5,
                            radiusY: 1.5,
                        },
                        &self.solid_brush,
                        1.5,
                        None,
                    );
                    self.d2d_context.DrawLine(
                        D2D_POINT_2F { x: cx - 8.5, y: cy + 5.5 },
                        D2D_POINT_2F { x: cx + 8.5, y: cy + 5.5 },
                        &self.solid_brush,
                        1.7,
                        None,
                    );
                }

                let big_txt = if dev.is_ac_only {
                    "AC".to_string()
                } else {
                    format!("{}%", dev.pct)
                };
                self.draw_text(
                    &big_txt,
                    &self.fmt_wg_big,
                    &D2D_RECT_F {
                        left: rect.left + 68.0,
                        top: rect.top + 26.0 + sw_dy,
                        right: rect.right - 8.0,
                        bottom: rect.top + 54.0 + sw_dy,
                    },
                    (0.98, 0.98, 0.99, sw_a),
                );
                let primary_lbl = if dev.is_bluetooth {
                    dev.name.as_str()
                } else if dev.is_ac_only {
                    "Plugged in"
                } else if dev.charging {
                    "Charging"
                } else {
                    "Laptop"
                };
                self.draw_text(
                    primary_lbl,
                    &self.fmt_wg_val,
                    &D2D_RECT_F {
                        left: rect.left + 68.0,
                        top: rect.top + 56.0 + sw_dy,
                        right: rect.right - 8.0,
                        bottom: rect.top + 73.0 + sw_dy,
                    },
                    (ring_col.0, ring_col.1, ring_col.2, sw_a),
                );
                let sub_txt = if dev_cnt > 1 {
                    if dev.is_bluetooth {
                        "Bluetooth · click"
                    } else if dev.charging {
                        "Laptop · click"
                    } else {
                        "On battery · click"
                    }
                } else if dev.is_bluetooth {
                    "Bluetooth device"
                } else if dev.is_ac_only {
                    "Desktop power"
                } else if dev.charging {
                    "AC connected"
                } else {
                    "On battery"
                };
                self.draw_text(
                    sub_txt,
                    &self.fmt_wg_sub,
                    &D2D_RECT_F {
                        left: rect.left + 68.0,
                        top: rect.top + 75.0 + sw_dy,
                        right: rect.right - 8.0,
                        bottom: rect.top + 92.0 + sw_dy,
                    },
                    (0.64, 0.66, 0.72, sw_a),
                );
            }
            WidgetKind::Water => {
                self.draw_text("WATER", &self.fmt_wg_label, &label_r, label_col);

                let add_pill = D2D_RECT_F {
                    left: rect.right - 44.0,
                    top: rect.top + 7.5,
                    right: rect.right - 10.0,
                    bottom: rect.top + 20.5,
                };
                self.set_color(0.12, 0.64, 1.0, (0.18 + 0.20 * hov) * card_alpha);
                self.d2d_context.FillRoundedRectangle(
                    &D2D1_ROUNDED_RECT {
                        rect: add_pill,
                        radiusX: 6.5,
                        radiusY: 6.5,
                    },
                    &self.solid_brush,
                );
                let step_lbl = format!("+{}", state.water.bottle_ml.min(999));
                self.draw_text(
                    &step_lbl,
                    &self.fmt_wg_pill,
                    &add_pill,
                    (0.52, 0.84, 1.0, (0.85 + 0.15 * hov) * card_alpha),
                );

                let gx = rect.left + 13.0;
                let gy = rect.top + 28.0;
                let gw = 36.0;
                let gh = 68.0;
                let fill_t = (springs.widget_water_fill.value().clamp(0.0, 1.0)) * (0.20 + 0.80 * enter);
                let splash = springs.widget_water_splash.value().clamp(0.0, 1.0);
                let cup_r = D2D_RECT_F {
                    left: gx,
                    top: gy,
                    right: gx + gw,
                    bottom: gy + gh,
                };
                let cup_rr = D2D1_ROUNDED_RECT {
                    rect: cup_r,
                    radiusX: 9.0,
                    radiusY: 9.0,
                };
                self.set_color(1.0, 1.0, 1.0, 0.07 * card_alpha);
                self.d2d_context.FillRoundedRectangle(&cup_rr, &self.solid_brush);

                if fill_t > 0.01 {
                    let inner_r = D2D_RECT_F {
                        left: gx + 1.8,
                        top: gy + 1.8,
                        right: gx + gw - 1.8,
                        bottom: gy + gh - 1.8,
                    };
                    if let Ok(clip_geo) = self.d2d_factory.CreateRoundedRectangleGeometry(
                        &D2D1_ROUNDED_RECT {
                            rect: inner_r,
                            radiusX: 7.4,
                            radiusY: 7.4,
                        },
                    ) {
                        let layer_params = D2D1_LAYER_PARAMETERS1 {
                            contentBounds: inner_r,
                            geometricMask: ManuallyDrop::new(Some(clip_geo.into())),
                            maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                            maskTransform: Matrix3x2::identity(),
                            opacity: 1.0,
                            opacityBrush: ManuallyDrop::new(None),
                            layerOptions: D2D1_LAYER_OPTIONS1_NONE,
                        };
                        self.d2d_context.PushLayer(&layer_params, None);

                        let inner_h = inner_r.bottom - inner_r.top;
                        let surface_y = inner_r.bottom - inner_h * fill_t;
                        let amp = 1.6 + 2.6 * splash;

                        if let Ok(wave_geo) = self.d2d_factory.CreatePathGeometry() {
                            if let Ok(sink) = wave_geo.Open() {
                                let segs = 12usize;
                                let y0 = surface_y + (phase * 2.4).sin() * amp;
                                sink.BeginFigure(
                                    D2D_POINT_2F { x: inner_r.left, y: y0 },
                                    windows::Win32::Graphics::Direct2D::Common::D2D1_FIGURE_BEGIN_FILLED,
                                );
                                for s in 1..=segs {
                                    let t = s as f32 / segs as f32;
                                    let wx = inner_r.left + (inner_r.right - inner_r.left) * t;
                                    let wy = surface_y
                                        + (t * std::f32::consts::TAU + phase * 2.4).sin() * amp
                                        + (t * std::f32::consts::TAU * 1.6 - phase * 1.8).cos()
                                            * (amp * 0.35);
                                    sink.AddLine(D2D_POINT_2F { x: wx, y: wy });
                                }
                                sink.AddLine(D2D_POINT_2F {
                                    x: inner_r.right,
                                    y: inner_r.bottom + 2.0,
                                });
                                sink.AddLine(D2D_POINT_2F {
                                    x: inner_r.left,
                                    y: inner_r.bottom + 2.0,
                                });
                                sink.EndFigure(
                                    windows::Win32::Graphics::Direct2D::Common::D2D1_FIGURE_END_CLOSED,
                                );
                                if sink.Close().is_ok() {
                                    let liq_stops = [
                                        D2D1_GRADIENT_STOP {
                                            position: 0.0,
                                            color: D2D1_COLOR_F {
                                                r: 0.24,
                                                g: 0.80,
                                                b: 1.0,
                                                a: 0.96 * card_alpha,
                                            },
                                        },
                                        D2D1_GRADIENT_STOP {
                                            position: 1.0,
                                            color: D2D1_COLOR_F {
                                                r: 0.04,
                                                g: 0.44,
                                                b: 0.96,
                                                a: 0.96 * card_alpha,
                                            },
                                        },
                                    ];
                                    if let Ok(sc) = ID2D1RenderTarget::CreateGradientStopCollection(
                                        &self.d2d_context,
                                        &liq_stops,
                                        D2D1_GAMMA_2_2,
                                        D2D1_EXTEND_MODE_CLAMP,
                                    ) {
                                        let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                                            startPoint: D2D_POINT_2F {
                                                x: inner_r.left,
                                                y: surface_y - 3.0,
                                            },
                                            endPoint: D2D_POINT_2F {
                                                x: inner_r.left,
                                                y: inner_r.bottom,
                                            },
                                        };
                                        if let Ok(lg) = self
                                            .d2d_context
                                            .CreateLinearGradientBrush(&props, None, &sc)
                                        {
                                            self.d2d_context.FillGeometry(&wave_geo, &lg, None);
                                        }
                                    }
                                }
                            }
                        }

                        if fill_t > 0.14 {
                            for b in 0..3 {
                                let bt = (phase * (0.34 + b as f32 * 0.11) + b as f32 * 0.37) % 1.0;
                                let bx = inner_r.left + 7.0 + b as f32 * 9.0 + (bt * 6.0).sin() * 1.4;
                                let by = inner_r.bottom - 4.0 - bt * (inner_h * fill_t - 6.0).max(2.0);
                                let ba = (1.0 - bt) * 0.35 * card_alpha;
                                if ba > 0.02 && by > surface_y + 2.0 {
                                    self.set_color(0.85, 0.96, 1.0, ba);
                                    self.d2d_context.FillEllipse(
                                        &D2D1_ELLIPSE {
                                            point: D2D_POINT_2F { x: bx, y: by },
                                            radiusX: 1.5,
                                            radiusY: 1.5,
                                        },
                                        &self.solid_brush,
                                    );
                                }
                            }
                        }

                        self.d2d_context.PopLayer();
                    }
                }

                self.set_color(1.0, 1.0, 1.0, 0.22 * card_alpha);
                self.d2d_context.DrawLine(
                    D2D_POINT_2F { x: gx + 4.5, y: gy + 9.0 },
                    D2D_POINT_2F { x: gx + 4.5, y: gy + gh - 9.0 },
                    &self.solid_brush,
                    1.3,
                    None,
                );
                self.set_color(1.0, 1.0, 1.0, (0.20 + 0.12 * hov) * card_alpha);
                self.d2d_context
                    .DrawRoundedRectangle(&cup_rr, &self.solid_brush, 1.15, None);

                let taken_txt = if state.water.taken_ml >= 1000 {
                    format!("{:.1} L", state.water.taken_ml as f32 / 1000.0)
                } else {
                    format!("{} ml", state.water.taken_ml)
                };
                self.draw_text(
                    &taken_txt,
                    &self.fmt_wg_big,
                    &D2D_RECT_F {
                        left: rect.left + 59.0,
                        top: rect.top + 27.0,
                        right: rect.right - 8.0,
                        bottom: rect.top + 55.0,
                    },
                    (0.98, 0.98, 0.99, card_alpha),
                );
                let pct = (state.water.progress() * 100.0).round() as u32;
                let goal_txt = format!("of {:.1} L · {}%", state.water.goal_ml as f32 / 1000.0, pct);
                self.draw_text(
                    &goal_txt,
                    &self.fmt_wg_sub,
                    &D2D_RECT_F {
                        left: rect.left + 59.0,
                        top: rect.top + 56.5,
                        right: rect.right - 8.0,
                        bottom: rect.top + 73.0,
                    },
                    (0.66, 0.72, 0.80, card_alpha),
                );

                let pip_prog = fill_t * 8.0;
                for i in 0..8 {
                    let lit = (pip_prog - i as f32).clamp(0.0, 1.0);
                    let dx_i = rect.left + 59.0 + i as f32 * 11.4;
                    let dy_i = rect.top + 84.5;
                    let pr = D2D_RECT_F {
                        left: dx_i,
                        top: dy_i - 3.6,
                        right: dx_i + 7.6,
                        bottom: dy_i + 3.6,
                    };
                    self.set_color(1.0, 1.0, 1.0, 0.15 * card_alpha);
                    self.d2d_context.FillRoundedRectangle(
                        &D2D1_ROUNDED_RECT {
                            rect: pr,
                            radiusX: 3.4,
                            radiusY: 3.4,
                        },
                        &self.solid_brush,
                    );
                    if lit > 0.02 {
                        self.set_color(0.16, 0.68, 1.0, lit * 0.98 * card_alpha);
                        self.d2d_context.FillRoundedRectangle(
                            &D2D1_ROUNDED_RECT {
                                rect: pr,
                                radiusX: 3.4,
                                radiusY: 3.4,
                            },
                            &self.solid_brush,
                        );
                    }
                }
            }
            WidgetKind::FocusToday => {
                self.draw_text("FOCUS TODAY", &self.fmt_wg_label, &label_r, label_col);

                let act = springs.widget_focus_active.value().clamp(0.0, 1.0);
                let pulse = 0.5 + 0.5 * (phase * 3.2).sin();

                let pill_r = D2D_RECT_F {
                    left: rect.right - 52.0,
                    top: rect.top + 7.5,
                    right: rect.right - 10.0,
                    bottom: rect.top + 20.5,
                };
                self.set_color(
                    0.58,
                    0.40,
                    1.0,
                    (0.14 + 0.18 * act + 0.08 * hov) * card_alpha,
                );
                self.d2d_context.FillRoundedRectangle(
                    &D2D1_ROUNDED_RECT {
                        rect: pill_r,
                        radiusX: 6.5,
                        radiusY: 6.5,
                    },
                    &self.solid_brush,
                );
                if act > 0.05 {
                    self.set_color(0.76, 0.62, 1.0, (0.55 + 0.45 * pulse) * act * card_alpha);
                    self.d2d_context.FillEllipse(
                        &D2D1_ELLIPSE {
                            point: D2D_POINT_2F {
                                x: pill_r.left + 7.5,
                                y: (pill_r.top + pill_r.bottom) * 0.5,
                            },
                            radiusX: 2.2,
                            radiusY: 2.2,
                        },
                        &self.solid_brush,
                    );
                }
                let badge_txt = if state.pomodoro.running { "  LIVE" } else { "READY" };
                self.draw_text(
                    badge_txt,
                    &self.fmt_wg_pill,
                    &pill_r,
                    (
                        0.76 + 0.16 * act,
                        0.68 + 0.16 * act,
                        1.0,
                        (0.84 + 0.16 * act) * card_alpha,
                    ),
                );

                let mins = state.pomodoro.today_focus_mins();
                let big = if state.pomodoro.running {
                    state.pomodoro.formatted_remaining()
                } else {
                    format!("{mins} min")
                };
                self.draw_text(
                    &big,
                    &self.fmt_wg_big,
                    &D2D_RECT_F {
                        left: rect.left + 12.0,
                        top: rect.top + 26.0,
                        right: rect.right - 10.0,
                        bottom: rect.top + 54.0,
                    },
                    (0.98, 0.98, 0.99, card_alpha),
                );
                let sess_txt = if state.pomodoro.running {
                    format!("{} · active", state.pomodoro.display_phase_label())
                } else if state.pomodoro.completed_sessions == 1 {
                    "1 session".to_string()
                } else {
                    format!("{} sessions", state.pomodoro.completed_sessions)
                };
                self.draw_text(
                    &sess_txt,
                    &self.fmt_wg_val,
                    &D2D_RECT_F {
                        left: rect.left + 12.0,
                        top: rect.top + 57.0,
                        right: rect.right - 72.0,
                        bottom: rect.top + 74.0,
                    },
                    (
                        0.72 - 0.04 * act,
                        0.72 - 0.14 * act,
                        0.76 + 0.24 * act,
                        card_alpha,
                    ),
                );
                let hint_txt = if state.pomodoro.running {
                    "Click to pause"
                } else {
                    "Click to start"
                };
                self.draw_text(
                    hint_txt,
                    &self.fmt_wg_sub,
                    &D2D_RECT_F {
                        left: rect.left + 12.0,
                        top: rect.top + 76.0,
                        right: rect.right - 72.0,
                        bottom: rect.top + 93.0,
                    },
                    (0.58 + 0.12 * hov, 0.60 + 0.12 * hov, 0.68 + 0.14 * hov, card_alpha),
                );

                let f_prog = springs.widget_focus_prog.value().clamp(0.0, 1.0);
                let mut bars = state.pomodoro.focus_history_bars();
                if bars[6] <= 0.01 && act > 0.01 {
                    bars[6] = (0.18 + 0.82 * f_prog).min(1.0);
                } else if act > 0.01 {
                    bars[6] = (bars[6] + 0.04 * act * pulse).min(1.0);
                }
                let base_y = rect.bottom - 12.0;
                for (i, &bh) in bars.iter().enumerate() {
                    let bx = rect.right - 66.0 + i as f32 * 8.0;
                    let has_data = bh > 0.01;
                    let h_px = if has_data {
                        (bh * 34.0 * (0.25 + 0.75 * enter)).max(6.0)
                    } else {
                        5.0
                    };
                    let bar_r = D2D_RECT_F {
                        left: bx,
                        top: base_y - h_px,
                        right: bx + 4.8,
                        bottom: base_y,
                    };
                    if i == 6 && (has_data || act > 0.05) {
                        self.set_color(0.62, 0.44, 1.0, (0.22 + 0.18 * act) * card_alpha);
                        self.d2d_context.FillRoundedRectangle(
                            &D2D1_ROUNDED_RECT {
                                rect: D2D_RECT_F {
                                    left: bar_r.left - 2.0,
                                    top: bar_r.top - 2.0,
                                    right: bar_r.right + 2.0,
                                    bottom: bar_r.bottom + 1.0,
                                },
                                radiusX: 3.4,
                                radiusY: 3.4,
                            },
                            &self.solid_brush,
                        );
                        self.set_color(0.66, 0.48, 1.0, 0.98 * card_alpha);
                    } else if has_data {
                        self.set_color(0.66, 0.52, 1.0, (0.48 + 0.14 * hov) * card_alpha);
                    } else {
                        self.set_color(1.0, 1.0, 1.0, (0.12 + 0.04 * hov) * card_alpha);
                    }
                    self.d2d_context.FillRoundedRectangle(
                        &D2D1_ROUNDED_RECT {
                            rect: bar_r,
                            radiusX: 2.4,
                            radiusY: 2.4,
                        },
                        &self.solid_brush,
                    );
                }
            }
            WidgetKind::WorldClock => {
                self.draw_text("WORLD CLOCK", &self.fmt_wg_label, &label_r, label_col);
                let c1 = WidgetState::world_clock_city(cfg.widget_city_1);
                let c2 = WidgetState::world_clock_city(cfg.widget_city_2);

                self.set_color(1.0, 1.0, 1.0, 0.08 * card_alpha);
                self.d2d_context.DrawLine(
                    D2D_POINT_2F {
                        x: rect.left + 12.0,
                        y: rect.top + 64.5,
                    },
                    D2D_POINT_2F {
                        x: rect.right - 12.0,
                        y: rect.top + 64.5,
                    },
                    &self.solid_brush,
                    0.85,
                    None,
                );

                let colon_alpha = (0.58 + 0.42 * (phase * std::f32::consts::PI).cos().abs()) * card_alpha;

                for (row, (city, time_s, delta_s)) in [c1, c2].iter().enumerate() {
                    let ry = rect.top + 26.5 + row as f32 * 40.5;
                    let city_hour: u32 = time_s
                        .get(..2)
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(12);
                    let is_day = (6..19).contains(&city_hour);

                    let icx = rect.left + 17.0;
                    let icy = ry + 11.5;
                    if is_day {
                        self.set_color(1.0, 0.78, 0.24, 0.22 * card_alpha);
                        self.d2d_context.FillEllipse(
                            &D2D1_ELLIPSE {
                                point: D2D_POINT_2F { x: icx, y: icy },
                                radiusX: 6.2,
                                radiusY: 6.2,
                            },
                            &self.solid_brush,
                        );
                        self.set_color(1.0, 0.82, 0.30, 0.98 * card_alpha);
                        self.d2d_context.FillEllipse(
                            &D2D1_ELLIPSE {
                                point: D2D_POINT_2F { x: icx, y: icy },
                                radiusX: 3.6,
                                radiusY: 3.6,
                            },
                            &self.solid_brush,
                        );
                    } else {
                        self.set_color(0.56, 0.66, 1.0, 0.20 * card_alpha);
                        self.d2d_context.FillEllipse(
                            &D2D1_ELLIPSE {
                                point: D2D_POINT_2F { x: icx, y: icy },
                                radiusX: 6.0,
                                radiusY: 6.0,
                            },
                            &self.solid_brush,
                        );
                        self.set_color(0.76, 0.84, 1.0, 0.96 * card_alpha);
                        self.d2d_context.FillEllipse(
                            &D2D1_ELLIPSE {
                                point: D2D_POINT_2F { x: icx, y: icy },
                                radiusX: 4.1,
                                radiusY: 4.1,
                            },
                            &self.solid_brush,
                        );
                        self.set_color(0.08, 0.09, 0.13, card_alpha);
                        self.d2d_context.FillEllipse(
                            &D2D1_ELLIPSE {
                                point: D2D_POINT_2F {
                                    x: icx + 2.0,
                                    y: icy - 1.4,
                                },
                                radiusX: 3.4,
                                radiusY: 3.4,
                            },
                            &self.solid_brush,
                        );
                    }

                    self.draw_text(
                        city,
                        &self.fmt_wg_val,
                        &D2D_RECT_F {
                            left: rect.left + 27.0,
                            top: ry + 0.5,
                            right: rect.right - 64.0,
                            bottom: ry + 18.0,
                        },
                        (0.97, 0.97, 0.99, card_alpha),
                    );
                    let meta = format!("{} · {}", delta_s, if is_day { "Day" } else { "Night" });
                    self.draw_text(
                        &meta,
                        &self.fmt_wg_sub,
                        &D2D_RECT_F {
                            left: rect.left + 27.0,
                            top: ry + 17.5,
                            right: rect.right - 64.0,
                            bottom: ry + 33.0,
                        },
                        (0.62, 0.65, 0.72, card_alpha),
                    );

                    let time_r = D2D_RECT_F {
                        left: rect.right - 72.0,
                        top: ry + 5.0,
                        right: rect.right - 11.0,
                        bottom: ry + 28.0,
                    };
                    if let Some((hh, mm)) = time_s.split_once(':') {
                        let mm_w = self.measure_width(mm, &self.fmt_wg_time);
                        let col_w_t = self.measure_width(":", &self.fmt_wg_time);
                        self.draw_text(mm, &self.fmt_wg_time, &time_r, (0.98, 0.98, 0.99, card_alpha));
                        self.draw_text(
                            ":",
                            &self.fmt_wg_time,
                            &D2D_RECT_F {
                                left: time_r.left,
                                top: time_r.top - 0.5,
                                right: time_r.right - mm_w,
                                bottom: time_r.bottom,
                            },
                            (0.98, 0.98, 0.99, colon_alpha),
                        );
                        self.draw_text(
                            hh,
                            &self.fmt_wg_time,
                            &D2D_RECT_F {
                                left: time_r.left,
                                top: time_r.top,
                                right: time_r.right - mm_w - col_w_t,
                                bottom: time_r.bottom,
                            },
                            (0.98, 0.98, 0.99, card_alpha),
                        );
                    } else {
                        self.draw_text(time_s, &self.fmt_wg_time, &time_r, (0.98, 0.98, 0.99, card_alpha));
                    }
                }
            }
            WidgetKind::Moon => {
                self.draw_text("MOON", &self.fmt_wg_label, &label_r, label_col);
                let (phase_name, illum, cycle, days_full) = WidgetState::moon_phase();
                let mx = rect.left + 36.0;
                let my = rect.top + ch * 0.58;
                let mr = 21.5;

                let stars = [
                    (rect.right - 18.0, rect.top + 14.0, 1.3_f32, 0.0_f32),
                    (rect.right - 30.0, rect.top + 18.5, 1.0, 1.8),
                    (rect.left + 15.0, rect.top + 32.0, 1.0, 3.4),
                    (rect.left + 58.0, rect.top + 28.0, 1.1, 4.9),
                ];
                for &(sx, sy, sr, off) in &stars {
                    let tw = 0.35 + 0.65 * (0.5 + 0.5 * (phase * 1.9 + off).sin());
                    self.set_color(0.88, 0.92, 1.0, tw * 0.55 * card_alpha);
                    self.d2d_context.FillEllipse(
                        &D2D1_ELLIPSE {
                            point: D2D_POINT_2F { x: sx, y: sy },
                            radiusX: sr,
                            radiusY: sr,
                        },
                        &self.solid_brush,
                    );
                }

                let halo_pulse = 0.5 + 0.5 * (phase * 1.2).sin();
                self.draw_radial_glow(
                    mx,
                    my,
                    mr + 14.0 + 2.0 * halo_pulse,
                    (0.84, 0.88, 1.0),
                    (0.24 + 0.08 * halo_pulse + 0.10 * hov) * card_alpha,
                );

                self.set_color(0.14, 0.15, 0.21, card_alpha);
                self.d2d_context.FillEllipse(
                    &D2D1_ELLIPSE {
                        point: D2D_POINT_2F { x: mx, y: my },
                        radiusX: mr,
                        radiusY: mr,
                    },
                    &self.solid_brush,
                );

                let waxing = cycle <= 0.5;
                let half_clip = if waxing {
                    D2D_RECT_F {
                        left: mx,
                        top: my - mr - 1.0,
                        right: mx + mr + 1.0,
                        bottom: my + mr + 1.0,
                    }
                } else {
                    D2D_RECT_F {
                        left: mx - mr - 1.0,
                        top: my - mr - 1.0,
                        right: mx,
                        bottom: my + mr + 1.0,
                    }
                };
                self.d2d_context
                    .PushAxisAlignedClip(&half_clip, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
                self.set_color(0.95, 0.93, 0.86, card_alpha);
                self.d2d_context.FillEllipse(
                    &D2D1_ELLIPSE {
                        point: D2D_POINT_2F { x: mx, y: my },
                        radiusX: mr,
                        radiusY: mr,
                    },
                    &self.solid_brush,
                );
                self.d2d_context.PopAxisAlignedClip();

                let term_rx = ((illum - 0.5).abs() * 2.0 * mr).clamp(0.4, mr);
                if illum >= 0.5 {
                    self.set_color(0.95, 0.93, 0.86, card_alpha);
                } else {
                    self.set_color(0.14, 0.15, 0.21, card_alpha);
                }
                self.d2d_context.FillEllipse(
                    &D2D1_ELLIPSE {
                        point: D2D_POINT_2F { x: mx, y: my },
                        radiusX: term_rx,
                        radiusY: mr,
                    },
                    &self.solid_brush,
                );

                self.set_color(0.45, 0.44, 0.42, 0.14 * card_alpha);
                for &(dx_c, dy_c, rc) in &[(-5.0_f32, -6.0_f32, 3.6_f32), (6.5, 4.5, 4.4), (-2.0, 7.0, 2.5)] {
                    self.d2d_context.FillEllipse(
                        &D2D1_ELLIPSE {
                            point: D2D_POINT_2F { x: mx + dx_c, y: my + dy_c },
                            radiusX: rc,
                            radiusY: rc,
                        },
                        &self.solid_brush,
                    );
                }
                self.set_color(1.0, 1.0, 1.0, 0.14 * card_alpha);
                self.d2d_context.DrawEllipse(
                    &D2D1_ELLIPSE {
                        point: D2D_POINT_2F { x: mx, y: my },
                        radiusX: mr,
                        radiusY: mr,
                    },
                    &self.solid_brush,
                    0.9,
                    None,
                );

                let phase_w = self.measure_width(phase_name, &self.fmt_wg_val);
                let avail_w = (rect.right - 8.0) - (rect.left + 66.0);
                let two_lines = phase_w > avail_w - 2.0;
                let pct_top = if two_lines { rect.top + 61.0 } else { rect.top + 52.0 };
                let full_top = if two_lines { rect.top + 79.0 } else { rect.top + 71.0 };
                self.draw_text(
                    phase_name,
                    &self.fmt_wg_val,
                    &D2D_RECT_F {
                        left: rect.left + 66.0,
                        top: rect.top + 30.0,
                        right: rect.right - 8.0,
                        bottom: rect.top + 61.0,
                    },
                    (0.97, 0.97, 0.99, card_alpha),
                );
                let pct_s = format!("{:.0}% lit", illum * 100.0);
                self.draw_text(
                    &pct_s,
                    &self.fmt_wg_val,
                    &D2D_RECT_F {
                        left: rect.left + 66.0,
                        top: pct_top,
                        right: rect.right - 8.0,
                        bottom: pct_top + 17.0,
                    },
                    (0.94, 0.90, 0.78, card_alpha),
                );
                let full_s = if days_full == 0 {
                    "Full tonight".to_string()
                } else {
                    format!("Full in {days_full}d")
                };
                self.draw_text(
                    &full_s,
                    &self.fmt_wg_sub,
                    &D2D_RECT_F {
                        left: rect.left + 66.0,
                        top: full_top,
                        right: rect.right - 8.0,
                        bottom: full_top + 16.0,
                    },
                    (0.64, 0.66, 0.72, card_alpha),
                );
            }
            WidgetKind::Countdown => {
                self.draw_text("COUNTDOWN", &self.fmt_wg_label, &label_r, label_col);
                let (ev_name, days_left, raw_prog) = WidgetState::countdown_info(cfg.widget_countdown);
                let anim_prog = (springs.widget_countdown_prog.value().clamp(0.0, 1.0))
                    .max(raw_prog * 0.01)
                    * (0.15 + 0.85 * enter);
                let pct_year = (raw_prog * 100.0).round() as u32;

                let pct_pill = D2D_RECT_F {
                    left: rect.right - 44.0,
                    top: rect.top + 7.5,
                    right: rect.right - 10.0,
                    bottom: rect.top + 20.5,
                };
                self.set_color(1.0, 0.64, 0.14, (0.18 + 0.10 * hov) * card_alpha);
                self.d2d_context.FillRoundedRectangle(
                    &D2D1_ROUNDED_RECT {
                        rect: pct_pill,
                        radiusX: 6.5,
                        radiusY: 6.5,
                    },
                    &self.solid_brush,
                );
                let pct_txt = format!("{pct_year}%");
                self.draw_text(
                    &pct_txt,
                    &self.fmt_wg_pill,
                    &pct_pill,
                    (1.0, 0.78, 0.36, 0.98 * card_alpha),
                );

                let days_s = format!("{days_left}");
                let days_w = self.measure_width(&days_s, &self.fmt_wg_big);
                self.draw_text(
                    &days_s,
                    &self.fmt_wg_big,
                    &D2D_RECT_F {
                        left: rect.left + 12.0,
                        top: rect.top + 31.0,
                        right: rect.left + 12.0 + days_w + 10.0,
                        bottom: rect.top + 61.0,
                    },
                    (0.99, 0.99, 1.0, card_alpha),
                );
                let text_l = (rect.left + 12.0 + days_w + 9.0).min(rect.right - 74.0);
                self.draw_text(
                    "days left",
                    &self.fmt_wg_val,
                    &D2D_RECT_F {
                        left: text_l,
                        top: rect.top + 33.0,
                        right: rect.right - 8.0,
                        bottom: rect.top + 50.0,
                    },
                    (0.96, 0.96, 0.98, card_alpha),
                );
                let to_s = format!("to {ev_name}");
                self.draw_text(
                    &to_s,
                    &self.fmt_wg_sub,
                    &D2D_RECT_F {
                        left: text_l,
                        top: rect.top + 50.5,
                        right: rect.right - 8.0,
                        bottom: rect.top + 67.5,
                    },
                    (0.68, 0.70, 0.76, card_alpha),
                );

                let bar_l = rect.left + 12.0;
                let bar_r = rect.right - 12.0;
                let bar_y = rect.bottom - 22.0;
                let bar_h = 7.0;
                self.set_color(1.0, 1.0, 1.0, 0.12 * card_alpha);
                self.d2d_context.FillRoundedRectangle(
                    &D2D1_ROUNDED_RECT {
                        rect: D2D_RECT_F {
                            left: bar_l,
                            top: bar_y,
                            right: bar_r,
                            bottom: bar_y + bar_h,
                        },
                        radiusX: 3.5,
                        radiusY: 3.5,
                    },
                    &self.solid_brush,
                );
                let fill_r_x = (bar_l + (bar_r - bar_l) * anim_prog).max(bar_l + bar_h);
                let bar_stops = [
                    D2D1_GRADIENT_STOP {
                        position: 0.0,
                        color: D2D1_COLOR_F {
                            r: 1.0,
                            g: 0.48,
                            b: 0.12,
                            a: 0.98 * card_alpha,
                        },
                    },
                    D2D1_GRADIENT_STOP {
                        position: 1.0,
                        color: D2D1_COLOR_F {
                            r: 1.0,
                            g: 0.80,
                            b: 0.24,
                            a: 0.98 * card_alpha,
                        },
                    },
                ];
                if let Ok(sc) = ID2D1RenderTarget::CreateGradientStopCollection(
                    &self.d2d_context,
                    &bar_stops,
                    D2D1_GAMMA_2_2,
                    D2D1_EXTEND_MODE_CLAMP,
                ) {
                    let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                        startPoint: D2D_POINT_2F { x: bar_l, y: bar_y },
                        endPoint: D2D_POINT_2F { x: fill_r_x, y: bar_y },
                    };
                    if let Ok(lg) = self.d2d_context.CreateLinearGradientBrush(&props, None, &sc) {
                        self.d2d_context.FillRoundedRectangle(
                            &D2D1_ROUNDED_RECT {
                                rect: D2D_RECT_F {
                                    left: bar_l,
                                    top: bar_y,
                                    right: fill_r_x,
                                    bottom: bar_y + bar_h,
                                },
                                radiusX: 3.5,
                                radiusY: 3.5,
                            },
                            &lg,
                        );
                    }
                }
                self.draw_radial_glow(
                    fill_r_x - 3.0,
                    bar_y + bar_h * 0.5,
                    9.5,
                    (1.0, 0.84, 0.36),
                    (0.55 + 0.20 * hov) * card_alpha,
                );
            }
            WidgetKind::Weather => {
                let (date_s, weekday_s, hour) = WidgetState::local_weather_date();
                let w = &state.widgets.weather;
                let is_night = if w.valid { !w.is_day } else { !(6..19).contains(&hour) };
                let wmo = if w.valid { w.wmo_code } else { 1 };
                let is_clear = wmo == 0;
                let is_rain = matches!(wmo, 51..=67 | 80..=82);
                let is_snow = matches!(wmo, 71..=77 | 85..=86);
                let is_storm = matches!(wmo, 95..=99);
                let is_overcast = matches!(wmo, 3 | 45 | 48) || is_rain || is_snow || is_storm;

                let bob = (phase * 1.4).sin() * 1.5;
                let glow_p = 0.5 + 0.5 * (phase * 1.1).sin();

                if is_night && !is_overcast {
                    for &(sx, sy, sr, off) in &[
                        (rect.left + 88.0, rect.top + 16.0, 1.1_f32, 0.4_f32),
                        (rect.left + 102.0, rect.top + 38.0, 0.95, 2.2),
                        (rect.right - 15.0, rect.top + 54.0, 1.15, 3.7),
                        (rect.left + 72.0, rect.top + 52.0, 0.9, 5.1),
                    ] {
                        let tw = 0.30 + 0.70 * (0.5 + 0.5 * (phase * 1.8 + off).sin());
                        self.set_color(0.92, 0.95, 1.0, tw * 0.65 * card_alpha);
                        self.d2d_context.FillEllipse(
                            &D2D1_ELLIPSE {
                                point: D2D_POINT_2F { x: sx, y: sy },
                                radiusX: sr,
                                radiusY: sr,
                            },
                            &self.solid_brush,
                        );
                    }
                }

                let hero_s = if w.valid {
                    format!("{}°", w.temp_c)
                } else {
                    date_s
                };
                self.draw_text(
                    &hero_s,
                    &self.fmt_wg_big,
                    &D2D_RECT_F {
                        left: rect.left + 13.0,
                        top: rect.top + 12.0,
                        right: rect.right - 52.0,
                        bottom: rect.top + 41.0,
                    },
                    (0.99, 0.99, 1.0, card_alpha),
                );

                let ix = rect.right - 29.0;
                let iy = rect.top + 29.0 + bob * 0.45;
                if !is_overcast {
                    let orb_x = if is_clear { ix } else { ix + 4.5 };
                    let orb_y = if is_clear { iy } else { iy - 4.5 };
                    if is_night {
                        self.draw_radial_glow(
                            orb_x,
                            orb_y,
                            16.5,
                            (0.98, 0.90, 0.56),
                            (0.32 + 0.12 * glow_p) * card_alpha,
                        );
                        self.set_color(0.98, 0.92, 0.60, 0.98 * card_alpha);
                        self.d2d_context.FillEllipse(
                            &D2D1_ELLIPSE {
                                point: D2D_POINT_2F { x: orb_x, y: orb_y },
                                radiusX: 8.2,
                                radiusY: 8.2,
                            },
                            &self.solid_brush,
                        );
                        self.set_color(0.09, 0.12, 0.24, card_alpha);
                        self.d2d_context.FillEllipse(
                            &D2D1_ELLIPSE {
                                point: D2D_POINT_2F { x: orb_x + 3.6, y: orb_y - 2.2 },
                                radiusX: 7.1,
                                radiusY: 7.1,
                            },
                            &self.solid_brush,
                        );
                    } else {
                        self.draw_radial_glow(
                            orb_x,
                            orb_y,
                            18.5,
                            (1.0, 0.88, 0.42),
                            (0.46 + 0.14 * glow_p) * card_alpha,
                        );
                        self.set_color(1.0, 0.84, 0.26, 0.98 * card_alpha);
                        self.d2d_context.FillEllipse(
                            &D2D1_ELLIPSE {
                                point: D2D_POINT_2F { x: orb_x, y: orb_y },
                                radiusX: 8.6,
                                radiusY: 8.6,
                            },
                            &self.solid_brush,
                        );
                    }
                }
                if !is_clear {
                    let cy_cloud = iy + if is_overcast { 0.0 } else { 3.2 } + bob * 0.55;
                    let cloud_col = if is_storm {
                        (0.76, 0.80, 0.90)
                    } else if is_rain {
                        (0.84, 0.89, 0.96)
                    } else {
                        (0.95, 0.97, 1.0)
                    };
                    self.set_color(0.84, 0.90, 0.98, 0.35 * card_alpha);
                    self.d2d_context.FillRoundedRectangle(
                        &D2D1_ROUNDED_RECT {
                            rect: D2D_RECT_F {
                                left: ix - 11.5,
                                top: cy_cloud + 0.5,
                                right: ix + 11.5,
                                bottom: cy_cloud + 7.8,
                            },
                            radiusX: 3.6,
                            radiusY: 3.6,
                        },
                        &self.solid_brush,
                    );
                    self.set_color(cloud_col.0, cloud_col.1, cloud_col.2, 0.96 * card_alpha);
                    self.d2d_context.FillEllipse(
                        &D2D1_ELLIPSE {
                            point: D2D_POINT_2F { x: ix - 4.5, y: cy_cloud },
                            radiusX: 6.5,
                            radiusY: 5.5,
                        },
                        &self.solid_brush,
                    );
                    self.d2d_context.FillEllipse(
                        &D2D1_ELLIPSE {
                            point: D2D_POINT_2F { x: ix + 3.2, y: cy_cloud - 1.2 },
                            radiusX: 7.5,
                            radiusY: 6.0,
                        },
                        &self.solid_brush,
                    );
                    self.d2d_context.FillRoundedRectangle(
                        &D2D1_ROUNDED_RECT {
                            rect: D2D_RECT_F {
                                left: ix - 11.0,
                                top: cy_cloud - 1.0,
                                right: ix + 11.0,
                                bottom: cy_cloud + 6.0,
                            },
                            radiusX: 3.5,
                            radiusY: 3.5,
                        },
                        &self.solid_brush,
                    );

                    if is_rain || is_storm {
                        for d in 0..3 {
                            let rt_drop = (phase * 1.9 + d as f32 * 0.33) % 1.0;
                            let dx_d = ix - 6.0 + d as f32 * 6.0;
                            let dy_d = cy_cloud + 7.5 + rt_drop * 8.0;
                            let da = (1.0 - rt_drop) * 0.90 * card_alpha;
                            self.set_color(0.56, 0.84, 1.0, da);
                            self.d2d_context.DrawLine(
                                D2D_POINT_2F { x: dx_d, y: dy_d },
                                D2D_POINT_2F { x: dx_d - 1.3, y: dy_d + 3.5 },
                                &self.solid_brush,
                                1.5,
                                None,
                            );
                        }
                    }
                    if is_snow {
                        for d in 0..3 {
                            let st = (phase * 0.9 + d as f32 * 0.35) % 1.0;
                            let sx = ix - 6.5 + d as f32 * 6.5 + (st * 6.0).sin() * 1.5;
                            let sy = cy_cloud + 7.5 + st * 8.5;
                            self.set_color(0.95, 0.98, 1.0, (1.0 - st) * 0.92 * card_alpha);
                            self.d2d_context.FillEllipse(
                                &D2D1_ELLIPSE {
                                    point: D2D_POINT_2F { x: sx, y: sy },
                                    radiusX: 1.5,
                                    radiusY: 1.5,
                                },
                                &self.solid_brush,
                            );
                        }
                    }
                    if is_storm {
                        let flash = 0.55 + 0.45 * (phase * 4.0).sin().abs();
                        self.set_color(1.0, 0.86, 0.28, flash * card_alpha);
                        self.d2d_context.DrawLine(
                            D2D_POINT_2F { x: ix + 1.0, y: cy_cloud + 5.0 },
                            D2D_POINT_2F { x: ix - 2.0, y: cy_cloud + 10.5 },
                            &self.solid_brush,
                            1.8,
                            None,
                        );
                        self.d2d_context.DrawLine(
                            D2D_POINT_2F { x: ix - 2.0, y: cy_cloud + 10.5 },
                            D2D_POINT_2F { x: ix + 1.8, y: cy_cloud + 10.5 },
                            &self.solid_brush,
                            1.8,
                            None,
                        );
                        self.d2d_context.DrawLine(
                            D2D_POINT_2F { x: ix + 1.8, y: cy_cloud + 10.5 },
                            D2D_POINT_2F { x: ix - 1.2, y: cy_cloud + 16.0 },
                            &self.solid_brush,
                            1.8,
                            None,
                        );
                    }
                }

                let bottom_title = if w.valid && !w.city.is_empty() {
                    w.city.as_str()
                } else {
                    weekday_s
                };
                self.draw_text(
                    bottom_title,
                    &self.fmt_wg_val,
                    &D2D_RECT_F {
                        left: rect.left + 13.0,
                        top: rect.bottom - 39.5,
                        right: rect.right - 12.0,
                        bottom: rect.bottom - 21.5,
                    },
                    (0.97, 0.98, 1.0, card_alpha),
                );
                let sky_s = if w.valid {
                    w.condition_label()
                } else if (6..11).contains(&hour) {
                    "Crisp morning"
                } else if (11..17).contains(&hour) {
                    "Partly sunny"
                } else if (17..20).contains(&hour) {
                    "Golden hour"
                } else {
                    "Clear evening"
                };
                self.draw_text(
                    sky_s,
                    &self.fmt_wg_sub,
                    &D2D_RECT_F {
                        left: rect.left + 13.0,
                        top: rect.bottom - 21.5,
                        right: rect.right - 12.0,
                        bottom: rect.bottom - 5.5,
                    },
                    (0.78, 0.84, 0.96, 0.94 * card_alpha),
                );
            }
        }

        self.d2d_context.PopAxisAlignedClip();
        self.d2d_context.SetTransform(&prev_xf);
    }

    unsafe fn draw_bottom_dock(
        &self,
        cfg: &Config,
        springs: &NotchSprings,
        layout: &ExpandedLayout,
        alpha: f32,
    ) {
        const ACTIVE_COLORS: [(f32, f32, f32); 7] = [
            (1.0, 0.329, 0.431),
            (0.549, 0.420, 1.0),
            (0.302, 0.678, 1.0),
            (1.0, 0.761, 0.278),
            (0.188, 0.780, 0.349),
            (0.039, 0.518, 1.0),
            (0.600, 0.600, 0.620),
        ];

        let raw_t = springs.dock_wobble();
        let dock_t = raw_t.clamp(0.0, 1.0);
        let brush: &ID2D1Brush = &self.solid_brush;
        let cy = (layout.base_h - 7.5) + 16.0 * raw_t;
        let gap = 4.25_f32;

        let (enabled_ids, page_centers, div_target_x, gear_center, dyn_dock_w) =
            compute_enabled_dock_layout(layout.cx, cy, cfg.page_order, cfg.page_enabled);
        let n = enabled_ids.len();

        let mut dot_ws = Vec::with_capacity(n);
        for &pid in &enabled_ids {
            dot_ws.push((4.0 + 11.0 * springs.dot_active[pid].value()).clamp(4.0, 17.5));
        }
        let total_dots_w: f32 =
            dot_ws.iter().sum::<f32>() + (n.saturating_sub(1) as f32) * gap;
        let mut rest_cx = Vec::with_capacity(n);
        let mut rx = layout.cx - total_dots_w * 0.5;
        for &dw in &dot_ws {
            rest_cx.push(rx + dw * 0.5);
            rx += dw + gap;
        }

        let dots_alpha = alpha * (1.0 - 1.45 * dock_t).clamp(0.0, 1.0);
        if dots_alpha > 0.01 {
            let dot_h = 4.0 + 3.0 * dock_t;
            let spread_t = (dock_t * 0.55).clamp(0.0, 1.0);
            for (slot, &pid) in enabled_ids.iter().enumerate() {
                let a_i = springs.dot_active[pid].value().clamp(0.0, 1.0);
                let dot_w = dot_ws[slot] + 2.5 * dock_t;
                let cx_i = rest_cx[slot] + (page_centers[slot].x - rest_cx[slot]) * spread_t;
                let rrect = D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: cx_i - dot_w * 0.5,
                        top: cy - dot_h * 0.5,
                        right: cx_i + dot_w * 0.5,
                        bottom: cy + dot_h * 0.5,
                    },
                    radiusX: dot_h * 0.5,
                    radiusY: dot_h * 0.5,
                };
                self.set_color(
                    0.34 + (0.96 - 0.34) * a_i,
                    0.36 + (0.97 - 0.36) * a_i,
                    0.40 + (0.99 - 0.40) * a_i,
                    (0.86 + 0.10 * a_i) * dots_alpha,
                );
                self.d2d_context
                    .FillRoundedRectangle(&rrect, &self.solid_brush);
            }
        }

        let dock_pill_alpha = alpha * ((dock_t - 0.05) / 0.85).clamp(0.0, 1.0);
        if dock_pill_alpha > 0.01 {
            let pill_t = ((raw_t - 0.04) / 0.96).max(0.0);
            let pill_w =
                (total_dots_w + (dyn_dock_w - total_dots_w) * pill_t).max(total_dots_w);
            let pill_h = (5.0 + (24.0 - 5.0) * pill_t).max(4.0);
            let pill = D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F {
                    left: layout.cx - pill_w * 0.5,
                    top: cy - pill_h * 0.5,
                    right: layout.cx + pill_w * 0.5,
                    bottom: cy + pill_h * 0.5,
                },
                radiusX: pill_h * 0.5,
                radiusY: pill_h * 0.5,
            };
            self.set_color(0.043, 0.043, 0.051, 0.96 * dock_pill_alpha);
            self.d2d_context
                .FillRoundedRectangle(&pill, &self.solid_brush);
            self.set_color(1.0, 1.0, 1.0, 0.07 * dock_pill_alpha);
            self.d2d_context
                .DrawRoundedRectangle(&pill, &self.solid_brush, 0.85, None);
        }

        let icons_alpha = alpha * ((dock_t - 0.14) / 0.86).clamp(0.0, 1.0);
        if icons_alpha > 0.01 {
            let div_t = ((dock_t - 0.10) / 0.90).clamp(0.0, 1.0);
            let div_x = layout.cx + (div_target_x - layout.cx) * div_t;
            self.set_color(1.0, 1.0, 1.0, 0.11 * icons_alpha);
            self.d2d_context.DrawLine(
                D2D_POINT_2F {
                    x: div_x,
                    y: cy - 6.0 * icons_alpha,
                },
                D2D_POINT_2F {
                    x: div_x,
                    y: cy + 6.0 * icons_alpha,
                },
                &self.solid_brush,
                1.0,
                None,
            );

            let spread = ((raw_t - 0.08) / 0.92).max(0.0);
            let overshoot = (raw_t - 1.0).max(0.0);
            let appear_scale = (0.45 + 0.55 * icons_alpha) * (1.0 + 0.35 * overshoot);

            for (slot, &pid) in enabled_ids.iter().enumerate() {
                let start_x = rest_cx[slot];
                let ix = start_x + (page_centers[slot].x - start_x) * spread;
                let iy = cy;
                let h_item = springs.dock_item_hover[pid].value().clamp(0.0, 1.1);
                let p_item = springs.dock_item_press[pid].value().clamp(0.0, 1.1);
                let active_i = springs.dot_active[pid].value().clamp(0.0, 1.0);

                let act_col = ACTIVE_COLORS[pid];
                let neutral = (
                    0.52 + 0.24 * h_item,
                    0.54 + 0.24 * h_item,
                    0.58 + 0.25 * h_item,
                );
                self.set_color(
                    neutral.0 + (act_col.0 - neutral.0) * active_i,
                    neutral.1 + (act_col.1 - neutral.1) * active_i,
                    neutral.2 + (act_col.2 - neutral.2) * active_i,
                    (0.92 + 0.08 * active_i.max(h_item)) * icons_alpha,
                );

                let item_scale = (1.0 + 0.11 * h_item - 0.15 * p_item).clamp(0.72, 1.25);
                let icon_size = 14.2 * appear_scale * item_scale;
                match pid {
                    0 => icons::draw_music_note(&self.d2d_context, ix, iy, icon_size, brush),
                    1 => icons::draw_stopwatch_stroked(&self.d2d_context, ix, iy, icon_size, brush, 1.4),
                    2 => icons::draw_water_drop_stroked(&self.d2d_context, ix, iy, icon_size, brush, 1.4),
                    3 => icons::draw_shelf_tray_stroked(&self.d2d_context, ix, iy, icon_size, brush, 1.4),
                    4 => icons::draw_checklist_stroked(&self.d2d_context, ix, iy, icon_size, brush, 1.4),
                    _ => icons::draw_widgets_grid(&self.d2d_context, ix, iy, icon_size, brush),
                }
            }

            let start_x = layout.cx + total_dots_w * 0.5;
            let ix = start_x + (gear_center.x - start_x) * spread;
            let iy = cy;
            let h_item = springs.dock_item_hover[6].value().clamp(0.0, 1.1);
            let p_item = springs.dock_item_press[6].value().clamp(0.0, 1.1);
            self.set_color(
                0.52 + 0.24 * h_item,
                0.54 + 0.24 * h_item,
                0.58 + 0.25 * h_item,
                (0.92 + 0.08 * h_item) * icons_alpha,
            );
            let item_scale = (1.0 + 0.11 * h_item - 0.15 * p_item).clamp(0.72, 1.25);
            let icon_size = 14.2 * appear_scale * item_scale;
            icons::draw_settings_gear_stroked(&self.d2d_context, ix, iy, icon_size, brush, 1.35);
        }
    }

    unsafe fn draw_shelf_select_pill(
        &self,
        rect: D2D_RECT_F,
        sel_t: f32,
        hover_amt: f32,
        alpha: f32,
    ) {
        let t = sel_t.clamp(0.0, 1.0);
        let rrect = D2D1_ROUNDED_RECT {
            rect,
            radiusX: 8.0,
            radiusY: 8.0,
        };
        let fill_r = 1.0 + (0.231 - 1.0) * t;
        let fill_g = 1.0 + (0.510 - 1.0) * t;
        let fill_b = 1.0 + (0.965 - 1.0) * t;
        let fill_a = ((0.10 + 0.08 * hover_amt) * (1.0 - t) + (0.28 + 0.10 * hover_amt) * t) * alpha;
        self.set_color(fill_r, fill_g, fill_b, fill_a);
        self.d2d_context.FillRoundedRectangle(&rrect, &self.solid_brush);

        let br_r = 1.0 + (0.376 - 1.0) * t;
        let br_g = 1.0 + (0.647 - 1.0) * t;
        let br_b = 1.0 + (0.980 - 1.0) * t;
        let br_a = ((0.14 + 0.10 * hover_amt) * (1.0 - t) + (0.48 + 0.20 * hover_amt) * t) * alpha;
        self.set_color(br_r, br_g, br_b, br_a);
        self.d2d_context
            .DrawRoundedRectangle(&rrect, &self.solid_brush, 0.9 + 0.1 * t, None);

        let fg_a = (0.92 + 0.06 * hover_amt) * alpha;
        if (1.0 - t) > 0.01 {
            self.draw_text(
                "Select",
                &self.fmt_pill_sm,
                &rect,
                (0.953, 0.957, 0.965, fg_a * (1.0 - t)),
            );
        }
        if t > 0.01 {
            self.draw_text(
                "Done",
                &self.fmt_pill_sm,
                &rect,
                (0.953, 0.957, 0.965, fg_a * t),
            );
        }
    }

    unsafe fn draw_shelf_clear_all_pill(&self, rect: D2D_RECT_F, hover_amt: f32, alpha: f32) {
        let rrect = D2D1_ROUNDED_RECT {
            rect,
            radiusX: 8.0,
            radiusY: 8.0,
        };
        self.set_color(0.937, 0.267, 0.267, (0.18 + 0.12 * hover_amt) * alpha);
        self.d2d_context.FillRoundedRectangle(&rrect, &self.solid_brush);
        self.set_color(0.973, 0.443, 0.443, (0.35 + 0.20 * hover_amt) * alpha);
        self.d2d_context
            .DrawRoundedRectangle(&rrect, &self.solid_brush, 1.0, None);
        self.draw_text(
            "Clear all",
            &self.fmt_pill_sm,
            &rect,
            (0.988, 0.647, 0.647, (0.94 + 0.06 * hover_amt) * alpha),
        );
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn draw_shelf_ambient_glow(
        &self,
        cx: f32,
        px: f32,
        base_h: f32,
        base_glow: f32,
        pool_glow: f32,
        rise: f32,
    ) {
        let glow_left = cx - 230.0;
        let glow_right = cx + 230.0;
        let glow_rect = D2D_RECT_F {
            left: glow_left,
            top: 0.0,
            right: glow_right,
            bottom: base_h + 24.0,
        };

        let mask_stops = [
            D2D1_GRADIENT_STOP {
                position: 0.0,
                color: D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 1.0 },
            },
            D2D1_GRADIENT_STOP {
                position: 0.42,
                color: D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.55 },
            },
            D2D1_GRADIENT_STOP {
                position: 0.78,
                color: D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.08 },
            },
            D2D1_GRADIENT_STOP {
                position: 1.0,
                color: D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 },
            },
        ];
        let opacity_brush: Option<ID2D1Brush> = ID2D1RenderTarget::CreateGradientStopCollection(
            &self.d2d_context,
            &mask_stops,
            D2D1_GAMMA_2_2,
            D2D1_EXTEND_MODE_CLAMP,
        )
        .ok()
        .and_then(|stops| {
            let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                startPoint: D2D_POINT_2F { x: cx, y: base_h },
                endPoint: D2D_POINT_2F { x: cx, y: 0.0 },
            };
            self.d2d_context
                .CreateLinearGradientBrush(&props, None, &stops)
                .ok()
        })
        .and_then(|b| b.cast().ok());

        let mut layer_params = D2D1_LAYER_PARAMETERS1 {
            contentBounds: glow_rect,
            geometricMask: ManuallyDrop::new(None),
            maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
            maskTransform: Matrix3x2::identity(),
            opacity: 1.0,
            opacityBrush: ManuallyDrop::new(opacity_brush),
            layerOptions: D2D1_LAYER_OPTIONS1_NONE,
        };
        self.d2d_context.PushLayer(&layer_params, None);

        if base_glow > 0.004 {
            let band_stops = [
                D2D1_GRADIENT_STOP {
                    position: 0.0,
                    color: D2D1_COLOR_F {
                        r: 40.0 / 255.0,
                        g: 170.0 / 255.0,
                        b: 240.0 / 255.0,
                        a: base_glow,
                    },
                },
                D2D1_GRADIENT_STOP {
                    position: 0.5,
                    color: D2D1_COLOR_F {
                        r: 52.0 / 255.0,
                        g: 104.0 / 255.0,
                        b: 236.0 / 255.0,
                        a: base_glow,
                    },
                },
                D2D1_GRADIENT_STOP {
                    position: 1.0,
                    color: D2D1_COLOR_F {
                        r: 104.0 / 255.0,
                        g: 78.0 / 255.0,
                        b: 222.0 / 255.0,
                        a: base_glow,
                    },
                },
            ];
            if let Ok(sc) = ID2D1RenderTarget::CreateGradientStopCollection(
                &self.d2d_context,
                &band_stops,
                D2D1_GAMMA_2_2,
                D2D1_EXTEND_MODE_CLAMP,
            ) {
                let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                    startPoint: D2D_POINT_2F { x: glow_left, y: 0.0 },
                    endPoint: D2D_POINT_2F { x: glow_right, y: 0.0 },
                };
                if let Ok(lin) = self.d2d_context.CreateLinearGradientBrush(&props, None, &sc) {
                    self.d2d_context.FillRectangle(&glow_rect, &lin);
                }
            }
        }

        if pool_glow > 0.004 {
            let u = ((px - glow_left) / (glow_right - glow_left)).clamp(0.0, 1.0);
            let (mix_r, mix_g, mix_b) = if u < 0.5 {
                let k = u * 2.0;
                (
                    (40.0 + (52.0 - 40.0) * k) / 255.0,
                    (170.0 + (104.0 - 170.0) * k) / 255.0,
                    (240.0 + (236.0 - 240.0) * k) / 255.0,
                )
            } else {
                let k = (u - 0.5) * 2.0;
                (
                    (52.0 + (104.0 - 52.0) * k) / 255.0,
                    (104.0 + (78.0 - 104.0) * k) / 255.0,
                    (236.0 + (222.0 - 236.0) * k) / 255.0,
                )
            };
            let hi_r = (mix_r + 51.0 / 255.0).min(1.0);
            let hi_g = (mix_g + 51.0 / 255.0).min(1.0);
            let hi_b = (mix_b + 51.0 / 255.0).min(1.0);
            let pool_stops = [
                D2D1_GRADIENT_STOP {
                    position: 0.0,
                    color: D2D1_COLOR_F {
                        r: hi_r,
                        g: hi_g,
                        b: hi_b,
                        a: pool_glow,
                    },
                },
                D2D1_GRADIENT_STOP {
                    position: 0.42,
                    color: D2D1_COLOR_F {
                        r: mix_r,
                        g: mix_g,
                        b: mix_b,
                        a: pool_glow * 0.48,
                    },
                },
                D2D1_GRADIENT_STOP {
                    position: 0.74,
                    color: D2D1_COLOR_F {
                        r: mix_r,
                        g: mix_g,
                        b: mix_b,
                        a: pool_glow * 0.14,
                    },
                },
                D2D1_GRADIENT_STOP {
                    position: 1.0,
                    color: D2D1_COLOR_F {
                        r: mix_r,
                        g: mix_g,
                        b: mix_b,
                        a: 0.0,
                    },
                },
            ];
            if let Ok(sc) = ID2D1RenderTarget::CreateGradientStopCollection(
                &self.d2d_context,
                &pool_stops,
                D2D1_GAMMA_2_2,
                D2D1_EXTEND_MODE_CLAMP,
            ) {
                let radius = 210.0 + 70.0 * rise;
                let rprops = D2D1_RADIAL_GRADIENT_BRUSH_PROPERTIES {
                    center: D2D_POINT_2F {
                        x: px,
                        y: base_h + 18.0,
                    },
                    gradientOriginOffset: D2D_POINT_2F { x: 0.0, y: 0.0 },
                    radiusX: radius,
                    radiusY: radius,
                };
                if let Ok(rad) = self.d2d_context.CreateRadialGradientBrush(&rprops, None, &sc) {
                    self.d2d_context.FillRectangle(&glow_rect, &rad);
                }
            }
        }

        self.d2d_context.PopLayer();
        ManuallyDrop::drop(&mut layer_params.opacityBrush);
    }

    unsafe fn draw_shelf_drop_beam(
        &self,
        px: f32,
        base_h: f32,
        drop_t: f32,
        alpha: f32,
        notch_geo: Option<&ID2D1PathGeometry>,
    ) {
        let q = 1.0 - (1.0 - (drop_t / 1.45).clamp(0.0, 1.0)).powi(3);
        let r = 24.0 + 430.0 * q;
        let peak_a = (1.0 - q) * 0.34 * alpha;
        let origin = D2D_POINT_2F {
            x: px,
            y: base_h - 14.0,
        };

        let draw_feathered_ring = |center_r: f32, half_w: f32, peak: f32| {
            if peak <= 0.002 {
                return;
            }
            let outer_r = (center_r + half_w).max(8.0);
            let inner0 = (center_r - half_w).max(0.0);
            let inner1 = (center_r - half_w * 0.32).max(inner0 + 1.0);
            let outer1 = (center_r + half_w * 0.32).min(outer_r - 1.0);
            let s0 = (inner0 / outer_r).clamp(0.0, 0.85);
            let s1 = (inner1 / outer_r).clamp(s0 + 0.01, 0.90);
            let sm = (center_r / outer_r).clamp(s1 + 0.01, 0.94);
            let s2 = (outer1 / outer_r).clamp(sm + 0.01, 0.98);
            let stops = [
                D2D1_GRADIENT_STOP {
                    position: s0,
                    color: D2D1_COLOR_F { r: 0.40, g: 0.78, b: 1.0, a: 0.0 },
                },
                D2D1_GRADIENT_STOP {
                    position: s1,
                    color: D2D1_COLOR_F { r: 0.50, g: 0.84, b: 1.0, a: peak * 0.45 },
                },
                D2D1_GRADIENT_STOP {
                    position: sm,
                    color: D2D1_COLOR_F { r: 0.784, g: 0.925, b: 1.0, a: peak },
                },
                D2D1_GRADIENT_STOP {
                    position: s2,
                    color: D2D1_COLOR_F { r: 0.50, g: 0.84, b: 1.0, a: peak * 0.40 },
                },
                D2D1_GRADIENT_STOP {
                    position: 1.0,
                    color: D2D1_COLOR_F { r: 0.40, g: 0.78, b: 1.0, a: 0.0 },
                },
            ];
            if let Ok(sc) = ID2D1RenderTarget::CreateGradientStopCollection(
                &self.d2d_context,
                &stops,
                D2D1_GAMMA_2_2,
                D2D1_EXTEND_MODE_CLAMP,
            ) {
                let rprops = D2D1_RADIAL_GRADIENT_BRUSH_PROPERTIES {
                    center: origin,
                    gradientOriginOffset: D2D_POINT_2F { x: 0.0, y: 0.0 },
                    radiusX: outer_r,
                    radiusY: outer_r,
                };
                if let Ok(rad) = self.d2d_context.CreateRadialGradientBrush(&rprops, None, &sc) {
                    if let Some(geo) = notch_geo {
                        self.d2d_context.FillGeometry(geo, &rad, None);
                    } else {
                        self.d2d_context.FillEllipse(
                            &D2D1_ELLIPSE {
                                point: origin,
                                radiusX: outer_r,
                                radiusY: outer_r,
                            },
                            &rad,
                        );
                    }
                }
            }
        };

        draw_feathered_ring(r, 38.0, peak_a);
        let inner_r = r * 0.68;
        if inner_r > 8.0 {
            draw_feathered_ring(inner_r, 24.0, peak_a * 0.45);
        }

        if let Some(geo) = notch_geo {
            let rise_a = (1.0 - (drop_t / 1.15).clamp(0.0, 1.0)).clamp(0.0, 1.0) * alpha;
            if rise_a > 0.01 {
                let stops = [
                    D2D1_GRADIENT_STOP {
                        position: 0.0,
                        color: D2D1_COLOR_F {
                            r: 0.784,
                            g: 0.925,
                            b: 1.0,
                            a: 0.72 * rise_a,
                        },
                    },
                    D2D1_GRADIENT_STOP {
                        position: 0.45,
                        color: D2D1_COLOR_F {
                            r: 0.494,
                            g: 0.698,
                            b: 1.0,
                            a: 0.34 * rise_a,
                        },
                    },
                    D2D1_GRADIENT_STOP {
                        position: 1.0,
                        color: D2D1_COLOR_F {
                            r: 0.157,
                            g: 0.667,
                            b: 0.941,
                            a: 0.0,
                        },
                    },
                ];
                if let Ok(sc) = ID2D1RenderTarget::CreateGradientStopCollection(
                    &self.d2d_context,
                    &stops,
                    D2D1_GAMMA_2_2,
                    D2D1_EXTEND_MODE_CLAMP,
                ) {
                    let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                        startPoint: D2D_POINT_2F { x: px, y: base_h },
                        endPoint: D2D_POINT_2F { x: px, y: 0.0 },
                    };
                    if let Ok(lin) = self.d2d_context.CreateLinearGradientBrush(&props, None, &sc) {
                        self.d2d_context.DrawGeometry(geo, &lin, 3.0, None);
                    }
                }
            }
        }
    }

    unsafe fn fill_linear_gradient_rrect(
        &self,
        rrect: &D2D1_ROUNDED_RECT,
        c0: (f32, f32, f32, f32),
        c1: (f32, f32, f32, f32),
    ) {
        let stops = [
            D2D1_GRADIENT_STOP {
                position: 0.0,
                color: D2D1_COLOR_F {
                    r: c0.0,
                    g: c0.1,
                    b: c0.2,
                    a: c0.3,
                },
            },
            D2D1_GRADIENT_STOP {
                position: 1.0,
                color: D2D1_COLOR_F {
                    r: c1.0,
                    g: c1.1,
                    b: c1.2,
                    a: c1.3,
                },
            },
        ];
        if let Ok(stop_col) = ID2D1RenderTarget::CreateGradientStopCollection(
            &self.d2d_context,
            &stops,
            D2D1_GAMMA_2_2,
            D2D1_EXTEND_MODE_CLAMP,
        ) {
            let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                startPoint: D2D_POINT_2F {
                    x: rrect.rect.left,
                    y: rrect.rect.top,
                },
                endPoint: D2D_POINT_2F {
                    x: rrect.rect.right,
                    y: rrect.rect.bottom,
                },
            };
            if let Ok(grad) = self
                .d2d_context
                .CreateLinearGradientBrush(&props, None, &stop_col)
            {
                self.d2d_context.FillRoundedRectangle(rrect, &grad);
                return;
            }
        }
        self.set_color(c0.0, c0.1, c0.2, c0.3);
        self.d2d_context
            .FillRoundedRectangle(rrect, &self.solid_brush);
    }

    unsafe fn draw_rounded_art_bmp(
        &self,
        bmp: &ID2D1Bitmap1,
        rect: &D2D_RECT_F,
        radius: f32,
        alpha: f32,
    ) {
        if alpha <= 0.005 {
            return;
        }
        let bmp_size = bmp.GetSize();
        if bmp_size.width <= 0.0 || bmp_size.height <= 0.0 {
            return;
        }
        let target_w = (rect.right - rect.left).max(1.0);
        let target_h = (rect.bottom - rect.top).max(1.0);
        let scale = (target_w / bmp_size.width).max(target_h / bmp_size.height);
        let draw_w = bmp_size.width * scale;
        let draw_h = bmp_size.height * scale;
        let tx = rect.left + (target_w - draw_w) * 0.5;
        let ty = rect.top + (target_h - draw_h) * 0.5;

        let bb_props = D2D1_BITMAP_BRUSH_PROPERTIES1 {
            extendModeX: D2D1_EXTEND_MODE_CLAMP,
            extendModeY: D2D1_EXTEND_MODE_CLAMP,
            interpolationMode: D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
        };
        let brush_props = windows::Win32::Graphics::Direct2D::D2D1_BRUSH_PROPERTIES {
            opacity: alpha,
            transform: Matrix3x2 {
                M11: scale,
                M12: 0.0,
                M21: 0.0,
                M22: scale,
                M31: tx,
                M32: ty,
            },
        };
        if let Ok(brush) = self
            .d2d_context
            .CreateBitmapBrush(bmp, Some(&bb_props), Some(&brush_props))
        {
            let rrect = D2D1_ROUNDED_RECT {
                rect: *rect,
                radiusX: radius,
                radiusY: radius,
            };
            self.d2d_context.FillRoundedRectangle(&rrect, &brush);
        }
    }

    unsafe fn draw_rounded_art(&self, rect: &D2D_RECT_F, radius: f32, alpha: f32, fade: f32) {
        let f = fade.clamp(0.0, 1.0);
        if f < 0.995 {
            if let Some(prev) = &self.prev_art_bitmap {
                self.draw_rounded_art_bmp(prev, rect, radius, alpha * (1.0 - f));
            }
        }
        if let Some(bmp) = &self.cached_art_bitmap {
            let cur_a = if self.prev_art_bitmap.is_some() && f < 0.995 {
                alpha * f
            } else {
                alpha
            };
            self.draw_rounded_art_bmp(bmp, rect, radius, cur_a);
        }
    }

    unsafe fn draw_text(
        &self,
        text: &str,
        format: &IDWriteTextFormat,
        rect: &D2D_RECT_F,
        rgba: (f32, f32, f32, f32),
    ) {
        self.draw_text_smooth(text, format, rect, rgba);
    }

    unsafe fn draw_text_smooth(
        &self,
        text: &str,
        format: &IDWriteTextFormat,
        rect: &D2D_RECT_F,
        rgba: (f32, f32, f32, f32),
    ) {
        let wide: Vec<u16> = text.encode_utf16().collect();
        self.set_color(rgba.0, rgba.1, rgba.2, rgba.3);
        self.d2d_context.DrawText(
            &wide,
            format,
            rect,
            &self.solid_brush,
            D2D1_DRAW_TEXT_OPTIONS_CLIP | D2D1_DRAW_TEXT_OPTIONS_NO_SNAP,
            DWRITE_MEASURING_MODE_NATURAL,
        );
    }
}

fn create_opaque_surface(
    d2d_context: &ID2D1DeviceContext,
    w_px: u32,
    h_px: u32,
    dpi: f32,
) -> Option<(ID2D1Bitmap1, ID2D1BitmapBrush1)> {
    let size = D2D_SIZE_U {
        width: w_px.max(2),
        height: h_px.max(2),
    };
    let props = D2D1_BITMAP_PROPERTIES1 {
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            alphaMode: D2D1_ALPHA_MODE_IGNORE,
        },
        dpiX: dpi,
        dpiY: dpi,
        bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET,
        ..Default::default()
    };
    let bmp = unsafe { d2d_context.CreateBitmap(size, None, 0, &props) }.ok()?;
    let bb_props = D2D1_BITMAP_BRUSH_PROPERTIES1 {
        extendModeX: D2D1_EXTEND_MODE_CLAMP,
        extendModeY: D2D1_EXTEND_MODE_CLAMP,
        interpolationMode: D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
    };
    let brush_props = D2D1_BRUSH_PROPERTIES {
        opacity: 1.0,
        transform: Matrix3x2::identity(),
    };
    let brush = unsafe {
        d2d_context.CreateBitmapBrush(&bmp, Some(&bb_props), Some(&brush_props))
    }
    .ok()?;
    Some((bmp, brush))
}

fn rgb_to_hsl(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) * 0.5;
    let d = max - min;
    if d <= 1e-5 {
        return (0.0, 0.0, l);
    }
    let s = if l > 0.5 {
        d / (2.0 - max - min).max(1e-5)
    } else {
        d / (max + min).max(1e-5)
    };
    let h = if (max - r).abs() <= 1e-5 {
        ((g - b) / d + if g < b { 6.0 } else { 0.0 }) / 6.0
    } else if (max - g).abs() <= 1e-5 {
        ((b - r) / d + 2.0) / 6.0
    } else {
        ((r - g) / d + 4.0) / 6.0
    };
    (h.rem_euclid(1.0), s.clamp(0.0, 1.0), l.clamp(0.0, 1.0))
}

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (f32, f32, f32) {
    if s <= 1e-5 {
        return (l, l, l);
    }
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let hue_to_rgb = |t_raw: f32| -> f32 {
        let t = t_raw.rem_euclid(1.0);
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    (
        hue_to_rgb(h + 1.0 / 3.0).clamp(0.0, 1.0),
        hue_to_rgb(h).clamp(0.0, 1.0),
        hue_to_rgb(h - 1.0 / 3.0).clamp(0.0, 1.0),
    )
}

#[inline]
fn atlas_cell_metrics(dpi: f32) -> (u32, f32) {
    let scale = (dpi / 96.0).max(0.5);
    let cell_px = (ATLAS_CELL_DIPS * scale * 2.0).round().max(32.0) as u32;
    let atlas_dpi = 96.0 * (cell_px as f32 / ATLAS_CELL_DIPS);
    (cell_px, atlas_dpi)
}

fn create_speaker_2x_bmp(d2d_context: &ID2D1DeviceContext, dpi: f32) -> Option<ID2D1Bitmap1> {
    let (cell_px, atlas_dpi) = atlas_cell_metrics(dpi);
    let props = D2D1_BITMAP_PROPERTIES1 {
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
        },
        dpiX: atlas_dpi,
        dpiY: atlas_dpi,
        bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET,
        ..Default::default()
    };
    let size = D2D_SIZE_U {
        width: cell_px,
        height: cell_px,
    };
    unsafe { d2d_context.CreateBitmap(size, None, 0, &props) }.ok()
}

fn create_and_bake_icon_atlas_2x(
    d2d_context: &ID2D1DeviceContext,
    solid_brush: &ID2D1SolidColorBrush,
    alt_brush: &ID2D1SolidColorBrush,
    dpi: f32,
) -> Option<ID2D1Bitmap1> {
    let (cell_px, atlas_dpi) = atlas_cell_metrics(dpi);
    let props = D2D1_BITMAP_PROPERTIES1 {
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
        },
        dpiX: atlas_dpi,
        dpiY: atlas_dpi,
        bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET,
        ..Default::default()
    };
    let size = D2D_SIZE_U {
        width: cell_px * ATLAS_COLS as u32,
        height: cell_px * ATLAS_ROWS as u32,
    };
    let atlas = unsafe { d2d_context.CreateBitmap(size, None, 0, &props) }.ok()?;
    let brush: &ID2D1Brush = solid_brush;
    let alt: &ID2D1Brush = alt_brush;

    unsafe {
        d2d_context.SetTarget(&atlas);
        d2d_context.SetDpi(atlas_dpi, atlas_dpi);
        d2d_context.BeginDraw();
        d2d_context.Clear(Some(&D2D1_COLOR_F {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.0,
        }));

        for (slot, &bake_size) in ATLAS_BAKE_SIZES.iter().enumerate() {
            let cx = (slot % ATLAS_COLS) as f32 * ATLAS_CELL_DIPS + ATLAS_CELL_DIPS * 0.5;
            let cy = (slot / ATLAS_COLS) as f32 * ATLAS_CELL_DIPS + ATLAS_CELL_DIPS * 0.5;
            solid_brush.SetColor(&D2D1_COLOR_F {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 1.0,
            });
            alt_brush.SetColor(&D2D1_COLOR_F {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 1.0,
            });

            match slot {
                ATLAS_SLOT_PLAY => icons::draw_play_triangle(d2d_context, cx, cy, bake_size, brush),
                ATLAS_SLOT_PAUSE => icons::draw_pause_bars(d2d_context, cx, cy, bake_size, brush),
                ATLAS_SLOT_PREV => icons::draw_prev_track(d2d_context, cx, cy, bake_size, brush),
                ATLAS_SLOT_NEXT => icons::draw_next_track(d2d_context, cx, cy, bake_size, brush),
                ATLAS_SLOT_LYRICS_STROKED => {
                    icons::draw_lyrics_bubble(d2d_context, cx, cy, bake_size, brush)
                }
                ATLAS_SLOT_LYRICS_FILLED => {
                    icons::draw_lyrics_bubble_filled(d2d_context, cx, cy, bake_size, brush, alt)
                }
                ATLAS_SLOT_MUSIC_NOTE => {
                    icons::draw_music_note(d2d_context, cx, cy, bake_size, brush)
                }
                ATLAS_SLOT_CAPS_ON => {
                    icons::draw_caps_lock(d2d_context, cx, cy, bake_size, brush, alt, true)
                }
                ATLAS_SLOT_CAPS_OFF => {
                    solid_brush.SetColor(&D2D1_COLOR_F {
                        r: 0.72,
                        g: 0.74,
                        b: 0.78,
                        a: 0.94,
                    });
                    alt_brush.SetColor(&D2D1_COLOR_F {
                        r: 0.48,
                        g: 0.50,
                        b: 0.55,
                        a: 0.90,
                    });
                    icons::draw_caps_lock(d2d_context, cx, cy, bake_size, brush, alt, false);
                }
                ATLAS_SLOT_BELL => icons::draw_bell(d2d_context, cx, cy, bake_size, brush, None),
                _ => {}
            }
        }

        if d2d_context.EndDraw(None, None).is_err() {
            return None;
        }
    }
    Some(atlas)
}

fn is_image_filename(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".png")
        || lower.ends_with(".jpg")
        || lower.ends_with(".jpeg")
        || lower.ends_with(".webp")
        || lower.ends_with(".bmp")
        || lower.ends_with(".gif")
}

fn create_notch_geometry(
    factory: &ID2D1Factory1,
    left: f32,
    total_w: f32,
    h: f32,
    ear_r: f32,
    bottom_r: f32,
) -> Option<ID2D1PathGeometry> {
    let ear_r = ear_r.max(0.5).min(total_w * 0.25).min((h * 0.45).max(0.5));
    let body_w = (total_w - 2.0 * ear_r).max(1.0);
    let bottom_r = bottom_r
        .max(0.5)
        .min(body_w * 0.49)
        .min((h - ear_r).max(0.5));

    let right = left + total_w;
    let body_left = left + ear_r;
    let body_right = right - ear_r;
    let ear_size = D2D_SIZE_F {
        width: ear_r,
        height: ear_r,
    };
    let bot_size = D2D_SIZE_F {
        width: bottom_r,
        height: bottom_r,
    };

    unsafe {
        let base_factory: &ID2D1Factory = factory;
        let geo = base_factory.CreatePathGeometry().ok()?;
        let sink = geo.Open().ok()?;

        sink.BeginFigure(D2D_POINT_2F { x: left, y: 0.0 }, D2D1_FIGURE_BEGIN_FILLED);
        sink.AddLine(D2D_POINT_2F { x: right, y: 0.0 });

        sink.AddArc(&D2D1_ARC_SEGMENT {
            point: D2D_POINT_2F {
                x: body_right,
                y: ear_r,
            },
            size: ear_size,
            rotationAngle: 0.0,
            sweepDirection: D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE,
            arcSize: D2D1_ARC_SIZE_SMALL,
        });

        sink.AddLine(D2D_POINT_2F {
            x: body_right,
            y: h - bottom_r,
        });

        sink.AddArc(&D2D1_ARC_SEGMENT {
            point: D2D_POINT_2F {
                x: body_right - bottom_r,
                y: h,
            },
            size: bot_size,
            rotationAngle: 0.0,
            sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
            arcSize: D2D1_ARC_SIZE_SMALL,
        });

        sink.AddLine(D2D_POINT_2F {
            x: body_left + bottom_r,
            y: h,
        });

        sink.AddArc(&D2D1_ARC_SEGMENT {
            point: D2D_POINT_2F {
                x: body_left,
                y: h - bottom_r,
            },
            size: bot_size,
            rotationAngle: 0.0,
            sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
            arcSize: D2D1_ARC_SIZE_SMALL,
        });

        sink.AddLine(D2D_POINT_2F {
            x: body_left,
            y: ear_r,
        });

        sink.AddArc(&D2D1_ARC_SEGMENT {
            point: D2D_POINT_2F { x: left, y: 0.0 },
            size: ear_size,
            rotationAngle: 0.0,
            sweepDirection: D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE,
            arcSize: D2D1_ARC_SIZE_SMALL,
        });

        sink.EndFigure(D2D1_FIGURE_END_CLOSED);
        sink.Close().ok()?;
        Some(geo)
    }
}

fn create_d3d11_and_factory() -> Result<(ID3D11Device, IDXGIFactory2)> {
    let feature_levels = [
        D3D_FEATURE_LEVEL_11_1,
        D3D_FEATURE_LEVEL_11_0,
        D3D_FEATURE_LEVEL_10_1,
        D3D_FEATURE_LEVEL_10_0,
    ];
    let mut device: Option<ID3D11Device> = None;

    let res = unsafe {
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            Some(&feature_levels),
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            None,
        )
    };

    if res.is_err() || device.is_none() {
        unsafe {
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_WARP,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&feature_levels),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                None,
            )?;
        }
    }

    let device = device.ok_or_else(windows::core::Error::from_win32)?;
    let dxgi_device: IDXGIDevice = device.cast()?;
    let adapter = unsafe { dxgi_device.GetAdapter()? };
    let factory: IDXGIFactory2 = unsafe { adapter.GetParent()? };
    Ok((device, factory))
}

fn create_swap_chain_target(
    d2d_context: &ID2D1DeviceContext,
    swap_chain: &IDXGISwapChain1,
    dpi: f32,
) -> Result<ID2D1Bitmap1> {
    let surface: IDXGISurface = unsafe { swap_chain.GetBuffer(0)? };
    let props = D2D1_BITMAP_PROPERTIES1 {
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
        },
        dpiX: dpi,
        dpiY: dpi,
        bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
        ..Default::default()
    };
    let target = unsafe { d2d_context.CreateBitmapFromDxgiSurface(&surface, Some(&props))? };
    unsafe {
        d2d_context.SetTarget(&target);
        d2d_context.SetDpi(dpi, dpi);
    }
    Ok(target)
}

fn create_eave_radial_brushes(
    d2d_context: &ID2D1DeviceContext,
) -> [Option<ID2D1RadialGradientBrush>; 4] {
    let colors = [
        (0.00_f32, 0.88_f32, 0.98_f32),
        (0.12, 0.54, 1.00),
        (0.58, 0.28, 1.00),
        (0.56, 0.80, 1.00),
    ];
    colors.map(|(r, g, b)| unsafe {
        let stops = [
            D2D1_GRADIENT_STOP {
                position: 0.0,
                color: D2D1_COLOR_F { r, g, b, a: 1.0 },
            },
            D2D1_GRADIENT_STOP {
                position: 1.0,
                color: D2D1_COLOR_F { r, g, b, a: 0.0 },
            },
        ];
        ID2D1RenderTarget::CreateGradientStopCollection(
            d2d_context,
            &stops,
            D2D1_GAMMA_2_2,
            D2D1_EXTEND_MODE_CLAMP,
        )
        .ok()
        .and_then(|sc| {
            let props = D2D1_RADIAL_GRADIENT_BRUSH_PROPERTIES {
                center: D2D_POINT_2F { x: 0.0, y: 0.0 },
                gradientOriginOffset: D2D_POINT_2F { x: 0.0, y: 0.0 },
                radiusX: 10.0,
                radiusY: 10.0,
            };
            d2d_context
                .CreateRadialGradientBrush(&props, None, &sc)
                .ok()
        })
    })
}

fn pick_ui_font_family(dwrite: &IDWriteFactory) -> HSTRING {
    let mut collection: Option<IDWriteFontCollection> = None;
    if unsafe { dwrite.GetSystemFontCollection(&mut collection, false) }.is_ok() {
        if let Some(col) = collection {
            for candidate in [
                w!("Segoe UI Variable Text"),
                w!("Segoe UI Variable"),
                w!("Segoe UI"),
            ] {
                let mut index = 0u32;
                let mut exists = BOOL(0);
                if unsafe { col.FindFamilyName(candidate, &mut index, &mut exists) }.is_ok()
                    && exists.as_bool()
                {
                    return HSTRING::from(unsafe {
                        candidate.to_string().unwrap_or_else(|_| "Segoe UI".into())
                    });
                }
            }
        }
    }
    HSTRING::from("Segoe UI")
}

fn create_text_format(
    dwrite: &IDWriteFactory,
    family: &HSTRING,
    size: f32,
    weight: windows::Win32::Graphics::DirectWrite::DWRITE_FONT_WEIGHT,
    style: windows::Win32::Graphics::DirectWrite::DWRITE_FONT_STYLE,
    align: windows::Win32::Graphics::DirectWrite::DWRITE_TEXT_ALIGNMENT,
    trim_ellipsis: bool,
) -> Result<IDWriteTextFormat> {
    unsafe {
        let fmt = dwrite.CreateTextFormat(
            family,
            None,
            weight,
            style,
            DWRITE_FONT_STRETCH_NORMAL,
            size,
            w!("en-US"),
        )?;
        fmt.SetTextAlignment(align)?;
        fmt.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
        fmt.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
        if trim_ellipsis {
            if let Ok(sign) = dwrite.CreateEllipsisTrimmingSign(&fmt) {
                let trimming = DWRITE_TRIMMING {
                    granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
                    delimiter: 0,
                    delimiterCount: 0,
                };
                let _ = fmt.SetTrimming(&trimming, &sign);
            }
        }
        Ok(fmt)
    }
}

fn local_system_time(offset_secs: u64) -> Option<SYSTEMTIME> {
    let unix_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        + offset_secs;
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
            return Some(local_st);
        }
    }
    None
}

fn format_header_datetime() -> (String, String, &'static str) {
    if let Some(local_st) = local_system_time(0) {
        const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
        const MONTHS: [&str; 12] = [
            "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
        ];
        let dow = DAYS.get(local_st.wDayOfWeek as usize).copied().unwrap_or("");
        let mon = MONTHS
            .get((local_st.wMonth as usize).saturating_sub(1))
            .copied()
            .unwrap_or("");
        let date = format!("{dow} {} {mon} {}", local_st.wDay, local_st.wYear);

        let (hour12, ampm) = match local_st.wHour {
            0 => (12, "AM"),
            1..=11 => (local_st.wHour, "AM"),
            12 => (12, "PM"),
            h => (h - 12, "PM"),
        };
        let time = format!("{hour12}:{:02}", local_st.wMinute);
        return (date, time, ampm);
    }
    ("Today".to_string(), "12:00".to_string(), "PM")
}

fn format_future_clock_24h(remaining_secs: f32) -> String {
    let add = remaining_secs.max(0.0).ceil() as u64;
    if let Some(st) = local_system_time(add) {
        return format!("{:02}:{:02}", st.wHour, st.wMinute);
    }
    "00:00".to_string()
}

fn format_mm_ss(secs: f64) -> String {
    let total = secs.max(0.0).floor() as u64;
    let m = total / 60;
    let s = total % 60;
    format!("{m:02}:{s:02}")
}

fn format_remaining_mm_ss(duration_secs: f64, position_secs: f64) -> String {
    let rem = (duration_secs - position_secs).max(0.0).ceil() as u64;
    let m = rem / 60;
    let s = rem % 60;
    format!("-{m:02}:{s:02}")
}

fn shelf_icon_hash(item: &ShelfItem) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in item.path.as_bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    let dims = (item.icon_width as u64)
        ^ ((item.icon_height as u64) << 16)
        ^ ((item.icon_bgra.as_ref().map_or(0, |v| v.len()) as u64) << 32);
    h ^= dims;
    h.wrapping_mul(0x0100_0000_01b3)
}

/// Extracts a weighted vibrant ambient RGB `(0..=1)` tint from premultiplied BGRA8 album art.
#[allow(dead_code)]
pub fn extract_dominant_song_color(bgra: &[u8]) -> (f32, f32, f32) {
    if bgra.len() < 4 {
        return (0.22, 0.47, 0.81);
    }
    let pixel_count = bgra.len() / 4;
    let step = (pixel_count / 1024).max(1);
    let mut sum_r = 0.0_f32;
    let mut sum_g = 0.0_f32;
    let mut sum_b = 0.0_f32;
    let mut sum_w = 0.0_f32;

    for idx in (0..pixel_count).step_by(step) {
        let base = idx * 4;
        let a = bgra[base + 3] as f32 / 255.0;
        if a <= 0.08 {
            continue;
        }
        let inv_a = 1.0 / a;
        let b = (bgra[base] as f32 / 255.0 * inv_a).clamp(0.0, 1.0);
        let g = (bgra[base + 1] as f32 / 255.0 * inv_a).clamp(0.0, 1.0);
        let r = (bgra[base + 2] as f32 / 255.0 * inv_a).clamp(0.0, 1.0);

        let max_c = r.max(g).max(b);
        let min_c = r.min(g).min(b);
        let chroma = max_c - min_c;
        let sat = if max_c > 0.001 { chroma / max_c } else { 0.0 };
        let luma = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        if !(0.06..=0.94).contains(&luma) {
            continue;
        }
        let w = (0.12 + sat * 2.8) * (0.25 + luma.min(0.80));
        sum_r += r * w;
        sum_g += g * w;
        sum_b += b * w;
        sum_w += w;
    }

    if sum_w <= 0.001 {
        return (0.22, 0.47, 0.81);
    }

    let mut r = sum_r / sum_w;
    let mut g = sum_g / sum_w;
    let mut b = sum_b / sum_w;
    let max_c = r.max(g).max(b);
    let min_c = r.min(g).min(b);
    let chroma = max_c - min_c;
    if chroma < 0.08 {
        let gray = ((r + g + b) / 3.0).clamp(0.42, 0.72);
        return (gray, gray, gray);
    }

    let avg = (r + g + b) / 3.0;
    r = (avg + (r - avg) * 1.55).max(0.0);
    g = (avg + (g - avg) * 1.55).max(0.0);
    b = (avg + (b - avg) * 1.55).max(0.0);
    let peak = r.max(g).max(b).max(0.001);
    let target_peak = peak.clamp(0.56, 0.84);
    let scale = target_peak / peak;
    (
        (r * scale).clamp(0.06, 0.86),
        (g * scale).clamp(0.06, 0.86),
        (b * scale).clamp(0.06, 0.86),
    )
}

fn mul_matrix3x2(a: Matrix3x2, b: Matrix3x2) -> Matrix3x2 {
    Matrix3x2 {
        M11: a.M11 * b.M11 + a.M12 * b.M21,
        M12: a.M11 * b.M12 + a.M12 * b.M22,
        M21: a.M21 * b.M11 + a.M22 * b.M21,
        M22: a.M21 * b.M12 + a.M22 * b.M22,
        M31: a.M31 * b.M11 + a.M32 * b.M21 + b.M31,
        M32: a.M31 * b.M12 + a.M32 * b.M22 + b.M32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hit_test_collapsed_returns_body_only() {
        assert_eq!(
            hit_test_notch(
                79.0,
                15.0,
                158.0,
                172.0,
                30.0,
                6.5,
                0.0,
                0.0,
                false,
                NotchPage::Music,
                ShelfTab::Files,
                0,
                0,
                0,
                false,
                false,
                0.0,
            ),
            HitTarget::Body
        );
        assert_eq!(
            hit_test_notch(
                -5.0,
                15.0,
                158.0,
                172.0,
                30.0,
                6.5,
                0.0,
                0.0,
                false,
                NotchPage::Music,
                ShelfTab::Files,
                0,
                0,
                0,
                false,
                false,
                0.0,
            ),
            HitTarget::None
        );
    }

    #[test]
    fn hit_test_expanded_music_dock_water_and_chevrons() {
        let (w, base_h, cur_h, ear) = (438.0, 172.0, 202.0, 22.0);
        let layout = ExpandedLayout::compute(w * 0.5, w - 2.0 * ear, base_h);
        assert!((layout.chevron_left_center.y - 94.0).abs() < 0.01);
        assert!((layout.chevron_right_center.y - 94.0).abs() < 0.01);

        assert_eq!(
            hit_test_notch(
                layout.play_center.x,
                layout.play_center.y,
                w,
                base_h,
                cur_h,
                ear,
                1.0,
                0.0,
                false,
                NotchPage::Music,
                ShelfTab::Files,
                0,
                0,
                0,
                false,
                false,
                0.0,
            ),
            HitTarget::PlayPauseButton
        );
        assert_eq!(
            hit_test_notch(
                layout.chevron_left_center.x,
                layout.chevron_left_center.y,
                w,
                base_h,
                cur_h,
                ear,
                1.0,
                0.0,
                false,
                NotchPage::Music,
                ShelfTab::Files,
                0,
                0,
                0,
                false,
                false,
                0.0,
            ),
            HitTarget::ChevronLeft
        );
        assert_eq!(
            hit_test_notch(
                layout.bell_center.x,
                layout.bell_center.y,
                w,
                base_h,
                cur_h,
                ear,
                1.0,
                0.0,
                false,
                NotchPage::Music,
                ShelfTab::Files,
                0,
                0,
                0,
                false,
                false,
                0.0,
            ),
            HitTarget::Body
        );
        assert_eq!(
            hit_test_notch(
                layout.bell_center.x,
                layout.bell_center.y,
                w,
                base_h,
                cur_h,
                ear,
                1.0,
                0.0,
                true,
                NotchPage::Music,
                ShelfTab::Files,
                0,
                0,
                0,
                false,
                false,
                0.0,
            ),
            HitTarget::BellButton
        );
        assert_eq!(
            hit_test_notch(
                layout.dock_item_centers[3].x,
                layout.dock_item_centers[3].y,
                w,
                base_h,
                cur_h,
                ear,
                1.0,
                1.0,
                false,
                NotchPage::Music,
                ShelfTab::Files,
                0,
                0,
                0,
                false,
                false,
                0.0,
            ),
            HitTarget::DockPage(NotchPage::Shelf)
        );
        assert_eq!(
            hit_test_notch(
                layout.dock_item_centers[5].x,
                layout.dock_item_centers[5].y,
                w,
                base_h,
                cur_h,
                ear,
                1.0,
                1.0,
                false,
                NotchPage::Music,
                ShelfTab::Files,
                0,
                0,
                0,
                false,
                false,
                0.0,
            ),
            HitTarget::DockSettings
        );
        let (sw, _, _) = water_controls(layout.body_left);
        assert_eq!(
            hit_test_notch(
                (sw.left + sw.right) * 0.5,
                (sw.top + sw.bottom) * 0.5,
                w,
                base_h,
                cur_h,
                ear,
                1.0,
                0.0,
                false,
                NotchPage::Water,
                ShelfTab::Files,
                0,
                0,
                0,
                false,
                false,
                0.0,
            ),
            HitTarget::WaterToggleReminder
        );
    }

    #[test]
    fn hit_test_todo_controls_and_inline_form() {
        let (w, h, ear) = (438.0, 172.0, 22.0);
        let layout = ExpandedLayout::compute(w * 0.5, w - 2.0 * ear, h);
        let cal = todo_calendar_center(layout.body_left);
        assert_eq!(
            hit_test_notch(
                cal.x,
                cal.y,
                w,
                h,
                h,
                ear,
                1.0,
                0.0,
                false,
                NotchPage::Todo,
                ShelfTab::Files,
                0,
                0,
                0,
                false,
                false,
                0.0,
            ),
            HitTarget::TodoCalendar
        );
        let (add_r, edit_r) = todo_button_rects(layout.body_left);
        assert_eq!(
            hit_test_notch(
                (add_r.left + add_r.right) * 0.5,
                (add_r.top + add_r.bottom) * 0.5,
                w,
                h,
                h,
                ear,
                1.0,
                0.0,
                false,
                NotchPage::Todo,
                ShelfTab::Files,
                0,
                0,
                0,
                false,
                false,
                0.0,
            ),
            HitTarget::TodoAdd
        );
        assert_eq!(
            hit_test_notch(
                (edit_r.left + edit_r.right) * 0.5,
                (edit_r.top + edit_r.bottom) * 0.5,
                w,
                h,
                h,
                ear,
                1.0,
                0.0,
                false,
                NotchPage::Todo,
                ShelfTab::Files,
                0,
                0,
                1,
                false,
                false,
                0.0,
            ),
            HitTarget::TodoEditToggle
        );
        let (_panel, _inp, cancel_r, confirm_r) = todo_inline_form_rects(layout.body_left);
        assert_eq!(
            hit_test_notch(
                (cancel_r.left + cancel_r.right) * 0.5,
                (cancel_r.top + cancel_r.bottom) * 0.5,
                w,
                h,
                h,
                ear,
                1.0,
                0.0,
                false,
                NotchPage::Todo,
                ShelfTab::Files,
                0,
                0,
                0,
                true,
                false,
                0.0,
            ),
            HitTarget::TodoCancelAdd
        );
        assert_eq!(
            hit_test_notch(
                (confirm_r.left + confirm_r.right) * 0.5,
                (confirm_r.top + confirm_r.bottom) * 0.5,
                w,
                h,
                h,
                ear,
                1.0,
                0.0,
                false,
                NotchPage::Todo,
                ShelfTab::Files,
                0,
                0,
                0,
                true,
                false,
                0.0,
            ),
            HitTarget::TodoConfirmAdd
        );
        assert!(hit_target_slot(HitTarget::TodoCalendar) == Some(35));
        assert!(hit_target_slot(HitTarget::TodoConfirmAdd) == Some(36));
        assert!(hit_target_slot(HitTarget::TodoCancelAdd) == Some(37));
    }

    #[test]
    fn format_mm_ss_and_remaining() {
        assert_eq!(format_mm_ss(-4.0), "00:00");
        assert_eq!(format_mm_ss(65.9), "01:05");
        assert_eq!(format_remaining_mm_ss(120.0, 45.0), "-01:15");
        assert_eq!(format_remaining_mm_ss(0.0, 0.0), "-00:00");
    }

    #[test]
    fn shelf_zone_and_clipboard_switcher_hit_tests() {
        let w = 438.0;
        let h = 172.0;
        let ear = 22.0;
        let layout = ExpandedLayout::compute(w * 0.5, w - 2.0 * ear, h);
        assert!(is_in_shelf_zone(w * 0.5, 96.0, w, h, ear));
        assert!(!is_in_shelf_zone(w * 0.5, 16.0, w, h, ear));
        assert_eq!(
            hit_test_notch(
                w * 0.5,
                96.0,
                w,
                h,
                h,
                ear,
                1.0,
                0.0,
                false,
                NotchPage::Shelf,
                ShelfTab::Files,
                0,
                4,
                0,
                false,
                false,
                0.0,
            ),
            HitTarget::ShelfDropZone
        );

        let (pill, files_r, clip_r) = shelf_switcher_rects(layout.body_right);
        assert!((pill.right - pill.left - 64.0).abs() < 0.01);
        assert!((pill.bottom - pill.top - 20.0).abs() < 0.01);
        assert_eq!(
            hit_test_notch(
                (files_r.left + files_r.right) * 0.5,
                (files_r.top + files_r.bottom) * 0.5,
                w,
                h,
                h,
                ear,
                1.0,
                0.0,
                false,
                NotchPage::Shelf,
                ShelfTab::Clipboard,
                0,
                4,
                0,
                false,
                false,
                0.0,
            ),
            HitTarget::ShelfTabFiles
        );
        assert_eq!(
            hit_test_notch(
                (clip_r.left + clip_r.right) * 0.5,
                (clip_r.top + clip_r.bottom) * 0.5,
                w,
                h,
                h,
                ear,
                1.0,
                0.0,
                false,
                NotchPage::Shelf,
                ShelfTab::Files,
                0,
                4,
                0,
                false,
                false,
                0.0,
            ),
            HitTarget::ShelfTabClipboard
        );
        let chip0 = shelf_card_rect(layout.body_left, 0);
        assert_eq!(
            hit_test_notch(
                (chip0.left + chip0.right) * 0.5,
                (chip0.top + chip0.bottom) * 0.5,
                w,
                h,
                h,
                ear,
                1.0,
                0.0,
                false,
                NotchPage::Shelf,
                ShelfTab::Files,
                3,
                4,
                0,
                false,
                false,
                0.0,
            ),
            HitTarget::ShelfChip(0)
        );
        let (sel_r, clear_r) = shelf_bottom_button_rects(layout.body_left, layout.body_right);
        assert_eq!(
            hit_test_notch(
                (sel_r.left + sel_r.right) * 0.5,
                (sel_r.top + sel_r.bottom) * 0.5,
                w,
                h,
                h,
                ear,
                1.0,
                0.0,
                false,
                NotchPage::Shelf,
                ShelfTab::Files,
                3,
                4,
                0,
                false,
                false,
                0.0,
            ),
            HitTarget::ShelfSelectToggle
        );
        assert_eq!(
            hit_test_notch(
                (clear_r.left + clear_r.right) * 0.5,
                (clear_r.top + clear_r.bottom) * 0.5,
                w,
                h,
                h,
                ear,
                1.0,
                0.0,
                false,
                NotchPage::Shelf,
                ShelfTab::Files,
                3,
                4,
                0,
                false,
                false,
                0.0,
            ),
            HitTarget::ShelfClear
        );
        assert_eq!(
            hit_test_notch(
                (clear_r.left + clear_r.right) * 0.5,
                (clear_r.top + clear_r.bottom) * 0.5,
                w,
                h,
                h,
                ear,
                1.0,
                0.0,
                false,
                NotchPage::Shelf,
                ShelfTab::Clipboard,
                0,
                4,
                0,
                false,
                false,
                0.0,
            ),
            HitTarget::ShelfClear
        );
        let cb0 = cb_card_rect(layout.body_left, 1);
        assert!((cb0.right - cb0.left - 77.0).abs() < 0.01);
        assert!((cb0.bottom - cb0.top - 81.0).abs() < 0.01);
        assert_eq!(
            hit_test_notch(
                (cb0.left + cb0.right) * 0.5,
                (cb0.top + cb0.bottom) * 0.5,
                w,
                h,
                h,
                ear,
                1.0,
                0.0,
                false,
                NotchPage::Shelf,
                ShelfTab::Clipboard,
                0,
                4,
                0,
                false,
                false,
                0.0,
            ),
            HitTarget::ClipboardCard(1)
        );
        assert_eq!(hit_target_slot(HitTarget::ShelfSelectToggle), Some(38));
        assert!(hit_target_slot(HitTarget::ClipboardCard(1)).is_some());
        assert!(hit_target_slot(HitTarget::ShelfTabClipboard).is_some());
        assert_eq!(
            hit_test_notch(
                (sel_r.left + sel_r.right) * 0.5,
                (sel_r.top + sel_r.bottom) * 0.5,
                w,
                h,
                h + 30.0,
                ear,
                1.0,
                1.0,
                false,
                NotchPage::Shelf,
                ShelfTab::Files,
                3,
                4,
                0,
                false,
                false,
                0.0,
            ),
            HitTarget::ShelfSelectToggle
        );
        assert_eq!(
            hit_test_notch(
                (clear_r.left + clear_r.right) * 0.5,
                (clear_r.top + clear_r.bottom) * 0.5,
                w,
                h,
                h + 30.0,
                ear,
                1.0,
                1.0,
                false,
                NotchPage::Shelf,
                ShelfTab::Files,
                3,
                4,
                0,
                false,
                false,
                0.0,
            ),
            HitTarget::ShelfClear
        );
    }

    #[test]
    fn hit_test_music_lyrics_split_mode_and_route_lock() {
        let (w, h, ear) = (564.0, 172.0, 22.0);
        let bl = ear;
        assert_eq!(
            hit_test_notch(
                bl + 53.0,
                136.5,
                w,
                h,
                h,
                ear,
                1.0,
                0.0,
                false,
                NotchPage::Music,
                ShelfTab::Files,
                0,
                0,
                0,
                false,
                true,
                0.0,
            ),
            HitTarget::LyricsButton
        );
        assert_eq!(
            hit_test_notch(
                bl + 131.0,
                136.5,
                w,
                h,
                h,
                ear,
                1.0,
                0.0,
                false,
                NotchPage::Music,
                ShelfTab::Files,
                0,
                0,
                0,
                false,
                true,
                0.0,
            ),
            HitTarget::PlayPauseButton
        );
        assert_eq!(
            hit_test_notch(
                bl + 320.0,
                56.0,
                w,
                h,
                h,
                ear,
                1.0,
                0.0,
                false,
                NotchPage::Music,
                ShelfTab::Files,
                0,
                0,
                0,
                false,
                true,
                0.0,
            ),
            HitTarget::LyricSeek(0)
        );
        assert_eq!(
            hit_test_notch(
                bl + 320.0,
                95.0,
                w,
                h,
                h,
                ear,
                1.0,
                0.0,
                false,
                NotchPage::Music,
                ShelfTab::Files,
                0,
                0,
                0,
                false,
                true,
                0.0,
            ),
            HitTarget::LyricSeek(1)
        );
        assert_eq!(
            hit_test_notch(
                bl + 320.0,
                136.0,
                w,
                h,
                h,
                ear,
                1.0,
                0.0,
                false,
                NotchPage::Music,
                ShelfTab::Files,
                0,
                0,
                0,
                false,
                true,
                0.0,
            ),
            HitTarget::LyricSeek(2)
        );
        assert_eq!(hit_target_slot(HitTarget::LyricSeek(0)), Some(39));
        assert_eq!(hit_target_slot(HitTarget::LyricSeek(2)), Some(41));

        assert_eq!(
            hit_test_notch(
                bl + 11.0,
                94.0,
                w,
                h,
                h,
                ear,
                1.0,
                0.0,
                false,
                NotchPage::Music,
                ShelfTab::Files,
                0,
                0,
                0,
                false,
                true,
                0.0,
            ),
            HitTarget::Body
        );
        assert_eq!(
            hit_test_notch(
                w * 0.5,
                h - 5.0,
                w,
                h,
                h,
                ear,
                1.0,
                0.0,
                false,
                NotchPage::Music,
                ShelfTab::Files,
                0,
                0,
                0,
                false,
                true,
                0.0,
            ),
            HitTarget::Body
        );
    }

    #[test]
    fn dominant_song_color_extracts_warm_rgb_and_falls_back() {
        let fallback = extract_dominant_song_color(&[]);
        assert_eq!(fallback, (0.22, 0.47, 0.81));

        // Warm amber/orange premultiplied BGRA pixels -> R > G > B.
        let warm = [30u8, 120u8, 220u8, 255u8].repeat(16);
        let (r, g, b) = extract_dominant_song_color(&warm);
        assert!(r > g && g > b, "expected warm R > G > B, got ({r}, {g}, {b})");
    }

    #[test]
    fn hit_test_music_volume_slider_expands_left_of_speaker() {
        let (w, h, ear) = (438.0, 172.0, 22.0);
        let layout = ExpandedLayout::compute(w * 0.5, w - 2.0 * ear, h);
        let (full_rect, _, _) = music_volume_slider_rects(layout.speaker_btn_center, 1.0);
        let mid_x = (full_rect.left + full_rect.right) * 0.5;
        let y = layout.speaker_btn_center.y;

        assert_eq!(
            hit_test_notch(
                mid_x,
                y,
                w,
                h,
                h,
                ear,
                1.0,
                0.0,
                false,
                NotchPage::Music,
                ShelfTab::Files,
                0,
                0,
                0,
                false,
                false,
                0.0,
            ),
            HitTarget::Body
        );

        assert_eq!(
            hit_test_notch(
                layout.speaker_btn_center.x,
                y,
                w,
                h,
                h,
                ear,
                1.0,
                0.0,
                false,
                NotchPage::Music,
                ShelfTab::Files,
                0,
                0,
                0,
                false,
                false,
                1.0,
            ),
            HitTarget::SpeakerButton
        );
        assert_eq!(
            hit_test_notch(
                layout.next_center.x,
                y,
                w,
                h,
                h,
                ear,
                1.0,
                0.0,
                false,
                NotchPage::Music,
                ShelfTab::Files,
                0,
                0,
                0,
                false,
                false,
                1.0,
            ),
            HitTarget::NextButton
        );
        match hit_test_notch(
            mid_x,
            y,
            w,
            h,
            h,
            ear,
            1.0,
            0.0,
            false,
            NotchPage::Music,
            ShelfTab::Files,
            0,
            0,
            0,
            false,
            false,
            1.0,
        ) {
            HitTarget::VolumeSlider(r) => assert!((r - 0.5).abs() < 0.02, "got {r}"),
            other => panic!("expected VolumeSlider, got {other:?}"),
        }
        assert_eq!(hit_target_slot(HitTarget::VolumeSlider(0.25)), Some(42));
    }
}
