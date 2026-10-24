//! MIDI / RMI playback by SoundFont synthesis (rustysynth).

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;
use rustysynth::{MidiFile, MidiFileSequencer, SoundFont, Synthesizer, SynthesizerSettings};

use super::decoder::{AudioDecoder, StreamInfo};

const RATE: u32 = 44_100;
const BLOCK: usize = 1024;
/// Let the last notes ring out after the final event.
const TAIL_SECS: f64 = 1.0;

pub const SOUNDFONT_MISSING: &str =
    "MIDI needs a SoundFont (.sf2): choose one in Settings or put soundfont.sf2 next to janmp";

/// The SoundFont chosen in the settings, if any.
static CONFIGURED: Mutex<Option<PathBuf>> = Mutex::new(None);
/// The loaded SoundFont, shared by all MIDI decoders.
static LOADED: Mutex<Option<(PathBuf, Arc<SoundFont>)>> = Mutex::new(None);

pub fn set_soundfont(path: Option<PathBuf>) {
    *CONFIGURED.lock() = path.filter(|p| !p.as_os_str().is_empty());
}

/// The configured SoundFont, or the first one found in the usual places.
pub fn soundfont_path() -> Option<PathBuf> {
    if let Some(p) = CONFIGURED.lock().clone() {
        return Some(p);
    }
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        candidates.push(dir.join("soundfont.sf2"));
    }
    candidates.extend(
        [
            "/usr/share/sounds/sf2/default-GM.sf2",
            "/usr/share/sounds/sf2/FluidR3_GM.sf2",
            "/usr/share/sounds/sf2/TimGM6mb.sf2",
            "/usr/share/soundfonts/default.sf2",
            "/usr/share/soundfonts/FluidR3_GM.sf2",
            "/usr/local/share/soundfonts/default.sf2",
            "/opt/homebrew/share/soundfonts/default.sf2",
        ]
        .map(PathBuf::from),
    );
    candidates.into_iter().find(|p| p.is_file())
}

fn soundfont() -> Result<Arc<SoundFont>, String> {
    let path = soundfont_path().ok_or(SOUNDFONT_MISSING)?;
    let mut loaded = LOADED.lock();
    if let Some((p, sf)) = loaded.as_ref() {
        if *p == path {
            return Ok(sf.clone());
        }
    }
    let data = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let sf = Arc::new(SoundFont::new(&mut Cursor::new(data)).map_err(|e| format!("{}: {e:?}", path.display()))?);
    *loaded = Some((path, sf.clone()));
    Ok(sf)
}

/// RMI files wrap a standard MIDI file in a RIFF "RMID" container.
fn unwrap_rmi(data: &[u8]) -> &[u8] {
    if data.len() < 12 || &data[..4] != b"RIFF" || &data[8..12] != b"RMID" {
        return data;
    }
    let mut pos = 12;
    while pos + 8 <= data.len() {
        let id = &data[pos..pos + 4];
        let len = u32::from_le_bytes([data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]]) as usize;
        let body = pos + 8;
        if id == b"data" {
            return &data[body..(body + len).min(data.len())];
        }
        pos = body + len + (len & 1);
    }
    data
}

pub fn load_midi(path: &Path) -> Result<MidiFile, String> {
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    MidiFile::new(&mut Cursor::new(unwrap_rmi(&data))).map_err(|e| format!("invalid MIDI file: {e:?}"))
}

/// Length in seconds, for the playlist scanner.
pub fn duration(path: &Path) -> Option<f64> {
    load_midi(path).ok().map(|m| m.get_length())
}

pub struct MidiDecoder {
    sequencer: MidiFileSequencer,
    midi: Arc<MidiFile>,
    length: f64,
    frames: u64,
    left: Vec<f32>,
    right: Vec<f32>,
}

impl MidiDecoder {
    pub fn open(path: &Path) -> Result<Self, String> {
        let midi = Arc::new(load_midi(path)?);
        let sf = soundfont()?;
        let settings = SynthesizerSettings::new(RATE as i32);
        let synth = Synthesizer::new(&sf, &settings).map_err(|e| format!("{e:?}"))?;
        let mut sequencer = MidiFileSequencer::new(synth);
        sequencer.play(&midi, false);
        let length = midi.get_length();
        Ok(Self { sequencer, midi, length, frames: 0, left: vec![0.0; BLOCK], right: vec![0.0; BLOCK] })
    }
}

impl AudioDecoder for MidiDecoder {
    fn spec(&self) -> (u32, usize) {
        (RATE, 2)
    }

    fn duration(&self) -> Option<f64> {
        Some(self.length)
    }

    fn info(&self) -> StreamInfo {
        StreamInfo {
            codec: "MIDI".into(),
            sample_rate: RATE,
            channels: 2,
            bits_per_sample: None,
            bitrate: None,
            station: None,
        }
    }

    fn next_chunk(&mut self, out: &mut Vec<f32>) -> Result<bool, String> {
        let end = ((self.length + TAIL_SECS) * RATE as f64) as u64;
        if self.frames >= end {
            return Ok(false);
        }
        let n = BLOCK.min((end - self.frames) as usize);
        self.sequencer.render(&mut self.left[..n], &mut self.right[..n]);
        out.reserve(n * 2);
        for (l, r) in self.left[..n].iter().zip(&self.right[..n]) {
            out.push(*l);
            out.push(*r);
        }
        self.frames += n as u64;
        Ok(true)
    }

    fn seek(&mut self, secs: f64) -> Result<(), String> {
        // The sequencer can only play from the start: replay quickly up to the target so
        // programs, controllers and held notes are in the right state.
        let target = secs.clamp(0.0, self.length);
        self.sequencer.play(&self.midi, false);
        self.sequencer.set_speed(64.0);
        while self.sequencer.get_position() + (BLOCK as f64 * 64.0 / RATE as f64) < target {
            self.sequencer.render(&mut self.left, &mut self.right);
        }
        // Finish at normal speed for an accurate position.
        self.sequencer.set_speed(1.0);
        while self.sequencer.get_position() < target {
            let n = (((target - self.sequencer.get_position()) * RATE as f64).ceil() as usize).clamp(1, BLOCK);
            self.sequencer.render(&mut self.left[..n], &mut self.right[..n]);
        }
        self.frames = (target * RATE as f64) as u64;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unwraps_rmi() {
        let smf = b"MThd\0\0\0\x06\0\0\0\x01\0\x60";
        let mut rmi = b"RIFF\0\0\0\0RMID".to_vec();
        rmi.extend_from_slice(b"data");
        rmi.extend_from_slice(&(smf.len() as u32).to_le_bytes());
        rmi.extend_from_slice(smf);
        assert_eq!(unwrap_rmi(&rmi), smf);
        assert_eq!(unwrap_rmi(smf), smf);
    }

    /// One note per beat at 120 bpm for `beats` beats.
    fn scale(beats: u8) -> Vec<u8> {
        let mut ev = vec![0x00, 0xC0, 0x00];
        for i in 0..beats {
            ev.extend_from_slice(&[0x00, 0x90, 60 + i % 12, 100, 0x83, 0x60, 0x80, 60 + i % 12, 0]);
        }
        ev.extend_from_slice(&[0x00, 0xFF, 0x2F, 0x00]);
        let mut smf = b"MThd\0\0\0\x06\0\0\0\x01\x01\xE0MTrk".to_vec();
        smf.extend_from_slice(&(ev.len() as u32).to_be_bytes());
        smf.extend_from_slice(&ev);
        smf
    }

    #[test]
    fn renders_and_seeks() {
        if soundfont_path().is_none() {
            eprintln!("no SoundFont installed, skipping");
            return;
        }
        let path = std::env::temp_dir().join(format!("janmp-midi-{}.mid", std::process::id()));
        std::fs::write(&path, scale(16)).unwrap();
        let mut d = MidiDecoder::open(&path).unwrap();
        std::fs::remove_file(&path).ok();
        assert!((d.duration().unwrap() - 8.0).abs() < 0.01);
        let mut out = Vec::new();
        while out.len() < RATE as usize {
            d.next_chunk(&mut out).unwrap();
        }
        assert!(out.iter().any(|s| s.abs() > 0.01), "synth produced silence");

        let t = std::time::Instant::now();
        d.seek(7.0).unwrap();
        assert!(t.elapsed().as_secs_f64() < 2.0, "seek took {:?}", t.elapsed());
        let mut rest = Vec::new();
        while d.next_chunk(&mut rest).unwrap() {}
        let secs = rest.len() as f64 / 2.0 / RATE as f64;
        assert!((secs - (1.0 + TAIL_SECS)).abs() < 0.05, "rendered {secs} s after seek");
    }
}
