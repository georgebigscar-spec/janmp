//! CUE sheet parsing. A cue sheet splits one (or several) audio files into tracks.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub struct CueTrack {
    pub file: PathBuf,
    pub number: u32,
    pub title: Option<String>,
    pub performer: Option<String>,
    pub album: Option<String>,
    pub album_performer: Option<String>,
    pub genre: Option<String>,
    pub year: Option<u32>,
    /// Start inside `file`, seconds.
    pub start: f64,
    /// End inside `file`, seconds (`None` = until end of file).
    pub end: Option<f64>,
}

/// Decode text that may be UTF-8 (with or without BOM) or a legacy Windows code page.
pub fn decode_text(bytes: &[u8]) -> String {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        // Cue sheets from the Russian-speaking world are very often CP1251.
        Err(_) => encoding_rs::WINDOWS_1251.decode(bytes).0.into_owned(),
    }
}

pub fn load(path: &Path) -> std::io::Result<Vec<CueTrack>> {
    let bytes = std::fs::read(path)?;
    let base = path.parent().unwrap_or(Path::new("."));
    Ok(parse(&decode_text(&bytes), base))
}

/// Split a cue line into the command and its arguments, honouring quotes.
fn tokens(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for c in line.chars() {
        match c {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// `mm:ss:ff` (75 frames per second) to seconds.
fn parse_msf(s: &str) -> Option<f64> {
    let mut it = s.split(':');
    let m: f64 = it.next()?.parse().ok()?;
    let sec: f64 = it.next()?.parse().ok()?;
    let f: f64 = it.next().unwrap_or("0").parse().ok()?;
    Some(m * 60.0 + sec + f / 75.0)
}

pub fn parse(text: &str, base: &Path) -> Vec<CueTrack> {
    let mut tracks: Vec<CueTrack> = Vec::new();
    let mut album = None;
    let mut album_performer = None;
    let mut genre = None;
    let mut year = None;
    let mut file: Option<PathBuf> = None;
    let mut in_track = false;

    for raw in text.lines() {
        let t = tokens(raw.trim());
        let Some(cmd) = t.first() else { continue };
        let arg = t.get(1).cloned();
        match cmd.to_ascii_uppercase().as_str() {
            "REM" => match t.get(1).map(|s| s.to_ascii_uppercase()).as_deref() {
                Some("GENRE") => genre = t.get(2).cloned(),
                Some("DATE") => year = t.get(2).and_then(|y| y.get(..4)?.parse().ok()),
                _ => {}
            },
            "FILE" => {
                if let Some(f) = arg {
                    let p = PathBuf::from(&f);
                    file = Some(if p.is_absolute() { p } else { base.join(p) });
                }
                in_track = false;
            }
            "TRACK" => {
                let number = arg.and_then(|n| n.parse().ok()).unwrap_or(tracks.len() as u32 + 1);
                let is_audio = t.get(2).is_none_or(|k| k.eq_ignore_ascii_case("AUDIO"));
                if let (Some(f), true) = (&file, is_audio) {
                    tracks.push(CueTrack {
                        file: f.clone(),
                        number,
                        title: None,
                        performer: None,
                        album: album.clone(),
                        album_performer: album_performer.clone(),
                        genre: genre.clone(),
                        year,
                        start: f64::NAN,
                        end: None,
                    });
                    in_track = true;
                } else {
                    in_track = false;
                }
            }
            "TITLE" => {
                if in_track {
                    if let Some(tr) = tracks.last_mut() {
                        tr.title = arg;
                    }
                } else {
                    album = arg;
                }
            }
            "PERFORMER" => {
                if in_track {
                    if let Some(tr) = tracks.last_mut() {
                        tr.performer = arg;
                    }
                } else {
                    album_performer = arg;
                }
            }
            "INDEX" if in_track => {
                let idx: u32 = arg.and_then(|n| n.parse().ok()).unwrap_or(1);
                let time = t.get(2).and_then(|s| parse_msf(s));
                if let (Some(tr), Some(time)) = (tracks.last_mut(), time) {
                    // INDEX 01 is the track start; INDEX 00 (pregap) only if 01 is missing.
                    if idx == 1 || (idx == 0 && tr.start.is_nan()) {
                        tr.start = time;
                    }
                }
            }
            _ => {}
        }
    }

    tracks.retain(|t| !t.start.is_nan());
    // Each track ends where the next one in the same file begins.
    for i in 0..tracks.len() {
        if let Some(next) = tracks.get(i + 1) {
            if next.file == tracks[i].file {
                tracks[i].end = Some(next.start);
            }
        }
        if tracks[i].performer.is_none() {
            tracks[i].performer = tracks[i].album_performer.clone();
        }
    }
    tracks
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"REM GENRE Rock
REM DATE 1994
PERFORMER "Кино"
TITLE "Последний герой"
FILE "album.flac" WAVE
  TRACK 01 AUDIO
    TITLE "Хочу перемен"
    INDEX 01 00:00:00
  TRACK 02 AUDIO
    TITLE "Группа крови"
    PERFORMER "Виктор Цой"
    INDEX 00 04:50:00
    INDEX 01 04:52:37
"#;

    #[test]
    fn parses_tracks_and_offsets() {
        let t = parse(SAMPLE, Path::new("/music"));
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].title.as_deref(), Some("Хочу перемен"));
        assert_eq!(t[0].performer.as_deref(), Some("Кино"));
        assert_eq!(t[0].album.as_deref(), Some("Последний герой"));
        assert_eq!(t[0].year, Some(1994));
        assert_eq!(t[0].file, PathBuf::from("/music/album.flac"));
        let start2 = 4.0 * 60.0 + 52.0 + 37.0 / 75.0;
        assert!((t[1].start - start2).abs() < 1e-9);
        assert_eq!(t[0].end, Some(t[1].start));
        assert_eq!(t[1].end, None);
        assert_eq!(t[1].performer.as_deref(), Some("Виктор Цой"));
    }

    #[test]
    fn decodes_cp1251() {
        let (bytes, _, _) = encoding_rs::WINDOWS_1251.encode("TITLE \"Привет\"");
        assert_eq!(decode_text(&bytes), "TITLE \"Привет\"");
    }
}
