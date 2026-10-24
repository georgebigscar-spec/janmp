//! Tag reading and cover art lookup, done on background threads.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crossbeam_channel::{Receiver, Sender};
use lofty::picture::PictureType;
use lofty::prelude::*;

/// Tags and technical properties of one file.
#[derive(Debug, Clone, Default)]
pub struct TrackMeta {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub genre: Option<String>,
    pub year: Option<u32>,
    pub track_no: Option<u32>,
    pub disc_no: Option<u32>,
    pub duration: Option<f64>,
    pub bitrate: Option<u32>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u8>,
    pub bit_depth: Option<u8>,
    pub file_size: u64,
}

pub fn read_meta(path: &Path) -> TrackMeta {
    let mut m = TrackMeta {
        file_size: std::fs::metadata(path).map(|md| md.len()).unwrap_or(0),
        ..Default::default()
    };
    let Ok(tagged) = lofty::read_from_path(path) else {
        fill_from_ffmpeg(path, &mut m);
        return m;
    };
    let props = tagged.properties();
    let d = props.duration().as_secs_f64();
    if d > 0.0 {
        m.duration = Some(d);
    }
    m.bitrate = props.audio_bitrate().or(props.overall_bitrate());
    m.sample_rate = props.sample_rate();
    m.channels = props.channels();
    m.bit_depth = props.bit_depth();

    if let Some(tag) = tagged.primary_tag().or_else(|| tagged.first_tag()) {
        let s = |v: Option<std::borrow::Cow<'_, str>>| {
            v.map(|c| c.trim().to_string()).filter(|s| !s.is_empty())
        };
        m.title = s(tag.title());
        m.artist = s(tag.artist());
        m.album = s(tag.album());
        m.genre = s(tag.genre());
        m.album_artist = tag
            .get_string(ItemKey::AlbumArtist)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        m.year = tag.date().map(|d| d.year as u32).filter(|y| *y > 0);
        m.track_no = tag.track();
        m.disc_no = tag.disk();
    }
    if m.duration.is_none() || m.title.is_none() {
        fill_from_ffmpeg(path, &mut m);
    }
    m
}

/// Formats lofty cannot read (WMA, DSD, tracker modules, AC3, DTS…) are probed with ffmpeg.
fn fill_from_ffmpeg(path: &Path, m: &mut TrackMeta) {
    if crate::audio::ffmpeg::ffmpeg_path().is_none() {
        return;
    }
    let Ok(p) = crate::audio::ffmpeg::probe(path) else { return };
    let tag = |k: &str| p.tag(k).map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
    let num = |k: &str| p.tag(k).and_then(|v| v.split('/').next()?.trim().parse::<u32>().ok());
    m.duration = m.duration.or(p.duration);
    m.bitrate = m.bitrate.or(p.bitrate);
    m.sample_rate = m.sample_rate.or(p.sample_rate);
    m.channels = m.channels.or(p.channels.map(|c| c as u8));
    m.title = m.title.take().or_else(|| tag("title"));
    m.artist = m.artist.take().or_else(|| tag("artist").or_else(|| tag("author")));
    m.album = m.album.take().or_else(|| tag("album"));
    m.album_artist = m.album_artist.take().or_else(|| tag("album_artist"));
    m.genre = m.genre.take().or_else(|| tag("genre"));
    m.year = m.year.or_else(|| tag("date").or_else(|| tag("year")).and_then(|d| d.get(..4)?.parse().ok()));
    m.track_no = m.track_no.or_else(|| num("track"));
    m.disc_no = m.disc_no.or_else(|| num("disc"));
}

/// Embedded front cover, or a cover image next to the file.
pub fn find_cover(path: &Path) -> Option<Vec<u8>> {
    if let Ok(tagged) = lofty::read_from_path(path) {
        for tag in tagged.tags() {
            let pics = tag.pictures();
            let front = pics
                .iter()
                .find(|p| p.pic_type() == PictureType::CoverFront)
                .or_else(|| pics.first());
            if let Some(p) = front {
                return Some(p.data().to_vec());
            }
        }
    }
    let dir = path.parent()?;
    const NAMES: &[&str] = &["cover", "folder", "front", "album", "albumart"];
    let entries = std::fs::read_dir(dir).ok()?;
    let mut fallback = None;
    for e in entries.flatten() {
        let p = e.path();
        let ext = p.extension().map(|x| x.to_string_lossy().to_ascii_lowercase());
        if !matches!(ext.as_deref(), Some("jpg" | "jpeg" | "png" | "bmp" | "webp")) {
            continue;
        }
        let stem = p.file_stem().map(|s| s.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        if NAMES.iter().any(|n| stem == *n || stem.starts_with(n)) {
            return std::fs::read(&p).ok();
        }
        if fallback.is_none() {
            fallback = Some(p);
        }
    }
    fallback.and_then(|p| std::fs::read(p).ok())
}

/// Decoded, downscaled cover ready for upload as a texture.
pub struct CoverImage {
    pub size: [usize; 2],
    pub rgba: Vec<u8>,
}

fn decode_cover(bytes: &[u8], max: u32) -> Option<CoverImage> {
    let img = image::load_from_memory(bytes).ok()?;
    let img = img.thumbnail(max, max).to_rgba8();
    Some(CoverImage { size: [img.width() as usize, img.height() as usize], rgba: img.into_raw() })
}

enum Job {
    Meta { uid: u64, path: PathBuf },
    Cover { path: PathBuf, max: u32 },
}

pub enum ScanResult {
    Meta { uid: u64, meta: TrackMeta },
    Cover { path: PathBuf, max: u32, image: Option<Arc<CoverImage>> },
}

/// Background worker pool for tag and cover reading.
pub struct Scanner {
    tx: Sender<Job>,
    cover_tx: Sender<Job>,
    pub results: Receiver<ScanResult>,
}

impl Scanner {
    pub fn new(repaint: impl Fn() + Send + Sync + 'static) -> Self {
        let (tx, rx) = crossbeam_channel::unbounded::<Job>();
        let (cover_tx, cover_rx) = crossbeam_channel::unbounded::<Job>();
        let (rtx, results) = crossbeam_channel::unbounded();
        let repaint = Arc::new(repaint);
        let workers = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2).clamp(1, 4);
        for (i, rx) in std::iter::repeat_n(rx, workers).chain([cover_rx]).enumerate() {
            let rtx = rtx.clone();
            let repaint = repaint.clone();
            std::thread::Builder::new()
                .name(format!("janmp-scan-{i}"))
                .spawn(move || {
                    for job in rx {
                        let r = match job {
                            Job::Meta { uid, path } => ScanResult::Meta { uid, meta: read_meta(&path) },
                            Job::Cover { path, max } => {
                                let image = find_cover(&path)
                                    .and_then(|b| decode_cover(&b, max))
                                    .map(Arc::new);
                                ScanResult::Cover { path, max, image }
                            }
                        };
                        if rtx.send(r).is_err() {
                            break;
                        }
                        repaint();
                    }
                })
                .expect("spawn scanner");
        }
        Self { tx, cover_tx, results }
    }

    pub fn request_meta(&self, uid: u64, path: PathBuf) {
        let _ = self.tx.send(Job::Meta { uid, path });
    }

    pub fn request_cover(&self, path: PathBuf, max: u32) {
        let _ = self.cover_tx.send(Job::Cover { path, max });
    }
}

/// Cover textures by file path. `None` means "looked, nothing found".
pub struct CoverCache {
    /// Maximum edge length of the decoded images.
    pub max: u32,
    textures: HashMap<PathBuf, Option<egui::TextureHandle>>,
    requested: std::collections::HashSet<PathBuf>,
}

impl CoverCache {
    pub fn new(max: u32) -> Self {
        Self { max, textures: HashMap::new(), requested: Default::default() }
    }

    /// Returns the texture if loaded; schedules loading otherwise.
    pub fn get(&mut self, path: &Path, scanner: &Scanner) -> Option<egui::TextureHandle> {
        if let Some(t) = self.textures.get(path) {
            return t.clone();
        }
        if self.requested.insert(path.to_path_buf()) {
            scanner.request_cover(path.to_path_buf(), self.max);
        }
        None
    }

    pub fn insert(&mut self, ctx: &egui::Context, path: PathBuf, image: Option<Arc<CoverImage>>) {
        let tex = image.map(|img| {
            let ci = egui::ColorImage::from_rgba_unmultiplied(img.size, &img.rgba);
            ctx.load_texture(format!("cover:{}", path.display()), ci, egui::TextureOptions::LINEAR)
        });
        self.textures.insert(path, tex);
    }
}
