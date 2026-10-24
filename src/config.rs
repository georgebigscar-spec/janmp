//! Settings and playlist persistence (JSON files in the platform config directory).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::hotkeys::HotkeySettings;
use crate::i18n::Lang;
use crate::playlist::{GroupMode, Playlist};
use crate::radio::record::RecordSettings;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum RepeatMode {
    Off,
    #[default]
    All,
    One,
}

impl RepeatMode {
    pub fn cycle(self) -> Self {
        match self {
            RepeatMode::Off => RepeatMode::All,
            RepeatMode::All => RepeatMode::One,
            RepeatMode::One => RepeatMode::Off,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub lang: Lang,
    pub volume: f32,
    pub muted: bool,
    pub balance: f32,
    pub repeat: RepeatMode,
    pub shuffle: bool,
    pub group: GroupMode,
    pub active_playlist: usize,
    pub system_frame: bool,
    pub show_spectrum: bool,
    pub show_remaining: bool,
    pub accent: [u8; 3],
    pub last_dir: Option<PathBuf>,
    pub record: RecordSettings,
    pub hotkeys: HotkeySettings,
    /// SoundFont for MIDI; `None` searches the usual places.
    pub soundfont: Option<PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            lang: Lang::detect(),
            volume: 0.8,
            muted: false,
            balance: 0.0,
            repeat: RepeatMode::All,
            shuffle: false,
            group: GroupMode::Album,
            active_playlist: 0,
            system_frame: false,
            show_spectrum: true,
            show_remaining: false,
            accent: [0xFF, 0x8C, 0x1A],
            last_dir: None,
            record: RecordSettings::default(),
            hotkeys: HotkeySettings::default(),
            soundfont: None,
        }
    }
}

pub fn config_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "janmp").map(|d| d.config_dir().to_path_buf())
}

pub fn write_atomic(path: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, data)?;
    std::fs::rename(tmp, path)
}

impl Settings {
    pub fn load() -> Self {
        config_dir()
            .and_then(|d| std::fs::read(d.join("settings.json")).ok())
            .and_then(|b| serde_json::from_slice::<Settings>(&b).ok())
            .map(|mut s| {
                s.hotkeys.complete();
                s
            })
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let Some(dir) = config_dir() else { return };
        match serde_json::to_vec_pretty(self) {
            Ok(b) => {
                if let Err(e) = write_atomic(&dir.join("settings.json"), &b) {
                    log::warn!("saving settings: {e}");
                }
            }
            Err(e) => log::warn!("serializing settings: {e}"),
        }
    }
}

pub fn load_playlists() -> Vec<Playlist> {
    config_dir()
        .and_then(|d| std::fs::read(d.join("playlists.json")).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save_playlists(playlists: &[Playlist]) {
    let Some(dir) = config_dir() else { return };
    match serde_json::to_vec(playlists) {
        Ok(b) => {
            if let Err(e) = write_atomic(&dir.join("playlists.json"), &b) {
                log::warn!("saving playlists: {e}");
            }
        }
        Err(e) => log::warn!("serializing playlists: {e}"),
    }
}
