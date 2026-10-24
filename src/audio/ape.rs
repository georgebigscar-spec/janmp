//! Built-in Monkey's Audio (.ape) decoder, so APE plays without FFmpeg.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use ape_decoder::ApeDecoder;

use super::decoder::{AudioDecoder, StreamInfo};

pub struct ApeFileDecoder {
    dec: ApeDecoder<BufReader<File>>,
    next_frame: u32,
    /// Blocks to drop from the start of the next decoded frame (after a seek).
    skip: usize,
    rate: u32,
    channels: usize,
    bits: u16,
    float: bool,
    big_endian: bool,
    bitrate: u32,
    total: u64,
}

impl ApeFileDecoder {
    /// Fails for files older than v3.95, which are left to FFmpeg.
    pub fn open(path: &Path) -> Result<Self, String> {
        let file = File::open(path).map_err(|e| e.to_string())?;
        let dec = ApeDecoder::new(BufReader::new(file)).map_err(|e| e.to_string())?;
        let i = dec.info();
        if !matches!(i.bits_per_sample, 8 | 16 | 24 | 32) || i.channels == 0 {
            return Err(format!("unsupported APE layout: {} bit, {} ch", i.bits_per_sample, i.channels));
        }
        Ok(Self {
            rate: i.sample_rate,
            channels: i.channels as usize,
            bits: i.bits_per_sample,
            float: i.is_floating_point,
            big_endian: i.is_big_endian,
            bitrate: i.average_bitrate_kbps,
            total: i.total_samples,
            dec,
            next_frame: 0,
            skip: 0,
        })
    }

    fn convert(&self, pcm: &[u8], out: &mut Vec<f32>) {
        let width = (self.bits / 8) as usize;
        let skip = (self.skip * self.channels * width).min(pcm.len());
        let bytes = &pcm[skip..];
        out.reserve(bytes.len() / width);
        let word = |b: &[u8]| -> [u8; 4] {
            let mut w = [0u8; 4];
            if self.big_endian {
                for (i, x) in b.iter().rev().enumerate() {
                    w[i] = *x;
                }
            } else {
                w[..b.len()].copy_from_slice(b);
            }
            w
        };
        for s in bytes.chunks_exact(width) {
            let w = word(s);
            let v = match self.bits {
                8 => (w[0] as f32 - 128.0) / 128.0,
                16 => i16::from_le_bytes([w[0], w[1]]) as f32 / 32_768.0,
                24 => (i32::from_le_bytes([0, w[0], w[1], w[2]]) >> 8) as f32 / 8_388_608.0,
                _ if self.float => f32::from_le_bytes(w),
                _ => i32::from_le_bytes(w) as f32 / 2_147_483_648.0,
            };
            out.push(v);
        }
    }
}

impl AudioDecoder for ApeFileDecoder {
    fn spec(&self) -> (u32, usize) {
        (self.rate, self.channels)
    }

    fn duration(&self) -> Option<f64> {
        Some(self.total as f64 / self.rate as f64)
    }

    fn info(&self) -> StreamInfo {
        StreamInfo {
            codec: "APE".into(),
            sample_rate: self.rate,
            channels: self.channels,
            bits_per_sample: Some(self.bits as u32),
            bitrate: (self.bitrate > 0).then_some(self.bitrate),
            station: None,
        }
    }

    fn next_chunk(&mut self, out: &mut Vec<f32>) -> Result<bool, String> {
        if self.next_frame >= self.dec.total_frames() {
            return Ok(false);
        }
        let pcm = self.dec.decode_frame(self.next_frame).map_err(|e| e.to_string())?;
        self.convert(&pcm, out);
        self.skip = 0;
        self.next_frame += 1;
        Ok(true)
    }

    fn seek(&mut self, secs: f64) -> Result<(), String> {
        let sample = (secs.max(0.0) * self.rate as f64) as u64;
        if sample >= self.total {
            self.next_frame = self.dec.total_frames();
            return Ok(());
        }
        let r = self.dec.seek(sample).map_err(|e| e.to_string())?;
        self.next_frame = r.frame_index;
        self.skip = r.skip_samples as usize;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxideav_ape::{encode_pcm, CompressionLevel, EncoderConfig};

    fn sine(n: usize, freq: f32, amp: f32) -> Vec<i32> {
        (0..n).map(|i| ((i as f32 * freq * std::f32::consts::TAU / 44_100.0).sin() * amp * 32_767.0) as i32).collect()
    }

    #[test]
    fn decodes_and_seeks() {
        let n = 44_100 * 5;
        let l = sine(n, 440.0, 0.5);
        let r = sine(n, 330.0, 0.25);
        let mut cfg = EncoderConfig::new(CompressionLevel::High, 2, 44_100, 16);
        cfg.blocks_per_frame = 44_100; // several frames, to exercise seeking across them
        let bytes = encode_pcm(cfg, &[l.clone(), r.clone()]).unwrap();
        let path = std::env::temp_dir().join(format!("janmp-ape-dec-{}.ape", std::process::id()));
        std::fs::write(&path, bytes).unwrap();

        let mut d = ApeFileDecoder::open(&path).unwrap();
        assert_eq!(d.spec(), (44_100, 2));
        assert!((d.duration().unwrap() - 5.0).abs() < 1e-6);
        let mut out = Vec::new();
        while d.next_chunk(&mut out).unwrap() {}
        assert_eq!(out.len(), n * 2);
        for (i, (a, b)) in l.iter().zip(&r).enumerate() {
            assert_eq!((out[2 * i] * 32_768.0) as i32, *a);
            assert_eq!((out[2 * i + 1] * 32_768.0) as i32, *b);
        }

        d.seek(2.5).unwrap();
        let mut after = Vec::new();
        d.next_chunk(&mut after).unwrap();
        let first = 44_100 * 5 / 2;
        assert_eq!((after[0] * 32_768.0) as i32, l[first]);
        assert_eq!((after[1] * 32_768.0) as i32, r[first]);
        std::fs::remove_file(&path).ok();
    }
}
