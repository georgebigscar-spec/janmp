//! Opening internet radio streams and stripping SHOUTcast/Icecast (ICY) metadata.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use crate::playlist::cue::decode_text;

/// Response headers that matter for a radio stream.
#[derive(Debug, Clone, Default)]
pub struct StreamHeaders {
    pub content_type: String,
    pub metaint: Option<usize>,
    pub name: Option<String>,
    pub genre: Option<String>,
    pub bitrate: Option<u32>,
}

impl StreamHeaders {
    /// Short codec name from the MIME type.
    pub fn codec(&self) -> &'static str {
        let ct = self.content_type.to_ascii_lowercase();
        if ct.contains("aacp") || ct.contains("aac+") {
            "AAC+"
        } else if ct.contains("aac") || ct.contains("mp4") {
            "AAC"
        } else if ct.contains("ogg") || ct.contains("opus") || ct.contains("vorbis") {
            "OGG"
        } else if ct.contains("flac") {
            "FLAC"
        } else if ct.contains("wav") || ct.contains("x-wav") {
            "WAV"
        } else {
            "MP3"
        }
    }

    /// File extension for a byte-exact copy of the stream.
    pub fn raw_extension(&self) -> &'static str {
        match self.codec() {
            "AAC+" | "AAC" => "aac",
            "OGG" => "ogg",
            "FLAC" => "flac",
            "WAV" => "wav",
            _ => "mp3",
        }
    }
}

pub type BoxRead = Box<dyn Read + Send>;

const USER_AGENT: &str = concat!("janmp/", env!("CARGO_PKG_VERSION"));

/// Connect and return headers plus the body reader (still containing ICY metadata blocks).
pub fn connect(url: &str) -> Result<(StreamHeaders, BoxRead), String> {
    if url.to_ascii_lowercase().starts_with("http://") {
        // Old SHOUTcast servers answer "ICY 200 OK", which HTTP libraries reject.
        match connect_raw(url, 0) {
            Ok(r) => return Ok(r),
            Err(e) => log::debug!("raw ICY connect failed ({e}), retrying with ureq"),
        }
    }
    connect_ureq(url)
}

fn connect_ureq(url: &str) -> Result<(StreamHeaders, BoxRead), String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_recv_response(Some(Duration::from_secs(15)))
        .build()
        .into();
    let resp = agent
        .get(url)
        .header("Icy-MetaData", "1")
        .header("User-Agent", USER_AGENT)
        .call()
        .map_err(|e| e.to_string())?;
    let h = |k: &str| resp.headers().get(k).and_then(|v| v.to_str().ok()).map(|s| s.trim().to_string());
    let headers = StreamHeaders {
        content_type: h("content-type").unwrap_or_default(),
        metaint: h("icy-metaint").and_then(|v| v.parse().ok()),
        name: h("icy-name").filter(|s| !s.is_empty()),
        genre: h("icy-genre").filter(|s| !s.is_empty()),
        bitrate: h("icy-br").and_then(|v| v.split(',').next()?.trim().parse().ok()),
    };
    Ok((headers, Box::new(resp.into_body().into_reader())))
}

fn connect_raw(url: &str, redirects: u32) -> Result<(StreamHeaders, BoxRead), String> {
    let rest = &url[7..];
    let (hostport, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let host = hostport.rsplit_once('@').map(|(_, h)| h).unwrap_or(hostport);
    let has_port = host.rsplit_once(':').is_some_and(|(_, p)| p.parse::<u16>().is_ok());
    let addr = if has_port { host.to_string() } else { format!("{host}:80") };
    let addrs: Vec<_> = std::net::ToSocketAddrs::to_socket_addrs(&addr).map_err(|e| e.to_string())?.collect();
    let sock = addrs
        .iter()
        .find_map(|a| TcpStream::connect_timeout(a, Duration::from_secs(10)).ok())
        .ok_or("connection failed")?;
    sock.set_read_timeout(Some(Duration::from_secs(20))).ok();
    let mut w = sock.try_clone().map_err(|e| e.to_string())?;
    write!(
        w,
        "GET {path} HTTP/1.0\r\nHost: {host}\r\nUser-Agent: {USER_AGENT}\r\nIcy-MetaData: 1\r\nAccept: */*\r\nConnection: close\r\n\r\n"
    )
    .map_err(|e| e.to_string())?;

    let mut reader = BufReader::new(sock);
    let mut status = String::new();
    reader.read_line(&mut status).map_err(|e| e.to_string())?;
    let code: u16 = status.split_whitespace().nth(1).and_then(|c| c.parse().ok()).ok_or("bad status line")?;
    let mut headers = StreamHeaders::default();
    let mut location = None;
    loop {
        let mut line = Vec::new();
        reader.read_until(b'\n', &mut line).map_err(|e| e.to_string())?;
        let line = decode_text(&line);
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        let Some((k, v)) = line.split_once(':') else { continue };
        let v = v.trim().to_string();
        match k.trim().to_ascii_lowercase().as_str() {
            "content-type" => headers.content_type = v,
            "icy-metaint" => headers.metaint = v.parse().ok(),
            "icy-name" if !v.is_empty() => headers.name = Some(v),
            "icy-genre" if !v.is_empty() => headers.genre = Some(v),
            "icy-br" => headers.bitrate = v.split(',').next().and_then(|b| b.trim().parse().ok()),
            "location" => location = Some(v),
            _ => {}
        }
    }
    if (300..400).contains(&code) {
        let loc = location.ok_or("redirect without location")?;
        if redirects > 5 {
            return Err("too many redirects".into());
        }
        return if loc.to_ascii_lowercase().starts_with("http://") {
            connect_raw(&loc, redirects + 1)
        } else {
            connect_ureq(&loc)
        };
    }
    if code != 200 {
        return Err(format!("HTTP {code}"));
    }
    Ok((headers, Box::new(reader)))
}

/// Reader that removes ICY metadata blocks and reports `StreamTitle` changes.
pub struct IcyReader<R> {
    inner: R,
    metaint: Option<usize>,
    until_meta: usize,
    pub title: Option<String>,
    pub title_changed: bool,
}

impl<R: Read> IcyReader<R> {
    pub fn new(inner: R, metaint: Option<usize>) -> Self {
        Self { inner, metaint, until_meta: metaint.unwrap_or(0), title: None, title_changed: false }
    }

    fn read_meta(&mut self) -> std::io::Result<()> {
        let mut len = [0u8; 1];
        self.inner.read_exact(&mut len)?;
        let n = len[0] as usize * 16;
        if n == 0 {
            return Ok(());
        }
        let mut buf = vec![0u8; n];
        self.inner.read_exact(&mut buf)?;
        let text = decode_text(&buf);
        if let Some(t) = parse_stream_title(&text) {
            if self.title.as_deref() != Some(t.as_str()) {
                self.title = Some(t);
                self.title_changed = true;
            }
        }
        Ok(())
    }
}

impl<R: Read> Read for IcyReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let Some(metaint) = self.metaint else { return self.inner.read(buf) };
        if self.until_meta == 0 {
            self.read_meta()?;
            self.until_meta = metaint;
        }
        let n = buf.len().min(self.until_meta);
        let got = self.inner.read(&mut buf[..n])?;
        self.until_meta -= got;
        Ok(got)
    }
}

/// Extract the value of `StreamTitle='…';` from an ICY metadata block.
pub fn parse_stream_title(meta: &str) -> Option<String> {
    let start = meta.find("StreamTitle='")? + "StreamTitle='".len();
    let rest = &meta[start..];
    // The title itself may contain apostrophes; the field ends at "';".
    let end = rest.find("';").unwrap_or_else(|| rest.trim_end_matches('\0').trim_end_matches('\'').len());
    let t = rest[..end].trim().to_string();
    (!t.is_empty()).then_some(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_title() {
        assert_eq!(
            parse_stream_title("StreamTitle='Кино - Группа крови';StreamUrl='';\0\0"),
            Some("Кино - Группа крови".into())
        );
        assert_eq!(parse_stream_title("StreamTitle='Guns N' Roses - Don't Cry';"), Some("Guns N' Roses - Don't Cry".into()));
        assert_eq!(parse_stream_title("StreamTitle='';"), None);
    }

    #[test]
    fn strips_metadata_blocks() {
        let meta = b"StreamTitle='A - B';";
        let mut block = vec![2u8];
        block.extend_from_slice(meta);
        block.resize(1 + 32, 0);
        let mut data = b"abcd".to_vec();
        data.extend_from_slice(&block);
        data.extend_from_slice(b"efgh");
        data.push(0);
        data.extend_from_slice(b"ij");
        let mut r = IcyReader::new(&data[..], Some(4));
        let mut out = Vec::new();
        r.read_to_end(&mut out).unwrap();
        assert_eq!(out, b"abcdefghij");
        assert_eq!(r.title.as_deref(), Some("A - B"));
    }
}
