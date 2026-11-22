use crate::config::Config;

#[derive(Debug, Clone, Copy)]
pub struct Spring {
    pub value: f32,
    pub target: f32,
    pub velocity: f32,
    pub stiffness: f32,
    pub damping: f32,
}

impl Spring {
    pub const fn new(value: f32, stiffness: f32, damping: f32) -> Self {
        Self {
            value,
            target: value,
            velocity: 0.0,
            stiffness,
            damping,
        }
    }

    pub fn from_response(value: f32, frequency_hz: f32, damping_ratio: f32) -> Self {
        let mut s = Self::new(value, 1.0, 1.0);
        s.set_response(frequency_hz, damping_ratio);
        s
    }

    pub fn set_response(&mut self, frequency_hz: f32, damping_ratio: f32) {
        let omega = 2.0 * std::f32::consts::PI * frequency_hz.max(0.1);
        self.stiffness = omega * omega;
        self.damping = 2.0 * damping_ratio.max(0.05) * omega;
    }

    pub fn set_target(&mut self, target: f32) {
        self.target = target;
    }

    pub fn snap_to(&mut self, value: f32) {
        self.value = value;
        self.target = value;
        self.velocity = 0.0;
    }

    pub fn step(&mut self, dt: f32) -> f32 {
        self.step_eps(dt, 0.05, 0.25)
    }

    pub fn step_unit(&mut self, dt: f32) -> f32 {
        self.step_eps(dt, 0.002, 0.01)
    }

    fn step_eps(&mut self, dt: f32, pos_eps: f32, vel_eps: f32) -> f32 {
        if self.is_settled_eps(pos_eps, vel_eps) {
            self.value = self.target;
            self.velocity = 0.0;
            return self.value;
        }

        let total_dt = dt.clamp(0.0, 0.05);
        if total_dt <= 0.0 {
            return self.value;
        }

        const MAX_SUBSTEP: f32 = 1.0 / 240.0;
        let steps = (total_dt / MAX_SUBSTEP).ceil().max(1.0) as u32;
        let sub_dt = total_dt / steps as f32;

        for _ in 0..steps {
            let displacement = self.value - self.target;
            let accel = -self.stiffness * displacement - self.damping * self.velocity;
            self.velocity += accel * sub_dt;
            self.value += self.velocity * sub_dt;
        }

        if self.is_settled_eps(pos_eps, vel_eps) {
            self.value = self.target;
            self.velocity = 0.0;
        }

        self.value
    }

    pub fn is_settled(&self) -> bool {
        self.is_settled_eps(0.05, 0.25)
    }

    pub fn is_unit_settled(&self) -> bool {
        self.is_settled_eps(0.002, 0.01)
    }

    fn is_settled_eps(&self, pos_eps: f32, vel_eps: f32) -> bool {
        (self.value - self.target).abs() < pos_eps && self.velocity.abs() < vel_eps
    }
}

#[derive(Debug, Clone, Copy)]
pub struct UnitSpring {
    inner: Spring,
}

impl UnitSpring {
    pub fn from_response(value: f32, frequency_hz: f32, damping_ratio: f32) -> Self {
        Self {
            inner: Spring::from_response(value, frequency_hz, damping_ratio),
        }
    }

    pub fn set_response(&mut self, frequency_hz: f32, damping_ratio: f32) {
        self.inner.set_response(frequency_hz, damping_ratio);
    }

    pub fn value(&self) -> f32 {
        self.inner.value
    }

    pub fn target(&self) -> f32 {
        self.inner.target
    }

    pub fn set_target(&mut self, target: f32) {
        self.inner.set_target(target);
    }

    pub fn snap_to(&mut self, value: f32) {
        self.inner.snap_to(value);
    }

    pub fn step(&mut self, dt: f32) -> f32 {
        self.inner.step_unit(dt)
    }

    pub fn is_settled(&self) -> bool {
        self.inner.is_unit_settled()
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct HoverStateMachine {
    pub hovered: bool,
    pub expanded: bool,
    pub dwell_secs: f32,
    pub leave_grace_secs: f32,
    leaving: bool,
}

impl HoverStateMachine {
    pub const DWELL_SECS: f32 = 0.300;
    pub const LEAVE_GRACE_SECS: f32 = 0.140;

    pub const fn new() -> Self {
        Self {
            hovered: false,
            expanded: false,
            dwell_secs: 0.0,
            leave_grace_secs: 0.0,
            leaving: false,
        }
    }

    pub fn notify_move(&mut self, in_notch: bool) {
        if in_notch {
            self.leaving = false;
            self.leave_grace_secs = 0.0;
            if !self.hovered {
                self.hovered = true;
                self.dwell_secs = 0.0;
            }
        } else {
            self.notify_leave();
        }
    }

    pub fn notify_leave(&mut self) {
        if self.hovered || self.expanded {
            self.leaving = true;
        }
    }

    pub fn tick(&mut self, dt: f32, open_on_hover: bool, is_drag_over: bool) {
        let dt = dt.clamp(0.0, 1.0);
        if is_drag_over {
            self.leaving = false;
            self.leave_grace_secs = 0.0;
            self.hovered = true;
            self.expanded = true;
            return;
        }

        if self.leaving {
            self.leave_grace_secs += dt;
            if self.leave_grace_secs >= Self::LEAVE_GRACE_SECS {
                self.leaving = false;
                self.leave_grace_secs = 0.0;
                self.hovered = false;
                self.dwell_secs = 0.0;
                if open_on_hover {
                    self.expanded = false;
                }
            }
        } else if self.hovered && !self.expanded && open_on_hover {
            self.dwell_secs += dt;
            if self.dwell_secs >= Self::DWELL_SECS {
                self.expanded = true;
            }
        }
    }

    pub fn toggle_click(&mut self, open_on_hover: bool) {
        if !open_on_hover {
            self.expanded = !self.expanded;
            self.dwell_secs = 0.0;
        } else if !self.expanded {
            self.expanded = true;
            self.hovered = true;
            self.leaving = false;
            self.leave_grace_secs = 0.0;
        }
    }

    pub fn reset(&mut self) {
        self.hovered = false;
        self.expanded = false;
        self.dwell_secs = 0.0;
        self.leave_grace_secs = 0.0;
        self.leaving = false;
    }

    pub fn is_waiting_dwell(&self) -> bool {
        self.hovered && !self.expanded && !self.leaving
    }

    pub fn is_waiting_leave_grace(&self) -> bool {
        self.leaving
    }
}

pub const MAX_HOVER_SLOTS: usize = 47;

#[derive(Debug, Clone, Copy, Default)]
pub struct MagneticArrowState {
    pub x: f32,
    pub y: f32,
    pub s: f32,
}

impl MagneticArrowState {
    pub fn step(
        &mut self,
        pointer: Option<(f32, f32)>,
        anchor: (f32, f32),
        inward_sign: f32,
        dt: f32,
    ) -> bool {
        const RADIUS: f32 = 58.0;
        const MAX_MOVE: f32 = 6.0;
        const PULL: f32 = 0.35;
        const EASE: f32 = 0.20;
        const LEAVE_FACTOR: f32 = 0.60;

        let mut tx = 0.0_f32;
        let mut ty = 0.0_f32;
        let mut ts = 0.0_f32;

        if let Some((px, py)) = pointer {
            let dx = px - anchor.0;
            let dy = py - anchor.1;
            let d = dx.hypot(dy);
            if d < RADIUS {
                let u = 1.0 - d / RADIUS;
                ts = 1.0 - (1.0 - u) * (1.0 - u);
                tx = dx * PULL * ts;
                ty = dy * PULL * ts;
                if inward_sign > 0.0 {
                    tx = tx.max(0.0);
                } else {
                    tx = tx.min(0.0);
                }
                let len = tx.hypot(ty);
                if len > MAX_MOVE {
                    tx = (tx / len) * MAX_MOVE;
                    ty = (ty / len) * MAX_MOVE;
                }
            }
        }

        let rate = if ts > 0.001 { EASE } else { EASE * LEAVE_FACTOR };
        let dt_ms = (dt.clamp(0.0, 0.05) * 1000.0).max(0.1);
        let f = 1.0 - (1.0 - rate).powf(dt_ms / 16.67);
        self.x += (tx - self.x) * f;
        self.y += (ty - self.y) * f;
        self.s += (ts - self.s) * f;

        if ts <= 0.001 && self.x.abs() < 0.01 && self.y.abs() < 0.01 && self.s < 0.002 {
            self.x = 0.0;
            self.y = 0.0;
            self.s = 0.0;
            false
        } else {
            true
        }
    }
}

#[derive(Debug, Clone)]
pub struct NotchSprings {
    pub width: Spring,
    pub height: Spring,
    pub radius: Spring,
    pub ear_radius: Spring,
    pub expand: UnitSpring,
    pub expand_bloom: Spring,
    pub hover: UnitSpring,
    pub dock_hover: UnitSpring,
    pub dock_wobble: UnitSpring,
    pub page_transition: UnitSpring,
    pub page_dir: f32,
    pub drag_over: UnitSpring,
    pub shelf_hover: UnitSpring,
    pub shelf_gulp: Spring,
    pub dock_item_hover: [UnitSpring; 7],
    pub dock_item_press: [UnitSpring; 7],
    pub hover_slots: [UnitSpring; MAX_HOVER_SLOTS],
    pub press_slots: [UnitSpring; MAX_HOVER_SLOTS],
    pub bell_visible: UnitSpring,
    pub dot_active: [UnitSpring; 6],
    pub shelf_tab_clipboard: UnitSpring,
    pub water_switch_on: UnitSpring,
    pub water_view_gauge: UnitSpring,
    pub dark_bg_border: UnitSpring,
    pub cursor_x: Spring,
    pub cursor_y: Spring,
    pub mag_left: MagneticArrowState,
    pub mag_right: MagneticArrowState,
    mag_arrows_moving: bool,
    pub shelf_drop_t: f32,
    pub shelf_drop_x: f32,
    pub card_drop_t: [f32; 5],
    pub card_spawn: [Spring; 5],
    shelf_clear_active: bool,
    shelf_clear_elapsed: f32,
    shelf_clear_count: usize,
    shelf_clear_finished: bool,
    pub shelf_empty_reveal: UnitSpring,
    pub clipboard_empty_reveal: UnitSpring,
    pub pomo_running_mix: UnitSpring,
    pub todo_form_open: UnitSpring,
    pub todo_edit_mode: UnitSpring,
    pub todo_empty_reveal: UnitSpring,
    pub todo_list_expanded: UnitSpring,
    pub lyrics_mode: UnitSpring,
    pub closed_lyric_width: Spring,
    pub lyric_line_change: UnitSpring,
    pub lyric_started_mix: UnitSpring,
    pub lyric_scroll_dir: f32,
    last_lyric_index: Option<usize>,
    pub osd_open: UnitSpring,
    pub osd_tall: bool,
    pub osd_wide: bool,
    pub ai_phase: crate::ai::AiPhase,
    pub ai_active: UnitSpring,
    pub ai_input_open: UnitSpring,
    pub ai_wave_open: UnitSpring,
    pub ai_wave_energy: UnitSpring,
    ai_wave_phase: f32,
    pub ai_card_open: UnitSpring,
    pub ai_card_h: Spring,
    pub ai_content_h: Spring,
    pub ai_scroll: Spring,
    pub ai_followup_open: UnitSpring,
    pub ai_scrollbar_vis: UnitSpring,
    pub ai_copy_anim: UnitSpring,
    pub ai_btn_hover: [UnitSpring; 5],
    pub ai_shake: Spring,
    pub bt_popup_arc: UnitSpring,
    bt_popup_delay: f32,
    pub osd_vol_bounce: Spring,
    osd_vol_bounce_cd: f32,
    pub caps_on_mix: UnitSpring,
    pub brightness_pct: Spring,
    pub mic_muted_mix: UnitSpring,
    pub song_color_r: UnitSpring,
    pub song_color_g: UnitSpring,
    pub song_color_b: UnitSpring,
    last_lyric_hash: u64,
    pub visualizer: [UnitSpring; 5],
    pub speaker_volume: Spring,
    pub volume_slider_open: UnitSpring,
    pub play_mix: UnitSpring,
    pub collapsed_art_mix: UnitSpring,
    pub audio_vis_mix: UnitSpring,
    pub left_ring_visible: UnitSpring,
    pub left_ring_switch: UnitSpring,
    pub left_ring_todo_prog: UnitSpring,
    pub left_ring_mode: u8,
    pub left_ring_prev_mode: u8,
    pub widget_day_idx: Spring,
    pub widget_day_pop: UnitSpring,
    pub widget_day_switch: UnitSpring,
    pub widget_left_hover: UnitSpring,
    pub widget_right_hover: UnitSpring,
    pub widget_left_press: UnitSpring,
    pub widget_right_press: UnitSpring,
    pub widget_enter: UnitSpring,
    pub widget_water_fill: UnitSpring,
    pub widget_water_splash: UnitSpring,
    pub widget_bat_arc: UnitSpring,
    pub widget_bat_switch: UnitSpring,
    pub widget_bat_rgb: [UnitSpring; 3],
    pub widget_focus_active: UnitSpring,
    pub widget_focus_prog: UnitSpring,
    pub widget_countdown_prog: UnitSpring,
    pub water_hourly: [UnitSpring; 8],
    pub shelf_select_mode: UnitSpring,
    pub album_art_fade: UnitSpring,
    pub clock_visible: UnitSpring,
    pub notch_reveal: UnitSpring,
    pub mascot_idle_presence: UnitSpring,
    pub mascot_idle_sleep: UnitSpring,
    eave_was_asleep: bool,
    mascot_wake_elapsed: Option<f32>,
    eave_sleep_z_active: bool,
    eave_sleep_z_elapsed: f32,
    eave_sleep_z_cooldown: f32,
    eave_aurora_phase: f32,
    eave_rng: u32,
    blink_double_pending: bool,
    last_art_rev: u64,
    widget_was_active: bool,
    pub mascot_look_x: UnitSpring,
    phase_secs: f32,
    blink_timer_secs: f32,
    blink_amount: f32,
}

impl NotchSprings {
    #[allow(dead_code)]
    pub const MAX_HOVER_SLOTS: usize = MAX_HOVER_SLOTS;

    pub fn new(cfg: &Config) -> Self {
        let collapsed_radius = 14.0_f32.min(cfg.notch_height * 0.48);
        let hover_slot = UnitSpring::from_response(0.0, 3.80, 0.82);
        let press_slot = UnitSpring::from_response(0.0, 5.60, 0.78);
        let spawn_spring = Spring::from_response(1.0, 2.35, 0.58);
        Self {
            width: Spring::from_response(cfg.notch_width, 2.75, 0.96),
            height: Spring::from_response(cfg.notch_height, 2.75, 0.96),
            radius: Spring::from_response(collapsed_radius, 2.75, 0.96),
            ear_radius: Spring::from_response(10.0, 2.75, 0.96),
            expand: UnitSpring::from_response(0.0, 2.75, 0.96),
            expand_bloom: Spring::from_response(0.0, 2.75, 0.96),
            hover: UnitSpring::from_response(0.0, 3.20, 0.78),
            dock_hover: UnitSpring::from_response(0.0, 2.75, 0.82),
            dock_wobble: UnitSpring::from_response(0.0, 2.55, 0.62),
            page_transition: UnitSpring::from_response(1.0, 2.85, 0.86),
            page_dir: 1.0,
            drag_over: UnitSpring::from_response(0.0, 3.40, 0.80),
            shelf_hover: UnitSpring::from_response(0.0, 3.40, 0.80),
            shelf_gulp: Spring::from_response(0.0, 3.20, 0.60),
            dock_item_hover: [UnitSpring::from_response(0.0, 3.80, 0.82); 7],
            dock_item_press: [UnitSpring::from_response(0.0, 5.60, 0.78); 7],
            hover_slots: [hover_slot; MAX_HOVER_SLOTS],
            press_slots: [press_slot; MAX_HOVER_SLOTS],
            bell_visible: UnitSpring::from_response(0.0, 3.00, 0.76),
            dot_active: [
                UnitSpring::from_response(1.0, 2.95, 0.76),
                UnitSpring::from_response(0.0, 2.95, 0.76),
                UnitSpring::from_response(0.0, 2.95, 0.76),
                UnitSpring::from_response(0.0, 2.95, 0.76),
                UnitSpring::from_response(0.0, 2.95, 0.76),
                UnitSpring::from_response(0.0, 2.95, 0.76),
            ],
            shelf_tab_clipboard: UnitSpring::from_response(0.0, 3.20, 0.80),
            water_switch_on: UnitSpring::from_response(0.0, 3.25, 0.80),
            water_view_gauge: UnitSpring::from_response(0.0, 3.10, 0.84),
            dark_bg_border: UnitSpring::from_response(0.0, 3.60, 0.92),
            cursor_x: Spring::from_response(280.0, 7.5, 0.88),
            cursor_y: Spring::from_response(90.0, 7.5, 0.88),
            mag_left: MagneticArrowState::default(),
            mag_right: MagneticArrowState::default(),
            mag_arrows_moving: false,
            shelf_drop_t: -1.0,
            shelf_drop_x: 280.0,
            card_drop_t: [-1.0; 5],
            card_spawn: [spawn_spring; 5],
            shelf_clear_active: false,
            shelf_clear_elapsed: 0.0,
            shelf_clear_count: 0,
            shelf_clear_finished: false,
            shelf_empty_reveal: UnitSpring::from_response(1.0, 2.80, 0.84),
            clipboard_empty_reveal: UnitSpring::from_response(0.0, 2.80, 0.84),
            pomo_running_mix: UnitSpring::from_response(0.0, 3.20, 0.94),
            todo_form_open: UnitSpring::from_response(0.0, 2.85, 0.84),
            todo_edit_mode: UnitSpring::from_response(0.0, 3.10, 0.82),
            todo_empty_reveal: UnitSpring::from_response(1.0, 2.80, 0.84),
            todo_list_expanded: UnitSpring::from_response(0.0, 2.00, 0.86),
            lyrics_mode: UnitSpring::from_response(0.0, 2.55, 0.82),
            closed_lyric_width: Spring::from_response(322.0, 2.70, 0.84),
            lyric_line_change: UnitSpring::from_response(1.0, 2.40, 0.84),
            lyric_started_mix: UnitSpring::from_response(1.0, 2.00, 0.88),
            lyric_scroll_dir: 1.0,
            last_lyric_index: None,
            osd_open: UnitSpring::from_response(0.0, 2.35, 0.85),
            osd_tall: false,
            osd_wide: false,
            ai_phase: crate::ai::AiPhase::Idle,
            ai_active: UnitSpring::from_response(0.0, 2.15, 0.92),
            ai_input_open: UnitSpring::from_response(0.0, 1.85, 0.88),
            ai_wave_open: UnitSpring::from_response(0.0, 1.80, 0.88),
            ai_wave_energy: UnitSpring::from_response(0.0, 1.35, 0.90),
            ai_wave_phase: 0.0,
            ai_card_open: UnitSpring::from_response(0.0, 1.55, 0.74),
            ai_card_h: Spring::from_response(96.0, 2.20, 0.86),
            ai_content_h: Spring::from_response(24.0, 3.00, 0.92),
            ai_scroll: Spring::from_response(0.0, 3.20, 0.92),
            ai_followup_open: UnitSpring::from_response(0.0, 2.40, 0.88),
            ai_scrollbar_vis: UnitSpring::from_response(0.0, 4.00, 0.90),
            ai_copy_anim: UnitSpring::from_response(0.0, 4.60, 0.82),
            ai_btn_hover: [UnitSpring::from_response(0.0, 4.40, 0.80); 5],
            ai_shake: Spring::from_response(0.0, 5.20, 0.36),
            bt_popup_arc: UnitSpring::from_response(0.0, 0.72, 0.92),
            bt_popup_delay: 0.0,
            osd_vol_bounce: Spring::from_response(0.0, 4.20, 0.44),
            osd_vol_bounce_cd: 0.0,
            caps_on_mix: UnitSpring::from_response(0.0, 2.20, 0.84),
            brightness_pct: Spring::from_response(100.0, 4.20, 0.86),
            mic_muted_mix: UnitSpring::from_response(0.0, 3.00, 0.86),
            song_color_r: UnitSpring::from_response(0.22, 1.80, 0.90),
            song_color_g: UnitSpring::from_response(0.47, 1.80, 0.90),
            song_color_b: UnitSpring::from_response(0.81, 1.80, 0.90),
            last_lyric_hash: 0,
            visualizer: [
                UnitSpring::from_response(0.24, 6.2, 0.76),
                UnitSpring::from_response(0.42, 6.8, 0.76),
                UnitSpring::from_response(0.62, 6.0, 0.74),
                UnitSpring::from_response(0.40, 6.6, 0.76),
                UnitSpring::from_response(0.24, 6.4, 0.76),
            ],
            speaker_volume: Spring::from_response(100.0, 3.60, 0.86),
            volume_slider_open: UnitSpring::from_response(0.0, 1.90, 0.88),
            play_mix: UnitSpring::from_response(0.0, 3.80, 0.82),
            collapsed_art_mix: UnitSpring::from_response(0.0, 3.40, 0.95),
            audio_vis_mix: UnitSpring::from_response(0.0, 3.40, 0.95),
            left_ring_visible: UnitSpring::from_response(0.0, 3.40, 0.95),
            left_ring_switch: UnitSpring::from_response(1.0, 3.40, 0.95),
            left_ring_todo_prog: UnitSpring::from_response(0.0, 3.10, 0.88),
            left_ring_mode: 0,
            left_ring_prev_mode: 0,
            widget_day_idx: Spring::from_response(0.0, 3.80, 0.86),
            widget_day_pop: UnitSpring::from_response(0.0, 5.20, 0.72),
            widget_day_switch: UnitSpring::from_response(1.0, 3.60, 0.92),
            widget_left_hover: UnitSpring::from_response(0.0, 3.80, 0.90),
            widget_right_hover: UnitSpring::from_response(0.0, 3.80, 0.90),
            widget_left_press: UnitSpring::from_response(0.0, 5.80, 0.78),
            widget_right_press: UnitSpring::from_response(0.0, 5.80, 0.78),
            widget_enter: UnitSpring::from_response(1.0, 3.10, 0.88),
            widget_water_fill: UnitSpring::from_response(0.0, 2.80, 0.88),
            widget_water_splash: UnitSpring::from_response(0.0, 2.40, 0.68),
            widget_bat_arc: UnitSpring::from_response(1.0, 2.60, 0.88),
            widget_bat_switch: UnitSpring::from_response(1.0, 3.40, 0.94),
            widget_bat_rgb: [
                UnitSpring::from_response(0.188, 3.00, 0.92),
                UnitSpring::from_response(0.820, 3.00, 0.92),
                UnitSpring::from_response(0.349, 3.00, 0.92),
            ],
            widget_focus_active: UnitSpring::from_response(0.0, 3.40, 0.88),
            widget_focus_prog: UnitSpring::from_response(0.0, 2.80, 0.88),
            widget_countdown_prog: UnitSpring::from_response(0.0, 2.60, 0.88),
            water_hourly: [
                UnitSpring::from_response(0.0, 2.90, 0.88),
                UnitSpring::from_response(0.0, 2.90, 0.88),
                UnitSpring::from_response(0.0, 2.90, 0.88),
                UnitSpring::from_response(0.0, 2.90, 0.88),
                UnitSpring::from_response(0.0, 2.90, 0.88),
                UnitSpring::from_response(0.0, 2.90, 0.88),
                UnitSpring::from_response(0.0, 2.90, 0.88),
                UnitSpring::from_response(0.0, 2.90, 0.88),
            ],
            shelf_select_mode: UnitSpring::from_response(0.0, 3.60, 0.90),
            album_art_fade: UnitSpring::from_response(1.0, 3.20, 0.94),
            clock_visible: UnitSpring::from_response(
                if cfg.show_clock { 1.0 } else { 0.0 },
                3.80,
                0.94,
            ),
            notch_reveal: UnitSpring::from_response(1.0, 2.35, 0.90),
            mascot_idle_presence: UnitSpring::from_response(0.0, 1.12, 0.96),
            mascot_idle_sleep: UnitSpring::from_response(0.0, 0.72, 0.98),
            eave_was_asleep: false,
            mascot_wake_elapsed: None,
            eave_sleep_z_active: false,
            eave_sleep_z_elapsed: 0.0,
            eave_sleep_z_cooldown: 0.0,
            eave_aurora_phase: 0.0,
            eave_rng: {
                let ns = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.subsec_nanos())
                    .unwrap_or(0x5EED_CAFE);
                (ns ^ 0x9E37_79B9).max(1)
            },
            blink_double_pending: false,
            last_art_rev: 0,
            widget_was_active: false,
            mascot_look_x: UnitSpring::from_response(0.0, 2.4, 0.92),
            phase_secs: 0.0,
            blink_timer_secs: 5.5,
            blink_amount: 0.0,
        }
    }

    pub fn apply_state(
        &mut self,
        cfg: &Config,
        expanded: bool,
        hovered: bool,
        dock_hovered: bool,
        is_drag_over: bool,
    ) {
        let lyrics_active = self.lyrics_mode.target() > 0.5;
        let osd_active = self.osd_open.target() > 0.5;
        if self.ai_phase == crate::ai::AiPhase::Input {
            self.width.set_response(1.55, 0.68);
            self.height.set_response(1.60, 0.70);
            self.radius.set_response(1.60, 0.74);
            self.ear_radius.set_response(1.60, 0.74);
            self.width.set_target(596.0);
            self.height.set_target(66.0);
            self.radius.set_target(30.0);
            self.ear_radius.set_target(16.0);
            self.expand.set_response(2.75, 0.96);
            self.expand_bloom.set_response(2.75, 0.96);
            self.expand.set_target(0.0);
            self.expand_bloom.set_target(0.0);
        } else if self.ai_phase == crate::ai::AiPhase::Busy {
            self.width.set_response(1.50, 0.68);
            self.height.set_response(1.50, 0.68);
            self.radius.set_response(1.55, 0.74);
            self.ear_radius.set_response(1.55, 0.74);
            self.width.set_target(300.0);
            self.height.set_target(120.0);
            self.radius.set_target(38.0);
            self.ear_radius.set_target(16.0);
            self.expand.set_response(2.75, 0.96);
            self.expand_bloom.set_response(2.75, 0.96);
            self.expand.set_target(0.0);
            self.expand_bloom.set_target(0.0);
        } else if self.ai_phase == crate::ai::AiPhase::Done {
            self.width.set_response(1.55, 0.78);
            self.height.set_response(1.55, 0.78);
            self.radius.set_response(1.60, 0.80);
            self.ear_radius.set_response(1.60, 0.80);
            self.width.set_target(260.0);
            self.height.set_target(54.0);
            self.radius.set_target(24.0);
            self.ear_radius.set_target(14.0);
            self.expand.set_response(2.75, 0.96);
            self.expand_bloom.set_response(2.75, 0.96);
            self.expand.set_target(0.0);
            self.expand_bloom.set_target(0.0);
        } else if expanded {
            self.width.set_response(2.30, 0.66);
            self.height.set_response(2.35, 0.68);
            self.radius.set_response(2.50, 0.72);
            self.ear_radius.set_response(2.50, 0.72);
            self.expand.set_response(2.35, 0.70);
            self.expand_bloom.set_response(2.30, 0.64);
            self.dock_hover.set_response(2.75, 0.82);
            self.dock_wobble.set_response(2.55, 0.62);

            let (target_w, dock_extra) = if lyrics_active {
                (564.0, 0.0)
            } else {
                (cfg.expanded_width, if dock_hovered { 30.0 } else { 0.0 })
            };
            self.width.set_target(target_w);
            self.height.set_target(cfg.expanded_height + dock_extra);
            self.radius.set_target(32.0_f32.min(cfg.expanded_height * 0.45));
            self.ear_radius.set_target(22.0);
            self.expand.set_target(1.0);
            self.expand_bloom.set_target(1.0);
        } else if osd_active {
            self.width.set_response(2.35, 0.85);
            self.height.set_response(2.35, 0.85);
            self.radius.set_response(2.35, 0.85);
            self.ear_radius.set_response(2.35, 0.85);
            if self.osd_wide {
                self.width.set_target(388.0);
                self.height.set_target(64.0);
                self.radius.set_target(23.0);
                self.ear_radius.set_target(15.0);
            } else if self.osd_tall {
                self.width.set_target(312.0);
                self.height.set_target(56.0);
                self.radius.set_target(22.0);
                self.ear_radius.set_target(15.0);
            } else {
                self.width.set_target(276.0);
                self.height.set_target(34.0);
                self.radius.set_target(15.5);
                self.ear_radius.set_target(11.0);
            }
            self.expand.set_response(2.75, 0.96);
            self.expand_bloom.set_response(2.75, 0.96);
            self.expand.set_target(0.0);
            self.expand_bloom.set_target(0.0);
        } else if hovered {
            if lyrics_active {
                self.width.set_response(2.55, 0.80);
                self.height.set_response(3.10, 0.70);
                self.radius.set_response(3.10, 0.70);
                self.ear_radius.set_response(3.10, 0.70);
                self.width
                    .set_target((self.closed_lyric_width.target + 16.0).clamp(338.0, 558.0));
                self.height.set_target(37.5);
                self.radius.set_target(16.5);
                self.ear_radius.set_target(12.5);
            } else if cfg.minimal_mode {
                self.width.set_response(3.10, 0.70);
                self.height.set_response(3.10, 0.70);
                self.radius.set_response(3.10, 0.70);
                self.ear_radius.set_response(3.10, 0.70);
                self.width.set_target(126.0);
                self.height.set_target(27.0);
                self.radius.set_target(13.0);
                self.ear_radius.set_target(10.5);
            } else {
                self.width.set_response(3.10, 0.70);
                self.height.set_response(3.10, 0.70);
                self.radius.set_response(3.10, 0.70);
                self.ear_radius.set_response(3.10, 0.70);
                self.width.set_target(cfg.notch_width + 20.0);
                self.height.set_target(cfg.notch_height + 4.5);
                self.radius.set_target(15.5);
                self.ear_radius.set_target(12.5);
            }
            self.expand.set_response(2.75, 0.96);
            self.expand_bloom.set_response(2.75, 0.96);
            self.expand.set_target(0.0);
            self.expand_bloom.set_target(0.0);
        } else {
            if lyrics_active {
                self.width.set_response(2.55, 0.76);
                self.height.set_response(3.10, 0.70);
                self.radius.set_response(3.10, 0.70);
                self.ear_radius.set_response(3.10, 0.70);
                self.width
                    .set_target(self.closed_lyric_width.target.clamp(322.0, 542.0));
                self.height.set_target(34.0);
                self.radius.set_target(15.5);
                self.ear_radius.set_target(11.0);
            } else {
                let (freq, damp) = if self.osd_open.value() > 0.01 {
                    (2.15, 0.88)
                } else {
                    (3.10, 0.70)
                };
                self.width.set_response(freq, damp);
                self.height.set_response(freq, damp);
                self.radius.set_response(freq, damp);
                self.ear_radius.set_response(freq, damp);
                if cfg.minimal_mode {
                    self.width.set_target(108.0);
                    self.height.set_target(24.0);
                    self.radius.set_target(11.5);
                    self.ear_radius.set_target(8.5);
                } else {
                    self.width.set_target(cfg.notch_width);
                    self.height.set_target(cfg.notch_height);
                    self.radius.set_target(14.0_f32.min(cfg.notch_height * 0.48));
                    self.ear_radius.set_target(10.0);
                }
            }
            self.expand.set_response(2.75, 0.96);
            self.expand_bloom.set_response(2.75, 0.96);
            self.dock_hover.set_response(2.85, 0.94);
            self.expand.set_target(0.0);
            self.expand_bloom.set_target(0.0);
        }

        self.hover.set_target(if hovered { 1.0 } else { 0.0 });
        let dock_active = expanded && dock_hovered && !lyrics_active;
        self.dock_hover.set_target(if dock_active { 1.0 } else { 0.0 });
        if !dock_active {
            self.dock_wobble.set_response(2.95, 0.94);
        }
        self.dock_wobble.set_target(if dock_active { 1.0 } else { 0.0 });
        self.drag_over.set_target(if is_drag_over { 1.0 } else { 0.0 });
    }

    pub fn slot_hover(&self, slot: usize) -> f32 {
        self.hover_slots.get(slot).map(|s| s.value()).unwrap_or(0.0)
    }

    pub fn slot_press(&self, slot: usize) -> f32 {
        self.press_slots.get(slot).map(|s| s.value()).unwrap_or(0.0)
    }

    pub fn slot_scale(&self, slot: usize, hover_amp: f32, press_amp: f32) -> f32 {
        let h = self.slot_hover(slot);
        let p = self.slot_press(slot);
        (1.0 + hover_amp * h - press_amp * p).clamp(0.72, 1.25)
    }

    pub fn set_active_hover_slot(&mut self, slot: Option<usize>) {
        for (i, s) in self.hover_slots.iter_mut().enumerate() {
            s.set_target(if slot == Some(i) { 1.0 } else { 0.0 });
        }
    }

    pub fn set_active_press_slot(&mut self, slot: Option<usize>) {
        for (i, s) in self.press_slots.iter_mut().enumerate() {
            s.set_target(if slot == Some(i) { 1.0 } else { 0.0 });
        }
    }

    pub fn set_has_notification(&mut self, active: bool) {
        self.bell_visible.set_target(if active { 1.0 } else { 0.0 });
    }

    pub fn set_active_page_index(&mut self, idx: usize) {
        for (i, dot) in self.dot_active.iter_mut().enumerate() {
            dot.set_target(if i == idx { 1.0 } else { 0.0 });
        }
    }

    pub fn set_shelf_clipboard_tab(&mut self, is_clipboard: bool) {
        self.shelf_tab_clipboard
            .set_target(if is_clipboard { 1.0 } else { 0.0 });
    }

    pub fn set_water_reminder(&mut self, enabled: bool) {
        self.water_switch_on
            .set_target(if enabled { 1.0 } else { 0.0 });
    }

    pub fn set_water_gauge(&mut self, show_gauge: bool) {
        self.water_view_gauge
            .set_target(if show_gauge { 1.0 } else { 0.0 });
    }

    pub fn set_dark_bg_border(&mut self, active: bool) {
        self.dark_bg_border
            .set_target(if active { 1.0 } else { 0.0 });
    }

    pub fn dark_bg_border(&self) -> f32 {
        self.dark_bg_border.value().clamp(0.0, 1.0)
    }

    pub fn trigger_page_switch(&mut self, direction: f32) {
        self.page_dir = if direction >= 0.0 { 1.0 } else { -1.0 };
        self.page_transition.snap_to(0.0);
        self.page_transition.set_target(1.0);
    }

    pub fn set_ai_btn_hovered(&mut self, idx: Option<usize>) {
        for (i, sp) in self.ai_btn_hover.iter_mut().enumerate() {
            sp.set_target(if idx == Some(i) { 1.0 } else { 0.0 });
        }
    }

    pub fn set_dock_hovered_index(&mut self, idx: Option<usize>) {
        for (i, item) in self.dock_item_hover.iter_mut().enumerate() {
            item.set_target(if idx == Some(i) { 1.0 } else { 0.0 });
        }
    }

    pub fn set_dock_pressed_index(&mut self, idx: Option<usize>) {
        for (i, item) in self.dock_item_press.iter_mut().enumerate() {
            item.set_target(if idx == Some(i) { 1.0 } else { 0.0 });
        }
    }

    pub fn set_shelf_zone_hovered(&mut self, zone_hovered: bool) {
        self.shelf_hover.set_target(if zone_hovered { 1.0 } else { 0.0 });
    }

    pub fn trigger_shelf_gulp(&mut self) {
        self.shelf_gulp.set_response(3.20, 0.60);
        self.shelf_gulp.snap_to(1.0);
        self.shelf_gulp.set_target(0.0);
    }

    pub fn trigger_shelf_drop(&mut self, drop_x_dips: f32, card_slot: Option<usize>) {
        self.shelf_drop_x = drop_x_dips;
        self.shelf_drop_t = 0.0;
        if let Some(slot) = card_slot {
            if slot < self.card_drop_t.len() {
                self.card_drop_t[slot] = 0.0;
                self.trigger_cards_spawn_range(slot, 1);
            }
        }
        self.trigger_shelf_gulp();
    }

    pub fn trigger_cards_spawn_range(&mut self, start_idx: usize, count: usize) {
        let end = (start_idx.saturating_add(count)).min(self.card_spawn.len());
        for i in start_idx..end {
            self.card_spawn[i].value = 0.0;
            self.card_spawn[i].velocity = 0.0;
            self.card_spawn[i].target = 1.0;
        }
    }

    pub fn trigger_shelf_clear(&mut self, item_count: usize) {
        if item_count == 0 {
            self.shelf_clear_active = false;
            self.shelf_clear_elapsed = 0.0;
            self.shelf_clear_count = 0;
            self.shelf_clear_finished = true;
            return;
        }
        self.shelf_clear_active = true;
        self.shelf_clear_elapsed = 0.0;
        self.shelf_clear_count = item_count;
        self.shelf_clear_finished = false;
    }

    pub fn is_shelf_clearing(&self) -> bool {
        self.shelf_clear_active
    }

    pub fn take_shelf_clear_finished(&mut self) -> bool {
        let done = self.shelf_clear_finished;
        self.shelf_clear_finished = false;
        done
    }

    pub fn shelf_item_clear_progress(&self, idx: usize) -> Option<f32> {
        if !self.shelf_clear_active {
            return None;
        }
        let delay = idx as f32 * 0.075;
        Some(((self.shelf_clear_elapsed - delay) / 0.82).clamp(0.0, 1.0))
    }

    pub fn shelf_clear_overall_progress(&self) -> Option<f32> {
        if !self.shelf_clear_active {
            return None;
        }
        let total_dur = self.shelf_clear_count.saturating_sub(1).min(4) as f32 * 0.075 + 0.82;
        Some((self.shelf_clear_elapsed / total_dur).clamp(0.0, 1.0))
    }

    #[allow(dead_code)]
    pub fn sync_content_transitions(
        &mut self,
        shelf_show_dropzone: bool,
        clip_empty: bool,
        pomo_running: bool,
        todo_adding: bool,
        todo_editing: bool,
        todo_empty: bool,
    ) {
        self.shelf_empty_reveal
            .set_target(if shelf_show_dropzone { 1.0 } else { 0.0 });
        self.clipboard_empty_reveal
            .set_target(if clip_empty { 1.0 } else { 0.0 });
        self.pomo_running_mix
            .set_target(if pomo_running { 1.0 } else { 0.0 });
        self.todo_form_open
            .set_target(if todo_adding { 1.0 } else { 0.0 });
        self.todo_edit_mode
            .set_target(if todo_editing { 1.0 } else { 0.0 });
        self.todo_empty_reveal
            .set_target(if todo_empty { 1.0 } else { 0.0 });
    }

    #[allow(dead_code)]
    pub fn set_todo_list_expanded(&mut self, expanded: bool) {
        self.todo_list_expanded
            .set_target(if expanded { 1.0 } else { 0.0 });
    }

    pub fn effective_left_ring_mode(mode: u8, pomo_running: bool, water_taken_ml: u32) -> u8 {
        if mode == 3 {
            if pomo_running {
                4
            } else if water_taken_ml > 0 {
                1
            } else {
                2
            }
        } else {
            mode
        }
    }

    pub fn sync_ring_transitions(
        &mut self,
        effective_left_mode: u8,
        has_collapsed_art: bool,
        has_active_audio_vis: bool,
    ) {
        self.collapsed_art_mix
            .set_target(if has_collapsed_art { 1.0 } else { 0.0 });
        self.audio_vis_mix
            .set_target(if has_active_audio_vis { 1.0 } else { 0.0 });

        if effective_left_mode == 0 {
            self.left_ring_visible.set_target(0.0);
        } else {
            if self.left_ring_mode == 0 {
                self.left_ring_mode = effective_left_mode;
                self.left_ring_prev_mode = 0;
                self.left_ring_switch.snap_to(1.0);
            } else if self.left_ring_mode != effective_left_mode {
                if self.left_ring_visible.value() > 0.02 {
                    self.left_ring_prev_mode = self.left_ring_mode;
                    self.left_ring_mode = effective_left_mode;
                    self.left_ring_switch.snap_to(0.0);
                    self.left_ring_switch.set_target(1.0);
                } else {
                    self.left_ring_mode = effective_left_mode;
                    self.left_ring_prev_mode = 0;
                    self.left_ring_switch.snap_to(1.0);
                }
            }
            self.left_ring_visible.set_target(1.0);
        }
    }

    pub fn trigger_widget_battery_switch(&mut self) {
        self.widget_bat_switch.snap_to(0.0);
        self.widget_bat_switch.set_target(1.0);
    }

    pub fn trigger_widget_card_press(&mut self, is_left: bool) {
        let s = if is_left {
            &mut self.widget_left_press
        } else {
            &mut self.widget_right_press
        };
        s.snap_to(1.0);
        s.set_target(0.0);
    }

    pub fn trigger_widget_day_pop(&mut self) {
        self.widget_day_pop.snap_to(1.0);
        self.widget_day_pop.set_target(0.0);
        self.widget_day_switch.snap_to(0.0);
        self.widget_day_switch.set_target(1.0);
    }

    pub fn trigger_widget_water_splash(&mut self) {
        self.widget_water_splash.snap_to(1.0);
        self.widget_water_splash.set_target(0.0);
    }

    pub fn sync_water_hourly(&mut self, bars: [u32; 8]) {
        let max_ml = if bars.iter().copied().max().unwrap_or(0) > 400 {
            800.0_f32
        } else {
            400.0_f32
        };
        for (i, &ml) in bars.iter().enumerate() {
            self.water_hourly[i].set_target((ml as f32 / max_ml).clamp(0.0, 1.0));
        }
    }

    pub fn sync_shelf_select(&mut self, active: bool) {
        self.shelf_select_mode
            .set_target(if active { 1.0 } else { 0.0 });
    }

    pub fn sync_album_art_rev(&mut self, rev: u64) {
        if rev != 0 && rev != self.last_art_rev {
            if self.last_art_rev != 0 {
                self.album_art_fade.snap_to(0.0);
                self.album_art_fade.set_target(1.0);
            }
            self.last_art_rev = rev;
        }
    }

    pub fn sync_widget_bat_rgb(&mut self, rgb: (f32, f32, f32)) {
        self.widget_bat_rgb[0].set_target(rgb.0.clamp(0.0, 1.0));
        self.widget_bat_rgb[1].set_target(rgb.1.clamp(0.0, 1.0));
        self.widget_bat_rgb[2].set_target(rgb.2.clamp(0.0, 1.0));
    }

    pub fn set_brightness(&mut self, pct: f32) {
        self.brightness_pct.set_target(pct.clamp(0.0, 100.0));
    }

    pub fn set_mic_muted(&mut self, muted: bool) {
        self.mic_muted_mix.set_target(if muted { 1.0 } else { 0.0 });
    }

    pub fn snap_mic_muted(&mut self, muted: bool) {
        self.mic_muted_mix.snap_to(if muted { 1.0 } else { 0.0 });
    }

    pub fn sync_left_ring_todo_prog(&mut self, prog: f32) {
        self.left_ring_todo_prog.set_target(prog.clamp(0.0, 1.0));
    }

    pub fn sync_clock_visible(&mut self, show: bool) {
        self.clock_visible.set_target(if show { 1.0 } else { 0.0 });
    }

    pub fn set_notch_reveal(&mut self, visible: bool) {
        if visible {
            self.notch_reveal.set_response(1.35, 0.62);
            self.notch_reveal.set_target(1.0);
        } else {
            self.notch_reveal.set_response(1.90, 0.96);
            self.notch_reveal.set_target(0.0);
        }
    }

    pub fn trigger_startup_reveal(&mut self) {
        self.notch_reveal.set_response(1.35, 0.62);
        self.notch_reveal.snap_to(0.0);
        self.notch_reveal.set_target(1.0);
    }

    pub fn sync_mascot_idle(&mut self, visible: bool, asleep: bool, waking: bool) {
        let now_asleep = visible && asleep;
        if now_asleep && !self.eave_was_asleep {
            self.eave_sleep_z_active = true;
            self.eave_sleep_z_elapsed = 0.0;
            self.eave_sleep_z_cooldown = 0.0;
        } else if !now_asleep {
            self.eave_sleep_z_active = false;
            self.eave_sleep_z_cooldown = 0.0;
        }
        self.eave_was_asleep = now_asleep;
        if visible && waking && !now_asleep {
            if self.mascot_wake_elapsed.is_none() {
                self.mascot_wake_elapsed = Some(0.0);
            }
        } else {
            self.mascot_wake_elapsed = None;
        }
        self.mascot_idle_presence
            .set_target(if visible { 1.0 } else { 0.0 });
        self.mascot_idle_sleep
            .set_target(if now_asleep { 1.0 } else { 0.0 });
    }

    pub fn eave_is_waking(&self) -> bool {
        self.mascot_wake_elapsed.is_some()
    }

    pub fn eave_aurora_phase(&self) -> f32 {
        self.eave_aurora_phase
    }

    pub fn eave_sleep_z_elapsed(&self) -> Option<f32> {
        self.eave_sleep_z_active
            .then_some(self.eave_sleep_z_elapsed)
    }

    fn next_rand_unit(&mut self) -> f32 {
        let mut x = self.eave_rng.max(1);
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.eave_rng = x.max(1);
        (x & 0x00FF_FFFF) as f32 / 16_777_215.0
    }

    pub fn widget_bat_rgb(&self) -> (f32, f32, f32) {
        (
            self.widget_bat_rgb[0].value().clamp(0.0, 1.0),
            self.widget_bat_rgb[1].value().clamp(0.0, 1.0),
            self.widget_bat_rgb[2].value().clamp(0.0, 1.0),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn sync_widget_springs(
        &mut self,
        widgets_visible: bool,
        selected_day: usize,
        water_prog: f32,
        bat_prog: f32,
        focus_running: bool,
        focus_prog: f32,
        countdown_prog: f32,
        left_hovered: bool,
        right_hovered: bool,
    ) {
        self.widget_enter.set_target(1.0);
        self.widget_was_active = widgets_visible;

        self.widget_day_idx.set_target(selected_day.min(6) as f32);
        self.widget_water_fill.set_target(water_prog.clamp(0.0, 1.0));
        self.widget_bat_arc.set_target(bat_prog.clamp(0.0, 1.0));
        self.widget_focus_active
            .set_target(if focus_running { 1.0 } else { 0.0 });
        self.widget_focus_prog.set_target(focus_prog.clamp(0.0, 1.0));
        self.widget_countdown_prog
            .set_target(countdown_prog.clamp(0.0, 1.0));
        self.widget_left_hover
            .set_target(if left_hovered { 1.0 } else { 0.0 });
        self.widget_right_hover
            .set_target(if right_hovered { 1.0 } else { 0.0 });
    }

    #[allow(dead_code)]
    pub fn sync_lyrics_state(
        &mut self,
        lyrics_active: bool,
        closed_target_w: f32,
        lyric_hash: u64,
        song_rgb: (f32, f32, f32),
    ) {
        self.lyrics_mode
            .set_target(if lyrics_active { 1.0 } else { 0.0 });
        self.closed_lyric_width
            .set_target(closed_target_w.clamp(322.0, 542.0));
        self.song_color_r.set_target(song_rgb.0.clamp(0.0, 1.0));
        self.song_color_g.set_target(song_rgb.1.clamp(0.0, 1.0));
        self.song_color_b.set_target(song_rgb.2.clamp(0.0, 1.0));
        if lyric_hash != self.last_lyric_hash {
            if self.last_lyric_hash != 0 && lyrics_active {
                self.lyric_line_change.snap_to(0.0);
                self.lyric_line_change.set_target(1.0);
            }
            self.last_lyric_hash = lyric_hash;
        }
    }

    #[allow(dead_code)]
    pub fn set_volume(&mut self, volume_pct: f32, muted: bool) {
        let target = if muted {
            0.0
        } else {
            volume_pct.clamp(0.0, 100.0)
        };
        self.speaker_volume.set_target(target);
    }

    #[allow(dead_code)]
    pub fn set_muted(&mut self, muted: bool) {
        self.set_volume(100.0, muted);
    }

    #[allow(dead_code)]
    pub fn set_speaker_volume(&mut self, volume: f32) {
        self.speaker_volume.set_target(volume.clamp(0.0, 100.0));
    }

    #[allow(dead_code)]
    pub fn speaker_volume(&self) -> f32 {
        self.speaker_volume.value.clamp(0.0, 100.0)
    }

    #[allow(dead_code)]
    pub fn set_volume_hovered(&mut self, hovered: bool) {
        self.volume_slider_open.set_target(if hovered { 1.0 } else { 0.0 });
    }

    #[allow(dead_code)]
    pub fn volume_slider_open(&self) -> f32 {
        self.volume_slider_open.value().clamp(0.0, 1.0)
    }

    #[allow(dead_code)]
    pub fn dock_wobble(&self) -> f32 {
        self.dock_wobble.value().max(0.0)
    }

    #[allow(dead_code)]
    pub fn set_osd_active(&mut self, active: bool) {
        if active {
            self.osd_open.set_response(2.35, 0.85);
            self.osd_open.set_target(1.0);
        } else {
            self.osd_open.set_response(2.15, 0.88);
            self.osd_open.set_target(0.0);
        }
    }

    #[allow(dead_code)]
    pub fn step_magnetic_arrows(
        &mut self,
        pointer: Option<(f32, f32)>,
        left_anchor: (f32, f32),
        right_anchor: (f32, f32),
        dt: f32,
    ) -> bool {
        let al = self.mag_left.step(pointer, left_anchor, 1.0, dt);
        let ar = self.mag_right.step(pointer, right_anchor, -1.0, dt);
        self.mag_arrows_moving = al || ar;
        self.mag_arrows_moving
    }

    pub fn set_osd_tall(&mut self, tall: bool) {
        self.osd_tall = tall;
        if !tall {
            self.osd_wide = false;
        }
    }

    pub fn set_osd_wide(&mut self, wide: bool) {
        self.osd_wide = wide;
        if wide {
            self.osd_tall = true;
        }
    }

    pub fn set_ai_phase(&mut self, phase: crate::ai::AiPhase, card_visible: bool) {
        self.ai_phase = phase;
        self.ai_active
            .set_target(if phase != crate::ai::AiPhase::Idle { 1.0 } else { 0.0 });
        self.ai_input_open
            .set_target(if phase == crate::ai::AiPhase::Input { 1.0 } else { 0.0 });
        self.ai_wave_open.set_target(
            if matches!(phase, crate::ai::AiPhase::Busy | crate::ai::AiPhase::Done) {
                1.0
            } else {
                0.0
            },
        );
        self.ai_wave_energy
            .set_target(if phase == crate::ai::AiPhase::Busy { 1.0 } else { 0.0 });
        self.ai_card_open
            .set_target(if card_visible { 1.0 } else { 0.0 });
    }

    pub fn ai_wave_phase(&self) -> f32 {
        self.ai_wave_phase
    }

    pub fn kick_ai_shake(&mut self) {
        self.ai_shake.velocity = 165.0;
    }

    pub fn shift_shelf_hover_slots(&mut self, is_clipboard: bool, step: i32) {
        if step == 0 {
            return;
        }
        let ranges: &[(usize, usize)] = if is_clipboard {
            &[(29, 4), (43, 4)]
        } else {
            &[(19, 5), (24, 5)]
        };
        for &(base, len) in ranges {
            let mut next_h = [UnitSpring::from_response(0.0, 3.80, 0.82); 5];
            let mut next_p = [UnitSpring::from_response(0.0, 5.60, 0.78); 5];
            for i in 0..len {
                let src = i as i32 + step;
                if src >= 0 && (src as usize) < len {
                    next_h[i] = self.hover_slots[base + src as usize];
                    next_p[i] = self.press_slots[base + src as usize];
                }
            }
            for i in 0..len {
                self.hover_slots[base + i] = next_h[i];
                self.press_slots[base + i] = next_p[i];
            }
        }
    }

    pub fn trigger_bt_popup_fill(&mut self, target_ratio: f32) {
        self.bt_popup_arc.snap_to(0.0);
        self.bt_popup_arc.set_target(target_ratio.clamp(0.0, 1.0));
        self.bt_popup_delay = if target_ratio > 0.01 { 0.22 } else { 0.0 };
    }

    pub fn osd_open(&self) -> f32 {
        self.osd_open.value().clamp(0.0, 1.0)
    }

    pub fn kick_osd_vol_bounce(&mut self, dir: f32) {
        if self.osd_vol_bounce_cd > 0.0 {
            return;
        }
        let sign = if dir >= 0.0 { 1.0 } else { -1.0 };
        self.osd_vol_bounce.target = 0.0;
        self.osd_vol_bounce.velocity = sign * 245.0;
        self.osd_vol_bounce_cd = 0.19;
    }

    pub fn osd_vol_bounce(&self) -> f32 {
        self.osd_vol_bounce.value.clamp(-14.0, 14.0)
    }

    #[allow(dead_code)]
    pub fn set_caps_on(&mut self, on: bool) {
        self.caps_on_mix.set_target(if on { 1.0 } else { 0.0 });
    }

    #[allow(dead_code)]
    pub fn snap_caps_on(&mut self, on: bool) {
        self.caps_on_mix.snap_to(if on { 1.0 } else { 0.0 });
    }

    #[allow(dead_code)]
    pub fn caps_on_mix(&self) -> f32 {
        self.caps_on_mix.value().clamp(0.0, 1.0)
    }

    #[allow(dead_code)]
    pub fn set_lyric_index(&mut self, idx: Option<usize>) {
        if idx != self.last_lyric_index {
            if let (Some(prev), Some(curr)) = (self.last_lyric_index, idx) {
                self.lyric_scroll_dir = if curr < prev { -1.0 } else { 1.0 };
            } else {
                self.lyric_scroll_dir = 1.0;
            }
            self.last_lyric_index = idx;
        }
    }

    pub fn set_lyric_started(&mut self, started: bool) {
        self.lyric_started_mix.set_target(if started { 1.0 } else { 0.0 });
    }

    #[allow(dead_code)]
    pub fn lyric_started_mix(&self) -> f32 {
        self.lyric_started_mix.value().clamp(0.0, 1.0)
    }

    #[allow(dead_code)]
    pub fn visualizer_levels(&self) -> [f32; 5] {
        [
            self.visualizer[0].value().clamp(0.15, 1.0),
            self.visualizer[1].value().clamp(0.15, 1.0),
            self.visualizer[2].value().clamp(0.15, 1.0),
            self.visualizer[3].value().clamp(0.15, 1.0),
            self.visualizer[4].value().clamp(0.15, 1.0),
        ]
    }

    #[allow(dead_code)]
    pub fn play_mix(&self) -> f32 {
        self.play_mix.value().clamp(0.0, 1.0)
    }

    pub fn snap_collapsed(&mut self, cfg: &Config) {
        self.width.snap_to(cfg.notch_width);
        self.height.snap_to(cfg.notch_height);
        self.radius
            .snap_to(14.0_f32.min(cfg.notch_height * 0.48));
        self.ear_radius.snap_to(10.0);
        self.expand.snap_to(0.0);
        self.expand_bloom.snap_to(0.0);
        self.hover.snap_to(0.0);
        self.dock_hover.snap_to(0.0);
        self.dock_wobble.snap_to(0.0);
        self.volume_slider_open.snap_to(0.0);
        self.page_transition.snap_to(1.0);
        self.drag_over.snap_to(0.0);
        self.shelf_hover.snap_to(0.0);
        self.shelf_gulp.snap_to(0.0);
        self.shelf_drop_t = -1.0;
        self.card_drop_t = [-1.0; 5];
        for c in &mut self.card_spawn {
            c.snap_to(1.0);
        }
        self.shelf_clear_active = false;
        self.shelf_clear_elapsed = 0.0;
        self.shelf_clear_count = 0;
        self.shelf_clear_finished = false;
        self.lyrics_mode.snap_to(0.0);
        self.closed_lyric_width.snap_to(322.0);
        self.lyric_line_change.snap_to(1.0);
        self.lyric_started_mix.snap_to(1.0);
        self.osd_open.snap_to(0.0);
        self.osd_vol_bounce.snap_to(0.0);
        self.osd_vol_bounce_cd = 0.0;
        self.mag_left = MagneticArrowState::default();
        self.mag_right = MagneticArrowState::default();
        self.last_lyric_hash = 0;
        self.last_lyric_index = None;
        self.lyric_scroll_dir = 1.0;
        for item in &mut self.dock_item_hover {
            item.snap_to(0.0);
        }
        for item in &mut self.dock_item_press {
            item.snap_to(0.0);
        }
        for slot in &mut self.hover_slots {
            slot.snap_to(0.0);
        }
        for slot in &mut self.press_slots {
            slot.snap_to(0.0);
        }
    }

    pub fn step(&mut self, dt: f32, is_playing: bool, show_mascot: bool) {
        let dt = dt.clamp(0.0, 0.05);
        self.width.step(dt);
        self.height.step(dt);
        self.radius.step(dt);
        self.ear_radius.step(dt);
        self.expand.step(dt);
        self.expand_bloom.step_unit(dt);
        self.hover.step(dt);
        self.dock_hover.step(dt);
        self.dock_wobble.step(dt);
        self.page_transition.step(dt);
        self.drag_over.step(dt);
        self.shelf_hover.step(dt);
        self.shelf_gulp.step_unit(dt);
        self.bell_visible.step(dt);
        self.shelf_tab_clipboard.step(dt);
        self.water_switch_on.step(dt);
        self.water_view_gauge.step(dt);
        self.dark_bg_border.step(dt);
        self.shelf_empty_reveal.step(dt);
        self.clipboard_empty_reveal.step(dt);
        self.pomo_running_mix.step(dt);
        self.todo_form_open.step(dt);
        self.todo_edit_mode.step(dt);
        self.todo_empty_reveal.step(dt);
        self.todo_list_expanded.step(dt);
        self.lyrics_mode.step(dt);
        self.closed_lyric_width.step(dt);
        self.lyric_line_change.step(dt);
        self.lyric_started_mix.step(dt);
        self.osd_open.step(dt);
        self.ai_active.step(dt);
        self.ai_input_open.step(dt);
        self.ai_wave_open.step(dt);
        self.ai_wave_energy.step(dt);
        if self.ai_wave_open.value() > 0.01 {
            let speed = self.ai_wave_energy.value().clamp(0.0, 1.0);
            if speed > 0.002 {
                self.ai_wave_phase = (self.ai_wave_phase + dt * speed) % 10_000.0;
            }
        }
        self.ai_card_open.step(dt);
        self.ai_card_h.step(dt);
        self.ai_content_h.step(dt);
        self.ai_scroll.step(dt);
        self.ai_followup_open.step(dt);
        self.ai_scrollbar_vis.step(dt);
        self.ai_copy_anim.step(dt);
        for sp in &mut self.ai_btn_hover {
            sp.step(dt);
        }
        self.ai_shake.step(dt);
        if self.bt_popup_delay > 0.0 {
            self.bt_popup_delay = (self.bt_popup_delay - dt).max(0.0);
        } else {
            self.bt_popup_arc.step(dt);
        }
        self.osd_vol_bounce.step(dt);
        if self.osd_vol_bounce_cd > 0.0 {
            self.osd_vol_bounce_cd = (self.osd_vol_bounce_cd - dt).max(0.0);
        }
        self.caps_on_mix.step(dt);
        self.brightness_pct.step(dt);
        self.mic_muted_mix.step(dt);
        self.song_color_r.step(dt);
        self.song_color_g.step(dt);
        self.song_color_b.step(dt);
        self.speaker_volume.step(dt);
        self.volume_slider_open.step(dt);
        self.play_mix.set_target(if is_playing { 1.0 } else { 0.0 });
        self.play_mix.step(dt);
        self.collapsed_art_mix.step(dt);
        self.audio_vis_mix.step(dt);
        self.left_ring_visible.step(dt);
        self.left_ring_switch.step(dt);
        self.left_ring_todo_prog.step(dt);
        self.widget_day_idx.step(dt);
        self.widget_day_pop.step(dt);
        self.widget_day_switch.step(dt);
        self.widget_left_hover.step(dt);
        self.widget_right_hover.step(dt);
        self.widget_left_press.step(dt);
        self.widget_right_press.step(dt);
        self.widget_enter.step(dt);
        self.widget_water_fill.step(dt);
        self.widget_water_splash.step(dt);
        self.widget_bat_arc.step(dt);
        self.widget_bat_switch.step(dt);
        for c in &mut self.widget_bat_rgb {
            c.step(dt);
        }
        self.widget_focus_active.step(dt);
        self.widget_focus_prog.step(dt);
        self.widget_countdown_prog.step(dt);
        for b in &mut self.water_hourly {
            b.step(dt);
        }
        self.shelf_select_mode.step(dt);
        self.album_art_fade.step(dt);
        self.clock_visible.step(dt);
        self.notch_reveal.step(dt);
        self.mascot_idle_presence.step(dt);
        let sleep_dt = if self.mascot_idle_sleep.target() < 0.5 && self.mascot_idle_sleep.value() > 0.01 {
            dt * 2.3
        } else {
            dt
        };
        self.mascot_idle_sleep.step(sleep_dt);
        if self.mascot_idle_presence.value() > 0.01 {
            let aurora_speed = 0.65 * (1.0 - 0.55 * self.mascot_idle_sleep.value().clamp(0.0, 1.0));
            self.eave_aurora_phase = (self.eave_aurora_phase + dt * aurora_speed) % 10_000.0;
        }
        if let Some(wt) = &mut self.mascot_wake_elapsed {
            *wt += dt;
        }
        self.cursor_x.step(dt);
        self.cursor_y.step(dt);

        for item in &mut self.dock_item_hover {
            item.step(dt);
        }
        for item in &mut self.dock_item_press {
            item.step(dt);
        }
        for slot in &mut self.hover_slots {
            slot.step(dt);
        }
        for slot in &mut self.press_slots {
            slot.step(dt);
        }
        for dot in &mut self.dot_active {
            dot.step(dt);
        }
        for spawn in &mut self.card_spawn {
            spawn.step_unit(dt);
        }

        if self.shelf_clear_active {
            self.shelf_clear_elapsed += dt;
            let total_dur = self.shelf_clear_count.saturating_sub(1).min(4) as f32 * 0.075 + 0.82;
            if self.shelf_clear_elapsed >= total_dur {
                self.shelf_clear_active = false;
                self.shelf_clear_finished = true;
            }
        }

        if self.shelf_drop_t >= 0.0 {
            self.shelf_drop_t += dt * 0.68;
            if self.shelf_drop_t >= 1.50 {
                self.shelf_drop_t = -1.0;
            }
        }
        for ct in &mut self.card_drop_t {
            if *ct >= 0.0 {
                *ct += dt * 1.25;
                if *ct >= 1.30 {
                    *ct = -1.0;
                }
            }
        }

        self.phase_secs = (self.phase_secs + dt) % 3600.0;

        if is_playing {
            let t = self.phase_secs;
            let wave = |f1: f32, p1: f32, f2: f32, p2: f32| -> f32 {
                0.62 * (0.5 + 0.5 * (t * f1 + p1).sin())
                    + 0.38 * (0.5 + 0.5 * (t * f2 + p2).sin())
            };
            let targets = [
                0.20 + 0.38 * wave(6.4, 0.0, 11.2, 0.7),
                0.28 + 0.54 * wave(7.8, 1.1, 13.4, 2.3),
                0.36 + 0.64 * wave(7.1, 2.2, 12.1, 0.4),
                0.26 + 0.52 * wave(8.3, 3.4, 14.0, 1.5),
                0.20 + 0.38 * wave(6.9, 4.5, 11.7, 2.9),
            ];
            for (bar, target) in self.visualizer.iter_mut().zip(targets) {
                bar.set_target(target);
                bar.step(dt);
            }
        } else {
            const REST_BELL: [f32; 5] = [0.24, 0.42, 0.62, 0.40, 0.24];
            for (bar, rest) in self.visualizer.iter_mut().zip(REST_BELL) {
                bar.set_target(rest);
                bar.step(dt);
            }
        }

        if self.eave_sleep_z_active {
            self.eave_sleep_z_elapsed += dt;
            if self.eave_sleep_z_elapsed >= 3.0 {
                self.eave_sleep_z_active = false;
                self.eave_sleep_z_cooldown = 8.0 + self.next_rand_unit() * 8.0;
            }
        } else if self.eave_was_asleep {
            self.eave_sleep_z_cooldown -= dt;
            if self.eave_sleep_z_cooldown <= 0.0 {
                self.eave_sleep_z_active = true;
                self.eave_sleep_z_elapsed = 0.0;
            }
        }

        if show_mascot || self.mascot_idle_presence.value() > 0.01 {
            let look_target = if let Some(wt) = self.mascot_wake_elapsed {
                if wt < 0.50 {
                    0.0
                } else if wt < 1.75 {
                    -1.0
                } else if wt < 3.05 {
                    1.0
                } else if wt < 4.00 {
                    -0.38
                } else {
                    0.0
                }
            } else {
                ((self.phase_secs * 0.55).sin() * 0.42 + (self.phase_secs * 0.23).cos() * 0.22)
                    .clamp(-0.75, 0.75)
            };
            self.mascot_look_x.set_target(look_target);
            self.mascot_look_x.step(dt);

            self.blink_timer_secs -= dt;
            if self.blink_timer_secs <= 0.0 {
                if self.blink_double_pending {
                    self.blink_double_pending = false;
                    self.blink_timer_secs = 0.24;
                } else {
                    let r_wait = self.next_rand_unit();
                    let r_dbl = self.next_rand_unit();
                    self.blink_timer_secs = 4.8 + r_wait * 6.2;
                    self.blink_double_pending = r_dbl < 0.22;
                }
            }
            self.blink_amount = if self.blink_timer_secs < 0.14 {
                let p = (self.blink_timer_secs / 0.14).clamp(0.0, 1.0);
                (p * std::f32::consts::PI).sin().clamp(0.0, 1.0)
            } else {
                0.0
            };
        } else {
            self.blink_amount = 0.0;
        }
    }

    pub fn phase_secs(&self) -> f32 {
        self.phase_secs
    }

    pub fn mascot_blink(&self) -> f32 {
        self.blink_amount
    }

    pub fn is_idle_settled(&self, is_playing: bool) -> bool {
        if is_playing {
            return false;
        }
        self.is_interactive_settled() && self.visualizer.iter().all(|b| b.is_settled())
    }

    pub fn is_interactive_settled(&self) -> bool {
        if self.shelf_hover.value() > 0.01
            || self.drag_over.value() > 0.01
            || self.shelf_drop_t >= 0.0
            || self.shelf_clear_active
            || self.mag_arrows_moving
            || self.card_drop_t.iter().any(|&t| t >= 0.0)
        {
            return false;
        }
        self.width.is_settled()
            && self.height.is_settled()
            && self.radius.is_settled()
            && self.ear_radius.is_settled()
            && self.expand.is_settled()
            && self.expand_bloom.is_unit_settled()
            && self.hover.is_settled()
            && self.dock_hover.is_settled()
            && self.dock_wobble.is_settled()
            && self.page_transition.is_settled()
            && self.drag_over.is_settled()
            && self.shelf_hover.is_settled()
            && self.shelf_gulp.is_unit_settled()
            && self.bell_visible.is_settled()
            && self.shelf_tab_clipboard.is_settled()
            && self.water_switch_on.is_settled()
            && self.water_view_gauge.is_settled()
            && self.dark_bg_border.is_settled()
            && self.shelf_empty_reveal.is_settled()
            && self.clipboard_empty_reveal.is_settled()
            && self.pomo_running_mix.is_settled()
            && self.todo_form_open.is_settled()
            && self.todo_edit_mode.is_settled()
            && self.todo_empty_reveal.is_settled()
            && self.todo_list_expanded.is_settled()
            && self.lyrics_mode.is_settled()
            && self.closed_lyric_width.is_settled()
            && self.lyric_line_change.is_settled()
            && self.lyric_started_mix.is_settled()
            && self.osd_open.is_settled()
            && self.ai_active.is_settled()
            && self.ai_input_open.is_settled()
            && self.ai_wave_open.is_settled()
            && self.ai_wave_energy.is_settled()
            && self.ai_card_open.is_settled()
            && self.ai_card_h.is_settled()
            && self.ai_content_h.is_settled()
            && self.ai_scroll.is_settled()
            && self.ai_followup_open.is_settled()
            && self.ai_scrollbar_vis.is_settled()
            && self.ai_copy_anim.is_settled()
            && self.ai_btn_hover.iter().all(|s| s.is_settled())
            && self.ai_shake.is_settled()
            && self.bt_popup_delay <= 0.0
            && self.bt_popup_arc.is_settled()
            && self.osd_vol_bounce.is_settled()
            && self.caps_on_mix.is_settled()
            && self.brightness_pct.is_settled()
            && self.mic_muted_mix.is_settled()
            && self.song_color_r.is_settled()
            && self.song_color_g.is_settled()
            && self.song_color_b.is_settled()
            && self.speaker_volume.is_settled()
            && self.volume_slider_open.is_settled()
            && self.play_mix.is_settled()
            && self.collapsed_art_mix.is_settled()
            && self.audio_vis_mix.is_settled()
            && self.left_ring_visible.is_settled()
            && self.left_ring_switch.is_settled()
            && self.left_ring_todo_prog.is_settled()
            && self.widget_day_idx.is_settled()
            && self.widget_day_pop.is_settled()
            && self.widget_day_switch.is_settled()
            && self.widget_left_hover.is_settled()
            && self.widget_right_hover.is_settled()
            && self.widget_left_press.is_settled()
            && self.widget_right_press.is_settled()
            && self.widget_enter.is_settled()
            && self.widget_water_fill.is_settled()
            && self.widget_water_splash.is_settled()
            && self.widget_bat_arc.is_settled()
            && self.widget_bat_switch.is_settled()
            && self.widget_bat_rgb.iter().all(|c| c.is_settled())
            && self.widget_focus_active.is_settled()
            && self.widget_focus_prog.is_settled()
            && self.widget_countdown_prog.is_settled()
            && self.water_hourly.iter().all(|b| b.is_settled())
            && self.shelf_select_mode.is_settled()
            && self.album_art_fade.is_settled()
            && self.clock_visible.is_settled()
            && self.notch_reveal.is_settled()
            && self.mascot_idle_presence.is_settled()
            && self.mascot_idle_sleep.is_settled()
            && self.cursor_x.is_settled()
            && self.cursor_y.is_settled()
            && self.dock_item_hover.iter().all(|i| i.is_settled())
            && self.dock_item_press.iter().all(|i| i.is_settled())
            && self.hover_slots.iter().all(|s| s.is_settled())
            && self.press_slots.iter().all(|s| s.is_settled())
            && self.dot_active.iter().all(|d| d.is_settled())
            && self.card_spawn.iter().all(|c| c.is_unit_settled())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spring_converges_and_settles() {
        let mut s = Spring::from_response(166.0, 1.95, 0.58);
        s.set_target(438.0);
        assert!(!s.is_settled());

        for _ in 0..600 {
            s.step(1.0 / 120.0);
        }

        assert!(s.is_settled(), "expected spring to settle, val={}, vel={}", s.value, s.velocity);
        assert!((s.value - 438.0).abs() < 0.05);
    }

    #[test]
    fn spring_survives_large_dt_spike() {
        let mut s = Spring::from_response(30.0, 2.35, 0.96);
        s.set_target(172.0);
        for _ in 0..60 {
            s.step(0.5);
        }
        assert!(s.value.is_finite());
        assert!(s.is_settled());
        assert!((s.value - 172.0).abs() < 0.05);
    }

    #[test]
    fn notch_springs_idle_settle_only_when_paused() {
        let cfg = Config::default();
        let mut rig = NotchSprings::new(&cfg);
        assert!(rig.is_idle_settled(false));
        assert!(!rig.is_idle_settled(true));

        rig.apply_state(&cfg, true, true, true, false);
        assert!(!rig.is_idle_settled(false));

        let mut max_bloom = 0.0_f32;
        for _ in 0..600 {
            rig.step(1.0 / 120.0, false, false);
            max_bloom = max_bloom.max(rig.expand_bloom.value);
        }
        assert!(max_bloom > 1.02, "expected underdamped bloom overshoot > 1.02, got {max_bloom}");
        assert!(rig.is_idle_settled(false));
        assert!((rig.width.value - cfg.expanded_width).abs() < 0.05);
        assert!((rig.height.value - (cfg.expanded_height + 30.0)).abs() < 0.05);
        assert!((rig.dock_hover.value() - 1.0).abs() < 0.01);
    }

    #[test]
    fn two_stage_hover_swell_dwell_and_no_bounce_shrink() {
        let cfg = Config::default();
        let mut sm = HoverStateMachine::new();
        sm.notify_move(true);
        assert!(sm.hovered && !sm.expanded);
        assert!(sm.is_waiting_dwell());

        sm.tick(0.20, true, false);
        assert!(sm.hovered && !sm.expanded);

        sm.tick(0.12, true, false);
        assert!(sm.expanded);

        sm.notify_leave();
        assert!(sm.is_waiting_leave_grace());
        sm.tick(0.08, true, false);
        assert!(sm.expanded);
        sm.tick(0.08, true, false);
        assert!(!sm.hovered && !sm.expanded);

        let mut rig = NotchSprings::new(&cfg);
        rig.apply_state(&cfg, true, true, false, false);
        for _ in 0..600 {
            rig.step(1.0 / 120.0, false, false);
        }
        rig.apply_state(&cfg, false, false, false, false);
        let mut min_w = f32::INFINITY;
        for _ in 0..600 {
            rig.step(1.0 / 120.0, false, false);
            min_w = min_w.min(rig.width.value);
        }
        assert!(
            min_w < cfg.notch_width - 1.0,
            "expected subtle closing wobble below target: {min_w}"
        );
        assert!((rig.width.value - cfg.notch_width).abs() < 0.25);
    }

    #[test]
    fn page_switch_and_shelf_drop_settle() {
        let cfg = Config::default();
        let mut rig = NotchSprings::new(&cfg);
        rig.trigger_page_switch(1.0);
        rig.set_active_hover_slot(Some(3));
        rig.set_active_press_slot(Some(3));
        rig.set_dock_pressed_index(Some(2));
        rig.set_has_notification(true);
        rig.set_active_page_index(2);
        rig.set_shelf_clipboard_tab(true);
        rig.set_water_reminder(true);
        rig.trigger_shelf_drop(260.0, Some(1));
        assert!(!rig.is_idle_settled(false));

        for _ in 0..600 {
            rig.step(1.0 / 120.0, false, false);
        }
        assert!(rig.is_idle_settled(false));
        assert!((rig.page_transition.value() - 1.0).abs() < 0.01);
        assert!((rig.slot_hover(3) - 1.0).abs() < 0.01);
        assert!((rig.slot_press(3) - 1.0).abs() < 0.01);
        assert!((rig.slot_scale(3, 0.09, 0.15) - 0.94).abs() < 0.02);
        assert!((rig.bell_visible.value() - 1.0).abs() < 0.01);
        assert!((rig.dot_active[2].value() - 1.0).abs() < 0.01);
        assert!((rig.shelf_tab_clipboard.value() - 1.0).abs() < 0.01);
        assert!((rig.water_switch_on.value() - 1.0).abs() < 0.01);
        assert!(rig.shelf_drop_t < 0.0);
        assert!(rig.card_drop_t[1] < 0.0);
        assert!((rig.card_spawn[1].value - 1.0).abs() < 0.01);
    }

    #[test]
    fn shelf_card_spawn_and_sequential_suction_clear() {
        let cfg = Config::default();
        let mut rig = NotchSprings::new(&cfg);
        rig.trigger_cards_spawn_range(0, 3);
        assert_eq!(rig.card_spawn[0].value, 0.0);
        assert!(!rig.is_idle_settled(false));

        let mut max_spawn = 0.0_f32;
        for _ in 0..480 {
            rig.step(1.0 / 120.0, false, false);
            max_spawn = max_spawn.max(rig.card_spawn[0].value);
        }
        assert!(max_spawn > 1.05, "expected top-down wobble overshoot > 1.05, got {max_spawn}");
        assert!(rig.is_idle_settled(false));

        rig.trigger_shelf_clear(3);
        assert!(rig.is_shelf_clearing());
        assert!(!rig.take_shelf_clear_finished());
        assert_eq!(rig.shelf_clear_overall_progress(), Some(0.0));

        for _ in 0..10 {
            rig.step(1.0 / 120.0, false, false);
        }
        let u0 = rig.shelf_item_clear_progress(0).expect("active");
        let u2 = rig.shelf_item_clear_progress(2).expect("active");
        let ov = rig.shelf_clear_overall_progress().expect("overall");
        assert!(u0 > u2, "expected staggered delay (u0={u0} > u2={u2})");
        assert!(ov > 0.0 && ov < 1.0);

        for _ in 0..180 {
            rig.step(1.0 / 120.0, false, false);
        }
        assert!(!rig.is_shelf_clearing());
        assert!(rig.shelf_clear_overall_progress().is_none());
        assert!(rig.take_shelf_clear_finished());
        assert!(!rig.take_shelf_clear_finished());

        rig.sync_content_transitions(false, true, true, true, true, false);
        assert!(!rig.is_idle_settled(false));
        for _ in 0..480 {
            rig.step(1.0 / 120.0, false, false);
        }
        assert!((rig.shelf_empty_reveal.value() - 0.0).abs() < 0.01);
        assert!((rig.clipboard_empty_reveal.value() - 1.0).abs() < 0.01);
        assert!((rig.pomo_running_mix.value() - 1.0).abs() < 0.01);
        assert!((rig.todo_form_open.value() - 1.0).abs() < 0.01);
        assert!((rig.todo_edit_mode.value() - 1.0).abs() < 0.01);
        assert!((rig.todo_empty_reveal.value() - 0.0).abs() < 0.01);
        assert!(rig.is_idle_settled(false));
    }

    #[test]
    fn lyrics_mode_closed_width_and_expanded_564_split() {
        let cfg = Config::default();
        let mut rig = NotchSprings::new(&cfg);
        rig.sync_lyrics_state(true, 260.0, 1001, (0.65, 0.32, 0.18));
        rig.apply_state(&cfg, false, false, false, false);
        for _ in 0..480 {
            rig.step(1.0 / 120.0, false, false);
        }
        assert!((rig.lyrics_mode.value() - 1.0).abs() < 0.01);
        assert!((rig.width.value - 322.0).abs() < 0.5);
        assert!((rig.height.value - 34.0).abs() < 0.2);

        rig.sync_lyrics_state(true, 410.0, 1002, (0.65, 0.32, 0.18));
        assert_eq!(rig.lyric_line_change.value(), 0.0);
        rig.apply_state(&cfg, false, false, false, false);
        for _ in 0..480 {
            rig.step(1.0 / 120.0, false, false);
        }
        assert!((rig.width.value - 410.0).abs() < 0.5);

        rig.apply_state(&cfg, true, true, true, false);
        for _ in 0..480 {
            rig.step(1.0 / 120.0, false, false);
        }
        assert!((rig.width.value - 564.0).abs() < 0.5);
        assert!((rig.height.value - cfg.expanded_height).abs() < 0.2);
        assert!((rig.dock_hover.value() - 0.0).abs() < 0.01);
        assert!(rig.is_idle_settled(false));
    }

    #[test]
    fn visualizer_five_bars_bell_and_speaker_volume_mute_smooth() {
        let cfg = Config::default();
        let mut rig = NotchSprings::new(&cfg);

        let rest = rig.visualizer_levels();
        assert_eq!(rest.len(), 5);
        assert!(rest[2] > rest[1] && rest[1] > rest[0]);
        assert!((rest[0] - rest[4]).abs() < 0.01);

        for _ in 0..90 {
            rig.step(1.0 / 120.0, true, false);
        }
        let active = rig.visualizer_levels();
        assert!(active.iter().all(|&v| (0.15..=1.0).contains(&v)));
        assert!(rig.play_mix() > 0.90);

        for _ in 0..240 {
            rig.step(1.0 / 120.0, false, false);
        }
        assert!(rig.play_mix() < 0.02);
        assert!(rig.is_idle_settled(false));

        assert!((rig.speaker_volume() - 100.0).abs() < 0.1);
        rig.set_muted(true);
        assert!(!rig.is_idle_settled(false));
        for _ in 0..12 {
            rig.step(1.0 / 120.0, false, false);
        }
        let mid_v = rig.speaker_volume();
        assert!(mid_v > 5.0 && mid_v < 95.0, "expected smooth mid transition, got {mid_v}");

        for _ in 0..240 {
            rig.step(1.0 / 120.0, false, false);
        }
        assert!(rig.speaker_volume() < 0.1);
        assert!(rig.is_idle_settled(false));

        rig.set_muted(false);
        for _ in 0..240 {
            rig.step(1.0 / 120.0, false, false);
        }
        assert!((rig.speaker_volume() - 100.0).abs() < 0.1);
        assert!(rig.is_idle_settled(false));
    }

    #[test]
    fn volume_slider_hover_percent_and_dock_wobble() {
        let cfg = Config::default();
        let mut rig = NotchSprings::new(&cfg);

        assert_eq!(rig.volume_slider_open(), 0.0);
        rig.set_volume_hovered(true);
        assert!(!rig.is_idle_settled(false));
        for _ in 0..18 {
            rig.step(1.0 / 120.0, false, false);
        }
        let open_mid = rig.volume_slider_open();
        assert!(open_mid > 0.1 && open_mid < 0.99, "mid={open_mid}");
        for _ in 0..180 {
            rig.step(1.0 / 120.0, false, false);
        }
        assert!((rig.volume_slider_open() - 1.0).abs() < 0.01);

        rig.set_volume(25.0, false);
        for _ in 0..240 {
            rig.step(1.0 / 120.0, false, false);
        }
        assert!((rig.speaker_volume() - 25.0).abs() < 0.1);

        rig.set_volume(25.0, true);
        for _ in 0..240 {
            rig.step(1.0 / 120.0, false, false);
        }
        assert!(rig.speaker_volume() < 0.1);

        rig.set_volume_hovered(false);
        for _ in 0..180 {
            rig.step(1.0 / 120.0, false, false);
        }
        assert!(rig.volume_slider_open() < 0.01);
        assert!(rig.is_idle_settled(false));

        rig.apply_state(&cfg, true, true, true, false);
        let mut max_wobble = 0.0_f32;
        for _ in 0..360 {
            rig.step(1.0 / 120.0, false, false);
            max_wobble = max_wobble.max(rig.dock_wobble());
        }
        assert!(
            (1.05..=1.12).contains(&max_wobble),
            "expected smooth ~8.4% single dock wobble overshoot, got {max_wobble}"
        );
        assert!((rig.dock_wobble() - 1.0).abs() < 0.01);
        assert!(rig.is_idle_settled(false));

        rig.apply_state(&cfg, false, false, false, false);
        for _ in 0..360 {
            rig.step(1.0 / 120.0, false, false);
        }
        assert!(rig.dock_wobble() < 0.01);
        assert!(rig.is_idle_settled(false));
    }

    #[test]
    fn osd_caps_lock_and_lyric_started_springs() {
        let cfg = Config::default();
        let mut rig = NotchSprings::new(&cfg);

        rig.set_osd_active(true);
        rig.apply_state(&cfg, false, false, false, false);
        assert!(!rig.is_idle_settled(false));
        for _ in 0..360 {
            rig.step(1.0 / 120.0, false, false);
        }
        assert!((rig.osd_open() - 1.0).abs() < 0.01);
        assert!((rig.width.value - 276.0).abs() < 0.5);
        assert!((rig.height.value - 34.0).abs() < 0.2);

        assert_eq!(rig.caps_on_mix(), 0.0);
        rig.set_caps_on(true);
        for _ in 0..18 {
            rig.step(1.0 / 120.0, false, false);
        }
        let caps_mid = rig.caps_on_mix();
        assert!(caps_mid > 0.05 && caps_mid < 0.95, "caps_mid={caps_mid}");
        for _ in 0..240 {
            rig.step(1.0 / 120.0, false, false);
        }
        assert!((rig.caps_on_mix() - 1.0).abs() < 0.01);

        rig.set_osd_active(false);
        rig.apply_state(&cfg, false, false, false, false);
        for _ in 0..360 {
            rig.step(1.0 / 120.0, false, false);
        }
        assert!(rig.osd_open() < 0.01);
        assert!((rig.width.value - cfg.notch_width).abs() < 0.5);

        assert_eq!(rig.lyric_started_mix(), 1.0);
        rig.set_lyric_started(false);
        for _ in 0..240 {
            rig.step(1.0 / 120.0, false, false);
        }
        assert!(rig.lyric_started_mix() < 0.01);
        rig.set_lyric_started(true);
        for _ in 0..240 {
            rig.step(1.0 / 120.0, false, false);
        }
        assert!((rig.lyric_started_mix() - 1.0).abs() < 0.01);
        assert!(rig.is_idle_settled(false));
    }
}
