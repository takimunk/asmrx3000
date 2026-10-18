//! Basic DSP building blocks.

use core::f32::consts::PI;

/// Topology-preserving-transform state variable filter (Zavalishin).
/// Stable under fast coefficient modulation, which we need for wind.
#[derive(Clone, Copy, Default)]
pub struct Svf {
    ic1: f32,
    ic2: f32,
    k: f32,
    a1: f32,
    a2: f32,
    a3: f32,
}

impl Svf {
    pub fn new(fc: f32, q: f32, sample_rate: f32) -> Self {
        let mut f = Self::default();
        f.set(fc, q, sample_rate);
        f
    }

    pub fn set(&mut self, fc: f32, q: f32, sample_rate: f32) {
        let fc = fc.clamp(10.0, 0.49 * sample_rate);
        let g = (PI * fc / sample_rate).tan();
        self.k = 1.0 / q.max(0.05);
        self.a1 = 1.0 / (1.0 + g * (g + self.k));
        self.a2 = g * self.a1;
        self.a3 = g * self.a2;
    }

    /// Zero states that have decayed into the subnormal range.
    #[inline]
    pub fn flush(&mut self) {
        if self.ic1.abs() < 1e-18 {
            self.ic1 = 0.0;
        }
        if self.ic2.abs() < 1e-18 {
            self.ic2 = 0.0;
        }
    }

    /// Returns (lowpass, bandpass normalised to unity peak, highpass).
    #[inline]
    pub fn process(&mut self, v0: f32) -> (f32, f32, f32) {
        let v3 = v0 - self.ic2;
        let v1 = self.a1 * self.ic1 + self.a2 * v3;
        let v2 = self.ic2 + self.a2 * self.ic1 + self.a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        (v2, v1 * self.k, v0 - self.k * v1 - v2)
    }
}

/// Linear below the knee, smooth tanh saturation above it. Output stays in (-1, 1).
#[inline]
pub fn soft_clip(x: f32) -> f32 {
    const KNEE: f32 = 0.7;
    let a = x.abs();
    if a <= KNEE {
        x
    } else {
        let y = KNEE + (1.0 - KNEE) * ((a - KNEE) / (1.0 - KNEE)).tanh();
        y.copysign(x)
    }
}
