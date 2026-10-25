//! 18-band graphic equalizer, applied in the output callback so changes are heard at once.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

/// Band centre frequencies (the AIMP layout).
pub const BANDS: [f32; 18] = [
    31.0, 63.0, 87.0, 125.0, 175.0, 250.0, 350.0, 500.0, 700.0, 1_000.0, 1_400.0, 2_000.0, 2_800.0, 4_000.0,
    5_600.0, 8_000.0, 11_200.0, 16_000.0,
];
pub const MAX_DB: f32 = 15.0;
/// Bands are spaced half an octave apart; this Q gives neighbouring bands a smooth overlap.
const Q: f64 = 1.8;

/// Equalizer settings written by the UI and read by the audio callback.
pub struct EqShared {
    enabled: AtomicBool,
    preamp_db: AtomicU32,
    gains_db: [AtomicU32; 18],
    version: AtomicU64,
}

impl Default for EqShared {
    fn default() -> Self {
        Self {
            enabled: AtomicBool::new(false),
            preamp_db: AtomicU32::new(0f32.to_bits()),
            gains_db: std::array::from_fn(|_| AtomicU32::new(0f32.to_bits())),
            version: AtomicU64::new(0),
        }
    }
}

impl EqShared {
    pub fn set(&self, enabled: bool, preamp_db: f32, gains_db: &[f32; 18]) {
        self.enabled.store(enabled, Ordering::Relaxed);
        self.preamp_db.store(preamp_db.clamp(-MAX_DB, MAX_DB).to_bits(), Ordering::Relaxed);
        for (a, g) in self.gains_db.iter().zip(gains_db) {
            a.store(g.clamp(-MAX_DB, MAX_DB).to_bits(), Ordering::Relaxed);
        }
        self.version.fetch_add(1, Ordering::Release);
    }
}

#[derive(Clone, Copy, Default)]
struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
}

impl Biquad {
    /// RBJ peaking filter.
    fn peaking(freq: f64, rate: f64, gain_db: f64) -> Self {
        let a = 10f64.powf(gain_db / 40.0);
        let w0 = std::f64::consts::TAU * freq / rate;
        let alpha = w0.sin() / (2.0 * Q);
        let cos = w0.cos();
        let a0 = 1.0 + alpha / a;
        Self {
            b0: (1.0 + alpha * a) / a0,
            b1: -2.0 * cos / a0,
            b2: (1.0 - alpha * a) / a0,
            a1: -2.0 * cos / a0,
            a2: (1.0 - alpha / a) / a0,
        }
    }
}

/// Per-callback processor; owns filter state for every channel.
pub struct Equalizer {
    rate: f64,
    channels: usize,
    version: u64,
    active: bool,
    preamp: f32,
    /// Filters with a non-zero gain.
    filters: Vec<Biquad>,
    /// Transposed direct form II state, `[channel][filter] = (z1, z2)`.
    state: Vec<[(f64, f64); 18]>,
}

impl Equalizer {
    pub fn new(rate: u32, channels: usize) -> Self {
        Self {
            rate: rate as f64,
            channels,
            version: u64::MAX,
            active: false,
            preamp: 1.0,
            filters: Vec::with_capacity(18),
            state: vec![[(0.0, 0.0); 18]; channels],
        }
    }

    /// Pick up new settings. Allocation-free, so it is safe in the audio callback.
    pub fn update(&mut self, shared: &EqShared) {
        let v = shared.version.load(Ordering::Acquire);
        if v == self.version {
            return;
        }
        self.version = v;
        let was_active = self.active;
        self.active = shared.enabled.load(Ordering::Relaxed);
        self.preamp = 10f32.powf(f32::from_bits(shared.preamp_db.load(Ordering::Relaxed)) / 20.0);
        let old_len = self.filters.len();
        self.filters.clear();
        let nyquist = self.rate / 2.0;
        for (f, g) in BANDS.iter().zip(&shared.gains_db) {
            let g = f32::from_bits(g.load(Ordering::Relaxed)) as f64;
            if g.abs() >= 0.05 && (*f as f64) < nyquist * 0.9 {
                self.filters.push(Biquad::peaking(*f as f64, self.rate, g));
            }
        }
        // The set of filters changed shape: start them from silence rather than from
        // another filter's history.
        if !was_active || self.filters.len() != old_len {
            self.state.fill([(0.0, 0.0); 18]);
        }
    }

    /// Filter one interleaved frame in place.
    #[inline]
    pub fn process(&mut self, frame: &mut [f32]) {
        if !self.active {
            return;
        }
        for (ch, s) in frame.iter_mut().enumerate().take(self.channels) {
            let mut x = (*s * self.preamp) as f64;
            let st = &mut self.state[ch];
            for (f, (z1, z2)) in self.filters.iter().zip(st.iter_mut()) {
                let y = f.b0 * x + *z1;
                *z1 = f.b1 * x - f.a1 * y + *z2;
                *z2 = f.b2 * x - f.a2 * y;
                x = y;
            }
            *s = x as f32;
        }
    }
}

/// Built-in presets: name and gains in dB.
pub const PRESETS: &[(&str, [f32; 18])] = &[
    ("Flat", [0.0; 18]),
    ("Rock", [5.0, 4.5, 4.0, 3.0, 1.5, 0.0, -1.0, -1.5, -1.0, 0.0, 1.0, 2.0, 3.0, 3.5, 4.0, 4.0, 4.5, 4.5]),
    ("Pop", [-1.0, -0.5, 0.0, 1.0, 2.0, 3.0, 3.5, 3.5, 3.0, 2.0, 1.0, 0.0, -0.5, -1.0, -1.0, -1.0, -1.0, -1.0]),
    ("Jazz", [3.0, 3.0, 2.5, 2.0, 1.0, 0.0, -1.0, -1.0, 0.0, 1.0, 1.5, 2.0, 2.5, 3.0, 3.0, 3.0, 3.0, 3.0]),
    ("Classical", [3.0, 3.0, 2.5, 2.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, -1.0, -2.0, -3.0, -3.5, -4.0, -4.0, -4.5]),
    ("Dance", [6.0, 5.5, 5.0, 4.0, 2.5, 1.0, 0.0, 0.0, 0.0, -1.0, -1.0, 0.0, 1.0, 2.0, 3.0, 3.5, 3.5, 3.0]),
    ("Bass boost", [7.0, 6.5, 6.0, 5.0, 4.0, 2.5, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
    ("Treble boost", [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.5, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 6.5, 7.0]),
    ("Vocal", [-2.0, -2.0, -1.5, -1.0, 0.0, 1.0, 2.5, 3.5, 4.0, 4.0, 3.5, 3.0, 2.0, 1.0, 0.0, -0.5, -1.0, -1.5]),
    ("Loudness", [6.0, 5.0, 4.0, 2.5, 1.0, 0.0, 0.0, -1.0, -1.0, -1.0, 0.0, 0.0, 1.0, 2.0, 3.0, 4.0, 4.5, 5.0]),
];

#[cfg(test)]
mod tests {
    use super::*;

    /// Amplitude of a sine at `freq` after the equalizer, once it has settled.
    fn response(eq: &mut Equalizer, freq: f32) -> f32 {
        let rate = 48_000.0;
        let mut peak = 0f32;
        for i in 0..48_000 {
            let s = (i as f32 * freq * std::f32::consts::TAU / rate).sin();
            let mut frame = [s, s];
            eq.process(&mut frame);
            if i > 24_000 {
                peak = peak.max(frame[0].abs());
            }
        }
        peak
    }

    fn db(x: f32) -> f32 {
        20.0 * x.log10()
    }

    #[test]
    fn boosts_and_cuts_bands() {
        let shared = EqShared::default();
        let mut gains = [0.0; 18];
        gains[9] = 12.0; // 1 kHz
        gains[1] = -12.0; // 63 Hz
        shared.set(true, 0.0, &gains);
        let mut eq = Equalizer::new(48_000, 2);
        eq.update(&shared);
        assert!((db(response(&mut eq, 1_000.0)) - 12.0).abs() < 1.0);
        assert!((db(response(&mut eq, 63.0)) + 12.0).abs() < 1.0);
        assert!(db(response(&mut eq, 8_000.0)).abs() < 1.0, "far bands stay flat");
    }

    #[test]
    fn disabled_is_transparent_and_preamp_scales() {
        let shared = EqShared::default();
        let mut eq = Equalizer::new(44_100, 2);
        eq.update(&shared);
        let mut f = [0.5, -0.25];
        eq.process(&mut f);
        assert_eq!(f, [0.5, -0.25]);
        shared.set(true, -6.0, &[0.0; 18]);
        eq.update(&shared);
        let mut f = [0.5, 0.5];
        eq.process(&mut f);
        assert!((f[0] - 0.5 * 0.501).abs() < 1e-3);
    }
}
