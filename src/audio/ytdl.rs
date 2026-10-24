//! YouTube (MPEG-DASH audio) and similar sites through `yt-dlp`, decoded by FFmpeg.
//!
//! yt-dlp turns a page URL into a direct audio URL (usually a DASH Opus or AAC track). Those
//! URLs expire after a few hours, so they are cached briefly and resolved again when needed.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde_json::Value;

use super::decoder::{AudioDecoder, StreamInfo};
use super::ffmpeg::{self, FfmpegDecoder, Probe};

pub const YTDLP_MISSING: &str = "yt-dlp is required for YouTube: install it or put yt-dlp next to janmp";

/// How long a resolved direct URL is reused.
const CACHE_TTL: Duration = Duration::from_secs(60 * 60);

pub fn ytdlp_path() -> Option<&'static Path> {
    static PATH: OnceLock<Option<PathBuf>> = OnceLock::new();
    PATH.get_or_init(|| ffmpeg::find_tool("yt-dlp", "JANMP_YTDLP", "--version")).as_deref()
}

/// Pages handled by yt-dlp rather than played as a radio stream.
pub fn is_ytdl_url(s: &str) -> bool {
    let l = s.trim().to_ascii_lowercase();
    let Some(rest) = l.strip_prefix("https://").or_else(|| l.strip_prefix("http://")) else { return false };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
    let host = host.split(':').next().unwrap_or("");
    const HOSTS: [&str; 5] = ["youtube.com", "youtu.be", "youtube-nocookie.com", "soundcloud.com", "bandcamp.com"];
    HOSTS.iter().any(|h| host == *h || host.ends_with(&format!(".{h}")))
}

/// One playable item of a page or playlist.
#[derive(Debug, Clone, Default)]
pub struct Entry {
    pub url: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration: Option<f64>,
}

/// A direct media URL with everything needed to play it.
#[derive(Debug, Clone)]
struct Resolved {
    media_url: String,
    headers: Vec<(String, String)>,
    probe: Probe,
}

fn run(args: &[&str], url: &str) -> Result<Value, String> {
    let exe = ytdlp_path().ok_or(YTDLP_MISSING)?;
    let out = ffmpeg::command(exe)
        .args(["--no-warnings", "--no-progress", "-J"])
        .args(args)
        .arg("--")
        .arg(url)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let line = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("yt-dlp failed");
        return Err(line.trim().trim_start_matches("ERROR: ").to_string());
    }
    serde_json::from_slice(&out.stdout).map_err(|e| format!("yt-dlp: {e}"))
}

fn text(v: &Value, keys: &[&str]) -> String {
    keys.iter().find_map(|k| v.get(*k).and_then(Value::as_str).filter(|s| !s.is_empty())).unwrap_or("").to_string()
}

fn entry_from(v: &Value, fallback_url: &str, album: &str) -> Entry {
    let mut url = text(v, &["webpage_url", "url"]);
    if url.is_empty() || !url.contains("://") {
        // Flat YouTube entries may carry only the video id.
        let id = text(v, &["id"]);
        url = if id.is_empty() { fallback_url.to_string() } else { format!("https://www.youtube.com/watch?v={id}") };
    }
    let title = text(v, &["track", "title"]);
    let artist = text(v, &["artist", "creator", "channel", "uploader"]);
    let own_album = text(v, &["album"]);
    Entry {
        url,
        title,
        artist,
        album: if own_album.is_empty() { album.to_string() } else { own_album },
        duration: v.get("duration").and_then(Value::as_f64).filter(|d| *d > 0.0),
    }
}

/// Titles and durations for a page: one entry for a video, many for a playlist or channel.
pub fn list_entries(url: &str) -> Result<Vec<Entry>, String> {
    let v = run(&["--flat-playlist"], url)?;
    if let Some(entries) = v.get("entries").and_then(Value::as_array) {
        let album = text(&v, &["title"]);
        let list: Vec<Entry> = entries.iter().map(|e| entry_from(e, url, &album)).collect();
        if list.is_empty() {
            return Err("the playlist is empty".into());
        }
        return Ok(list);
    }
    let site = text(&v, &["extractor_key"]);
    Ok(vec![entry_from(&v, url, &site)])
}

fn cache() -> &'static Mutex<HashMap<String, (Instant, Resolved)>> {
    static CACHE: OnceLock<Mutex<HashMap<String, (Instant, Resolved)>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

fn resolve(url: &str) -> Result<Resolved, String> {
    if let Some((at, r)) = cache().lock().get(url) {
        if at.elapsed() < CACHE_TTL {
            return Ok(r.clone());
        }
    }
    let v = run(&["--no-playlist", "-f", "bestaudio/best"], url)?;
    // With a merged format the chosen streams are listed in `requested_formats`.
    let fmt = v
        .get("requested_formats")
        .and_then(Value::as_array)
        .and_then(|f| f.iter().find(|f| f.get("acodec").and_then(Value::as_str).is_some_and(|c| c != "none")))
        .unwrap_or(&v);
    let media_url = text(fmt, &["url"]);
    if media_url.is_empty() {
        return Err("yt-dlp returned no audio URL".into());
    }
    let headers = fmt
        .get("http_headers")
        .or_else(|| v.get("http_headers"))
        .and_then(Value::as_object)
        .map(|h| h.iter().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect())
        .unwrap_or_default();
    let codec = match text(fmt, &["acodec"]).split('.').next().unwrap_or("") {
        "" | "none" => text(fmt, &["ext"]).to_uppercase(),
        "mp4a" => "AAC".into(),
        c => c.to_uppercase(),
    };
    let probe = Probe {
        duration: v.get("duration").and_then(Value::as_f64).filter(|d| *d > 0.0),
        codec,
        sample_rate: fmt.get("asr").and_then(Value::as_u64).map(|r| r as u32),
        channels: fmt.get("audio_channels").and_then(Value::as_u64).map(|c| c as usize),
        bitrate: fmt.get("abr").or_else(|| fmt.get("tbr")).and_then(Value::as_f64).map(|b| b.round() as u32),
        tags: Vec::new(),
    };
    let r = Resolved { media_url, headers, probe };
    cache().lock().insert(url.to_string(), (Instant::now(), r.clone()));
    Ok(r)
}

/// Resolve ahead of time so the next track starts without a pause.
pub fn prefetch(url: &str) {
    let url = url.to_string();
    std::thread::spawn(move || {
        if let Err(e) = resolve(&url) {
            log::debug!("prefetch {url}: {e}");
        }
    });
}

pub struct YtdlDecoder {
    inner: FfmpegDecoder,
}

impl YtdlDecoder {
    pub fn open(url: &str) -> Result<Self, String> {
        if ffmpeg::ffmpeg_path().is_none() {
            return Err(super::decoder::FFMPEG_MISSING.into());
        }
        let r = resolve(url)?;
        let mut args: Vec<String> =
            ["-reconnect", "1", "-reconnect_streamed", "1", "-reconnect_delay_max", "5"].map(String::from).to_vec();
        let mut extra = String::new();
        for (k, v) in &r.headers {
            if k.eq_ignore_ascii_case("user-agent") {
                args.extend(["-user_agent".into(), v.clone()]);
            } else {
                extra.push_str(&format!("{k}: {v}\r\n"));
            }
        }
        if !extra.is_empty() {
            args.extend(["-headers".into(), extra]);
        }
        let media = Path::new(&r.media_url);
        let mut probe = r.probe;
        if probe.sample_rate.is_none() || probe.channels.is_none() {
            // Some sites do not describe their formats; ask ffmpeg instead.
            let p = ffmpeg::probe_with(media, &args)?;
            probe.sample_rate = probe.sample_rate.or(p.sample_rate);
            probe.channels = probe.channels.or(p.channels);
            probe.duration = probe.duration.or(p.duration);
            probe.bitrate = probe.bitrate.or(p.bitrate);
            if p.codec.len() < 8 {
                probe.codec = p.codec.to_uppercase();
            }
        }
        let inner = FfmpegDecoder::with_probe(media, probe, args).inspect_err(|_| {
            // The URL may have expired; resolve again next time.
            cache().lock().remove(url);
        })?;
        Ok(Self { inner })
    }
}

impl AudioDecoder for YtdlDecoder {
    fn spec(&self) -> (u32, usize) {
        self.inner.spec()
    }
    fn duration(&self) -> Option<f64> {
        self.inner.duration()
    }
    fn info(&self) -> StreamInfo {
        self.inner.info()
    }
    fn next_chunk(&mut self, out: &mut Vec<f32>) -> Result<bool, String> {
        self.inner.next_chunk(out)
    }
    fn seek(&mut self, secs: f64) -> Result<(), String> {
        self.inner.seek(secs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_sites() {
        assert!(is_ytdl_url("https://www.youtube.com/watch?v=dQw4w9WgXcQ"));
        assert!(is_ytdl_url("https://youtu.be/dQw4w9WgXcQ"));
        assert!(is_ytdl_url("https://music.youtube.com/playlist?list=PL123"));
        assert!(is_ytdl_url("https://artist.bandcamp.com/album/x"));
        assert!(!is_ytdl_url("http://radio.example.com:8000/stream"));
        assert!(!is_ytdl_url("https://notyoutube.com/x"));
        assert!(!is_ytdl_url("/music/youtube.com.mp3"));
    }

    #[test]
    fn parses_flat_entries() {
        let v: Value = serde_json::from_str(
            r#"{"_type":"url","id":"abc","title":"Song","channel":"Band","duration":215.0}"#,
        )
        .unwrap();
        let e = entry_from(&v, "https://youtu.be/x", "Mix");
        assert_eq!(e.url, "https://www.youtube.com/watch?v=abc");
        assert_eq!((e.title.as_str(), e.artist.as_str(), e.album.as_str()), ("Song", "Band", "Mix"));
        assert_eq!(e.duration, Some(215.0));
    }

    /// Needs yt-dlp, ffmpeg and network access; `JANMP_TEST_YT` is a page or direct media URL.
    #[test]
    #[ignore]
    fn ytdl_smoke() {
        let url = std::env::var("JANMP_TEST_YT").expect("set JANMP_TEST_YT");
        let entries = list_entries(&url).unwrap();
        eprintln!("{entries:?}");
        let mut d = YtdlDecoder::open(&entries[0].url).unwrap();
        eprintln!("{:?} {:?}", d.info(), d.duration());
        let mut out = Vec::new();
        while out.len() < 200_000 && d.next_chunk(&mut out).unwrap() {}
        assert!(out.iter().any(|s| s.abs() > 0.01));
        if let Some(len) = d.duration() {
            d.seek(len / 2.0).unwrap();
            let mut more = Vec::new();
            assert!(d.next_chunk(&mut more).unwrap());
        }
    }
}
