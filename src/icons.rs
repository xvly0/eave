
use std::f32::consts::{PI, TAU};
use std::mem::ManuallyDrop;

use windows::Foundation::Numerics::Matrix3x2;
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_BEZIER_SEGMENT, D2D1_COLOR_F, D2D1_FIGURE_BEGIN_FILLED, D2D1_FIGURE_BEGIN_HOLLOW,
    D2D1_FIGURE_END_CLOSED, D2D1_FIGURE_END_OPEN, D2D1_FILL_MODE_ALTERNATE, D2D_POINT_2F,
    D2D_RECT_F, D2D_SIZE_F,
};
use windows::Win32::Graphics::Direct2D::{
    ID2D1Brush, ID2D1DeviceContext, ID2D1Factory, ID2D1GeometrySink, ID2D1PathGeometry,
    ID2D1StrokeStyle, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_ARC_SEGMENT, D2D1_ARC_SIZE_LARGE,
    D2D1_ARC_SIZE_SMALL, D2D1_CAP_STYLE_ROUND, D2D1_DASH_STYLE_SOLID, D2D1_ELLIPSE,
    D2D1_LAYER_OPTIONS1_NONE, D2D1_LAYER_PARAMETERS1, D2D1_LINE_JOIN_ROUND,
    D2D1_QUADRATIC_BEZIER_SEGMENT, D2D1_ROUNDED_RECT, D2D1_STROKE_STYLE_PROPERTIES,
    D2D1_SWEEP_DIRECTION_CLOCKWISE, D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE,
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Vec2 {
    pub x: f32,
    pub y: f32,
}

impl Vec2 {
    #[inline]
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    #[inline]
    pub fn to_d2d(self, cx: f32, cy: f32, size: f32) -> D2D_POINT_2F {
        D2D_POINT_2F {
            x: cx + self.x * size,
            y: cy + self.y * size,
        }
    }
}

#[inline]
fn pt(cx: f32, cy: f32, size: f32, nx: f32, ny: f32) -> D2D_POINT_2F {
    D2D_POINT_2F {
        x: cx + nx * size,
        y: cy + ny * size,
    }
}

#[inline]
fn default_stroke(size: f32) -> f32 {
    (size * 0.105).max(1.2)
}

#[inline]
fn create_round_stroke(factory: &ID2D1Factory) -> Option<ID2D1StrokeStyle> {
    let props = D2D1_STROKE_STYLE_PROPERTIES {
        startCap: D2D1_CAP_STYLE_ROUND,
        endCap: D2D1_CAP_STYLE_ROUND,
        dashCap: D2D1_CAP_STYLE_ROUND,
        lineJoin: D2D1_LINE_JOIN_ROUND,
        miterLimit: 10.0,
        dashStyle: D2D1_DASH_STYLE_SOLID,
        dashOffset: 0.0,
    };
    unsafe { factory.CreateStrokeStyle(&props, None).ok() }
}

#[inline]
fn build_path<F>(factory: &ID2D1Factory, build: F) -> Option<ID2D1PathGeometry>
where
    F: FnOnce(&ID2D1GeometrySink),
{
    unsafe {
        let geo = factory.CreatePathGeometry().ok()?;
        let sink = geo.Open().ok()?;
        build(&sink);
        sink.Close().ok()?;
        Some(geo)
    }
}

#[inline]
fn add_cubic(
    sink: &ID2D1GeometrySink,
    c1: D2D_POINT_2F,
    c2: D2D_POINT_2F,
    end: D2D_POINT_2F,
) {
    unsafe {
        sink.AddBezier(&D2D1_BEZIER_SEGMENT {
            point1: c1,
            point2: c2,
            point3: end,
        });
    }
}

#[inline]
#[allow(clippy::too_many_arguments)]
fn add_arc(
    sink: &ID2D1GeometrySink,
    cx: f32,
    cy: f32,
    size: f32,
    end_vx: f32,
    end_vy: f32,
    rx: f32,
    ry: f32,
    large_arc: bool,
    sweep_cw: bool,
) {
    let scale = size / 24.0;
    unsafe {
        sink.AddArc(&D2D1_ARC_SEGMENT {
            point: vb24(cx, cy, size, end_vx, end_vy),
            size: D2D_SIZE_F {
                width: rx * scale,
                height: ry * scale,
            },
            rotationAngle: 0.0,
            sweepDirection: if sweep_cw {
                D2D1_SWEEP_DIRECTION_CLOCKWISE
            } else {
                D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE
            },
            arcSize: if large_arc {
                D2D1_ARC_SIZE_LARGE
            } else {
                D2D1_ARC_SIZE_SMALL
            },
        });
    }
}

#[inline]
#[allow(clippy::too_many_arguments)]
fn add_quad(
    sink: &ID2D1GeometrySink,
    cx: f32,
    cy: f32,
    size: f32,
    ctrl_vx: f32,
    ctrl_vy: f32,
    end_vx: f32,
    end_vy: f32,
) {
    unsafe {
        sink.AddQuadraticBezier(&D2D1_QUADRATIC_BEZIER_SEGMENT {
            point1: vb24(cx, cy, size, ctrl_vx, ctrl_vy),
            point2: vb24(cx, cy, size, end_vx, end_vy),
        });
    }
}

#[inline]
fn with_opacity_layer<F>(
    ctx: &ID2D1DeviceContext,
    cx: f32,
    cy: f32,
    size: f32,
    opacity: f32,
    draw: F,
) where
    F: FnOnce(),
{
    if opacity <= 0.005 {
        return;
    }
    if opacity >= 0.999 {
        draw();
        return;
    }
    let params = D2D1_LAYER_PARAMETERS1 {
        contentBounds: D2D_RECT_F {
            left: cx - size,
            top: cy - size,
            right: cx + size,
            bottom: cy + size,
        },
        geometricMask: ManuallyDrop::new(None),
        maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
        maskTransform: Matrix3x2::identity(),
        opacity: opacity.clamp(0.0, 1.0),
        opacityBrush: ManuallyDrop::new(None),
        layerOptions: D2D1_LAYER_OPTIONS1_NONE,
    };
    unsafe {
        ctx.PushLayer(&params, None);
        draw();
        ctx.PopLayer();
    }
}

pub(crate) fn gear_outline_points(
    teeth: usize,
    r_outer: f32,
    r_inner: f32,
    tooth_half_angle: f32,
    base_half_angle: f32,
) -> Vec<Vec2> {
    let mut pts = Vec::with_capacity(teeth * 4);
    let step = TAU / teeth as f32;
    for i in 0..teeth {
        let a = i as f32 * step - PI * 0.5;
        let angles_radii = [
            (a - base_half_angle, r_inner),
            (a - tooth_half_angle, r_outer),
            (a + tooth_half_angle, r_outer),
            (a + base_half_angle, r_inner),
        ];
        for (ang, r) in angles_radii {
            pts.push(Vec2::new(ang.cos() * r, ang.sin() * r));
        }
    }
    pts
}

#[inline]
pub(crate) fn viewbox_24_to_norm(vx: f32, vy: f32) -> Vec2 {
    Vec2::new((vx - 12.0) / 24.0, (vy - 12.0) / 24.0)
}

#[inline]
fn vb24(cx: f32, cy: f32, size: f32, vx: f32, vy: f32) -> D2D_POINT_2F {
    viewbox_24_to_norm(vx, vy).to_d2d(cx, cy, size)
}

#[inline]
pub(crate) fn chevron_points(pointing_right: bool) -> [Vec2; 3] {
    if pointing_right {
        [
            Vec2::new(-0.14, -0.28),
            Vec2::new(0.16, 0.0),
            Vec2::new(-0.14, 0.28),
        ]
    } else {
        [
            Vec2::new(0.14, -0.28),
            Vec2::new(-0.16, 0.0),
            Vec2::new(0.14, 0.28),
        ]
    }
}

#[inline]
pub(crate) fn speaker_bar_opacities_for(volume: f32, max_bars: u8) -> (f32, f32, f32, f32) {
    let v = volume.clamp(0.0, 100.0);
    let mute_x = (1.0 - v / 0.8).clamp(0.0, 1.0);
    if max_bars <= 2 {
        let b1 = (v / 35.0).clamp(0.0, 1.0);
        let b2 = ((v - 35.0) / 35.0).clamp(0.0, 1.0);
        (b1, b2, 0.0, mute_x)
    } else {
        let b1 = (v / 33.333).clamp(0.0, 1.0);
        let b2 = ((v - 33.333) / 33.333).clamp(0.0, 1.0);
        let b3 = ((v - 66.666) / 33.333).clamp(0.0, 1.0);
        (b1, b2, b3, mute_x)
    }
}

#[inline]
#[allow(dead_code)]
pub(crate) fn speaker_bar_opacities(volume: f32) -> (f32, f32, f32, f32) {
    speaker_bar_opacities_for(volume, 3)
}

#[inline]
pub(crate) fn audio_bar_spans(levels: Option<[f32; 5]>) -> [(f32, f32, f32); 5] {
    const XS: [f32; 5] = [4.0, 8.0, 12.0, 16.0, 20.0];
    const DEFAULTS: [(f32, f32); 5] = [
        (10.4, 13.6),
        (9.4, 14.6),
        (8.2, 15.8),
        (9.6, 14.4),
        (10.4, 13.6),
    ];
    match levels {
        None => std::array::from_fn(|i| (XS[i], DEFAULTS[i].0, DEFAULTS[i].1)),
        Some(lvls) => std::array::from_fn(|i| {
            let half = (lvls[i].clamp(0.0, 1.0) * 6.2).max(0.8);
            (XS[i], 12.0 - half, 12.0 + half)
        }),
    }
}

pub fn draw_music_note(
    ctx: &ID2D1DeviceContext,
    cx: f32,
    cy: f32,
    size: f32,
    brush: &ID2D1Brush,
) {
    if size <= 0.0 {
        return;
    }
    let Ok(factory) = (unsafe { ctx.GetFactory() }) else {
        return;
    };

    let vb = |vx: f32, vy: f32| vb24(cx, cy, size, vx, vy);
    let head_r = (2.8 / 24.0) * size;

    unsafe {
        ctx.FillEllipse(
            &D2D1_ELLIPSE {
                point: vb(6.9, 17.3),
                radiusX: head_r,
                radiusY: head_r,
            },
            brush,
        );
        ctx.FillEllipse(
            &D2D1_ELLIPSE {
                point: vb(16.7, 15.3),
                radiusX: head_r,
                radiusY: head_r,
            },
            brush,
        );
    }

    if let Some(geo) = build_path(&factory, |sink| unsafe {
        sink.BeginFigure(vb(8.6, 17.3), D2D1_FIGURE_BEGIN_FILLED);
        sink.AddLine(vb(8.6, 6.9));
        add_cubic(sink, vb(8.6, 6.3), vb(9.0, 5.8), vb(9.6, 5.7));
        sink.AddLine(vb(18.2, 3.8));
        add_cubic(sink, vb(19.0, 3.6), vb(19.7, 4.2), vb(19.7, 5.0));
        sink.AddLine(vb(19.7, 15.4));
        sink.AddLine(vb(17.5, 15.4));
        sink.AddLine(vb(17.5, 8.2));
        sink.AddLine(vb(10.8, 9.7));
        sink.AddLine(vb(10.8, 17.3));
        sink.EndFigure(D2D1_FIGURE_END_CLOSED);
    }) {
        unsafe { ctx.FillGeometry(&geo, brush, None) };
    }
}

#[allow(dead_code)]
pub fn draw_stopwatch(
    ctx: &ID2D1DeviceContext,
    cx: f32,
    cy: f32,
    size: f32,
    brush: &ID2D1Brush,
) {
    draw_stopwatch_stroked(ctx, cx, cy, size, brush, default_stroke(size));
}

#[allow(dead_code)]
pub fn draw_stopwatch_stroked(
    ctx: &ID2D1DeviceContext,
    cx: f32,
    cy: f32,
    size: f32,
    brush: &ID2D1Brush,
    stroke_width: f32,
) {
    if size <= 0.0 {
        return;
    }
    let Ok(factory) = (unsafe { ctx.GetFactory() }) else {
        return;
    };
    let stroke = create_round_stroke(&factory);
    let body_center = pt(cx, cy, size, 0.0, 0.075);
    let r = 0.31 * size;

    unsafe {
        ctx.DrawEllipse(
            &D2D1_ELLIPSE {
                point: body_center,
                radiusX: r,
                radiusY: r,
            },
            brush,
            stroke_width,
            stroke.as_ref(),
        );
        ctx.DrawLine(
            pt(cx, cy, size, 0.0, -0.235),
            pt(cx, cy, size, 0.0, -0.355),
            brush,
            stroke_width,
            stroke.as_ref(),
        );
        ctx.DrawLine(
            pt(cx, cy, size, -0.105, -0.365),
            pt(cx, cy, size, 0.105, -0.365),
            brush,
            stroke_width,
            stroke.as_ref(),
        );
        ctx.DrawLine(
            pt(cx, cy, size, 0.225, -0.145),
            pt(cx, cy, size, 0.305, -0.225),
            brush,
            stroke_width,
            stroke.as_ref(),
        );
        ctx.DrawLine(
            body_center,
            pt(cx, cy, size, 0.08, -0.06),
            brush,
            stroke_width,
            stroke.as_ref(),
        );
    }
}

#[allow(dead_code)]
pub fn draw_water_drop(
    ctx: &ID2D1DeviceContext,
    cx: f32,
    cy: f32,
    size: f32,
    brush: &ID2D1Brush,
) {
    draw_water_drop_stroked(ctx, cx, cy, size, brush, default_stroke(size));
}

#[allow(dead_code)]
pub fn draw_water_drop_stroked(
    ctx: &ID2D1DeviceContext,
    cx: f32,
    cy: f32,
    size: f32,
    brush: &ID2D1Brush,
    stroke_width: f32,
) {
    if size <= 0.0 {
        return;
    }
    let Ok(factory) = (unsafe { ctx.GetFactory() }) else {
        return;
    };
    let stroke = create_round_stroke(&factory);

    if let Some(geo) = build_path(&factory, |sink| unsafe {
        sink.BeginFigure(pt(cx, cy, size, 0.0, -0.38), D2D1_FIGURE_BEGIN_HOLLOW);
        add_cubic(
            sink,
            pt(cx, cy, size, 0.12, -0.20),
            pt(cx, cy, size, 0.29, -0.02),
            pt(cx, cy, size, 0.29, 0.13),
        );
        add_cubic(
            sink,
            pt(cx, cy, size, 0.29, 0.29),
            pt(cx, cy, size, 0.16, 0.39),
            pt(cx, cy, size, 0.0, 0.39),
        );
        add_cubic(
            sink,
            pt(cx, cy, size, -0.16, 0.39),
            pt(cx, cy, size, -0.29, 0.29),
            pt(cx, cy, size, -0.29, 0.13),
        );
        add_cubic(
            sink,
            pt(cx, cy, size, -0.29, -0.02),
            pt(cx, cy, size, -0.12, -0.20),
            pt(cx, cy, size, 0.0, -0.38),
        );
        sink.EndFigure(D2D1_FIGURE_END_CLOSED);
    }) {
        unsafe { ctx.DrawGeometry(&geo, brush, stroke_width, stroke.as_ref()) };
    }
}

pub fn draw_water_drop_filled(
    ctx: &ID2D1DeviceContext,
    cx: f32,
    cy: f32,
    size: f32,
    brush: &ID2D1Brush,
) {
    if size <= 0.0 {
        return;
    }
    let Ok(factory) = (unsafe { ctx.GetFactory() }) else {
        return;
    };
    if let Some(geo) = build_path(&factory, |sink| unsafe {
        sink.BeginFigure(pt(cx, cy, size, 0.0, -0.40), D2D1_FIGURE_BEGIN_FILLED);
        add_cubic(
            sink,
            pt(cx, cy, size, 0.13, -0.21),
            pt(cx, cy, size, 0.30, -0.02),
            pt(cx, cy, size, 0.30, 0.14),
        );
        add_cubic(
            sink,
            pt(cx, cy, size, 0.30, 0.30),
            pt(cx, cy, size, 0.17, 0.40),
            pt(cx, cy, size, 0.0, 0.40),
        );
        add_cubic(
            sink,
            pt(cx, cy, size, -0.17, 0.40),
            pt(cx, cy, size, -0.30, 0.30),
            pt(cx, cy, size, -0.30, 0.14),
        );
        add_cubic(
            sink,
            pt(cx, cy, size, -0.30, -0.02),
            pt(cx, cy, size, -0.13, -0.21),
            pt(cx, cy, size, 0.0, -0.40),
        );
        sink.EndFigure(D2D1_FIGURE_END_CLOSED);
    }) {
        unsafe { ctx.FillGeometry(&geo, brush, None) };
    }
}

#[allow(dead_code)]
pub fn draw_shelf_tray(
    ctx: &ID2D1DeviceContext,
    cx: f32,
    cy: f32,
    size: f32,
    brush: &ID2D1Brush,
) {
    draw_shelf_tray_stroked(ctx, cx, cy, size, brush, default_stroke(size));
}

#[allow(dead_code)]
pub fn draw_shelf_tray_stroked(
    ctx: &ID2D1DeviceContext,
    cx: f32,
    cy: f32,
    size: f32,
    brush: &ID2D1Brush,
    stroke_width: f32,
) {
    if size <= 0.0 {
        return;
    }
    let Ok(factory) = (unsafe { ctx.GetFactory() }) else {
        return;
    };
    let stroke = create_round_stroke(&factory);

    if let Some(geo) = build_path(&factory, |sink| unsafe {
        sink.BeginFigure(pt(cx, cy, size, -0.25, -0.24), D2D1_FIGURE_BEGIN_HOLLOW);
        sink.AddLine(pt(cx, cy, size, 0.25, -0.24));
        sink.AddLine(pt(cx, cy, size, 0.37, 0.05));
        sink.AddLine(pt(cx, cy, size, 0.37, 0.21));
        add_cubic(
            sink,
            pt(cx, cy, size, 0.37, 0.26),
            pt(cx, cy, size, 0.34, 0.28),
            pt(cx, cy, size, 0.29, 0.28),
        );
        sink.AddLine(pt(cx, cy, size, -0.29, 0.28));
        add_cubic(
            sink,
            pt(cx, cy, size, -0.34, 0.28),
            pt(cx, cy, size, -0.37, 0.26),
            pt(cx, cy, size, -0.37, 0.21),
        );
        sink.AddLine(pt(cx, cy, size, -0.37, 0.05));
        sink.EndFigure(D2D1_FIGURE_END_CLOSED);

        sink.BeginFigure(pt(cx, cy, size, -0.36, 0.05), D2D1_FIGURE_BEGIN_HOLLOW);
        sink.AddLine(pt(cx, cy, size, -0.14, 0.05));
        add_cubic(
            sink,
            pt(cx, cy, size, -0.11, 0.16),
            pt(cx, cy, size, 0.11, 0.16),
            pt(cx, cy, size, 0.14, 0.05),
        );
        sink.AddLine(pt(cx, cy, size, 0.36, 0.05));
        sink.EndFigure(D2D1_FIGURE_END_OPEN);
    }) {
        unsafe { ctx.DrawGeometry(&geo, brush, stroke_width, stroke.as_ref()) };
    }
}

pub fn draw_shelf_drop_hero(
    ctx: &ID2D1DeviceContext,
    cx: f32,
    cy: f32,
    size: f32,
    brush: &ID2D1Brush,
    arrow_dy: f32,
) {
    if size <= 0.0 {
        return;
    }
    let Ok(factory) = (unsafe { ctx.GetFactory() }) else {
        return;
    };
    let stroke = create_round_stroke(&factory);

    let vb = |vx: f32, vy: f32| -> D2D_POINT_2F {
        pt(cx, cy, size, (vx - 12.0) / 24.0, (vy - 12.0) / 24.0)
    };
    let vb_arr = |vx: f32, vy: f32| -> D2D_POINT_2F {
        let p = vb(vx, vy);
        D2D_POINT_2F {
            x: p.x,
            y: p.y + arrow_dy,
        }
    };
    let push_opacity_fill = |geo: &ID2D1PathGeometry, opacity: f32| {
        let params = D2D1_LAYER_PARAMETERS1 {
            contentBounds: D2D_RECT_F {
                left: cx - size,
                top: cy - size,
                right: cx + size,
                bottom: cy + size,
            },
            geometricMask: ManuallyDrop::new(None),
            maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
            maskTransform: Matrix3x2::identity(),
            opacity,
            opacityBrush: ManuallyDrop::new(None),
            layerOptions: D2D1_LAYER_OPTIONS1_NONE,
        };
        unsafe {
            ctx.PushLayer(&params, None);
            ctx.FillGeometry(geo, brush, None);
            ctx.PopLayer();
        }
    };

    if let Some(pocket_geo) = build_path(&factory, |sink| unsafe {
        sink.BeginFigure(vb(5.0, 12.5), D2D1_FIGURE_BEGIN_FILLED);
        sink.AddLine(vb(19.0, 12.5));
        sink.AddLine(vb(20.6, 17.5));
        add_cubic(sink, vb(21.0, 19.2), vb(19.9, 20.5), vb(18.2, 20.5));
        sink.AddLine(vb(5.8, 20.5));
        add_cubic(sink, vb(4.1, 20.5), vb(3.0, 19.2), vb(3.4, 17.5));
        sink.EndFigure(D2D1_FIGURE_END_CLOSED);
    }) {
        push_opacity_fill(&pocket_geo, 0.22);
    }

    if let Some(tray_geo) = build_path(&factory, |sink| unsafe {
        sink.BeginFigure(vb(3.5, 14.5), D2D1_FIGURE_BEGIN_HOLLOW);
        sink.AddLine(vb(3.5, 17.8));
        add_cubic(sink, vb(3.5, 19.6), vb(4.9, 20.8), vb(6.7, 20.8));
        sink.AddLine(vb(17.3, 20.8));
        add_cubic(sink, vb(19.1, 20.8), vb(20.5, 19.6), vb(20.5, 17.8));
        sink.AddLine(vb(20.5, 14.5));
        sink.EndFigure(D2D1_FIGURE_END_OPEN);
    }) {
        unsafe { ctx.DrawGeometry(&tray_geo, brush, (size * 0.072).max(1.8), stroke.as_ref()) };
    }

    let arr_sw = (size * 0.074).max(1.85);
    unsafe {
        ctx.DrawLine(
            vb_arr(12.0, 4.0),
            vb_arr(12.0, 14.6),
            brush,
            arr_sw,
            stroke.as_ref(),
        );
    }
    if let Some(head_geo) = build_path(&factory, |sink| unsafe {
        sink.BeginFigure(vb_arr(7.8, 10.6), D2D1_FIGURE_BEGIN_HOLLOW);
        sink.AddLine(vb_arr(12.0, 15.0));
        sink.AddLine(vb_arr(16.2, 10.6));
        sink.EndFigure(D2D1_FIGURE_END_OPEN);
    }) {
        unsafe { ctx.DrawGeometry(&head_geo, brush, arr_sw, stroke.as_ref()) };
    }
}

#[allow(dead_code)]
pub fn draw_checklist(
    ctx: &ID2D1DeviceContext,
    cx: f32,
    cy: f32,
    size: f32,
    brush: &ID2D1Brush,
) {
    draw_checklist_stroked(ctx, cx, cy, size, brush, default_stroke(size));
}

#[allow(dead_code)]
pub fn draw_checklist_stroked(
    ctx: &ID2D1DeviceContext,
    cx: f32,
    cy: f32,
    size: f32,
    brush: &ID2D1Brush,
    stroke_width: f32,
) {
    if size <= 0.0 {
        return;
    }
    let Ok(factory) = (unsafe { ctx.GetFactory() }) else {
        return;
    };
    let stroke = create_round_stroke(&factory);

    if let Some(geo) = build_path(&factory, |sink| unsafe {
        for row_y in [-0.16_f32, 0.16_f32] {
            sink.BeginFigure(pt(cx, cy, size, -0.35, row_y + 0.01), D2D1_FIGURE_BEGIN_HOLLOW);
            sink.AddLine(pt(cx, cy, size, -0.25, row_y + 0.09));
            sink.AddLine(pt(cx, cy, size, -0.09, row_y - 0.08));
            sink.EndFigure(D2D1_FIGURE_END_OPEN);

            sink.BeginFigure(pt(cx, cy, size, 0.04, row_y), D2D1_FIGURE_BEGIN_HOLLOW);
            sink.AddLine(pt(cx, cy, size, 0.36, row_y));
            sink.EndFigure(D2D1_FIGURE_END_OPEN);
        }
    }) {
        unsafe { ctx.DrawGeometry(&geo, brush, stroke_width, stroke.as_ref()) };
    }
}

#[allow(dead_code)]
pub fn draw_settings_gear(
    ctx: &ID2D1DeviceContext,
    cx: f32,
    cy: f32,
    size: f32,
    brush: &ID2D1Brush,
) {
    draw_settings_gear_stroked(ctx, cx, cy, size, brush, default_stroke(size));
}

#[allow(dead_code)]
pub fn draw_settings_gear_stroked(
    ctx: &ID2D1DeviceContext,
    cx: f32,
    cy: f32,
    size: f32,
    brush: &ID2D1Brush,
    stroke_width: f32,
) {
    if size <= 0.0 {
        return;
    }
    let Ok(factory) = (unsafe { ctx.GetFactory() }) else {
        return;
    };
    let stroke = create_round_stroke(&factory);
    let pts = gear_outline_points(8, 0.38, 0.27, 0.115, 0.22);

    if let Some(geo) = build_path(&factory, |sink| unsafe {
        if let Some((first, rest)) = pts.split_first() {
            sink.BeginFigure(first.to_d2d(cx, cy, size), D2D1_FIGURE_BEGIN_HOLLOW);
            for p in rest {
                sink.AddLine(p.to_d2d(cx, cy, size));
            }
            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
        }
    }) {
        unsafe { ctx.DrawGeometry(&geo, brush, stroke_width, stroke.as_ref()) };
    }

    let inner_r = 0.125 * size;
    unsafe {
        ctx.DrawEllipse(
            &D2D1_ELLIPSE {
                point: D2D_POINT_2F { x: cx, y: cy },
                radiusX: inner_r,
                radiusY: inner_r,
            },
            brush,
            stroke_width,
            stroke.as_ref(),
        );
    }
}

pub fn draw_bell(
    ctx: &ID2D1DeviceContext,
    cx: f32,
    cy: f32,
    size: f32,
    brush: &ID2D1Brush,
    badge_brush: Option<&ID2D1Brush>,
) {
    if size <= 0.0 {
        return;
    }
    let Ok(factory) = (unsafe { ctx.GetFactory() }) else {
        return;
    };

    let vb = |vx: f32, vy: f32| -> D2D_POINT_2F {
        pt(cx, cy, size, (vx - 12.0) / 24.0, (vy - 12.0) / 24.0)
    };

    if let Some(geo) = build_path(&factory, |sink| unsafe {
        sink.BeginFigure(vb(12.0, 3.2), D2D1_FIGURE_BEGIN_FILLED);
        add_cubic(sink, vb(15.5, 3.2), vb(18.3, 6.0), vb(18.3, 9.5));
        sink.AddLine(vb(18.3, 14.9));
        sink.AddLine(vb(20.0, 17.2));
        add_cubic(sink, vb(20.4, 17.7), vb(20.0, 18.5), vb(19.3, 18.5));
        sink.AddLine(vb(4.7, 18.5));
        add_cubic(sink, vb(4.0, 18.5), vb(3.6, 17.7), vb(4.0, 17.2));
        sink.AddLine(vb(5.7, 14.9));
        sink.AddLine(vb(5.7, 9.5));
        add_cubic(sink, vb(5.7, 6.0), vb(8.5, 3.2), vb(12.0, 3.2));
        sink.EndFigure(D2D1_FIGURE_END_CLOSED);

        sink.BeginFigure(vb(9.6, 19.6), D2D1_FIGURE_BEGIN_FILLED);
        sink.AddLine(vb(14.4, 19.6));
        add_cubic(sink, vb(14.4, 22.8), vb(9.6, 22.8), vb(9.6, 19.6));
        sink.EndFigure(D2D1_FIGURE_END_CLOSED);
    }) {
        unsafe { ctx.FillGeometry(&geo, brush, None) };
    }

    if let Some(badge) = badge_brush {
        let dot_r = (0.145 * size).max(2.1);
        unsafe {
            ctx.FillEllipse(
                &D2D1_ELLIPSE {
                    point: pt(cx, cy, size, 0.28, -0.27),
                    radiusX: dot_r,
                    radiusY: dot_r,
                },
                badge,
            );
        }
    }
}

#[allow(dead_code)]
pub fn draw_lyrics_bubble(
    ctx: &ID2D1DeviceContext,
    cx: f32,
    cy: f32,
    size: f32,
    brush: &ID2D1Brush,
) {
    draw_lyrics_bubble_stroked(ctx, cx, cy, size, brush, 2.1 * (size / 24.0));
}

fn add_lyrics_bubble_figure(
    sink: &ID2D1GeometrySink,
    cx: f32,
    cy: f32,
    size: f32,
    filled: bool,
) {
    let vb = |vx: f32, vy: f32| vb24(cx, cy, size, vx, vy);
    let fig_begin = if filled {
        D2D1_FIGURE_BEGIN_FILLED
    } else {
        D2D1_FIGURE_BEGIN_HOLLOW
    };
    unsafe {
        sink.BeginFigure(vb(6.2, 2.8), fig_begin);
        sink.AddLine(vb(17.8, 2.8));
        add_arc(sink, cx, cy, size, 22.2, 7.2, 4.4, 4.4, false, true);
        sink.AddLine(vb(22.2, 13.8));
        add_arc(sink, cx, cy, size, 17.8, 18.2, 4.4, 4.4, false, true);
        sink.AddLine(vb(13.2, 18.2));
        sink.AddLine(vb(7.2, 22.6));
        sink.AddLine(vb(7.8, 18.2));
        sink.AddLine(vb(6.2, 18.2));
        add_arc(sink, cx, cy, size, 1.8, 13.8, 4.4, 4.4, false, true);
        sink.AddLine(vb(1.8, 7.2));
        add_arc(sink, cx, cy, size, 6.2, 2.8, 4.4, 4.4, false, true);
        sink.EndFigure(D2D1_FIGURE_END_CLOSED);
    }
}

fn add_lyrics_note_figure(
    sink: &ID2D1GeometrySink,
    cx: f32,
    cy: f32,
    size: f32,
    dx: f32,
) {
    let vb = |vx: f32, vy: f32| vb24(cx, cy, size, vx + dx, vy);
    unsafe {
        sink.BeginFigure(vb(10.75, 9.5), D2D1_FIGURE_BEGIN_FILLED);
        add_arc(sink, cx, cy, size, 8.22 + dx, 11.22, 1.85, 1.85, true, false);
        add_quad(sink, cx, cy, size, 9.15 + dx, 12.2, 7.7 + dx, 13.5);
        sink.AddLine(vb(7.73, 13.47));
        add_arc(sink, cx, cy, size, 7.71 + dx, 14.66, 0.82, 0.82, false, false);
        add_quad(sink, cx, cy, size, 8.5 + dx, 15.05, 9.15 + dx, 14.45);
        add_quad(sink, cx, cy, size, 10.92 + dx, 12.75, 10.75 + dx, 9.5);
        sink.EndFigure(D2D1_FIGURE_END_CLOSED);
    }
}

#[allow(dead_code)]
pub fn draw_lyrics_bubble_stroked(
    ctx: &ID2D1DeviceContext,
    cx: f32,
    cy: f32,
    size: f32,
    brush: &ID2D1Brush,
    stroke_width: f32,
) {
    if size <= 0.0 {
        return;
    }
    let Ok(factory) = (unsafe { ctx.GetFactory() }) else {
        return;
    };
    let stroke = create_round_stroke(&factory);
    let sw = stroke_width.max(1.0);

    if let Some(outline_geo) = build_path(&factory, |sink| {
        add_lyrics_bubble_figure(sink, cx, cy, size, false);
    }) {
        unsafe { ctx.DrawGeometry(&outline_geo, brush, sw, stroke.as_ref()) };
    }
    if let Some(notes_geo) = build_path(&factory, |sink| {
        for dx in [0.0_f32, 5.8_f32] {
            add_lyrics_note_figure(sink, cx, cy, size, dx);
        }
    }) {
        unsafe { ctx.FillGeometry(&notes_geo, brush, None) };
    }
}

#[allow(dead_code)]
pub unsafe fn draw_lyrics_bubble_filled(
    ctx: &ID2D1DeviceContext,
    cx: f32,
    cy: f32,
    size: f32,
    fill_brush: &ID2D1Brush,
    _dot_brush: &ID2D1Brush,
) {
    if size <= 0.0 {
        return;
    }
    let Ok(factory) = ctx.GetFactory() else {
        return;
    };

    if let Some(geo) = build_path(&factory, |sink| {
        sink.SetFillMode(D2D1_FILL_MODE_ALTERNATE);
        add_lyrics_bubble_figure(sink, cx, cy, size, true);
        for dx in [0.0_f32, 5.8_f32] {
            add_lyrics_note_figure(sink, cx, cy, size, dx);
        }
    }) {
        ctx.FillGeometry(&geo, fill_brush, None);
    }
}

const SPEAKER_BARS: [(f32, f32, f32, f32); 3] = [
    (13.99, 9.05, 14.95, 4.2),
    (16.55, 6.51, 17.49, 7.8),
    (19.11, 3.99, 20.01, 11.4),
];

fn add_speaker_cone_figure(sink: &ID2D1GeometrySink, cx: f32, cy: f32, size: f32) {
    let vb = |vx: f32, vy: f32| vb24(cx, cy, size, vx, vy);
    unsafe {
        sink.BeginFigure(vb(2.0, 12.0), D2D1_FIGURE_BEGIN_FILLED);
        sink.AddLine(vb(2.0, 9.5));
        add_arc(sink, cx, cy, size, 2.9, 8.6, 0.9, 0.9, false, true);
        sink.AddLine(vb(5.87, 8.6));
        add_arc(sink, cx, cy, size, 6.45, 8.39, 0.9, 0.9, false, false);
        sink.AddLine(vb(10.42, 5.05));
        add_arc(sink, cx, cy, size, 11.9, 5.73, 0.9, 0.9, false, true);
        sink.AddLine(vb(11.9, 18.27));
        add_arc(sink, cx, cy, size, 10.42, 18.95, 0.9, 0.9, false, true);
        sink.AddLine(vb(6.45, 15.61));
        add_arc(sink, cx, cy, size, 5.87, 15.4, 0.9, 0.9, false, false);
        sink.AddLine(vb(2.9, 15.4));
        add_arc(sink, cx, cy, size, 2.0, 14.5, 0.9, 0.9, false, true);
        sink.EndFigure(D2D1_FIGURE_END_CLOSED);
    }
}

fn draw_speaker_bar(
    ctx: &ID2D1DeviceContext,
    factory: &ID2D1Factory,
    cx: f32,
    cy: f32,
    size: f32,
    sw: f32,
    stroke: Option<&ID2D1StrokeStyle>,
    brush: &ID2D1Brush,
    bar: (f32, f32, f32, f32),
) {
    let (vx, y0, y1, r) = bar;
    if let Some(geo) = build_path(factory, |sink| unsafe {
        sink.BeginFigure(vb24(cx, cy, size, vx, y0), D2D1_FIGURE_BEGIN_HOLLOW);
        add_arc(sink, cx, cy, size, vx, y1, r, r, false, true);
        sink.EndFigure(D2D1_FIGURE_END_OPEN);
    }) {
        unsafe { ctx.DrawGeometry(&geo, brush, sw, stroke) };
    }
}

fn draw_speaker_mute_x(
    ctx: &ID2D1DeviceContext,
    cx: f32,
    cy: f32,
    size: f32,
    sw: f32,
    stroke: Option<&ID2D1StrokeStyle>,
    brush: &ID2D1Brush,
) {
    let vb = |vx: f32, vy: f32| vb24(cx, cy, size, vx, vy);
    unsafe {
        ctx.DrawLine(vb(14.8, 9.2), vb(20.4, 14.8), brush, sw, stroke);
        ctx.DrawLine(vb(20.4, 9.2), vb(14.8, 14.8), brush, sw, stroke);
    }
}

#[allow(dead_code)]
pub fn draw_speaker(
    ctx: &ID2D1DeviceContext,
    cx: f32,
    cy: f32,
    size: f32,
    brush: &ID2D1Brush,
    muted: bool,
) {
    unsafe { draw_speaker_two_tone(ctx, cx, cy, size, brush, brush, muted) };
}

#[allow(dead_code)]
pub unsafe fn draw_speaker_two_tone(
    ctx: &ID2D1DeviceContext,
    cx: f32,
    cy: f32,
    size: f32,
    primary_brush: &ID2D1Brush,
    outer_wave_brush: &ID2D1Brush,
    muted: bool,
) {
    if size <= 0.0 {
        return;
    }
    let Ok(factory) = ctx.GetFactory() else {
        return;
    };
    let stroke = create_round_stroke(&factory);
    let sw = (2.0 * (size / 24.0)).max(1.0);

    if let Some(cone) = build_path(&factory, |sink| add_speaker_cone_figure(sink, cx, cy, size)) {
        ctx.FillGeometry(&cone, primary_brush, None);
    }

    if muted {
        draw_speaker_mute_x(ctx, cx, cy, size, sw, stroke.as_ref(), primary_brush);
    } else {
        for (i, &bar) in SPEAKER_BARS.iter().enumerate() {
            let b_brush = if i < 2 { primary_brush } else { outer_wave_brush };
            draw_speaker_bar(ctx, &factory, cx, cy, size, sw, stroke.as_ref(), b_brush, bar);
        }
    }
}

#[allow(dead_code)]
pub fn draw_speaker_volume_bars(
    ctx: &ID2D1DeviceContext,
    cx: f32,
    cy: f32,
    size: f32,
    brush: &ID2D1Brush,
    volume: f32,
    max_bars: u8,
) {
    if size <= 0.0 {
        return;
    }
    let Ok(factory) = (unsafe { ctx.GetFactory() }) else {
        return;
    };
    let stroke = create_round_stroke(&factory);
    let sw = (2.0 * (size / 24.0)).max(1.0);

    if let Some(cone) = build_path(&factory, |sink| add_speaker_cone_figure(sink, cx, cy, size)) {
        unsafe { ctx.FillGeometry(&cone, brush, None) };
    }

    let (b1, b2, b3, mute_x) = speaker_bar_opacities_for(volume, max_bars);
    for (bar, opacity) in SPEAKER_BARS.iter().zip([b1, b2, b3]) {
        with_opacity_layer(ctx, cx, cy, size, opacity, || {
            draw_speaker_bar(ctx, &factory, cx, cy, size, sw, stroke.as_ref(), brush, *bar);
        });
    }
    with_opacity_layer(ctx, cx, cy, size, mute_x, || {
        draw_speaker_mute_x(ctx, cx, cy, size, sw, stroke.as_ref(), brush);
    });
}

#[allow(dead_code)]
pub fn draw_speaker_volume(
    ctx: &ID2D1DeviceContext,
    cx: f32,
    cy: f32,
    size: f32,
    brush: &ID2D1Brush,
    volume: f32,
) {
    draw_speaker_volume_bars(ctx, cx, cy, size, brush, volume, 3);
}

#[allow(dead_code)]
pub fn draw_caps_lock(
    ctx: &ID2D1DeviceContext,
    cx: f32,
    cy: f32,
    size: f32,
    brush: &ID2D1Brush,
    underline_brush: &ID2D1Brush,
    filled: bool,
) {
    if size <= 0.0 {
        return;
    }
    let Ok(factory) = (unsafe { ctx.GetFactory() }) else {
        return;
    };
    let stroke = create_round_stroke(&factory);
    let sw = (2.0 * (size / 24.0)).max(1.1);
    let vb = |vx: f32, vy: f32| vb24(cx, cy, size, vx, vy);

    if filled {
        if let Some(geo) = build_path(&factory, |sink| unsafe {
            sink.BeginFigure(vb(11.3, 4.0), D2D1_FIGURE_BEGIN_FILLED);
            add_quad(sink, cx, cy, size, 12.0, 3.1, 12.7, 4.0);
            sink.AddLine(vb(19.9, 11.2));
            add_quad(sink, cx, cy, size, 20.6, 11.9, 19.5, 12.0);
            sink.AddLine(vb(16.0, 12.0));
            add_quad(sink, cx, cy, size, 15.6, 12.0, 15.6, 12.4);
            sink.AddLine(vb(15.6, 15.8));
            add_quad(sink, cx, cy, size, 15.6, 16.6, 14.8, 16.6);
            sink.AddLine(vb(9.2, 16.6));
            add_quad(sink, cx, cy, size, 8.4, 16.6, 8.4, 15.8);
            sink.AddLine(vb(8.4, 12.4));
            add_quad(sink, cx, cy, size, 8.4, 12.0, 8.0, 12.0);
            sink.AddLine(vb(4.5, 12.0));
            add_quad(sink, cx, cy, size, 3.4, 11.9, 4.1, 11.2);
            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
        }) {
            unsafe { ctx.FillGeometry(&geo, brush, None) };
        }
    } else if let Some(geo) = build_path(&factory, |sink| unsafe {
        sink.BeginFigure(vb(12.0, 3.6), D2D1_FIGURE_BEGIN_HOLLOW);
        sink.AddLine(vb(19.6, 11.2));
        sink.AddLine(vb(15.6, 11.2));
        sink.AddLine(vb(15.6, 16.0));
        sink.AddLine(vb(8.4, 16.0));
        sink.AddLine(vb(8.4, 11.2));
        sink.AddLine(vb(4.4, 11.2));
        sink.EndFigure(D2D1_FIGURE_END_CLOSED);
    }) {
        unsafe { ctx.DrawGeometry(&geo, brush, sw, stroke.as_ref()) };
    }

    unsafe {
        ctx.DrawLine(
            vb(8.4, 19.6),
            vb(15.6, 19.6),
            underline_brush,
            sw,
            stroke.as_ref(),
        );
    }
}

fn draw_double_track_svg(
    ctx: &ID2D1DeviceContext,
    cx: f32,
    cy: f32,
    size: f32,
    brush: &ID2D1Brush,
    pointing_right: bool,
) {
    if size <= 0.0 {
        return;
    }
    let Ok(factory) = (unsafe { ctx.GetFactory() }) else {
        return;
    };
    let mirror_x = |vx: f32| if pointing_right { vx } else { 24.0 - vx };
    let vb = |vx: f32, vy: f32| vb24(cx, cy, size, mirror_x(vx), vy);
    let sweep_cw = pointing_right;

    if let Some(geo) = build_path(&factory, |sink| unsafe {
        for dx in [0.0_f32, 11.8_f32] {
            sink.BeginFigure(vb(0.4 + dx, 12.0), D2D1_FIGURE_BEGIN_FILLED);
            sink.AddLine(vb(0.4 + dx, 6.79));
            add_arc(sink, cx, cy, size, mirror_x(2.52 + dx), 5.68, 1.35, 1.35, false, sweep_cw);
            sink.AddLine(vb(10.01 + dx, 10.89));
            add_arc(sink, cx, cy, size, mirror_x(10.01 + dx), 13.11, 1.35, 1.35, false, sweep_cw);
            sink.AddLine(vb(2.52 + dx, 18.32));
            add_arc(sink, cx, cy, size, mirror_x(0.4 + dx), 17.21, 1.35, 1.35, false, sweep_cw);
            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
        }
    }) {
        unsafe { ctx.FillGeometry(&geo, brush, None) };
    }
}

pub fn draw_prev_track(
    ctx: &ID2D1DeviceContext,
    cx: f32,
    cy: f32,
    size: f32,
    brush: &ID2D1Brush,
) {
    draw_double_track_svg(ctx, cx, cy, size, brush, false);
}

pub fn draw_play_triangle(
    ctx: &ID2D1DeviceContext,
    cx: f32,
    cy: f32,
    size: f32,
    brush: &ID2D1Brush,
) {
    if size <= 0.0 {
        return;
    }
    let Ok(factory) = (unsafe { ctx.GetFactory() }) else {
        return;
    };
    let vb = |vx: f32, vy: f32| vb24(cx, cy, size, vx, vy);
    if let Some(geo) = build_path(&factory, |sink| unsafe {
        sink.BeginFigure(vb(4.8, 12.0), D2D1_FIGURE_BEGIN_FILLED);
        sink.AddLine(vb(4.8, 5.01));
        add_arc(sink, cx, cy, size, 7.58, 3.41, 1.85, 1.85, false, true);
        sink.AddLine(vb(19.64, 10.4));
        add_arc(sink, cx, cy, size, 19.64, 13.6, 1.85, 1.85, false, true);
        sink.AddLine(vb(7.58, 20.59));
        add_arc(sink, cx, cy, size, 4.8, 18.99, 1.85, 1.85, false, true);
        sink.EndFigure(D2D1_FIGURE_END_CLOSED);
    }) {
        unsafe { ctx.FillGeometry(&geo, brush, None) };
    }
}

pub fn draw_pause_bars(
    ctx: &ID2D1DeviceContext,
    cx: f32,
    cy: f32,
    size: f32,
    brush: &ID2D1Brush,
) {
    if size <= 0.0 {
        return;
    }
    let scale = size / 24.0;
    let ox = cx - size * 0.5;
    let oy = cy - size * 0.5;
    let r = 2.1 * scale;
    for x in [4.2_f32, 14.4_f32] {
        let rrect = D2D1_ROUNDED_RECT {
            rect: D2D_RECT_F {
                left: ox + x * scale,
                top: oy + 2.8 * scale,
                right: ox + (x + 5.4) * scale,
                bottom: oy + (2.8 + 18.4) * scale,
            },
            radiusX: r,
            radiusY: r,
        };
        unsafe { ctx.FillRoundedRectangle(&rrect, brush) };
    }
}

pub fn draw_next_track(
    ctx: &ID2D1DeviceContext,
    cx: f32,
    cy: f32,
    size: f32,
    brush: &ID2D1Brush,
) {
    draw_double_track_svg(ctx, cx, cy, size, brush, true);
}

#[allow(dead_code)]
pub fn draw_audio_bars(
    ctx: &ID2D1DeviceContext,
    cx: f32,
    cy: f32,
    size: f32,
    brush: &ID2D1Brush,
    levels: Option<[f32; 5]>,
) {
    if size <= 0.0 {
        return;
    }
    let Ok(factory) = (unsafe { ctx.GetFactory() }) else {
        return;
    };
    let stroke = create_round_stroke(&factory);
    let sw = (2.4 * (size / 24.0)).max(1.0);
    for (vx, y0, y1) in audio_bar_spans(levels) {
        unsafe {
            ctx.DrawLine(
                vb24(cx, cy, size, vx, y0),
                vb24(cx, cy, size, vx, y1),
                brush,
                sw,
                stroke.as_ref(),
            );
        }
    }
}

fn draw_chevron(
    ctx: &ID2D1DeviceContext,
    cx: f32,
    cy: f32,
    size: f32,
    brush: &ID2D1Brush,
    stroke_width: f32,
    pointing_right: bool,
) {
    if size <= 0.0 {
        return;
    }
    let Ok(factory) = (unsafe { ctx.GetFactory() }) else {
        return;
    };
    let stroke = create_round_stroke(&factory);
    let pts = chevron_points(pointing_right);
    if let Some(geo) = build_path(&factory, |sink| unsafe {
        sink.BeginFigure(pts[0].to_d2d(cx, cy, size), D2D1_FIGURE_BEGIN_HOLLOW);
        sink.AddLine(pts[1].to_d2d(cx, cy, size));
        sink.AddLine(pts[2].to_d2d(cx, cy, size));
        sink.EndFigure(D2D1_FIGURE_END_OPEN);
    }) {
        unsafe { ctx.DrawGeometry(&geo, brush, stroke_width, stroke.as_ref()) };
    }
}

#[allow(dead_code)]
pub fn draw_chevron_left(
    ctx: &ID2D1DeviceContext,
    cx: f32,
    cy: f32,
    size: f32,
    brush: &ID2D1Brush,
) {
    draw_chevron_left_stroked(ctx, cx, cy, size, brush, default_stroke(size));
}

#[allow(dead_code)]
pub fn draw_chevron_left_stroked(
    ctx: &ID2D1DeviceContext,
    cx: f32,
    cy: f32,
    size: f32,
    brush: &ID2D1Brush,
    stroke_width: f32,
) {
    draw_chevron(ctx, cx, cy, size, brush, stroke_width, false);
}

#[allow(dead_code)]
pub fn draw_chevron_right(
    ctx: &ID2D1DeviceContext,
    cx: f32,
    cy: f32,
    size: f32,
    brush: &ID2D1Brush,
) {
    draw_chevron_right_stroked(ctx, cx, cy, size, brush, default_stroke(size));
}

#[allow(dead_code)]
pub fn draw_chevron_right_stroked(
    ctx: &ID2D1DeviceContext,
    cx: f32,
    cy: f32,
    size: f32,
    brush: &ID2D1Brush,
    stroke_width: f32,
) {
    draw_chevron(ctx, cx, cy, size, brush, stroke_width, true);
}

#[allow(dead_code)]
pub fn draw_clock_glyph(
    ctx: &ID2D1DeviceContext,
    cx: f32,
    cy: f32,
    size: f32,
    brush: &ID2D1Brush,
) {
    draw_clock_glyph_stroked(ctx, cx, cy, size, brush, default_stroke(size));
}

#[allow(dead_code)]
pub fn draw_clock_glyph_stroked(
    ctx: &ID2D1DeviceContext,
    cx: f32,
    cy: f32,
    size: f32,
    brush: &ID2D1Brush,
    stroke_width: f32,
) {
    if size <= 0.0 {
        return;
    }
    let Ok(factory) = (unsafe { ctx.GetFactory() }) else {
        return;
    };
    let stroke = create_round_stroke(&factory);
    let r = 0.36 * size;
    unsafe {
        ctx.DrawEllipse(
            &D2D1_ELLIPSE {
                point: D2D_POINT_2F { x: cx, y: cy },
                radiusX: r,
                radiusY: r,
            },
            brush,
            stroke_width,
            stroke.as_ref(),
        );
        ctx.DrawLine(
            D2D_POINT_2F { x: cx, y: cy },
            pt(cx, cy, size, 0.0, -0.20),
            brush,
            stroke_width,
            stroke.as_ref(),
        );
        ctx.DrawLine(
            D2D_POINT_2F { x: cx, y: cy },
            pt(cx, cy, size, 0.16, 0.09),
            brush,
            stroke_width,
            stroke.as_ref(),
        );
    }
}

#[allow(dead_code)]
pub fn draw_mascot_eyes(
    ctx: &ID2D1DeviceContext,
    cx: f32,
    cy: f32,
    size: f32,
    brush: &ID2D1Brush,
) {
    draw_mascot_eyes_animated(ctx, cx, cy, size, brush, 0.0, 0.0, false);
}

#[allow(dead_code)]
pub fn draw_mascot_eyes_animated(
    ctx: &ID2D1DeviceContext,
    cx: f32,
    cy: f32,
    size: f32,
    brush: &ID2D1Brush,
    look_x: f32,
    blink: f32,
    happy: bool,
) {
    if size <= 0.0 {
        return;
    }
    let eye_dx = 0.24 * size;
    let eye_w = (if happy { 0.17 } else { 0.15 }) * size;
    let eye_h = (0.28 * size * (1.0 - blink.clamp(0.0, 1.0) * 0.85)).max(1.2);
    let gx = look_x.clamp(-1.0, 1.0) * 0.09 * size;

    for side in [-1.0_f32, 1.0_f32] {
        let ex = cx + side * eye_dx + gx;
        let rrect = D2D1_ROUNDED_RECT {
            rect: D2D_RECT_F {
                left: ex - eye_w * 0.5,
                top: cy - eye_h * 0.5,
                right: ex + eye_w * 0.5,
                bottom: cy + eye_h * 0.5,
            },
            radiusX: eye_w * 0.5,
            radiusY: (eye_h * 0.5).min(eye_w * 0.65),
        };
        unsafe { ctx.FillRoundedRectangle(&rrect, brush) };
    }
}

#[allow(dead_code)]
pub fn draw_reset_ccw(
    ctx: &ID2D1DeviceContext,
    cx: f32,
    cy: f32,
    size: f32,
    brush: &ID2D1Brush,
) {
    if size <= 0.0 {
        return;
    }
    let Ok(factory) = (unsafe { ctx.GetFactory() }) else {
        return;
    };
    let stroke = create_round_stroke(&factory);
    let sw = (size * (2.4 / 24.0)).max(1.25);
    let vb = |vx: f32, vy: f32| -> D2D_POINT_2F {
        pt(cx, cy, size, (vx - 12.0) / 24.0, (vy - 12.0) / 24.0)
    };
    let r = (7.0 / 24.0) * size;

    if let Some(geo) = build_path(&factory, |sink| unsafe {
        sink.BeginFigure(vb(5.0, 12.0), D2D1_FIGURE_BEGIN_HOLLOW);
        sink.AddArc(&D2D1_ARC_SEGMENT {
            point: vb(7.0, 7.0),
            size: windows::Win32::Graphics::Direct2D::Common::D2D_SIZE_F {
                width: r,
                height: r,
            },
            rotationAngle: 0.0,
            sweepDirection: D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE,
            arcSize: D2D1_ARC_SIZE_LARGE,
        });
        sink.EndFigure(D2D1_FIGURE_END_OPEN);

        sink.BeginFigure(vb(4.0, 3.5), D2D1_FIGURE_BEGIN_HOLLOW);
        sink.AddLine(vb(4.0, 8.0));
        sink.AddLine(vb(8.5, 8.0));
        sink.EndFigure(D2D1_FIGURE_END_OPEN);
    }) {
        unsafe { ctx.DrawGeometry(&geo, brush, sw, stroke.as_ref()) };
    }
}

#[allow(dead_code)]
pub fn draw_sliders_h(
    ctx: &ID2D1DeviceContext,
    cx: f32,
    cy: f32,
    size: f32,
    brush: &ID2D1Brush,
) {
    if size <= 0.0 {
        return;
    }
    let Ok(factory) = (unsafe { ctx.GetFactory() }) else {
        return;
    };
    let stroke = create_round_stroke(&factory);
    let sw = (size * (2.0 / 24.0)).max(1.2);
    let vb = |vx: f32, vy: f32| -> D2D_POINT_2F {
        pt(cx, cy, size, (vx - 12.0) / 24.0, (vy - 12.0) / 24.0)
    };
    let knob_r = (2.1 / 24.0) * size;

    unsafe {
        ctx.DrawLine(vb(4.0, 8.0), vb(6.9, 8.0), brush, sw, stroke.as_ref());
        ctx.DrawLine(vb(11.1, 8.0), vb(20.0, 8.0), brush, sw, stroke.as_ref());
        ctx.DrawEllipse(
            &D2D1_ELLIPSE {
                point: vb(9.0, 8.0),
                radiusX: knob_r,
                radiusY: knob_r,
            },
            brush,
            sw,
            stroke.as_ref(),
        );

        ctx.DrawLine(vb(4.0, 16.0), vb(12.9, 16.0), brush, sw, stroke.as_ref());
        ctx.DrawLine(vb(17.1, 16.0), vb(20.0, 16.0), brush, sw, stroke.as_ref());
        ctx.DrawEllipse(
            &D2D1_ELLIPSE {
                point: vb(15.0, 16.0),
                radiusX: knob_r,
                radiusY: knob_r,
            },
            brush,
            sw,
            stroke.as_ref(),
        );
    }
}

#[allow(dead_code)]
pub fn draw_plus_circle_filled(
    ctx: &ID2D1DeviceContext,
    cx: f32,
    cy: f32,
    size: f32,
    brush: &ID2D1Brush,
) {
    if size <= 0.0 {
        return;
    }
    let Ok(factory) = (unsafe { ctx.GetFactory() }) else {
        return;
    };
    let vb = |vx: f32, vy: f32| -> D2D_POINT_2F {
        pt(cx, cy, size, (vx - 12.0) / 24.0, (vy - 12.0) / 24.0)
    };

    if let Some(geo) = build_path(&factory, |sink| unsafe {
        sink.SetFillMode(D2D1_FILL_MODE_ALTERNATE);

        const K: f32 = 0.552_284_8;
        let r = 10.0_f32;
        let rk = r * K;
        sink.BeginFigure(vb(12.0 + r, 12.0), D2D1_FIGURE_BEGIN_FILLED);
        add_cubic(sink, vb(12.0 + r, 12.0 + rk), vb(12.0 + rk, 12.0 + r), vb(12.0, 12.0 + r));
        add_cubic(sink, vb(12.0 - rk, 12.0 + r), vb(12.0 - r, 12.0 + rk), vb(12.0 - r, 12.0));
        add_cubic(sink, vb(12.0 - r, 12.0 - rk), vb(12.0 - rk, 12.0 - r), vb(12.0, 12.0 - r));
        add_cubic(sink, vb(12.0 + rk, 12.0 - r), vb(12.0 + r, 12.0 - rk), vb(12.0 + r, 12.0));
        sink.EndFigure(D2D1_FIGURE_END_CLOSED);

        sink.BeginFigure(vb(11.0, 7.0), D2D1_FIGURE_BEGIN_FILLED);
        sink.AddLine(vb(13.0, 7.0));
        sink.AddLine(vb(13.0, 11.0));
        sink.AddLine(vb(17.0, 11.0));
        sink.AddLine(vb(17.0, 13.0));
        sink.AddLine(vb(13.0, 13.0));
        sink.AddLine(vb(13.0, 17.0));
        sink.AddLine(vb(11.0, 17.0));
        sink.AddLine(vb(11.0, 13.0));
        sink.AddLine(vb(7.0, 13.0));
        sink.AddLine(vb(7.0, 11.0));
        sink.AddLine(vb(11.0, 11.0));
        sink.EndFigure(D2D1_FIGURE_END_CLOSED);
    }) {
        unsafe { ctx.FillGeometry(&geo, brush, None) };
    }
}

#[allow(dead_code)]
pub fn draw_pencil_edit(
    ctx: &ID2D1DeviceContext,
    cx: f32,
    cy: f32,
    size: f32,
    brush: &ID2D1Brush,
) {
    if size <= 0.0 {
        return;
    }
    let Ok(factory) = (unsafe { ctx.GetFactory() }) else {
        return;
    };
    let stroke = create_round_stroke(&factory);
    let sw = (size * (2.0 / 24.0)).max(1.25);
    let vb = |vx: f32, vy: f32| vb24(cx, cy, size, vx, vy);

    if let Some(geo) = build_path(&factory, |sink| unsafe {
        sink.BeginFigure(pt(cx, cy, size, -0.34, 0.36), D2D1_FIGURE_BEGIN_HOLLOW);
        sink.AddLine(pt(cx, cy, size, 0.34, 0.36));
        sink.EndFigure(D2D1_FIGURE_END_OPEN);

        sink.BeginFigure(vb(14.5, 4.5), D2D1_FIGURE_BEGIN_HOLLOW);
        sink.AddLine(vb(19.5, 9.5));
        sink.AddLine(vb(10.0, 19.0));
        sink.AddLine(vb(5.0, 19.0));
        sink.AddLine(vb(5.0, 14.0));
        sink.EndFigure(D2D1_FIGURE_END_CLOSED);
    }) {
        unsafe { ctx.DrawGeometry(&geo, brush, sw, stroke.as_ref()) };
    }
}

#[allow(dead_code)]
pub unsafe fn draw_calendar_badge_icon(
    ctx: &ID2D1DeviceContext,
    cx: f32,
    cy: f32,
    size: f32,
    brush: &ID2D1Brush,
    stroke_w: f32,
) {
    if size <= 0.0 {
        return;
    }
    let Ok(factory) = ctx.GetFactory() else {
        return;
    };
    let stroke = create_round_stroke(&factory);
    let sw = stroke_w.max(1.2);
    let half_w = size * 0.39;
    let top = cy - size * 0.28;
    let bottom = cy + size * 0.42;
    let rx = size * 0.16;

    let frame = D2D1_ROUNDED_RECT {
        rect: D2D_RECT_F {
            left: cx - half_w,
            top,
            right: cx + half_w,
            bottom,
        },
        radiusX: rx,
        radiusY: rx,
    };
    ctx.DrawRoundedRectangle(&frame, brush, sw, stroke.as_ref());

    let divider_y = top + size * 0.25;
    ctx.DrawLine(
        D2D_POINT_2F {
            x: cx - half_w,
            y: divider_y,
        },
        D2D_POINT_2F {
            x: cx + half_w,
            y: divider_y,
        },
        brush,
        sw,
        stroke.as_ref(),
    );

    for px in [cx - size * 0.18, cx + size * 0.18] {
        ctx.DrawLine(
            D2D_POINT_2F {
                x: px,
                y: top - size * 0.14,
            },
            D2D_POINT_2F {
                x: px,
                y: top + size * 0.04,
            },
            brush,
            sw,
            stroke.as_ref(),
        );
    }
}

#[allow(dead_code)]
pub unsafe fn draw_no_tasks_badge(
    ctx: &ID2D1DeviceContext,
    cx: f32,
    cy: f32,
    size: f32,
    badge_brush: &ID2D1Brush,
    cutout_brush: &ID2D1Brush,
) {
    if size <= 0.0 {
        return;
    }
    let Ok(factory) = ctx.GetFactory() else {
        return;
    };
    let stroke = create_round_stroke(&factory);
    let sw = (size * 0.095).max(1.4);

    let half_w = size * 0.50;
    let half_h = size * 0.47;
    let rx = size * 0.25;
    let badge = D2D1_ROUNDED_RECT {
        rect: D2D_RECT_F {
            left: cx - half_w,
            top: cy - half_h,
            right: cx + half_w,
            bottom: cy + half_h,
        },
        radiusX: rx,
        radiusY: rx,
    };
    ctx.FillRoundedRectangle(&badge, badge_brush);

    if let Some(geo) = build_path(&factory, |sink| {
        for row_y in [-0.14_f32, 0.14_f32] {
            sink.BeginFigure(pt(cx, cy, size, -0.27, row_y + 0.01), D2D1_FIGURE_BEGIN_HOLLOW);
            sink.AddLine(pt(cx, cy, size, -0.19, row_y + 0.075));
            sink.AddLine(pt(cx, cy, size, -0.075, row_y - 0.06));
            sink.EndFigure(D2D1_FIGURE_END_OPEN);

            sink.BeginFigure(pt(cx, cy, size, 0.03, row_y), D2D1_FIGURE_BEGIN_HOLLOW);
            sink.AddLine(pt(cx, cy, size, 0.27, row_y));
            sink.EndFigure(D2D1_FIGURE_END_OPEN);
        }
    }) {
        ctx.DrawGeometry(&geo, cutout_brush, sw, stroke.as_ref());
    }
}

#[allow(dead_code)]
pub fn draw_checkmark(
    ctx: &ID2D1DeviceContext,
    cx: f32,
    cy: f32,
    size: f32,
    brush: &ID2D1Brush,
    stroke_width: f32,
) {
    if size <= 0.0 {
        return;
    }
    let Ok(factory) = (unsafe { ctx.GetFactory() }) else {
        return;
    };
    let stroke = create_round_stroke(&factory);
    let vb = |vx: f32, vy: f32| -> D2D_POINT_2F {
        pt(cx, cy, size, (vx - 5.0) / 10.0, (vy - 5.0) / 10.0)
    };

    if let Some(geo) = build_path(&factory, |sink| unsafe {
        sink.BeginFigure(vb(2.0, 5.2), D2D1_FIGURE_BEGIN_HOLLOW);
        sink.AddLine(vb(4.0, 7.2));
        sink.AddLine(vb(8.0, 2.8));
        sink.EndFigure(D2D1_FIGURE_END_OPEN);
    }) {
        unsafe { ctx.DrawGeometry(&geo, brush, stroke_width, stroke.as_ref()) };
    }
}

pub fn draw_checkmark_animated(
    ctx: &ID2D1DeviceContext,
    cx: f32,
    cy: f32,
    size: f32,
    brush: &ID2D1Brush,
    stroke_width: f32,
    progress: f32,
) {
    let p = progress.clamp(0.0, 1.0);
    if size <= 0.0 || p <= 0.01 {
        return;
    }
    let Ok(factory) = (unsafe { ctx.GetFactory() }) else {
        return;
    };
    let stroke = create_round_stroke(&factory);
    let vb = |vx: f32, vy: f32| -> D2D_POINT_2F {
        pt(cx, cy, size, (vx - 5.0) / 10.0, (vy - 5.0) / 10.0)
    };
    let p0 = vb(2.0, 5.2);
    let p1 = vb(4.0, 7.2);
    let p2 = vb(8.0, 2.8);

    if let Some(geo) = build_path(&factory, |sink| unsafe {
        sink.BeginFigure(p0, D2D1_FIGURE_BEGIN_HOLLOW);
        if p <= 0.40 {
            let t = p / 0.40;
            sink.AddLine(D2D_POINT_2F {
                x: p0.x + (p1.x - p0.x) * t,
                y: p0.y + (p1.y - p0.y) * t,
            });
        } else {
            let t = (p - 0.40) / 0.60;
            sink.AddLine(p1);
            sink.AddLine(D2D_POINT_2F {
                x: p1.x + (p2.x - p1.x) * t,
                y: p1.y + (p2.y - p1.y) * t,
            });
        }
        sink.EndFigure(D2D1_FIGURE_END_OPEN);
    }) {
        unsafe { ctx.DrawGeometry(&geo, brush, stroke_width, stroke.as_ref()) };
    }
}

#[allow(dead_code)]
pub fn draw_folder_thumb(ctx: &ID2D1DeviceContext, rect: D2D_RECT_F, alpha: f32) {
    let w = rect.right - rect.left;
    let h = rect.bottom - rect.top;
    if w <= 0.0 || h <= 0.0 || alpha <= 0.005 {
        return;
    }
    let Ok(factory) = (unsafe { ctx.GetFactory() }) else {
        return;
    };
    let vb = |vx: f32, vy: f32| -> D2D_POINT_2F {
        D2D_POINT_2F {
            x: rect.left + (vx / 44.0) * w,
            y: rect.top + (vy / 44.0) * h,
        }
    };

    let Ok(back_brush) = (unsafe {
        ctx.CreateSolidColorBrush(
            &D2D1_COLOR_F {
                r: 0.910,
                g: 0.655,
                b: 0.125,
                a: alpha,
            },
            None,
        )
    }) else {
        return;
    };
    let Ok(front_brush) = (unsafe {
        ctx.CreateSolidColorBrush(
            &D2D1_COLOR_F {
                r: 1.0,
                g: 0.788,
                b: 0.302,
                a: alpha,
            },
            None,
        )
    }) else {
        return;
    };

    if let Some(back_geo) = build_path(&factory, |sink| unsafe {
        sink.BeginFigure(vb(4.0, 11.0), D2D1_FIGURE_BEGIN_FILLED);
        add_cubic(sink, vb(4.0, 9.3), vb(5.3, 8.0), vb(7.0, 8.0));
        sink.AddLine(vb(17.0, 8.0));
        sink.AddLine(vb(21.0, 12.0));
        sink.AddLine(vb(37.0, 12.0));
        add_cubic(sink, vb(38.7, 12.0), vb(40.0, 13.3), vb(40.0, 15.0));
        sink.AddLine(vb(40.0, 34.0));
        add_cubic(sink, vb(40.0, 35.7), vb(38.7, 37.0), vb(37.0, 37.0));
        sink.AddLine(vb(7.0, 37.0));
        add_cubic(sink, vb(5.3, 37.0), vb(4.0, 35.7), vb(4.0, 34.0));
        sink.EndFigure(D2D1_FIGURE_END_CLOSED);
    }) {
        unsafe { ctx.FillGeometry(&back_geo, &back_brush, None) };
    }

    if let Some(front_geo) = build_path(&factory, |sink| unsafe {
        sink.BeginFigure(vb(4.0, 16.0), D2D1_FIGURE_BEGIN_FILLED);
        sink.AddLine(vb(40.0, 16.0));
        sink.AddLine(vb(40.0, 34.0));
        add_cubic(sink, vb(40.0, 35.7), vb(38.7, 37.0), vb(37.0, 37.0));
        sink.AddLine(vb(7.0, 37.0));
        add_cubic(sink, vb(5.3, 37.0), vb(4.0, 35.7), vb(4.0, 34.0));
        sink.EndFigure(D2D1_FIGURE_END_CLOSED);
    }) {
        unsafe { ctx.FillGeometry(&front_geo, &front_brush, None) };
    }
}

#[allow(dead_code)]
pub fn draw_file_thumb(
    ctx: &ID2D1DeviceContext,
    rect: D2D_RECT_F,
    is_image: bool,
    alpha: f32,
) {
    let w = rect.right - rect.left;
    let h = rect.bottom - rect.top;
    if w <= 0.0 || h <= 0.0 || alpha <= 0.005 {
        return;
    }
    let Ok(factory) = (unsafe { ctx.GetFactory() }) else {
        return;
    };
    let rx = (w.min(h) * (9.0 / 44.0)).max(4.0);

    if is_image {
        let Ok(clip_geo) = (unsafe {
            factory.CreateRoundedRectangleGeometry(&D2D1_ROUNDED_RECT {
                rect,
                radiusX: rx,
                radiusY: rx,
            })
        }) else {
            return;
        };

        let mut layer_params = D2D1_LAYER_PARAMETERS1 {
            contentBounds: rect,
            geometricMask: ManuallyDrop::new(Some(clip_geo.into())),
            maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
            maskTransform: Matrix3x2::identity(),
            opacity: alpha,
            opacityBrush: ManuallyDrop::new(None),
            layerOptions: D2D1_LAYER_OPTIONS1_NONE,
        };

        unsafe { ctx.PushLayer(&layer_params, None) };

        if let Ok(brush) = unsafe {
            ctx.CreateSolidColorBrush(
                &D2D1_COLOR_F {
                    r: 0.337,
                    g: 0.761,
                    b: 1.0,
                    a: 1.0,
                },
                None,
            )
        } {
            unsafe {
                ctx.FillRectangle(
                    &D2D_RECT_F {
                        left: rect.left,
                        top: rect.top,
                        right: rect.right,
                        bottom: rect.top + h * 0.56,
                    },
                    &brush,
                );
                brush.SetColor(&D2D1_COLOR_F {
                    r: 0.953,
                    g: 0.827,
                    b: 0.561,
                    a: 1.0,
                });
                ctx.FillRectangle(
                    &D2D_RECT_F {
                        left: rect.left,
                        top: rect.top + h * 0.55,
                        right: rect.right,
                        bottom: rect.top + h * 0.71,
                    },
                    &brush,
                );
                brush.SetColor(&D2D1_COLOR_F {
                    r: 0.180,
                    g: 0.608,
                    b: 0.839,
                    a: 1.0,
                });
                ctx.FillRectangle(
                    &D2D_RECT_F {
                        left: rect.left,
                        top: rect.top + h * 0.70,
                        right: rect.right,
                        bottom: rect.bottom,
                    },
                    &brush,
                );
                brush.SetColor(&D2D1_COLOR_F {
                    r: 1.0,
                    g: 0.953,
                    b: 0.769,
                    a: 1.0,
                });
                let sun_r = w.min(h) * 0.13;
                ctx.FillEllipse(
                    &D2D1_ELLIPSE {
                        point: D2D_POINT_2F {
                            x: rect.left + w * 0.72,
                            y: rect.top + h * 0.30,
                        },
                        radiusX: sun_r,
                        radiusY: sun_r,
                    },
                    &brush,
                );
            }
        }

        unsafe {
            ctx.PopLayer();
            ManuallyDrop::drop(&mut layer_params.geometricMask);
        }
    } else {
        let Ok(brush) = (unsafe {
            ctx.CreateSolidColorBrush(
                &D2D1_COLOR_F {
                    r: 0.145,
                    g: 0.165,
                    b: 0.215,
                    a: alpha,
                },
                None,
            )
        }) else {
            return;
        };
        let vb = |vx: f32, vy: f32| -> D2D_POINT_2F {
            D2D_POINT_2F {
                x: rect.left + (vx / 44.0) * w,
                y: rect.top + (vy / 44.0) * h,
            }
        };

        if let Some(doc_geo) = build_path(&factory, |sink| unsafe {
            sink.BeginFigure(vb(10.0, 5.0), D2D1_FIGURE_BEGIN_FILLED);
            sink.AddLine(vb(27.0, 5.0));
            sink.AddLine(vb(36.0, 14.0));
            sink.AddLine(vb(36.0, 36.0));
            add_cubic(sink, vb(36.0, 37.8), vb(34.8, 39.0), vb(33.0, 39.0));
            sink.AddLine(vb(10.0, 39.0));
            add_cubic(sink, vb(8.2, 39.0), vb(7.0, 37.8), vb(7.0, 36.0));
            sink.AddLine(vb(7.0, 8.0));
            add_cubic(sink, vb(7.0, 6.2), vb(8.2, 5.0), vb(10.0, 5.0));
            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
        }) {
            unsafe {
                ctx.FillGeometry(&doc_geo, &brush, None);
                brush.SetColor(&D2D1_COLOR_F {
                    r: 0.28,
                    g: 0.32,
                    b: 0.42,
                    a: alpha,
                });
                ctx.DrawGeometry(&doc_geo, &brush, (w * 0.025).max(1.0), None);
            }
        }

        if let Some(fold_geo) = build_path(&factory, |sink| unsafe {
            sink.BeginFigure(vb(27.0, 5.0), D2D1_FIGURE_BEGIN_FILLED);
            sink.AddLine(vb(36.0, 14.0));
            sink.AddLine(vb(27.0, 14.0));
            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
        }) {
            unsafe {
                brush.SetColor(&D2D1_COLOR_F {
                    r: 0.25,
                    g: 0.29,
                    b: 0.39,
                    a: alpha,
                });
                ctx.FillGeometry(&fold_geo, &brush, None);
            }
        }

        let stroke = create_round_stroke(&factory);
        let sw = (w * 0.045).max(1.4);
        unsafe {
            brush.SetColor(&D2D1_COLOR_F {
                r: 0.56,
                g: 0.62,
                b: 0.74,
                a: 0.9 * alpha,
            });
            ctx.DrawLine(vb(12.5, 20.0), vb(30.5, 20.0), &brush, sw, stroke.as_ref());
            ctx.DrawLine(vb(12.5, 25.5), vb(30.5, 25.5), &brush, sw, stroke.as_ref());
            ctx.DrawLine(vb(12.5, 31.0), vb(23.5, 31.0), &brush, sw, stroke.as_ref());
        }
    }
}

#[inline]
pub(crate) fn text_lines_bar_segments(left: f32, top: f32, size: f32) -> (f32, [(Vec2, Vec2); 3]) {
    let sw = (size * 0.14).max(1.4);
    let r = sw * 0.5;
    let x0 = left + r;
    let y0 = top + r;
    let bars = [
        (
            Vec2::new(x0, y0),
            Vec2::new(left + (size - r).max(r), y0),
        ),
        (
            Vec2::new(x0, y0 + 0.38 * size),
            Vec2::new(left + (size * 0.82 - r).max(r), y0 + 0.38 * size),
        ),
        (
            Vec2::new(x0, y0 + 0.76 * size),
            Vec2::new(left + (size * 0.58 - r).max(r), y0 + 0.76 * size),
        ),
    ];
    (sw, bars)
}

#[allow(dead_code)]
pub unsafe fn draw_folder_outline(
    ctx: &ID2D1DeviceContext,
    cx: f32,
    cy: f32,
    size: f32,
    brush: &ID2D1Brush,
    stroke_w: f32,
) {
    if size <= 0.0 {
        return;
    }
    let Ok(factory) = ctx.GetFactory() else {
        return;
    };
    let stroke = create_round_stroke(&factory);
    let sw = stroke_w.max(1.1);

    let left = -0.46_f32;
    let right = 0.46_f32;
    let top = -0.36_f32;
    let bottom = 0.36_f32;
    let tab_end_x = left + 0.92 * 0.42;
    let shoulder_y = top + 0.72 * 0.22;
    let r = 0.10_f32;

    if let Some(geo) = build_path(&factory, |sink| {
        sink.BeginFigure(pt(cx, cy, size, left + r, top), D2D1_FIGURE_BEGIN_HOLLOW);
        sink.AddLine(pt(cx, cy, size, tab_end_x - 0.04, top));
        sink.AddLine(pt(cx, cy, size, tab_end_x + 0.09, shoulder_y));
        sink.AddLine(pt(cx, cy, size, right - r, shoulder_y));
        add_cubic(
            sink,
            pt(cx, cy, size, right - r * 0.35, shoulder_y),
            pt(cx, cy, size, right, shoulder_y + r * 0.35),
            pt(cx, cy, size, right, shoulder_y + r),
        );
        sink.AddLine(pt(cx, cy, size, right, bottom - r));
        add_cubic(
            sink,
            pt(cx, cy, size, right, bottom - r * 0.35),
            pt(cx, cy, size, right - r * 0.35, bottom),
            pt(cx, cy, size, right - r, bottom),
        );
        sink.AddLine(pt(cx, cy, size, left + r, bottom));
        add_cubic(
            sink,
            pt(cx, cy, size, left + r * 0.35, bottom),
            pt(cx, cy, size, left, bottom - r * 0.35),
            pt(cx, cy, size, left, bottom - r),
        );
        sink.AddLine(pt(cx, cy, size, left, top + r));
        add_cubic(
            sink,
            pt(cx, cy, size, left, top + r * 0.35),
            pt(cx, cy, size, left + r * 0.35, top),
            pt(cx, cy, size, left + r, top),
        );
        sink.EndFigure(D2D1_FIGURE_END_CLOSED);
    }) {
        ctx.DrawGeometry(&geo, brush, sw, stroke.as_ref());
    }
}

#[allow(dead_code)]
pub unsafe fn draw_text_lines_icon(
    ctx: &ID2D1DeviceContext,
    left: f32,
    top: f32,
    size: f32,
    brush: &ID2D1Brush,
) {
    if size <= 0.0 {
        return;
    }
    let Ok(factory) = ctx.GetFactory() else {
        return;
    };
    let stroke = create_round_stroke(&factory);
    let (sw, bars) = text_lines_bar_segments(left, top, size);

    for (a, b) in bars {
        ctx.DrawLine(
            D2D_POINT_2F { x: a.x, y: a.y },
            D2D_POINT_2F { x: b.x, y: b.y },
            brush,
            sw,
            stroke.as_ref(),
        );
    }
}

pub unsafe fn draw_widgets_grid(
    ctx: &ID2D1DeviceContext,
    cx: f32,
    cy: f32,
    size: f32,
    brush: &ID2D1Brush,
) {
    if size <= 0.0 {
        return;
    }
    let gap = size * 0.14;
    let cell = (size * 0.78 - gap) * 0.5;
    let rad = cell * 0.30;
    let half = cell + gap * 0.5;
    for &(dx, dy) in &[(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
        let x0 = cx + if dx < 0.0 { -half } else { gap * 0.5 };
        let y0 = cy + if dy < 0.0 { -half } else { gap * 0.5 };
        ctx.FillRoundedRectangle(
            &D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F {
                    left: x0,
                    top: y0,
                    right: x0 + cell,
                    bottom: y0 + cell,
                },
                radiusX: rad,
                radiusY: rad,
            },
            brush,
        );
    }
}

pub unsafe fn draw_bluetooth_icon(
    ctx: &ID2D1DeviceContext,
    cx: f32,
    cy: f32,
    size: f32,
    brush: &ID2D1Brush,
    stroke_w: f32,
) {
    if size <= 0.0 {
        return;
    }
    let Ok(factory) = ctx.GetFactory() else {
        return;
    };
    let stroke = create_round_stroke(&factory);
    let s = size / 24.0;
    let pt = |x: f32, y: f32| -> D2D_POINT_2F {
        D2D_POINT_2F {
            x: cx + (x - 12.0) * s,
            y: cy + (y - 12.0) * s,
        }
    };
    if let Ok(geo) = factory.CreatePathGeometry() {
        if let Ok(sink) = geo.Open() {
            sink.BeginFigure(pt(6.5, 6.5), D2D1_FIGURE_BEGIN_HOLLOW);
            sink.AddLine(pt(17.5, 16.8));
            sink.AddLine(pt(11.8, 22.0));
            sink.AddLine(pt(11.8, 2.0));
            sink.AddLine(pt(17.5, 7.2));
            sink.AddLine(pt(6.5, 17.5));
            sink.EndFigure(D2D1_FIGURE_END_OPEN);
            if sink.Close().is_ok() {
                ctx.DrawGeometry(&geo, brush, stroke_w, stroke.as_ref());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gear_outline_has_expected_vertex_count_and_bounds() {
        let pts = gear_outline_points(8, 0.38, 0.27, 0.115, 0.22);
        assert_eq!(pts.len(), 32);
        for p in &pts {
            let r = (p.x * p.x + p.y * p.y).sqrt();
            assert!((r - 0.38).abs() < 1e-4 || (r - 0.27).abs() < 1e-4);
            assert!(p.x.abs() <= 0.40 && p.y.abs() <= 0.40);
        }
    }

    #[test]
    fn chevron_points_are_symmetric_and_oriented() {
        let left = chevron_points(false);
        let right = chevron_points(true);
        assert!(left[1].x < left[0].x);
        assert!(right[1].x > right[0].x);
        assert!((left[0].y + left[2].y).abs() < 1e-6);
        assert!((right[0].y + right[2].y).abs() < 1e-6);
        assert!((left[1].x + right[1].x).abs() < 1e-6);
    }

    #[test]
    fn speaker_bar_opacities_fade_across_thirds_and_mute_at_zero() {
        let (b1, b2, b3, mx) = speaker_bar_opacities(0.0);
        assert_eq!((b1, b2, b3, mx), (0.0, 0.0, 0.0, 1.0));

        let (b1, b2, b3, mx) = speaker_bar_opacities(16.6665);
        assert!((b1 - 0.5).abs() < 1e-3);
        assert_eq!((b2, b3, mx), (0.0, 0.0, 0.0));

        let (b1, b2, b3, mx) = speaker_bar_opacities(50.0);
        assert_eq!(b1, 1.0);
        assert!((b2 - 0.5).abs() < 1e-3);
        assert_eq!((b3, mx), (0.0, 0.0));

        let (b1, b2, b3, mx) = speaker_bar_opacities(100.0);
        assert_eq!((b1, b2, b3, mx), (1.0, 1.0, 1.0, 0.0));
    }

    #[test]
    fn speaker_bar_opacities_two_arc_mode_caps_at_two_bars() {
        assert_eq!(speaker_bar_opacities_for(0.0, 2), (0.0, 0.0, 0.0, 1.0));
        let (b1, b2, b3, mx) = speaker_bar_opacities_for(17.5, 2);
        assert!((b1 - 0.5).abs() < 1e-6);
        assert_eq!((b2, b3, mx), (0.0, 0.0, 0.0));

        let (b1, b2, b3, mx) = speaker_bar_opacities_for(52.5, 2);
        assert_eq!(b1, 1.0);
        assert!((b2 - 0.5).abs() < 1e-6);
        assert_eq!((b3, mx), (0.0, 0.0));

        assert_eq!(speaker_bar_opacities_for(70.0, 2), (1.0, 1.0, 0.0, 0.0));
        assert_eq!(speaker_bar_opacities_for(100.0, 2), (1.0, 1.0, 0.0, 0.0));
    }

    #[test]
    fn audio_bar_spans_default_and_dynamic_levels() {
        let def = audio_bar_spans(None);
        assert_eq!(def[0], (4.0, 10.4, 13.6));
        assert_eq!(def[1], (8.0, 9.4, 14.6));
        assert_eq!(def[2], (12.0, 8.2, 15.8));
        assert_eq!(def[3], (16.0, 9.6, 14.4));
        assert_eq!(def[4], (20.0, 10.4, 13.6));

        let dyn_spans = audio_bar_spans(Some([0.0, 0.25, 0.5, 0.75, 1.0]));
        for (i, (x, y0, y1)) in dyn_spans.iter().enumerate() {
            assert_eq!(*x, 4.0 + i as f32 * 4.0);
            assert!((y0 + y1 - 24.0).abs() < 1e-5);
            assert!(y1 > y0);
        }
        assert!(dyn_spans[4].2 - dyn_spans[4].1 > dyn_spans[0].2 - dyn_spans[0].1);
    }

    #[test]
    fn viewbox_24_mapping_centers_origin_and_bounds() {
        let center = viewbox_24_to_norm(12.0, 12.0);
        assert!(center.x.abs() < 1e-6 && center.y.abs() < 1e-6);
        let tl = viewbox_24_to_norm(0.0, 0.0);
        let br = viewbox_24_to_norm(24.0, 24.0);
        assert!((tl.x + 0.5).abs() < 1e-6 && (tl.y + 0.5).abs() < 1e-6);
        assert!((br.x - 0.5).abs() < 1e-6 && (br.y - 0.5).abs() < 1e-6);
        let scaled = vb24(100.0, 50.0, 24.0, 4.2, 2.8);
        assert!((scaled.x - (88.0 + 4.2)).abs() < 1e-5);
        assert!((scaled.y - (38.0 + 2.8)).abs() < 1e-5);
    }

    #[test]
    fn vec2_scaling_respects_center_and_size() {
        let p = Vec2::new(0.25, -0.5).to_d2d(100.0, 40.0, 20.0);
        assert!((p.x - 105.0).abs() < 1e-6);
        assert!((p.y - 30.0).abs() < 1e-6);
    }

    #[test]
    fn text_lines_icon_staggers_widths_and_vertical_spacing() {
        let (sw, bars) = text_lines_bar_segments(10.0, 20.0, 12.0);
        assert!((sw - 1.68).abs() < 1e-3);
        let len0 = bars[0].1.x - bars[0].0.x;
        let len1 = bars[1].1.x - bars[1].0.x;
        let len2 = bars[2].1.x - bars[2].0.x;
        assert!(len0 > len1 && len1 > len2 && len2 > 0.0);
        assert!((bars[1].0.y - bars[0].0.y - 0.38 * 12.0).abs() < 1e-4);
        assert!((bars[2].0.y - bars[0].0.y - 0.76 * 12.0).abs() < 1e-4);
    }
}
