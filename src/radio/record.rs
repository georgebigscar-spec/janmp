//! Recording internet radio to files.
//!
//! `Original` writes the received bytes unchanged (lossless for MP3 / AAC / AAC+ streams).
//! Other formats encode the decoded PCM: WAV natively, the rest through ffmpeg.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Stdio};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum RecordFormat {
    /// The stream as received, without re-encoding.
    #[default]
    Original,
    Wav,
    Flac,
    Ogg,
    Mp3,
    Wv,
    Wma,
    Ape,
}

impl RecordFormat {
    pub const ALL: [RecordFormat; 8] = [
        RecordFormat::Original,
        RecordFormat::Flac,
        RecordFormat::Ape,
        RecordFormat::Wv,
        RecordFormat::Wav,
        RecordFormat::Ogg,
        RecordFormat::Mp3,
        RecordFormat::Wma,
    ];

    pub fn label(self) -> &'static str {
        match self {
            RecordFormat::Original => "Original",
            RecordFormat::Wav => "WAV",
            RecordFormat::Flac => "FLAC",
            RecordFormat::Ogg => "OGG Vorbis",
            RecordFormat::Mp3 => "MP3",
            RecordFormat::Wv => "WavPack",
            RecordFormat::Wma => "WMA",
            RecordFormat::Ape => "APE",
        }
    }

    fn extension(self) -> &'static str {
        match self {
            RecordFormat::Original => "",
            RecordFormat::Wav => "wav",
            RecordFormat::Flac => "flac",
            RecordFormat::Ogg => "ogg",
            RecordFormat::Mp3 => "mp3",
            RecordFormat::Wv => "wv",
            RecordFormat::Wma => "wma",
            RecordFormat::Ape => "ape",
        }
    }

    /// Encoder arguments for ffmpeg.
    fn ffmpeg_args(self) -> &'static [&'static str] {
        match self {
            RecordFormat::Flac => &["-c:a", "flac", "-compression_level", "5"],
            RecordFormat::Ogg => &["-c:a", "libvorbis", "-q:a", "6"],
            RecordFormat::Mp3 => &["-c:a", "libmp3lame", "-b:a", "320k"],
            RecordFormat::Wv => &["-c:a", "wavpack"],
            RecordFormat::Wma => &["-c:a", "wmav2", "-b:a", "192k"],
            _ => &[],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RecordSettings {
    pub format: RecordFormat,
    pub dir: PathBuf,
    /// Start a new file whenever the station announces a new song.
    pub split_by_title: bool,
}

impl Default for RecordSettings {
    fn default() -> Self {
        let dir = directories::UserDirs::new()
            .and_then(|u| u.audio_dir().map(Path::to_path_buf))
            .unwrap_or_else(|| PathBuf::from("."))
            .join("janmp Recordings");
        Self { format: RecordFormat::Original, dir, split_by_title: true }
    }
}

enum Sink {
    Raw(BufWriter<File>),
    Wav(hound::WavWriter<BufWriter<File>>),
    Ffmpeg { child: Child, stdin: BufWriter<ChildStdin> },
}

pub struct Recorder {
    settings: RecordSettings,
    station: String,
    raw_ext: &'static str,
    rate: u32,
    channels: usize,
    title: Option<String>,
    sink: Option<Sink>,
    pub files: Vec<PathBuf>,
}

/// Replace characters that are not allowed in file names.
fn sanitize(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control() { '_' } else { c })
        .collect();
    let trimmed = cleaned.trim().trim_end_matches('.').to_string();
    trimmed.chars().take(150).collect()
}

impl Recorder {
    pub fn new(
        settings: RecordSettings,
        station: &str,
        raw_ext: &'static str,
        rate: u32,
        channels: usize,
        title: Option<String>,
    ) -> Result<Self, String> {
        if settings.format == RecordFormat::Ape {
            return Err("APE encoding is not supported yet; choose FLAC or WavPack for lossless".into());
        }
        let needs_ffmpeg = !matches!(settings.format, RecordFormat::Original | RecordFormat::Wav);
        if needs_ffmpeg && crate::audio::ffmpeg::ffmpeg_path().is_none() {
            return Err(crate::audio::decoder::FFMPEG_MISSING.into());
        }
        std::fs::create_dir_all(&settings.dir).map_err(|e| e.to_string())?;
        let mut r = Self {
            settings,
            station: sanitize(station),
            raw_ext,
            rate,
            channels,
            title,
            sink: None,
            files: Vec::new(),
        };
        r.open_file()?;
        Ok(r)
    }

    pub fn is_raw(&self) -> bool {
        self.settings.format == RecordFormat::Original
    }

    fn open_file(&mut self) -> Result<(), String> {
        let stamp = chrono_like_stamp();
        let mut name = if self.station.is_empty() { "Radio".to_string() } else { self.station.clone() };
        if let Some(t) = &self.title {
            name = format!("{name} - {}", sanitize(t));
        }
        let ext = if self.is_raw() { self.raw_ext } else { self.settings.format.extension() };
        let path = self.settings.dir.join(format!("{name} [{stamp}].{ext}"));
        let sink = match self.settings.format {
            RecordFormat::Original => Sink::Raw(BufWriter::new(File::create(&path).map_err(|e| e.to_string())?)),
            RecordFormat::Wav => {
                let spec = hound::WavSpec {
                    channels: self.channels as u16,
                    sample_rate: self.rate,
                    bits_per_sample: 16,
                    sample_format: hound::SampleFormat::Int,
                };
                Sink::Wav(hound::WavWriter::create(&path, spec).map_err(|e| e.to_string())?)
            }
            fmt => {
                let ffmpeg = crate::audio::ffmpeg::ffmpeg_path().ok_or(crate::audio::decoder::FFMPEG_MISSING)?;
                let mut cmd = crate::audio::ffmpeg::command(ffmpeg);
                cmd.args(["-hide_banner", "-nostdin", "-loglevel", "error", "-y", "-f", "f32le"])
                    .arg("-ar")
                    .arg(self.rate.to_string())
                    .arg("-ac")
                    .arg(self.channels.to_string())
                    .args(["-i", "pipe:0"])
                    .args(fmt.ffmpeg_args());
                if let Some(t) = &self.title {
                    cmd.arg("-metadata").arg(format!("title={t}"));
                }
                cmd.arg("-metadata").arg(format!("album={}", self.station));
                let mut child = cmd
                    .arg(&path)
                    .stdin(Stdio::piped())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .map_err(|e| e.to_string())?;
                let stdin = BufWriter::new(child.stdin.take().ok_or("no stdin")?);
                Sink::Ffmpeg { child, stdin }
            }
        };
        log::info!("recording to {}", path.display());
        self.files.push(path);
        self.sink = Some(sink);
        Ok(())
    }

    fn close_file(&mut self) {
        match self.sink.take() {
            Some(Sink::Raw(mut w)) => {
                let _ = w.flush();
            }
            Some(Sink::Wav(w)) => {
                let _ = w.finalize();
            }
            Some(Sink::Ffmpeg { mut child, stdin }) => {
                drop(stdin);
                let _ = child.wait();
            }
            None => {}
        }
    }

    /// The station announced a new song.
    pub fn on_title(&mut self, title: &str) {
        self.title = Some(title.to_string());
        // A byte-level cut is safe for MP3/AAC frames, not for Ogg pages.
        let can_split = !(self.is_raw() && self.raw_ext == "ogg");
        if self.settings.split_by_title && can_split {
            self.close_file();
            if let Err(e) = self.open_file() {
                log::warn!("recording: {e}");
            }
        }
    }

    pub fn write_raw(&mut self, bytes: &[u8]) {
        if let Some(Sink::Raw(w)) = &mut self.sink {
            if let Err(e) = w.write_all(bytes) {
                log::warn!("recording: {e}");
                self.sink = None;
            }
        }
    }

    pub fn write_pcm(&mut self, samples: &[f32]) {
        let failed = match &mut self.sink {
            Some(Sink::Wav(w)) => samples
                .iter()
                .try_for_each(|s| w.write_sample((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16))
                .is_err(),
            Some(Sink::Ffmpeg { stdin, .. }) => {
                let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
                stdin.write_all(&bytes).is_err()
            }
            _ => false,
        };
        if failed {
            log::warn!("recording: write failed");
            self.sink = None;
        }
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        self.close_file();
    }
}

/// Local-time-free timestamp `YYYYMMDD-HHMMSS` (UTC) without pulling in a date crate.
fn chrono_like_stamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = secs / 86_400;
    let (h, m, s) = ((secs / 3600) % 24, (secs / 60) % 60, secs % 60);
    // Civil-from-days (Howard Hinnant's algorithm).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if mo <= 2 { 1 } else { 0 };
    format!("{y:04}{mo:02}{d:02}-{h:02}{m:02}{s:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_names() {
        assert_eq!(sanitize("AC/DC: Back?"), "AC_DC_ Back_");
    }

    #[test]
    fn stamp_shape() {
        let s = chrono_like_stamp();
        assert_eq!(s.len(), 15);
        assert!(s.starts_with("20"));
    }

    #[test]
    fn raw_recording_splits_on_title() {
        let dir = std::env::temp_dir().join(format!("janmp-rec-{}", std::process::id()));
        let settings = RecordSettings { format: RecordFormat::Original, dir: dir.clone(), split_by_title: true };
        let mut r = Recorder::new(settings, "Test FM", "mp3", 44_100, 2, Some("A - One".into())).unwrap();
        r.write_raw(b"first");
        r.on_title("B - Two");
        r.write_raw(b"second");
        let files = r.files.clone();
        drop(r);
        assert_eq!(files.len(), 2);
        assert!(files[1].file_name().unwrap().to_string_lossy().starts_with("Test FM - B - Two ["));
        assert_eq!(std::fs::read(&files[1]).unwrap(), b"second");
        let _ = std::fs::remove_dir_all(dir);
    }
}
