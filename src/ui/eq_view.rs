//! Equalizer and ReplayGain window.

use egui::{pos2, vec2, Align2, FontId, Rect, Response, RichText, Sense, Stroke, Ui};

use crate::app::JanmpApp;
use crate::audio::eq::{BANDS, MAX_DB, PRESETS};
use crate::audio::replaygain::RgMode;
use crate::config::EqPreset;
use crate::ui::skin::Skin;

const SLIDER_W: f32 = 18.0;
const SLIDER_H: f32 = 130.0;

fn band_label(f: f32) -> String {
    if f >= 1_000.0 {
        let k = f / 1_000.0;
        if k.fract() == 0.0 || k > 10.0 { format!("{}k", k.round()) } else { format!("{k:.1}k") }
    } else {
        format!("{f}")
    }
}

/// Vertical dB slider in the style of a hardware graphic equalizer.
fn eq_slider(ui: &mut Ui, skin: &Skin, value: &mut f32, enabled: bool) -> Response {
    let (rect, mut resp) = ui.allocate_exact_size(vec2(SLIDER_W, SLIDER_H), Sense::click_and_drag());
    let rail = Rect::from_center_size(rect.center(), vec2(4.0, SLIDER_H - 12.0));
    let to_y = |v: f32| rail.center().y - v / MAX_DB * rail.height() / 2.0;
    if let Some(p) = resp.interact_pointer_pos() {
        if resp.dragged() || resp.clicked() {
            let v = (rail.center().y - p.y) / (rail.height() / 2.0) * MAX_DB;
            // Snap to half-dB steps, with a small dead zone at zero.
            let v = if v.abs() < 0.4 { 0.0 } else { (v * 2.0).round() / 2.0 };
            *value = v.clamp(-MAX_DB, MAX_DB);
            resp.mark_changed();
        }
    }
    if resp.secondary_clicked() {
        *value = 0.0;
        resp.mark_changed();
    }
    if resp.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll != 0.0 {
            *value = (*value + scroll.signum() * 0.5).clamp(-MAX_DB, MAX_DB);
            resp.mark_changed();
        }
    }
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        p.rect_filled(rail, 2.0, skin.rail);
        let zero = to_y(0.0);
        p.line_segment([pos2(rect.left() + 2.0, zero), pos2(rect.right() - 2.0, zero)], Stroke::new(1.0, skin.border));
        let y = to_y(*value);
        let accent = if enabled { skin.accent } else { skin.text_dim };
        let fill = Rect::from_x_y_ranges(rail.x_range(), zero.min(y)..=zero.max(y));
        p.rect_filled(fill, 2.0, accent);
        let knob = Rect::from_center_size(pos2(rail.center().x, y), vec2(SLIDER_W - 4.0, 7.0));
        let kc = if resp.hovered() || resp.dragged() { skin.text_bright } else { skin.text };
        p.rect_filled(knob, 2.0, kc);
    }
    resp.on_hover_text(format!("{:+.1} dB", value))
}

pub fn eq_window(app: &mut JanmpApp, ctx: &egui::Context) {
    if !app.show_eq {
        return;
    }
    let t = app.tr();
    let skin = app.skin;
    let mut open = true;
    let mut changed = false;
    egui::Window::new(t.equalizer)
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .show(ctx, |ui| {
            let eq = &mut app.settings.eq;
            ui.horizontal(|ui| {
                changed |= ui.checkbox(&mut eq.enabled, t.eq_enabled).changed();
                ui.add_space(8.0);
                ui.label(t.eq_preset);
                let current = PRESETS
                    .iter()
                    .position(|(_, g)| *g == eq.gains)
                    .map(|i| t.eq_presets[i].to_string())
                    .or_else(|| eq.presets.iter().find(|p| p.gains == eq.gains).map(|p| p.name.clone()))
                    .unwrap_or_else(|| t.eq_custom.to_string());
                egui::ComboBox::from_id_salt("eq_preset").width(150.0).selected_text(&current).show_ui(ui, |ui| {
                    for (i, (_, g)) in PRESETS.iter().enumerate() {
                        if ui.selectable_label(eq.gains == *g, t.eq_presets[i]).clicked() {
                            eq.gains = *g;
                            changed = true;
                        }
                    }
                    if !eq.presets.is_empty() {
                        ui.separator();
                    }
                    for p in &eq.presets {
                        if ui.selectable_label(eq.gains == p.gains, &p.name).clicked() {
                            eq.gains = p.gains;
                            changed = true;
                        }
                    }
                });
                let user = eq.presets.iter().position(|p| p.gains == eq.gains);
                if let Some(i) = user {
                    if ui.button(t.eq_delete).clicked() {
                        eq.presets.remove(i);
                    }
                } else if PRESETS.iter().all(|(_, g)| *g != eq.gains) {
                    let name = &mut app.eq_new_preset;
                    match name {
                        None => {
                            if ui.button(t.eq_save).clicked() {
                                *name = Some(String::new());
                            }
                        }
                        Some(n) => {
                            let r = ui.add(egui::TextEdit::singleline(n).hint_text(t.eq_preset_name).desired_width(110.0));
                            r.request_focus();
                            if r.lost_focus() {
                                if ui.input(|i| i.key_pressed(egui::Key::Enter)) && !n.trim().is_empty() {
                                    eq.presets.push(EqPreset { name: n.trim().to_string(), gains: eq.gains });
                                }
                                *name = None;
                            }
                        }
                    }
                }
            });
            ui.add_space(6.0);

            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let column = |ui: &mut Ui, value: &mut f32, label: &str| -> bool {
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        let r = eq_slider(ui, &skin, value, eq.enabled);
                        let (lr, _) = ui.allocate_exact_size(vec2(SLIDER_W, 12.0), Sense::hover());
                        ui.painter().text(lr.center(), Align2::CENTER_CENTER, label, FontId::proportional(9.5), skin.text_dim);
                        r.changed()
                    })
                    .inner
                };
                changed |= column(ui, &mut eq.preamp_db, "Pre");
                // Scale between the preamp and the bands.
                let (scale, _) = ui.allocate_exact_size(vec2(20.0, SLIDER_H), Sense::hover());
                for db in [15, 0, -15] {
                    let y = scale.center().y - db as f32 / MAX_DB * (SLIDER_H - 12.0) / 2.0;
                    let text = if db > 0 { format!("+{db}") } else { db.to_string() };
                    ui.painter().text(pos2(scale.center().x, y), Align2::CENTER_CENTER, text, FontId::proportional(9.0), skin.text_dim);
                }
                for (g, f) in eq.gains.iter_mut().zip(BANDS) {
                    changed |= column(ui, g, &band_label(f));
                }
            });
            ui.label(RichText::new(format!("{}: {:+.1} dB. {}", t.eq_preamp, eq.preamp_db, t.eq_hint)).size(11.0).color(skin.text_dim));

            ui.add_space(8.0);
            ui.separator();
            ui.label(RichText::new(t.replaygain).strong());
            let rg = &mut app.settings.replaygain;
            egui::Grid::new("rg").num_columns(2).spacing(vec2(12.0, 6.0)).show(ui, |ui| {
                ui.label(t.rg_mode);
                ui.horizontal(|ui| {
                    for (i, m) in RgMode::ALL.into_iter().enumerate() {
                        changed |= ui.selectable_value(&mut rg.mode, m, t.rg_modes[i]).changed();
                    }
                });
                ui.end_row();
                ui.add_enabled_ui(rg.mode != RgMode::Off, |ui| ui.label(t.rg_preamp));
                changed |= ui
                    .add_enabled(rg.mode != RgMode::Off, egui::Slider::new(&mut rg.preamp_db, -15.0..=15.0).step_by(0.5).suffix(" dB"))
                    .changed();
                ui.end_row();
                ui.add_enabled_ui(rg.mode != RgMode::Off, |ui| ui.label(t.rg_fallback));
                changed |= ui
                    .add_enabled(rg.mode != RgMode::Off, egui::Slider::new(&mut rg.fallback_db, -15.0..=0.0).step_by(0.5).suffix(" dB"))
                    .changed();
                ui.end_row();
            });
            changed |= ui.add_enabled(rg.mode != RgMode::Off, egui::Checkbox::new(&mut rg.prevent_clipping, t.rg_clip)).changed();
        });
    if changed {
        app.apply_dsp();
        app.mark_settings_dirty();
    }
    if !open {
        app.show_eq = false;
        app.save();
    }
}
