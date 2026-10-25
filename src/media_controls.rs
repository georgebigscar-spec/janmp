//! System media controls: MPRIS on Linux, the Windows media overlay (SMTC) and the macOS
//! Now Playing widget. Shows the current track and forwards play/pause/next/seek requests.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender};
use souvlaki::{MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig};

use crate::audio::PlaybackState;

/// What the OS is currently shown, to send only changes.
#[derive(Default, PartialEq, Clone)]
struct Shown {
    title: String,
    artist: String,
    album: String,
    duration: Option<u64>,
    /// `file://` URL and the file behind it.
    cover: Option<(String, PathBuf)>,
}

pub struct SystemControls {
    controls: Option<MediaControls>,
    rx: Receiver<MediaControlEvent>,
    shown: Shown,
    state: Option<PlaybackState>,
    /// Position last reported, and when, to detect seeks.
    reported: Option<(f64, f64)>,
    volume: Option<f64>,
    cover_file: Option<PathBuf>,
}

/// Write a cover image where the OS can read it, returning a `file://` URL.
fn cover_url(track: &Path) -> Option<(String, PathBuf)> {
    let data = crate::meta::find_cover(track)?;
    let ext = if data.starts_with(b"\x89PNG") { "png" } else { "jpg" };
    // A new name per image: desktop shells cache artwork by URL.
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("janmp-cover-{}-{n}.{ext}", std::process::id()));
    std::fs::write(&path, data).ok()?;
    let s = path.to_string_lossy().replace('\\', "/");
    let url = if s.starts_with('/') { format!("file://{s}") } else { format!("file:///{s}") };
    Some((url, path))
}

impl Drop for SystemControls {
    fn drop(&mut self) {
        if let Some(f) = self.cover_file.take() {
            let _ = std::fs::remove_file(f);
        }
    }
}

impl SystemControls {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        let controls = Self::create(cc, tx);
        Self { controls, rx, shown: Shown::default(), state: None, reported: None, volume: None, cover_file: None }
    }

    fn create(cc: &eframe::CreationContext<'_>, tx: Sender<MediaControlEvent>) -> Option<MediaControls> {
        #[cfg(windows)]
        let hwnd = {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            match cc.window_handle().ok()?.as_raw() {
                RawWindowHandle::Win32(h) => Some(h.hwnd.get() as *mut std::ffi::c_void),
                _ => None,
            }
        };
        #[cfg(not(windows))]
        let hwnd = None;
        let config = PlatformConfig { display_name: "janmp", dbus_name: "janmp", hwnd };
        let mut controls = match MediaControls::new(config) {
            Ok(c) => c,
            Err(e) => {
                log::warn!("system media controls unavailable: {e:?}");
                return None;
            }
        };
        let ctx = cc.egui_ctx.clone();
        if let Err(e) = controls.attach(move |event| {
            let _ = tx.send(event);
            ctx.request_repaint();
        }) {
            log::warn!("system media controls unavailable: {e:?}");
            return None;
        }
        Some(controls)
    }

    pub fn poll(&self) -> Vec<MediaControlEvent> {
        self.rx.try_iter().collect()
    }

    /// Publish the current track and playback state. Cheap when nothing changed.
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        track: Option<(&str, &str, &str, &Path, bool)>,
        duration: Option<f64>,
        state: PlaybackState,
        position: f64,
        volume: f64,
        now: f64,
    ) {
        let Some(controls) = self.controls.as_mut() else { return };
        let shown = match track {
            Some((title, artist, album, path, remote)) => {
                let same_track = self.shown.title == title && self.shown.artist == artist && self.shown.album == album;
                let cover = if same_track || remote { self.shown.cover.clone().filter(|_| same_track) } else { cover_url(path) };
                Shown {
                    title: title.to_string(),
                    artist: artist.to_string(),
                    album: album.to_string(),
                    duration: duration.map(|d| d as u64),
                    cover,
                }
            }
            None => Shown::default(),
        };
        if shown != self.shown {
            if self.shown.cover != shown.cover {
                if let Some(old) = self.cover_file.take() {
                    let _ = std::fs::remove_file(old);
                }
                self.cover_file = shown.cover.as_ref().map(|(_, f)| f.clone());
            }
            let meta = MediaMetadata {
                title: Some(shown.title.as_str()).filter(|s| !s.is_empty()),
                artist: Some(shown.artist.as_str()).filter(|s| !s.is_empty()),
                album: Some(shown.album.as_str()).filter(|s| !s.is_empty()),
                cover_url: shown.cover.as_ref().map(|(u, _)| u.as_str()),
                duration: shown.duration.map(Duration::from_secs),
            };
            if let Err(e) = controls.set_metadata(meta) {
                log::debug!("media controls metadata: {e:?}");
            }
            self.shown = shown;
            self.reported = None;
        }

        // Re-send the position on state changes and when it jumps (a seek); the OS
        // extrapolates it while playing.
        let expected = self.reported.map(|(pos, at)| if state == PlaybackState::Playing { pos + (now - at) } else { pos });
        let jumped = expected.is_none_or(|e| (e - position).abs() > 1.5);
        if self.state != Some(state) || jumped {
            let progress = Some(MediaPosition(Duration::from_secs_f64(position.max(0.0))));
            let playback = match state {
                PlaybackState::Playing => MediaPlayback::Playing { progress },
                PlaybackState::Paused => MediaPlayback::Paused { progress },
                PlaybackState::Stopped => MediaPlayback::Stopped,
            };
            if let Err(e) = controls.set_playback(playback) {
                log::debug!("media controls playback: {e:?}");
            }
            self.state = Some(state);
            self.reported = Some((position, now));
        }
        if self.volume.is_none_or(|v| (v - volume).abs() > 0.001) {
            // Only MPRIS has a volume property.
            #[cfg(all(unix, not(target_os = "macos")))]
            let _ = controls.set_volume(volume);
            self.volume = Some(volume);
        }
    }
}
