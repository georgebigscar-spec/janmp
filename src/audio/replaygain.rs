//! ReplayGain: per-track or per-album loudness normalization from tags.

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};

use lofty::file::TaggedFileExt;
use lofty::tag::ItemKey;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum RgMode {
    #[default]
    Off,
    Track,
    Album,
}

impl RgMode {
    pub const ALL: [RgMode; 3] = [RgMode::Off, RgMode::Track, RgMode::Album];
}

/// Gains (dB) and peaks (linear) found in a file's tags.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct RgTags {
    pub track_gain: Option<f32>,
    pub track_peak: Option<f32>,
    pub album_gain: Option<f32>,
    pub album_peak: Option<f32>,
}

/// Parse "-6.54 dB" / "+1.2" / "0.98765".
fn number(s: &str) -> Option<f32> {
    let s = s.trim();
    let s = s.strip_suffix("dB").or_else(|| s.strip_suffix("db")).or_else(|| s.strip_suffix("DB")).unwrap_or(s);
    s.trim().trim_start_matches('+').parse().ok().filter(|v: &f32| v.is_finite())
}

pub fn read_tags(path: &Path) -> RgTags {
    let Ok(file) = lofty::read_from_path(path) else { return RgTags::default() };
    let mut rg = RgTags::default();
    for tag in file.tags() {
        let get = |k: ItemKey| tag.get_string(k).and_then(number);
        rg.track_gain = rg.track_gain.or_else(|| get(ItemKey::ReplayGainTrackGain));
        rg.track_peak = rg.track_peak.or_else(|| get(ItemKey::ReplayGainTrackPeak));
        rg.album_gain = rg.album_gain.or_else(|| get(ItemKey::ReplayGainAlbumGain));
        rg.album_peak = rg.album_peak.or_else(|| get(ItemKey::ReplayGainAlbumPeak));
        // Opus R128 gains: Q7.8 fixed point relative to -23 LUFS; ReplayGain targets -18.
        let r128 = |k: ItemKey| {
            tag.get_string(k).and_then(|v| v.trim().parse::<i32>().ok()).map(|q| q as f32 / 256.0 + 5.0)
        };
        rg.track_gain = rg.track_gain.or_else(|| r128(ItemKey::R128TrackGain));
        rg.album_gain = rg.album_gain.or_else(|| r128(ItemKey::R128AlbumGain));
    }
    rg
}

/// ReplayGain settings shared with the engine.
pub struct RgShared {
    mode: AtomicU8,
    preamp_db: AtomicU32,
    /// Gain for files without tags, dB.
    fallback_db: AtomicU32,
    prevent_clipping: AtomicBool,
}

impl Default for RgShared {
    fn default() -> Self {
        Self {
            mode: AtomicU8::new(0),
            preamp_db: AtomicU32::new(0f32.to_bits()),
            fallback_db: AtomicU32::new(0f32.to_bits()),
            prevent_clipping: AtomicBool::new(true),
        }
    }
}

impl RgShared {
    pub fn set(&self, s: &RgSettings) {
        self.mode.store(s.mode as u8, Ordering::Relaxed);
        self.preamp_db.store(s.preamp_db.to_bits(), Ordering::Relaxed);
        self.fallback_db.store(s.fallback_db.to_bits(), Ordering::Relaxed);
        self.prevent_clipping.store(s.prevent_clipping, Ordering::Relaxed);
    }

    /// Linear gain to apply to a file with these tags under the current settings.
    pub fn gain(&self, tags: &RgTags) -> f32 {
        let mode = match self.mode.load(Ordering::Relaxed) {
            1 => RgMode::Track,
            2 => RgMode::Album,
            _ => return 1.0,
        };
        let preamp = f32::from_bits(self.preamp_db.load(Ordering::Relaxed));
        let fallback = f32::from_bits(self.fallback_db.load(Ordering::Relaxed));
        let (gain, peak) = match mode {
            RgMode::Album => (tags.album_gain.or(tags.track_gain), tags.album_peak.or(tags.track_peak)),
            _ => (tags.track_gain.or(tags.album_gain), tags.track_peak.or(tags.album_peak)),
        };
        let linear = match gain {
            Some(g) => 10f32.powf((g + preamp) / 20.0),
            None => 10f32.powf(fallback / 20.0),
        };
        match peak.filter(|p| *p > 0.0) {
            Some(p) if self.prevent_clipping.load(Ordering::Relaxed) => linear.min(1.0 / p),
            _ => linear,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RgSettings {
    pub mode: RgMode,
    pub preamp_db: f32,
    pub fallback_db: f32,
    pub prevent_clipping: bool,
}

impl Default for RgSettings {
    fn default() -> Self {
        Self { mode: RgMode::Off, preamp_db: 0.0, fallback_db: 0.0, prevent_clipping: true }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_values() {
        assert_eq!(number("-6.54 dB"), Some(-6.54));
        assert_eq!(number("+1.20 dB"), Some(1.2));
        assert_eq!(number("0.988"), Some(0.988));
        assert_eq!(number("n/a"), None);
    }

    #[test]
    fn computes_gain() {
        let s = RgShared::default();
        let tags = RgTags { track_gain: Some(-6.0), track_peak: Some(0.9), album_gain: Some(-8.0), album_peak: Some(1.0) };
        assert_eq!(s.gain(&tags), 1.0, "off by default");
        s.set(&RgSettings { mode: RgMode::Track, ..Default::default() });
        assert!((s.gain(&tags) - 0.501).abs() < 1e-3);
        s.set(&RgSettings { mode: RgMode::Album, ..Default::default() });
        assert!((s.gain(&tags) - 0.398).abs() < 1e-3);
        // A boost is limited by the peak so the track does not clip.
        let quiet = RgTags { track_gain: Some(6.0), track_peak: Some(0.8), ..Default::default() };
        s.set(&RgSettings { mode: RgMode::Track, ..Default::default() });
        assert!((s.gain(&quiet) - 1.25).abs() < 1e-3);
        s.set(&RgSettings { mode: RgMode::Track, fallback_db: -3.0, ..Default::default() });
        assert!((s.gain(&RgTags::default()) - 0.708).abs() < 1e-3);
    }

    /// `JANMP_TEST_RG` names a file tagged with ReplayGain.
    #[test]
    #[ignore]
    fn reads_file_tags() {
        let path = std::env::var("JANMP_TEST_RG").expect("set JANMP_TEST_RG");
        let t = read_tags(Path::new(&path));
        eprintln!("{t:?}");
        assert!(t.track_gain.is_some());
    }
}
