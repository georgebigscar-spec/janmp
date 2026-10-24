//! Decoding through an external `ffmpeg` process.
//!
//! Covers everything Symphonia cannot: APE, WMA, AC3, DTS, Musepack, TAK, TTA, WavPack,
//! OptimFROG, Speex, Opus, DSD, ATRAC and (with libopenmpt, present in common builds) tracker
//! modules. ffmpeg decodes to raw `f32le` on stdout; seeking restarts it with `-ss`.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::OnceLock;

use super::decoder::{AudioDecoder, StreamInfo};

/// Frames read from the pipe per `next_chunk` call.
const CHUNK_FRAMES: usize = 4096;

fn command(program: &Path) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// Locate ffmpeg: `$JANMP_FFMPEG`, next to the janmp executable, then `PATH`.
pub fn ffmpeg_path() -> Option<&'static Path> {
    static PATH: OnceLock<Option<PathBuf>> = OnceLock::new();
    PATH.get_or_init(|| {
        let exe = if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" };
        let mut candidates = Vec::new();
        if let Some(p) = std::env::var_os("JANMP_FFMPEG") {
            candidates.push(PathBuf::from(p));
        }
        if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
            candidates.push(dir.join(exe));
            candidates.push(dir.join("ffmpeg").join(exe));
        }
        candidates.push(PathBuf::from(exe));
        candidates.into_iter().find(|c| {
            command(c)
                .arg("-version")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
        })
    })
    .as_deref()
}

/// What `ffmpeg -i` reports about a file.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Probe {
    pub duration: Option<f64>,
    pub codec: String,
    pub sample_rate: Option<u32>,
    pub channels: Option<usize>,
    pub bitrate: Option<u32>,
    pub tags: Vec<(String, String)>,
}

impl Probe {
    pub fn tag(&self, key: &str) -> Option<&str> {
        self.tags.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v.as_str())
    }
}

pub fn probe(path: &Path) -> Result<Probe, String> {
    let ffmpeg = ffmpeg_path().ok_or("FFmpeg not found")?;
    let out = command(ffmpeg)
        .args(["-hide_banner", "-nostdin", "-i"])
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&out.stderr);
    let p = parse_probe(&text);
    if p.codec.is_empty() {
        let last = text.lines().rev().find(|l| !l.contains("output file")).unwrap_or("no audio stream");
        return Err(last.trim().to_string());
    }
    Ok(p)
}

fn parse_hms(s: &str) -> Option<f64> {
    let mut parts = s.trim().split(':');
    let h: f64 = parts.next()?.parse().ok()?;
    let m: f64 = parts.next()?.parse().ok()?;
    let sec: f64 = parts.next()?.parse().ok()?;
    Some(h * 3600.0 + m * 60.0 + sec)
}

fn layout_channels(layout: &str) -> Option<usize> {
    let l = layout.trim();
    if let Some(n) = l.strip_suffix(" channels").and_then(|n| n.trim().parse().ok()) {
        return Some(n);
    }
    let base = l.split('(').next().unwrap_or(l);
    Some(match base {
        "mono" => 1,
        "stereo" | "downmix" => 2,
        "2.1" | "3.0" => 3,
        "quad" | "4.0" | "3.1" => 4,
        "5.0" | "4.1" => 5,
        "5.1" | "6.0" => 6,
        "6.1" | "7.0" => 7,
        "7.1" => 8,
        _ => return None,
    })
}

pub fn parse_probe(text: &str) -> Probe {
    let mut p = Probe::default();
    // Container tags sit under "  Metadata:"; Ogg/Opus keep them on the audio stream instead.
    let mut stream_tags = Vec::new();
    let (mut in_global_meta, mut in_stream_meta) = (false, false);
    for line in text.lines() {
        let t = line.trim();
        if line.starts_with("  Metadata:") {
            in_global_meta = p.codec.is_empty() && p.duration.is_none();
            continue;
        }
        if line.starts_with("    Metadata:") {
            in_stream_meta = !p.codec.is_empty() && stream_tags.is_empty();
            continue;
        }
        let target = if in_global_meta && line.starts_with("    ") {
            Some(&mut p.tags)
        } else if in_stream_meta && line.starts_with("      ") {
            Some(&mut stream_tags)
        } else {
            None
        };
        if let Some(target) = target {
            if let Some((k, v)) = t.split_once(':') {
                target.push((k.trim().to_string(), v.trim().to_string()));
            }
            continue;
        }
        in_global_meta = false;
        in_stream_meta = false;
        if let Some(rest) = t.strip_prefix("Duration:") {
            let mut fields = rest.split(',');
            p.duration = fields.next().and_then(parse_hms).filter(|d| *d > 0.0);
            for f in fields {
                if let Some(b) = f.trim().strip_prefix("bitrate:") {
                    p.bitrate = b.trim().split(' ').next().and_then(|n| n.parse().ok());
                }
            }
        } else if t.starts_with("Stream #") && p.codec.is_empty() {
            let Some((_, audio)) = t.split_once("Audio: ") else { continue };
            let parts: Vec<&str> = audio.split(", ").collect();
            p.codec = parts[0].split([' ', '(']).next().unwrap_or("").to_uppercase();
            for (i, part) in parts.iter().enumerate() {
                if let Some(hz) = part.strip_suffix(" Hz") {
                    p.sample_rate = hz.trim().parse().ok();
                    p.channels = parts.get(i + 1).and_then(|l| layout_channels(l));
                }
                if let Some(kb) = part.split(' ').next().filter(|_| part.contains(" kb/s")) {
                    p.bitrate = kb.parse().ok().or(p.bitrate);
                }
            }
        }
    }
    if p.tags.iter().all(|(k, _)| k.eq_ignore_ascii_case("encoder")) {
        p.tags.extend(stream_tags);
    }
    p
}

pub struct FfmpegDecoder {
    path: PathBuf,
    probe: Probe,
    rate: u32,
    channels: usize,
    child: Option<(Child, ChildStdout)>,
    bytes: Vec<u8>,
    /// Seek by decoding and discarding (`-ss` after `-i`). Needed for demuxers that cannot
    /// seek, such as libopenmpt.
    output_seek: bool,
    /// Start of the running process and whether it has produced any audio yet.
    start: f64,
    produced: bool,
}

impl FfmpegDecoder {
    pub fn open(path: &Path) -> Result<Self, String> {
        let mut probe = probe(path)?;
        // libopenmpt reports its rendered PCM as the codec; show the module type instead.
        let tracker = super::formats::backend_for(path) == Some(super::formats::Backend::Tracker);
        if tracker {
            if let Some(ext) = path.extension() {
                probe.codec = ext.to_string_lossy().to_uppercase();
            }
        }
        // DSD and exotic rates are converted to something every device handles.
        let rate = match probe.sample_rate {
            Some(r) if (8_000..=192_000).contains(&r) => r,
            _ => 88_200,
        };
        let channels = probe.channels.unwrap_or(2).clamp(1, 8);
        let mut d = Self {
            path: path.to_path_buf(),
            probe,
            rate,
            channels,
            child: None,
            bytes: Vec::new(),
            output_seek: tracker,
            start: 0.0,
            produced: false,
        };
        d.spawn(0.0)?;
        Ok(d)
    }

    fn spawn(&mut self, start: f64) -> Result<(), String> {
        self.kill();
        let ffmpeg = ffmpeg_path().ok_or("FFmpeg not found")?;
        let mut cmd = command(ffmpeg);
        cmd.args(["-hide_banner", "-nostdin", "-loglevel", "error"]);
        let ss = format!("{start:.6}");
        if start > 0.0 && !self.output_seek {
            cmd.arg("-ss").arg(&ss);
        }
        cmd.arg("-i").arg(&self.path);
        if start > 0.0 && self.output_seek {
            cmd.arg("-ss").arg(&ss);
        }
        cmd.args(["-map", "0:a:0", "-vn", "-sn", "-f", "f32le", "-acodec", "pcm_f32le"]);
        // Only convert when needed: forcing a layout on streams with an unnamed one
        // ("1 channels") makes ffmpeg's filter graph fail.
        if self.probe.channels != Some(self.channels) {
            cmd.arg("-ac").arg(self.channels.to_string());
        }
        if self.probe.sample_rate != Some(self.rate) {
            cmd.arg("-ar").arg(self.rate.to_string());
        }
        cmd.arg("-")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = cmd.spawn().map_err(|e| e.to_string())?;
        let stdout = child.stdout.take().ok_or("no stdout")?;
        self.child = Some((child, stdout));
        self.bytes.clear();
        self.start = start;
        self.produced = false;
        Ok(())
    }

    fn kill(&mut self) {
        if let Some((mut child, _)) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for FfmpegDecoder {
    fn drop(&mut self) {
        self.kill();
    }
}

impl AudioDecoder for FfmpegDecoder {
    fn spec(&self) -> (u32, usize) {
        (self.rate, self.channels)
    }

    fn duration(&self) -> Option<f64> {
        self.probe.duration
    }

    fn info(&self) -> StreamInfo {
        StreamInfo {
            codec: self.probe.codec.clone(),
            sample_rate: self.probe.sample_rate.unwrap_or(self.rate),
            channels: self.probe.channels.unwrap_or(self.channels),
            bits_per_sample: None,
        }
    }

    fn next_chunk(&mut self, out: &mut Vec<f32>) -> Result<bool, String> {
        let Some((_, stdout)) = self.child.as_mut() else { return Ok(false) };
        let frame_bytes = 4 * self.channels;
        let want = CHUNK_FRAMES * frame_bytes;
        let mut buf = [0u8; 16384];
        while self.bytes.len() < want {
            let room = (want - self.bytes.len()).min(buf.len());
            let n = stdout.read(&mut buf[..room]).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            self.bytes.extend_from_slice(&buf[..n]);
        }
        let usable = self.bytes.len() - self.bytes.len() % frame_bytes;
        if usable == 0 {
            self.kill();
            // A seek that yields nothing before the end means the demuxer cannot seek:
            // retry by decoding from the start and discarding.
            let before_end = self.probe.duration.is_none_or(|d| self.start < d - 0.5);
            if !self.produced && self.start > 0.0 && !self.output_seek && before_end {
                self.output_seek = true;
                self.spawn(self.start)?;
                return self.next_chunk(out);
            }
            return Ok(false);
        }
        self.produced = true;
        out.extend(self.bytes[..usable].chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])));
        self.bytes.drain(..usable);
        Ok(true)
    }

    fn seek(&mut self, secs: f64) -> Result<(), String> {
        self.spawn(secs.max(0.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "Input #0, ape, from 'x.ape':
  Metadata:
    Title           : Кукушка
    Artist          : Кино
    Album           : Чёрный альбом
    Year            : 1990
    Track           : 3/8
  Duration: 00:06:36.72, start: 0.000000, bitrate: 836 kb/s
  Stream #0:0: Audio: ape (APE  / 0x20455041), 44100 Hz, stereo, s16p
At least one output file must be specified
";

    #[test]
    fn parses_ffmpeg_banner() {
        let p = parse_probe(SAMPLE);
        assert_eq!(p.codec, "APE");
        assert_eq!(p.sample_rate, Some(44_100));
        assert_eq!(p.channels, Some(2));
        assert_eq!(p.bitrate, Some(836));
        assert!((p.duration.unwrap() - 396.72).abs() < 1e-9);
        assert_eq!(p.tag("artist"), Some("Кино"));
        assert_eq!(p.tag("TITLE"), Some("Кукушка"));
        assert_eq!(p.tag("track"), Some("3/8"));
    }

    /// End-to-end through a real ffmpeg; skipped when it is not installed.
    #[test]
    fn decodes_and_seeks_via_ffmpeg() {
        let Some(ff) = ffmpeg_path() else { return };
        let dir = std::env::temp_dir().join(format!("janmp-ff-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for (ext, codec) in [("wv", "wavpack"), ("wma", "wmav2"), ("tta", "tta")] {
            let path = dir.join(format!("t.{ext}"));
            let ok = command(ff)
                .args(["-loglevel", "error", "-y", "-f", "lavfi", "-i", "sine=frequency=440:duration=2", "-ac", "2", "-c:a", codec])
                .arg(&path)
                .status()
                .is_ok_and(|s| s.success());
            if !ok {
                continue;
            }
            let mut d = FfmpegDecoder::open(&path).unwrap();
            let (rate, ch) = d.spec();
            assert_eq!(ch, 2);
            let mut all = Vec::new();
            while d.next_chunk(&mut all).unwrap() {}
            let secs = all.len() as f64 / ch as f64 / rate as f64;
            assert!((secs - 2.0).abs() < 0.1, "{ext}: decoded {secs} s");
            d.seek(1.5).unwrap();
            let mut rest = Vec::new();
            while d.next_chunk(&mut rest).unwrap() {}
            let secs = rest.len() as f64 / ch as f64 / rate as f64;
            assert!((secs - 0.5).abs() < 0.1, "{ext}: after seek {secs} s");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn layouts() {
        assert_eq!(layout_channels("5.1(side)"), Some(6));
        assert_eq!(layout_channels("mono"), Some(1));
        assert_eq!(layout_channels("3 channels"), Some(3));
    }
}
