//! Main window layout: title bar, player panel, playlist, footer, dialogs.

pub mod eq_view;
pub mod hotkeys_view;
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
    url_dialog(app, ui.ctx());
    stats_window(app, ui.ctx());
    hotkeys_view::hotkeys_window(app, ui.ctx());
    eq_view::eq_window(app, ui.ctx());
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
            let text = match (&app.stream_title, track.is_stream()) {
                (Some(song), true) => format!("{} — {song}", track.title),
                _ => track.display_title(),
            };
            ui.add(egui::Label::new(egui::RichText::new(text).color(skin.text_dim).size(12.0)).truncate());
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
    if ui.button(t.add_url).clicked() {
        app.url_dialog = Some(String::new());
    }
    if ui.button(t.add_audio_cd).clicked() {
        app.add_audio_cd();
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
    ui.menu_button(t.bookmarks, |ui| {
        ui.set_min_width(260.0);
        if ui.button(t.add_bookmark).clicked() {
            app.add_bookmark();
        }
        ui.separator();
        if app.library.bookmarks.is_empty() {
            ui.label(egui::RichText::new(t.no_bookmarks).color(app.skin.text_dim));
        }
        let mut open = None;
        let mut remove = None;
        for (i, b) in app.library.bookmarks.iter().enumerate() {
            ui.horizontal(|ui| {
                if icon_button(ui, &app.skin, Icon::Close, 18.0, false, "").clicked() {
                    remove = Some(i);
                }
                if ui.button(format!("{}  @ {}", b.title, widgets::fmt_time(b.position))).clicked() {
                    open = Some(i);
                }
            });
        }
        if let Some(i) = open {
            app.open_bookmark(i);
        }
        if let Some(i) = remove {
            app.remove_bookmark(i);
        }
    });
    if ui.button(t.statistics).clicked() {
        app.show_stats = true;
    }
    if ui.button(format!("{}…", t.equalizer)).clicked() {
        app.show_eq = true;
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
    if ui.button(t.hotkeys_configure).clicked() {
        app.show_hotkeys = true;
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
            ui.checkbox(&mut app.settings.show_waveform, t.show_waveform);
            ui.checkbox(&mut app.settings.system_frame, t.system_frame);
            ui.add_space(8.0);
            ui.label(egui::RichText::new(t.record_section).strong());
            egui::Grid::new("record").num_columns(2).spacing(vec2(12.0, 6.0)).show(ui, |ui| {
                use crate::radio::record::RecordFormat;
                let label = |f: RecordFormat| if f == RecordFormat::Original { t.record_original } else { f.label() };
                ui.label(t.record_format);
                egui::ComboBox::from_id_salt("recfmt")
                    .width(240.0)
                    .selected_text(label(app.settings.record.format))
                    .show_ui(ui, |ui| {
                        for f in RecordFormat::ALL {
                            ui.selectable_value(&mut app.settings.record.format, f, label(f));
                        }
                    });
                ui.end_row();
                ui.label(t.record_dir);
                ui.horizontal(|ui| {
                    ui.add(egui::Label::new(app.settings.record.dir.display().to_string()).truncate());
                    if ui.button(t.browse).clicked() {
                        if let Some(d) = rfd::FileDialog::new().set_directory(&app.settings.record.dir).pick_folder() {
                            app.settings.record.dir = d;
                        }
                    }
                });
                ui.end_row();
            });
            ui.checkbox(&mut app.settings.record.split_by_title, t.record_split);
            ui.add_space(8.0);
            ui.label(egui::RichText::new(t.ffmpeg).strong());
            match crate::audio::ffmpeg::ffmpeg_path() {
                Some(p) => ui.label(egui::RichText::new(p.display().to_string()).color(app.skin.text_dim)),
                None => ui.label(egui::RichText::new(t.ffmpeg_missing).color(app.skin.text_dim)),
            };
            ui.add_space(8.0);
            ui.label(egui::RichText::new(t.soundfont).strong());
            ui.horizontal(|ui| {
                let current = crate::audio::midi::soundfont_path();
                let text = match (&app.settings.soundfont, &current) {
                    (None, Some(p)) => format!("{} ({})", p.display(), t.soundfont_auto),
                    (_, Some(p)) => p.display().to_string(),
                    (_, None) => t.soundfont_missing.to_string(),
                };
                if ui.button(t.browse).clicked() {
                    let mut dlg = rfd::FileDialog::new().add_filter("SoundFont", &["sf2"]);
                    if let Some(dir) = current.as_ref().and_then(|p| p.parent()) {
                        dlg = dlg.set_directory(dir);
                    }
                    if let Some(p) = dlg.pick_file() {
                        crate::audio::midi::set_soundfont(Some(p.clone()));
                        app.settings.soundfont = Some(p);
                    }
                }
                if app.settings.soundfont.is_some() && ui.button(t.soundfont_auto).clicked() {
                    app.settings.soundfont = None;
                    crate::audio::midi::set_soundfont(None);
                }
                ui.add(egui::Label::new(egui::RichText::new(text).color(app.skin.text_dim)).truncate());
            });
            ui.add_space(8.0);
            ui.label(egui::RichText::new(t.hotkeys).strong());
            if ui.button(t.hotkeys_configure).clicked() {
                app.show_hotkeys = true;
            }
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
    if let Some((_, until, _)) = &mut app.toast {
        // Messages created without a clock start counting when first shown.
        if *until == f64::NEG_INFINITY {
            *until = now + 4.0;
        }
    }
    let Some((msg, until, is_error)) = &app.toast else { return };
    let fill = if *is_error { Color32::from_rgb(0x5A, 0x22, 0x22) } else { Color32::from_rgb(0x24, 0x44, 0x2A) };
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
                .fill(fill)
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

fn url_dialog(app: &mut JanmpApp, ctx: &egui::Context) {
    let Some(mut text) = app.url_dialog.take() else { return };
    let t = app.tr();
    let mut open = true;
    let mut submit = false;
    egui::Window::new(t.add_url)
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .show(ctx, |ui| {
            let resp = ui.add(egui::TextEdit::singleline(&mut text).hint_text(t.url_hint).desired_width(360.0));
            resp.request_focus();
            if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                submit = true;
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui.button(t.add).clicked() {
                    submit = true;
                }
            });
        });
    if submit {
        app.add_url(&text);
    } else if open && !ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.url_dialog = Some(text);
    }
}

fn stats_window(app: &mut JanmpApp, ctx: &egui::Context) {
    if !app.show_stats {
        return;
    }
    let t = app.tr();
    let skin = app.skin;
    let lib = &app.library;
    let mut open = true;
    egui::Window::new(t.statistics.trim_end_matches('…'))
        .open(&mut open)
        .collapsible(false)
        .default_size(vec2(440.0, 520.0))
        .show(ctx, |ui| {
            egui::Grid::new("stats_totals").num_columns(2).spacing(vec2(12.0, 4.0)).show(ui, |ui| {
                ui.label(t.stats_total_time);
                ui.label(egui::RichText::new(widgets::fmt_time(lib.total_listened())).strong());
                ui.end_row();
                ui.label(t.stats_total_plays);
                ui.label(egui::RichText::new(lib.total_plays().to_string()).strong());
                ui.end_row();
            });
            ui.label(egui::RichText::new(t.stats_auto_hint).size(11.0).color(skin.text_dim));
            egui::ScrollArea::vertical().show(ui, |ui| {
                let section = |ui: &mut egui::Ui, title: &str| {
                    ui.add_space(8.0);
                    ui.label(egui::RichText::new(title).strong().color(skin.accent));
                };
                section(ui, t.stats_top_tracks);
                for (i, (_, s)) in lib.top_tracks(20).into_iter().enumerate() {
                    let name = if s.artist.is_empty() { s.title.clone() } else { format!("{} - {}", s.artist, s.title) };
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(format!("{:>2}.", i + 1)).color(skin.text_dim));
                        ui.add(egui::Label::new(name).truncate());
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(egui::RichText::new(format!("{} {} · {} {}", s.plays, t.stats_plays, s.skips, t.stats_skips)).color(skin.text_dim));
                        });
                    });
                }
                section(ui, t.stats_top_artists);
                for (i, (artist, plays, secs)) in lib.top_artists(10).into_iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(format!("{:>2}.", i + 1)).color(skin.text_dim));
                        ui.label(artist);
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(egui::RichText::new(format!("{plays} {} · {}", t.stats_plays, widgets::fmt_time(secs))).color(skin.text_dim));
                        });
                    });
                }
                section(ui, t.stats_recent);
                for (_, s) in lib.recent(15) {
                    let name = if s.artist.is_empty() { s.title.clone() } else { format!("{} - {}", s.artist, s.title) };
                    ui.add(egui::Label::new(name).truncate());
                }
            });
        });
    app.show_stats = open;
}
