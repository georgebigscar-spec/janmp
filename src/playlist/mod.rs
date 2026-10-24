//! Playlists and their tracks.

pub mod cue;
pub mod m3u;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::audio::engine::MediaRequest;
use crate::audio::formats;
use crate::meta::TrackMeta;

static NEXT_UID: AtomicU64 = AtomicU64::new(1);

/// Process-unique id of a playlist entry (not persisted).
pub fn new_uid() -> u64 {
    NEXT_UID.fetch_add(1, Ordering::Relaxed)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CueRange {
    pub number: u32,
    pub start: f64,
    pub end: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Track {
    #[serde(skip, default = "new_uid")]
    pub uid: u64,
    pub path: PathBuf,
    pub cue: Option<CueRange>,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub genre: String,
    pub year: Option<u32>,
    pub track_no: Option<u32>,
    pub disc_no: Option<u32>,
    pub duration: Option<f64>,
    pub bitrate: Option<u32>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u8>,
    pub file_size: u64,
    pub scanned: bool,
}

impl Default for Track {
    fn default() -> Self {
        Self {
            uid: new_uid(),
            path: PathBuf::new(),
            cue: None,
            title: String::new(),
            artist: String::new(),
            album: String::new(),
            album_artist: String::new(),
            genre: String::new(),
            year: None,
            track_no: None,
            disc_no: None,
            duration: None,
            bitrate: None,
            sample_rate: None,
            channels: None,
            file_size: 0,
            scanned: false,
        }
    }
}

impl Track {
    pub fn from_path(path: PathBuf) -> Self {
        let title = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        Self { path, title, ..Default::default() }
    }

    pub fn from_cue(ct: &cue::CueTrack) -> Self {
        Self {
            path: ct.file.clone(),
            cue: Some(CueRange { number: ct.number, start: ct.start, end: ct.end }),
            title: ct.title.clone().unwrap_or_else(|| format!("Track {:02}", ct.number)),
            artist: ct.performer.clone().unwrap_or_default(),
            album: ct.album.clone().unwrap_or_default(),
            album_artist: ct.album_performer.clone().unwrap_or_default(),
            genre: ct.genre.clone().unwrap_or_default(),
            year: ct.year,
            track_no: Some(ct.number),
            duration: ct.end.map(|e| e - ct.start),
            ..Default::default()
        }
    }

    /// Merge tags read from the file. Cue tracks keep their own titles and offsets.
    pub fn apply_meta(&mut self, m: TrackMeta) {
        self.scanned = true;
        self.bitrate = m.bitrate;
        self.sample_rate = m.sample_rate;
        self.channels = m.channels;
        self.file_size = m.file_size;
        let fill = |dst: &mut String, src: Option<String>| {
            if dst.is_empty() {
                if let Some(s) = src {
                    *dst = s;
                }
            }
        };
        if let Some(cue) = &self.cue {
            fill(&mut self.album, m.album);
            fill(&mut self.genre, m.genre);
            self.year = self.year.or(m.year);
            if self.duration.is_none() {
                self.duration = m.duration.map(|d| (d - cue.start).max(0.0));
            }
            return;
        }
        if let Some(t) = m.title {
            self.title = t;
        }
        self.artist = m.artist.unwrap_or_default();
        self.album = m.album.unwrap_or_default();
        self.album_artist = m.album_artist.unwrap_or_default();
        self.genre = m.genre.unwrap_or_default();
        self.year = m.year;
        self.track_no = m.track_no;
        self.disc_no = m.disc_no;
        self.duration = m.duration;
    }

    pub fn display_title(&self) -> String {
        if self.artist.is_empty() {
            self.title.clone()
        } else {
            format!("{} - {}", self.artist, self.title)
        }
    }

    pub fn media_request(&self) -> MediaRequest {
        MediaRequest {
            path: self.path.clone(),
            start: self.cue.as_ref().map(|c| c.start).unwrap_or(0.0),
            end: self.cue.as_ref().and_then(|c| c.end),
        }
    }

    pub fn album_artist_or_artist(&self) -> &str {
        if self.album_artist.is_empty() { &self.artist } else { &self.album_artist }
    }

    pub fn group_key(&self, mode: GroupMode) -> String {
        match mode {
            GroupMode::None => String::new(),
            GroupMode::Album => {
                let artist = self.album_artist_or_artist();
                match (artist.is_empty(), self.album.is_empty()) {
                    (_, true) => self.folder_name(),
                    (true, false) => self.album.clone(),
                    (false, false) => format!("{artist} - {}", self.album),
                }
            }
            GroupMode::Artist => self.album_artist_or_artist().to_string(),
            GroupMode::Folder => self.path.parent().map(|p| p.display().to_string()).unwrap_or_default(),
            GroupMode::Genre => self.genre.clone(),
            GroupMode::Year => self.year.map(|y| y.to_string()).unwrap_or_default(),
        }
    }

    fn folder_name(&self) -> String {
        self.path
            .parent()
            .and_then(|p| p.file_name())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    /// Short technical summary: "FLAC · 44 kHz · 1024 kbps · Stereo".
    pub fn tech_summary(&self) -> String {
        let mut parts = Vec::new();
        if let Some(ext) = self.path.extension() {
            parts.push(ext.to_string_lossy().to_uppercase());
        }
        if let Some(sr) = self.sample_rate {
            parts.push(format!("{:.1} kHz", sr as f64 / 1000.0).replace(".0 ", " "));
        }
        if let Some(br) = self.bitrate {
            parts.push(format!("{br} kbps"));
        }
        match self.channels {
            Some(1) => parts.push("Mono".into()),
            Some(2) => parts.push("Stereo".into()),
            Some(n) => parts.push(format!("{n} ch")),
            None => {}
        }
        parts.join(" · ")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum GroupMode {
    None,
    #[default]
    Album,
    Artist,
    Folder,
    Genre,
    Year,
}

impl GroupMode {
    pub const ALL: [GroupMode; 6] = [
        GroupMode::None,
        GroupMode::Album,
        GroupMode::Artist,
        GroupMode::Folder,
        GroupMode::Genre,
        GroupMode::Year,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    Title,
    Artist,
    Album,
    Duration,
    Path,
    Random,
    Reverse,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Playlist {
    #[serde(skip, default = "new_uid")]
    pub id: u64,
    pub name: String,
    pub tracks: Vec<Track>,
    #[serde(skip)]
    pub selected: HashSet<u64>,
    /// Last clicked entry, anchor for shift-selection.
    #[serde(skip)]
    pub anchor: Option<u64>,
}

impl Default for Playlist {
    fn default() -> Self {
        Self { id: new_uid(), name: String::new(), tracks: Vec::new(), selected: HashSet::new(), anchor: None }
    }
}

impl Playlist {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), ..Default::default() }
    }

    pub fn index_of(&self, uid: u64) -> Option<usize> {
        self.tracks.iter().position(|t| t.uid == uid)
    }

    pub fn get(&self, uid: u64) -> Option<&Track> {
        self.tracks.iter().find(|t| t.uid == uid)
    }

    pub fn get_mut(&mut self, uid: u64) -> Option<&mut Track> {
        self.tracks.iter_mut().find(|t| t.uid == uid)
    }

    /// Add files, folders (recursively), cue sheets and playlists.
    /// Returns the newly added tracks that still need tag scanning.
    pub fn add_paths(&mut self, paths: &[PathBuf]) -> Vec<(u64, PathBuf)> {
        let mut new_tracks = Vec::new();
        for p in paths {
            collect(p, &mut new_tracks, 0);
        }
        let scan = new_tracks.iter().map(|t: &Track| (t.uid, t.path.clone())).collect();
        self.tracks.extend(new_tracks);
        scan
    }

    pub fn remove_selected(&mut self) {
        let sel = std::mem::take(&mut self.selected);
        self.tracks.retain(|t| !sel.contains(&t.uid));
    }

    pub fn remove_missing(&mut self) {
        self.tracks.retain(|t| t.path.exists());
    }

    pub fn remove_duplicates(&mut self) {
        let mut seen = HashSet::new();
        self.tracks.retain(|t| {
            seen.insert((t.path.clone(), t.cue.as_ref().map(|c| c.number)))
        });
    }

    pub fn sort(&mut self, key: SortKey) {
        let lc = |s: &str| s.to_lowercase();
        match key {
            SortKey::Title => self.tracks.sort_by_key(|t| lc(&t.title)),
            SortKey::Artist => self.tracks.sort_by(|a, b| {
                (lc(a.album_artist_or_artist()), lc(&a.album), a.disc_no, a.track_no, lc(&a.title)).cmp(&(
                    lc(b.album_artist_or_artist()),
                    lc(&b.album),
                    b.disc_no,
                    b.track_no,
                    lc(&b.title),
                ))
            }),
            SortKey::Album => self.tracks.sort_by(|a, b| {
                (lc(&a.album), a.disc_no, a.track_no).cmp(&(lc(&b.album), b.disc_no, b.track_no))
            }),
            SortKey::Duration => self.tracks.sort_by(|a, b| {
                a.duration.unwrap_or(0.0).total_cmp(&b.duration.unwrap_or(0.0))
            }),
            SortKey::Path => self.tracks.sort_by(|a, b| {
                (&a.path, a.cue.as_ref().map(|c| c.number)).cmp(&(&b.path, b.cue.as_ref().map(|c| c.number)))
            }),
            SortKey::Random => {
                use rand::seq::SliceRandom;
                self.tracks.shuffle(&mut rand::rng());
            }
            SortKey::Reverse => self.tracks.reverse(),
        }
    }

    pub fn total_duration(&self) -> f64 {
        self.tracks.iter().filter_map(|t| t.duration).sum()
    }

    pub fn total_size(&self) -> u64 {
        // A cue-split file is counted once.
        let mut seen = HashSet::new();
        self.tracks.iter().filter(|t| seen.insert(&t.path)).map(|t| t.file_size).sum()
    }

    pub fn select_only(&mut self, uid: u64) {
        self.selected.clear();
        self.selected.insert(uid);
        self.anchor = Some(uid);
    }

    pub fn select_range_to(&mut self, uid: u64) {
        let Some(a) = self.anchor.and_then(|a| self.index_of(a)) else {
            return self.select_only(uid);
        };
        let Some(b) = self.index_of(uid) else { return };
        let (lo, hi) = (a.min(b), a.max(b));
        self.selected = self.tracks[lo..=hi].iter().map(|t| t.uid).collect();
    }

    pub fn toggle_selected(&mut self, uid: u64) {
        if !self.selected.remove(&uid) {
            self.selected.insert(uid);
        }
        self.anchor = Some(uid);
    }

    /// Move the selected tracks up (`-1`) or down (`+1`) by one row.
    pub fn move_selected(&mut self, dir: i32) {
        let n = self.tracks.len();
        if dir < 0 {
            for i in 1..n {
                if self.selected.contains(&self.tracks[i].uid) && !self.selected.contains(&self.tracks[i - 1].uid) {
                    self.tracks.swap(i, i - 1);
                }
            }
        } else {
            for i in (0..n.saturating_sub(1)).rev() {
                if self.selected.contains(&self.tracks[i].uid) && !self.selected.contains(&self.tracks[i + 1].uid) {
                    self.tracks.swap(i, i + 1);
                }
            }
        }
    }
}

const MAX_DEPTH: usize = 32;

fn collect(path: &Path, out: &mut Vec<Track>, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    if path.is_dir() {
        let Ok(rd) = std::fs::read_dir(path) else { return };
        let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
        entries.sort_by_key(|p| p.to_string_lossy().to_lowercase());

        // Files described by a cue sheet in this folder are added through the sheet.
        let mut covered: HashSet<PathBuf> = HashSet::new();
        for cue_path in entries.iter().filter(|p| is_ext(p, "cue")) {
            if let Ok(tracks) = cue::load(cue_path) {
                let usable: Vec<_> = tracks.into_iter().filter(|t| t.file.exists()).collect();
                for t in &usable {
                    covered.insert(t.file.clone());
                }
                out.extend(usable.iter().map(Track::from_cue));
            }
        }
        for e in &entries {
            if e.is_dir() {
                collect(e, out, depth + 1);
            } else if formats::is_audio_file(e) && !covered.contains(e) {
                out.push(Track::from_path(e.clone()));
            }
        }
        return;
    }
    if is_ext(path, "cue") {
        if let Ok(tracks) = cue::load(path) {
            out.extend(tracks.iter().filter(|t| t.file.exists()).map(Track::from_cue));
        }
    } else if formats::is_playlist_file(path) {
        if let Ok(entries) = m3u::load(path) {
            for e in entries {
                if let m3u::Entry::File { path, .. } = e {
                    if path.exists() && depth < 4 {
                        collect(&path, out, depth + 1);
                    }
                }
            }
        }
    } else if formats::is_audio_file(path) {
        out.push(Track::from_path(path.to_path_buf()));
    }
}

fn is_ext(p: &Path, ext: &str) -> bool {
    p.extension().is_some_and(|e| e.eq_ignore_ascii_case(ext))
}
