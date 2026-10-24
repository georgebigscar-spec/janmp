//! Custom-painted widgets: vector icons, seek bar, compact slider, marquee text.

use egui::{pos2, vec2, Color32, FontId, Pos2, Rect, Response, Sense, Shape, Stroke, Ui};

use super::skin::Skin;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Icon {
    Play,
    Pause,
    Stop,
    Prev,
    Next,
    Shuffle,
    Repeat,
    RepeatOne,
    Volume,
    Mute,
    Menu,
    Minimize,
    Maximize,
    Close,
    Plus,
    Minus,
    Sort,
    Group,
    Search,
    Note,
}

/// Paint `icon` centred in `rect`.
pub fn paint_icon(painter: &egui::Painter, rect: Rect, icon: Icon, color: Color32) {
    let c = rect.center();
    let s = rect.width().min(rect.height()) * 0.5;
    let stroke = Stroke::new((s * 0.16).clamp(1.2, 2.2), color);
    let tri = |pts: Vec<Pos2>| Shape::convex_polygon(pts, color, Stroke::NONE);
    let p = |x: f32, y: f32| pos2(c.x + x * s, c.y + y * s);
    match icon {
        Icon::Play => {
            painter.add(tri(vec![p(-0.45, -0.6), p(0.65, 0.0), p(-0.45, 0.6)]));
        }
        Icon::Pause => {
            painter.rect_filled(Rect::from_min_max(p(-0.5, -0.55), p(-0.12, 0.55)), 1.0, color);
            painter.rect_filled(Rect::from_min_max(p(0.12, -0.55), p(0.5, 0.55)), 1.0, color);
        }
        Icon::Stop => {
            painter.rect_filled(Rect::from_min_max(p(-0.48, -0.48), p(0.48, 0.48)), 1.5, color);
        }
        Icon::Prev => {
            painter.rect_filled(Rect::from_min_max(p(-0.62, -0.5), p(-0.42, 0.5)), 0.5, color);
            painter.add(tri(vec![p(0.55, -0.5), p(0.55, 0.5), p(-0.35, 0.0)]));
        }
        Icon::Next => {
            painter.rect_filled(Rect::from_min_max(p(0.42, -0.5), p(0.62, 0.5)), 0.5, color);
            painter.add(tri(vec![p(-0.55, -0.5), p(0.35, 0.0), p(-0.55, 0.5)]));
        }
        Icon::Shuffle => {
            painter.line(vec![p(-0.7, -0.4), p(-0.25, -0.4), p(0.25, 0.4), p(0.6, 0.4)], stroke);
            painter.line(vec![p(-0.7, 0.4), p(-0.25, 0.4), p(0.25, -0.4), p(0.6, -0.4)], stroke);
            painter.add(tri(vec![p(0.5, -0.65), p(0.8, -0.4), p(0.5, -0.15)]));
            painter.add(tri(vec![p(0.5, 0.15), p(0.8, 0.4), p(0.5, 0.65)]));
        }
        Icon::Repeat | Icon::RepeatOne => {
            painter.line(vec![p(-0.6, 0.1), p(-0.6, -0.4), p(0.45, -0.4)], stroke);
            painter.line(vec![p(0.6, -0.1), p(0.6, 0.4), p(-0.45, 0.4)], stroke);
            painter.add(tri(vec![p(0.35, -0.65), p(0.7, -0.4), p(0.35, -0.15)]));
            painter.add(tri(vec![p(-0.35, 0.15), p(-0.7, 0.4), p(-0.35, 0.65)]));
            if icon == Icon::RepeatOne {
                painter.text(c, egui::Align2::CENTER_CENTER, "1", FontId::proportional(s * 0.75), color);
            }
        }
        Icon::Volume | Icon::Mute => {
            painter.add(tri(vec![p(-0.7, -0.22), p(-0.35, -0.22), p(0.05, -0.6), p(0.05, 0.6), p(-0.35, 0.22), p(-0.7, 0.22)]));
            if icon == Icon::Mute {
                painter.line_segment([p(0.25, -0.3), p(0.75, 0.3)], stroke);
                painter.line_segment([p(0.75, -0.3), p(0.25, 0.3)], stroke);
            } else {
                for (r, a) in [(0.38f32, 0.9f32), (0.65, 0.85)] {
                    let pts: Vec<Pos2> = (0..=10)
                        .map(|i| {
                            let t = -a + 2.0 * a * i as f32 / 10.0;
                            pos2(c.x + 0.05 * s + r * s * t.cos(), c.y + r * s * t.sin())
                        })
                        .collect();
                    painter.line(pts, stroke);
                }
            }
        }
        Icon::Menu => {
            for y in [-0.45, 0.0, 0.45] {
                painter.line_segment([p(-0.6, y), p(0.6, y)], stroke);
            }
        }
        Icon::Minimize => {
            painter.line_segment([p(-0.5, 0.35), p(0.5, 0.35)], stroke);
        }
        Icon::Maximize => {
            painter.rect_stroke(
                Rect::from_min_max(p(-0.45, -0.45), p(0.45, 0.45)),
                1.0,
                stroke,
                egui::StrokeKind::Middle,
            );
        }
        Icon::Close => {
            painter.line_segment([p(-0.45, -0.45), p(0.45, 0.45)], stroke);
            painter.line_segment([p(0.45, -0.45), p(-0.45, 0.45)], stroke);
        }
        Icon::Plus => {
            painter.line_segment([p(-0.55, 0.0), p(0.55, 0.0)], stroke);
            painter.line_segment([p(0.0, -0.55), p(0.0, 0.55)], stroke);
        }
        Icon::Minus => {
            painter.line_segment([p(-0.55, 0.0), p(0.55, 0.0)], stroke);
        }
        Icon::Sort => {
            for (i, w) in [0.6f32, 0.4, 0.2].iter().enumerate() {
                let y = -0.45 + i as f32 * 0.45;
                painter.line_segment([p(-0.6, y), p(-0.6 + 2.0 * w, y)], stroke);
            }
        }
        Icon::Group => {
            painter.rect_filled(Rect::from_min_max(p(-0.65, -0.6), p(0.65, -0.25)), 1.0, color);
            painter.line_segment([p(-0.35, 0.1), p(0.65, 0.1)], stroke);
            painter.line_segment([p(-0.35, 0.5), p(0.65, 0.5)], stroke);
        }
        Icon::Note => {
            painter.circle_filled(p(-0.3, 0.45), 0.24 * s, color);
            painter.circle_filled(p(0.45, 0.3), 0.24 * s, color);
            let w = Stroke::new(stroke.width * 1.2, color);
            painter.line_segment([p(-0.08, 0.45), p(-0.08, -0.55)], w);
            painter.line_segment([p(0.67, 0.3), p(0.67, -0.7)], w);
            painter.add(Shape::convex_polygon(
                vec![p(-0.08, -0.55), p(0.67, -0.7), p(0.67, -0.45), p(-0.08, -0.3)],
                color,
                Stroke::NONE,
            ));
        }
        Icon::Search => {
            painter.circle_stroke(p(-0.12, -0.12), 0.38 * s, stroke);
            painter.line_segment([p(0.17, 0.17), p(0.6, 0.6)], stroke);
        }
    }
}

/// A flat icon button. `active` paints it in the accent colour (toggle state).
pub fn icon_button(ui: &mut Ui, skin: &Skin, icon: Icon, size: f32, active: bool, tip: &str) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    if ui.is_rect_visible(rect) {
        let hovered = resp.hovered();
        if hovered {
            ui.painter().rect_filled(rect, 4.0, skin.hover);
        }
        let color = if active {
            skin.accent
        } else if hovered {
            skin.control_hover
        } else {
            skin.control
        };
        let pad = size * 0.24;
        paint_icon(ui.painter(), rect.shrink(pad), icon, color);
    }
    if tip.is_empty() { resp } else { resp.on_hover_text(tip) }
}

/// Big round play/pause button.
pub fn round_button(ui: &mut Ui, skin: &Skin, icon: Icon, size: f32, tip: &str) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    if ui.is_rect_visible(rect) {
        let fill = if resp.is_pointer_button_down_on() {
            skin.accent.gamma_multiply(0.8)
        } else if resp.hovered() {
            skin.accent
        } else {
            skin.accent.gamma_multiply(0.9)
        };
        ui.painter().circle_filled(rect.center(), size * 0.5, fill);
        paint_icon(ui.painter(), rect.shrink(size * 0.3), icon, Color32::from_rgb(0x18, 0x18, 0x18));
    }
    resp.on_hover_text(tip)
}

/// A horizontal bar for seeking. Returns the position the user picked (0..1) once released.
pub fn seek_bar(ui: &mut Ui, skin: &Skin, fraction: f32, height: f32, enabled: bool) -> (Response, Option<f32>) {
    let width = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(width, height), Sense::click_and_drag());
    let id = resp.id;
    let pointer_frac = resp
        .interact_pointer_pos()
        .or(resp.hover_pos())
        .map(|p| ((p.x - rect.left()) / rect.width()).clamp(0.0, 1.0));

    let dragging = enabled && resp.dragged();
    let shown = if dragging { pointer_frac.unwrap_or(fraction) } else { fraction };
    let mut picked = None;
    if enabled && (resp.drag_stopped() || resp.clicked()) {
        picked = pointer_frac;
    }
    ui.ctx().data_mut(|d| d.insert_temp(id, dragging));

    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let rail_h = if resp.hovered() || dragging { 6.0 } else { 4.0 };
        let rail = Rect::from_center_size(rect.center(), vec2(rect.width(), rail_h));
        painter.rect_filled(rail, rail_h / 2.0, Color32::from_rgb(0x3A, 0x3A, 0x40));
        if enabled {
            let mut filled = rail;
            filled.set_right(rail.left() + rail.width() * shown.clamp(0.0, 1.0));
            painter.rect_filled(filled, rail_h / 2.0, skin.accent);
            if resp.hovered() || dragging {
                painter.circle_filled(pos2(filled.right(), rail.center().y), 6.0, skin.text_bright);
                if let (Some(f), false) = (pointer_frac, dragging) {
                    let x = rail.left() + rail.width() * f;
                    painter.line_segment(
                        [pos2(x, rail.top() - 2.0), pos2(x, rail.bottom() + 2.0)],
                        Stroke::new(1.0, skin.text_dim),
                    );
                }
            }
        }
    }
    (resp, picked)
}

/// Compact horizontal slider (volume / balance). Mouse wheel adjusts too.
pub fn mini_slider(ui: &mut Ui, skin: &Skin, value: &mut f32, range: (f32, f32), width: f32, centered: bool) -> Response {
    let (rect, mut resp) = ui.allocate_exact_size(vec2(width, 18.0), Sense::click_and_drag());
    let (lo, hi) = range;
    if let Some(p) = resp.interact_pointer_pos() {
        if resp.dragged() || resp.clicked() {
            let f = ((p.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
            *value = lo + f * (hi - lo);
            resp.mark_changed();
        }
    }
    if resp.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll != 0.0 {
            *value = (*value + scroll / 400.0 * (hi - lo)).clamp(lo, hi);
            resp.mark_changed();
        }
    }
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let rail = Rect::from_center_size(rect.center(), vec2(rect.width(), 4.0));
        painter.rect_filled(rail, 2.0, Color32::from_rgb(0x3A, 0x3A, 0x40));
        let f = ((*value - lo) / (hi - lo)).clamp(0.0, 1.0);
        let x = rail.left() + rail.width() * f;
        let fill = if centered {
            Rect::from_x_y_ranges(rail.center().x.min(x)..=rail.center().x.max(x), rail.y_range())
        } else {
            Rect::from_x_y_ranges(rail.left()..=x, rail.y_range())
        };
        painter.rect_filled(fill, 2.0, skin.accent);
        let r = if resp.hovered() || resp.dragged() { 6.0 } else { 4.5 };
        painter.circle_filled(pos2(x, rail.center().y), r, skin.text_bright);
    }
    resp
}

/// Single-line text that scrolls back and forth when it does not fit.
pub fn marquee(ui: &mut Ui, text: &str, font: FontId, color: Color32) -> Response {
    let width = ui.available_width().max(10.0);
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font.clone(), color);
    let h = galley.size().y;
    let (rect, resp) = ui.allocate_exact_size(vec2(width, h), Sense::hover());
    let overflow = galley.size().x - width;
    let mut x = rect.left();
    if overflow > 0.0 {
        // Pause at each end, then glide at ~30 px/s.
        let travel = overflow / 30.0;
        let period = 2.0 * travel + 4.0;
        let t = (ui.input(|i| i.time) as f32) % period;
        let off = if t < 2.0 {
            0.0
        } else if t < 2.0 + travel {
            (t - 2.0) * 30.0
        } else if t < 4.0 + travel {
            overflow
        } else {
            overflow - (t - 4.0 - travel) * 30.0
        };
        x -= off;
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
    }
    ui.painter().with_clip_rect(rect).galley(pos2(x, rect.top()), galley, color);
    resp
}

pub fn fmt_time(secs: f64) -> String {
    if !secs.is_finite() || secs < 0.0 {
        return "--:--".into();
    }
    let s = secs as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
    } else {
        format!("{:02}:{:02}", s / 60, s % 60)
    }
}

pub fn fmt_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = bytes as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 { format!("{bytes} B") } else { format!("{v:.2} {}", UNITS[u]) }
}
