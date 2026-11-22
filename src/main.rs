#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod ai;
mod compositor;
mod config;
mod icons;
mod lyrics;
mod markdown;
mod media;
mod ocr;
mod osd_monitors;
mod pages;
mod physics;
mod renderer;
mod settings_ui;
mod shelf;
mod updater;
mod window;

use std::cell::RefCell;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::process::Command;
use std::rc::Rc;
use std::thread;
use std::time::{Duration, Instant};

use compositor::CompositorHost;
use config::Config;
use lyrics::{LyricLinePair, LyricsService};
use media::{MediaCommand, MediaService};
use pages::{NotchPage, PomodoroState, TodoState, WaterState, WidgetState};
use physics::{HoverStateMachine, NotchSprings};
use renderer::{
    AlbumArtView, CollapsedOsdKind, EaveViewState, HitTarget, MediaViewState, NotchRenderer,
};
use settings_ui::{SettingsAction, SettingsSnapshot, SettingsWindow};
use shelf::{ShelfStore, ShelfTab};
use window::{is_caps_lock_on, NotchWindow, TrayAction, WindowEvent};
use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::UI::Input::KeyboardAndMouse::{DragDetect, ReleaseCapture};
use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

const SHADOW_PAD_X_DIPS: f32 = 61.0;
const SHADOW_PAD_BOTTOM_DIPS: f32 = 58.0;
const DOCK_EXTRA_H_DIPS: f32 = 30.0;

const MASCOT_IDLE_AWAKE_SECS: f32 = 20.0;
const MASCOT_IDLE_SLEEP_SECS: f32 = 40.0;
const MASCOT_WAKE_LINGER_SECS: f32 = 5.0;
const AUTO_RING_OVERRIDE_SECS: u64 = 10;

fn system_idle_secs() -> f32 {
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
    #[link(name = "kernel32")]
    extern "system" {
        fn GetTickCount() -> u32;
    }
    let mut lii = LASTINPUTINFO {
        cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
        dwTime: 0,
    };
    let ok = unsafe { GetLastInputInfo(&mut lii) }.as_bool();
    if !ok {
        return 0.0;
    }
    let now = unsafe { GetTickCount() };
    now.wrapping_sub(lii.dwTime) as f32 / 1000.0
}

fn tray_menu_state_from_cfg(cfg: &Config) -> window::TrayMenuState {
    let size_idx = if cfg.notch_width <= 159.0 {
        0
    } else if cfg.notch_width >= 176.0 {
        2
    } else {
        1
    };
    window::TrayMenuState {
        start_with_windows: settings_ui::is_startup_enabled(),
        show_mascot: cfg.show_mascot,
        minimal_mode: cfg.minimal_mode,
        size_idx,
    }
}

#[derive(Clone, Debug)]
enum QueuedOsd {
    Bluetooth {
        name: String,
        pct: u8,
        connected: bool,
    },
    AudioDevice {
        name: String,
    },
    Network(osd_monitors::NetworkEvent),
    Charging {
        charging: bool,
        pct: u8,
    },
    Notification(osd_monitors::ToastNotification),
    MicInUse {
        app: String,
    },
    FocusAlert {
        next_is_focus: bool,
        is_long_break: bool,
        next_mins: u32,
    },
    WaterReminder,
    TodoReminder,
}

fn queued_osd_from_bt(ev: pages::BtPopupEvent) -> QueuedOsd {
    match ev {
        pages::BtPopupEvent::Connected { name, pct } => QueuedOsd::Bluetooth {
            name,
            pct,
            connected: true,
        },
        pages::BtPopupEvent::Disconnected { name } => QueuedOsd::Bluetooth {
            name,
            pct: 0,
            connected: false,
        },
    }
}

fn activate_queued_osd(
    item: QueuedOsd,
    now: Instant,
    widgets: &mut WidgetState,
    active_osd: &mut CollapsedOsdKind,
    osd_deadline: &mut Option<Instant>,
    springs: &mut NotchSprings,
) {
    match item {
        QueuedOsd::Bluetooth {
            name,
            pct,
            connected,
        } => {
            widgets.popup_bt_name = name;
            widgets.popup_bt_pct = pct;
            widgets.popup_bt_connected = connected;
            *active_osd = CollapsedOsdKind::Bluetooth;
            *osd_deadline = Some(now + Duration::from_millis(3400));
            springs.set_osd_wide(false);
            springs.set_osd_tall(true);
            springs.trigger_bt_popup_fill(if connected { pct as f32 / 100.0 } else { 0.0 });
            springs.set_osd_active(true);
        }
        QueuedOsd::AudioDevice { name } => {
            widgets.active_audio_device = name;
            *active_osd = CollapsedOsdKind::AudioDevice;
            *osd_deadline = Some(now + Duration::from_millis(2800));
            springs.set_osd_wide(false);
            springs.set_osd_tall(true);
            springs.set_osd_active(true);
        }
        QueuedOsd::Network(ev) => {
            widgets.active_network = Some(ev);
            *active_osd = CollapsedOsdKind::Network;
            *osd_deadline = Some(now + Duration::from_millis(3200));
            springs.set_osd_wide(false);
            springs.set_osd_tall(true);
            springs.set_osd_active(true);
        }
        QueuedOsd::Charging { charging, pct } => {
            *active_osd = CollapsedOsdKind::Charging { charging, pct };
            *osd_deadline = Some(now + Duration::from_millis(3400));
            springs.set_osd_wide(false);
            springs.set_osd_tall(true);
            springs.trigger_bt_popup_fill(pct as f32 / 100.0);
            springs.set_osd_active(true);
        }
        QueuedOsd::Notification(toast) => {
            widgets.active_toast = Some(toast);
            *active_osd = CollapsedOsdKind::Notification;
            *osd_deadline = Some(now + Duration::from_millis(4000));
            springs.set_osd_wide(false);
            springs.set_osd_tall(true);
            springs.set_osd_active(true);
        }
        QueuedOsd::MicInUse { app } => {
            widgets.active_mic_app = app;
            *active_osd = CollapsedOsdKind::MicInUse;
            *osd_deadline = Some(now + Duration::from_millis(2800));
            springs.set_osd_wide(false);
            springs.set_osd_tall(false);
            springs.set_osd_active(true);
        }
        QueuedOsd::FocusAlert {
            next_is_focus,
            is_long_break,
            next_mins,
        } => {
            widgets.osd_alert_progress = 1.0;
            *active_osd = CollapsedOsdKind::FocusAlert {
                next_is_focus,
                is_long_break,
                next_mins,
            };
            *osd_deadline = Some(now + Duration::from_secs(12));
            springs.set_osd_wide(true);
            springs.set_osd_active(true);
        }
        QueuedOsd::WaterReminder => {
            widgets.osd_alert_progress = 1.0;
            *active_osd = CollapsedOsdKind::WaterReminder;
            *osd_deadline = Some(now + Duration::from_secs(12));
            springs.set_osd_wide(true);
            springs.set_osd_active(true);
        }
        QueuedOsd::TodoReminder => {
            widgets.osd_alert_progress = 1.0;
            *active_osd = CollapsedOsdKind::TodoReminder;
            *osd_deadline = Some(now + Duration::from_secs(12));
            springs.set_osd_wide(true);
            springs.set_osd_active(true);
        }
    }
}

fn enqueue_or_show_osd(
    item: QueuedOsd,
    cfg: &Config,
    now: Instant,
    expanded: bool,
    widgets: &mut WidgetState,
    active_osd: &mut CollapsedOsdKind,
    osd_deadline: &mut Option<Instant>,
    osd_queue: &mut VecDeque<QueuedOsd>,
    springs: &mut NotchSprings,
) {
    let enabled = match &item {
        QueuedOsd::Bluetooth { .. } => cfg.osd_bluetooth,
        QueuedOsd::AudioDevice { .. } => cfg.osd_audio_device,
        QueuedOsd::Network(_) => cfg.osd_wifi,
        QueuedOsd::Charging { .. } => cfg.osd_charging,
        QueuedOsd::Notification(_) => cfg.osd_notifications,
        QueuedOsd::MicInUse { .. } => cfg.osd_mic_in_use,
        QueuedOsd::FocusAlert { .. } | QueuedOsd::WaterReminder | QueuedOsd::TodoReminder => true,
    };
    if !enabled || expanded {
        return;
    }
    if *active_osd == CollapsedOsdKind::None && osd_deadline.is_none() {
        activate_queued_osd(item, now, widgets, active_osd, osd_deadline, springs);
        return;
    }
    if let Some(back) = osd_queue.back_mut() {
        let same_kind = matches!(
            (&*back, &item),
            (QueuedOsd::AudioDevice { .. }, QueuedOsd::AudioDevice { .. })
                | (QueuedOsd::Network(_), QueuedOsd::Network(_))
                | (QueuedOsd::Charging { .. }, QueuedOsd::Charging { .. })
                | (QueuedOsd::MicInUse { .. }, QueuedOsd::MicInUse { .. })
        );
        if same_kind {
            *back = item;
            return;
        }
    }
    if osd_queue.len() < 6 {
        osd_queue.push_back(item);
    }
}

fn main() {
    if let Err(err) = run() {
        eprintln!("eave error: {err:?}");
        std::process::exit(1);
    }
}

fn run() -> windows::core::Result<()> {
    updater::cleanup_old_binary();
    let first_run = !Config::resolve_path().exists();
    let (mut cfg, cfg_path) = Config::load_or_create();
    if first_run {
        settings_ui::set_startup_enabled(true);
    }

    NotchWindow::enable_per_monitor_dpi();
    let _ole = shelf::init_ole()?;
    CompositorHost::init_sta()?;

    let (mut max_w_dips, mut max_h_dips) = compute_surface_bounds(&cfg);
    let mut window = NotchWindow::create(
        cfg.monitor_index,
        max_w_dips,
        max_h_dips,
        cfg.notch_width,
        cfg.notch_height,
        cfg.top_offset,
        cfg.horizontal_offset,
    )?;

    let shelf_store = Rc::new(RefCell::new(ShelfStore::load_or_create()));
    let _drop_reg = shelf::register_drop_target(window.hwnd(), Rc::clone(&shelf_store))?;

    let mut renderer = NotchRenderer::new(max_w_dips, max_h_dips, window.dpi())?;
    let init_scale = window.dpi_scale();
    let init_radius = 14.0_f32.min(cfg.notch_height * 0.48);
    let mut compositor = CompositorHost::new(
        window.hwnd(),
        renderer.swap_chain(),
        cfg.notch_width * init_scale,
        cfg.notch_height * init_scale,
        init_radius * init_scale,
        10.0 * init_scale,
        max_w_dips * init_scale,
        max_h_dips * init_scale,
    )?;
    compositor.set_backdrop_visible(cfg.glass_tint[3] < 255);

    let mut springs = NotchSprings::new(&cfg);
    let mut media = MediaService::start_with_wake(window.hwnd());
    let mut lyrics = LyricsService::start();

    let mut page = NotchPage::Music;
    let mut prev_page = page;
    let mut pomodoro = PomodoroState::new();
    let mut water = WaterState::new();
    let mut todo = TodoState::load_or_create();
    let mut widgets = WidgetState::new();
    {
        let saved_turns = ai::load_ai_turns();
        if !saved_turns.is_empty() {
            for (q, a) in &saved_turns {
                widgets.ai.history.push(("user".to_string(), q.clone()));
                widgets.ai.history.push(("model".to_string(), a.clone()));
            }
            if let Some((last_q, last_a)) = saved_turns.last() {
                widgets.ai.submitted_prompt = last_q.clone();
                widgets.ai.response = last_a.clone();
            }
            widgets.ai.turns = saved_turns;
        }
    }
    let mut last_power_poll = Instant::now();
    let mut bt_osd_suppress_vol_until: Option<Instant> = None;
    let mut last_audio_ep_seq: u64 = 0;
    let mut settings_win = SettingsWindow::new();

    let mut hover_sm = HoverStateMachine::new();
    let mut dock_hovered = false;
    let mut prev_shelf_len = shelf_store.borrow().items.len();
    let mut hover_target = HitTarget::None;
    let mut prev_ai_btn_hov: Option<usize> = None;
    let mut prev_ai_show_followup = false;
    let mut ai_follow_footer_lift = false;
    let mut last_pointer_notch: Option<(f32, f32)> = None;
    let mut lyrics_open = false;
    let (mut volume_pct, mut muted) = media::read_system_volume().unwrap_or((100.0, false));
    let mut dragging_volume = false;
    let mut dragging_progress = false;
    let mut progress_scrub_ratio: Option<f64> = None;
    let mut last_progress_seek: Option<Instant> = None;
    let mut dragging_todo_idx: Option<usize> = None;
    let mut todo_drag_moved = false;
    let mut shelf_edge_resist: i32 = 0;
    let mut volume_hovered = false;
    let mut last_volume_write: Option<Instant> = None;
    let mut caps_lock_on = is_caps_lock_on();
    let mut active_osd = CollapsedOsdKind::None;
    let mut osd_deadline: Option<Instant> = None;
    let mut osd_queue: VecDeque<QueuedOsd> = VecDeque::with_capacity(6);
    let mut has_notification = false;
    let mut notif_seen_since: Option<Instant> = None;
    let mut last_art_rev: u64 = 0;
    let mut last_lyric_track_key = String::new();
    let mut song_rgb: (f32, f32, f32) = (0.22, 0.47, 0.81);
    let mut events: Vec<WindowEvent> = Vec::with_capacity(16);

    let mut fullscreen_should_hide = window.is_fullscreen_active_now(cfg.hide_fullscreen);
    window.sync_tray_menu_state(tray_menu_state_from_cfg(&cfg));
    let mut hidden = if fullscreen_should_hide {
        springs.notch_reveal.snap_to(0.0);
        window.set_visible(false);
        true
    } else {
        springs.trigger_startup_reveal();
        false
    };
    let mut last_tick = Instant::now();
    let mut last_pomo_tick = Instant::now();
    let mut last_water_reminder = Instant::now();
    let mut last_clip_poll = Instant::now();
    let mut last_backdrop_poll = Instant::now() - Duration::from_secs(1);
    let mut last_dark_backdrop = false;
    let mut last_wheel_switch = Instant::now() - Duration::from_secs(1);
    let mut last_render_at = Instant::now() - Duration::from_secs(1);
    let mut eave_was_idle_active = false;
    let mut mascot_wake_until: Option<Instant> = None;
    let mut auto_ring_override: Option<(u8, Instant)> = None;

    let mut brightness_mon = osd_monitors::BrightnessMonitor::new();
    let mut power_charge_mon = osd_monitors::PowerChargeMonitor::new();
    let mut toast_mon = osd_monitors::ToastNotificationMonitor::new();
    let mut mic_usage_mon = osd_monitors::MicUsageMonitor::new();
    let mut network_mon = osd_monitors::NetworkMonitor::new();
    let mut ai_svc = ai::AiService::new(window.hwnd());
    let update_checker = updater::UpdateChecker::new();
    let app_started = Instant::now();
    let mut auto_update_checked = false;
    let mut last_seen_update_status = updater::UpdateStatus::Idle;
    let mut ai_card_visible = false;
    let mut ai_card_auto_hide: Option<Instant> = None;
    let mut ai_copied_until: Option<Instant> = None;
    let mut ai_active_req_id: u64 = 0;
    window.register_ai_hotkey(cfg.ai_enabled, cfg.ai_hotkey);
    window.register_ocr_hotkey();
    window.register_hide_fullscreen_hotkey();
    widgets.ai.has_api_key = !cfg.ai_api_key.trim().is_empty();
    widgets.ai.model_label = ai::model_label_for_id(&cfg.ai_model).to_string();

    let init_snap = media.snapshot();
    let mut last_ext_vol_seq: u64 = init_snap.external_volume_seq;
    let mut last_mic_mute_seq: u64 = init_snap.mic_mute_seq;
    let mut last_mic_in_use_seq: u64 = init_snap.mic_in_use_seq;
    let mut last_mic_osd_sent: Option<(String, Instant)> = None;
    update_song_color_cache(&init_snap, &mut last_art_rev, &mut song_rgb);
    springs.set_has_notification(has_notification);
    springs.set_volume(volume_pct, muted);
    springs
        .speaker_volume
        .snap_to(if muted { 0.0 } else { volume_pct });
    springs.snap_caps_on(caps_lock_on);
    let init_br = brightness_mon.current_pct() as f32;
    springs.brightness_pct.snap_to(init_br);
    springs.set_brightness(init_br);
    springs.snap_mic_muted(init_snap.mic_muted);
    springs.set_lyric_started(true);
    render_frame(
        &cfg,
        &springs,
        &media,
        &lyrics,
        &mut renderer,
        &mut compositor,
        &mut window,
        max_w_dips,
        max_h_dips,
        page,
        prev_page,
        &pomodoro,
        &water,
        &shelf_store.borrow(),
        &todo,
        &widgets,
        lyrics_open,
        muted,
        has_notification,
        song_rgb,
        hover_target,
        active_osd,
        hidden,
    );

    loop {
        events.clear();
        window.sync_hit_test_cursor();
        let snap_for_idle = media.snapshot();
        let is_drag_over_idle = shelf_store.borrow().is_drag_over;
        let has_copied_toast = shelf_store.borrow().active_copied_index().is_some();
        let ai_has_active_blur = widgets
            .ai
            .chunks
            .last()
            .is_some_and(|&(_, _, birth)| (springs.phase_secs() - birth).abs() < 0.50);
        let can_sleep = hidden
            || (!is_drag_over_idle
                && !has_copied_toast
                && osd_deadline.is_none()
                && !hover_sm.is_waiting_dwell()
                && !hover_sm.is_waiting_leave_grace()
                && !pomodoro.running
                && widgets.ai.phase != ai::AiPhase::Busy
                && widgets.ai.phase != ai::AiPhase::Input
                && springs.ai_wave_energy.value() <= 0.01
                && !ai_has_active_blur
                && ai_card_auto_hide.is_none()
                && ai_copied_until.is_none()
                && {
                    let s = shelf_store.borrow();
                    s.copied_feedback.is_none()
                        && (s.files_scroll_offset as f32 - s.files_scroll_anim).abs() <= 0.005
                        && (s.clip_scroll_offset as f32 - s.clip_scroll_anim).abs() <= 0.005
                }
                && springs.mascot_idle_presence.value() <= 0.01
                && springs.is_idle_settled(snap_for_idle.is_playing));

        if can_sleep {
            if !window.wait_and_pump(&mut events) {
                break;
            }
            last_tick = Instant::now();
        } else if !window.pump_events(&mut events) {
            break;
        }

        let mut should_quit = false;
        let mut need_resize = false;
        let drop_px_opt = shelf_store.borrow_mut().take_drop_screen_px();
        let mut force_render = shelf_store.borrow_mut().take_dirty();

        let shelf_len_now = shelf_store.borrow().items.len();
        if drop_px_opt.is_some() || (force_render && shelf_len_now > prev_shelf_len) {
            let drop_x = drop_px_opt
                .map(|(sx, _)| screen_to_canvas_dips(window.hwnd(), window.dpi_scale(), sx, 0).0)
                .unwrap_or(max_w_dips * 0.5);
            let added = shelf_len_now.saturating_sub(prev_shelf_len);
            let card_slot = if added > 0 {
                Some((shelf_len_now - 1).min(4))
            } else {
                None
            };
            shelf_store.borrow_mut().active_tab = ShelfTab::Files;
            springs.trigger_shelf_drop(drop_x, card_slot);
            if added > 1 && prev_shelf_len < 5 {
                springs.trigger_cards_spawn_range(prev_shelf_len, added.min(5 - prev_shelf_len));
            }
            force_render = true;
        }
        prev_shelf_len = shelf_len_now;

        let is_drag_over = shelf_store.borrow().is_drag_over;
        if is_drag_over {
            if let Some((sx, sy)) = shelf_store.borrow().drag_cursor_screen_px {
                let (cx, cy) = screen_to_canvas_dips(window.hwnd(), window.dpi_scale(), sx, sy);
                springs.cursor_x.set_target(cx);
                springs.cursor_y.set_target(cy);
            }
            if page != NotchPage::Shelf {
                prev_page = page;
                springs.trigger_page_switch(1.0);
                page = NotchPage::Shelf;
                if todo.adding {
                    todo.cancel_adding();
                    window.set_keyboard_focus(false);
                }
            }
            shelf_store.borrow_mut().active_tab = ShelfTab::Files;
            force_render = true;
        }

        let mut snap = media.snapshot();
        if !snap.has_session {
            let filtered = filter_media_hit(hover_target, false);
            if filtered != hover_target {
                hover_target = filtered;
                springs.set_active_hover_slot(renderer::hit_target_slot(hover_target));
                force_render = true;
            }
        }

        for ev in events.drain(..) {
            match ev {
                WindowEvent::MouseMove { x_dips, y_dips } => {
                    let prev_hover_target = hover_target;
                    let prev_dock_hovered = dock_hovered;
                    let prev_volume_hovered = volume_hovered;
                    hover_sm.notify_move(true);
                    last_pointer_notch = Some((x_dips, y_dips));
                    if hover_sm.expanded && page == NotchPage::Shelf {
                        let canvas_x = (max_w_dips - springs.width.value) * 0.5 + x_dips;
                        springs.cursor_x.set_target(canvas_x);
                        springs.cursor_y.set_target(y_dips);
                    }
                    if widgets.ai.sel_dragging && ai_card_visible {
                        let canvas_cx = (max_w_dips * 0.5).round();
                        let pill_left = canvas_cx - springs.width.value * 0.5;
                        let canvas_x = pill_left + x_dips;
                        let (_, body_str, is_live) = widgets.ai.displayed_turn();
                        let is_err = is_live && widgets.ai.error.is_some();
                        let show_followup = widgets.ai.phase == ai::AiPhase::Done && !is_err;
                        let card_h = springs.ai_card_h.value.clamp(90.0, 310.0);
                        let card_a = springs.ai_card_open.value().clamp(0.0, 1.0);
                        let slide_y = (1.0 - card_a) * -14.0;
                        let (mut card_r, _, _, _) =
                            renderer::ai_card_rects(canvas_cx, springs.height.value, card_h);
                        card_r.top += slide_y;
                        card_r.bottom += slide_y;
                        let (_, content_h) =
                            renderer.measure_ai_card_height(body_str, show_followup);
                        let visible_h =
                            (card_h - 38.0 - if show_followup { 42.0 } else { 16.0 }).max(24.0);
                        let max_scroll = (content_h - visible_h).max(0.0);
                        let scroll = springs.ai_scroll.value.clamp(0.0, max_scroll);
                        if let Some(pos) = renderer.hit_test_ai_text_pos(
                            body_str,
                            card_r,
                            scroll,
                            canvas_x,
                            y_dips,
                        ) {
                            if widgets.ai.sel_focus != Some(pos) {
                                widgets.ai.sel_focus = Some(pos);
                                force_render = true;
                            }
                        }
                        continue;
                    }

                    dock_hovered = hover_sm.expanded
                        && !(page == NotchPage::Music && lyrics_open)
                        && renderer::is_in_dock_zone(
                            x_dips,
                            y_dips,
                            springs.width.value,
                            cfg.expanded_height,
                            springs.height.value,
                            springs.dock_hover.value(),
                        );
                    let (shelf_tab, shelf_len, clip_len, todo_len) = {
                        let s = shelf_store.borrow();
                        (s.active_tab, s.items.len(), s.clipboard_items.len(), todo.visible_len())
                    };
                    hover_target = filter_media_hit(
                        renderer::hit_test_notch_ex(
                            x_dips,
                            y_dips,
                            springs.width.value,
                            cfg.expanded_height,
                            springs.height.value,
                            springs.ear_radius.value,
                            springs.expand.value(),
                            springs.dock_hover.value(),
                            has_notification,
                            page,
                            shelf_tab,
                            shelf_len,
                            clip_len,
                            todo_len,
                            todo.adding,
                            lyrics_open,
                            springs.volume_slider_open(),
                            springs.todo_list_expanded.value(),
                            cfg.page_order,
                            cfg.page_enabled,
                            cfg.widget_left,
                            cfg.widget_right,
                        ),
                        snap.has_session,
                    );
                    if dragging_progress && snap.has_session && snap.duration_secs > 0.0 {
                        let ratio = match hover_target {
                            HitTarget::ProgressBar(r) => r,
                            _ => progress_bar_ratio_from_x(
                                x_dips,
                                springs.width.value,
                                springs.ear_radius.value,
                                lyrics_open && page == NotchPage::Music,
                            ),
                        };
                        progress_scrub_ratio = Some(ratio);
                        if last_progress_seek
                            .map_or(true, |t| t.elapsed() >= Duration::from_millis(65))
                        {
                            media.send_command(MediaCommand::Seek(ratio * snap.duration_secs));
                            last_progress_seek = Some(Instant::now());
                        }
                    }
                    if dragging_volume {
                        let ratio = match hover_target {
                            HitTarget::VolumeSlider(r) => r,
                            _ => volume_slider_ratio_from_x(
                                x_dips,
                                springs.width.value,
                                springs.ear_radius.value,
                            ),
                        };
                        apply_volume_pct(
                            (ratio as f32) * 100.0,
                            &mut volume_pct,
                            &mut muted,
                            &mut last_volume_write,
                            &media,
                            &mut springs,
                        );
                        hover_target = HitTarget::VolumeSlider(ratio);
                    }
                    if let Some(cur_idx) = dragging_todo_idx {
                        if todo.editing && page == NotchPage::Todo && !todo.items.is_empty() {
                            let vis_slot = ((y_dips - 54.0) / 32.0).floor().clamp(0.0, 2.0) as usize;
                            let target_idx = (todo.scroll_offset + vis_slot)
                                .min(todo.items.len().saturating_sub(1));
                            if target_idx != cur_idx && todo.move_item(cur_idx, target_idx) {
                                dragging_todo_idx = Some(target_idx);
                                todo_drag_moved = true;
                            }
                        } else {
                            dragging_todo_idx = None;
                            todo_drag_moved = false;
                        }
                    }
                    volume_hovered = hover_sm.expanded
                        && page == NotchPage::Music
                        && !lyrics_open
                        && (dragging_volume
                            || matches!(
                                hover_target,
                                HitTarget::SpeakerButton | HitTarget::VolumeSlider(_)
                            ));
                    springs.set_volume_hovered(volume_hovered);
                    springs.set_dock_hovered_index(renderer::dock_hovered_index(hover_target));
                    springs.set_active_hover_slot(renderer::hit_target_slot(hover_target));
                    let mut ai_btn_hov: Option<usize> = None;
                    if widgets.ai.phase != ai::AiPhase::Idle || ai_card_visible {
                        let canvas_cx = (max_w_dips * 0.5).round();
                        let pill_left = canvas_cx - springs.width.value * 0.5;
                        let canvas_x = pill_left + x_dips;
                        if widgets.ai.phase == ai::AiPhase::Input
                            && y_dips <= springs.height.value + 4.0
                        {
                            let enter_r = renderer::ai_input_enter_rect(
                                pill_left,
                                springs.width.value,
                                springs.height.value,
                                springs.ear_radius.value,
                            );
                            if canvas_x >= enter_r.left - 3.0
                                && canvas_x <= enter_r.right + 3.0
                                && y_dips >= enter_r.top - 4.0
                                && y_dips <= enter_r.bottom + 4.0
                            {
                                ai_btn_hov = Some(4);
                            } else if !widgets.ai.turns.is_empty()
                                || !widgets.ai.response.is_empty()
                            {
                                let toggle_r = renderer::ai_input_toggle_rect(
                                    pill_left,
                                    springs.width.value,
                                    springs.height.value,
                                    springs.ear_radius.value,
                                );
                                if canvas_x >= toggle_r.left - 3.0
                                    && canvas_x <= toggle_r.right + 3.0
                                    && y_dips >= toggle_r.top - 4.0
                                    && y_dips <= toggle_r.bottom + 4.0
                                {
                                    ai_btn_hov = Some(3);
                                }
                            }
                        } else if ai_card_visible && y_dips >= springs.height.value + 2.0 {
                            let card_h = springs.ai_card_h.value.clamp(90.0, 310.0);
                            let card_a = springs.ai_card_open.value().clamp(0.0, 1.0);
                            let slide_y = (1.0 - card_a) * -14.0;
                            let (_, mut copy_r, mut close_r, _) =
                                renderer::ai_card_rects(canvas_cx, springs.height.value, card_h);
                            copy_r.top += slide_y;
                            copy_r.bottom += slide_y;
                            close_r.top += slide_y;
                            close_r.bottom += slide_y;
                            let clear_r = renderer::ai_clear_rect(copy_r);
                            if canvas_x >= clear_r.left - 3.0
                                && canvas_x <= clear_r.right + 3.0
                                && y_dips >= clear_r.top - 4.0
                                && y_dips <= clear_r.bottom + 4.0
                            {
                                ai_btn_hov = Some(0);
                            } else if canvas_x >= copy_r.left - 3.0
                                && canvas_x <= copy_r.right + 3.0
                                && y_dips >= copy_r.top - 4.0
                                && y_dips <= copy_r.bottom + 4.0
                            {
                                ai_btn_hov = Some(1);
                            } else if canvas_x >= close_r.left - 3.0
                                && canvas_x <= close_r.right + 3.0
                                && y_dips >= close_r.top - 4.0
                                && y_dips <= close_r.bottom + 4.0
                            {
                                ai_btn_hov = Some(2);
                            }
                        }
                    }
                    let ai_btn_changed = ai_btn_hov != prev_ai_btn_hov;
                    prev_ai_btn_hov = ai_btn_hov;
                    springs.set_ai_btn_hovered(ai_btn_hov);
                    if hover_target != prev_hover_target
                        || dock_hovered != prev_dock_hovered
                        || volume_hovered != prev_volume_hovered
                        || dragging_progress
                        || dragging_volume
                        || dragging_todo_idx.is_some()
                        || ai_btn_changed
                    {
                        force_render = true;
                    }
                }
                WindowEvent::MouseLeave => {
                    hover_sm.notify_leave();
                    last_pointer_notch = None;
                    dock_hovered = false;
                    if !dragging_volume {
                        volume_hovered = false;
                        springs.set_volume_hovered(false);
                    }
                    hover_target = HitTarget::None;
                    prev_ai_btn_hov = None;
                    springs.set_dock_hovered_index(None);
                    springs.set_active_hover_slot(None);
                    springs.set_active_press_slot(None);
                    springs.set_dock_pressed_index(None);
                    springs.set_ai_btn_hovered(None);
                    force_render = true;
                }
                WindowEvent::MouseWheel { delta } => {
                    let mut wheel_consumed = false;
                    if ai_card_visible && !hover_sm.expanded {
                        let (_, body_str, is_live) = widgets.ai.displayed_turn();
                        let is_err = is_live && widgets.ai.error.is_some();
                        let show_followup =
                            widgets.ai.phase == ai::AiPhase::Done && !is_err;
                        let (card_h, content_h) =
                            renderer.measure_ai_card_height(body_str, show_followup);
                        let visible_h =
                            (card_h - 38.0 - if show_followup { 42.0 } else { 16.0 }).max(24.0);
                        let max_scroll = (content_h - visible_h).max(0.0);
                        let step = -(delta as f32 / 120.0) * 56.0;
                        widgets.ai.scroll_y = (widgets.ai.scroll_y + step).clamp(0.0, max_scroll);
                        springs.ai_scroll.set_target(widgets.ai.scroll_y);
                        force_render = true;
                        wheel_consumed = true;
                    } else if !hover_sm.expanded && hover_sm.hovered && widgets.ai.phase == ai::AiPhase::Idle {
                        let step = (delta as f32 / 120.0) * 5.0;
                        if step > 0.0 && volume_pct >= 99.5 {
                            springs.kick_osd_vol_bounce(1.0);
                        } else if step < 0.0 && (volume_pct <= 0.5 || muted) {
                            springs.kick_osd_vol_bounce(-1.0);
                        }
                        apply_volume_pct(
                            volume_pct + step,
                            &mut volume_pct,
                            &mut muted,
                            &mut last_volume_write,
                            &media,
                            &mut springs,
                        );
                        if cfg.osd_volume {
                            active_osd = CollapsedOsdKind::Volume;
                            osd_deadline = Some(Instant::now() + Duration::from_secs(2));
                            springs.set_osd_tall(false);
                            springs.set_osd_active(true);
                        }
                        force_render = true;
                        wheel_consumed = true;
                    } else if hover_sm.expanded
                        && page == NotchPage::Music
                        && !lyrics_open
                        && matches!(
                            hover_target,
                            HitTarget::SpeakerButton | HitTarget::VolumeSlider(_)
                        )
                    {
                        let step = (delta as f32 / 120.0) * 5.0;
                        apply_volume_pct(
                            volume_pct + step,
                            &mut volume_pct,
                            &mut muted,
                            &mut last_volume_write,
                            &media,
                            &mut springs,
                        );
                        volume_hovered = true;
                        springs.set_volume_hovered(true);
                        force_render = true;
                        wheel_consumed = true;
                    } else if hover_sm.expanded
                        && page == NotchPage::Shelf
                        && (matches!(
                            hover_target,
                            HitTarget::ShelfChip(_)
                                | HitTarget::ShelfRemove(_)
                                | HitTarget::ClipboardCard(_)
                                | HitTarget::ClipboardRemove(_)
                        ) || last_pointer_notch.is_some_and(|(px, py)| {
                            renderer::is_in_shelf_zone(
                                px,
                                py,
                                springs.width.value,
                                cfg.expanded_height,
                                springs.ear_radius.value,
                            )
                        }))
                    {
                        let step = if delta > 0 { -1 } else { 1 };
                        let (scrolled, is_clip, can_scroll_list) = {
                            let mut s = shelf_store.borrow_mut();
                            match s.active_tab {
                                ShelfTab::Files if s.items.len() > 5 => {
                                    (s.scroll_files_by(step), false, true)
                                }
                                ShelfTab::Clipboard if s.clipboard_items.len() > 4 => {
                                    (s.scroll_clip_by(step), true, true)
                                }
                                _ => (false, s.active_tab == ShelfTab::Clipboard, false),
                            }
                        };
                        if scrolled {
                            shelf_edge_resist = 0;
                            springs.shift_shelf_hover_slots(is_clip, step);
                            springs.set_active_hover_slot(renderer::hit_target_slot(hover_target));
                            force_render = true;
                            wheel_consumed = true;
                        } else if can_scroll_list {
                            if shelf_edge_resist != step {
                                shelf_edge_resist = step;
                                wheel_consumed = true;
                            } else {
                                shelf_edge_resist = 0;
                            }
                        }
                    } else if hover_sm.expanded
                        && page == NotchPage::Todo
                        && !todo.adding
                        && todo.visible_len() > 3
                        && matches!(hover_target, HitTarget::TodoToggle(_) | HitTarget::TodoPanelEmpty)
                    {
                        let step = if delta > 0 { -1 } else { 1 };
                        if todo.scroll_by(step) {
                            force_render = true;
                            wheel_consumed = true;
                        }
                    }
                    if !wheel_consumed
                        && hover_sm.expanded
                        && !(page == NotchPage::Music && lyrics_open)
                        && last_wheel_switch.elapsed() >= Duration::from_millis(140)
                    {
                        let next_page = if delta < 0 {
                            prev_page = page;
                            springs.trigger_page_switch(1.0);
                            NotchPage::ALL[cfg.next_page_idx(page.index())]
                        } else if delta > 0 {
                            prev_page = page;
                            springs.trigger_page_switch(-1.0);
                            NotchPage::ALL[cfg.prev_page_idx(page.index())]
                        } else {
                            page
                        };
                        if next_page != page {
                            page = next_page;
                            if todo.adding {
                                todo.cancel_adding();
                                window.set_keyboard_focus(false);
                            }
                            last_wheel_switch = Instant::now();
                            force_render = true;
                        }
                    }
                }
                WindowEvent::KeyChar(ch) => {
                    if ai_card_visible && widgets.ai.phase != ai::AiPhase::Input {
                        if ch == '\u{0003}' {
                            let body_str = widgets.ai.displayed_turn().1;
                            let to_copy = if let Some((s, e)) = widgets.ai.normalized_selection() {
                                renderer.extract_ai_selected_text(body_str, s, e)
                            } else {
                                body_str.to_string()
                            };
                            if !to_copy.is_empty() && ai::write_clipboard_text(&to_copy) {
                                widgets.ai.copied_feedback = true;
                                ai_copied_until =
                                    Some(Instant::now() + Duration::from_millis(1600));
                                force_render = true;
                            }
                            continue;
                        }
                        if ch == '\u{0001}' {
                            let body_str = widgets.ai.displayed_turn().1;
                            if let Some(end_pos) = renderer.ai_select_all_end(body_str) {
                                widgets.ai.sel_anchor = Some((0, 0));
                                widgets.ai.sel_focus = Some(end_pos);
                                widgets.ai.sel_dragging = false;
                                force_render = true;
                            }
                            continue;
                        }
                    }
                    if widgets.ai.phase == ai::AiPhase::Done
                        && (!ch.is_control() || ch == '\u{0016}')
                    {
                        widgets.ai.clear_selection();
                        widgets.ai.phase = ai::AiPhase::Input;
                        widgets.ai.prompt.clear();
                        springs.set_ai_phase(ai::AiPhase::Input, ai_card_visible);
                        window.set_keyboard_focus(true);
                    }
                    if widgets.ai.phase == ai::AiPhase::Input {
                        if ch == '\u{0016}' {
                            if let Some(clip) = ai::read_clipboard_text() {
                                let clean: String = clip
                                    .chars()
                                    .map(|c| if c == '\r' || c == '\n' { ' ' } else { c })
                                    .collect();
                                for c in clean.chars() {
                                    if widgets.ai.prompt.chars().count() < 500 {
                                        widgets.ai.prompt.push(c);
                                    }
                                }
                                force_render = true;
                            }
                        } else if !ch.is_control() && widgets.ai.prompt.chars().count() < 500 {
                            widgets.ai.prompt.push(ch);
                            force_render = true;
                        }
                    } else if page == NotchPage::Todo && todo.adding && !ch.is_control() {
                        todo.push_draft_char(ch);
                        force_render = true;
                    }
                }
                WindowEvent::KeySpecial { vk } => {
                    if widgets.ai.phase == ai::AiPhase::Input {
                        match vk {
                            0x0D => {
                                let q = widgets.ai.prompt.trim().to_string();
                                if q.is_empty() {
                                    springs.kick_ai_shake();
                                } else {
                                    widgets.ai.submitted_prompt = q.clone();
                                    widgets.ai.prompt.clear();
                                    widgets.ai.response.clear();
                                    widgets.ai.chunks.clear();
                                    widgets.ai.viewing_turn = None;
                                    widgets.ai.error = None;
                                    widgets.ai.copied_feedback = false;
                                    widgets.ai.scroll_y = 0.0;
                                    springs.ai_scroll.snap_to(0.0);
                                    widgets.ai.model_label =
                                        ai::model_label_for_id(&cfg.ai_model).to_string();
                                    widgets.ai.phase = ai::AiPhase::Busy;
                                    ai_card_visible = true;
                                    ai_card_auto_hide = None;
                                    ai_active_req_id = ai_svc.start_request(
                                        cfg.ai_api_key.clone(),
                                        cfg.ai_model.clone(),
                                        q,
                                        widgets.ai.history.clone(),
                                    );
                                    springs.set_ai_phase(ai::AiPhase::Busy, true);
                                    window.set_keyboard_focus(false);
                                }
                                force_render = true;
                            }
                            0x1B => {
                                ai_svc.cancel();
                                widgets.ai.phase = ai::AiPhase::Idle;
                                widgets.ai.prompt.clear();
                                widgets.ai.viewing_turn = None;
                                widgets.ai.chunks.clear();
                                widgets.ai.clear_selection();
                                ai_card_visible = false;
                                ai_card_auto_hide = None;
                                springs.set_ai_phase(ai::AiPhase::Idle, false);
                                window.set_keyboard_focus(false);
                                force_render = true;
                            }
                            0x08 => {
                                widgets.ai.prompt.pop();
                                force_render = true;
                            }
                            _ => {}
                        }
                    } else if vk == 0x1B
                        && (widgets.ai.phase != ai::AiPhase::Idle || ai_card_visible)
                    {
                        if widgets.ai.normalized_selection().is_some() {
                            widgets.ai.clear_selection();
                            force_render = true;
                            continue;
                        }
                        ai_svc.cancel();
                        widgets.ai.phase = ai::AiPhase::Idle;
                        widgets.ai.viewing_turn = None;
                        widgets.ai.chunks.clear();
                        widgets.ai.clear_selection();
                        ai_card_visible = false;
                        ai_card_auto_hide = None;
                        springs.set_ai_phase(ai::AiPhase::Idle, false);
                        window.set_keyboard_focus(false);
                        force_render = true;
                    } else if page == NotchPage::Todo && todo.adding {
                        match vk {
                            0x0D => {
                                todo.confirm_adding();
                                auto_ring_override = Some((
                                    2,
                                    Instant::now() + Duration::from_secs(AUTO_RING_OVERRIDE_SECS),
                                ));
                                window.set_keyboard_focus(false);
                                force_render = true;
                            }
                            0x1B => {
                                todo.cancel_adding();
                                window.set_keyboard_focus(false);
                                force_render = true;
                            }
                            0x09 => {
                                todo.draft_focus_time = !todo.draft_focus_time;
                                force_render = true;
                            }
                            0x08 => {
                                todo.pop_draft_char();
                                force_render = true;
                            }
                            _ => {}
                        }
                    }
                }
                WindowEvent::AiHotkey => {
                    if cfg.ai_enabled {
                        if widgets.ai.phase == ai::AiPhase::Input {
                            if ai_card_visible && !widgets.ai.response.is_empty() {
                                widgets.ai.phase = ai::AiPhase::Done;
                                widgets.ai.prompt.clear();
                                springs.set_ai_phase(ai::AiPhase::Done, true);
                            } else {
                                widgets.ai.phase = ai::AiPhase::Idle;
                                widgets.ai.prompt.clear();
                                widgets.ai.viewing_turn = None;
                                widgets.ai.chunks.clear();
                                widgets.ai.clear_selection();
                                ai_card_visible = false;
                                springs.set_ai_phase(ai::AiPhase::Idle, false);
                            }
                            window.set_keyboard_focus(false);
                        } else if widgets.ai.phase == ai::AiPhase::Done {
                            widgets.ai.prompt.clear();
                            widgets.ai.clear_selection();
                            widgets.ai.phase = ai::AiPhase::Input;
                            springs.set_ai_phase(ai::AiPhase::Input, ai_card_visible);
                            window.set_keyboard_focus(true);
                        } else {
                            ai_svc.cancel();
                            hover_sm.expanded = false;
                            hover_sm.hovered = false;
                            active_osd = CollapsedOsdKind::None;
                            osd_deadline = None;
                            osd_queue.clear();
                            springs.set_osd_active(false);
                            widgets.ai.prompt.clear();
                            widgets.ai.viewing_turn = None;
                            widgets.ai.chunks.clear();
                            widgets.ai.clear_selection();
                            widgets.ai.error = None;
                            if let Some((last_q, last_a)) = widgets.ai.turns.last() {
                                widgets.ai.submitted_prompt = last_q.clone();
                                widgets.ai.response = last_a.clone();
                            }
                            widgets.ai.has_api_key = !cfg.ai_api_key.trim().is_empty();
                            widgets.ai.model_label =
                                ai::model_label_for_id(&cfg.ai_model).to_string();
                            widgets.ai.phase = ai::AiPhase::Input;
                            let show_saved = !widgets.ai.turns.is_empty();
                            ai_card_visible = show_saved;
                            ai_card_auto_hide = None;
                            springs.set_ai_phase(ai::AiPhase::Input, show_saved);
                            if hidden {
                                window.set_visible(true);
                                hidden = false;
                                springs.set_notch_reveal(true);
                            }
                            window.set_keyboard_focus(true);
                        }
                        force_render = true;
                    }
                }
                WindowEvent::OcrHotkey => {
                    ocr::start_screen_ocr_async();
                }
                WindowEvent::ToggleHideFullscreenHotkey => {
                    cfg.hide_fullscreen = !cfg.hide_fullscreen;
                    let _ = cfg.save_to(&cfg_path);
                    if settings_win.is_open() {
                        settings_win.sync_snapshot(build_settings_snapshot(
                            &cfg,
                            &pomodoro,
                            &water,
                            &todo,
                            &shelf_store,
                            update_checker.status(),
                        ));
                    }
                    force_render = true;
                }
                WindowEvent::FocusLost => {
                    if widgets.ai.phase == ai::AiPhase::Input {
                        if ai_card_visible && !widgets.ai.response.is_empty() {
                            widgets.ai.phase = ai::AiPhase::Done;
                            widgets.ai.prompt.clear();
                            springs.set_ai_phase(ai::AiPhase::Done, true);
                        } else {
                            widgets.ai.phase = ai::AiPhase::Idle;
                            widgets.ai.prompt.clear();
                            springs.set_ai_phase(ai::AiPhase::Idle, ai_card_visible);
                        }
                        window.set_keyboard_focus(false);
                        force_render = true;
                    }
                }
                WindowEvent::LeftDown { x_dips, y_dips } => {
                    if ai_card_visible && y_dips >= springs.height.value + 2.0 {
                        let canvas_cx = (max_w_dips * 0.5).round();
                        let pill_left = canvas_cx - springs.width.value * 0.5;
                        let canvas_x = pill_left + x_dips;
                        let (_, body_str, is_live) = widgets.ai.displayed_turn();
                        let is_err = is_live && widgets.ai.error.is_some();
                        let show_followup = widgets.ai.phase == ai::AiPhase::Done && !is_err;
                        let card_h = springs.ai_card_h.value.clamp(90.0, 310.0);
                        let card_a = springs.ai_card_open.value().clamp(0.0, 1.0);
                        let slide_y = (1.0 - card_a) * -14.0;
                        let (mut card_r, _, _, mut followup_r) =
                            renderer::ai_card_rects(canvas_cx, springs.height.value, card_h);
                        card_r.top += slide_y;
                        card_r.bottom += slide_y;
                        followup_r.top += slide_y;
                        followup_r.bottom += slide_y;
                        let body_top = card_r.top + 38.0;
                        let body_bottom = if show_followup {
                            followup_r.top - 6.0
                        } else {
                            card_r.bottom - 16.0
                        };
                        if canvas_x >= card_r.left + 12.0
                            && canvas_x <= card_r.right - 16.0
                            && y_dips >= body_top
                            && y_dips <= body_bottom
                        {
                            let (_, content_h) =
                                renderer.measure_ai_card_height(body_str, show_followup);
                            let visible_h = (body_bottom - body_top).max(24.0);
                            let max_scroll = (content_h - visible_h).max(0.0);
                            let scroll = springs.ai_scroll.value.clamp(0.0, max_scroll);
                            let pos = renderer.hit_test_ai_text_pos(
                                body_str,
                                card_r,
                                scroll,
                                canvas_x,
                                y_dips,
                            );
                            widgets.ai.sel_anchor = pos;
                            widgets.ai.sel_focus = pos;
                            widgets.ai.sel_dragging = pos.is_some();
                            if widgets.ai.phase != ai::AiPhase::Input {
                                window.set_keyboard_focus(true);
                            }
                            force_render = true;
                            continue;
                        }
                    }
                    let (shelf_tab, shelf_len, clip_len, todo_len) = {
                        let s = shelf_store.borrow();
                        (s.active_tab, s.items.len(), s.clipboard_items.len(), todo.visible_len())
                    };
                    let pressed_target = filter_media_hit(
                        renderer::hit_test_notch_ex(
                            x_dips,
                            y_dips,
                            springs.width.value,
                            cfg.expanded_height,
                            springs.height.value,
                            springs.ear_radius.value,
                            springs.expand.value(),
                            springs.dock_hover.value(),
                            has_notification,
                            page,
                            shelf_tab,
                            shelf_len,
                            clip_len,
                            todo_len,
                            todo.adding,
                            lyrics_open,
                            springs.volume_slider_open(),
                            springs.todo_list_expanded.value(),
                            cfg.page_order,
                            cfg.page_enabled,
                            cfg.widget_left,
                            cfg.widget_right,
                        ),
                        snap.has_session,
                    );
                    springs.set_active_press_slot(renderer::hit_target_slot(pressed_target));
                    springs.set_dock_pressed_index(renderer::dock_hovered_index(pressed_target));

                    if let HitTarget::VolumeSlider(ratio) = pressed_target {
                        dragging_volume = true;
                        volume_hovered = true;
                        springs.set_volume_hovered(true);
                        apply_volume_pct(
                            (ratio as f32) * 100.0,
                            &mut volume_pct,
                            &mut muted,
                            &mut last_volume_write,
                            &media,
                            &mut springs,
                        );
                    } else if let HitTarget::ProgressBar(ratio) = pressed_target {
                        if snap.has_session && snap.duration_secs > 0.0 {
                            dragging_progress = true;
                            progress_scrub_ratio = Some(ratio);
                            media.send_command(MediaCommand::Seek(ratio * snap.duration_secs));
                            last_progress_seek = Some(Instant::now());
                        }
                    } else if let HitTarget::TodoToggle(vis_idx) = pressed_target {
                        if todo.editing {
                            let actual_idx = todo.scroll_offset + vis_idx;
                            let card_right = springs.ear_radius.value + 364.0;
                            if actual_idx < todo.items.len() && x_dips < card_right - 55.5 {
                                dragging_todo_idx = Some(actual_idx);
                                todo_drag_moved = false;
                            }
                        }
                    } else if let HitTarget::ShelfChip(vis_idx) = pressed_target {
                        let actual_idx = shelf_store.borrow().files_scroll_offset + vis_idx;
                        let drag_paths = shelf_store.borrow().drag_paths_for_item(actual_idx);
                        if !drag_paths.is_empty() {
                            let mut pt = POINT::default();
                            let is_drag = unsafe {
                                GetCursorPos(&mut pt).is_ok()
                                    && DragDetect(window.hwnd(), pt).as_bool()
                            };
                            unsafe {
                                let _ = ReleaseCapture();
                            }
                            springs.set_active_press_slot(None);
                            springs.set_dock_pressed_index(None);
                            if is_drag {
                                let _ = shelf::start_drag_out(window.hwnd(), &drag_paths);
                            } else if shelf_store.borrow().select_mode {
                                shelf_store.borrow_mut().toggle_item_selected(actual_idx);
                            } else {
                                shelf_store.borrow().open_item(actual_idx);
                            }
                        }
                    }
                    force_render = true;
                }
                WindowEvent::LeftUp { x_dips, y_dips } => {
                    let was_ai_sel_dragging = widgets.ai.sel_dragging;
                    widgets.ai.sel_dragging = false;
                    let was_dragging_volume = dragging_volume;
                    dragging_volume = false;
                    let was_dragging_progress = dragging_progress;
                    dragging_progress = false;
                    let last_scrub = progress_scrub_ratio.take();
                    let was_todo_drag = dragging_todo_idx.take().is_some() && todo_drag_moved;
                    todo_drag_moved = false;
                    springs.set_active_press_slot(None);
                    springs.set_dock_pressed_index(None);
                    if x_dips <= -9999.0 || y_dips <= -9999.0 {
                        if was_dragging_progress {
                            if let Some(r) = last_scrub {
                                if snap.has_session && snap.duration_secs > 0.0 {
                                    media.send_command(MediaCommand::Seek(r * snap.duration_secs));
                                }
                            }
                        }
                        if was_dragging_volume {
                            volume_hovered = false;
                            springs.set_volume_hovered(false);
                        }
                        force_render = true;
                        continue;
                    }
                    let (shelf_tab, shelf_len, clip_len, todo_len) = {
                        let s = shelf_store.borrow();
                        (s.active_tab, s.items.len(), s.clipboard_items.len(), todo.visible_len())
                    };
                    let hit = filter_media_hit(
                        renderer::hit_test_notch_ex(
                            x_dips,
                            y_dips,
                            springs.width.value,
                            cfg.expanded_height,
                            springs.height.value,
                            springs.ear_radius.value,
                            springs.expand.value(),
                            springs.dock_hover.value(),
                            has_notification,
                            page,
                            shelf_tab,
                            shelf_len,
                            clip_len,
                            todo_len,
                            todo.adding,
                            lyrics_open,
                            springs.volume_slider_open(),
                            springs.todo_list_expanded.value(),
                            cfg.page_order,
                            cfg.page_enabled,
                            cfg.widget_left,
                            cfg.widget_right,
                        ),
                        snap.has_session,
                    );
                    if was_dragging_progress {
                        if snap.has_session && snap.duration_secs > 0.0 {
                            let ratio = match hit {
                                HitTarget::ProgressBar(r) => r,
                                _ => progress_bar_ratio_from_x(
                                    x_dips,
                                    springs.width.value,
                                    springs.ear_radius.value,
                                    lyrics_open && page == NotchPage::Music,
                                ),
                            };
                            media.send_command(MediaCommand::Seek(ratio * snap.duration_secs));
                        }
                        force_render = true;
                        continue;
                    }
                    if was_dragging_volume {
                        let ratio = match hit {
                            HitTarget::VolumeSlider(r) => r,
                            _ => volume_slider_ratio_from_x(
                                x_dips,
                                springs.width.value,
                                springs.ear_radius.value,
                            ),
                        };
                        apply_volume_pct(
                            (ratio as f32) * 100.0,
                            &mut volume_pct,
                            &mut muted,
                            &mut last_volume_write,
                            &media,
                            &mut springs,
                        );
                        volume_hovered = matches!(
                            hit,
                            HitTarget::SpeakerButton | HitTarget::VolumeSlider(_)
                        );
                        springs.set_volume_hovered(volume_hovered);
                        force_render = true;
                        continue;
                    }
                    if widgets.ai.phase == ai::AiPhase::Input && y_dips <= springs.height.value + 4.0
                    {
                        let canvas_cx = (max_w_dips * 0.5).round();
                        let pill_left = canvas_cx - springs.width.value * 0.5;
                        let canvas_x = pill_left + x_dips;
                        let enter_r = renderer::ai_input_enter_rect(
                            pill_left,
                            springs.width.value,
                            springs.height.value,
                            springs.ear_radius.value,
                        );
                        if canvas_x >= enter_r.left - 4.0
                            && canvas_x <= enter_r.right + 4.0
                            && y_dips >= enter_r.top - 6.0
                            && y_dips <= enter_r.bottom + 6.0
                        {
                            let q = widgets.ai.prompt.trim().to_string();
                            if q.is_empty() {
                                springs.kick_ai_shake();
                                window.set_keyboard_focus(true);
                            } else {
                                widgets.ai.submitted_prompt = q.clone();
                                widgets.ai.prompt.clear();
                                widgets.ai.response.clear();
                                widgets.ai.chunks.clear();
                                widgets.ai.viewing_turn = None;
                                widgets.ai.clear_selection();
                                widgets.ai.error = None;
                                widgets.ai.copied_feedback = false;
                                widgets.ai.scroll_y = 0.0;
                                springs.ai_scroll.snap_to(0.0);
                                springs.ai_content_h.snap_to(24.0);
                                widgets.ai.model_label =
                                    ai::model_label_for_id(&cfg.ai_model).to_string();
                                widgets.ai.phase = ai::AiPhase::Busy;
                                ai_card_visible = true;
                                ai_card_auto_hide = None;
                                ai_active_req_id = ai_svc.start_request(
                                    cfg.ai_api_key.clone(),
                                    cfg.ai_model.clone(),
                                    q,
                                    widgets.ai.history.clone(),
                                );
                                springs.set_ai_phase(ai::AiPhase::Busy, true);
                                springs.set_ai_btn_hovered(None);
                                prev_ai_btn_hov = None;
                                window.set_keyboard_focus(false);
                            }
                            force_render = true;
                            continue;
                        }
                        let has_saved_chat =
                            !widgets.ai.turns.is_empty() || !widgets.ai.response.is_empty();
                        if has_saved_chat {
                            let toggle_r = renderer::ai_input_toggle_rect(
                                pill_left,
                                springs.width.value,
                                springs.height.value,
                                springs.ear_radius.value,
                            );
                            if canvas_x >= toggle_r.left - 4.0
                                && canvas_x <= toggle_r.right + 4.0
                                && y_dips >= toggle_r.top - 6.0
                                && y_dips <= toggle_r.bottom + 6.0
                            {
                                ai_card_visible = !ai_card_visible;
                                if !ai_card_visible {
                                    widgets.ai.clear_selection();
                                }
                                springs.set_ai_phase(ai::AiPhase::Input, ai_card_visible);
                                window.set_keyboard_focus(true);
                                force_render = true;
                                continue;
                            }
                        }
                        window.set_keyboard_focus(true);
                        force_render = true;
                        continue;
                    }
                    if widgets.ai.phase == ai::AiPhase::Busy && y_dips <= springs.height.value + 2.0
                    {
                        force_render = true;
                        continue;
                    }
                    if widgets.ai.phase == ai::AiPhase::Done && y_dips <= springs.height.value + 2.0
                    {
                        widgets.ai.prompt.clear();
                        widgets.ai.phase = ai::AiPhase::Input;
                        springs.set_ai_phase(ai::AiPhase::Input, ai_card_visible);
                        window.set_keyboard_focus(true);
                        force_render = true;
                        continue;
                    }
                    if ai_card_visible && y_dips >= springs.height.value + 2.0 {
                        let canvas_cx = (max_w_dips * 0.5).round();
                        let pill_left = canvas_cx - springs.width.value * 0.5;
                        let canvas_x = pill_left + x_dips;
                        let is_live = widgets.ai.displayed_turn().2;
                        let is_err = is_live && widgets.ai.error.is_some();
                        let show_followup = widgets.ai.phase == ai::AiPhase::Done && !is_err;
                        let card_h = springs.ai_card_h.value.clamp(90.0, 310.0);
                        let card_a = springs.ai_card_open.value().clamp(0.0, 1.0);
                        let slide_y = (1.0 - card_a) * -14.0;
                        let (mut card_r, mut copy_r, mut close_r, mut followup_r) =
                            renderer::ai_card_rects(canvas_cx, springs.height.value, card_h);
                        card_r.top += slide_y;
                        card_r.bottom += slide_y;
                        copy_r.top += slide_y;
                        copy_r.bottom += slide_y;
                        close_r.top += slide_y;
                        close_r.bottom += slide_y;
                        followup_r.top += slide_y;
                        followup_r.bottom += slide_y;
                        let total_turns = widgets.ai.total_turns();
                        if total_turns >= 2 {
                            let dot_rects = renderer::ai_turn_dot_rects(card_r, total_turns);
                            for (i, dr) in dot_rects.iter().enumerate() {
                                if canvas_x >= dr.left - 3.0
                                    && canvas_x <= dr.right + 3.0
                                    && y_dips >= dr.top - 2.0
                                    && y_dips <= dr.bottom + 2.0
                                {
                                    widgets.ai.viewing_turn = if i + 1 == total_turns {
                                        None
                                    } else {
                                        Some(i)
                                    };
                                    widgets.ai.clear_selection();
                                    widgets.ai.scroll_y = 0.0;
                                    springs.ai_scroll.set_target(0.0);
                                    force_render = true;
                                    break;
                                }
                            }
                            if force_render && canvas_x < card_r.left {
                                continue;
                            }
                        }
                        if canvas_x >= close_r.left - 6.0
                            && canvas_x <= close_r.right + 6.0
                            && y_dips >= close_r.top - 6.0
                            && y_dips <= close_r.bottom + 6.0
                        {
                            ai_svc.cancel();
                            widgets.ai.phase = ai::AiPhase::Idle;
                            widgets.ai.viewing_turn = None;
                            widgets.ai.chunks.clear();
                            widgets.ai.clear_selection();
                            ai_card_visible = false;
                            ai_card_auto_hide = None;
                            springs.set_ai_phase(ai::AiPhase::Idle, false);
                            window.set_extra_hit_box(None);
                            window.set_keyboard_focus(false);
                            force_render = true;
                            continue;
                        }
                        let clear_r = renderer::ai_clear_rect(copy_r);
                        if canvas_x >= clear_r.left - 5.0
                            && canvas_x <= clear_r.right + 5.0
                            && y_dips >= clear_r.top - 6.0
                            && y_dips <= clear_r.bottom + 6.0
                        {
                            ai_svc.cancel();
                            widgets.ai.history.clear();
                            widgets.ai.turns.clear();
                            widgets.ai.submitted_prompt.clear();
                            widgets.ai.response.clear();
                            widgets.ai.error = None;
                            widgets.ai.viewing_turn = None;
                            widgets.ai.chunks.clear();
                            widgets.ai.clear_selection();
                            widgets.ai.scroll_y = 0.0;
                            springs.ai_scroll.snap_to(0.0);
                            ai::save_ai_turns(&[]);
                            ai_card_visible = false;
                            ai_card_auto_hide = None;
                            widgets.ai.prompt.clear();
                            widgets.ai.phase = ai::AiPhase::Input;
                            springs.set_ai_phase(ai::AiPhase::Input, false);
                            window.set_extra_hit_box(None);
                            window.set_keyboard_focus(true);
                            force_render = true;
                            continue;
                        }
                        if canvas_x >= copy_r.left - 5.0
                            && canvas_x <= copy_r.right + 5.0
                            && y_dips >= copy_r.top - 6.0
                            && y_dips <= copy_r.bottom + 6.0
                        {
                            let body_str = widgets.ai.displayed_turn().1;
                            let to_copy = if let Some((s, e)) = widgets.ai.normalized_selection() {
                                renderer.extract_ai_selected_text(body_str, s, e)
                            } else {
                                body_str.to_string()
                            };
                            if !to_copy.trim().is_empty() && ai::write_clipboard_text(&to_copy) {
                                widgets.ai.copied_feedback = true;
                                ai_copied_until =
                                    Some(Instant::now() + Duration::from_millis(1600));
                            }
                            force_render = true;
                            continue;
                        }
                        if show_followup
                            && canvas_x >= followup_r.left
                            && canvas_x <= followup_r.right
                            && y_dips >= followup_r.top - 4.0
                            && y_dips <= followup_r.bottom + 4.0
                        {
                            widgets.ai.prompt.clear();
                            widgets.ai.clear_selection();
                            widgets.ai.phase = ai::AiPhase::Input;
                            springs.set_ai_phase(ai::AiPhase::Input, true);
                            window.set_keyboard_focus(true);
                            force_render = true;
                            continue;
                        }
                        if canvas_x >= card_r.left
                            && canvas_x <= card_r.right
                            && y_dips >= card_r.top
                            && y_dips <= card_r.bottom
                        {
                            if was_ai_sel_dragging && widgets.ai.normalized_selection().is_some() {
                                force_render = true;
                                continue;
                            }
                            widgets.ai.clear_selection();
                            let body_str = widgets.ai.displayed_turn().1;
                            let (_, content_h) =
                                renderer.measure_ai_card_height(body_str, show_followup);
                            let visible_h =
                                (card_h - 38.0 - if show_followup { 42.0 } else { 16.0 }).max(24.0);
                            let max_scroll = (content_h - visible_h).max(0.0);
                            let scroll = springs.ai_scroll.value.clamp(0.0, max_scroll);
                            if let Some(url) = renderer.hit_test_ai_link(
                                body_str,
                                card_r,
                                scroll,
                                show_followup,
                                canvas_x,
                                y_dips,
                            ) {
                                open_url_in_default_browser(&url);
                            }
                            force_render = true;
                            continue;
                        }
                    }
                    match hit {
                        HitTarget::Body => {
                            let is_interactive_osd = !hover_sm.expanded
                                && matches!(
                                    active_osd,
                                    CollapsedOsdKind::FocusAlert { .. }
                                        | CollapsedOsdKind::WaterReminder
                                        | CollapsedOsdKind::TodoReminder
                                );
                            if is_interactive_osd {
                                let (dismiss_r, cta_r) = renderer::interactive_osd_button_rects(
                                    springs.width.value,
                                    springs.height.value,
                                    springs.ear_radius.value,
                                );
                                let in_dismiss = x_dips >= dismiss_r.left - 3.0
                                    && x_dips <= dismiss_r.right + 3.0
                                    && y_dips >= dismiss_r.top - 3.0
                                    && y_dips <= dismiss_r.bottom + 3.0;
                                let in_cta = x_dips >= cta_r.left - 2.0
                                    && x_dips <= cta_r.right + 2.0
                                    && y_dips >= cta_r.top - 3.0
                                    && y_dips <= cta_r.bottom + 3.0;
                                if in_dismiss {
                                    osd_deadline = None;
                                    springs.set_osd_active(false);
                                    force_render = true;
                                } else if in_cta {
                                    match active_osd {
                                        CollapsedOsdKind::FocusAlert { .. } => {
                                            pomodoro.running = true;
                                        }
                                        CollapsedOsdKind::WaterReminder => {
                                            water.add_drink();
                                            last_water_reminder = Instant::now();
                                            auto_ring_override = Some((
                                                1,
                                                Instant::now()
                                                    + Duration::from_secs(AUTO_RING_OVERRIDE_SECS),
                                            ));
                                        }
                                        CollapsedOsdKind::TodoReminder => {
                                            if let Some(idx) = todo.active_reminder_idx {
                                                if todo.items.get(idx).is_some_and(|it| !it.done) {
                                                    todo.toggle_done(idx);
                                                }
                                            }
                                            auto_ring_override = Some((
                                                2,
                                                Instant::now()
                                                    + Duration::from_secs(AUTO_RING_OVERRIDE_SECS),
                                            ));
                                        }
                                        _ => {}
                                    }
                                    osd_deadline = None;
                                    springs.set_osd_active(false);
                                    force_render = true;
                                } else {
                                    let target_p = match active_osd {
                                        CollapsedOsdKind::FocusAlert { .. } => NotchPage::Pomodoro,
                                        CollapsedOsdKind::WaterReminder => NotchPage::Water,
                                        CollapsedOsdKind::TodoReminder => NotchPage::Todo,
                                        _ => page,
                                    };
                                    prev_page = page;
                                    page = target_p;
                                    osd_deadline = None;
                                    active_osd = CollapsedOsdKind::None;
                                    springs.set_osd_active(false);
                                    hover_sm.toggle_click(cfg.open_on_hover);
                                    force_render = true;
                                }
                            } else {
                                hover_sm.toggle_click(cfg.open_on_hover);
                                if !hover_sm.expanded {
                                    dock_hovered = false;
                                    springs.set_dock_hovered_index(None);
                                    springs.set_active_hover_slot(None);
                                    if todo.adding {
                                        todo.cancel_adding();
                                        window.set_keyboard_focus(false);
                                    }
                                }
                            }
                        }
                        HitTarget::ChevronLeft => {
                            prev_page = page;
                            page = NotchPage::ALL[cfg.prev_page_idx(page.index())];
                            springs.trigger_page_switch(-1.0);
                            if todo.adding {
                                todo.cancel_adding();
                                window.set_keyboard_focus(false);
                            }
                        }
                        HitTarget::ChevronRight => {
                            prev_page = page;
                            page = NotchPage::ALL[cfg.next_page_idx(page.index())];
                            springs.trigger_page_switch(1.0);
                            if todo.adding {
                                todo.cancel_adding();
                                window.set_keyboard_focus(false);
                            }
                        }
                        HitTarget::BellButton => {
                            has_notification = false;
                            notif_seen_since = None;
                        }
                        HitTarget::DockZone => {}
                        HitTarget::DockPage(target_page) => {
                            if target_page != page {
                                let dir = if cfg.page_slot(target_page.index())
                                    > cfg.page_slot(page.index())
                                {
                                    1.0
                                } else {
                                    -1.0
                                };
                                prev_page = page;
                                springs.trigger_page_switch(dir);
                                page = target_page;
                                if todo.adding {
                                    todo.cancel_adding();
                                    window.set_keyboard_focus(false);
                                }
                            }
                        }
                        HitTarget::DockSettings => {
                            settings_win.open_or_focus(build_settings_snapshot(
                                &cfg,
                                &pomodoro,
                                &water,
                                &todo,
                                &shelf_store,
                                update_checker.status(),
                            ));
                        }
                        HitTarget::PrevButton => media.send_command(MediaCommand::Previous),
                        HitTarget::PlayPauseButton => media.send_command(MediaCommand::PlayPause),
                        HitTarget::NextButton => media.send_command(MediaCommand::Next),
                        HitTarget::ProgressBar(ratio) => {
                            if snap.duration_secs > 0.0 {
                                media.send_command(MediaCommand::Seek(ratio * snap.duration_secs));
                            }
                        }
                        HitTarget::LyricsButton => {
                            if lyrics_open {
                                lyrics_open = false;
                            } else if snap.has_session {
                                lyrics_open = true;
                                if page == NotchPage::Music {
                                    dock_hovered = false;
                                }
                            }
                        }
                        HitTarget::LyricSeek(line_idx) => {
                            if cfg.show_lyrics && !cfg.minimal_mode && snap.has_session {
                                let pair = lyrics.get_lines(
                                    &snap.track_key,
                                    &snap.title,
                                    &snap.artist,
                                    snap.position_secs,
                                );
                                let seek_secs = [
                                    pair.prev_time_secs,
                                    pair.current_time_secs,
                                    pair.next_time_secs,
                                ]
                                .get(line_idx)
                                .copied()
                                .flatten();
                                if let Some(ts) = seek_secs {
                                    media.send_command(MediaCommand::Seek(ts));
                                }
                            }
                        }
                        HitTarget::SpeakerButton => {
                            muted = !muted;
                            last_volume_write = Some(Instant::now());
                            media.send_command(MediaCommand::SetMuted(muted));
                        }
                        HitTarget::VolumeSlider(ratio) => {
                            apply_volume_pct(
                                (ratio as f32) * 100.0,
                                &mut volume_pct,
                                &mut muted,
                                &mut last_volume_write,
                                &media,
                                &mut springs,
                            );
                        }
                        HitTarget::PomodoroToggle => {
                            pomodoro.toggle();
                            auto_ring_override = None;
                            last_pomo_tick = Instant::now();
                        }
                        HitTarget::PomodoroReset => {
                            pomodoro.reset();
                            auto_ring_override = None;
                            last_pomo_tick = Instant::now();
                        }
                        HitTarget::PomodoroSwitch => {
                            pomodoro.switch_phase();
                            auto_ring_override = None;
                            last_pomo_tick = Instant::now();
                        }
                        HitTarget::PomodoroSegment(seg_idx) => {
                            pomodoro.select_segment(seg_idx);
                            auto_ring_override = None;
                            last_pomo_tick = Instant::now();
                        }
                        HitTarget::WaterAdd => {
                            water.add_drink();
                            last_water_reminder = Instant::now();
                            auto_ring_override = Some((
                                1,
                                Instant::now() + Duration::from_secs(AUTO_RING_OVERRIDE_SECS),
                            ));
                        }
                        HitTarget::WaterReset => {
                            water.reset_day();
                            auto_ring_override = Some((
                                1,
                                Instant::now() + Duration::from_secs(AUTO_RING_OVERRIDE_SECS),
                            ));
                        }
                        HitTarget::WaterToggleReminder => {
                            water.toggle_reminder();
                            if water.reminder_enabled {
                                last_water_reminder = Instant::now();
                            }
                        }
                        HitTarget::WaterSwitchView => {
                            water.toggle_view();
                        }
                        HitTarget::ShelfChip(_) => {}
                        HitTarget::ShelfRemove(vis_idx) => {
                            let mut s = shelf_store.borrow_mut();
                            let actual_idx = s.files_scroll_offset + vis_idx;
                            if s.select_mode {
                                s.toggle_item_selected(actual_idx);
                            } else {
                                s.start_remove_file_at(actual_idx);
                            }
                        }
                        HitTarget::ShelfSelectToggle => {
                            shelf_store.borrow_mut().toggle_select_mode();
                        }
                        HitTarget::ShelfClear => {
                            let count = {
                                let s = shelf_store.borrow();
                                match s.active_tab {
                                    ShelfTab::Files => s.items.len().min(5),
                                    ShelfTab::Clipboard => s.clipboard_items.len().min(4),
                                }
                            };
                            if !springs.is_shelf_clearing() && count > 0 {
                                springs.trigger_shelf_clear(count);
                            }
                        }
                        HitTarget::ShelfTabFiles => {
                            shelf_store.borrow_mut().active_tab = ShelfTab::Files;
                        }
                        HitTarget::ShelfTabClipboard => {
                            let mut s = shelf_store.borrow_mut();
                            s.active_tab = ShelfTab::Clipboard;
                            s.poll_clipboard();
                        }
                        HitTarget::ClipboardCard(vis_idx) => {
                            let actual_idx = shelf_store.borrow().clip_scroll_offset + vis_idx;
                            shelf_store.borrow_mut().copy_clip_at(actual_idx);
                        }
                        HitTarget::ClipboardRemove(vis_idx) => {
                            let actual_idx = shelf_store.borrow().clip_scroll_offset + vis_idx;
                            shelf_store.borrow_mut().start_remove_clip_at(actual_idx);
                        }
                        HitTarget::ShelfDropZone => {}
                        HitTarget::TodoToggle(vis_idx) => {
                            let vis = todo.visible_indices();
                            if let Some(&actual_idx) = vis.get(todo.scroll_offset + vis_idx) {
                                if todo.editing {
                                    let card_right = springs.ear_radius.value + 362.0;
                                    if !was_todo_drag && actual_idx < todo.items.len() {
                                        if x_dips >= card_right - 21.5 {
                                            todo.remove_at(actual_idx);
                                            auto_ring_override = Some((
                                                2,
                                                Instant::now()
                                                    + Duration::from_secs(AUTO_RING_OVERRIDE_SECS),
                                            ));
                                        } else if x_dips >= card_right - 38.5 {
                                            if !todo.schedule_filter && actual_idx + 1 < todo.items.len() {
                                                todo.move_item(actual_idx, actual_idx + 1);
                                            }
                                        } else if x_dips >= card_right - 55.5
                                            && !todo.schedule_filter
                                            && actual_idx > 0
                                        {
                                            todo.move_item(actual_idx, actual_idx - 1);
                                        }
                                    }
                                } else {
                                    todo.toggle_done(actual_idx);
                                    auto_ring_override = Some((
                                        2,
                                        Instant::now()
                                            + Duration::from_secs(AUTO_RING_OVERRIDE_SECS),
                                    ));
                                }
                            }
                        }
                        HitTarget::TodoPanelEmpty => {
                            todo.toggle_expanded_list();
                        }
                        HitTarget::TodoAdd => {
                            todo.start_adding();
                            window.set_keyboard_focus(true);
                        }
                        HitTarget::TodoEditToggle => {
                            todo.toggle_editing();
                            window.set_keyboard_focus(false);
                        }
                        HitTarget::TodoCalendar => {
                            todo.toggle_schedule_filter();
                            window.set_keyboard_focus(false);
                        }
                        HitTarget::TodoFormTitle => {
                            todo.draft_focus_time = false;
                            window.set_keyboard_focus(true);
                        }
                        HitTarget::TodoFormTime => {
                            todo.draft_focus_time = true;
                            window.set_keyboard_focus(true);
                        }
                        HitTarget::TodoFormDaily => {
                            todo.toggle_draft_daily();
                        }
                        HitTarget::TodoConfirmAdd => {
                            todo.confirm_adding();
                            auto_ring_override = Some((
                                2,
                                Instant::now() + Duration::from_secs(AUTO_RING_OVERRIDE_SECS),
                            ));
                            window.set_keyboard_focus(false);
                        }
                        HitTarget::TodoCancelAdd => {
                            todo.cancel_adding();
                            window.set_keyboard_focus(false);
                        }
                        HitTarget::TodoClearDone => {
                            todo.clear_done();
                            auto_ring_override = Some((
                                2,
                                Instant::now() + Duration::from_secs(AUTO_RING_OVERRIDE_SECS),
                            ));
                        }
                        HitTarget::WidgetWeekDay(d) => {
                            let next = d.min(6);
                            if widgets.selected_week_day != next {
                                widgets.selected_week_day = next;
                                springs.trigger_widget_day_pop();
                            }
                        }
                        HitTarget::WidgetCardLeft => {
                            springs.trigger_widget_card_press(true);
                            match cfg.widget_left {
                                1 => {
                                    widgets.cycle_battery_device();
                                    springs.trigger_widget_battery_switch();
                                }
                                2 => {
                                    let _ = water.add_drink();
                                    springs.trigger_widget_water_splash();
                                    auto_ring_override = Some((
                                        1,
                                        Instant::now()
                                            + Duration::from_secs(AUTO_RING_OVERRIDE_SECS),
                                    ));
                                }
                                3 => {
                                    pomodoro.toggle();
                                    auto_ring_override = None;
                                }
                                _ => {}
                            }
                        }
                        HitTarget::WidgetCardRight => {
                            springs.trigger_widget_card_press(false);
                            match cfg.widget_right {
                                1 => {
                                    widgets.cycle_battery_device();
                                    springs.trigger_widget_battery_switch();
                                }
                                2 => {
                                    let _ = water.add_drink();
                                    springs.trigger_widget_water_splash();
                                    auto_ring_override = Some((
                                        1,
                                        Instant::now()
                                            + Duration::from_secs(AUTO_RING_OVERRIDE_SECS),
                                    ));
                                }
                                3 => {
                                    pomodoro.toggle();
                                    auto_ring_override = None;
                                }
                                _ => {}
                            }
                        }
                        HitTarget::None => {}
                    }
                    force_render = true;
                }
                WindowEvent::TrayAction(action) => match action {
                    TrayAction::OpenSettings => {
                        settings_win.open_or_focus(build_settings_snapshot(
                            &cfg,
                            &pomodoro,
                            &water,
                            &todo,
                            &shelf_store,
                            update_checker.status(),
                        ));
                    }
                    TrayAction::ToggleStartup => {
                        let next = !settings_ui::is_startup_enabled();
                        let _ = settings_ui::set_startup_enabled(next);
                        window.sync_tray_menu_state(tray_menu_state_from_cfg(&cfg));
                        if settings_win.is_open() {
                            settings_win.sync_snapshot(build_settings_snapshot(
                                &cfg,
                                &pomodoro,
                                &water,
                                &todo,
                                &shelf_store,
                                update_checker.status(),
                            ));
                        }
                    }
                    TrayAction::ToggleMascot => {
                        cfg.show_mascot = !cfg.show_mascot;
                        let _ = cfg.save_to(&cfg_path);
                        window.sync_tray_menu_state(tray_menu_state_from_cfg(&cfg));
                        if settings_win.is_open() {
                            settings_win.sync_snapshot(build_settings_snapshot(
                                &cfg,
                                &pomodoro,
                                &water,
                                &todo,
                                &shelf_store,
                                update_checker.status(),
                            ));
                        }
                        force_render = true;
                    }
                    TrayAction::ToggleMinimal => {
                        cfg.minimal_mode = !cfg.minimal_mode;
                        let _ = cfg.save_to(&cfg_path);
                        window.sync_tray_menu_state(tray_menu_state_from_cfg(&cfg));
                        if settings_win.is_open() {
                            settings_win.sync_snapshot(build_settings_snapshot(
                                &cfg,
                                &pomodoro,
                                &water,
                                &todo,
                                &shelf_store,
                                update_checker.status(),
                            ));
                        }
                        force_render = true;
                    }
                    TrayAction::SetNotchSize(idx) => {
                        match idx {
                            0 => {
                                cfg.notch_width = 152.0;
                                cfg.expanded_width = 408.0;
                            }
                            2 => {
                                cfg.notch_width = 186.0;
                                cfg.expanded_width = 486.0;
                            }
                            _ => {
                                cfg.notch_width = 166.0;
                                cfg.expanded_width = 438.0;
                            }
                        }
                        let _ = cfg.save_to(&cfg_path);
                        let (nw, nh) = compute_surface_bounds(&cfg);
                        if (nw - max_w_dips).abs() > 0.5 || (nh - max_h_dips).abs() > 0.5 {
                            max_w_dips = nw;
                            max_h_dips = nh;
                            window.reposition_canvas(
                                max_w_dips,
                                max_h_dips,
                                cfg.top_offset,
                                cfg.horizontal_offset,
                            );
                            need_resize = true;
                        }
                        window.sync_tray_menu_state(tray_menu_state_from_cfg(&cfg));
                        if settings_win.is_open() {
                            settings_win.sync_snapshot(build_settings_snapshot(
                                &cfg,
                                &pomodoro,
                                &water,
                                &todo,
                                &shelf_store,
                                update_checker.status(),
                            ));
                        }
                        force_render = true;
                    }
                    TrayAction::ResetPosition => {
                        hover_sm.reset();
                        dock_hovered = false;
                        dragging_volume = false;
                        volume_hovered = false;
                        hover_target = HitTarget::None;
                        springs.snap_collapsed(&cfg);
                        springs.set_volume(volume_pct, muted);
                        springs
                            .speaker_volume
                            .snap_to(if muted { 0.0 } else { volume_pct });
                        window.refresh_monitor(cfg.monitor_index);
                        window.reposition_canvas(
                            max_w_dips,
                            max_h_dips,
                            cfg.top_offset,
                            cfg.horizontal_offset,
                        );
                        need_resize = true;
                        force_render = true;
                    }
                    TrayAction::Exit => {
                        should_quit = true;
                    }
                },
                WindowEvent::DpiChanged(_) | WindowEvent::DisplayChanged => {
                    window.refresh_monitor(cfg.monitor_index);
                    window.reposition_canvas(
                        max_w_dips,
                        max_h_dips,
                        cfg.top_offset,
                        cfg.horizontal_offset,
                    );
                    need_resize = true;
                    force_render = true;
                }
                WindowEvent::CapsLockCheck => {
                    let now_on = is_caps_lock_on();
                    if now_on != caps_lock_on {
                        caps_lock_on = now_on;
                        springs.set_caps_on(now_on);
                        if !hover_sm.expanded && cfg.osd_caps_lock {
                            active_osd = CollapsedOsdKind::CapsLock { on: now_on };
                            osd_deadline = Some(Instant::now() + Duration::from_millis(1800));
                            springs.set_osd_tall(false);
                            springs.set_osd_active(true);
                        }
                        force_render = true;
                    }
                }
                WindowEvent::VolumeKey { up } => {
                    if !hover_sm.expanded {
                        if cfg.osd_volume {
                            active_osd = CollapsedOsdKind::Volume;
                            osd_deadline = Some(Instant::now() + Duration::from_secs(2));
                            springs.set_osd_tall(false);
                            springs.set_osd_active(true);
                        }
                        if up && volume_pct >= 99.5 {
                            springs.kick_osd_vol_bounce(1.0);
                        } else if !up && (volume_pct <= 0.5 || muted) {
                            springs.kick_osd_vol_bounce(-1.0);
                        }
                        force_render = true;
                    }
                }
                WindowEvent::TimerTick => {
                    let fs_now = window.is_fullscreen_active_now(cfg.hide_fullscreen);
                    if fs_now != fullscreen_should_hide {
                        fullscreen_should_hide = fs_now;
                        if fs_now {
                            springs.set_notch_reveal(false);
                            hover_sm.expanded = false;
                            hover_sm.hovered = false;
                            dock_hovered = false;
                            force_render = true;
                        } else {
                            if hidden {
                                springs.notch_reveal.snap_to(0.0);
                                window.set_visible(true);
                                hidden = false;
                            }
                            springs.set_notch_reveal(true);
                            force_render = true;
                        }
                    }
                    if last_power_poll.elapsed() >= Duration::from_millis(1000) {
                        last_power_poll = Instant::now();
                        if let Some(ev) = widgets.poll_bluetooth_events() {
                            let now_bt = Instant::now();
                            bt_osd_suppress_vol_until = Some(now_bt + Duration::from_millis(3200));
                            enqueue_or_show_osd(
                                queued_osd_from_bt(ev),
                                &cfg,
                                now_bt,
                                hover_sm.expanded,
                                &mut widgets,
                                &mut active_osd,
                                &mut osd_deadline,
                                &mut osd_queue,
                                &mut springs,
                            );
                            force_render = true;
                        } else if page == NotchPage::Widgets && hover_sm.expanded {
                            force_render = true;
                        }
                    }
                    {
                        let mut s = shelf_store.borrow_mut();
                        s.poll_clipboard();
                        if s.clear_expired_copied_feedback() {
                            force_render = true;
                        }
                    }
                    if !hidden {
                        force_render = true;
                    }
                }
                WindowEvent::Quit => {
                    should_quit = true;
                }
            }
        }

        if should_quit {
            break;
        }

        let settings_actions = settings_win.drain_actions();
        if !settings_actions.is_empty() {
            for act in settings_actions {
                match act {
                    SettingsAction::UpdateConfig(next_cfg) => {
                        let mon_changed = next_cfg.monitor_index != cfg.monitor_index;
                        let pos_changed = (next_cfg.top_offset - cfg.top_offset).abs() > 0.01
                            || (next_cfg.horizontal_offset - cfg.horizontal_offset).abs() > 0.01;
                        let (nw, nh) = compute_surface_bounds(&next_cfg);
                        let bounds_changed =
                            (nw - max_w_dips).abs() > 0.5 || (nh - max_h_dips).abs() > 0.5;
                        cfg = next_cfg;
                        let _ = cfg.save_to(&cfg_path);
                        window.register_ai_hotkey(cfg.ai_enabled, cfg.ai_hotkey);
    window.register_ocr_hotkey();
    window.register_hide_fullscreen_hotkey();
                        widgets.ai.has_api_key = !cfg.ai_api_key.trim().is_empty();
                        widgets.ai.model_label = ai::model_label_for_id(&cfg.ai_model).to_string();
                        window.sync_tray_menu_state(tray_menu_state_from_cfg(&cfg));
                        if mon_changed {
                            window.refresh_monitor(cfg.monitor_index);
                        }
                        if mon_changed || pos_changed || bounds_changed {
                            max_w_dips = nw;
                            max_h_dips = nh;
                            window.reposition_canvas(
                                max_w_dips,
                                max_h_dips,
                                cfg.top_offset,
                                cfg.horizontal_offset,
                            );
                        }
                        if bounds_changed || mon_changed {
                            need_resize = true;
                        }
                        compositor.set_backdrop_visible(cfg.glass_tint[3] < 255);
                        if !cfg.show_lyrics {
                            lyrics_open = false;
                        }
                        if !cfg.page_enabled.get(page.index()).copied().unwrap_or(true) {
                            prev_page = page;
                            page = NotchPage::ALL[cfg.first_enabled_page_idx()];
                            springs.trigger_page_switch(1.0);
                        }
                        force_render = true;
                    }
                    SettingsAction::SetPomodoroDurations {
                        focus_mins,
                        break_mins,
                        long_break_mins,
                    } => {
                        pomodoro.focus_secs = (focus_mins.clamp(1, 180) * 60) as f32;
                        pomodoro.break_secs = (break_mins.clamp(1, 60) * 60) as f32;
                        pomodoro.long_break_secs = (long_break_mins.clamp(1, 90) * 60) as f32;
                        if !pomodoro.running {
                            pomodoro.remaining_secs = pomodoro.phase_duration_secs();
                        }
                        force_render = true;
                    }
                    SettingsAction::SetWaterSettings {
                        goal_ml,
                        bottle_ml,
                        reminder_enabled,
                    } => {
                        water.goal_ml = goal_ml.clamp(500, 10000);
                        water.bottle_ml = bottle_ml.clamp(50, 2000);
                        water.reminder_enabled = reminder_enabled;
                        force_render = true;
                    }
                    SettingsAction::ClearCompletedTodos => {
                        todo.items.retain(|it| !it.done);
                        todo.clamp_scroll();
                        let _ = todo.save();
                        force_render = true;
                    }
                    SettingsAction::ClearShelfFiles => {
                        shelf_store.borrow_mut().clear();
                        force_render = true;
                    }
                    SettingsAction::ClearShelfClipboard => {
                        let mut s = shelf_store.borrow_mut();
                        s.clipboard_items.clear();
                        s.clip_scroll_offset = 0;
                        s.clip_scroll_anim = 0.0;
                        force_render = true;
                    }
                    SettingsAction::OpenConfigFile => {
                        open_settings_file(&cfg, &cfg_path);
                    }
                    SettingsAction::ResetDefaults => {
                        cfg = Config::default();
                        let _ = cfg.save_to(&cfg_path);
                        window.refresh_monitor(cfg.monitor_index);
                        let (nw, nh) = compute_surface_bounds(&cfg);
                        max_w_dips = nw;
                        max_h_dips = nh;
                        window.reposition_canvas(
                            max_w_dips,
                            max_h_dips,
                            cfg.top_offset,
                            cfg.horizontal_offset,
                        );
                        compositor.set_backdrop_visible(cfg.glass_tint[3] < 255);
                        need_resize = true;
                        force_render = true;
                    }
                    SettingsAction::CheckForUpdates => {
                        update_checker.check_async(&cfg.update_repo);
                    }
                    SettingsAction::InstallUpdate {
                        version,
                        download_url,
                    } => {
                        update_checker.install_update_async(version, download_url);
                    }
                    SettingsAction::OpenUpdateUrl(url) => {
                        updater::open_external_url(&url);
                    }
                    SettingsAction::OpenGitHubRepo => {
                        let url = updater::github_repo_url(&cfg.update_repo);
                        updater::open_external_url(&url);
                    }
                    SettingsAction::TriggerSmartOcr => {
                        ocr::start_screen_ocr_async();
                    }
                }
            }
            if !auto_update_checked
                && cfg.auto_check_updates
                && app_started.elapsed() >= Duration::from_secs(4)
            {
                auto_update_checked = true;
                update_checker.check_async(&cfg.update_repo);
            }
            let cur_upd = update_checker.status();
            let upd_changed = cur_upd != last_seen_update_status;
            if upd_changed {
                last_seen_update_status = cur_upd;
                if settings_win.is_open() {
                    settings_win.sync_snapshot(build_settings_snapshot(
                        &cfg,
                        &pomodoro,
                        &water,
                        &todo,
                        &shelf_store,
                        update_checker.status(),
                    ));
                }
            }
            if settings_win.is_open() {
                settings_win.sync_snapshot(build_settings_snapshot(
                    &cfg,
                    &pomodoro,
                    &water,
                    &todo,
                    &shelf_store,
                    update_checker.status(),
                ));
            }
        }

        let now = Instant::now();
        if now.duration_since(last_clip_poll) >= Duration::from_millis(300) {
            last_clip_poll = now;
            let mut s = shelf_store.borrow_mut();
            s.poll_clipboard();
            if s.clear_expired_copied_feedback() || s.take_dirty() {
                force_render = true;
            }
        }
        if !hidden && now.duration_since(last_backdrop_poll) >= Duration::from_millis(350) {
            last_backdrop_poll = now;
            let live_w = springs.width.value.max(80.0);
            let live_h = springs.height.value.max(24.0);
            let live_left = (max_w_dips - live_w) * 0.5;
            let is_dark = window::detect_dark_backdrop_behind_notch(
                window.hwnd(),
                live_left,
                live_w,
                live_h,
                window.dpi(),
            );
            springs.set_dark_bg_border(is_dark);
            if is_dark != last_dark_backdrop {
                last_dark_backdrop = is_dark;
                force_render = true;
            }
        }
        let pomo_dt = now.saturating_duration_since(last_pomo_tick).as_secs_f32();
        last_pomo_tick = now;
        if pomodoro.tick(pomo_dt) {
            has_notification = true;
            osd_monitors::play_eave_chime();
            let next_is_focus = pomodoro.phase == pages::PomodoroPhase::Focus;
            let is_long_break = !next_is_focus
                && pomodoro.completed_sessions > 0
                && pomodoro.completed_sessions % 4 == 0;
            let next_mins = (pomodoro.phase_duration_secs() / 60.0).round().max(1.0) as u32;
            enqueue_or_show_osd(
                QueuedOsd::FocusAlert {
                    next_is_focus,
                    is_long_break,
                    next_mins,
                },
                &cfg,
                now,
                hover_sm.expanded,
                &mut widgets,
                &mut active_osd,
                &mut osd_deadline,
                &mut osd_queue,
                &mut springs,
            );
            force_render = true;
        }
        if water.reminder_enabled
            && water.taken_ml < water.goal_ml
            && last_water_reminder.elapsed() >= Duration::from_secs(45 * 60)
        {
            last_water_reminder = now;
            has_notification = true;
            osd_monitors::play_eave_chime();
            enqueue_or_show_osd(
                QueuedOsd::WaterReminder,
                &cfg,
                now,
                hover_sm.expanded,
                &mut widgets,
                &mut active_osd,
                &mut osd_deadline,
                &mut osd_queue,
                &mut springs,
            );
            force_render = true;
        }
        let repeat_mins = [5u16, 10, 15][cfg.todo_snooze_mins.min(2) as usize];
        if todo
            .check_due_reminder_now(cfg.todo_remind_start, cfg.todo_remind_end, repeat_mins)
            .is_some()
        {
            has_notification = true;
            osd_monitors::play_eave_chime();
            enqueue_or_show_osd(
                QueuedOsd::TodoReminder,
                &cfg,
                now,
                hover_sm.expanded,
                &mut widgets,
                &mut active_osd,
                &mut osd_deadline,
                &mut osd_queue,
                &mut springs,
            );
            force_render = true;
        }

        if need_resize {
            let _ = renderer.resize(max_w_dips, max_h_dips, window.dpi());
        }

        let dt = now.saturating_duration_since(last_tick).as_secs_f32();
        last_tick = now;

        if widgets.ai.phase == ai::AiPhase::Busy {
            let ai_snap = ai_svc.snapshot();
            if ai_snap.request_id == ai_active_req_id {
                if let Some(lbl) = ai_snap.model_label {
                    if widgets.ai.model_label != lbl {
                        widgets.ai.model_label = lbl;
                        force_render = true;
                    }
                }
                if widgets.ai.response != ai_snap.response || widgets.ai.error != ai_snap.error {
                    let prev_len = widgets.ai.response.len();
                    let next_len = ai_snap.response.len();
                    if next_len > prev_len {
                        widgets
                            .ai
                            .chunks
                            .push((prev_len, next_len, springs.phase_secs()));
                    }
                    widgets.ai.response = ai_snap.response;
                    widgets.ai.error = ai_snap.error;
                    force_render = true;
                }
                if ai_snap.finished {
                    if widgets.ai.error.is_none() && !widgets.ai.response.trim().is_empty() {
                        widgets
                            .ai
                            .history
                            .push(("user".to_string(), widgets.ai.submitted_prompt.clone()));
                        widgets
                            .ai
                            .history
                            .push(("model".to_string(), widgets.ai.response.clone()));
                        if widgets.ai.history.len() > ai::MAX_AI_TURNS * 2 {
                            let drain = widgets.ai.history.len() - ai::MAX_AI_TURNS * 2;
                            widgets.ai.history.drain(..drain);
                        }
                        widgets
                            .ai
                            .turns
                            .push((widgets.ai.submitted_prompt.clone(), widgets.ai.response.clone()));
                        if widgets.ai.turns.len() > ai::MAX_AI_TURNS {
                            let drain = widgets.ai.turns.len() - ai::MAX_AI_TURNS;
                            widgets.ai.turns.drain(..drain);
                        }
                        ai::save_ai_turns(&widgets.ai.turns);
                    }
                    widgets.ai.phase = ai::AiPhase::Done;
                    ai_card_auto_hide = None;
                    springs.set_ai_phase(ai::AiPhase::Done, ai_card_visible);
                    window.set_keyboard_focus(true);
                    force_render = true;
                }
            }
        }
        if let Some(until) = ai_copied_until {
            if now >= until {
                ai_copied_until = None;
                widgets.ai.copied_feedback = false;
                force_render = true;
            }
        }
        if let Some(hide_at) = ai_card_auto_hide {
            if now >= hide_at {
                ai_card_auto_hide = None;
                ai_card_visible = false;
                if widgets.ai.phase == ai::AiPhase::Done {
                    widgets.ai.phase = ai::AiPhase::Idle;
                }
                springs.set_ai_phase(widgets.ai.phase, false);
                force_render = true;
            }
        }
        if ai_card_visible {
            let (_, body_str, is_live) = widgets.ai.displayed_turn();
            let is_err = is_live && widgets.ai.error.is_some();
            let show_followup = widgets.ai.phase == ai::AiPhase::Done && !is_err;
            let (target_card_h, content_h) =
                renderer.measure_ai_card_height(body_str, show_followup);
            if springs.ai_card_open.value() < 0.05 {
                springs.ai_card_h.snap_to(target_card_h);
                springs.ai_content_h.snap_to(content_h);
                springs
                    .ai_followup_open
                    .snap_to(if show_followup { 1.0 } else { 0.0 });
            } else {
                springs.ai_card_h.set_target(target_card_h);
                springs.ai_content_h.set_target(content_h);
                springs
                    .ai_followup_open
                    .set_target(if show_followup { 1.0 } else { 0.0 });
            }
            let settled_vis_h =
                (target_card_h - 38.0 - if show_followup { 42.0 } else { 16.0 }).max(24.0);
            let settled_max_scroll = (content_h - settled_vis_h).max(0.0);
            springs
                .ai_scrollbar_vis
                .set_target(if settled_max_scroll > 2.0 { 1.0 } else { 0.0 });

            let fu = springs.ai_followup_open.value().clamp(0.0, 1.0);
            let live_vis_h = (target_card_h - 38.0 - (16.0 + 26.0 * fu)).max(24.0);
            let max_scroll = (content_h - live_vis_h).max(0.0);
            if show_followup && !prev_ai_show_followup {
                let prev_vis_h = (target_card_h - 38.0 - 16.0).max(24.0);
                let prev_max = (content_h - prev_vis_h).max(0.0);
                if widgets.ai.scroll_y >= prev_max - 28.0 {
                    ai_follow_footer_lift = true;
                }
            }
            if !show_followup || fu >= 0.995 {
                if fu >= 0.995 && ai_follow_footer_lift {
                    widgets.ai.scroll_y = settled_max_scroll;
                    springs.ai_scroll.set_target(settled_max_scroll);
                }
                ai_follow_footer_lift = false;
            }
            prev_ai_show_followup = show_followup;
            if is_live && widgets.ai.phase == ai::AiPhase::Busy {
                widgets.ai.scroll_y = max_scroll;
                springs.ai_scroll.set_target(max_scroll);
            } else if ai_follow_footer_lift {
                widgets.ai.scroll_y = max_scroll;
                springs.ai_scroll.snap_to(max_scroll);
            } else {
                widgets.ai.scroll_y = widgets.ai.scroll_y.clamp(0.0, settled_max_scroll);
                springs.ai_scroll.set_target(widgets.ai.scroll_y);
            }
            if springs.ai_card_open.value() > 0.05 {
                let half_w = if widgets.ai.total_turns() >= 2 { 316.0 } else { 286.0 };
                let live_h = springs.ai_card_h.value.max(target_card_h);
                window.set_extra_hit_box(Some((
                    half_w,
                    springs.height.value - 2.0,
                    springs.height.value + 24.0 + live_h,
                )));
            } else {
                window.set_extra_hit_box(None);
            }
        } else {
            prev_ai_show_followup = false;
            ai_follow_footer_lift = false;
            springs.ai_followup_open.set_target(0.0);
            springs.ai_scrollbar_vis.set_target(0.0);
            window.set_extra_hit_box(None);
        }

        let is_drag_over_now = shelf_store.borrow().is_drag_over;
        let is_clipboard_tab = shelf_store.borrow().active_tab == ShelfTab::Clipboard;
        let prev_expanded = hover_sm.expanded;
        let prev_hovered = hover_sm.hovered;
        let osd_is_interactive = matches!(
            active_osd,
            CollapsedOsdKind::FocusAlert { .. }
                | CollapsedOsdKind::WaterReminder
                | CollapsedOsdKind::TodoReminder
        ) && osd_deadline.is_some();
        let ai_blocks_hover =
            matches!(widgets.ai.phase, ai::AiPhase::Input | ai::AiPhase::Busy) || ai_card_visible;
        hover_sm.tick(
            dt,
            cfg.open_on_hover && !osd_is_interactive && !ai_blocks_hover,
            is_drag_over_now,
        );
        window.set_expanded_keep_open(hover_sm.expanded);
        if hover_sm.expanded != prev_expanded || hover_sm.hovered != prev_hovered {
            force_render = true;
            if !hover_sm.expanded && todo.adding {
                todo.cancel_adding();
                window.set_keyboard_focus(false);
            }
        }
        if hover_sm.expanded
            && !dragging_progress
            && !dragging_volume
            && dragging_todo_idx.is_none()
        {
            if let Some((px, py)) = last_pointer_notch {
                let (shelf_tab, shelf_len, clip_len, todo_len) = {
                    let s = shelf_store.borrow();
                    (
                        s.active_tab,
                        s.items.len(),
                        s.clipboard_items.len(),
                        todo.visible_len(),
                    )
                };
                dock_hovered = !(page == NotchPage::Music && lyrics_open)
                    && renderer::is_in_dock_zone(
                        px,
                        py,
                        springs.width.value,
                        cfg.expanded_height,
                        springs.height.value,
                        springs.dock_hover.value(),
                    );
                let new_target = filter_media_hit(
                    renderer::hit_test_notch_ex(
                        px,
                        py,
                        springs.width.value,
                        cfg.expanded_height,
                        springs.height.value,
                        springs.ear_radius.value,
                        springs.expand.value(),
                        springs.dock_hover.value(),
                        has_notification,
                        page,
                        shelf_tab,
                        shelf_len,
                        clip_len,
                        todo_len,
                        todo.adding,
                        lyrics_open,
                        springs.volume_slider_open(),
                        springs.todo_list_expanded.value(),
                        cfg.page_order,
                        cfg.page_enabled,
                        cfg.widget_left,
                        cfg.widget_right,
                    ),
                    snap.has_session,
                );
                if new_target != hover_target {
                    hover_target = new_target;
                    springs.set_dock_hovered_index(renderer::dock_hovered_index(hover_target));
                    springs.set_active_hover_slot(renderer::hit_target_slot(hover_target));
                    volume_hovered = page == NotchPage::Music
                        && !lyrics_open
                        && matches!(
                            hover_target,
                            HitTarget::SpeakerButton | HitTarget::VolumeSlider(_)
                        );
                    force_render = true;
                }
            }
        }
        let shelf_zone_hovered = page == NotchPage::Shelf
            && !is_clipboard_tab
            && (hover_target == HitTarget::ShelfDropZone || is_drag_over_now);
        let (shelf_show_dropzone, clip_empty) = {
            let s = shelf_store.borrow();
            let clearing = springs.is_shelf_clearing();
            (
                (s.items.is_empty() && !clearing) || is_drag_over_now,
                s.clipboard_items.is_empty() && !clearing,
            )
        };
        update_song_color_cache(&snap, &mut last_art_rev, &mut song_rgb);
        let frame_lyric_pair = if cfg.show_lyrics && !cfg.minimal_mode && snap.has_session {
            lyrics.get_lines(
                &snap.track_key,
                &snap.title,
                &snap.artist,
                snap.position_secs,
            )
        } else {
            LyricLinePair::default()
        };
        if lyrics_open && !snap.has_session {
            lyrics_open = false;
            force_render = true;
        }
        let lyrics_active = page == NotchPage::Music && lyrics_open;
        if lyrics_active {
            dock_hovered = false;
        }
        if !hover_sm.expanded || page != NotchPage::Music || lyrics_active {
            dragging_volume = false;
            volume_hovered = false;
        }
        if !hover_sm.expanded || page != NotchPage::Music || !snap.has_session {
            dragging_progress = false;
            progress_scrub_ratio = None;
        }
        if let Some(r) = progress_scrub_ratio {
            if snap.duration_secs > 0.0 {
                snap.position_secs = (r * snap.duration_secs).clamp(0.0, snap.duration_secs);
            }
        }
        if hover_sm.expanded
            && (osd_deadline.is_some()
                || active_osd != CollapsedOsdKind::None
                || !osd_queue.is_empty())
        {
            active_osd = CollapsedOsdKind::None;
            osd_deadline = None;
            osd_queue.clear();
            springs.set_osd_active(false);
            force_render = true;
        }
        if snap.audio_endpoint_seq != last_audio_ep_seq {
            last_audio_ep_seq = snap.audio_endpoint_seq;
            volume_pct = snap.volume_pct;
            muted = snap.muted;
            bt_osd_suppress_vol_until = Some(now + Duration::from_millis(3200));
            if let Some(ev) = widgets.poll_bluetooth_events() {
                last_power_poll = now;
                enqueue_or_show_osd(
                    queued_osd_from_bt(ev),
                    &cfg,
                    now,
                    hover_sm.expanded,
                    &mut widgets,
                    &mut active_osd,
                    &mut osd_deadline,
                    &mut osd_queue,
                    &mut springs,
                );
                force_render = true;
            } else if !snap.audio_device_name.is_empty() {
                enqueue_or_show_osd(
                    QueuedOsd::AudioDevice {
                        name: snap.audio_device_name.clone(),
                    },
                    &cfg,
                    now,
                    hover_sm.expanded,
                    &mut widgets,
                    &mut active_osd,
                    &mut osd_deadline,
                    &mut osd_queue,
                    &mut springs,
                );
                force_render = true;
            }
        }
        let mut detected_mic_app: Option<String> = None;
        if snap.mic_in_use_seq != last_mic_in_use_seq {
            last_mic_in_use_seq = snap.mic_in_use_seq;
            if !snap.mic_in_use_app.is_empty() {
                detected_mic_app = Some(snap.mic_in_use_app.clone());
            }
        }
        if let Some(Some(reg_app)) = mic_usage_mon.poll_changed() {
            if detected_mic_app.is_none() {
                detected_mic_app = Some(reg_app);
            }
        }
        if let Some(app_name) = detected_mic_app {
            let duplicate = last_mic_osd_sent.as_ref().is_some_and(|(prev, ts)| {
                prev.eq_ignore_ascii_case(&app_name) && ts.elapsed() < Duration::from_millis(1200)
            });
            if !duplicate {
                last_mic_osd_sent = Some((app_name.clone(), now));
                bt_osd_suppress_vol_until = Some(now + Duration::from_millis(2000));
                if active_osd == CollapsedOsdKind::Volume && cfg.osd_mic_in_use && !hover_sm.expanded {
                    activate_queued_osd(
                        QueuedOsd::MicInUse { app: app_name },
                        now,
                        &mut widgets,
                        &mut active_osd,
                        &mut osd_deadline,
                        &mut springs,
                    );
                } else {
                    enqueue_or_show_osd(
                        QueuedOsd::MicInUse { app: app_name },
                        &cfg,
                        now,
                        hover_sm.expanded,
                        &mut widgets,
                        &mut active_osd,
                        &mut osd_deadline,
                        &mut osd_queue,
                        &mut springs,
                    );
                }
                force_render = true;
            }
        }
        if snap.mic_mute_seq != last_mic_mute_seq {
            last_mic_mute_seq = snap.mic_mute_seq;
            springs.set_mic_muted(snap.mic_muted);
            if !hover_sm.expanded && cfg.osd_mic_mute {
                active_osd = CollapsedOsdKind::MicMute {
                    muted: snap.mic_muted,
                };
                osd_deadline = Some(now + Duration::from_millis(1900));
                springs.set_osd_tall(false);
                springs.set_osd_active(true);
            }
            force_render = true;
        }
        if let Some(br_pct) = brightness_mon.poll_changed() {
            springs.set_brightness(br_pct as f32);
            if !hover_sm.expanded && cfg.osd_brightness {
                active_osd = CollapsedOsdKind::Brightness;
                osd_deadline = Some(now + Duration::from_millis(1750));
                springs.set_osd_tall(false);
                springs.set_osd_active(true);
            }
            force_render = true;
        }
        if let Some((charging, bat_pct)) = power_charge_mon.poll_changed() {
            widgets.refresh_power_status();
            enqueue_or_show_osd(
                QueuedOsd::Charging {
                    charging,
                    pct: bat_pct,
                },
                &cfg,
                now,
                hover_sm.expanded,
                &mut widgets,
                &mut active_osd,
                &mut osd_deadline,
                &mut osd_queue,
                &mut springs,
            );
            force_render = true;
        }
        if let Some(net_ev) = network_mon.poll_changed() {
            enqueue_or_show_osd(
                QueuedOsd::Network(net_ev),
                &cfg,
                now,
                hover_sm.expanded,
                &mut widgets,
                &mut active_osd,
                &mut osd_deadline,
                &mut osd_queue,
                &mut springs,
            );
            force_render = true;
        }
        if let Some(toast) = toast_mon.poll_new() {
            has_notification = true;
            enqueue_or_show_osd(
                QueuedOsd::Notification(toast),
                &cfg,
                now,
                hover_sm.expanded,
                &mut widgets,
                &mut active_osd,
                &mut osd_deadline,
                &mut osd_queue,
                &mut springs,
            );
            force_render = true;
        }
        if snap.external_volume_seq != last_ext_vol_seq {
            last_ext_vol_seq = snap.external_volume_seq;
            volume_pct = snap.volume_pct;
            muted = snap.muted;
            if let Some(ev) = widgets.poll_bluetooth_events() {
                last_power_poll = now;
                bt_osd_suppress_vol_until = Some(now + Duration::from_millis(3200));
                enqueue_or_show_osd(
                    queued_osd_from_bt(ev),
                    &cfg,
                    now,
                    hover_sm.expanded,
                    &mut widgets,
                    &mut active_osd,
                    &mut osd_deadline,
                    &mut osd_queue,
                    &mut springs,
                );
            } else {
                let bt_has_priority = active_osd == CollapsedOsdKind::Bluetooth
                    || bt_osd_suppress_vol_until.is_some_and(|t| now < t);
                if !hover_sm.expanded && !bt_has_priority && cfg.osd_volume {
                    active_osd = CollapsedOsdKind::Volume;
                    osd_deadline = Some(now + Duration::from_secs(2));
                    springs.set_osd_tall(false);
                    springs.set_osd_active(true);
                }
            }
            force_render = true;
        } else {
            let can_sync_volume = !dragging_volume
                && last_volume_write
                    .map(|t| t.elapsed() >= Duration::from_millis(300))
                    .unwrap_or(true);
            if can_sync_volume
                && ((snap.volume_pct - volume_pct).abs() > 0.25 || snap.muted != muted)
            {
                volume_pct = snap.volume_pct;
                muted = snap.muted;
                force_render = true;
            }
        }
        if let Some(deadline) = osd_deadline {
            let is_alert = matches!(
                active_osd,
                CollapsedOsdKind::FocusAlert { .. }
                    | CollapsedOsdKind::WaterReminder
                    | CollapsedOsdKind::TodoReminder
            );
            if is_alert {
                if hover_sm.hovered {
                    let paused_deadline = deadline + Duration::from_secs_f32(dt.max(0.0));
                    osd_deadline = Some(paused_deadline);
                    let rem = paused_deadline.saturating_duration_since(now).as_secs_f32();
                    widgets.osd_alert_progress = (rem / 12.0).clamp(0.0, 1.0);
                } else {
                    let rem = deadline.saturating_duration_since(now).as_secs_f32();
                    widgets.osd_alert_progress = (rem / 12.0).clamp(0.0, 1.0);
                    if now >= deadline {
                        osd_deadline = None;
                        springs.set_osd_active(false);
                        force_render = true;
                    }
                }
            } else if now >= deadline {
                osd_deadline = None;
                springs.set_osd_active(false);
                force_render = true;
            }
        } else if active_osd != CollapsedOsdKind::None && springs.osd_open() <= 0.04 {
            if let Some(next_osd) = osd_queue.pop_front() {
                activate_queued_osd(
                    next_osd,
                    now,
                    &mut widgets,
                    &mut active_osd,
                    &mut osd_deadline,
                    &mut springs,
                );
                force_render = true;
            } else if springs.osd_open() <= 0.01 {
                active_osd = CollapsedOsdKind::None;
            }
        }
        let closed_text = if !frame_lyric_pair.current.is_empty() {
            frame_lyric_pair.current.as_str()
        } else if !frame_lyric_pair.next.is_empty() {
            frame_lyric_pair.next.as_str()
        } else if !snap.title.is_empty() {
            snap.title.as_str()
        } else {
            "Playing..."
        };
        let closed_w = renderer.measure_closed_lyric_target_width(closed_text);
        let effective_lyric_idx = if frame_lyric_pair.pre_start {
            1
        } else {
            frame_lyric_pair.active_index
        };
        let lyric_hash = compute_lyric_hash(closed_text, effective_lyric_idx);

        springs.set_active_page_index(page.index());
        springs.set_shelf_clipboard_tab(is_clipboard_tab);
        springs.set_water_reminder(water.reminder_enabled);
        springs.set_water_gauge(water.show_gauge);
        springs.set_volume(volume_pct, muted);
        springs.set_volume_hovered(volume_hovered);
        if snap.track_key != last_lyric_track_key {
            last_lyric_track_key.clone_from(&snap.track_key);
            if frame_lyric_pair.pre_start || !frame_lyric_pair.has_synced {
                springs.lyric_started_mix.snap_to(0.0);
            }
        }
        springs.set_lyric_started(!frame_lyric_pair.pre_start && frame_lyric_pair.has_synced);
        if hover_sm.expanded && has_notification {
            let started = *notif_seen_since.get_or_insert(now);
            if now.saturating_duration_since(started) >= Duration::from_millis(2500) {
                has_notification = false;
                notif_seen_since = None;
                force_render = true;
            }
        } else {
            notif_seen_since = None;
        }
        springs.set_has_notification(has_notification);
        springs.set_shelf_zone_hovered(shelf_zone_hovered);
        springs.sync_content_transitions(
            shelf_show_dropzone,
            clip_empty,
            pomodoro.running,
            todo.adding,
            todo.editing,
            todo.visible_len() == 0,
        );
        if auto_ring_override.is_some_and(|(_, until)| now >= until) {
            auto_ring_override = None;
        }
        let effective_left_ring = if cfg.clock_ring_mode == 3 {
            if let Some((ov_mode, _)) = auto_ring_override {
                ov_mode
            } else {
                NotchSprings::effective_left_ring_mode(
                    cfg.clock_ring_mode,
                    pomodoro.running,
                    water.taken_ml,
                )
            }
        } else {
            NotchSprings::effective_left_ring_mode(
                cfg.clock_ring_mode,
                pomodoro.running,
                water.taken_ml,
            )
        };
        let todo_prog = if todo.items.is_empty() {
            0.0
        } else {
            todo.completed_count() as f32 / todo.items.len() as f32
        };
        springs.sync_left_ring_todo_prog(todo_prog);
        springs.sync_ring_transitions(
            effective_left_ring,
            snap.has_session && snap.thumbnail.is_some(),
            snap.has_session && snap.is_playing,
        );
        if (cfg.widget_left == 7 || cfg.widget_right == 7)
            && widgets.poll_weather(cfg.widget_weather_city)
        {
            force_render = true;
        }
        if page == NotchPage::Widgets
            && hover_sm.expanded
            && last_power_poll.elapsed() >= Duration::from_secs(2)
        {
            widgets.refresh_power_status();
            last_power_poll = Instant::now();
        }
        let active_bat = widgets.active_battery();
        let bat_prog = match active_bat.as_ref() {
            Some(d) if !d.is_ac_only => d.pct as f32 / 100.0,
            Some(_) => 1.0,
            None => 0.0,
        };
        let bat_rgb = match active_bat.as_ref() {
            Some(d) if d.is_bluetooth => {
                if d.pct <= 15 {
                    (1.0, 0.27, 0.23)
                } else if d.pct <= 25 {
                    (1.0, 0.62, 0.04)
                } else {
                    (0.24, 0.68, 1.0)
                }
            }
            Some(d) => {
                if d.is_ac_only || d.charging || d.pct > 25 {
                    (0.188, 0.820, 0.349)
                } else if d.pct > 15 {
                    (1.0, 0.62, 0.04)
                } else {
                    (1.0, 0.27, 0.23)
                }
            }
            None => (0.55, 0.58, 0.65),
        };
        let (_, _, countdown_prog) = WidgetState::countdown_info(cfg.widget_countdown);
        springs.sync_widget_springs(
            page == NotchPage::Widgets && hover_sm.expanded,
            widgets.selected_week_day,
            water.progress(),
            bat_prog,
            pomodoro.running,
            pomodoro.progress(),
            countdown_prog,
            hover_target == HitTarget::WidgetCardLeft,
            hover_target == HitTarget::WidgetCardRight,
        );
        springs.sync_widget_bat_rgb(bat_rgb);
        springs.sync_water_hourly(water.visible_window().0);
        springs.sync_shelf_select(shelf_store.borrow().select_mode);
        springs.sync_album_art_rev(snap.thumbnail.as_ref().map_or(0, |t| t.revision));
        let pc_idle_secs = system_idle_secs();
        let eave_can_show = cfg.show_mascot
            && !hidden
            && !hover_sm.expanded
            && !hover_sm.hovered
            && !is_drag_over_now
            && osd_deadline.is_none()
            && !lyrics_active;
        let (mascot_visible, mascot_asleep) = if !eave_can_show {
            eave_was_idle_active = false;
            mascot_wake_until = None;
            (false, false)
        } else {
            let raw_idle_awake = pc_idle_secs >= MASCOT_IDLE_AWAKE_SECS;
            let raw_idle_asleep = pc_idle_secs >= MASCOT_IDLE_SLEEP_SECS;
            if raw_idle_awake {
                eave_was_idle_active = true;
                mascot_wake_until = None;
            } else if eave_was_idle_active {
                eave_was_idle_active = false;
                mascot_wake_until =
                    Some(Instant::now() + Duration::from_secs_f32(MASCOT_WAKE_LINGER_SECS));
            }
            if mascot_wake_until.is_some_and(|until| Instant::now() >= until) {
                mascot_wake_until = None;
            }
            (raw_idle_awake || mascot_wake_until.is_some(), raw_idle_asleep)
        };
        springs.sync_mascot_idle(mascot_visible, mascot_asleep, mascot_wake_until.is_some());
        springs.sync_clock_visible(cfg.show_clock);
        springs.set_todo_list_expanded(todo.expanded_list && !todo.adding && todo.visible_len() > 0);
        springs.set_lyric_index(Some(effective_lyric_idx));
        springs.sync_lyrics_state(lyrics_active, closed_w, lyric_hash, song_rgb);
        springs.apply_state(
            &cfg,
            hover_sm.expanded,
            hover_sm.hovered,
            dock_hovered,
            is_drag_over_now,
        );
        let mag_ptr = if hover_sm.expanded && !(page == NotchPage::Music && lyrics_open) {
            last_pointer_notch
        } else {
            None
        };
        let ear_r_live = springs.ear_radius.value;
        let cur_w_live = springs.width.value;
        let mag_animating = springs.step_magnetic_arrows(
            mag_ptr,
            (ear_r_live + 13.0, 94.0),
            (cur_w_live - ear_r_live - 13.0, 94.0),
            dt,
        );
        let clip_copy_animating = shelf_store.borrow().copied_feedback.is_some();
        let shelf_scrolling = shelf_store.borrow_mut().step_scroll_animations(dt);
        let todo_animating = todo.step_animations(dt);
        let settings_animating = settings_win.tick_and_paint();
        let widget_has_ambient = |k: u8| match k {
            1 => active_bat.as_ref().is_some_and(|d| d.charging),
            2 | 4 | 5 | 7 => true,
            3 => pomodoro.running,
            _ => false,
        };
        let widget_ambient_animating = page == NotchPage::Widgets
            && hover_sm.expanded
            && (widget_has_ambient(cfg.widget_left) || widget_has_ambient(cfg.widget_right));
        let animating = !springs.is_idle_settled(snap.is_playing)
            || todo_animating
            || pomodoro.running
            || clip_copy_animating
            || shelf_scrolling
            || mag_animating
            || settings_animating
            || widget_ambient_animating
            || (matches!(
                active_osd,
                CollapsedOsdKind::Bluetooth
                    | CollapsedOsdKind::Charging { .. }
                    | CollapsedOsdKind::MicInUse
                    | CollapsedOsdKind::FocusAlert { .. }
                    | CollapsedOsdKind::WaterReminder
                    | CollapsedOsdKind::TodoReminder
            ) && osd_deadline.is_some())
            || springs.mascot_idle_presence.value() > 0.01
            || springs.ai_wave_open.value() > 0.01
            || widgets.ai.phase == ai::AiPhase::Input;
        springs
            .ai_copy_anim
            .set_target(if widgets.ai.copied_feedback { 1.0 } else { 0.0 });
        springs.step(dt, snap.is_playing, false);
        if fullscreen_should_hide && !hidden && springs.notch_reveal.value() <= 0.01 {
            window.set_visible(false);
            hidden = true;
        }
        if springs.take_shelf_clear_finished() {
            shelf_store.borrow_mut().clear_active_tab();
            let (after_len, after_show_dropzone, after_clip_empty) = {
                let s = shelf_store.borrow();
                (
                    s.items.len(),
                    s.items.is_empty() || is_drag_over_now,
                    s.clipboard_items.is_empty(),
                )
            };
            prev_shelf_len = after_len;
            springs.sync_content_transitions(
                after_show_dropzone,
                after_clip_empty,
                pomodoro.running,
                todo.adding,
                todo.editing,
                todo.visible_len() == 0,
            );
            force_render = true;
        }

        let interactive_anim = !springs.is_interactive_settled()
            || todo_animating
            || clip_copy_animating
            || shelf_scrolling
            || mag_animating
            || settings_animating
            || force_render
            || need_resize
            || springs.eave_is_waking();
        let pace_interval = if springs.mascot_idle_presence.value() > 0.98
            && springs.mascot_idle_sleep.value() > 0.98
            && springs.eave_sleep_z_elapsed().is_none()
        {
            Duration::from_millis(33)
        } else {
            Duration::from_millis(16)
        };
        let did_render = if interactive_anim {
            true
        } else if animating {
            now.duration_since(last_render_at) >= pace_interval
        } else {
            false
        };
        if did_render {
            last_render_at = now;
            let shelf_ref = shelf_store.borrow();
            render_frame(
                &cfg,
                &springs,
                &media,
                &lyrics,
                &mut renderer,
                &mut compositor,
                &mut window,
                max_w_dips,
                max_h_dips,
                page,
                prev_page,
                &pomodoro,
                &water,
                &shelf_ref,
                &todo,
                &widgets,
                lyrics_open,
                muted,
                has_notification,
                song_rgb,
                hover_target,
                active_osd,
                hidden,
            );
        }

        if !did_render
            && !hidden
            && (animating
                || osd_deadline.is_some()
                || hover_sm.is_waiting_dwell()
                || hover_sm.is_waiting_leave_grace()
                || shelf_store.borrow().active_copied_index().is_some())
        {
            thread::sleep(Duration::from_millis(4));
        }
    }

    ai_svc.cancel();
    lyrics.shutdown();
    media.shutdown();
    Ok(())
}

fn open_url_in_default_browser(url: &str) {
    let trimmed = url.trim();
    if !(trimmed.starts_with("https://") || trimmed.starts_with("http://")) {
        return;
    }
    let wide_url = windows::core::HSTRING::from(trimmed);
    unsafe {
        let _ = windows::Win32::UI::Shell::ShellExecuteW(
            None,
            windows::core::w!("open"),
            &wide_url,
            None,
            None,
            windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL,
        );
    }
}

fn compute_surface_bounds(cfg: &Config) -> (f32, f32) {
    let max_w = cfg.expanded_width.max(656.0).max(cfg.notch_width) + SHADOW_PAD_X_DIPS * 2.0;
    let max_h = (cfg.expanded_height.max(cfg.notch_height)
        + DOCK_EXTRA_H_DIPS
        + SHADOW_PAD_BOTTOM_DIPS)
        .max(512.0);
    (max_w, max_h)
}

fn volume_slider_ratio_from_x(x_dips: f32, total_w_dips: f32, ear_r_dips: f32) -> f64 {
    let body_w = (total_w_dips - 2.0 * ear_r_dips).max(160.0);
    let body_right = (total_w_dips + body_w) * 0.5;
    let speaker_x = body_right - 41.0;
    let slider_right = speaker_x - 15.0;
    let slider_left = slider_right - 65.0;
    ((x_dips - slider_left) / 65.0).clamp(0.0, 1.0) as f64
}

fn progress_bar_ratio_from_x(
    x_dips: f32,
    total_w_dips: f32,
    ear_r_dips: f32,
    lyrics_split: bool,
) -> f64 {
    let body_w = (total_w_dips - 2.0 * ear_r_dips).max(160.0);
    let body_left = (total_w_dips - body_w) * 0.5;
    let body_right = body_left + body_w;
    let (left, right) = if lyrics_split {
        (body_left + 69.0, body_left + 188.0)
    } else {
        (body_left + 76.0, (body_right - 79.0).max(body_left + 108.0))
    };
    let width = (right - left).max(1.0);
    ((x_dips - left) / width).clamp(0.0, 1.0) as f64
}

fn apply_volume_pct(
    next_pct: f32,
    volume_pct: &mut f32,
    muted: &mut bool,
    last_volume_write: &mut Option<Instant>,
    media: &MediaService,
    springs: &mut NotchSprings,
) {
    *volume_pct = next_pct.clamp(0.0, 100.0);
    if *volume_pct > 0.0 && *muted {
        *muted = false;
        media.send_command(MediaCommand::SetMuted(false));
    }
    *last_volume_write = Some(Instant::now());
    media.send_command(MediaCommand::SetVolume(*volume_pct));
    springs.set_volume(*volume_pct, *muted);
}

fn update_song_color_cache(
    snap: &media::MediaSnapshot,
    last_art_rev: &mut u64,
    song_rgb: &mut (f32, f32, f32),
) {
    if !snap.has_session {
        *last_art_rev = 0;
        *song_rgb = (0.22, 0.47, 0.81);
        return;
    }
    match snap.thumbnail.as_ref() {
        Some(thumb) if thumb.revision != *last_art_rev => {
            *last_art_rev = thumb.revision;
            *song_rgb = renderer::extract_dominant_song_color(&thumb.bgra);
        }
        None => {
            *last_art_rev = 0;
            *song_rgb = (0.22, 0.47, 0.81);
        }
        _ => {}
    }
}

fn compute_lyric_hash(closed_text: &str, active_index: usize) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    closed_text.hash(&mut hasher);
    active_index.hash(&mut hasher);
    hasher.finish().max(1)
}

#[allow(clippy::too_many_arguments)]
fn render_frame(
    cfg: &Config,
    springs: &NotchSprings,
    media: &MediaService,
    lyrics: &LyricsService,
    renderer: &mut NotchRenderer,
    compositor: &mut CompositorHost,
    window: &mut NotchWindow,
    max_w_dips: f32,
    max_h_dips: f32,
    page: NotchPage,
    prev_page: NotchPage,
    pomodoro: &PomodoroState,
    water: &WaterState,
    shelf: &ShelfStore,
    todo: &TodoState,
    widgets: &WidgetState,
    lyrics_open: bool,
    muted: bool,
    has_notification: bool,
    song_rgb: (f32, f32, f32),
    hover_target: HitTarget,
    collapsed_osd: CollapsedOsdKind,
    hidden: bool,
) {
    let cur_w = springs.width.value;
    let cur_h = springs.height.value;
    let radius = springs.radius.value;
    let ear_r = springs.ear_radius.value;

    window.update_bounds(cur_w, cur_h, radius, ear_r, cfg.top_offset);

    let scale = window.dpi_scale();
    let _ = compositor.update_layout(
        cur_w * scale,
        cur_h * scale,
        radius * scale,
        ear_r * scale,
        max_w_dips * scale,
        max_h_dips * scale,
    );

    if hidden {
        return;
    }

    let snap = media.snapshot();
    let lyric_pair = if cfg.show_lyrics && !cfg.minimal_mode && snap.has_session {
        lyrics.get_lines(
            &snap.track_key,
            &snap.title,
            &snap.artist,
            snap.position_secs,
        )
    } else {
        LyricLinePair::default()
    };

    let thumb_view = snap.thumbnail.as_ref().map(|t| AlbumArtView {
        width: t.width,
        height: t.height,
        bgra: &t.bgra,
        revision: t.revision,
    });

    let media_state = MediaViewState {
        has_session: snap.has_session,
        is_playing: snap.is_playing,
        title: &snap.title,
        artist: &snap.artist,
        album: &snap.album,
        position_secs: snap.position_secs,
        duration_secs: snap.duration_secs,
        thumbnail: thumb_view,
        lyric_prev_prev: &lyric_pair.prev_prev,
        lyric_prev: &lyric_pair.prev,
        lyric_current: &lyric_pair.current,
        lyric_next: &lyric_pair.next,
        lyric_next_after: &lyric_pair.next_after,
        lyric_started: !lyric_pair.pre_start,
        lyric_prev_time: lyric_pair.prev_time_secs,
        lyric_current_time: lyric_pair.current_time_secs,
        lyric_next_time: lyric_pair.next_time_secs,
        ambient_rgb: Some(song_rgb),
    };

    let view_state = EaveViewState {
        page,
        prev_page,
        media: media_state,
        pomodoro,
        water,
        shelf,
        todo,
        widgets,
        lyrics_open,
        muted,
        has_notification,
        collapsed_osd,
    };

    let _ = renderer.render(cfg, springs, &view_state, hover_target);
}

#[inline]
fn filter_media_hit(target: HitTarget, has_session: bool) -> HitTarget {
    if !has_session
        && matches!(
            target,
            HitTarget::LyricsButton
                | HitTarget::PrevButton
                | HitTarget::PlayPauseButton
                | HitTarget::NextButton
                | HitTarget::ProgressBar(_)
        )
    {
        HitTarget::Body
    } else {
        target
    }
}

fn screen_to_canvas_dips(hwnd: HWND, dpi_scale: f32, screen_x: i32, screen_y: i32) -> (f32, f32) {
    let mut pt = POINT {
        x: screen_x,
        y: screen_y,
    };
    unsafe {
        let _ = ScreenToClient(hwnd, &mut pt);
    }
    (pt.x as f32 / dpi_scale, pt.y as f32 / dpi_scale)
}

fn build_settings_snapshot(
    cfg: &Config,
    pomodoro: &PomodoroState,
    water: &WaterState,
    todo: &TodoState,
    shelf_store: &Rc<RefCell<ShelfStore>>,
    update_status: updater::UpdateStatus,
) -> SettingsSnapshot {
    let s = shelf_store.borrow();
    SettingsSnapshot {
        cfg: cfg.clone(),
        start_with_windows: false,
        pomo_focus_mins: (pomodoro.focus_secs / 60.0).round() as u32,
        pomo_break_mins: (pomodoro.break_secs / 60.0).round() as u32,
        pomo_long_break_mins: (pomodoro.long_break_secs / 60.0).round() as u32,
        water_goal_ml: water.goal_ml,
        water_bottle_ml: water.bottle_ml,
        water_reminder_enabled: water.reminder_enabled,
        todo_done: todo.completed_count(),
        todo_total: todo.items.len(),
        shelf_files_count: s.items.len(),
        shelf_clip_count: s.clipboard_items.len(),
        update_status,
    }
}

fn open_settings_file(cfg: &Config, cfg_path: &PathBuf) {
    if !cfg_path.exists() {
        let _ = cfg.save_to(cfg_path);
    }
    let _ = Command::new("notepad.exe").arg(cfg_path).spawn();
}
