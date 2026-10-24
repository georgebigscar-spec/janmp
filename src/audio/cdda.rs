//! Audio CD playback by reading raw CDDA sectors from the drive.
//!
//! Windows shows audio tracks as `TrackNN.cda` files on the drive; each holds the track's
//! position. On Linux the disc's table of contents is read from the device and tracks are
//! addressed as `cdda:///dev/sr0#N`. (macOS mounts audio CDs as AIFF files, which play as is.)

use std::path::Path;

use super::decoder::{AudioDecoder, StreamInfo};

/// Bytes of 16-bit stereo audio per CD sector (1/75 s).
const SECTOR_BYTES: usize = 2352;
const FRAMES_PER_SECTOR: u32 = 588;
const SECTORS_PER_READ: u32 = 20;

pub const URL_PREFIX: &str = "cdda://";

/// Position of one track on the disc, in sectors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CdTrack {
    pub number: u32,
    pub start: u32,
    pub sectors: u32,
}

impl CdTrack {
    pub fn duration(&self) -> f64 {
        self.sectors as f64 / 75.0
    }
}

/// Parse a Windows `.cda` file (a RIFF "CDDA" header describing the track).
pub fn parse_cda(data: &[u8]) -> Option<CdTrack> {
    if data.len() < 44 || &data[..4] != b"RIFF" || &data[8..12] != b"CDDA" || &data[12..16] != b"fmt " {
        return None;
    }
    let u16_at = |i: usize| u16::from_le_bytes([data[i], data[i + 1]]);
    let u32_at = |i: usize| u32::from_le_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]]);
    Some(CdTrack { number: u16_at(22) as u32, start: u32_at(28), sectors: u32_at(32) })
}

pub fn is_cd_url(s: &str) -> bool {
    s.starts_with(URL_PREFIX)
}

pub fn cd_url(device: &str, track: u32) -> String {
    format!("{URL_PREFIX}{device}#{track}")
}

fn parse_cd_url(s: &str) -> Option<(&str, u32)> {
    let (dev, n) = s.strip_prefix(URL_PREFIX)?.rsplit_once('#')?;
    Some((dev, n.parse().ok()?))
}

/// Track info for the playlist scanner.
pub fn track_info(path: &Path) -> Option<CdTrack> {
    let s = path.to_string_lossy();
    if let Some((dev, n)) = parse_cd_url(&s) {
        return read_toc(dev).ok()?.into_iter().find(|t| t.number == n);
    }
    parse_cda(&std::fs::read(path).ok()?)
}

pub struct CdDecoder {
    drive: platform::Drive,
    track: CdTrack,
    /// Next sector to read, relative to the track start.
    pos: u32,
    /// Frames to drop from the next read (sample-accurate seek).
    skip: usize,
    buf: Vec<u8>,
}

impl CdDecoder {
    pub fn open(path: &Path) -> Result<Self, String> {
        let s = path.to_string_lossy();
        let (drive, track) = if let Some((dev, n)) = parse_cd_url(&s) {
            let toc = read_toc(dev)?;
            let track = toc.into_iter().find(|t| t.number == n).ok_or("no such track on the disc")?;
            (platform::Drive::open(dev)?, track)
        } else {
            let data = std::fs::read(path).map_err(|e| e.to_string())?;
            let track = parse_cda(&data).ok_or("not a CD track file")?;
            (platform::Drive::open_for_cda(path)?, track)
        };
        Ok(Self { drive, track, pos: 0, skip: 0, buf: vec![0; SECTOR_BYTES * SECTORS_PER_READ as usize] })
    }
}

impl AudioDecoder for CdDecoder {
    fn spec(&self) -> (u32, usize) {
        (44_100, 2)
    }

    fn duration(&self) -> Option<f64> {
        Some(self.track.duration())
    }

    fn info(&self) -> StreamInfo {
        StreamInfo {
            codec: "CD Audio".into(),
            sample_rate: 44_100,
            channels: 2,
            bits_per_sample: Some(16),
            bitrate: Some(1411),
            station: None,
        }
    }

    fn next_chunk(&mut self, out: &mut Vec<f32>) -> Result<bool, String> {
        if self.pos >= self.track.sectors {
            return Ok(false);
        }
        let count = SECTORS_PER_READ.min(self.track.sectors - self.pos);
        let bytes = &mut self.buf[..count as usize * SECTOR_BYTES];
        self.drive.read(self.track.start + self.pos, count, bytes)?;
        self.pos += count;
        let skip = (self.skip * 4).min(bytes.len());
        self.skip = 0;
        out.extend(bytes[skip..].as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes(*b) as f32 / 32_768.0));
        Ok(true)
    }

    fn seek(&mut self, secs: f64) -> Result<(), String> {
        let frame = (secs.max(0.0) * 44_100.0) as u64;
        let sector = (frame / FRAMES_PER_SECTOR as u64).min(self.track.sectors as u64) as u32;
        self.pos = sector;
        self.skip = (frame - sector as u64 * FRAMES_PER_SECTOR as u64) as usize;
        Ok(())
    }
}

/// Audio tracks of the disc in `device`.
pub fn read_toc(device: &str) -> Result<Vec<CdTrack>, String> {
    platform::read_toc(device)
}

/// Drives that hold an audio CD right now, with their tracks.
pub fn find_discs() -> Vec<(String, Vec<CdTrack>)> {
    platform::devices().into_iter().filter_map(|d| Some((d.clone(), read_toc(&d).ok().filter(|t| !t.is_empty())?))).collect()
}

#[cfg(target_os = "linux")]
mod platform {
    use std::fs::File;
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::OpenOptionsExt;
    use std::path::Path;

    use super::CdTrack;

    const CDROMREADTOCHDR: libc::c_ulong = 0x5305;
    const CDROMREADTOCENTRY: libc::c_ulong = 0x5306;
    const CDROMREADAUDIO: libc::c_ulong = 0x530e;
    const CDROM_LBA: u8 = 0x01;
    const CDROM_LEADOUT: u8 = 0xAA;
    const CDROM_DATA_TRACK: u8 = 0x04;

    #[repr(C)]
    #[derive(Default)]
    struct TocHeader {
        first: u8,
        last: u8,
    }

    #[repr(C)]
    #[derive(Default)]
    struct TocEntry {
        track: u8,
        /// `adr` in the low four bits, `ctrl` in the high four.
        adr_ctrl: u8,
        format: u8,
        lba: i32,
        datamode: u8,
    }

    #[repr(C)]
    struct ReadAudio {
        lba: i32,
        addr_format: u8,
        nframes: i32,
        buf: *mut u8,
    }

    pub struct Drive(File);

    fn open(device: &str) -> Result<File, String> {
        std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(device)
            .map_err(|e| format!("{device}: {e}"))
    }

    impl Drive {
        pub fn open(device: &str) -> Result<Self, String> {
            open(device).map(Drive)
        }

        pub fn open_for_cda(_path: &Path) -> Result<Self, String> {
            Err("CD track files (.cda) are a Windows feature; use Menu → Add audio CD".into())
        }

        pub fn read(&mut self, lba: u32, count: u32, buf: &mut [u8]) -> Result<(), String> {
            let mut req = ReadAudio { lba: lba as i32, addr_format: CDROM_LBA, nframes: count as i32, buf: buf.as_mut_ptr() };
            // SAFETY: `req.buf` points to `count` sectors of writable memory.
            let r = unsafe { libc::ioctl(self.0.as_raw_fd(), CDROMREADAUDIO as _, &mut req) };
            if r < 0 {
                return Err(format!("reading the CD: {}", std::io::Error::last_os_error()));
            }
            Ok(())
        }
    }

    pub fn read_toc(device: &str) -> Result<Vec<CdTrack>, String> {
        let f = open(device)?;
        let fd = f.as_raw_fd();
        let mut hdr = TocHeader::default();
        // SAFETY: the kernel fills a `cdrom_tochdr`, which `TocHeader` mirrors.
        if unsafe { libc::ioctl(fd, CDROMREADTOCHDR as _, &mut hdr) } < 0 {
            return Err(format!("{device}: no audio CD ({})", std::io::Error::last_os_error()));
        }
        let entry = |track: u8| -> Result<TocEntry, String> {
            let mut e = TocEntry { track, format: CDROM_LBA, ..Default::default() };
            // SAFETY: the kernel fills a `cdrom_tocentry`, which `TocEntry` mirrors.
            if unsafe { libc::ioctl(fd, CDROMREADTOCENTRY as _, &mut e) } < 0 {
                return Err(format!("{device}: {}", std::io::Error::last_os_error()));
            }
            Ok(e)
        };
        let mut entries = Vec::new();
        for n in hdr.first..=hdr.last {
            entries.push(entry(n)?);
        }
        entries.push(entry(CDROM_LEADOUT)?);
        Ok(entries
            .windows(2)
            .filter(|w| (w[0].adr_ctrl >> 4) & CDROM_DATA_TRACK == 0)
            .map(|w| CdTrack { number: w[0].track as u32, start: w[0].lba as u32, sectors: (w[1].lba - w[0].lba).max(0) as u32 })
            .collect())
    }

    pub fn devices() -> Vec<String> {
        let mut v: Vec<String> = (0..4).map(|i| format!("/dev/sr{i}")).filter(|d| Path::new(d).exists()).collect();
        if v.is_empty() && Path::new("/dev/cdrom").exists() {
            v.push("/dev/cdrom".into());
        }
        v
    }
}

#[cfg(windows)]
mod platform {
    use std::os::windows::ffi::OsStrExt;
    use std::path::{Component, Path, Prefix};

    use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_READ, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING};
    use windows_sys::Win32::System::IO::DeviceIoControl;

    use super::{CdTrack, SECTOR_BYTES};

    const IOCTL_CDROM_RAW_READ: u32 = 0x0002_403E;
    const CDDA: i32 = 2;

    #[repr(C)]
    struct RawReadInfo {
        disk_offset: i64,
        sector_count: u32,
        track_mode: i32,
    }

    pub struct Drive(HANDLE);

    // SAFETY: the handle is only used by the thread that owns the decoder.
    unsafe impl Send for Drive {}

    impl Drop for Drive {
        fn drop(&mut self) {
            // SAFETY: the handle came from CreateFileW and is closed once.
            unsafe { CloseHandle(self.0) };
        }
    }

    impl Drive {
        pub fn open(device: &str) -> Result<Self, String> {
            let wide: Vec<u16> = std::ffi::OsStr::new(device).encode_wide().chain(Some(0)).collect();
            // SAFETY: `wide` is a NUL-terminated UTF-16 string.
            let h = unsafe {
                CreateFileW(
                    wide.as_ptr(),
                    GENERIC_READ,
                    FILE_SHARE_READ | FILE_SHARE_WRITE,
                    std::ptr::null(),
                    OPEN_EXISTING,
                    0,
                    std::ptr::null_mut(),
                )
            };
            if h == INVALID_HANDLE_VALUE {
                return Err(format!("{device}: {}", std::io::Error::last_os_error()));
            }
            Ok(Drive(h))
        }

        /// The drive that holds a `D:\Track01.cda` file.
        pub fn open_for_cda(path: &Path) -> Result<Self, String> {
            let letter = match path.components().next() {
                Some(Component::Prefix(p)) => match p.kind() {
                    Prefix::Disk(l) | Prefix::VerbatimDisk(l) => l as char,
                    _ => return Err("not on a CD drive".into()),
                },
                _ => return Err("not on a CD drive".into()),
            };
            Self::open(&format!(r"\\.\{letter}:"))
        }

        pub fn read(&mut self, lba: u32, count: u32, buf: &mut [u8]) -> Result<(), String> {
            let info = RawReadInfo { disk_offset: lba as i64 * 2048, sector_count: count, track_mode: CDDA };
            let mut got = 0u32;
            // SAFETY: `buf` holds `count` sectors; the input struct matches RAW_READ_INFO.
            let ok = unsafe {
                DeviceIoControl(
                    self.0,
                    IOCTL_CDROM_RAW_READ,
                    &info as *const RawReadInfo as *const _,
                    std::mem::size_of::<RawReadInfo>() as u32,
                    buf.as_mut_ptr() as *mut _,
                    (count as usize * SECTOR_BYTES) as u32,
                    &mut got,
                    std::ptr::null_mut(),
                )
            };
            if ok == 0 {
                return Err(format!("reading the CD: {}", std::io::Error::last_os_error()));
            }
            Ok(())
        }
    }

    /// Windows lists the tracks as `.cda` files, so the playlist adds those instead.
    pub fn read_toc(_device: &str) -> Result<Vec<CdTrack>, String> {
        Err("open the CD drive's .cda files instead".into())
    }

    /// Drive roots that contain `.cda` tracks.
    pub fn devices() -> Vec<String> {
        Vec::new()
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
mod platform {
    use std::path::Path;

    use super::CdTrack;

    pub struct Drive;

    impl Drive {
        pub fn open(_device: &str) -> Result<Self, String> {
            Err("audio CD reading is not supported on this system".into())
        }
        pub fn open_for_cda(_path: &Path) -> Result<Self, String> {
            Self::open("")
        }
        pub fn read(&mut self, _lba: u32, _count: u32, _buf: &mut [u8]) -> Result<(), String> {
            Self::open("").map(|_| ())
        }
    }

    pub fn read_toc(device: &str) -> Result<Vec<CdTrack>, String> {
        Drive::open(device).map(|_| Vec::new())
    }

    pub fn devices() -> Vec<String> {
        Vec::new()
    }
}

/// Windows drive roots holding `.cda` tracks of an inserted audio CD.
pub fn cda_roots() -> Vec<std::path::PathBuf> {
    if !cfg!(windows) {
        return Vec::new();
    }
    (b'D'..=b'Z')
        .map(|l| std::path::PathBuf::from(format!("{}:\\", l as char)))
        .filter(|root| {
            std::fs::read_dir(root).is_ok_and(|mut d| {
                d.any(|e| e.is_ok_and(|e| e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("cda"))))
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cda(track: u16, start: u32, sectors: u32) -> Vec<u8> {
        let mut v = b"RIFF".to_vec();
        v.extend_from_slice(&36u32.to_le_bytes());
        v.extend_from_slice(b"CDDAfmt ");
        v.extend_from_slice(&24u32.to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&track.to_le_bytes());
        v.extend_from_slice(&0x1234_5678u32.to_le_bytes());
        v.extend_from_slice(&start.to_le_bytes());
        v.extend_from_slice(&sectors.to_le_bytes());
        v.extend_from_slice(&[0; 8]); // MSF copies of the same positions
        v
    }

    #[test]
    fn parses_cda() {
        let t = parse_cda(&cda(3, 33_000, 18_000)).unwrap();
        assert_eq!(t, CdTrack { number: 3, start: 33_000, sectors: 18_000 });
        assert_eq!(t.duration(), 240.0);
        assert!(parse_cda(b"RIFF....WAVEfmt ").is_none());
    }

    #[test]
    fn cd_urls() {
        let u = cd_url("/dev/sr0", 7);
        assert!(is_cd_url(&u));
        assert_eq!(parse_cd_url(&u), Some(("/dev/sr0", 7)));
    }
}
