//! Hotkey editor window: local and global shortcuts for every action.

use egui::{vec2, RichText};

use crate::app::JanmpApp;
use crate::hotkeys::{is_modifier, Action, Capture, HotkeySettings, Shortcut};

pub fn hotkeys_window(app: &mut JanmpApp, ctx: &egui::Context) {
    if !app.show_hotkeys {
        app.hotkey_capture = None;
        return;
    }
    capture_key(app, ctx);
    let t = app.tr();
    let skin = app.skin;
    let mut open = true;
    let mut global_changed = false;
    egui::Window::new(t.hotkeys)
        .open(&mut open)
        .collapsible(false)
        .default_size(vec2(440.0, 560.0))
        .show(ctx, |ui| {
            let keys = &mut app.settings.hotkeys;
            global_changed |= ui.checkbox(&mut keys.global_enabled, t.hotkeys_global_enabled).changed();
            global_changed |= ui.checkbox(&mut keys.media_keys, t.hotkeys_media_keys).changed();
            if let Some(msg) = global_unavailable(app) {
                ui.label(RichText::new(msg).size(11.0).color(skin.text_dim));
            }
            if !app.global_keys.failed.is_empty() {
                let list = app.global_keys.failed.join(", ");
                ui.label(RichText::new(format!("{} {list}", t.hotkeys_failed)).size(11.0).color(egui::Color32::from_rgb(0xE0, 0x60, 0x60)));
            }
            ui.label(RichText::new(t.hotkeys_hint).size(11.0).color(skin.text_dim));
            ui.add_space(4.0);

            let mut start: Option<Capture> = None;
            let mut remove_local: Option<(Action, usize)> = None;
            let mut remove_global: Option<Action> = None;
            let capture = app.hotkey_capture;
            let keys = &app.settings.hotkeys;
            let press = |ui: &mut egui::Ui, label: String, active: bool| {
                let text = if active { RichText::new(t.hotkeys_press).color(skin.accent) } else { RichText::new(label) };
                ui.add(egui::Button::new(text.size(12.0)).selected(active))
            };
            egui::ScrollArea::vertical().auto_shrink([false, true]).show(ui, |ui| {
                egui::Grid::new("hotkeys").num_columns(3).striped(true).spacing(vec2(10.0, 4.0)).show(ui, |ui| {
                    ui.label(RichText::new(t.hotkeys_action).strong());
                    ui.label(RichText::new(t.hotkeys_local).strong());
                    ui.label(RichText::new(t.hotkeys_global).strong());
                    ui.end_row();
                    for action in Action::ALL {
                        ui.label(t.actions[action.index()]);
                        ui.horizontal(|ui| {
                            let list = keys.local.get(&action).map(Vec::as_slice).unwrap_or_default();
                            for (i, sc) in list.iter().enumerate() {
                                let cap = Capture { action, global: false, slot: Some(i) };
                                let r = press(ui, sc.to_string(), capture == Some(cap));
                                if r.clicked() {
                                    start = Some(cap);
                                } else if r.secondary_clicked() {
                                    remove_local = Some((action, i));
                                }
                            }
                            let cap = Capture { action, global: false, slot: None };
                            if press(ui, "+".into(), capture == Some(cap)).clicked() {
                                start = Some(cap);
                            }
                        });
                        if action.can_be_global() {
                            ui.add_enabled_ui(keys.global_enabled, |ui| {
                                let cap = Capture { action, global: true, slot: None };
                                let label = keys.global.get(&action).map(Shortcut::to_string).unwrap_or_else(|| "+".into());
                                let r = press(ui, label, capture == Some(cap));
                                if r.clicked() {
                                    start = Some(cap);
                                } else if r.secondary_clicked() {
                                    remove_global = Some(action);
                                }
                            });
                        } else {
                            ui.label(RichText::new("—").color(skin.text_dim));
                        }
                        ui.end_row();
                    }
                });
            });
            ui.add_space(6.0);
            if ui.button(t.hotkeys_reset).clicked() {
                app.settings.hotkeys = HotkeySettings::default();
                app.hotkey_capture = None;
                global_changed = true;
            }

            if let Some(c) = start {
                app.hotkey_capture = if app.hotkey_capture == Some(c) { None } else { Some(c) };
            }
            let keys = &mut app.settings.hotkeys;
            if let Some((a, i)) = remove_local {
                if let Some(v) = keys.local.get_mut(&a) {
                    if i < v.len() {
                        v.remove(i);
                    }
                }
            }
            if let Some(a) = remove_global {
                global_changed |= keys.global.remove(&a).is_some();
            }
        });
    if global_changed {
        app.apply_global_hotkeys();
    }
    if !open {
        app.show_hotkeys = false;
        app.hotkey_capture = None;
        app.save();
    }
}

fn global_unavailable(app: &JanmpApp) -> Option<&'static str> {
    let wayland = cfg!(target_os = "linux") && std::env::var_os("WAYLAND_DISPLAY").is_some();
    (app.global_keys.unavailable.is_some() || wayland).then_some(app.tr().hotkeys_unavailable)
}

/// While the editor waits for a shortcut, take the next key press from the input.
fn capture_key(app: &mut JanmpApp, ctx: &egui::Context) {
    let Some(cap) = app.hotkey_capture else { return };
    let pressed = ctx.input_mut(|i| {
        let mut got = None;
        i.events.retain(|e| match e {
            egui::Event::Key { key, physical_key, pressed: true, modifiers, .. } if !is_modifier(*key) => {
                got.get_or_insert((physical_key.unwrap_or(*key), *modifiers));
                false
            }
            _ => true,
        });
        got
    });
    let Some((key, mods)) = pressed else { return };
    log::debug!("captured {key:?} {mods:?} for {cap:?}");
    app.hotkey_capture = None;
    if key == egui::Key::Escape && mods.is_none() {
        return;
    }
    let sc = Shortcut::from_event(key, mods);
    let t = app.tr();
    let keys = &mut app.settings.hotkeys;
    let moved_from = if cap.global {
        let other = keys.global_conflict(sc, cap.action);
        if let Some(o) = other {
            keys.global.remove(&o);
        }
        keys.global.insert(cap.action, sc);
        other
    } else {
        let other = keys.local_conflict(sc, cap.action);
        if let Some(v) = other.and_then(|o| keys.local.get_mut(&o)) {
            v.retain(|x| *x != sc);
        }
        let v = keys.local.entry(cap.action).or_default();
        match cap.slot {
            Some(i) if i < v.len() => v[i] = sc,
            _ if v.contains(&sc) => {}
            _ => v.push(sc),
        }
        other
    };
    if cap.global {
        app.apply_global_hotkeys();
    }
    if let Some(o) = moved_from {
        let now = ctx.input(|i| i.time);
        app.show_info(format!("{sc}: {} «{}»", t.hotkeys_reassigned, t.actions[o.index()]), now);
    }
}
