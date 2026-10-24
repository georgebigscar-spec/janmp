//! Decoder abstraction. Every backend produces interleaved `f32` PCM.

use std::fs::File;
use std::path::{Path, PathBuf};

use crate::radio::record::RecordSettings;

use symphonia::core::codecs::audio::{AudioDecoder as SymDecoder, AudioDecoderOptions};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::units::{Time, TimeBase};

use super::ffmpeg::{self, FfmpegDecoder};
use super::formats::{backend_for, Backend};

/// Technical description of an opened stream, shown in the player panel.
#[derive(Debug, Clone, Default)]
pub struct StreamInfo {
    pub codec: String,
    pub sample_rate: u32,
    pub channels: usize,
    pub bits_per_sample: Option<u32>,
    pub bitrate: Option<u32>,
    /// Radio station name (ICY `icy-name`).
    pub station: Option<String>,
}

pub trait AudioDecoder: Send {
    /// Current sample rate and channel count of the produced PCM.
    fn spec(&self) -> (u32, usize);
    /// Total duration in seconds, if known.
    fn duration(&self) -> Option<f64>;
    fn info(&self) -> StreamInfo;
    /// Decode the next portion of audio and append interleaved samples to `out`.
    /// Returns `Ok(false)` at end of stream.
    fn next_chunk(&mut self, out: &mut Vec<f32>) -> Result<bool, String>;
    /// Sample-accurate seek: the next chunk starts at `secs`.
    fn seek(&mut self, secs: f64) -> Result<(), String>;
    /// New "now playing" title announced by a radio station.
    fn take_title(&mut self) -> Option<String> {
        None
    }
    /// Start (`Some`) or stop (`None`) recording. Returns the files finished by this call.
    fn set_recording(&mut self, _settings: Option<RecordSettings>) -> Result<Vec<PathBuf>, String> {
        Err("only radio streams can be recorded".into())
    }
}

pub fn open(path: &Path) -> Result<Box<dyn AudioDecoder>, String> {
    let s = path.to_string_lossy();
    if super::stream::is_stream_url(&s) {
        return Ok(Box::new(super::stream::StreamDecoder::open(s.trim())?));
    }
    match backend_for(path) {
        Some(Backend::Symphonia) | None => match SymphoniaDecoder::open(path) {
            Ok(d) => Ok(Box::new(d)),
            // E.g. Opus inside .ogg / .webm: Symphonia demuxes it but has no decoder.
            Err(e) => FfmpegDecoder::open(path).map(|d| Box::new(d) as Box<dyn AudioDecoder>).map_err(|fe| {
                if ffmpeg::ffmpeg_path().is_some() { fe } else { e }
            }),
        },
        Some(Backend::Ffmpeg) | Some(Backend::Tracker) => {
            if ffmpeg::ffmpeg_path().is_none() {
                return Err(FFMPEG_MISSING.into());
            }
            Ok(Box::new(FfmpegDecoder::open(path)?))
        }
        Some(Backend::Midi) => Err("MIDI playback is not available yet".into()),
    }
}

pub const FFMPEG_MISSING: &str =
    "FFmpeg is required for this format: install it or put ffmpeg next to janmp";

pub struct SymphoniaDecoder {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn SymDecoder>,
    track_id: u32,
    time_base: Option<TimeBase>,
    rate: u32,
    channels: usize,
    duration: Option<f64>,
    /// After a seek, drop audio that lies before this position (seconds).
    skip_until: Option<f64>,
    info: StreamInfo,
    tmp: Vec<f32>,
}

impl SymphoniaDecoder {
    pub fn open(path: &Path) -> Result<Self, String> {
        let file = File::open(path).map_err(|e| e.to_string())?;
        let mss = MediaSourceStream::new(Box::new(file), Default::default());
        let mut hint = Hint::new();
        if let Some(ext) = path.extension() {
            hint.with_extension(&ext.to_string_lossy());
        }
        let format = symphonia::default::get_probe()
            .probe(&hint, mss, FormatOptions::default(), MetadataOptions::default())
            .map_err(|e| e.to_string())?;
        Self::from_format(format)
    }

    pub fn from_format(format: Box<dyn FormatReader>) -> Result<Self, String> {
        let track = format
            .default_track(TrackType::Audio)
            .ok_or_else(|| "no audio track".to_string())?;
        let params = track
            .codec_params
            .as_ref()
            .and_then(|p| p.audio())
            .ok_or_else(|| "no audio codec parameters".to_string())?
            .clone();
        let decoder = symphonia::default::get_codecs()
            .make_audio_decoder(&params, &AudioDecoderOptions::default())
            .map_err(|e| e.to_string())?;

        let rate = params.sample_rate.unwrap_or(44_100);
        let channels = params.channels.as_ref().map(|c| c.count()).unwrap_or(2).max(1);
        let time_base = track.time_base;
        let duration = match (time_base, track.duration) {
            (Some(tb), Some(d)) => tb.calc_duration(d).map(|t| t.as_secs_f64()),
            _ => None,
        }
        .or_else(|| track.num_frames.map(|n| n as f64 / rate as f64));

        let info = StreamInfo {
            codec: decoder.codec_info().short_name.to_uppercase(),
            sample_rate: rate,
            channels,
            bits_per_sample: params.bits_per_sample,
            bitrate: None,
            station: None,
        };
        let track_id = track.id;
        Ok(Self {
            format,
            decoder,
            track_id,
            time_base,
            rate,
            channels,
            duration,
            skip_until: None,
            info,
            tmp: Vec::new(),
        })
    }
}

impl AudioDecoder for SymphoniaDecoder {
    fn spec(&self) -> (u32, usize) {
        (self.rate, self.channels)
    }

    fn duration(&self) -> Option<f64> {
        self.duration
    }

    fn info(&self) -> StreamInfo {
        self.info.clone()
    }

    fn next_chunk(&mut self, out: &mut Vec<f32>) -> Result<bool, String> {
        // Bound the number of consecutive bad packets so a broken file cannot spin forever.
        let mut errors = 0;
        loop {
            let packet = match self.format.next_packet() {
                Ok(Some(p)) => p,
                Ok(None) => return Ok(false),
                Err(SymError::ResetRequired) => return Ok(false),
                Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    return Ok(false)
                }
                Err(e) => return Err(e.to_string()),
            };
            if packet.track_id != self.track_id {
                continue;
            }
            let buf = match self.decoder.decode(&packet) {
                Ok(b) => b,
                Err(SymError::DecodeError(_)) | Err(SymError::IoError(_)) => {
                    errors += 1;
                    if errors > 100 {
                        return Err("too many decode errors".into());
                    }
                    continue;
                }
                Err(e) => return Err(e.to_string()),
            };
            let spec = buf.spec();
            self.rate = spec.rate();
            self.channels = spec.channels().count().max(1);
            buf.copy_to_vec_interleaved(&mut self.tmp);

            let mut start = 0usize;
            if let Some(target) = self.skip_until {
                let pkt_time = self
                    .time_base
                    .and_then(|tb| tb.calc_time(packet.pts))
                    .map(|t| t.as_secs_f64())
                    .unwrap_or(target);
                let frames = self.tmp.len() / self.channels;
                let pkt_end = pkt_time + frames as f64 / self.rate as f64;
                if pkt_end <= target {
                    continue;
                }
                if pkt_time < target {
                    let drop = ((target - pkt_time) * self.rate as f64).round() as usize;
                    start = drop.min(frames) * self.channels;
                }
                self.skip_until = None;
            }
            out.extend_from_slice(&self.tmp[start..]);
            return Ok(true);
        }
    }

    fn seek(&mut self, secs: f64) -> Result<(), String> {
        let time = Time::try_from_secs_f64(secs.max(0.0)).ok_or("bad seek time")?;
        self.format
            .seek(SeekMode::Accurate, SeekTo::Time { time, track_id: Some(self.track_id) })
            .map_err(|e| e.to_string())?;
        self.decoder.reset();
        self.skip_until = Some(secs);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Write a 16-bit PCM WAV with a ramp so sample positions are recognisable.
    fn write_wav(path: &Path, rate: u32, channels: u16, frames: u32) {
        let mut data = Vec::new();
        for i in 0..frames {
            let v = ((i % 1000) as i16 - 500) * 40;
            for _ in 0..channels {
                data.extend_from_slice(&v.to_le_bytes());
            }
        }
        let mut f = Vec::new();
        let byte_rate = rate * channels as u32 * 2;
        f.extend_from_slice(b"RIFF");
        f.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        f.extend_from_slice(b"WAVEfmt ");
        f.extend_from_slice(&16u32.to_le_bytes());
        f.extend_from_slice(&1u16.to_le_bytes());
        f.extend_from_slice(&channels.to_le_bytes());
        f.extend_from_slice(&rate.to_le_bytes());
        f.extend_from_slice(&byte_rate.to_le_bytes());
        f.extend_from_slice(&(channels * 2).to_le_bytes());
        f.extend_from_slice(&16u16.to_le_bytes());
        f.extend_from_slice(b"data");
        f.extend_from_slice(&(data.len() as u32).to_le_bytes());
        f.extend_from_slice(&data);
        std::fs::write(path, f).unwrap();
    }

    fn decode_all(dec: &mut dyn AudioDecoder) -> Vec<f32> {
        let mut out = Vec::new();
        while dec.next_chunk(&mut out).unwrap() {}
        out
    }

    #[test]
    fn decodes_and_seeks_wav() {
        let dir = std::env::temp_dir().join(format!("janmp-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ramp.wav");
        write_wav(&path, 8000, 2, 16000);

        let mut dec = open(&path).unwrap();
        assert_eq!(dec.spec(), (8000, 2));
        assert!((dec.duration().unwrap() - 2.0).abs() < 1e-6);
        assert_eq!(decode_all(dec.as_mut()).len(), 32000);

        dec.seek(1.5).unwrap();
        let rest = decode_all(dec.as_mut());
        assert_eq!(rest.len(), 4000 * 2, "seek must be sample accurate");
        // Frame 12000 has ramp value (12000 % 1000 - 500) * 40.
        let expect = -500.0 * 40.0 / 32768.0;
        assert!((rest[0] - expect).abs() < 1e-3, "got {}", rest[0]);
        let _ = std::fs::remove_dir_all(dir);
    }
}
