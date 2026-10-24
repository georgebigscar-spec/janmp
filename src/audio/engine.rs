//! Playback engine: a worker thread that owns the output stream and the active decoder.
//!
//! The UI talks to it through [`Player`] (commands in, events out) and reads the live
//! position, volume and visualizer samples from [`Shared`] without locking.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};

use super::decoder::{self, AudioDecoder, StreamInfo};
use super::output::Output;
use super::resample::StreamResampler;
use super::vis::VisTap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackState {
    Stopped,
    Playing,
    Paused,
}

/// A piece of media to play: a whole file or a CUE-defined region of it.
#[derive(Debug, Clone)]
pub struct MediaRequest {
    pub path: PathBuf,
    /// Start offset inside the file, in seconds.
    pub start: f64,
    /// Optional end offset inside the file, in seconds.
    pub end: Option<f64>,
}

enum Command {
    Load(MediaRequest),
    /// Queue the next item for gapless continuation.
    Enqueue(MediaRequest),
    Play,
    Pause,
    Stop,
    Seek(f64),
    Shutdown,
}

#[derive(Debug, Clone)]
pub enum PlayerEvent {
    /// A newly loaded item started playing.
    Started { info: StreamInfo, duration: Option<f64> },
    /// The current item is about to run out; enqueue the next one now for gapless playback.
    AboutToFinish,
    /// Playback moved on to the enqueued item.
    TrackChanged { info: StreamInfo, duration: Option<f64> },
    /// The last item finished and nothing was enqueued.
    Finished,
    Error(String),
}

/// State shared between the engine thread, the audio callback and the UI.
pub struct Shared {
    pub paused: AtomicBool,
    pub volume: AtomicU32,
    pub balance: AtomicU32,
    pub played_frames: AtomicU64,
    pub flush_req: AtomicU64,
    pub flush_ack: AtomicU64,
    pub out_rate: AtomicU32,
    pub out_channels: AtomicU32,
    pub stream_error: AtomicBool,
    pub vis: VisTap,
    state: AtomicU8,
    /// Position (seconds, f64 bits) of the current item when `track_start_frames` was played.
    base_pos: AtomicU64,
    track_start_frames: AtomicU64,
    duration: AtomicU64,
}

impl Shared {
    fn new(volume: f32) -> Self {
        Self {
            paused: AtomicBool::new(false),
            volume: AtomicU32::new(volume.to_bits()),
            balance: AtomicU32::new(0f32.to_bits()),
            played_frames: AtomicU64::new(0),
            flush_req: AtomicU64::new(0),
            flush_ack: AtomicU64::new(0),
            out_rate: AtomicU32::new(44_100),
            out_channels: AtomicU32::new(2),
            stream_error: AtomicBool::new(false),
            vis: VisTap::default(),
            state: AtomicU8::new(0),
            base_pos: AtomicU64::new(0f64.to_bits()),
            track_start_frames: AtomicU64::new(0),
            duration: AtomicU64::new(f64::NAN.to_bits()),
        }
    }

    fn set_state(&self, s: PlaybackState) {
        self.state.store(s as u8, Ordering::Relaxed);
    }
}

/// Handle to the playback engine, owned by the UI.
pub struct Player {
    tx: Sender<Command>,
    events: Receiver<PlayerEvent>,
    shared: Arc<Shared>,
}

impl Player {
    pub fn new(volume: f32, repaint: impl Fn() + Send + 'static) -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        let (etx, events) = crossbeam_channel::unbounded();
        let shared = Arc::new(Shared::new(volume));
        let s = shared.clone();
        std::thread::Builder::new()
            .name("janmp-engine".into())
            .spawn(move || Engine::new(s, rx, etx, Box::new(repaint)).run())
            .expect("spawn engine thread");
        Self { tx, events, shared }
    }

    pub fn load(&self, req: MediaRequest) {
        let _ = self.tx.send(Command::Load(req));
    }
    pub fn enqueue(&self, req: MediaRequest) {
        let _ = self.tx.send(Command::Enqueue(req));
    }
    pub fn play(&self) {
        let _ = self.tx.send(Command::Play);
    }
    pub fn pause(&self) {
        let _ = self.tx.send(Command::Pause);
    }
    pub fn stop(&self) {
        let _ = self.tx.send(Command::Stop);
    }
    pub fn seek(&self, secs: f64) {
        let _ = self.tx.send(Command::Seek(secs));
    }
    pub fn set_volume(&self, v: f32) {
        self.shared.volume.store(v.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }
    pub fn set_balance(&self, b: f32) {
        self.shared.balance.store(b.clamp(-1.0, 1.0).to_bits(), Ordering::Relaxed);
    }

    pub fn poll_event(&self) -> Option<PlayerEvent> {
        self.events.try_recv().ok()
    }

    pub fn state(&self) -> PlaybackState {
        match self.shared.state.load(Ordering::Relaxed) {
            1 => PlaybackState::Playing,
            2 => PlaybackState::Paused,
            _ => PlaybackState::Stopped,
        }
    }

    /// Current position within the playing item, in seconds.
    pub fn position(&self) -> f64 {
        if self.state() == PlaybackState::Stopped {
            return 0.0;
        }
        let s = &self.shared;
        let rate = s.out_rate.load(Ordering::Relaxed).max(1) as f64;
        let played = s.played_frames.load(Ordering::Relaxed);
        let start = s.track_start_frames.load(Ordering::Relaxed);
        let base = f64::from_bits(s.base_pos.load(Ordering::Relaxed));
        base + played.saturating_sub(start) as f64 / rate
    }

    pub fn duration(&self) -> Option<f64> {
        let d = f64::from_bits(self.shared.duration.load(Ordering::Relaxed));
        d.is_finite().then_some(d)
    }

    pub fn vis_snapshot(&self, out: &mut [f32]) {
        self.shared.vis.snapshot(out);
    }

    pub fn output_rate(&self) -> u32 {
        self.shared.out_rate.load(Ordering::Relaxed)
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        let _ = self.tx.send(Command::Shutdown);
    }
}

/// The item currently being decoded (which can be ahead of what is audible).
struct Current {
    dec: Box<dyn AudioDecoder>,
    req: MediaRequest,
    duration: Option<f64>,
    /// Source position of the next decoded sample, in seconds.
    src_pos: f64,
    resampler: Option<StreamResampler>,
    src_spec: (u32, usize),
    eof: bool,
}

/// An enqueued item whose first sample has been written but is not audible yet.
struct Transition {
    at_frame: u64,
    info: StreamInfo,
    duration: Option<f64>,
}

struct Engine {
    shared: Arc<Shared>,
    rx: Receiver<Command>,
    events: Sender<PlayerEvent>,
    repaint: Box<dyn Fn() + Send>,
    output: Option<Output>,
    current: Option<Current>,
    transition: Option<Transition>,
    /// Converted samples not yet pushed into the ring buffer.
    pending: Vec<f32>,
    pending_off: usize,
    /// Frames written into the ring buffer since the last flush.
    written_frames: u64,
    about_to_finish_sent: bool,
    scratch: Vec<f32>,
    mapped: Vec<f32>,
}

impl Engine {
    fn new(
        shared: Arc<Shared>,
        rx: Receiver<Command>,
        events: Sender<PlayerEvent>,
        repaint: Box<dyn Fn() + Send>,
    ) -> Self {
        let output = match Output::open(shared.clone()) {
            Ok(o) => Some(o),
            Err(e) => {
                let _ = events.send(PlayerEvent::Error(format!("audio output: {e}")));
                None
            }
        };
        Self {
            shared,
            rx,
            events,
            repaint,
            output,
            current: None,
            transition: None,
            pending: Vec::new(),
            pending_off: 0,
            written_frames: 0,
            about_to_finish_sent: false,
            scratch: Vec::new(),
            mapped: Vec::new(),
        }
    }

    fn emit(&self, e: PlayerEvent) {
        let _ = self.events.send(e);
        (self.repaint)();
    }

    fn run(mut self) {
        loop {
            // Block while idle, otherwise poll briefly so decoding keeps flowing.
            let busy = self.current.is_some();
            let cmd = if busy {
                match self.rx.recv_timeout(Duration::from_millis(if self.buffer_full() { 10 } else { 0 })) {
                    Ok(c) => Some(c),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            } else {
                match self.rx.recv_timeout(Duration::from_millis(200)) {
                    Ok(c) => Some(c),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            };
            if let Some(cmd) = cmd {
                if !self.handle(cmd) {
                    return;
                }
                continue;
            }
            if self.shared.stream_error.swap(false, Ordering::Relaxed) {
                self.reopen_output();
            }
            self.check_transition();
            if self.current.is_some() {
                self.step();
            }
        }
    }

    fn buffer_full(&self) -> bool {
        self.pending_off < self.pending.len()
            && self.output.as_ref().is_some_and(|o| o.producer.slots() == 0)
    }

    fn handle(&mut self, cmd: Command) -> bool {
        match cmd {
            Command::Load(req) => self.load(req),
            Command::Enqueue(req) => self.enqueue(req),
            Command::Play => {
                if self.current.is_some() {
                    self.shared.paused.store(false, Ordering::Relaxed);
                    self.shared.set_state(PlaybackState::Playing);
                }
            }
            Command::Pause => {
                if self.current.is_some() {
                    self.shared.paused.store(true, Ordering::Relaxed);
                    self.shared.set_state(PlaybackState::Paused);
                }
            }
            Command::Stop => {
                self.flush();
                self.current = None;
                self.transition = None;
                self.shared.set_state(PlaybackState::Stopped);
                self.shared.vis.clear();
            }
            Command::Seek(t) => self.seek(t),
            Command::Shutdown => return false,
        }
        (self.repaint)();
        true
    }

    fn open(req: &MediaRequest) -> Result<Current, String> {
        let mut dec = decoder::open(&req.path)?;
        if req.start > 0.0 {
            dec.seek(req.start)?;
        }
        let total = dec.duration();
        let duration = match (req.end, total) {
            (Some(e), _) => Some(e - req.start),
            (None, Some(t)) => Some(t - req.start),
            _ => None,
        };
        let src_spec = dec.spec();
        Ok(Current {
            dec,
            req: req.clone(),
            duration,
            src_pos: req.start,
            resampler: None,
            src_spec,
            eof: false,
        })
    }

    fn load(&mut self, req: MediaRequest) {
        self.flush();
        self.transition = None;
        match Self::open(&req) {
            Ok(cur) => {
                let info = cur.dec.info();
                self.set_timeline(0.0, 0, cur.duration);
                self.emit(PlayerEvent::Started { info, duration: cur.duration });
                self.current = Some(cur);
                self.about_to_finish_sent = false;
                self.shared.paused.store(false, Ordering::Relaxed);
                self.shared.set_state(PlaybackState::Playing);
            }
            Err(e) => {
                self.current = None;
                self.shared.set_state(PlaybackState::Stopped);
                self.emit(PlayerEvent::Error(format!("{}: {e}", req.path.display())));
            }
        }
    }

    fn enqueue(&mut self, req: MediaRequest) {
        // Only meaningful while the current item is draining.
        let draining = self.current.as_ref().is_some_and(|c| c.eof);
        if !draining {
            return self.load(req);
        }
        match Self::open(&req) {
            Ok(cur) => {
                // Flush the old item's resampler tail before switching.
                self.transition = Some(Transition {
                    at_frame: self.written_frames + (self.pending.len() - self.pending_off) as u64
                        / self.out_channels() as u64,
                    info: cur.dec.info(),
                    duration: cur.duration,
                });
                self.current = Some(cur);
                self.about_to_finish_sent = false;
            }
            Err(e) => self.emit(PlayerEvent::Error(format!("{}: {e}", req.path.display()))),
        }
    }

    fn seek(&mut self, t: f64) {
        if self.transition.is_some() {
            return;
        }
        let Some(cur) = self.current.as_mut() else { return };
        let t = match cur.duration {
            Some(d) => t.clamp(0.0, (d - 0.05).max(0.0)),
            None => t.max(0.0),
        };
        let target = cur.req.start + t;
        if let Err(e) = cur.dec.seek(target) {
            log::warn!("seek failed: {e}");
            return;
        }
        cur.src_pos = target;
        cur.eof = false;
        if let Some(r) = cur.resampler.as_mut() {
            r.reset();
        }
        let duration = cur.duration;
        self.flush();
        self.about_to_finish_sent = false;
        self.set_timeline(t, 0, duration);
    }

    fn set_timeline(&self, base: f64, start_frames: u64, duration: Option<f64>) {
        self.shared.base_pos.store(base.to_bits(), Ordering::Relaxed);
        self.shared.track_start_frames.store(start_frames, Ordering::Relaxed);
        self.shared.duration.store(duration.unwrap_or(f64::NAN).to_bits(), Ordering::Relaxed);
    }

    /// Drop everything buffered, waiting for the audio callback to acknowledge.
    fn flush(&mut self) {
        self.pending.clear();
        self.pending_off = 0;
        let req = self.shared.flush_req.fetch_add(1, Ordering::AcqRel) + 1;
        if self.output.is_some() {
            let deadline = Instant::now() + Duration::from_millis(300);
            while self.shared.flush_ack.load(Ordering::Acquire) != req && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        self.shared.played_frames.store(0, Ordering::Relaxed);
        self.written_frames = 0;
        self.shared.vis.clear();
    }

    fn out_channels(&self) -> usize {
        self.output.as_ref().map(|o| o.channels).unwrap_or(2)
    }

    fn reopen_output(&mut self) {
        let pos = Player::position_from(&self.shared);
        self.output = None;
        match Output::open(self.shared.clone()) {
            Ok(o) => {
                self.output = Some(o);
                if let Some(c) = self.current.as_mut() {
                    c.resampler = None;
                }
                self.seek(pos);
            }
            Err(e) => self.emit(PlayerEvent::Error(format!("audio output: {e}"))),
        }
    }

    fn check_transition(&mut self) {
        let Some(t) = &self.transition else { return };
        if self.shared.played_frames.load(Ordering::Relaxed) >= t.at_frame {
            let t = self.transition.take().unwrap();
            self.set_timeline(0.0, t.at_frame, t.duration);
            self.emit(PlayerEvent::TrackChanged { info: t.info, duration: t.duration });
        }
    }

    /// One unit of work: push pending samples or decode more.
    fn step(&mut self) {
        let Some(out) = self.output.as_mut() else {
            // No device: pretend to play nothing.
            std::thread::sleep(Duration::from_millis(50));
            return;
        };

        if self.pending_off < self.pending.len() {
            let n = out.producer.slots().min(self.pending.len() - self.pending_off);
            let n = n - n % out.channels;
            if n > 0 {
                if let Ok(chunk) = out.producer.write_chunk_uninit(n) {
                    chunk.fill_from_iter(self.pending[self.pending_off..self.pending_off + n].iter().copied());
                    self.pending_off += n;
                    self.written_frames += (n / out.channels) as u64;
                }
            }
            if self.pending_off >= self.pending.len() {
                self.pending.clear();
                self.pending_off = 0;
            }
            return;
        }

        let (out_rate, out_ch) = (out.rate, out.channels);
        let cap = out.producer.buffer().capacity();
        let drained = out.producer.slots() == cap;
        let cur = self.current.as_mut().unwrap();

        if cur.eof {
            if !self.about_to_finish_sent {
                self.about_to_finish_sent = true;
                self.emit(PlayerEvent::AboutToFinish);
            }
            if drained && self.transition.is_none() {
                self.current = None;
                self.shared.set_state(PlaybackState::Stopped);
                self.emit(PlayerEvent::Finished);
            } else {
                std::thread::sleep(Duration::from_millis(5));
            }
            return;
        }

        self.scratch.clear();
        let more = match cur.dec.next_chunk(&mut self.scratch) {
            Ok(m) => m,
            Err(e) => {
                log::warn!("decode error: {e}");
                false
            }
        };
        let (rate, ch) = cur.dec.spec();
        if (rate, ch) != cur.src_spec {
            cur.src_spec = (rate, ch);
            cur.resampler = None;
        }
        let mut frames = self.scratch.len() / ch;
        let mut reached_end = !more;
        if let Some(end) = cur.req.end {
            let remaining = ((end - cur.src_pos) * rate as f64).max(0.0) as usize;
            if frames >= remaining {
                frames = remaining;
                reached_end = true;
            }
        }
        cur.src_pos += frames as f64 / rate as f64;

        map_channels(&self.scratch[..frames * ch], ch, out_ch, &mut self.mapped);
        if rate != out_rate && cur.resampler.is_none() {
            match StreamResampler::new(rate, out_rate, out_ch) {
                Ok(r) => cur.resampler = Some(r),
                Err(e) => log::error!("resampler: {e}"),
            }
        }
        match (rate != out_rate, cur.resampler.as_mut()) {
            (true, Some(r)) => {
                r.process(&self.mapped, &mut self.pending);
                if reached_end {
                    r.finish(&mut self.pending);
                }
            }
            _ => self.pending.extend_from_slice(&self.mapped),
        }
        if reached_end {
            cur.eof = true;
        }
    }
}

impl Player {
    fn position_from(s: &Shared) -> f64 {
        let rate = s.out_rate.load(Ordering::Relaxed).max(1) as f64;
        let played = s.played_frames.load(Ordering::Relaxed);
        let start = s.track_start_frames.load(Ordering::Relaxed);
        f64::from_bits(s.base_pos.load(Ordering::Relaxed)) + played.saturating_sub(start) as f64 / rate
    }
}

/// Convert interleaved audio between channel layouts.
fn map_channels(input: &[f32], in_ch: usize, out_ch: usize, out: &mut Vec<f32>) {
    out.clear();
    if in_ch == out_ch {
        out.extend_from_slice(input);
        return;
    }
    let frames = input.len() / in_ch;
    out.reserve(frames * out_ch);
    for f in input.chunks_exact(in_ch) {
        match (in_ch, out_ch) {
            (1, _) => out.extend(std::iter::repeat_n(f[0], out_ch)),
            (_, 1) => out.push(f.iter().sum::<f32>() / in_ch as f32),
            (_, 2) => {
                // Simple downmix: front L/R plus center and surrounds at -3 dB.
                let c = if in_ch > 2 { f[2] * 0.707 } else { 0.0 };
                let (sl, sr) = if in_ch > 5 { (f[4] * 0.707, f[5] * 0.707) } else { (0.0, 0.0) };
                out.push(((f[0] + c + sl) / 1.7).clamp(-1.0, 1.0));
                out.push(((f[1] + c + sr) / 1.7).clamp(-1.0, 1.0));
            }
            _ => {
                out.extend((0..out_ch).map(|i| f.get(i).copied().unwrap_or(0.0)));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::map_channels;

    #[test]
    fn mono_to_stereo_duplicates() {
        let mut out = Vec::new();
        map_channels(&[0.5, -0.5], 1, 2, &mut out);
        assert_eq!(out, vec![0.5, 0.5, -0.5, -0.5]);
    }

    #[test]
    fn stereo_to_mono_averages() {
        let mut out = Vec::new();
        map_channels(&[1.0, 0.0], 2, 1, &mut out);
        assert_eq!(out, vec![0.5]);
    }
}

/// Needs a real (or virtual) audio device: `cargo test -- --ignored engine_smoke`.
#[cfg(test)]
mod device_tests {
    use super::*;

    #[test]
    #[ignore]
    fn engine_smoke() {
        let path = std::env::var("JANMP_TEST_FILE").expect("set JANMP_TEST_FILE");
        let p = Player::new(0.5, || {});
        let req = MediaRequest { path: path.into(), start: 0.0, end: None };
        p.load(req.clone());
        // Virtual devices may take a moment to wake up; measure from the first played frame.
        let deadline = Instant::now() + Duration::from_secs(5);
        while p.position() == 0.0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        let t0 = p.position();
        std::thread::sleep(Duration::from_millis(1000));
        let pos = p.position() - t0;
        assert!(pos > 0.85 && pos < 1.15, "advanced {pos} s in 1 s");
        p.seek(4.0);
        std::thread::sleep(Duration::from_millis(500));
        let pos = p.position();
        assert!(pos > 4.0 && pos < 4.7, "position after seek: {pos}");
        p.pause();
        std::thread::sleep(Duration::from_millis(300));
        let a = p.position();
        std::thread::sleep(Duration::from_millis(500));
        assert!((p.position() - a).abs() < 0.05, "position must not move while paused");
        p.play();
        // Gapless: enqueue the same file again once the first is about to finish.
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut changed = false;
        while Instant::now() < deadline && !changed {
            match p.poll_event() {
                Some(PlayerEvent::AboutToFinish) => p.enqueue(req.clone()),
                Some(PlayerEvent::TrackChanged { .. }) => changed = true,
                Some(PlayerEvent::Error(e)) => panic!("{e}"),
                _ => std::thread::sleep(Duration::from_millis(10)),
            }
        }
        assert!(changed, "gapless transition did not happen");
        assert!(p.position() < 0.5);
    }
}
