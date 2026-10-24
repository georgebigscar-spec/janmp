//! Playlist tabs, the grouped track list and the playlist footer.

use std::path::PathBuf;

use egui::{pos2, vec2, Align, Align2, FontId, Layout, Rect, RichText, ScrollArea, Sense, Stroke};

use super::widgets::{fmt_size, fmt_time, icon_button, paint_icon, Icon};
use crate::app::{JanmpApp, TrackRef};
use crate::audio::PlaybackState;
use crate::playlist::{GroupMode, SortKey, Track};

const ROW_H: f32 = 22.0;
const HEADER_H: f32 = 48.0;
const TABS_H: f32 = 28.0;

pub enum Row {
    Header { first: usize, count: usize, title: String, subtitle: String },
    Track { idx: usize },
}

#[derive(Default)]
pub struct RowCache {
    key: Option<(u64, u64, GroupMode, String, crate::i18n::Lang)>,
    rows: Vec<Row>,
    offsets: Vec<f32>,
    total: f32,
}

impl RowCache {
    fn rebuild(&mut self, app: &JanmpApp, pl_idx: usize) {
        let pl = &app.playlists[pl_idx];
        let mode = app.settings.group;
        let key = (pl.id, app.epoch, mode, app.search.clone(), app.settings.lang);
        if self.key.as_ref() == Some(&key) {
            return;
        }
        let lang = app.settings.lang;
        let needle = app.search.to_lowercase();
        let matches = |t: &Track| {
            needle.is_empty()
                || t.title.to_lowercase().contains(&needle)
                || t.artist.to_lowercase().contains(&needle)
                || t.album.to_lowercase().contains(&needle)
                || t.path.to_string_lossy().to_lowercase().contains(&needle)
        };
        self.rows.clear();
        let visible: Vec<usize> = (0..pl.tracks.len()).filter(|&i| matches(&pl.tracks[i])).collect();
        if mode == GroupMode::None {
            self.rows.extend(visible.iter().map(|&idx| Row::Track { idx }));
        } else {
            let mut i = 0;
            while i < visible.len() {
                let gk = pl.tracks[visible[i]].group_key(mode);
                let mut j = i + 1;
                while j < visible.len() && pl.tracks[visible[j]].group_key(mode) == gk {
                    j += 1;
                }
                let members = &visible[i..j];
                let first = &pl.tracks[members[0]];
                let dur: f64 = members.iter().filter_map(|&k| pl.tracks[k].duration).sum();
                let mut sub = Vec::new();
                if mode == GroupMode::Album {
                    if let Some(y) = first.year {
                        sub.push(y.to_string());
                    }
                    if !first.genre.is_empty() {
                        sub.push(first.genre.clone());
                    }
                }
                sub.push(lang.tracks(members.len()));
                if dur > 0.0 {
                    sub.push(fmt_time(dur));
                }
                self.rows.push(Row::Header {
                    first: members[0],
                    count: members.len(),
                    title: match (gk.is_empty(), first.is_stream()) {
                        (false, _) => gk,
                        (true, true) => lang.strings().radio.to_string(),
                        (true, false) => lang.strings().unknown.to_string(),
                    },
                    subtitle: sub.join(" · "),
                });
                self.rows.extend(members.iter().map(|&idx| Row::Track { idx }));
                i = j;
            }
        }
        self.offsets.clear();
        let mut y = 0.0;
        for r in &self.rows {
            self.offsets.push(y);
            y += match r {
                Row::Header { .. } => HEADER_H,
                Row::Track { .. } => ROW_H,
            };
        }
        self.total = y;
        self.key = Some(key);
    }
}

enum Action {
    Play(TrackRef),
    Click { uid: u64, ctrl: bool, shift: bool },
    SelectRange(usize, usize),
    Queue(Vec<TrackRef>),
    RemoveSelected,
    OpenFolder(PathBuf),
    SelectAll,
}

pub fn show(app: &mut JanmpApp, ui: &mut egui::Ui) {
    tabs(app, ui);
    let pl_idx = app.active();
    let mut cache = std::mem::take(&mut app.list_cache);
    cache.rebuild(app, pl_idx);

    if app.playlists[pl_idx].tracks.is_empty() {
        let rect = ui.available_rect_before_wrap();
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            app.tr().drop_hint,
            FontId::proportional(13.0),
            app.skin.text_dim,
        );
        app.list_cache = cache;
        return;
    }

    let mut actions = Vec::new();
    let skin = app.skin;
    let t = app.tr();
    let playing = app.player.state();
    let scroll_to = app.scroll_to_current.then_some(app.current).flatten();
    app.scroll_to_current = false;
    let pl = &app.playlists[pl_idx];
    let grouped = app.settings.group != GroupMode::None;

    ScrollArea::vertical().auto_shrink([false, false]).id_salt(("list", pl.id)).show_viewport(ui, |ui, viewport| {
        ui.set_height(cache.total);
        let width = ui.available_width();
        let origin = ui.max_rect().min;

        if let Some(r) = scroll_to.filter(|r| r.playlist == pl.id) {
            if let Some(i) = cache.rows.iter().position(|row| matches!(row, Row::Track { idx } if pl.tracks[*idx].uid == r.uid)) {
                let rect = Rect::from_min_size(origin + vec2(0.0, cache.offsets[i]), vec2(width, ROW_H));
                ui.scroll_to_rect(rect, Some(Align::Center));
            }
        }

        let start = cache.offsets.partition_point(|&y| y + HEADER_H < viewport.min.y);
        for (i, row) in cache.rows.iter().enumerate().skip(start) {
            let y = cache.offsets[i];
            if y > viewport.max.y {
                break;
            }
            match row {
                Row::Header { first, count, title, subtitle } => {
                    let rect = Rect::from_min_size(origin + vec2(0.0, y), vec2(width, HEADER_H));
                    let resp = ui.interact(rect, ui.scope_id().with(("hdr", pl.tracks[*first].uid)), Sense::click());
                    let painter = ui.painter_at(rect);
                    painter.rect_filled(rect, 0.0, if resp.hovered() { skin.hover } else { skin.group_header });
                    let thumb = Rect::from_min_size(rect.min + vec2(6.0, 4.0), vec2(40.0, 40.0));
                    match app.covers_thumb.get(&pl.tracks[*first].path, &app.scanner) {
                        Some(tex) => {
                            egui::Image::new(&tex).corner_radius(2.0).paint_at(ui, thumb);
                        }
                        None => {
                            painter.rect_filled(thumb, 2.0, skin.list_bg);
                            paint_icon(&painter, thumb.shrink(12.0), Icon::Note, skin.text_dim.gamma_multiply(0.6));
                        }
                    }
                    let tx = thumb.right() + 10.0;
                    let text_clip = Rect::from_min_max(pos2(tx, rect.top()), pos2(rect.right() - 8.0, rect.bottom()));
                    let tp = ui.painter_at(text_clip);
                    tp.text(pos2(tx, rect.top() + 8.0), Align2::LEFT_TOP, title, FontId::proportional(14.0), skin.text_bright);
                    tp.text(pos2(tx, rect.top() + 28.0), Align2::LEFT_TOP, subtitle, FontId::proportional(11.0), skin.text_dim);
                    painter.line_segment([rect.left_bottom(), rect.right_bottom()], Stroke::new(1.0, skin.border));
                    if resp.clicked() {
                        actions.push(Action::SelectRange(*first, *count));
                    }
                    if resp.double_clicked() {
                        actions.push(Action::Play(TrackRef { playlist: pl.id, uid: pl.tracks[*first].uid }));
                    }
                }
                Row::Track { idx } => {
                    let tr = &pl.tracks[*idx];
                    let r = TrackRef { playlist: pl.id, uid: tr.uid };
                    let rect = Rect::from_min_size(origin + vec2(0.0, y), vec2(width, ROW_H));
                    let resp = ui.interact(rect, ui.scope_id().with(("row", tr.uid)), Sense::click());
                    let selected = pl.selected.contains(&tr.uid);
                    let is_current = app.current == Some(r);
                    let painter = ui.painter_at(rect);
                    let bg = if selected {
                        Some(skin.selection)
                    } else if resp.hovered() {
                        Some(skin.hover)
                    } else if i % 2 == 1 {
                        Some(skin.list_alt)
                    } else {
                        None
                    };
                    if let Some(bg) = bg {
                        painter.rect_filled(rect, 0.0, bg);
                    }
                    if is_current {
                        if !selected {
                            painter.rect_filled(rect, 0.0, skin.accent_dim);
                        }
                        painter.rect_filled(Rect::from_min_size(rect.min, vec2(3.0, ROW_H)), 0.0, skin.accent);
                    }

                    let color = if is_current { skin.accent } else { skin.text };
                    let font = FontId::proportional(13.0);
                    let mut x = rect.left() + if grouped { 14.0 } else { 8.0 };
                    let cy = rect.center().y;

                    // Number column, replaced by a state icon for the current track.
                    let num_w = 30.0;
                    if is_current && playing != PlaybackState::Stopped {
                        let icon = if playing == PlaybackState::Playing { Icon::Play } else { Icon::Pause };
                        paint_icon(&painter, Rect::from_center_size(pos2(x + num_w - 8.0, cy), vec2(10.0, 10.0)), icon, skin.accent);
                    } else {
                        let n = match (grouped, tr.track_no) {
                            (true, Some(n)) => n as usize,
                            _ => idx + 1,
                        };
                        painter.text(pos2(x + num_w - 2.0, cy), Align2::RIGHT_CENTER, format!("{n}."), FontId::proportional(12.0), skin.text_dim);
                    }
                    x += num_w + 8.0;

                    let dur_text = tr.duration.map(fmt_time).unwrap_or_default();
                    let dur_w = 54.0;
                    painter.text(pos2(rect.right() - 10.0, cy), Align2::RIGHT_CENTER, &dur_text, FontId::proportional(12.0), if is_current { skin.accent } else { skin.text_dim });

                    if let Some(qpos) = app.queue.iter().position(|q| *q == r) {
                        let badge = format!("{}", qpos + 1);
                        let br = Rect::from_min_size(pos2(x, cy - 8.0), vec2(18.0, 16.0));
                        painter.rect_filled(br, 3.0, skin.accent);
                        painter.text(br.center(), Align2::CENTER_CENTER, badge, FontId::proportional(10.0), skin.bg);
                        x += 24.0;
                    }

                    let title = if grouped && (tr.artist.is_empty() || tr.artist == tr.album_artist_or_artist()) && !tr.album.is_empty() {
                        tr.title.clone()
                    } else {
                        tr.display_title()
                    };
                    let clip = Rect::from_min_max(pos2(x, rect.top()), pos2(rect.right() - dur_w - 6.0, rect.bottom()));
                    ui.painter_at(clip).text(pos2(x, cy), Align2::LEFT_CENTER, title, font, color);

                    if resp.clicked() {
                        let m = ui.input(|i| i.modifiers);
                        actions.push(Action::Click { uid: tr.uid, ctrl: m.command, shift: m.shift });
                    }
                    if resp.double_clicked() {
                        actions.push(Action::Play(r));
                    }
                    if resp.secondary_clicked() && !selected {
                        actions.push(Action::Click { uid: tr.uid, ctrl: false, shift: false });
                    }
                    let path = tr.path.clone();
                    let in_queue = app.queue.contains(&r);
                    resp.context_menu(|ui| {
                        ui.set_min_width(200.0);
                        if ui.button(t.ctx_play).clicked() {
                            actions.push(Action::Play(r));
                        }
                        let qlabel = if in_queue { t.ctx_unqueue } else { t.ctx_queue };
                        if ui.button(qlabel).clicked() {
                            let mut refs: Vec<TrackRef> = pl
                                .tracks
                                .iter()
                                .filter(|t| pl.selected.contains(&t.uid))
                                .map(|t| TrackRef { playlist: pl.id, uid: t.uid })
                                .collect();
                            if refs.is_empty() {
                                refs.push(r);
                            }
                            actions.push(Action::Queue(refs));
                        }
                        ui.separator();
                        if ui.button(t.select_all).clicked() {
                            actions.push(Action::SelectAll);
                        }
                        if ui.button(t.ctx_remove).clicked() {
                            actions.push(Action::RemoveSelected);
                        }
                        ui.separator();
                        if ui.button(t.ctx_open_folder).clicked() {
                            actions.push(Action::OpenFolder(path.clone()));
                        }
                    });
                }
            }
        }
    });
    app.list_cache = cache;

    for a in actions {
        let pl = &mut app.playlists[pl_idx];
        match a {
            Action::Play(r) => app.play(r),
            Action::Click { uid, ctrl, shift } => {
                if shift {
                    pl.select_range_to(uid);
                } else if ctrl {
                    pl.toggle_selected(uid);
                } else {
                    pl.select_only(uid);
                }
            }
            Action::SelectRange(first, count) => {
                pl.selected = pl.tracks[first..first + count].iter().map(|t| t.uid).collect();
                pl.anchor = Some(pl.tracks[first].uid);
            }
            Action::Queue(refs) => {
                for r in refs {
                    app.toggle_queue(r);
                }
            }
            Action::RemoveSelected => {
                pl.remove_selected();
                app.mark_dirty();
            }
            Action::SelectAll => pl.selected = pl.tracks.iter().map(|t| t.uid).collect(),
            Action::OpenFolder(p) => open_in_file_manager(&p),
        }
    }
}

fn tabs(app: &mut JanmpApp, ui: &mut egui::Ui) {
    let skin = app.skin;
    let t = app.tr();
    let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width(), TABS_H), Sense::hover());
    ui.painter().rect_filled(bar, 0.0, skin.bg);
    ui.painter().line_segment([bar.left_bottom(), bar.right_bottom()], Stroke::new(1.0, skin.border));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(bar.shrink2(vec2(4.0, 0.0))).layout(Layout::left_to_right(Align::Center)));
    let ui = &mut child;

    let active = app.active();
    let mut action = None;
    ScrollArea::horizontal().scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden).show(ui, |ui| {
        ui.horizontal_centered(|ui| {
            for (i, pl) in app.playlists.iter().enumerate() {
                if let Some((ri, name)) = &mut app.renaming {
                    if *ri == i {
                        let resp = ui.add(egui::TextEdit::singleline(name).desired_width(120.0));
                        resp.request_focus();
                        if resp.lost_focus() {
                            action = Some(TabAction::FinishRename);
                        }
                        continue;
                    }
                }
                let is_active = i == active;
                let galley = ui.painter().layout_no_wrap(pl.name.clone(), FontId::proportional(13.0), skin.text);
                let size = vec2(galley.size().x + 20.0, TABS_H - 4.0);
                let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
                let color = if is_active { skin.text_bright } else if resp.hovered() { skin.text } else { skin.text_dim };
                if is_active {
                    ui.painter().rect_filled(rect, 3.0, skin.panel);
                    ui.painter().rect_filled(Rect::from_min_max(pos2(rect.left(), rect.bottom() - 2.0), rect.right_bottom()), 0.0, skin.accent);
                }
                ui.painter().galley(rect.center() - galley.size() / 2.0, galley, color);
                if resp.clicked() {
                    action = Some(TabAction::Switch(i));
                }
                if resp.double_clicked() {
                    app.renaming = Some((i, pl.name.clone()));
                }
                resp.context_menu(|ui| {
                    if ui.button(t.rename_playlist).clicked() {
                        app.renaming = Some((i, pl.name.clone()));
                    }
                    if ui.button(t.clear_playlist).clicked() {
                        action = Some(TabAction::Clear(i));
                    }
                    if ui.button(t.delete_playlist).clicked() {
                        action = Some(TabAction::Close(i));
                    }
                });
            }
            if icon_button(ui, &skin, Icon::Plus, 22.0, false, t.new_playlist).clicked() {
                action = Some(TabAction::New);
            }
        });
    });

    match action {
        Some(TabAction::Switch(i)) => app.settings.active_playlist = i,
        Some(TabAction::New) => app.new_playlist(),
        Some(TabAction::FinishRename) => {
            if let Some((i, name)) = app.renaming.take() {
                if !name.trim().is_empty() {
                    app.playlists[i].name = name.trim().to_string();
                    app.mark_dirty();
                }
            }
        }
        Some(TabAction::Clear(i)) => {
            app.playlists[i].tracks.clear();
            app.playlists[i].selected.clear();
            app.mark_dirty();
        }
        Some(TabAction::Close(i)) => app.close_playlist(i),
        None => {}
    }
}

enum TabAction {
    Switch(usize),
    New,
    FinishRename,
    Clear(usize),
    Close(usize),
}

pub fn footer(app: &mut JanmpApp, ui: &mut egui::Ui) {
    let skin = app.skin;
    let t = app.tr();
    let idx = app.active();
    ui.horizontal_centered(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        let add = icon_button(ui, &skin, Icon::Plus, 24.0, false, t.add);
        egui::Popup::menu(&add).show(|ui| {
            ui.set_min_width(200.0);
            if ui.button(t.add_files).clicked() {
                app.open_files_dialog();
            }
            if ui.button(t.add_folder).clicked() {
                app.open_folder_dialog();
            }
            if ui.button(t.add_url).clicked() {
                app.url_dialog = Some(String::new());
            }
            if ui.button(t.open_playlist).clicked() {
                app.open_playlist_dialog();
            }
        });
        let rem = icon_button(ui, &skin, Icon::Minus, 24.0, false, t.remove);
        egui::Popup::menu(&rem).show(|ui| {
            ui.set_min_width(220.0);
            if ui.button(t.ctx_remove).clicked() {
                app.playlists[idx].remove_selected();
                app.mark_dirty();
            }
            if ui.button(t.remove_missing).clicked() {
                app.playlists[idx].remove_missing();
                app.mark_dirty();
            }
            if ui.button(t.remove_duplicates).clicked() {
                app.playlists[idx].remove_duplicates();
                app.mark_dirty();
            }
            ui.separator();
            if ui.button(t.clear_playlist).clicked() {
                app.playlists[idx].tracks.clear();
                app.mark_dirty();
            }
        });
        let sort = icon_button(ui, &skin, Icon::Sort, 24.0, false, t.sort);
        egui::Popup::menu(&sort).show(|ui| {
            ui.set_min_width(200.0);
            for (label, key) in [
                (t.sort_title, SortKey::Title),
                (t.sort_artist, SortKey::Artist),
                (t.sort_album, SortKey::Album),
                (t.sort_duration, SortKey::Duration),
                (t.sort_path, SortKey::Path),
                (t.sort_random, SortKey::Random),
                (t.sort_reverse, SortKey::Reverse),
            ] {
                if ui.button(label).clicked() {
                    app.playlists[idx].sort(key);
                    app.mark_dirty();
                }
            }
        });
        let group = icon_button(ui, &skin, Icon::Group, 24.0, app.settings.group != GroupMode::None, t.group);
        egui::Popup::menu(&group).show(|ui| {
            ui.set_min_width(180.0);
            for mode in GroupMode::ALL {
                let label = match mode {
                    GroupMode::None => t.group_none,
                    GroupMode::Album => t.group_album,
                    GroupMode::Artist => t.group_artist,
                    GroupMode::Folder => t.group_folder,
                    GroupMode::Genre => t.group_genre,
                    GroupMode::Year => t.group_year,
                };
                if ui.radio(app.settings.group == mode, label).clicked() {
                    app.settings.group = mode;
                    app.mark_dirty();
                }
            }
        });
        ui.add_space(6.0);
        let (icon_rect, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
        paint_icon(ui.painter(), icon_rect.shrink(3.0), Icon::Search, skin.text_dim);
        let search = ui.add(
            egui::TextEdit::singleline(&mut app.search)
                .hint_text(t.search_hint)
                .desired_width(150.0),
        );
        if app.focus_search {
            search.request_focus();
            app.focus_search = false;
        }
        if search.changed() {
            app.epoch += 1;
        }
        if search.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            app.search.clear();
            app.epoch += 1;
        }

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let pl = &app.playlists[idx];
            let stats = format!(
                "{} · {} · {}",
                app.settings.lang.tracks(pl.tracks.len()),
                fmt_time(pl.total_duration()),
                fmt_size(pl.total_size())
            );
            ui.label(RichText::new(stats).size(11.0).color(skin.text_dim));
        });
    });
}

fn open_in_file_manager(path: &std::path::Path) {
    #[cfg(target_os = "windows")]
    let r = std::process::Command::new("explorer").arg(format!("/select,{}", path.display())).spawn();
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").arg("-R").arg(path).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let r = std::process::Command::new("xdg-open").arg(path.parent().unwrap_or(path)).spawn();
    if let Err(e) = r {
        log::warn!("open folder: {e}");
    }
}
