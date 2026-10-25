//! Track waveforms for the seek bar: computed in the background by decoding the track,
//! shown progressively while they are being computed, and cached on disk.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crossbeam_channel::{Receiver, Sender};

use crate::audio::engine::MediaRequest;

/// Columns per waveform, independent of the bar's width.
pub const POINTS: usize = 600;

/// Peak and RMS per column, 0..=255.
#[derive(Debug, Clone, Default)]
pub struct Waveform {
    pub peak: Vec<u8>,
    pub rms: Vec<u8>,
    /// Columns computed so far (`POINTS` when finished).
    pub done: usize,
}

impl Waveform {
    pub fn is_complete(&self) -> bool {
        self.done >= POINTS
    }

    fn encode(&self) -> Vec<u8> {
        let mut v = b"JWF1".to_vec();
        v.extend_from_slice(&self.peak);
        v.extend_from_slice(&self.rms);
        v
    }

    fn decode(b: &[u8]) -> Option<Self> {
        let body = b.strip_prefix(b"JWF1")?;
        (body.len() == POINTS * 2).then(|| Self { peak: body[..POINTS].to_vec(), rms: body[POINTS..].to_vec(), done: POINTS })
    }
}

struct Job {
    key: String,
    req: MediaRequest,
    generation: u64,
}

pub struct WaveformService {
    tx: Sender<Job>,
    rx: Receiver<(String, Waveform)>,
    generation: Arc<AtomicU64>,
    cache: HashMap<String, Arc<Waveform>>,
    /// Key of the waveform being computed or shown.
    current: Option<String>,
}

fn cache_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "janmp").map(|d| d.cache_dir().join("waveforms"))
}

/// Disk cache name: the item plus the file's size and modification time, so edited files
/// are recomputed.
fn cache_file(req: &MediaRequest) -> Option<PathBuf> {
    let md = std::fs::metadata(&req.path).ok()?;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    req.path.hash(&mut h);
    req.start.to_bits().hash(&mut h);
    req.end.map(f64::to_bits).hash(&mut h);
    md.len().hash(&mut h);
    md.modified().ok().hash(&mut h);
    Some(cache_dir()?.join(format!("{:016x}.wf", h.finish())))
}

/// Which items get a waveform: local files, but not radio, web pages or CD tracks
/// (reading the disc twice would disturb playback).
pub fn supported(req: &MediaRequest) -> bool {
    let s = req.path.to_string_lossy();
    !crate::audio::stream::is_remote_url(&s)
        && !crate::audio::cdda::is_cd_url(&s)
        && crate::audio::formats::backend_for(&req.path) != Some(crate::audio::formats::Backend::Cd)
        && req.path.is_file()
}

fn compute(job: &Job, generation: &AtomicU64, tx: &Sender<(String, Waveform)>, repaint: &dyn Fn()) -> Result<(), String> {
    let stale = || generation.load(Ordering::Relaxed) != job.generation;
    let mut dec = crate::audio::decoder::open(&job.req.path)?;
    if job.req.start > 0.0 {
        dec.seek(job.req.start)?;
    }
    let (rate, ch) = dec.spec();
    let duration = match (job.req.end, dec.duration()) {
        (Some(e), _) => e - job.req.start,
        (None, Some(d)) => d - job.req.start,
        _ => return Err("unknown duration".into()),
    };
    if duration <= 0.0 {
        return Err("empty".into());
    }
    let total_frames = (duration * rate as f64).max(1.0);
    let per_point = total_frames / POINTS as f64;
    let mut peak = vec![0f32; POINTS];
    let mut sq = vec![0f64; POINTS];
    let mut count = vec![0u32; POINTS];
    let mut frame: u64 = 0;
    let mut buf = Vec::new();
    let mut last_sent = std::time::Instant::now();
    let snapshot = |peak: &[f32], sq: &[f64], count: &[u32], done: usize| {
        // Normalize to the loudest column so quiet masters still show their shape.
        let max = peak.iter().copied().fold(0.0f32, f32::max).max(1e-4);
        Waveform {
            peak: peak.iter().map(|p| (p / max * 255.0).round() as u8).collect(),
            rms: sq
                .iter()
                .zip(count)
                .map(|(s, n)| if *n == 0 { 0 } else { ((s / *n as f64).sqrt() as f32 / max * 255.0).round().min(255.0) as u8 })
                .collect(),
            done,
        }
    };
    loop {
        if stale() {
            return Ok(());
        }
        buf.clear();
        let more = dec.next_chunk(&mut buf)?;
        let (r, c) = dec.spec();
        if (r, c) != (rate, ch) {
            break; // A format change mid-file: keep what we have.
        }
        for f in buf.chunks_exact(ch) {
            let i = (frame as f64 / per_point) as usize;
            if i >= POINTS {
                break;
            }
            let m = f.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
            let avg = f.iter().sum::<f32>() / ch as f32;
            peak[i] = peak[i].max(m);
            sq[i] += (avg * avg) as f64;
            count[i] += 1;
            frame += 1;
        }
        let done = ((frame as f64 / per_point) as usize).min(POINTS);
        if done >= POINTS || !more {
            break;
        }
        if last_sent.elapsed().as_millis() > 200 {
            let _ = tx.send((job.key.clone(), snapshot(&peak, &sq, &count, done)));
            repaint();
            last_sent = std::time::Instant::now();
        }
    }
    let wf = snapshot(&peak, &sq, &count, POINTS);
    if let Some(f) = cache_file(&job.req) {
        if let Some(dir) = f.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(f, wf.encode());
    }
    let _ = tx.send((job.key.clone(), wf));
    repaint();
    Ok(())
}

impl WaveformService {
    pub fn new(repaint: impl Fn() + Send + 'static) -> Self {
        let (tx, jobs) = crossbeam_channel::unbounded::<Job>();
        let (rtx, rx) = crossbeam_channel::unbounded();
        let generation = Arc::new(AtomicU64::new(0));
        let g = generation.clone();
        std::thread::Builder::new()
            .name("waveform".into())
            .spawn(move || {
                while let Ok(mut job) = jobs.recv() {
                    // Only the newest request matters.
                    while let Ok(j) = jobs.try_recv() {
                        job = j;
                    }
                    if g.load(Ordering::Relaxed) != job.generation {
                        continue;
                    }
                    if let Some(wf) = cache_file(&job.req).and_then(|f| std::fs::read(f).ok()).and_then(|b| Waveform::decode(&b)) {
                        let _ = rtx.send((job.key.clone(), wf));
                        repaint();
                        continue;
                    }
                    if let Err(e) = compute(&job, &g, &rtx, &repaint) {
                        log::debug!("waveform for {}: {e}", job.req.path.display());
                    }
                }
            })
            .expect("spawn waveform thread");
        Self { tx, rx, generation, cache: HashMap::new(), current: None }
    }

    /// The waveform for `key`, starting its computation when needed.
    pub fn get(&mut self, key: &str, req: &MediaRequest) -> Option<Arc<Waveform>> {
        for (k, wf) in self.rx.try_iter() {
            if self.cache.len() > 64 {
                self.cache.retain(|ck, w| ck == &k || !w.is_complete());
            }
            self.cache.insert(k, Arc::new(wf));
        }
        if self.current.as_deref() != Some(key) {
            self.current = Some(key.to_string());
            let done = self.cache.get(key).is_some_and(|w| w.is_complete());
            // A new request makes any computation in progress stale.
            let generation = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
            if !done && supported(req) {
                self.cache.remove(key);
                let _ = self.tx.send(Job { key: key.to_string(), req: req.clone(), generation });
            }
        }
        self.cache.get(key).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn computes_and_encodes() {
        // A WAV whose second half is silent.
        let path = std::env::temp_dir().join(format!("janmp-wf-{}.wav", std::process::id()));
        let spec = hound::WavSpec { channels: 1, sample_rate: 8_000, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
        let mut w = hound::WavWriter::create(&path, spec).unwrap();
        for i in 0..16_000 {
            let s = if i < 8_000 { ((i as f32 * 0.3).sin() * 20_000.0) as i16 } else { 0 };
            w.write_sample(s).unwrap();
        }
        w.finalize().unwrap();

        let (tx, rx) = crossbeam_channel::unbounded();
        let job = Job { key: "k".into(), req: MediaRequest { path: path.clone(), start: 0.0, end: None }, generation: 0 };
        compute(&job, &AtomicU64::new(0), &tx, &|| {}).unwrap();
        let wf = rx.try_iter().last().unwrap().1;
        assert!(wf.is_complete());
        assert!(wf.peak[..POINTS / 2 - 2].iter().all(|p| *p > 200));
        assert!(wf.peak[POINTS / 2 + 2..].iter().all(|p| *p == 0));
        let back = Waveform::decode(&wf.encode()).unwrap();
        assert_eq!(back.peak, wf.peak);
        if let Some(f) = cache_file(&job.req) {
            let _ = std::fs::remove_file(f);
        }
        std::fs::remove_file(&path).ok();
    }
}
