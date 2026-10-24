//! Known file extensions and which decoding backend handles them.

use std::path::Path;

/// Decoding backend responsible for a format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// Pure-Rust decoding via Symphonia (always available).
    Symphonia,
    /// External ffmpeg process (APE, WMA, AC3, DTS, MPC, TAK, TTA, WV, OFR, SPX, DSD, ATRAC, Opus...).
    Ffmpeg,
    /// Tracker modules, decoded by ffmpeg's libopenmpt demuxer.
    Tracker,
    /// MIDI synthesis via SoundFont.
    Midi,
    /// Audio CD track (`.cda` on Windows).
    Cd,
}

/// Extensions decoded by Symphonia right now.
pub const SYMPHONIA_EXTS: &[&str] = &[
    "mp1", "mp2", "mp3", "mpga", "flac", "ogg", "oga", "wav", "wave", "aif", "aiff", "aifc",
    "m4a", "m4b", "mp4", "aac", "adts", "alac", "caf", "mka", "mkv", "webm",
];

/// Extensions that will be served by the FFmpeg backend.
pub const FFMPEG_EXTS: &[&str] = &[
    "ape", "mac", "wma", "asf", "ac3", "eac3", "dts", "dtshd", "mpc", "mp+", "mpp", "tak", "tta",
    "wv", "ofr", "ofs", "spx", "opus", "dsf", "dff", "aa3", "at3", "oma", "omg", "amr", "au",
    "snd", "voc", "w64", "rf64", "mlp", "thd", "truehd", "shn", "ra", "rm",
];

/// Tracker module extensions (libopenmpt).
pub const TRACKER_EXTS: &[&str] = &[
    "it", "mod", "mo3", "mtm", "s3m", "umx", "xm", "mdz", "itz", "s3z", "xmz", "mptm", "669",
    "amf", "ams", "dbm", "dmf", "dsm", "far", "mdl", "med", "okt", "psm", "ptm", "stm", "ult",
];

/// MIDI extensions.
pub const MIDI_EXTS: &[&str] = &["mid", "midi", "rmi"];

/// Audio CD track files.
pub const CD_EXTS: &[&str] = &["cda"];

/// Playlist / cue sheet extensions understood when adding files.
pub const PLAYLIST_EXTS: &[&str] = &["cue", "m3u", "m3u8", "pls"];

fn ext_of(path: &Path) -> Option<String> {
    path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase())
}

pub fn backend_for(path: &Path) -> Option<Backend> {
    let ext = ext_of(path)?;
    let ext = ext.as_str();
    if SYMPHONIA_EXTS.contains(&ext) {
        Some(Backend::Symphonia)
    } else if FFMPEG_EXTS.contains(&ext) {
        Some(Backend::Ffmpeg)
    } else if TRACKER_EXTS.contains(&ext) {
        Some(Backend::Tracker)
    } else if MIDI_EXTS.contains(&ext) {
        Some(Backend::Midi)
    } else if CD_EXTS.contains(&ext) {
        Some(Backend::Cd)
    } else {
        None
    }
}

/// Whether a file looks like audio we can put into a playlist.
pub fn is_audio_file(path: &Path) -> bool {
    backend_for(path).is_some()
}

pub fn is_playlist_file(path: &Path) -> bool {
    ext_of(path).is_some_and(|e| PLAYLIST_EXTS.contains(&e.as_str()))
}

/// All extensions for file dialogs.
pub fn all_audio_exts() -> Vec<&'static str> {
    SYMPHONIA_EXTS
        .iter()
        .chain(FFMPEG_EXTS)
        .chain(TRACKER_EXTS)
        .chain(MIDI_EXTS)
        .chain(CD_EXTS)
        .copied()
        .collect()
}
