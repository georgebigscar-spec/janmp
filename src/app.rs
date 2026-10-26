//! Application state, playback logic and event handling. Drawing lives in `ui::*`.

use std::collections::VecDeque;
use std::path::PathBuf;

use crate::audio::decoder::StreamInfo;
use crate::audio::{PlaybackState, Player, PlayerEvent};
use crate::config::{self, RepeatMode, Settings};
use crate::hotkeys::{Action, Capture, GlobalHotkeys, Shortcut};
use crate::i18n::Strings;
use crate::meta::{CoverCache, ScanResult, Scanner};
use crate::playlist::{Playlist, Track};
use crate::ui::skin::Skin;
use crate::stats::{Bookmark, Library};
use crate::ui::spectrum::Spectrum;

type YtdlResult = (TrackRef, Result<Vec<crate::audio::ytdl::Entry>, String>);

/// Time actually heard of the current track, for statistics.
struct Listen {
    track: TrackRef,
    listened: f64,
    last_pos: f64,
}

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
    /// Message, expiry time, is-error.
    pub toast: Option<(String, f64, bool)>,
    /// "Now playing" announced by the current radio station.
    pub stream_title: Option<String>,
    pub recording: bool,
    /// Waiting for the engine to open the current item (radio can take seconds).
    pub connecting: bool,
    /// Text of the "add URL" dialog while it is open.
    pub url_dialog: Option<String>,
    pub library: Library,
    listen: Option<Listen>,
    /// Position deltas are ignored until this time (a seek happened).
    seek_guard_until: f64,
    /// Seek to apply once the next item has started (bookmarks).
    pending_seek: Option<f64>,
    pub show_stats: bool,
    pub show_hotkeys: bool,
    pub show_eq: bool,
    /// Name being typed for a new equalizer preset.
    pub eq_new_preset: Option<String>,
    /// Hotkey editor waiting for a key press.
    pub hotkey_capture: Option<Capture>,
    pub global_keys: GlobalHotkeys,
    system_controls: crate::media_controls::SystemControls,
    pub waveforms: crate::waveform::WaveformService,
    ytdl_tx: crossbeam_channel::Sender<YtdlResult>,
    ytdl_rx: crossbeam_channel::Receiver<YtdlResult>,
    library_dirty: bool,
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
        let skin = Skin::new(settings.theme, settings.accent);
        skin.apply(&cc.egui_ctx);

        let ctx = cc.egui_ctx.clone();
        let player = Player::new(settings.volume, move || ctx.request_repaint());
        player.set_volume(if settings.muted { 0.0 } else { settings.volume });
        player.set_balance(settings.balance);
        let ctx = cc.egui_ctx.clone();
        let scanner = Scanner::new(move || ctx.request_repaint());

        crate::audio::midi::set_soundfont(settings.soundfont.clone());
        player.set_eq(settings.eq.enabled, settings.eq.preamp_db, &settings.eq.gains);
        player.set_replaygain(&settings.replaygain);
        let ctx = cc.egui_ctx.clone();
        let mut global_keys = GlobalHotkeys::new(move || ctx.request_repaint());
        global_keys.apply(&settings.hotkeys);

        let (ytdl_tx, ytdl_rx) = crossbeam_channel::unbounded();

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
            stream_title: None,
            recording: false,
            connecting: false,
            url_dialog: None,
            library: Library::load(),
            listen: None,
            seek_guard_until: 0.0,
            pending_seek: None,
            show_stats: false,
            show_hotkeys: false,
            show_eq: false,
            eq_new_preset: None,
            hotkey_capture: None,
            global_keys,
            system_controls: crate::media_controls::SystemControls::new(cc),
            waveforms: {
                let ctx = cc.egui_ctx.clone();
                crate::waveform::WaveformService::new(move || ctx.request_repaint())
            },
            ytdl_tx,
            ytdl_rx,
            library_dirty: false,
            epoch: 0,
            scroll_to_current: false,
            list_cache: Default::default(),
            error_streak: 0,
            dirty: false,
            last_save: 0.0,
        };

        // Files passed on the command line are added to the active playlist and played.
        let (urls, args): (Vec<PathBuf>, Vec<PathBuf>) = std::env::args_os()
            .skip(1)
            .map(PathBuf::from)
            .partition(|p| crate::audio::stream::is_stream_url(&p.to_string_lossy()));
        for u in &urls {
            app.add_url(&u.to_string_lossy());
        }
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

    /// Settings changed; the playlist layout did not.
    pub fn mark_settings_dirty(&mut self) {
        self.dirty = true;
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
        if self.track(r).is_none() {
            return;
        }
        self.finish_listen(true);
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
        self.stream_title = None;
        self.recording = false;
        self.connecting = true;
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
        self.finish_listen(true);
        self.pending_next = None;
        self.recording = false;
        self.connecting = false;
        self.player.stop();
    }

    pub fn next(&mut self) {
        if let Some(n) = self.next_of(self.current, true) {
            self.play(n);
        }
    }

    pub fn prev(&mut self) {
        if self.player.state() != PlaybackState::Stopped && self.player.position() > 3.0 {
            self.seek_to(0.0);
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

    pub fn seek_relative(&mut self, delta: f64) {
        if self.player.state() != PlaybackState::Stopped {
            self.seek_to((self.player.position() + delta).max(0.0));
        }
    }

    pub fn seek_to(&mut self, secs: f64) {
        self.seek_guard_until = f64::MAX;
        self.player.seek(secs);
    }

    // ----- statistics, ratings, bookmarks -----

    fn start_listen(&mut self) {
        self.listen = self
            .current
            .filter(|r| self.track(*r).is_some_and(|t| !t.is_stream()))
            .map(|r| Listen { track: r, listened: 0.0, last_pos: self.player.position() });
    }

    /// Close the current listening session and account it in the statistics.
    fn finish_listen(&mut self, by_user: bool) {
        let Some(l) = self.listen.take() else { return };
        let Some(t) = self.track(l.track).cloned() else { return };
        self.library.record_listen(&t, l.listened, t.duration, by_user);
        self.library_dirty = true;
        self.epoch += 1;
    }

    fn sample_listen(&mut self, now: f64) {
        if self.seek_guard_until == f64::MAX {
            // The seek was just sent; ignore movement for a moment while it lands.
            self.seek_guard_until = now + 0.4;
        }
        let pos = self.player.position();
        let playing = self.player.state() == PlaybackState::Playing;
        let Some(l) = self.listen.as_mut() else { return };
        let d = pos - l.last_pos;
        l.last_pos = pos;
        if playing && now >= self.seek_guard_until && d > 0.0 && d < 600.0 {
            l.listened += d;
        }
    }

    pub fn rating_of(&self, t: &Track) -> Option<(u8, bool)> {
        self.library.get(t).and_then(|s| s.effective_rating())
    }

    pub fn set_rating(&mut self, r: TrackRef, rating: Option<u8>) {
        if let Some(t) = self.track(r).cloned() {
            self.library.set_rating(&t, rating);
            self.library_dirty = true;
            self.epoch += 1;
        }
    }

    pub fn add_bookmark(&mut self) {
        let Some(t) = self.current_track().cloned() else { return };
        if t.is_stream() || self.player.state() == PlaybackState::Stopped {
            return;
        }
        self.library.bookmarks.push(Bookmark {
            path: t.path.clone(),
            cue_number: t.cue.as_ref().map(|c| c.number),
            title: t.display_title(),
            position: self.player.position(),
            created: crate::stats::now_secs(),
        });
        self.library_dirty = true;
        let now = self.player.position();
        let msg = format!("{}: {} @ {}", self.tr().bookmark_added, t.display_title(), crate::ui::widgets::fmt_time(now));
        self.toast = Some((msg, f64::NEG_INFINITY, false));
    }

    pub fn open_bookmark(&mut self, i: usize) {
        let Some(b) = self.library.bookmarks.get(i).cloned() else { return };
        let matches = |t: &Track| t.path == b.path && t.cue.as_ref().map(|c| c.number) == b.cue_number;
        let found = self
            .playlists
            .iter()
            .find_map(|pl| pl.tracks.iter().find(|t| matches(t)).map(|t| TrackRef { playlist: pl.id, uid: t.uid }));
        let r = match found {
            Some(r) => Some(r),
            None => {
                let idx = self.active();
                self.add_paths(idx, std::slice::from_ref(&b.path));
                let pl = &self.playlists[idx];
                pl.tracks.iter().rev().find(|t| matches(t)).map(|t| TrackRef { playlist: pl.id, uid: t.uid })
            }
        };
        if let Some(r) = r {
            self.play(r);
            self.pending_seek = Some(b.position);
        }
    }

    pub fn remove_bookmark(&mut self, i: usize) {
        if i < self.library.bookmarks.len() {
            self.library.bookmarks.remove(i);
            self.library_dirty = true;
        }
    }

    /// Sorting that needs the statistics (rating, play count).
    pub fn sort_by_stats(&mut self, by_rating: bool) {
        let idx = self.active();
        let lib = &self.library;
        self.playlists[idx].tracks.sort_by_cached_key(|t| {
            let s = lib.get(t);
            let key = if by_rating {
                s.and_then(|s| s.effective_rating()).map(|(r, manual)| r as u32 * 2 + manual as u32).unwrap_or(0)
            } else {
                s.map(|s| s.plays).unwrap_or(0)
            };
            std::cmp::Reverse(key)
        });
        self.mark_dirty();
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
        self.toast = Some((msg, now + 6.0, true));
    }

    pub fn show_info(&mut self, msg: String, now: f64) {
        self.toast = Some((msg, now + 6.0, false));
    }

    /// Stations added by bare URL take the name the server announces.
    fn adopt_station_name(&mut self, station: Option<&str>) {
        let (Some(r), Some(name)) = (self.current, station) else { return };
        let Some(pl) = self.playlists.iter_mut().find(|p| p.id == r.playlist) else { return };
        if let Some(t) = pl.get_mut(r.uid) {
            if t.is_stream() && t.title == t.path.to_string_lossy() {
                t.title = name.to_string();
                self.mark_dirty();
            }
        }
    }

    pub fn current_is_stream(&self) -> bool {
        self.current_track().is_some_and(|t| t.is_stream())
    }

    pub fn toggle_recording(&mut self) {
        if self.recording {
            self.player.record(None);
        } else if self.current_is_stream() && self.player.state() != PlaybackState::Stopped {
            self.player.record(Some(self.settings.record.clone()));
        }
    }

    pub fn add_url(&mut self, url: &str) {
        let url = url.trim();
        if url.is_empty() {
            return;
        }
        let idx = self.active();
        let lower = url.to_ascii_lowercase();
        let is_playlist = [".m3u", ".m3u8", ".pls"].iter().any(|e| lower.split('?').next().unwrap_or("").ends_with(e));
        let tracks: Vec<Track> = if is_playlist {
            // A station playlist: fetch it and add the streams it lists.
            match fetch_text(url) {
                Ok(text) => {
                    let base = std::path::Path::new("");
                    let entries = if lower.contains(".pls") {
                        crate::playlist::m3u::parse_pls(&text, base)
                    } else {
                        crate::playlist::m3u::parse_m3u(&text, base)
                    };
                    entries
                        .into_iter()
                        .filter_map(|e| match e {
                            crate::playlist::m3u::Entry::Url { url, title } => Some(Track::from_url(&url, title)),
                            _ => None,
                        })
                        .collect()
                }
                Err(e) => {
                    self.toast = Some((e, f64::MAX, true));
                    return;
                }
            }
        } else if crate::audio::ytdl::is_ytdl_url(url) {
            // A placeholder now; titles (or the whole playlist) arrive from yt-dlp.
            let t = Track::from_url(url, None);
            let r = TrackRef { playlist: self.playlists[idx].id, uid: t.uid };
            let tx = self.ytdl_tx.clone();
            let url = url.to_string();
            std::thread::spawn(move || {
                let _ = tx.send((r, crate::audio::ytdl::list_entries(&url)));
            });
            self.show_info(self.tr().resolving.to_string(), f64::NEG_INFINITY);
            vec![t]
        } else {
            vec![Track::from_url(url, None)]
        };
        let pl = &mut self.playlists[idx];
        let first = tracks.first().map(|t| TrackRef { playlist: pl.id, uid: t.uid });
        pl.tracks.extend(tracks);
        self.mark_dirty();
        if let Some(r) = first {
            self.play(r);
        }
    }

    /// Add the tracks of the audio CD in the drive to the active playlist.
    pub fn add_audio_cd(&mut self) {
        let idx = self.active();
        let roots = crate::audio::cdda::cda_roots();
        if !roots.is_empty() {
            if let Some(r) = self.add_paths(idx, &roots) {
                self.play(r);
            }
            return;
        }
        let t = self.tr();
        let discs = crate::audio::cdda::find_discs();
        if discs.is_empty() {
            self.show_error(t.no_audio_cd.to_string(), f64::NEG_INFINITY);
            return;
        }
        let mut tracks = Vec::new();
        for (dev, toc) in discs {
            for ct in toc {
                let mut tr = Track::from_url(&crate::audio::cdda::cd_url(&dev, ct.number), Some(format!("{} {:02}", t.cd_track, ct.number)));
                tr.album = t.audio_cd.to_string();
                tr.track_no = Some(ct.number);
                tr.duration = Some(ct.duration());
                tracks.push(tr);
            }
        }
        let pl = &mut self.playlists[idx];
        let first = tracks.first().map(|t| TrackRef { playlist: pl.id, uid: t.uid });
        pl.tracks.extend(tracks);
        self.mark_dirty();
        if let Some(r) = first {
            self.play(r);
        }
    }

    /// Resolve the next YouTube entry while this one plays, so it starts without a pause.
    fn prefetch_next(&self) {
        if self.settings.shuffle && self.queue.is_empty() {
            return;
        }
        let next = self.queue.front().copied().or_else(|| {
            let cur = self.current?;
            let pl = self.playlist_by_id(cur.playlist)?;
            let i = pl.index_of(cur.uid)?;
            pl.tracks.get(i + 1).map(|t| TrackRef { playlist: pl.id, uid: t.uid })
        });
        if let Some(t) = next.and_then(|r| self.track(r)) {
            let url = t.path.to_string_lossy();
            if crate::audio::ytdl::is_ytdl_url(&url) {
                crate::audio::ytdl::prefetch(&url);
            }
        }
    }

    /// Fill in titles from yt-dlp, expanding a playlist page into its entries.
    fn apply_ytdl(&mut self, r: TrackRef, result: Result<Vec<crate::audio::ytdl::Entry>, String>) {
        let entries = match result {
            Ok(e) => e,
            Err(e) => {
                let t = self.tr();
                let text = if e.contains(crate::audio::ytdl::YTDLP_MISSING) { t.ytdlp_missing.to_string() } else { e };
                self.show_error(text, f64::NEG_INFINITY);
                return;
            }
        };
        let Some(pl) = self.playlists.iter_mut().find(|p| p.id == r.playlist) else { return };
        let Some(pos) = pl.index_of(r.uid) else { return };
        let fill = |t: &mut Track, e: &crate::audio::ytdl::Entry| {
            t.path = PathBuf::from(&e.url);
            if !e.title.is_empty() {
                t.title = e.title.clone();
            }
            t.artist = e.artist.clone();
            t.album = e.album.clone();
            t.duration = e.duration.or(t.duration);
        };
        let mut rest = Vec::new();
        for (i, e) in entries.iter().enumerate() {
            if i == 0 {
                // Keep the playing placeholder's identity; a playlist page now plays its first item.
                fill(&mut pl.tracks[pos], e);
            } else {
                let mut t = Track::from_url(&e.url, None);
                fill(&mut t, e);
                rest.push(t);
            }
        }
        let n = rest.len();
        pl.tracks.splice(pos + 1..pos + 1, rest);
        if n > 0 && self.current == Some(r) {
            // The placeholder pointed at the playlist page itself; start its first item.
            self.play(r);
        }
        self.mark_dirty();
    }

    // ----- events -----

    fn process_events(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        while let Ok((r, result)) = self.ytdl_rx.try_recv() {
            self.apply_ytdl(r, result);
        }
        while let Some(ev) = self.player.poll_event() {
            log::debug!("player event: {ev:?} (current {:?}, pending {:?})", self.current, self.pending_next);
            match ev {
                PlayerEvent::Started { info, duration } => {
                    if let Some(p) = self.pending_next.take() {
                        self.current = Some(p);
                    }
                    self.connecting = false;
                    self.error_streak = 0;
                    self.adopt_station_name(info.station.as_deref());
                    if let Some(pos) = self.pending_seek.take() {
                        self.seek_to(pos);
                    }
                    self.start_listen();
                    self.prefetch_next();
                    self.stream_info = Some(info);
                    self.fix_duration(duration);
                }
                PlayerEvent::AboutToFinish => {
                    // A radio stream that ended does not roll over to the next entry.
                    if self.current_is_stream() {
                        continue;
                    }
                    if let Some(n) = self.next_of(self.current, false) {
                        if let Some(t) = self.track(n) {
                            let req = t.media_request();
                            self.pending_next = Some(n);
                            self.player.enqueue(req);
                        }
                    }
                }
                PlayerEvent::TrackChanged { info, duration } => {
                    self.finish_listen(false);
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
                    self.start_listen();
                }
                PlayerEvent::Finished => {
                    self.finish_listen(false);
                    self.pending_next = None;
                    self.recording = false;
                }
                PlayerEvent::StreamError(msg) => {
                    let t = self.tr().playback_error;
                    self.show_error(format!("{t}: {msg}"), now);
                }
                PlayerEvent::StreamTitle(title) => {
                    self.stream_title = Some(title);
                }
                PlayerEvent::Recording { active, files, error } => {
                    self.recording = active;
                    let t = self.tr();
                    if let Some(e) = error {
                        let text = if e.contains(crate::audio::decoder::FFMPEG_MISSING) {
                            t.ffmpeg_missing.to_string()
                        } else {
                            format!("{}: {e}", t.recording_error)
                        };
                        self.show_error(text, now);
                    } else if !active && !files.is_empty() {
                        let dir = files[0].parent().map(|d| d.display().to_string()).unwrap_or_default();
                        self.show_info(format!("{} ({}): {dir}", t.recording_saved, files.len()), now);
                    }
                }
                PlayerEvent::Error(msg) => {
                    self.connecting = false;
                    let failed = self.pending_next.take().or(self.current);
                    let t = self.tr();
                    let text = if msg.contains(crate::audio::decoder::FFMPEG_MISSING) {
                        t.ffmpeg_missing.to_string()
                    } else if msg.contains(crate::audio::ytdl::YTDLP_MISSING) {
                        t.ytdlp_missing.to_string()
                    } else if msg.contains(crate::audio::midi::SOUNDFONT_MISSING) {
                        t.soundfont_missing.to_string()
                    } else {
                        format!("{}: {msg}", t.playback_error)
                    };
                    self.show_error(text, now);
                    self.error_streak += 1;
                    let failed_stream = failed.and_then(|r| self.track(r)).is_some_and(|t| t.is_stream());
                    if failed.is_some() && !failed_stream && self.error_streak < 30 {
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

        self.sample_listen(now);

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

        if (self.dirty || self.library_dirty) && now - self.last_save > 5.0 {
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
        if self.library_dirty {
            self.library.save();
            self.library_dirty = false;
        }
    }

    fn handle_hotkeys(&mut self, ctx: &egui::Context) {
        if self.hotkey_capture.is_some() || ctx.egui_wants_keyboard_input() {
            return;
        }
        let keys = &self.settings.hotkeys;
        let mut actions = Vec::new();
        ctx.input_mut(|i| {
            i.events.retain(|e| {
                let egui::Event::Key { key, physical_key, pressed: true, repeat, modifiers } = e else {
                    return true;
                };
                // Physical key first, so letter shortcuts work with any keyboard layout.
                let found = physical_key
                    .and_then(|k| keys.local_action(Shortcut::from_event(k, *modifiers)))
                    .or_else(|| keys.local_action(Shortcut::from_event(*key, *modifiers)));
                match found {
                    Some(a) => {
                        if !*repeat || a.repeats() {
                            actions.push(a);
                        }
                        false
                    }
                    None => true,
                }
            });
        });
        for a in actions {
            self.run_action(a);
        }
    }

    pub fn run_action(&mut self, action: Action) {
        log::debug!("hotkey action {action:?}");
        match action {
            Action::PlayPause => self.play_pause(),
            Action::Pause => self.player.pause(),
            Action::Stop => self.stop(),
            Action::Prev => self.prev(),
            Action::Next => self.next(),
            Action::SeekBack => self.seek_relative(-5.0),
            Action::SeekForward => self.seek_relative(5.0),
            Action::VolumeUp => self.set_volume(self.settings.volume + 0.05),
            Action::VolumeDown => self.set_volume(self.settings.volume - 0.05),
            Action::Mute => self.toggle_mute(),
            Action::Shuffle => self.settings.shuffle = !self.settings.shuffle,
            Action::Repeat => self.settings.repeat = self.settings.repeat.cycle(),
            Action::Bookmark => self.add_bookmark(),
            Action::Record => {
                if self.current_is_stream() {
                    self.toggle_recording();
                }
            }
            Action::Search => self.focus_search = true,
            Action::SelectAll => {
                let idx = self.active();
                let pl = &mut self.playlists[idx];
                pl.selected = pl.tracks.iter().map(|t| t.uid).collect();
            }
            Action::Remove => {
                let idx = self.active();
                self.playlists[idx].remove_selected();
                self.mark_dirty();
            }
            Action::Queue => {
                let pl = &self.playlists[self.active()];
                let refs: Vec<TrackRef> = pl
                    .tracks
                    .iter()
                    .filter(|t| pl.selected.contains(&t.uid))
                    .map(|t| TrackRef { playlist: pl.id, uid: t.uid })
                    .collect();
                for r in refs {
                    self.toggle_queue(r);
                }
            }
            Action::PlaySelected => self.play_current_or_selected(),
            Action::MoveUp | Action::MoveDown => {
                let idx = self.active();
                self.playlists[idx].move_selected(if action == Action::MoveUp { -1 } else { 1 });
                self.mark_dirty();
            }
        }
    }

    /// Requests from the OS media overlay / MPRIS, then publish what is playing.
    fn handle_system_controls(&mut self, ctx: &egui::Context) {
        use souvlaki::{MediaControlEvent as E, SeekDirection};
        for ev in self.system_controls.poll() {
            log::debug!("system media control: {ev:?}");
            let state = self.player.state();
            match ev {
                E::Play if state != PlaybackState::Playing => self.play_pause(),
                E::Pause if state == PlaybackState::Playing => self.play_pause(),
                E::Toggle => self.play_pause(),
                E::Next => self.next(),
                E::Previous => self.prev(),
                E::Stop => self.stop(),
                E::Seek(d) => self.seek_relative(if d == SeekDirection::Forward { 5.0 } else { -5.0 }),
                E::SeekBy(d, by) => {
                    let s = by.as_secs_f64();
                    self.seek_relative(if d == SeekDirection::Forward { s } else { -s });
                }
                E::SetPosition(p) => self.seek_to(p.0.as_secs_f64()),
                E::SetVolume(v) => self.set_volume(v as f32),
                E::OpenUri(u) => self.add_url(&u),
                E::Raise => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
                E::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                _ => {}
            }
        }
        let now = ctx.input(|i| i.time);
        let state = self.player.state();
        let title = self.stream_title.clone();
        let track = self.current_track().map(|t| {
            let (title, artist) = match &title {
                // Radio: the song the station announces, with the station as the album.
                Some(s) if t.is_stream() => match s.split_once(" - ") {
                    Some((a, b)) => (b.to_string(), a.to_string()),
                    None => (s.clone(), String::new()),
                },
                _ => (t.title.clone(), t.artist.clone()),
            };
            let album = if t.is_stream() { t.title.clone() } else { t.album.clone() };
            (title, artist, album, t.path.clone(), t.is_remote())
        });
        let duration = self.player.duration();
        let volume = if self.settings.muted { 0.0 } else { self.settings.volume as f64 };
        self.system_controls.update(
            track.as_ref().map(|(a, b, c, p, r)| (a.as_str(), b.as_str(), c.as_str(), p.as_path(), *r)),
            duration,
            state,
            self.player.position(),
            volume,
            now,
        );
    }

    /// Send the equalizer and ReplayGain settings to the engine.
    pub fn apply_dsp(&mut self) {
        let eq = &self.settings.eq;
        self.player.set_eq(eq.enabled, eq.preamp_db, &eq.gains);
        self.player.set_replaygain(&self.settings.replaygain);
    }

    /// Re-register system-wide hotkeys after the settings changed.
    pub fn apply_global_hotkeys(&mut self) {
        self.global_keys.apply(&self.settings.hotkeys);
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
                self.toast = Some((e.to_string(), f64::MAX, true));
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
        for a in self.global_keys.poll() {
            self.run_action(a);
        }
        self.handle_system_controls(ctx);
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

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.finish_listen(false);
        self.save();
    }
}

/// Download a small text resource (a station playlist).
fn fetch_text(url: &str) -> Result<String, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(15)))
        .build()
        .into();
    let mut resp = agent.get(url).call().map_err(|e| e.to_string())?;
    let bytes = resp.body_mut().with_config().limit(1 << 20).read_to_vec().map_err(|e| e.to_string())?;
    Ok(crate::playlist::cue::decode_text(&bytes))
}
