//! Main window layout: title bar, player panel, playlist, footer, dialogs.

pub mod player_panel;
pub mod playlist_view;
pub mod skin;
pub mod spectrum;
pub mod widgets;

use egui::{vec2, Align, Color32, CursorIcon, Frame, Layout, Margin, Panel, ResizeDirection, Sense, ViewportCommand};

use crate::app::JanmpApp;
use crate::i18n::Lang;
use widgets::{icon_button, Icon};

pub fn draw(app: &mut JanmpApp, ui: &mut egui::Ui) {
    let skin = app.skin;
    if !app.settings.system_frame {
        Panel::top("titlebar")
            .exact_size(30.0)
            .frame(Frame::new().fill(skin.bg).inner_margin(Margin::symmetric(6, 0)))
            .show(ui, |ui| title_bar(app, ui));
    }
    Panel::top("player")
        .frame(Frame::new().fill(skin.panel).inner_margin(Margin::same(10)))
        .show(ui, |ui| player_panel::show(app, ui));
    Panel::bottom("footer")
        .exact_size(30.0)
        .frame(Frame::new().fill(skin.panel).inner_margin(Margin::symmetric(6, 0)))
        .show(ui, |ui| playlist_view::footer(app, ui));
    egui::CentralPanel::default()
        .frame(Frame::new().fill(skin.list_bg))
        .show(ui, |ui| playlist_view::show(app, ui));

    settings_window(app, ui.ctx());
    toast(app, ui.ctx());
    if !app.settings.system_frame {
        resize_handles(ui.ctx());
    }
}

fn title_bar(app: &mut JanmpApp, ui: &mut egui::Ui) {
    let skin = app.skin;
    let t = app.tr();
    let rect = ui.max_rect();
    // Whole bar drags the window; buttons placed on top take precedence.
    let drag = ui.interact(rect, ui.scope_id().with("drag"), Sense::click_and_drag());
    if drag.drag_started() {
        ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
    }
    let maximized = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
    if drag.double_clicked() {
        ui.ctx().send_viewport_cmd(ViewportCommand::Maximized(!maximized));
    }

    ui.horizontal_centered(|ui| {
        let menu = icon_button(ui, &skin, Icon::Menu, 24.0, false, t.menu);
        egui::Popup::menu(&menu).show(|ui| main_menu(app, ui));
        ui.add_space(4.0);
        ui.label(egui::RichText::new("janmp").color(skin.accent).strong().size(13.0));
        if let Some(track) = app.current_track() {
            ui.label(egui::RichText::new(track.display_title()).color(skin.text_dim).size(12.0));
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if icon_button(ui, &skin, Icon::Close, 24.0, false, t.close).clicked() {
                ui.ctx().send_viewport_cmd(ViewportCommand::Close);
            }
            if icon_button(ui, &skin, Icon::Maximize, 24.0, false, t.maximize).clicked() {
                ui.ctx().send_viewport_cmd(ViewportCommand::Maximized(!maximized));
            }
            if icon_button(ui, &skin, Icon::Minimize, 24.0, false, t.minimize).clicked() {
                ui.ctx().send_viewport_cmd(ViewportCommand::Minimized(true));
            }
        });
    });
}

pub fn main_menu(app: &mut JanmpApp, ui: &mut egui::Ui) {
    let t = app.tr();
    ui.set_min_width(220.0);
    if ui.button(t.add_files).clicked() {
        app.open_files_dialog();
    }
    if ui.button(t.add_folder).clicked() {
        app.open_folder_dialog();
    }
    ui.separator();
    if ui.button(t.new_playlist).clicked() {
        app.new_playlist();
    }
    if ui.button(t.open_playlist).clicked() {
        app.open_playlist_dialog();
    }
    if ui.button(t.save_playlist).clicked() {
        app.save_playlist_dialog();
    }
    ui.separator();
    ui.menu_button(t.language, |ui| {
        for lang in Lang::ALL {
            if ui.radio(app.settings.lang == lang, lang.native_name()).clicked() {
                app.settings.lang = lang;
                app.mark_dirty();
            }
        }
    });
    if ui.button(t.settings).clicked() {
        app.show_settings = true;
    }
    ui.separator();
    if ui.button(t.exit).clicked() {
        ui.ctx().send_viewport_cmd(ViewportCommand::Close);
    }
}

fn settings_window(app: &mut JanmpApp, ctx: &egui::Context) {
    if !app.show_settings {
        return;
    }
    let t = app.tr();
    let mut open = true;
    egui::Window::new(t.settings_title)
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(420.0)
        .show(ctx, |ui| {
            egui::Grid::new("settings").num_columns(2).spacing(vec2(12.0, 8.0)).show(ui, |ui| {
                ui.label(t.language);
                egui::ComboBox::from_id_salt("lang")
                    .selected_text(app.settings.lang.native_name())
                    .show_ui(ui, |ui| {
                        for lang in Lang::ALL {
                            ui.selectable_value(&mut app.settings.lang, lang, lang.native_name());
                        }
                    });
                ui.end_row();

                ui.label(t.accent_color);
                let mut c = app.settings.accent;
                if ui.color_edit_button_srgb(&mut c).changed() {
                    app.settings.accent = c;
                    app.skin = skin::Skin::new(c);
                    app.skin.apply(ui.ctx());
                }
                ui.end_row();
            });
            ui.add_space(4.0);
            ui.checkbox(&mut app.settings.show_spectrum, t.show_spectrum);
            ui.checkbox(&mut app.settings.system_frame, t.system_frame);
            ui.add_space(8.0);
            ui.label(egui::RichText::new(t.hotkeys).strong());
            ui.label(egui::RichText::new(t.hotkeys_help).color(app.skin.text_dim));
            ui.add_space(8.0);
            ui.label(egui::RichText::new(t.about).strong());
            ui.label(format!("{} v{}", t.about_text, env!("CARGO_PKG_VERSION")));
        });
    if !open {
        app.show_settings = false;
        app.save();
    }
}

fn toast(app: &mut JanmpApp, ctx: &egui::Context) {
    let now = ctx.input(|i| i.time);
    let Some((msg, until)) = &app.toast else { return };
    if now > *until {
        app.toast = None;
        return;
    }
    let msg = msg.clone();
    let mut dismiss = false;
    egui::Area::new(egui::Id::unique("toast"))
        .anchor(egui::Align2::CENTER_BOTTOM, vec2(0.0, -40.0))
        .show(ctx, |ui| {
            let resp = Frame::new()
                .fill(Color32::from_rgb(0x5A, 0x22, 0x22))
                .corner_radius(4.0)
                .inner_margin(Margin::symmetric(10, 6))
                .show(ui, |ui| {
                    ui.set_max_width(380.0);
                    ui.label(egui::RichText::new(&msg).color(Color32::WHITE));
                })
                .response
                .interact(Sense::click());
            dismiss = resp.clicked();
        });
    if dismiss {
        app.toast = None;
    }
    ctx.request_repaint_after(std::time::Duration::from_millis(500));
}

/// Borderless windows need their own resize grips along the edges.
fn resize_handles(ctx: &egui::Context) {
    if ctx.input(|i| i.viewport().maximized.unwrap_or(false)) {
        return;
    }
    let rect = ctx.content_rect();
    let Some(pos) = ctx.input(|i| i.pointer.hover_pos()) else { return };
    let m = 5.0;
    let (l, r) = (pos.x - rect.left() < m, rect.right() - pos.x < m);
    let (t, b) = (pos.y - rect.top() < m, rect.bottom() - pos.y < m);
    let dir = match (l, r, t, b) {
        (true, _, true, _) => Some((ResizeDirection::NorthWest, CursorIcon::ResizeNorthWest)),
        (_, true, true, _) => Some((ResizeDirection::NorthEast, CursorIcon::ResizeNorthEast)),
        (true, _, _, true) => Some((ResizeDirection::SouthWest, CursorIcon::ResizeSouthWest)),
        (_, true, _, true) => Some((ResizeDirection::SouthEast, CursorIcon::ResizeSouthEast)),
        (true, ..) => Some((ResizeDirection::West, CursorIcon::ResizeWest)),
        (_, true, ..) => Some((ResizeDirection::East, CursorIcon::ResizeEast)),
        (_, _, true, _) => Some((ResizeDirection::North, CursorIcon::ResizeNorth)),
        (_, _, _, true) => Some((ResizeDirection::South, CursorIcon::ResizeSouth)),
        _ => None,
    };
    if let Some((dir, cursor)) = dir {
        ctx.set_cursor_icon(cursor);
        if ctx.input(|i| i.pointer.primary_pressed()) {
            ctx.send_viewport_cmd(ViewportCommand::BeginResize(dir));
        }
    }
}
