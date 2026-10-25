//! Building blocks shared by the underwater voices (whale.rs, dolphin.rs, reef.rs, sea.rs).
//!
//! Comfort comes first here: every pitch is a deterministic, smooth contour (no vibrato, no
//! jitter, no modulated delay lines), every tone is band-limited (harmonics fade out before
//! they could fold over Nyquist), every onset is ramped and the reverberation is a static
//! feedback delay network (fixed delays: it smears in time, never in pitch).

#![allow(clippy::needless_range_loop)] // parallel fixed-size arrays, indexed together

use core::f32::consts::{PI, TAU};

use super::common::{flush, Svf};

// ---------------------------------------------------------------------------------------
// Band-limited harmonic oscillator
// ---------------------------------------------------------------------------------------

/// Most harmonics a voice carries.
pub const MAX_H: usize = 20;
/// Samples between control updates of an oscillator (weights and frequency are interpolated
/// linearly in between).
pub const CTRL: usize = 16;

/// Amplitude of harmonic `k` (1-based) of `f0` against the Nyquist limit: 1 below 0.36·sr,
/// a raised-cosine fade to 0 at 0.45·sr, so no partial ever aliases, even mid-glide.
#[inline]
pub fn alias_taper(fk: f32, sr: f32) -> f32 {
    let (a, b) = (0.36 * sr, 0.45 * sr);
    if fk <= a {
        1.0
    } else if fk >= b {
        0.0
    } else {
        0.5 + 0.5 * (PI * (fk - a) / (b - a)).cos()
    }
}

/// Additive oscillator: `MAX_H` harmonics of one phase via the Chebyshev recurrence
/// sin(kφ) = 2cos φ · sin((k−1)φ) − sin((k−2)φ). Weights and phase increment glide linearly
/// over each control interval: no steps, no zipper.
#[derive(Clone, Copy)]
pub struct Harmonic {
    phase: f32,
    inc: f32,
    dinc: f32,
    w: [f32; MAX_H],
    dw: [f32; MAX_H],
    n: usize,
}

impl Default for Harmonic {
    fn default() -> Self {
        Self { phase: 0.0, inc: 0.0, dinc: 0.0, w: [0.0; MAX_H], dw: [0.0; MAX_H], n: 0 }
    }
}

impl Harmonic {
    /// Restarts at zero phase and zero amplitude (the envelope brings it in).
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Targets for the end of the next `steps` samples: fundamental `f0` (Hz) and harmonic
    /// weights `target[k-1]` (the Nyquist taper is applied here).
    pub fn glide_to(&mut self, f0: f32, target: &[f32; MAX_H], steps: usize, sr: f32) {
        let steps = steps.max(1) as f32;
        let inc = (f0 / sr).clamp(0.0, 0.49);
        if self.n == 0 && self.inc == 0.0 {
            self.inc = inc;
        }
        self.dinc = (inc - self.inc) / steps;
        let mut n = 0;
        for k in 0..MAX_H {
            let t = target[k] * alias_taper(f0 * (k + 1) as f32, sr);
            self.dw[k] = (t - self.w[k]) / steps;
            if t != 0.0 || self.w[k] != 0.0 {
                n = k + 1;
            }
        }
        self.n = n;
    }

    #[inline]
    pub fn tick(&mut self) -> f32 {
        let x = TAU * self.phase;
        let (s1, c) = x.sin_cos();
        let c2 = 2.0 * c;
        let (mut prev, mut cur) = (0.0f32, s1);
        let mut acc = 0.0;
        for k in 0..self.n {
            acc += self.w[k] * cur;
            self.w[k] += self.dw[k];
            let next = c2 * cur - prev;
            prev = cur;
            cur = next;
        }
        self.phase += self.inc;
        self.phase -= self.phase.floor();
        self.inc += self.dinc;
        acc
    }

    /// Lands exactly on the targets (call after the `steps` samples of a glide).
    pub fn settle(&mut self) {
        self.dinc = 0.0;
        for k in 0..self.n {
            self.w[k] = flush(self.w[k]);
            self.dw[k] = 0.0;
        }
    }
}

/// Harmonic weights with a spectral tilt (amplitude ∝ k^−`tilt`) and one formant-like
/// resonance (Gaussian bump of relative height `boost` at `formant_hz`, width `width_hz`),
/// normalised to unit power so loudness does not follow the pitch. Harmonics above
/// `max_hz` are left out.
pub fn harmonic_weights(f0: f32, tilt: f32, formant_hz: f32, width_hz: f32, boost: f32, max_hz: f32, n_max: usize) -> [f32; MAX_H] {
    let mut w = [0.0f32; MAX_H];
    let mut p = 0.0;
    for (k, wk) in w.iter_mut().enumerate().take(n_max.min(MAX_H)) {
        let fk = f0 * (k + 1) as f32;
        if fk > max_hz && k > 0 {
            break;
        }
        let z = (fk - formant_hz) / width_hz.max(1.0);
        let a = ((k + 1) as f32).powf(-tilt) * (1.0 + boost * (-0.5 * z * z).exp());
        *wk = a;
        p += a * a;
    }
    let g = if p > 0.0 { 1.0 / p.sqrt() } else { 0.0 };
    for v in w.iter_mut() {
        *v *= g;
    }
    w
}

// ---------------------------------------------------------------------------------------
// Contours and envelopes
// ---------------------------------------------------------------------------------------

/// Most knots of a pitch contour.
pub const KNOTS: usize = 5;

/// Interpolation between two knots.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Ease {
    /// 6u⁵ − 15u⁴ + 10u³: level at both knots (an S-shaped glide).
    #[default]
    Smooth,
    /// u²: starts level, ends moving (an accelerating sweep: whoops, chirps).
    In,
    /// 1 − (1 − u)²: starts moving, ends level.
    Out,
    /// u: a constant glide in cents per second. Held tones use it: after any straight-line
    /// detrend nothing is left that could read as a waver.
    Linear,
}

impl Ease {
    #[inline]
    fn at(self, u: f32) -> f32 {
        let u = u.clamp(0.0, 1.0);
        match self {
            Ease::Smooth => u * u * u * (u * (u * 6.0 - 15.0) + 10.0),
            Ease::In => u * u,
            Ease::Out => 1.0 - (1.0 - u) * (1.0 - u),
            Ease::Linear => u,
        }
    }
}

/// A pitch contour: knots at shares of the unit's length, interpolated in log frequency.
/// Deterministic and smooth by construction: the only way a pitch moves.
#[derive(Clone, Copy, Debug, Default)]
pub struct Contour {
    pub t: [f32; KNOTS],
    pub log2f: [f32; KNOTS],
    pub n: usize,
    pub ease: Ease,
}

impl Contour {
    /// Evenly spaced knots through `hz`.
    pub fn through(hz: &[f32], ease: Ease) -> Self {
        let n = hz.len().clamp(1, KNOTS);
        let mut c = Contour { n, ease, ..Default::default() };
        for i in 0..n {
            c.t[i] = if n > 1 { i as f32 / (n - 1) as f32 } else { 0.0 };
            c.log2f[i] = hz[i].max(1.0).log2();
        }
        c
    }

    /// Frequency at share `u` (0..1) of the unit.
    #[inline]
    pub fn at(&self, u: f32) -> f32 {
        if self.n <= 1 {
            return self.log2f[0].exp2();
        }
        let u = u.clamp(0.0, 1.0);
        let mut i = 0;
        while i + 2 < self.n && u > self.t[i + 1] {
            i += 1;
        }
        let span = (self.t[i + 1] - self.t[i]).max(1e-6);
        let x = (u - self.t[i]) / span;
        if self.n == 2 {
            let e = self.ease.at(x);
            return (self.log2f[0] + (self.log2f[1] - self.log2f[0]) * e).exp2();
        }
        // Three or more knots: monotone cubic Hermite (Fritsch–Carlson). The pitch never
        // stalls at an inner knot on its way up or down (a per-segment ease would stop it
        // there, a staircase heard as a waver) and never overshoots a knot.
        let (y0, y1) = (self.log2f[i], self.log2f[i + 1]);
        let m0 = self.slope(i) * span;
        let m1 = self.slope(i + 1) * span;
        let (x2, x3) = (x * x, x * x * x);
        let y = (2.0 * x3 - 3.0 * x2 + 1.0) * y0 + (x3 - 2.0 * x2 + x) * m0 + (-2.0 * x3 + 3.0 * x2) * y1 + (x3 - x2) * m1;
        y.exp2()
    }

    /// Fritsch–Carlson tangent at knot `k` (log2 Hz per unit of `t`): zero at a local extremum,
    /// the harmonic mean of the neighbouring secants otherwise; zero at the ends, so every
    /// unit starts and ends on a level pitch.
    fn slope(&self, k: usize) -> f32 {
        if k == 0 || k + 1 >= self.n {
            return 0.0;
        }
        let d0 = (self.log2f[k] - self.log2f[k - 1]) / (self.t[k] - self.t[k - 1]).max(1e-6);
        let d1 = (self.log2f[k + 1] - self.log2f[k]) / (self.t[k + 1] - self.t[k]).max(1e-6);
        if d0 * d1 <= 0.0 {
            0.0
        } else {
            2.0 * d0 * d1 / (d0 + d1)
        }
    }

    /// Multiplies every knot by `k` (transposition).
    pub fn scaled(mut self, k: f32) -> Self {
        let d = k.max(1e-3).log2();
        for v in self.log2f.iter_mut() {
            *v += d;
        }
        self
    }

    #[cfg(test)]
    pub fn max_hz(&self) -> f32 {
        self.log2f[..self.n.max(1)].iter().fold(f32::MIN, |m, &v| m.max(v)).exp2()
    }
}

/// Raised-cosine attack and release around a plateau, `t` and lengths in seconds.
#[inline]
pub fn envelope(t: f32, len: f32, attack: f32, release: f32) -> f32 {
    if t <= 0.0 || t >= len {
        return 0.0;
    }
    let a = attack.min(0.5 * len).max(1e-4);
    let r = release.min(0.5 * len).max(1e-4);
    let up = if t < a { (0.5 * PI * t / a).sin().powi(2) } else { 1.0 };
    let down = if t > len - r { (0.5 * PI * (len - t) / r).sin().powi(2) } else { 1.0 };
    up * down
}

// ---------------------------------------------------------------------------------------
// Filters
// ---------------------------------------------------------------------------------------

/// Two-pole (Butterworth) low-pass: the distance / softness control of a voice.
#[derive(Clone, Copy)]
pub struct Lowpass {
    svf: Svf,
}

impl Lowpass {
    pub fn new(fc: f32, sr: f32) -> Self {
        Self { svf: Svf::new(fc, core::f32::consts::FRAC_1_SQRT_2, sr) }
    }
    pub fn set(&mut self, fc: f32, sr: f32) {
        self.svf.set(fc, core::f32::consts::FRAC_1_SQRT_2, sr);
    }
    #[inline]
    pub fn tick(&mut self, x: f32) -> f32 {
        self.svf.process(x).0
    }
    pub fn flush(&mut self) {
        self.svf.flush();
    }
}

/// Two-pole (Butterworth) high-pass.
#[derive(Clone, Copy)]
pub struct Highpass {
    svf: Svf,
}

impl Highpass {
    pub fn new(fc: f32, sr: f32) -> Self {
        Self { svf: Svf::new(fc, core::f32::consts::FRAC_1_SQRT_2, sr) }
    }
    #[inline]
    pub fn tick(&mut self, x: f32) -> f32 {
        self.svf.process(x).2
    }
    pub fn flush(&mut self) {
        self.svf.flush();
    }
}

/// One-pole low-pass (gentle 6 dB/octave tilt).
#[derive(Clone, Copy, Default)]
pub struct OnePole {
    a: f32,
    y: f32,
}

impl OnePole {
    pub fn new(fc: f32, sr: f32) -> Self {
        Self { a: super::common::one_pole_coef(fc, sr), y: 0.0 }
    }
    #[inline]
    pub fn tick(&mut self, x: f32) -> f32 {
        self.y += (x - self.y) * self.a;
        self.y
    }
    pub fn flush(&mut self) {
        self.y = flush(self.y);
    }
}

/// Fixed Schroeder allpass with a short delay (up to 512 samples): smears an impulse over a
/// few milliseconds without colouring the long-term spectrum (multipath scattering).
#[derive(Clone, Copy)]
pub struct Allpass {
    buf: [f32; 512],
    len: usize,
    pos: usize,
    g: f32,
}

impl Allpass {
    pub fn new(ms: f32, g: f32, sr: f32) -> Self {
        Self { buf: [0.0; 512], len: ((ms * 1e-3 * sr) as usize).clamp(1, 512), pos: 0, g }
    }
    #[inline]
    pub fn tick(&mut self, x: f32) -> f32 {
        let d = self.buf[self.pos];
        let w = x + self.g * d;
        self.buf[self.pos] = flush(w);
        self.pos += 1;
        if self.pos == self.len {
            self.pos = 0;
        }
        d - self.g * w
    }
}

// ---------------------------------------------------------------------------------------
// Damped pulse (fish sounds, clicks)
// ---------------------------------------------------------------------------------------

/// A damped sinusoid with a smooth onset: the difference of two decaying phasors
/// (e^{−d₁t} − e^{−d₂t})·sin ωt, which starts at zero amplitude and zero slope.
#[derive(Clone, Copy, Default)]
pub struct SoftPulse {
    re: [f32; 2],
    im: [f32; 2],
    cr: [f32; 2],
    ci: [f32; 2],
}

impl SoftPulse {
    /// `rise` (s) sets the second decay (faster), `decay` (s) the first.
    pub fn strike(&mut self, freq: f32, decay: f32, rise: f32, amp: f32, sr: f32) {
        let (s, c) = (TAU * freq.clamp(1.0, 0.45 * sr) / sr).sin_cos();
        let d1 = 1.0 / decay.max(1e-4);
        let d2 = d1 + 1.0 / rise.max(1e-4);
        // Peak of e^{-d1 t} - e^{-d2 t}, so `amp` is the envelope's maximum.
        let tp = (d2 / d1).ln() / (d2 - d1);
        let peak = (-d1 * tp).exp() - (-d2 * tp).exp();
        let a = amp / peak.max(1e-6);
        for (k, d) in [d1, d2].into_iter().enumerate() {
            let g = (-d / sr).exp();
            self.cr[k] = c * g;
            self.ci[k] = s * g;
            self.re[k] += if k == 0 { a } else { -a };
        }
    }

    #[inline]
    pub fn tick(&mut self) -> f32 {
        let mut y = 0.0;
        for k in 0..2 {
            let re = self.re[k] * self.cr[k] - self.im[k] * self.ci[k];
            let im = self.re[k] * self.ci[k] + self.im[k] * self.cr[k];
            self.re[k] = re;
            self.im[k] = im;
            y += im;
        }
        y
    }

    pub fn energy(&self) -> f32 {
        self.re[0] * self.re[0] + self.im[0] * self.im[0] + self.re[1] * self.re[1] + self.im[1] * self.im[1]
    }

    pub fn flush(&mut self) {
        if self.energy() < 1e-20 {
            *self = Self::default();
        }
    }
}

// ---------------------------------------------------------------------------------------
// Static reverberation
// ---------------------------------------------------------------------------------------

const LINES: usize = 8;
/// Line lengths, ms, at size 1 (mutually prime in samples at 48 kHz, 37–83 ms).
const LINE_MS: [f32; LINES] = [37.1, 41.3, 47.9, 53.7, 61.1, 67.3, 73.9, 83.1];
const DIFF_MS: [f32; 2] = [5.3, 7.9];

/// Underwater multipath reverberation: two input allpass diffusers and an 8-line feedback
/// delay network (Hadamard mixing), damped above `damp_hz`, decaying in `t60` seconds.
/// Delays are fixed: the tail never modulates the pitch. Buffers are allocated at build.
pub struct Reverb {
    lines: Vec<f32>,
    start: [usize; LINES],
    len: [usize; LINES],
    pos: [usize; LINES],
    gain: [f32; LINES],
    lp: [f32; LINES],
    a: f32,
    diff: Vec<f32>,
    dstart: [usize; 2],
    dlen: [usize; 2],
    dpos: [usize; 2],
}

impl Reverb {
    /// `size` scales the line lengths (1..3), `t60` (s), `damp_hz` the tail's low-pass.
    pub fn new(sr: f32, size: f32, t60: f32, damp_hz: f32) -> Self {
        let size = size.clamp(0.5, 3.0);
        let mut start = [0; LINES];
        let mut len = [0; LINES];
        let mut total = 0;
        for i in 0..LINES {
            len[i] = ((LINE_MS[i] * size * 1e-3 * sr) as usize).max(8);
            start[i] = total;
            total += len[i];
        }
        let dlen = [((DIFF_MS[0] * 1e-3 * sr) as usize).max(4), ((DIFF_MS[1] * 1e-3 * sr) as usize).max(4)];
        let mut r = Self {
            lines: vec![0.0; total],
            start,
            len,
            pos: [0; LINES],
            gain: [0.0; LINES],
            lp: [0.0; LINES],
            a: 1.0,
            diff: vec![0.0; dlen[0] + dlen[1]],
            dstart: [0, dlen[0]],
            dlen,
            dpos: [0; 2],
        };
        r.set(sr, t60, damp_hz);
        r
    }

    pub fn set(&mut self, sr: f32, t60: f32, damp_hz: f32) {
        for i in 0..LINES {
            self.gain[i] = 10f32.powf(-3.0 * self.len[i] as f32 / (t60.max(0.05) * sr));
        }
        self.a = super::common::one_pole_coef(damp_hz, sr);
    }

    #[inline]
    pub fn tick(&mut self, x: f32) -> f32 {
        // Input diffusion (Schroeder allpasses, g = 0.6).
        let mut v = x;
        for k in 0..2 {
            let i = self.dstart[k] + self.dpos[k];
            let d = self.diff[i];
            let w = v + 0.6 * d;
            self.diff[i] = w;
            v = d - 0.6 * w;
            self.dpos[k] += 1;
            if self.dpos[k] == self.dlen[k] {
                self.dpos[k] = 0;
            }
        }
        let mut o = [0.0f32; LINES];
        for i in 0..LINES {
            let y = self.lines[self.start[i] + self.pos[i]];
            self.lp[i] += (y - self.lp[i]) * self.a;
            o[i] = self.lp[i] * self.gain[i];
        }
        // Fast Walsh–Hadamard transform, scaled to stay unitary.
        let mut h = 1;
        while h < LINES {
            let mut j = 0;
            while j < LINES {
                for k in j..j + h {
                    let (a, b) = (o[k], o[k + h]);
                    o[k] = a + b;
                    o[k + h] = a - b;
                }
                j += 2 * h;
            }
            h *= 2;
        }
        let s = 1.0 / (LINES as f32).sqrt();
        let mut out = 0.0;
        for i in 0..LINES {
            let sign = if i & 1 == 0 { 1.0 } else { -1.0 };
            let idx = self.start[i] + self.pos[i];
            out += sign * self.lines[idx];
            self.lines[idx] = o[i] * s + v * sign * s;
            self.pos[i] += 1;
            if self.pos[i] == self.len[i] {
                self.pos[i] = 0;
            }
        }
        out * s
    }

    pub fn flush(&mut self) {
        for v in self.lp.iter_mut() {
            *v = flush(*v);
        }
    }
}

/// Sanity of the shared blocks.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn harmonic_oscillator_never_puts_energy_near_nyquist() {
        let sr = 48_000.0;
        let mut h = Harmonic::default();
        let w = [1.0f32; MAX_H];
        // Glide from 2 kHz to 9 kHz with 20 equal harmonics: everything above 0.45·sr is gone.
        let n = 48_000;
        let mut x = vec![0.0f32; n];
        let mut i = 0;
        while i < n {
            let f0 = 2000.0 + 7000.0 * (i as f32 / n as f32);
            h.glide_to(f0, &w, CTRL, sr);
            for _ in 0..CTRL {
                x[i] = h.tick();
                i += 1;
            }
            h.settle();
        }
        // Goertzel-free check: energy of the last 4096 samples above 21.6 kHz via a DFT band.
        let seg = &x[n - 4096..];
        let mut hi = 0.0f64;
        let mut tot = 0.0f64;
        for b in 1..2048 {
            let f = b as f64 * sr as f64 / 4096.0;
            let (mut re, mut im) = (0.0f64, 0.0f64);
            for (j, &v) in seg.iter().enumerate() {
                let wv = 0.5 - 0.5 * (core::f64::consts::TAU * j as f64 / 4096.0).cos();
                let p = core::f64::consts::TAU * b as f64 * j as f64 / 4096.0;
                re += v as f64 * wv * p.cos();
                im += v as f64 * wv * p.sin();
            }
            let e = re * re + im * im;
            tot += e;
            if f > 21_600.0 {
                hi += e;
            }
        }
        assert!(hi / tot < 1e-4, "energy near Nyquist {:.2e}", hi / tot);
    }

    #[test]
    fn contour_is_monotone_between_knots_and_hits_them() {
        // A rising contour through an inner knot keeps rising there (no stall, no staircase).
        let up = Contour::through(&[200.0, 300.0, 600.0], Ease::Smooth);
        let d = |u: f32| up.at(u + 0.01) / up.at(u);
        assert!(d(0.495) > 1.004, "stalls at the inner knot: {}", d(0.495));
        let c = Contour::through(&[200.0, 400.0, 300.0], Ease::Smooth);
        assert!((c.at(0.0) - 200.0).abs() < 1e-2 && (c.at(0.5) - 400.0).abs() < 1e-2 && (c.at(1.0) - 300.0).abs() < 1e-2);
        let mut prev = c.at(0.0);
        for i in 1..=50 {
            let f = c.at(i as f32 / 100.0);
            assert!(f >= prev - 1e-3);
            prev = f;
        }
    }

    #[test]
    fn soft_pulse_starts_silent() {
        let mut p = SoftPulse::default();
        p.strike(400.0, 0.01, 0.002, 1.0, 48_000.0);
        let first = p.tick().abs();
        let mut peak = 0.0f32;
        for _ in 0..2000 {
            peak = peak.max(p.tick().abs());
        }
        assert!(first < 0.02 * peak, "onset {first} vs peak {peak}");
        assert!(peak > 0.5 && peak < 1.2, "peak {peak}");
    }

    #[test]
    fn reverb_decays_and_stays_bounded() {
        let sr = 48_000.0;
        let mut r = Reverb::new(sr, 1.5, 2.0, 3000.0);
        let mut e_early = 0.0;
        let mut e_late = 0.0;
        for i in 0..(4.0 * sr) as usize {
            let y = r.tick(if i == 0 { 1.0 } else { 0.0 });
            assert!(y.is_finite() && y.abs() < 2.0);
            if (4800..24_000).contains(&i) {
                e_early += y * y;
            }
            if i >= 3 * 48_000 {
                e_late += y * y;
            }
        }
        // ~60 dB per 2 s: the last second is far below the first half second.
        assert!(e_late < 1e-3 * e_early, "{e_late} vs {e_early}");
    }
}
