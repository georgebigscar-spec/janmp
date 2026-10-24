//! The top "player" area: cover, track info, time, spectrum, seek bar and transport controls.

use egui::{vec2, Align, FontId, Layout, RichText, Sense};

use super::widgets::{fmt_time, icon_button, paint_icon, paint_star, marquee, mini_slider, round_button, seek_bar, Icon};
use crate::app::JanmpApp;
use crate::audio::PlaybackState;
use crate::config::RepeatMode;

const COVER: f32 = 92.0;

pub fn show(app: &mut JanmpApp, ui: &mut egui::Ui) {
    let skin = app.skin;
    let t = app.tr();
    let state = app.player.state();
    let now = ui.input(|i| i.time);
    let active = state == PlaybackState::Playing;
    if app.settings.show_spectrum {
        app.spectrum.update(&app.player, now, active);
    }

    let track = app.current_track().cloned();
    let pos = app.player.position();
    let dur = app.player.duration().or(track.as_ref().and_then(|t| t.duration));

    ui.horizontal(|ui| {
        // Cover art.
        let (rect, _) = ui.allocate_exact_size(vec2(COVER, COVER), Sense::hover());
        let tex = track.as_ref().and_then(|tr| app.covers_big.get(&tr.path, &app.scanner));
        match tex {
            Some(tex) => {
                egui::Image::new(&tex).corner_radius(3.0).paint_at(ui, rect);
            }
            None => {
                ui.painter().rect_filled(rect, 3.0, skin.list_bg);
                paint_icon(ui.painter(), rect.shrink(COVER * 0.3), Icon::Note, skin.text_dim.gamma_multiply(0.4));
            }
        }

        // Right column: time + spectrum.
        let right_w = 150.0;
        let info_w = (ui.available_width() - right_w - 8.0).max(60.0);
        ui.allocate_ui_with_layout(vec2(info_w, COVER), Layout::top_down(Align::Min), |ui| {
            ui.add_space(4.0);
            match &track {
                Some(tr) => {
                    let (first, mut second) = match (&app.stream_title, tr.is_stream()) {
                        (Some(song), true) => (song.clone(), tr.title.clone()),
                        (None, true) => (tr.title.clone(), t.radio.to_string()),
                        _ => (tr.title.clone(), tr.artist.clone()),
                    };
                    marquee(ui, &first, FontId::proportional(17.0), skin.text_bright);
                    ui.add_space(2.0);
                    if !tr.album.is_empty() && !tr.is_stream() {
                        if !second.is_empty() {
                            second.push_str(" — ");
                        }
                        second.push_str(&tr.album);
                        if let Some(y) = tr.year {
                            second.push_str(&format!(" ({y})"));
                        }
                    }
                    marquee(ui, &second, FontId::proportional(13.0), skin.text);
                    ui.add_space(6.0);
                    let tech = match &app.stream_info {
                        Some(si) => {
                            let mut s = format!("{} · {:.1} kHz", si.codec, si.sample_rate as f64 / 1000.0)
                                .replace(".0 kHz", " kHz");
                            if let Some(br) = si.bitrate.or(tr.bitrate) {
                                s.push_str(&format!(" · {br} kbps"));
                            }
                            if let Some(b) = si.bits_per_sample {
                                s.push_str(&format!(" · {b} bit"));
                            }
                            s.push_str(match si.channels {
                                1 => " · Mono",
                                2 => " · Stereo",
                                _ => "",
                            });
                            if si.channels > 2 {
                                s.push_str(&format!(" · {} ch", si.channels));
                            }
                            s
                        }
                        None => tr.tech_summary(),
                    };
                    ui.add(egui::Label::new(RichText::new(tech).size(11.0).color(skin.text_dim)).truncate());
                    if !tr.is_stream() {
                        if let (Some(r), Some(k)) = (app.current, rating_stars(ui, &skin, app.rating_of(tr))) {
                            let same = app.rating_of(tr) == Some((k, true));
                            app.set_rating(r, if same { None } else { Some(k) });
                        }
                    }
                    let status = match state {
                        _ if app.connecting => t.connecting,
                        _ if app.recording => "● REC",
                        PlaybackState::Paused => t.paused,
                        PlaybackState::Stopped => t.stopped,
                        PlaybackState::Playing => "",
                    };
                    if !status.is_empty() {
                        ui.label(RichText::new(status).size(11.0).color(skin.accent));
                    }
                }
                None => {
                    ui.add_space(22.0);
                    ui.label(RichText::new(t.nothing_playing).size(15.0).color(skin.text_dim));
                }
            }
        });

        ui.allocate_ui_with_layout(vec2(right_w, COVER), Layout::top_down(Align::Max), |ui| {
            let shown = if app.settings.show_remaining {
                dur.map(|d| format!("-{}", fmt_time((d - pos).max(0.0)))).unwrap_or_else(|| fmt_time(pos))
            } else {
                fmt_time(pos)
            };
            let time = ui
                .add(
                    egui::Label::new(RichText::new(shown).size(30.0).color(skin.text_bright).monospace())
                        .sense(Sense::click()),
                )
                .on_hover_text(t.time_toggle_hint);
            if time.clicked() {
                app.settings.show_remaining = !app.settings.show_remaining;
            }
            if app.settings.show_spectrum {
                ui.add_space(2.0);
                let resp = app.spectrum.show(ui, &skin, vec2(right_w, 40.0));
                if resp.clicked() {
                    app.settings.show_spectrum = false;
                }
            }
        });
    });

    ui.add_space(6.0);
    let frac = match dur {
        Some(d) if d > 0.0 => (pos / d) as f32,
        _ => 0.0,
    };
    let seekable = state != PlaybackState::Stopped && dur.is_some();
    let (resp, picked) = seek_bar(ui, &skin, frac, 14.0, seekable);
    if let (Some(f), Some(d)) = (picked, dur) {
        app.seek_to(f as f64 * d);
    }
    if seekable {
        if let (Some(p), Some(d)) = (resp.hover_pos(), dur) {
            let f = ((p.x - resp.rect.left()) / resp.rect.width()).clamp(0.0, 1.0) as f64;
            resp.on_hover_text_at_pointer(fmt_time(f * d));
        }
    }
    ui.horizontal(|ui| {
        ui.label(RichText::new(fmt_time(pos)).size(11.0).color(skin.text_dim));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new(dur.map(fmt_time).unwrap_or_else(|| "--:--".into())).size(11.0).color(skin.text_dim));
        });
    });

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        if icon_button(ui, &skin, Icon::Prev, 30.0, false, t.prev).clicked() {
            app.prev();
        }
        let (icon, tip) = if state == PlaybackState::Playing { (Icon::Pause, t.pause) } else { (Icon::Play, t.play) };
        if round_button(ui, &skin, icon, 38.0, tip).clicked() {
            app.play_pause();
        }
        if icon_button(ui, &skin, Icon::Stop, 30.0, false, t.stop).clicked() {
            app.stop();
        }
        if icon_button(ui, &skin, Icon::Next, 30.0, false, t.next).clicked() {
            app.next();
        }
        ui.add_space(10.0);
        if icon_button(ui, &skin, Icon::Shuffle, 28.0, app.settings.shuffle, t.shuffle).clicked() {
            app.settings.shuffle = !app.settings.shuffle;
            app.mark_dirty();
        }
        let (ricon, rtip, ron) = match app.settings.repeat {
            RepeatMode::Off => (Icon::Repeat, t.repeat_off, false),
            RepeatMode::All => (Icon::Repeat, t.repeat_all, true),
            RepeatMode::One => (Icon::RepeatOne, t.repeat_one, true),
        };
        if icon_button(ui, &skin, ricon, 28.0, ron, rtip).clicked() {
            app.settings.repeat = app.settings.repeat.cycle();
            app.mark_dirty();
        }
        if app.current_is_stream() && state != PlaybackState::Stopped {
            let tip = if app.recording { t.stop_recording } else { t.record };
            if icon_button(ui, &skin, Icon::Record, 28.0, app.recording, tip).clicked() {
                app.toggle_recording();
            }
        }

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let mut vol = if app.settings.muted { 0.0 } else { app.settings.volume };
            let r = mini_slider(ui, &skin, &mut vol, (0.0, 1.0), 90.0, false)
                .on_hover_text(format!("{}: {}%", t.volume, (vol * 100.0).round()));
            if r.changed() {
                app.set_volume(vol);
            }
            let vicon = if app.settings.muted || app.settings.volume == 0.0 { Icon::Mute } else { Icon::Volume };
            if icon_button(ui, &skin, vicon, 26.0, false, t.mute).clicked() {
                app.toggle_mute();
            }
            let mut bal = app.settings.balance;
            let r = mini_slider(ui, &skin, &mut bal, (-1.0, 1.0), 44.0, true)
                .on_hover_text(format!("{}: {:+.0}%", t.balance, bal * 100.0));
            if r.double_clicked() {
                bal = 0.0;
            }
            if r.changed() || r.double_clicked() {
                // Snap to centre near zero.
                if bal.abs() < 0.05 {
                    bal = 0.0;
                }
                app.settings.balance = bal;
                app.player.set_balance(bal);
                app.mark_dirty();
            }
        });
    });
}

/// Clickable five-star rating; returns the star the user clicked.
fn rating_stars(ui: &mut egui::Ui, skin: &super::skin::Skin, rating: Option<(u8, bool)>) -> Option<u8> {
    const STEP: f32 = 13.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(STEP * 5.0, 14.0), Sense::click());
    let hover = resp.hover_pos().map(|p| (((p.x - rect.left()) / STEP) as u8 + 1).min(5));
    for k in 1..=5u8 {
        let c = egui::pos2(rect.left() + STEP * (k as f32 - 0.5), rect.center().y);
        let fill = match (hover, rating) {
            (Some(h), _) if k <= h => Some(skin.text_bright),
            (None, Some((r, manual))) if k <= r => Some(if manual { skin.accent } else { skin.text_dim }),
            _ => None,
        };
        paint_star(ui.painter(), c, 5.5, fill, skin.text_dim.gamma_multiply(0.7));
    }
    if resp.clicked() { hover } else { None }
}
