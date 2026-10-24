//! Lock-free tap of the most recent output samples for the visualizer.

use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

pub const VIS_SIZE: usize = 4096;

pub struct VisTap {
    buf: Vec<AtomicU32>,
    write: AtomicUsize,
}

impl Default for VisTap {
    fn default() -> Self {
        Self { buf: (0..VIS_SIZE).map(|_| AtomicU32::new(0)).collect(), write: AtomicUsize::new(0) }
    }
}

impl VisTap {
    /// Called from the audio callback with mono samples.
    pub fn push(&self, sample: f32) {
        let i = self.write.fetch_add(1, Ordering::Relaxed) % VIS_SIZE;
        self.buf[i].store(sample.to_bits(), Ordering::Relaxed);
    }

    /// Copy the newest `out.len()` samples, oldest first.
    pub fn snapshot(&self, out: &mut [f32]) {
        let n = out.len().min(VIS_SIZE);
        let end = self.write.load(Ordering::Relaxed);
        for (k, o) in out.iter_mut().take(n).enumerate() {
            let idx = (end + VIS_SIZE - n + k) % VIS_SIZE;
            *o = f32::from_bits(self.buf[idx].load(Ordering::Relaxed));
        }
    }

    pub fn clear(&self) {
        for s in &self.buf {
            s.store(0, Ordering::Relaxed);
        }
    }
}
