//! Internet radio as an [`AudioDecoder`].
//!
//! A network thread reads the HTTP stream, strips ICY metadata and forwards the raw bytes to
//! the recorder and to a decoder: ffmpeg when available (handles HE-AAC / AAC+), otherwise
//! Symphonia over a pipe. Decoded PCM reaches the engine through a bounded channel that also
//! serves as the jitter buffer.

use std::io::{Read, Write};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::{bounded, Receiver, RecvTimeoutError, Sender};
use parking_lot::Mutex;

use super::decoder::{AudioDecoder, StreamInfo, SymphoniaDecoder};
use super::ffmpeg;
use crate::radio::http::{self, IcyReader, StreamHeaders};
use crate::radio::record::{RecordSettings, Recorder};

/// Output format when ffmpeg decodes the stream.
const FF_RATE: u32 = 48_000;
const FF_CHANNELS: usize = 2;
/// Audio buffered before playback starts (and after an underrun).
const PREBUFFER_SECS: f64 = 1.5;
/// Upper bound of buffered audio; older data is dropped while paused.
const MAX_BUFFER_SECS: f64 = 30.0;
const RECONNECT_ATTEMPTS: u32 = 5;

struct Shared {
    stop: AtomicBool,
    title: Mutex<Option<String>>,
    /// Titles waiting for the audio they belong to: (frame index, title).
    pending_titles: Mutex<std::collections::VecDeque<(u64, String)>>,
    /// Frames decoded so far.
    decoded_frames: AtomicU64,
    recorder: Mutex<Option<Recorder>>,
    /// Frames sitting in the PCM channel.
    queued_frames: AtomicUsize,
    error: Mutex<Option<String>>,
}

pub struct StreamDecoder {
    shared: Arc<Shared>,
    pcm: Receiver<Vec<f32>>,
    headers: StreamHeaders,
    url: String,
    rate: u32,
    channels: usize,
    buffering: bool,
    finished: bool,
    /// Frames handed to the engine (or dropped) so far.
    delivered_frames: u64,
}

pub fn is_stream_url(s: &str) -> bool {
    let l = s.trim_start().to_ascii_lowercase();
    l.starts_with("http://") || l.starts_with("https://")
}

/// Raw bytes → decoder input.
enum Feed {
    Ffmpeg(std::process::ChildStdin),
    Pipe(Sender<Vec<u8>>),
}

impl StreamDecoder {
    pub fn open(url: &str) -> Result<Self, String> {
        let (headers, body) = http::connect(url)?;
        let shared = Arc::new(Shared {
            stop: AtomicBool::new(false),
            title: Mutex::new(None),
            pending_titles: Mutex::new(Default::default()),
            decoded_frames: AtomicU64::new(0),
            recorder: Mutex::new(None),
            queued_frames: AtomicUsize::new(0),
            error: Mutex::new(None),
        });

        let (pcm_tx, pcm) = bounded::<Vec<f32>>(4096);
        let (feed, rate, channels) = match ffmpeg::ffmpeg_path() {
            Some(ff) => {
                let mut child = ffmpeg::command(ff)
                    .args(["-hide_banner", "-nostdin", "-loglevel", "error", "-probesize", "65536", "-i", "pipe:0"])
                    .args(["-map", "0:a:0", "-vn", "-f", "f32le", "-acodec", "pcm_f32le"])
                    .arg("-ar")
                    .arg(FF_RATE.to_string())
                    .arg("-ac")
                    .arg(FF_CHANNELS.to_string())
                    .arg("-")
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .spawn()
                    .map_err(|e| e.to_string())?;
                let stdin = child.stdin.take().ok_or("no stdin")?;
                let stdout = child.stdout.take().ok_or("no stdout")?;
                let s = shared.clone();
                std::thread::Builder::new()
                    .name("janmp-radio-pcm".into())
                    .spawn(move || pcm_from_ffmpeg(stdout, child, pcm_tx, s))
                    .map_err(|e| e.to_string())?;
                (Feed::Ffmpeg(stdin), FF_RATE, FF_CHANNELS)
            }
            None => {
                let (byte_tx, byte_rx) = bounded::<Vec<u8>>(256);
                let (spec_tx, spec_rx) = bounded(1);
                let s = shared.clone();
                let hint = headers.raw_extension();
                std::thread::Builder::new()
                    .name("janmp-radio-pcm".into())
                    .spawn(move || pcm_from_symphonia(byte_rx, hint, pcm_tx, spec_tx, s))
                    .map_err(|e| e.to_string())?;
                // Bytes must flow before Symphonia can probe, so start the network first.
                let feed = Feed::Pipe(byte_tx);
                let s = shared.clone();
                let metaint = headers.metaint;
                std::thread::Builder::new()
                    .name("janmp-radio-net".into())
                    .spawn({
                        let url = url.to_string();
                        move || network(url, body, metaint, feed, s)
                    })
                    .map_err(|e| e.to_string())?;
                let (rate, channels) = spec_rx
                    .recv_timeout(Duration::from_secs(15))
                    .map_err(|_| "could not detect the stream format".to_string())??;
                return Ok(Self {
                    shared,
                    pcm,
                    headers,
                    url: url.to_string(),
                    rate,
                    channels,
                    buffering: true,
                    finished: false,
                    delivered_frames: 0,
                });
            }
        };
        let s = shared.clone();
        let metaint = headers.metaint;
        let url_owned = url.to_string();
        std::thread::Builder::new()
            .name("janmp-radio-net".into())
            .spawn(move || network(url_owned, body, metaint, feed, s))
            .map_err(|e| e.to_string())?;
        Ok(Self {
            shared,
            pcm,
            headers,
            url: url.to_string(),
            rate,
            channels,
            buffering: true,
            finished: false,
            delivered_frames: 0,
        })
    }

}

impl Drop for StreamDecoder {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        self.shared.recorder.lock().take();
    }
}

fn network(url: String, body: http::BoxRead, metaint: Option<usize>, mut feed: Feed, shared: Arc<Shared>) {
    let mut reader = IcyReader::new(body, metaint);
    let mut buf = vec![0u8; 16 * 1024];
    let mut attempts = 0;
    while !shared.stop.load(Ordering::Relaxed) {
        let n = match reader.read(&mut buf) {
            Ok(0) | Err(_) => {
                // Radio connections drop now and then: reconnect a few times.
                attempts += 1;
                if attempts > RECONNECT_ATTEMPTS || shared.stop.load(Ordering::Relaxed) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(500 * attempts as u64));
                match http::connect(&url) {
                    Ok((h, body)) => {
                        let title = reader.title.take();
                        reader = IcyReader::new(body, h.metaint);
                        reader.title = title;
                    }
                    Err(e) => log::warn!("radio reconnect: {e}"),
                }
                continue;
            }
            Ok(n) => n,
        };
        attempts = 0;
        if reader.title_changed {
            reader.title_changed = false;
            let t = reader.title.clone().unwrap_or_default();
            if let Some(rec) = shared.recorder.lock().as_mut() {
                rec.on_title(&t);
            }
            let at = shared.decoded_frames.load(Ordering::Relaxed);
            shared.pending_titles.lock().push_back((at, t.clone()));
            *shared.title.lock() = Some(t);
        }
        if let Some(rec) = shared.recorder.lock().as_mut() {
            if rec.is_raw() {
                rec.write_raw(&buf[..n]);
            }
        }
        let ok = match &mut feed {
            Feed::Ffmpeg(stdin) => stdin.write_all(&buf[..n]).is_ok(),
            Feed::Pipe(tx) => tx.send(buf[..n].to_vec()).is_ok(),
        };
        if !ok {
            break;
        }
    }
    if !shared.stop.load(Ordering::Relaxed) {
        shared.error.lock().get_or_insert_with(|| "radio stream ended".into());
    }
}

fn push_pcm(samples: Vec<f32>, channels: usize, tx: &Sender<Vec<f32>>, shared: &Shared) -> bool {
    if let Some(rec) = shared.recorder.lock().as_mut() {
        if !rec.is_raw() {
            rec.write_pcm(&samples);
        }
    }
    let frames = samples.len() / channels;
    shared.queued_frames.fetch_add(frames, Ordering::Relaxed);
    shared.decoded_frames.fetch_add(frames as u64, Ordering::Relaxed);
    tx.send(samples).is_ok()
}

fn pcm_from_ffmpeg(mut stdout: std::process::ChildStdout, mut child: std::process::Child, tx: Sender<Vec<f32>>, shared: Arc<Shared>) {
    let frame = 4 * FF_CHANNELS;
    let mut bytes = Vec::new();
    let mut buf = vec![0u8; 8192 * frame / 4];
    while !shared.stop.load(Ordering::Relaxed) {
        let n = match stdout.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        bytes.extend_from_slice(&buf[..n]);
        let usable = bytes.len() - bytes.len() % frame;
        if usable == 0 {
            continue;
        }
        let samples: Vec<f32> =
            bytes[..usable].chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
        bytes.drain(..usable);
        if !push_pcm(samples, FF_CHANNELS, &tx, &shared) {
            break;
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// `Read` over chunks arriving through a channel.
struct ChannelReader {
    rx: Receiver<Vec<u8>>,
    cur: Vec<u8>,
    pos: usize,
}

impl Read for ChannelReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        while self.pos >= self.cur.len() {
            match self.rx.recv() {
                Ok(c) => {
                    self.cur = c;
                    self.pos = 0;
                }
                Err(_) => return Ok(0),
            }
        }
        let n = buf.len().min(self.cur.len() - self.pos);
        buf[..n].copy_from_slice(&self.cur[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

fn pcm_from_symphonia(
    rx: Receiver<Vec<u8>>,
    ext: &'static str,
    tx: Sender<Vec<f32>>,
    spec_tx: Sender<Result<(u32, usize), String>>,
    shared: Arc<Shared>,
) {
    use symphonia::core::formats::probe::Hint;
    use symphonia::core::io::{MediaSourceStream, ReadOnlySource};
    let reader = ChannelReader { rx, cur: Vec::new(), pos: 0 };
    let mss = MediaSourceStream::new(Box::new(ReadOnlySource::new(reader)), Default::default());
    let mut hint = Hint::new();
    hint.with_extension(ext);
    let opened = symphonia::default::get_probe()
        .probe(&hint, mss, Default::default(), Default::default())
        .map_err(|e| e.to_string())
        .and_then(SymphoniaDecoder::from_format);
    let mut dec = match opened {
        Ok(d) => d,
        Err(e) => {
            let _ = spec_tx.send(Err(e));
            return;
        }
    };
    let (_, channels) = dec.spec();
    let _ = spec_tx.send(Ok(dec.spec()));
    while !shared.stop.load(Ordering::Relaxed) {
        let mut out = Vec::new();
        match dec.next_chunk(&mut out) {
            Ok(true) => {
                if !out.is_empty() && !push_pcm(out, channels, &tx, &shared) {
                    break;
                }
            }
            _ => break,
        }
    }
}

impl AudioDecoder for StreamDecoder {
    fn spec(&self) -> (u32, usize) {
        (self.rate, self.channels)
    }

    fn duration(&self) -> Option<f64> {
        None
    }

    fn info(&self) -> StreamInfo {
        StreamInfo {
            codec: self.headers.codec().to_string(),
            sample_rate: self.rate,
            channels: self.channels,
            bits_per_sample: None,
            bitrate: self.headers.bitrate,
            station: self.headers.name.clone(),
        }
    }

    fn next_chunk(&mut self, out: &mut Vec<f32>) -> Result<bool, String> {
        if self.finished {
            return Ok(false);
        }
        let rate = self.rate as f64;
        // Never let the jitter buffer grow without bound (e.g. while paused).
        while self.shared.queued_frames.load(Ordering::Relaxed) as f64 > MAX_BUFFER_SECS * rate {
            match self.pcm.try_recv() {
                Ok(c) => {
                    self.shared.queued_frames.fetch_sub(c.len() / self.channels, Ordering::Relaxed);
                    self.delivered_frames += (c.len() / self.channels) as u64;
                }
                Err(_) => break,
            }
        }
        if self.buffering {
            let queued = self.shared.queued_frames.load(Ordering::Relaxed) as f64 / rate;
            let ended = self.shared.error.lock().is_some();
            if queued < PREBUFFER_SECS && !ended {
                // Return quickly so the engine stays responsive to commands.
                std::thread::sleep(Duration::from_millis(20));
                return Ok(true);
            }
            self.buffering = false;
        }
        match self.pcm.recv_timeout(Duration::from_millis(50)) {
            Ok(c) => {
                self.shared.queued_frames.fetch_sub(c.len() / self.channels, Ordering::Relaxed);
                self.delivered_frames += (c.len() / self.channels) as u64;
                out.extend_from_slice(&c);
                Ok(true)
            }
            Err(RecvTimeoutError::Timeout) => {
                // Underrun: wait for the buffer to refill instead of stuttering.
                self.buffering = true;
                Ok(true)
            }
            Err(RecvTimeoutError::Disconnected) => {
                self.finished = true;
                match self.shared.error.lock().clone() {
                    Some(e) => Err(e),
                    None => Ok(false),
                }
            }
        }
    }

    fn seek(&mut self, _secs: f64) -> Result<(), String> {
        Err("radio streams cannot be seeked".into())
    }

    /// Titles are released when the audio they belong to reaches the engine.
    fn take_title(&mut self) -> Option<String> {
        let mut q = self.shared.pending_titles.lock();
        let mut latest = None;
        while q.front().is_some_and(|(at, _)| *at <= self.delivered_frames) {
            latest = q.pop_front().map(|(_, t)| t);
        }
        latest
    }

    fn set_recording(&mut self, settings: Option<RecordSettings>) -> Result<Vec<std::path::PathBuf>, String> {
        let mut slot = self.shared.recorder.lock();
        let finished = slot.take().map(|r| r.files.clone()).unwrap_or_default();
        if let Some(settings) = settings {
            let station = self.headers.name.clone().unwrap_or_else(|| self.url.clone());
            let title = self.shared.title.lock().clone();
            *slot = Some(Recorder::new(settings, &station, self.headers.raw_extension(), self.rate, self.channels, title)?);
        }
        Ok(finished)
    }
}
