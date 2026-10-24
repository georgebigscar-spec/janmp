//! Listening statistics, ratings (manual and automatic) and bookmarks.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::playlist::Track;

/// Stable identity of a track across sessions: file path plus cue track number.
pub fn track_key(t: &Track) -> String {
    match &t.cue {
        Some(c) => format!("{}#{}", t.path.display(), c.number),
        None => t.path.display().to_string(),
    }
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TrackStats {
    pub title: String,
    pub artist: String,
    pub plays: u32,
    pub skips: u32,
    pub listened_secs: f64,
    pub first_played: u64,
    pub last_played: u64,
    /// Rating set by the user, 1..=5.
    pub rating: Option<u8>,
}

impl TrackStats {
    /// Rating derived from how often the track is played to the end versus skipped,
    /// weighted by how often it is played at all. `None` until there is some history.
    pub fn auto_rating(&self) -> Option<u8> {
        let total = self.plays + self.skips;
        if total == 0 {
            return None;
        }
        let completion = self.plays as f64 / total as f64;
        let popularity = ((1.0 + self.plays as f64).ln() / 21f64.ln()).min(1.0);
        let score = 1.0 + 4.0 * (0.6 * completion + 0.4 * popularity);
        Some(score.round().clamp(1.0, 5.0) as u8)
    }

    /// User rating if set, otherwise the automatic one. The flag says which.
    pub fn effective_rating(&self) -> Option<(u8, bool)> {
        self.rating.map(|r| (r, true)).or_else(|| self.auto_rating().map(|r| (r, false)))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bookmark {
    pub path: PathBuf,
    pub cue_number: Option<u32>,
    pub title: String,
    /// Position inside the track, seconds.
    pub position: f64,
    pub created: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Library {
    pub tracks: HashMap<String, TrackStats>,
    pub bookmarks: Vec<Bookmark>,
}

impl Library {
    fn file() -> Option<PathBuf> {
        crate::config::config_dir().map(|d| d.join("library.json"))
    }

    pub fn load() -> Self {
        Self::file()
            .and_then(|f| std::fs::read(f).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let Some(f) = Self::file() else { return };
        match serde_json::to_vec(self) {
            Ok(b) => {
                if let Err(e) = crate::config::write_atomic(&f, &b) {
                    log::warn!("saving library: {e}");
                }
            }
            Err(e) => log::warn!("serializing library: {e}"),
        }
    }

    pub fn get(&self, t: &Track) -> Option<&TrackStats> {
        self.tracks.get(&track_key(t))
    }

    fn entry(&mut self, t: &Track) -> &mut TrackStats {
        let e = self.tracks.entry(track_key(t)).or_default();
        e.title = t.title.clone();
        e.artist = t.artist.clone();
        e
    }

    pub fn set_rating(&mut self, t: &Track, rating: Option<u8>) {
        self.entry(t).rating = rating.map(|r| r.clamp(1, 5));
    }

    /// Account one listening session of `t`. Counts as a play when at least half the track
    /// (or four minutes) was heard; leaving it earlier by user action counts as a skip.
    pub fn record_listen(&mut self, t: &Track, listened: f64, duration: Option<f64>, by_user: bool) {
        if listened < 1.0 {
            return;
        }
        let threshold = duration.map(|d| (d * 0.5).min(240.0)).unwrap_or(240.0);
        let now = now_secs();
        let e = self.entry(t);
        e.listened_secs += listened;
        if listened >= threshold {
            e.plays += 1;
            if e.first_played == 0 {
                e.first_played = now;
            }
            e.last_played = now;
        } else if by_user {
            e.skips += 1;
        }
    }

    pub fn total_listened(&self) -> f64 {
        self.tracks.values().map(|s| s.listened_secs).sum()
    }

    pub fn total_plays(&self) -> u64 {
        self.tracks.values().map(|s| s.plays as u64).sum()
    }

    pub fn top_tracks(&self, n: usize) -> Vec<(&String, &TrackStats)> {
        let mut v: Vec<_> = self.tracks.iter().filter(|(_, s)| s.plays > 0).collect();
        v.sort_by(|a, b| b.1.plays.cmp(&a.1.plays).then(b.1.last_played.cmp(&a.1.last_played)));
        v.truncate(n);
        v
    }

    pub fn top_artists(&self, n: usize) -> Vec<(String, u32, f64)> {
        let mut m: HashMap<&str, (u32, f64)> = HashMap::new();
        for s in self.tracks.values().filter(|s| !s.artist.is_empty()) {
            let e = m.entry(&s.artist).or_default();
            e.0 += s.plays;
            e.1 += s.listened_secs;
        }
        let mut v: Vec<_> = m.into_iter().filter(|(_, (p, _))| *p > 0).map(|(a, (p, l))| (a.to_string(), p, l)).collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then(b.2.total_cmp(&a.2)));
        v.truncate(n);
        v
    }

    pub fn recent(&self, n: usize) -> Vec<(&String, &TrackStats)> {
        let mut v: Vec<_> = self.tracks.iter().filter(|(_, s)| s.last_played > 0).collect();
        v.sort_by_key(|(_, s)| std::cmp::Reverse(s.last_played));
        v.truncate(n);
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track() -> Track {
        let mut t = Track::from_path("/m/a.mp3".into());
        t.duration = Some(200.0);
        t
    }

    #[test]
    fn plays_and_skips() {
        let mut lib = Library::default();
        let t = track();
        lib.record_listen(&t, 150.0, t.duration, false);
        lib.record_listen(&t, 20.0, t.duration, true);
        lib.record_listen(&t, 20.0, t.duration, false); // stopped by end of list, not a skip
        let s = lib.get(&t).unwrap();
        assert_eq!((s.plays, s.skips), (1, 1));
        assert!((s.listened_secs - 190.0).abs() < 1e-9);
    }

    #[test]
    fn auto_rating_rewards_completed_plays() {
        let loved = TrackStats { plays: 20, skips: 0, ..Default::default() };
        let skipped = TrackStats { plays: 1, skips: 9, ..Default::default() };
        assert_eq!(loved.auto_rating(), Some(5));
        assert!(skipped.auto_rating().unwrap() <= 2);
        assert_eq!(TrackStats::default().auto_rating(), None);
        let manual = TrackStats { rating: Some(3), ..loved };
        assert_eq!(manual.effective_rating(), Some((3, true)));
    }

    #[test]
    fn cue_tracks_have_distinct_keys() {
        let mut a = track();
        let mut b = track();
        a.cue = Some(crate::playlist::CueRange { number: 1, start: 0.0, end: Some(10.0) });
        b.cue = Some(crate::playlist::CueRange { number: 2, start: 10.0, end: None });
        assert_ne!(track_key(&a), track_key(&b));
    }
}
