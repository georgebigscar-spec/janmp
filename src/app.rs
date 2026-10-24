//! Application state, playback logic and event handling. Drawing lives in `ui::*`.

use std::collections::VecDeque;
use std::path::PathBuf;

use egui::{Key, Modifiers};

use crate::audio::decoder::StreamInfo;
use crate::audio::{PlaybackState, Player, PlayerEvent};
use crate::config::{self, RepeatMode, Settings};
use crate::i18n::Strings;
use crate::meta::{CoverCache, ScanResult, Scanner};
use crate::playlist::{Playlist, Track};
use crate::ui::skin::Skin;
use crate::ui::spectrum::Spectrum;

/// A playlist entry address that survives sorting and closing other playlists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrackRef {
    pub playlist: u64,
    pub uid: u64,
}

pub struct JanmpApp {
    pub settings: Settings,
    pub skin: Skin,
    pub player: Player,
    pub scanner: Scanner,
    pub covers_big: CoverCache,
    pub covers_thumb: CoverCache,
    pub playlists: Vec<Playlist>,
    pub current: Option<TrackRef>,
    pending_next: Option<TrackRef>,
    pub queue: VecDeque<TrackRef>,
    history: Vec<TrackRef>,
    pub stream_info: Option<StreamInfo>,
    pub spectrum: Spectrum,
    pub search: String,
    pub focus_search: bool,
    pub renaming: Option<(usize, String)>,
    pub show_settings: bool,
    pub toast: Option<(String, f64)>,
    /// Bumped on every playlist mutation; invalidates cached list layouts.
    pub epoch: u64,
    pub scroll_to_current: bool,
    pub list_cache: crate::ui::playlist_view::RowCache,
    error_streak: usize,
    dirty: bool,
    last_save: f64,
}

impl JanmpApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let settings = Settings::load();
        let skin = Skin::new(settings.accent);
        skin.apply(&cc.egui_ctx);

        let ctx = cc.egui_ctx.clone();
        let player = Player::new(settings.volume, move || ctx.request_repaint());
        player.set_volume(if settings.muted { 0.0 } else { settings.volume });
        player.set_balance(settings.balance);
        let ctx = cc.egui_ctx.clone();
        let scanner = Scanner::new(move || ctx.request_repaint());

        let mut playlists = config::load_playlists();
        if playlists.is_empty() {
            playlists.push(Playlist::new(settings.lang.strings().default_playlist));
        }
        // Rescan entries whose tags were never read (e.g. the app quit mid-scan).
        for pl in &playlists {
            for t in pl.tracks.iter().filter(|t| !t.scanned) {
                scanner.request_meta(t.uid, t.path.clone());
            }
        }
        let mut settings = settings;
        settings.active_playlist = settings.active_playlist.min(playlists.len() - 1);

        let mut app = Self {
            settings,
            skin,
            player,
            scanner,
            covers_big: CoverCache::new(256),
            covers_thumb: CoverCache::new(64),
            playlists,
            current: None,
            pending_next: None,
            queue: VecDeque::new(),
            history: Vec::new(),
            stream_info: None,
            spectrum: Spectrum::default(),
            search: String::new(),
            focus_search: false,
            renaming: None,
            show_settings: false,
            toast: None,
            epoch: 0,
            scroll_to_current: false,
            list_cache: Default::default(),
            error_streak: 0,
            dirty: false,
            last_save: 0.0,
        };

        // Files passed on the command line are added to the active playlist and played.
        let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
        if !args.is_empty() {
            let first = app.add_paths(app.settings.active_playlist, &args);
            if let Some(r) = first {
                app.play(r);
            }
        }
        app
    }

    pub fn tr(&self) -> &'static Strings {
        self.settings.lang.strings()
    }

    pub fn active(&self) -> usize {
        self.settings.active_playlist.min(self.playlists.len().saturating_sub(1))
    }

    pub fn playlist_by_id(&self, id: u64) -> Option<&Playlist> {
        self.playlists.iter().find(|p| p.id == id)
    }

    pub fn track(&self, r: TrackRef) -> Option<&Track> {
        self.playlist_by_id(r.playlist)?.get(r.uid)
    }

    pub fn current_track(&self) -> Option<&Track> {
        self.current.and_then(|r| self.track(r))
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
        self.epoch += 1;
    }

    /// Add paths to playlist `idx`; returns the first added entry.
    pub fn add_paths(&mut self, idx: usize, paths: &[PathBuf]) -> Option<TrackRef> {
        let pl = self.playlists.get_mut(idx)?;
        let before = pl.tracks.len();
        let scan = pl.add_paths(paths);
        let first = pl.tracks.get(before).map(|t| TrackRef { playlist: pl.id, uid: t.uid });
        for (uid, path) in scan {
            self.scanner.request_meta(uid, path);
        }
        if let Some(dir) = paths.first().and_then(|p| if p.is_dir() { Some(p.clone()) } else { p.parent().map(|d| d.to_path_buf()) }) {
            self.settings.last_dir = Some(dir);
        }
        self.mark_dirty();
        first
    }

    // ----- playback control -----

    pub fn play(&mut self, r: TrackRef) {
        let Some(t) = self.track(r) else { return };
        let req = t.media_request();
        self.queue.retain(|q| *q != r);
        if let Some(cur) = self.current {
            if cur != r {
                self.history.push(cur);
                if self.history.len() > 500 {
                    self.history.remove(0);
                }
            }
        }
        self.current = Some(r);
        self.pending_next = None;
        self.stream_info = None;
        self.scroll_to_current = true;
        self.player.load(req);
    }

    pub fn play_pause(&mut self) {
        match self.player.state() {
            PlaybackState::Playing => self.player.pause(),
            PlaybackState::Paused => self.player.play(),
            PlaybackState::Stopped => self.play_current_or_selected(),
        }
    }

    pub fn play_current_or_selected(&mut self) {
        let pl = &self.playlists[self.active()];
        let selected = pl.tracks.iter().find(|t| pl.selected.contains(&t.uid)).map(|t| t.uid);
        let r = selected
            .map(|uid| TrackRef { playlist: pl.id, uid })
            .or(self.current.filter(|r| self.track(*r).is_some()))
            .or_else(|| pl.tracks.first().map(|t| TrackRef { playlist: pl.id, uid: t.uid }));
        if let Some(r) = r {
            self.play(r);
        }
    }

    pub fn stop(&mut self) {
        self.pending_next = None;
        self.player.stop();
    }

    pub fn next(&mut self) {
        if let Some(n) = self.next_of(self.current, true) {
            self.play(n);
        }
    }

    pub fn prev(&mut self) {
        if self.player.state() != PlaybackState::Stopped && self.player.position() > 3.0 {
            self.player.seek(0.0);
            return;
        }
        if self.settings.shuffle {
            while let Some(h) = self.history.pop() {
                if self.track(h).is_some() {
                    self.current = None;
                    self.play(h);
                    return;
                }
            }
        }
        let Some(cur) = self.current else { return };
        let Some(pl) = self.playlist_by_id(cur.playlist) else { return };
        let Some(i) = pl.index_of(cur.uid) else { return };
        let n = pl.tracks.len();
        let j = if i > 0 {
            i - 1
        } else if self.settings.repeat == RepeatMode::All {
            n - 1
        } else {
            return;
        };
        let r = TrackRef { playlist: pl.id, uid: pl.tracks[j].uid };
        self.play(r);
    }

    /// The entry that should follow `from`. `manual` = the user pressed "next".
    fn next_of(&mut self, from: Option<TrackRef>, manual: bool) -> Option<TrackRef> {
        while let Some(q) = self.queue.pop_front() {
            if self.track(q).is_some() {
                return Some(q);
            }
        }
        let from = from.filter(|r| self.track(*r).is_some());
        let pl = match from {
            Some(r) => self.playlist_by_id(r.playlist)?,
            None => &self.playlists[self.active()],
        };
        let n = pl.tracks.len();
        if n == 0 {
            return None;
        }
        let at = |i: usize| TrackRef { playlist: pl.id, uid: pl.tracks[i].uid };
        let Some(cur) = from.and_then(|r| pl.index_of(r.uid)) else {
            return Some(at(0));
        };
        if !manual && self.settings.repeat == RepeatMode::One {
            return Some(at(cur));
        }
        if self.settings.shuffle && n > 1 {
            use rand::Rng;
            let mut j = rand::rng().random_range(0..n - 1);
            if j >= cur {
                j += 1;
            }
            return Some(at(j));
        }
        if cur + 1 < n {
            Some(at(cur + 1))
        } else if manual || self.settings.repeat == RepeatMode::All {
            Some(at(0))
        } else {
            None
        }
    }

    pub fn seek_relative(&self, delta: f64) {
        if self.player.state() != PlaybackState::Stopped {
            self.player.seek((self.player.position() + delta).max(0.0));
        }
    }

    pub fn set_volume(&mut self, v: f32) {
        self.settings.volume = v.clamp(0.0, 1.0);
        self.settings.muted = false;
        self.player.set_volume(self.settings.volume);
        self.dirty = true;
    }

    pub fn toggle_mute(&mut self) {
        self.settings.muted = !self.settings.muted;
        self.player.set_volume(if self.settings.muted { 0.0 } else { self.settings.volume });
        self.dirty = true;
    }

    pub fn toggle_queue(&mut self, r: TrackRef) {
        if let Some(pos) = self.queue.iter().position(|q| *q == r) {
            self.queue.remove(pos);
        } else {
            self.queue.push_back(r);
        }
    }

    pub fn show_error(&mut self, msg: String, now: f64) {
        log::warn!("{msg}");
        self.toast = Some((msg, now + 6.0));
    }

    // ----- events -----

    fn process_events(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        while let Some(ev) = self.player.poll_event() {
            log::debug!("player event: {ev:?} (current {:?}, pending {:?})", self.current, self.pending_next);
            match ev {
                PlayerEvent::Started { info, duration } => {
                    if let Some(p) = self.pending_next.take() {
                        self.current = Some(p);
                    }
                    self.error_streak = 0;
                    self.stream_info = Some(info);
                    self.fix_duration(duration);
                }
                PlayerEvent::AboutToFinish => {
                    if let Some(n) = self.next_of(self.current, false) {
                        if let Some(t) = self.track(n) {
                            let req = t.media_request();
                            self.pending_next = Some(n);
                            self.player.enqueue(req);
                        }
                    }
                }
                PlayerEvent::TrackChanged { info, duration } => {
                    if let Some(p) = self.pending_next.take() {
                        if let Some(c) = self.current {
                            self.history.push(c);
                        }
                        self.current = Some(p);
                        self.scroll_to_current = true;
                    }
                    self.error_streak = 0;
                    self.stream_info = Some(info);
                    self.fix_duration(duration);
                }
                PlayerEvent::Finished => {
                    self.pending_next = None;
                }
                PlayerEvent::Error(msg) => {
                    let failed = self.pending_next.take().or(self.current);
                    let t = self.tr().playback_error;
                    self.show_error(format!("{t}: {msg}"), now);
                    self.error_streak += 1;
                    if failed.is_some() && self.error_streak < 30 {
                        self.current = failed;
                        if let Some(n) = self.next_of(failed, true) {
                            if Some(n) != failed {
                                self.play(n);
                            }
                        }
                    }
                }
            }
        }

        let mut changed = false;
        while let Ok(r) = self.scanner.results.try_recv() {
            match r {
                ScanResult::Meta { uid, meta } => {
                    for pl in &mut self.playlists {
                        if let Some(t) = pl.get_mut(uid) {
                            t.apply_meta(meta);
                            changed = true;
                            break;
                        }
                    }
                }
                ScanResult::Cover { path, max, image } => {
                    if max == self.covers_big.max {
                        self.covers_big.insert(ctx, path, image);
                    } else {
                        self.covers_thumb.insert(ctx, path, image);
                    }
                }
            }
        }
        if changed {
            self.mark_dirty();
        }

        if self.dirty && now - self.last_save > 5.0 {
            self.save();
            self.last_save = now;
        }
    }

    /// Decoders know exact durations; keep the playlist entry in sync.
    fn fix_duration(&mut self, duration: Option<f64>) {
        let (Some(r), Some(d)) = (self.current, duration) else { return };
        let Some(pl) = self.playlists.iter_mut().find(|p| p.id == r.playlist) else { return };
        if let Some(t) = pl.get_mut(r.uid) {
            if t.duration.is_none_or(|old| (old - d).abs() > 1.0) {
                t.duration = Some(d);
                self.epoch += 1;
                self.dirty = true;
            }
        }
    }

    pub fn save(&mut self) {
        self.settings.save();
        config::save_playlists(&self.playlists);
        self.dirty = false;
    }

    fn handle_hotkeys(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let mut actions: Vec<fn(&mut JanmpApp)> = Vec::new();
        ctx.input_mut(|i| {
            let none = Modifiers::NONE;
            let ctrl = Modifiers::COMMAND;
            if i.consume_key(none, Key::Space) || i.consume_key(none, Key::X) {
                actions.push(|a| a.play_pause());
            }
            if i.consume_key(none, Key::C) {
                actions.push(|a| a.player.pause());
            }
            if i.consume_key(none, Key::V) {
                actions.push(|a| a.stop());
            }
            if i.consume_key(none, Key::Z) {
                actions.push(|a| a.prev());
            }
            if i.consume_key(none, Key::B) {
                actions.push(|a| a.next());
            }
            if i.consume_key(none, Key::ArrowLeft) {
                actions.push(|a| a.seek_relative(-5.0));
            }
            if i.consume_key(none, Key::ArrowRight) {
                actions.push(|a| a.seek_relative(5.0));
            }
            if i.consume_key(ctrl, Key::ArrowUp) {
                actions.push(|a| a.set_volume(a.settings.volume + 0.05));
            }
            if i.consume_key(ctrl, Key::ArrowDown) {
                actions.push(|a| a.set_volume(a.settings.volume - 0.05));
            }
            if i.consume_key(ctrl, Key::F) {
                actions.push(|a| a.focus_search = true);
            }
            if i.consume_key(ctrl, Key::A) {
                actions.push(|a| {
                    let idx = a.active();
                    let pl = &mut a.playlists[idx];
                    pl.selected = pl.tracks.iter().map(|t| t.uid).collect();
                });
            }
            if i.consume_key(none, Key::Delete) {
                actions.push(|a| {
                    let idx = a.active();
                    a.playlists[idx].remove_selected();
                    a.mark_dirty();
                });
            }
            if i.consume_key(none, Key::Q) {
                actions.push(|a| {
                    let pl = &a.playlists[a.active()];
                    let refs: Vec<TrackRef> = pl
                        .tracks
                        .iter()
                        .filter(|t| pl.selected.contains(&t.uid))
                        .map(|t| TrackRef { playlist: pl.id, uid: t.uid })
                        .collect();
                    for r in refs {
                        a.toggle_queue(r);
                    }
                });
            }
            if i.consume_key(none, Key::Enter) {
                actions.push(|a| a.play_current_or_selected());
            }
            if i.consume_key(Modifiers::ALT, Key::ArrowUp) {
                actions.push(|a| {
                    let idx = a.active();
                    a.playlists[idx].move_selected(-1);
                    a.mark_dirty();
                });
            }
            if i.consume_key(Modifiers::ALT, Key::ArrowDown) {
                actions.push(|a| {
                    let idx = a.active();
                    a.playlists[idx].move_selected(1);
                    a.mark_dirty();
                });
            }
        });
        for act in actions {
            act(self);
        }
    }

    fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()).collect());
        if !dropped.is_empty() {
            let idx = self.active();
            self.add_paths(idx, &dropped);
        }
    }

    pub fn open_files_dialog(&mut self) {
        let t = self.tr();
        let mut dlg = rfd::FileDialog::new()
            .add_filter(t.audio_files, &crate::audio::formats::all_audio_exts())
            .add_filter(t.playlists, crate::audio::formats::PLAYLIST_EXTS);
        if let Some(d) = &self.settings.last_dir {
            dlg = dlg.set_directory(d);
        }
        if let Some(files) = dlg.pick_files() {
            let idx = self.active();
            self.add_paths(idx, &files);
        }
    }

    pub fn open_folder_dialog(&mut self) {
        let mut dlg = rfd::FileDialog::new();
        if let Some(d) = &self.settings.last_dir {
            dlg = dlg.set_directory(d);
        }
        if let Some(dirs) = dlg.pick_folders() {
            let idx = self.active();
            self.add_paths(idx, &dirs);
        }
    }

    pub fn open_playlist_dialog(&mut self) {
        let t = self.tr();
        if let Some(file) = rfd::FileDialog::new().add_filter(t.playlists, crate::audio::formats::PLAYLIST_EXTS).pick_file() {
            let name = file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            self.playlists.push(Playlist::new(name));
            let idx = self.playlists.len() - 1;
            self.settings.active_playlist = idx;
            self.add_paths(idx, &[file]);
        }
    }

    pub fn save_playlist_dialog(&mut self) {
        let pl = &self.playlists[self.active()];
        if let Some(file) = rfd::FileDialog::new()
            .add_filter("M3U8", &["m3u8"])
            .set_file_name(format!("{}.m3u8", pl.name))
            .save_file()
        {
            if let Err(e) = crate::playlist::m3u::save_m3u8(&file, &pl.tracks) {
                self.toast = Some((e.to_string(), f64::MAX));
            }
        }
    }

    pub fn new_playlist(&mut self) {
        self.playlists.push(Playlist::new(self.tr().new_playlist));
        self.settings.active_playlist = self.playlists.len() - 1;
        self.mark_dirty();
    }

    pub fn close_playlist(&mut self, idx: usize) {
        if idx >= self.playlists.len() {
            return;
        }
        let id = self.playlists[idx].id;
        self.playlists.remove(idx);
        self.queue.retain(|q| q.playlist != id);
        if self.playlists.is_empty() {
            self.playlists.push(Playlist::new(self.tr().default_playlist));
        }
        if self.settings.active_playlist >= idx && self.settings.active_playlist > 0 {
            self.settings.active_playlist -= 1;
        }
        self.mark_dirty();
    }
}

impl eframe::App for JanmpApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.process_events(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.handle_dropped_files(&ctx);
        self.handle_hotkeys(&ctx);
        crate::ui::draw(self, ui);
        if self.player.state() == PlaybackState::Playing || !self.spectrum.is_silent() {
            ctx.request_repaint_after(std::time::Duration::from_millis(33));
        }
    }

    fn save(&mut self, _storage: &mut dyn eframe::Storage) {
        self.save();
    }
}
