//! Spectrum analyzer: FFT of the output tap, log-spaced bars with falling peaks.

use std::sync::Arc;

use egui::{pos2, vec2, Color32, Rect, Sense, Ui};
use rustfft::num_complex::Complex32;
use rustfft::{Fft, FftPlanner};

use super::skin::Skin;

const FFT_SIZE: usize = 2048;
pub const BARS: usize = 28;

pub struct Spectrum {
    fft: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    samples: Vec<f32>,
    buf: Vec<Complex32>,
    bars: [f32; BARS],
    peaks: [f32; BARS],
    last: f64,
}

impl Default for Spectrum {
    fn default() -> Self {
        let fft = FftPlanner::new().plan_fft_forward(FFT_SIZE);
        let window = (0..FFT_SIZE)
            .map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / (FFT_SIZE - 1) as f32).cos())
            .collect();
        Self {
            fft,
            window,
            samples: vec![0.0; FFT_SIZE],
            buf: vec![Complex32::default(); FFT_SIZE],
            bars: [0.0; BARS],
            peaks: [0.0; BARS],
            last: 0.0,
        }
    }
}

impl Spectrum {
    /// Recompute bar heights (0..1) from the newest samples.
    pub fn update(&mut self, player: &crate::audio::Player, now: f64, active: bool) {
        let dt = (now - self.last).clamp(0.0, 0.1) as f32;
        self.last = now;
        if active {
            player.vis_snapshot(&mut self.samples);
        } else {
            self.samples.fill(0.0);
        }
        for (i, c) in self.buf.iter_mut().enumerate() {
            *c = Complex32::new(self.samples[i] * self.window[i], 0.0);
        }
        self.fft.process(&mut self.buf);

        let rate = player.output_rate().max(8000) as f32;
        let bin_hz = rate / FFT_SIZE as f32;
        let (f_lo, f_hi) = (40.0f32, 16_000.0f32.min(rate / 2.0));
        for b in 0..BARS {
            let lo = f_lo * (f_hi / f_lo).powf(b as f32 / BARS as f32);
            let hi = f_lo * (f_hi / f_lo).powf((b + 1) as f32 / BARS as f32);
            let (i0, i1) = ((lo / bin_hz) as usize, ((hi / bin_hz) as usize).max((lo / bin_hz) as usize + 1));
            let mut m = 0.0f32;
            for c in &self.buf[i0.min(FFT_SIZE / 2)..i1.min(FFT_SIZE / 2)] {
                m = m.max(c.norm());
            }
            // Magnitude to dB, mapped onto 0..1 over a 60 dB range.
            let db = 20.0 * (m / (FFT_SIZE as f32 * 0.25) + 1e-9).log10();
            let v = ((db + 60.0) / 60.0).clamp(0.0, 1.0);
            let fall = 2.2 * dt;
            self.bars[b] = if v > self.bars[b] { v } else { (self.bars[b] - fall).max(v) };
            self.peaks[b] = if self.bars[b] > self.peaks[b] { self.bars[b] } else { (self.peaks[b] - 0.5 * dt).max(0.0) };
        }
    }

    pub fn is_silent(&self) -> bool {
        self.peaks.iter().all(|p| *p <= 0.001)
    }

    pub fn show(&self, ui: &mut Ui, skin: &Skin, size: egui::Vec2) -> egui::Response {
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
        if !ui.is_rect_visible(rect) {
            return resp;
        }
        let painter = ui.painter();
        let gap = 2.0;
        let w = (rect.width() - gap * (BARS as f32 - 1.0)) / BARS as f32;
        let top = skin.highlight;
        let bottom = skin.accent.gamma_multiply(0.45);
        for b in 0..BARS {
            let x = rect.left() + b as f32 * (w + gap);
            let h = self.bars[b] * rect.height();
            if h < 1.0 && self.peaks[b] < 0.01 {
                continue;
            }
            let bar = Rect::from_min_size(pos2(x, rect.bottom() - h), vec2(w, h));
            // Vertical gradient via a two-colour mesh.
            let mut mesh = egui::Mesh::default();
            let t = lerp_color(bottom, top, self.bars[b]);
            mesh.colored_vertex(bar.left_top(), t);
            mesh.colored_vertex(bar.right_top(), t);
            mesh.colored_vertex(bar.right_bottom(), bottom);
            mesh.colored_vertex(bar.left_bottom(), bottom);
            mesh.add_triangle(0, 1, 2);
            mesh.add_triangle(0, 2, 3);
            painter.add(mesh);
            let py = rect.bottom() - self.peaks[b] * rect.height();
            painter.rect_filled(
                Rect::from_min_size(pos2(x, py - 2.0), vec2(w, 2.0)),
                0.0,
                skin.text_bright.gamma_multiply(0.8),
            );
        }
        resp
    }
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t) as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}
