//! Streaming sample-rate conversion on top of rubato's FFT resampler.

use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Indexing, Resampler as _};

const CHUNK: usize = 1024;

pub struct StreamResampler {
    inner: Fft<f32>,
    channels: usize,
    ratio: f64,
    /// Interleaved input waiting for a full chunk.
    pending: Vec<f32>,
    out_buf: Vec<f32>,
    /// Output frames still to drop to compensate the resampler's delay.
    delay_left: usize,
    in_frames_total: u64,
    out_frames_total: u64,
}

impl StreamResampler {
    pub fn new(from: u32, to: u32, channels: usize) -> Result<Self, String> {
        let inner = Fft::<f32>::new(from as usize, to as usize, CHUNK, channels, FixedSync::Input)
            .map_err(|e| e.to_string())?;
        let delay_left = inner.output_delay();
        let out_buf = vec![0.0; inner.output_frames_max() * channels];
        Ok(Self {
            inner,
            channels,
            ratio: to as f64 / from as f64,
            pending: Vec::with_capacity(CHUNK * channels * 2),
            out_buf,
            delay_left,
            in_frames_total: 0,
            out_frames_total: 0,
        })
    }

    /// Feed interleaved input, appending resampled interleaved output to `out`.
    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        self.pending.extend_from_slice(input);
        self.in_frames_total += (input.len() / self.channels) as u64;
        loop {
            let need = self.inner.input_frames_next();
            if self.pending.len() / self.channels < need {
                break;
            }
            self.run(need, None, out);
            self.pending.drain(..need * self.channels);
        }
    }

    /// Flush everything still buffered (end of stream).
    pub fn finish(&mut self, out: &mut Vec<f32>) {
        let expected = (self.in_frames_total as f64 * self.ratio).round() as u64;
        let frames = self.pending.len() / self.channels;
        self.run(frames, Some(frames), out);
        self.pending.clear();
        // Push zeros through until the delayed tail has come out.
        let mut guard = 0;
        while self.out_frames_total < expected && guard < 8 {
            let need = self.inner.input_frames_next();
            self.pending.resize(need * self.channels, 0.0);
            self.run(need, Some(0), out);
            self.pending.clear();
            guard += 1;
        }
        if self.out_frames_total > expected {
            let extra = (self.out_frames_total - expected) as usize * self.channels;
            let keep = out.len().saturating_sub(extra);
            out.truncate(keep);
            self.out_frames_total = expected;
        }
    }

    fn run(&mut self, frames: usize, partial: Option<usize>, out: &mut Vec<f32>) {
        let need = self.inner.input_frames_next();
        // Partial chunks must still be backed by a full-size buffer.
        if self.pending.len() < need * self.channels {
            self.pending.resize(need * self.channels, 0.0);
        }
        let in_frames = self.pending.len() / self.channels;
        let out_frames = self.out_buf.len() / self.channels;
        let Ok(input) = InterleavedSlice::new(&self.pending[..], self.channels, in_frames) else {
            return;
        };
        let Ok(mut output) = InterleavedSlice::new_mut(&mut self.out_buf[..], self.channels, out_frames)
        else {
            return;
        };
        let mut indexing = Indexing::new();
        indexing.partial_len = partial;
        let _ = frames;
        match self.inner.process_into_buffer(&input, &mut output, Some(&indexing)) {
            Ok((_, produced)) => {
                let mut start = 0;
                if self.delay_left > 0 {
                    start = self.delay_left.min(produced);
                    self.delay_left -= start;
                }
                out.extend_from_slice(&self.out_buf[start * self.channels..produced * self.channels]);
                self.out_frames_total += (produced - start) as u64;
            }
            Err(e) => log::warn!("resampler error: {e}"),
        }
    }

    pub fn reset(&mut self) {
        self.inner.reset();
        self.pending.clear();
        self.delay_left = self.inner.output_delay();
        self.in_frames_total = 0;
        self.out_frames_total = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::StreamResampler;

    #[test]
    fn output_length_matches_ratio() {
        let mut r = StreamResampler::new(44_100, 48_000, 2).unwrap();
        let input = vec![0.25f32; 44_100 * 2];
        let mut out = Vec::new();
        for chunk in input.chunks(777 * 2) {
            r.process(chunk, &mut out);
        }
        r.finish(&mut out);
        assert_eq!(out.len(), 48_000 * 2);
        // DC level is preserved in the middle of the signal.
        assert!((out[48_000] - 0.25).abs() < 0.01, "{}", out[48_000]);
    }
}
