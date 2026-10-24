//! M3U / M3U8 / PLS reading and M3U8 writing.

use std::io::Write;
use std::path::{Path, PathBuf};

use super::cue::decode_text;
use super::Track;

/// An entry of a playlist file: either a local path or a URL (internet radio).
#[derive(Debug, Clone, PartialEq)]
pub enum Entry {
    File { path: PathBuf, title: Option<String> },
    Url { url: String, title: Option<String> },
}

fn make_entry(target: &str, title: Option<String>, base: &Path) -> Entry {
    let lower = target.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return Entry::Url { url: target.to_string(), title };
    }
    let target = target.strip_prefix("file://").unwrap_or(target);
    let p = PathBuf::from(target);
    Entry::File { path: if p.is_absolute() { p } else { base.join(p) }, title }
}

pub fn load(path: &Path) -> std::io::Result<Vec<Entry>> {
    let text = decode_text(&std::fs::read(path)?);
    let base = path.parent().unwrap_or(Path::new("."));
    let is_pls = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("pls"));
    Ok(if is_pls { parse_pls(&text, base) } else { parse_m3u(&text, base) })
}

pub fn parse_m3u(text: &str, base: &Path) -> Vec<Entry> {
    let mut out = Vec::new();
    let mut title = None;
    for line in text.lines().map(str::trim) {
        if line.is_empty() {
            continue;
        }
        if let Some(info) = line.strip_prefix("#EXTINF:") {
            title = info.split_once(',').map(|(_, t)| t.trim().to_string()).filter(|t| !t.is_empty());
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        out.push(make_entry(line, title.take(), base));
    }
    out
}

pub fn parse_pls(text: &str, base: &Path) -> Vec<Entry> {
    let mut files: Vec<(u32, String)> = Vec::new();
    let mut titles: std::collections::HashMap<u32, String> = Default::default();
    for line in text.lines().map(str::trim) {
        let Some((k, v)) = line.split_once('=') else { continue };
        let k = k.trim().to_ascii_lowercase();
        if let Some(n) = k.strip_prefix("file").and_then(|n| n.parse().ok()) {
            files.push((n, v.trim().to_string()));
        } else if let Some(n) = k.strip_prefix("title").and_then(|n| n.parse().ok()) {
            titles.insert(n, v.trim().to_string());
        }
    }
    files.sort_by_key(|(n, _)| *n);
    files.into_iter().map(|(n, f)| make_entry(&f, titles.remove(&n), base)).collect()
}

pub fn save_m3u8(path: &Path, tracks: &[Track]) -> std::io::Result<()> {
    let mut f = std::io::BufWriter::new(std::fs::File::create(path)?);
    writeln!(f, "#EXTM3U")?;
    let mut last: Option<&Path> = None;
    for t in tracks {
        // A cue-split file is written once; the cue sheet next to it carries the split.
        if t.cue.is_some() && last == Some(t.path.as_path()) {
            continue;
        }
        last = Some(t.path.as_path());
        let secs = t.duration.map(|d| d.round() as i64).unwrap_or(-1);
        writeln!(f, "#EXTINF:{secs},{} - {}", t.artist, t.title)?;
        writeln!(f, "{}", t.path.display())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn m3u_with_titles_and_urls() {
        let e = parse_m3u("#EXTM3U\n#EXTINF:123,Song\nsub/a.mp3\nhttp://radio/stream\n", Path::new("/m"));
        assert_eq!(e[0], Entry::File { path: "/m/sub/a.mp3".into(), title: Some("Song".into()) });
        assert_eq!(e[1], Entry::Url { url: "http://radio/stream".into(), title: None });
    }

    #[test]
    fn pls_entries() {
        let e = parse_pls("[playlist]\nFile1=http://x/1\nTitle1=Radio One\nNumberOfEntries=1\n", Path::new("/"));
        assert_eq!(e, vec![Entry::Url { url: "http://x/1".into(), title: Some("Radio One".into()) }]);
    }
}
