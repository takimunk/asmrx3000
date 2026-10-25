//! Propagation delay line with band-limited fractional reads.
//!
//! The delay of every path is the retarded time r/c, so a source that moves makes the read
//! position slide and Doppler shift, frequency-dependent with no special-casing, falls out of
//! the physics. Reads use an 8-tap Kaiser-windowed sinc (256 phases, linearly interpolated):
//! flat to ~18 kHz at 48 kHz and free of the modulation noise linear interpolation produces
//! when the fractional position moves. Early-reflection taps use cheap linear interpolation.

use core::f64::consts::PI;
use std::sync::OnceLock;

use crate::simd::F4;

pub const TAPS: usize = 8;
const HALF: i64 = 3; // taps cover n = -3..=4 around floor(position)
const PHASES: usize = 256;
const CUTOFF: f64 = 0.92; // fraction of Nyquist
const KAISER_BETA: f64 = 7.0;

/// Minimum delay (samples) a sinc read needs so every tap is already written.
pub const MIN_DELAY: f64 = 5.0;

fn bessel_i0(x: f64) -> f64 {
    let mut sum = 1.0;
    let mut term = 1.0;
    for k in 1..50 {
        term *= (x / (2.0 * k as f64)).powi(2);
        sum += term;
        if term < 1e-14 * sum {
            break;
        }
    }
    sum
}

/// Per phase: 8 taps followed by their differences to the next phase.
fn table() -> &'static [[f32; 2 * TAPS]] {
    static T: OnceLock<Vec<[f32; 2 * TAPS]>> = OnceLock::new();
    T.get_or_init(|| {
        let span = TAPS as f64 / 2.0 + 0.5;
        let taps = |p: usize| -> [f32; TAPS] {
            let mu = p as f64 / PHASES as f64;
            let mut h = [0.0f64; TAPS];
            for (k, v) in h.iter_mut().enumerate() {
                let t = (k as i64 - HALF) as f64 - mu;
                let sinc = if t.abs() < 1e-9 { 1.0 } else { (PI * CUTOFF * t).sin() / (PI * CUTOFF * t) };
                let r = (t / span).clamp(-1.0, 1.0);
                *v = CUTOFF * sinc * bessel_i0(KAISER_BETA * (1.0 - r * r).sqrt()) / bessel_i0(KAISER_BETA);
            }
            let dc: f64 = h.iter().sum();
            h.map(|v| (v / dc) as f32)
        };
        (0..PHASES)
            .map(|p| {
                let (a, b) = (taps(p), taps(p + 1));
                core::array::from_fn(|k| if k < TAPS { a[k] } else { b[k - TAPS] - a[k - TAPS] })
            })
            .collect()
    })
}

/// Builds the shared interpolation table (call once off the audio thread).
pub fn init() {
    table();
}

pub struct DelayLine {
    buf: Vec<f32>,
    mask: usize,
    /// Absolute index of the next sample to be written.
    w: usize,
    /// Samples written since the buffer was last known to be all zeros (saturates at its
    /// length): what `clear` has to wipe.
    dirty: usize,
}

impl DelayLine {
    /// `max_delay` in samples; the buffer is the next power of two above it plus the taps.
    pub fn new(max_delay: usize) -> Self {
        let n = (max_delay + 2 * TAPS + 512).next_power_of_two();
        Self { buf: vec![0.0; n], mask: n - 1, w: 0, dirty: 0 }
    }

    /// Largest delay (samples) a read may ask for.
    pub fn max_delay(&self) -> f64 {
        (self.buf.len() - 2 * TAPS - 512) as f64
    }

    /// Appends a block. Returns the absolute index of its first sample.
    pub fn write(&mut self, x: &[f32]) -> usize {
        let start = self.w;
        for (j, &v) in x.iter().enumerate() {
            self.buf[(start + j) & self.mask] = v;
        }
        self.w += x.len();
        self.dirty = (self.dirty + x.len()).min(self.buf.len());
        start
    }

    /// Zeroes everything written since the buffer was last clear, so `skip` can follow (no
    /// allocation; at most one pass over the buffer). Without it a sleeping path that wakes
    /// reads its first delay's worth of samples from whatever was written there long before:
    /// a burst of stale sound (heard as a click when a scene's sound came back in).
    pub fn clear(&mut self) {
        for j in 1..=self.dirty {
            self.buf[self.w.wrapping_sub(j) & self.mask] = 0.0;
        }
        self.dirty = 0;
    }

    /// Advances over a block of silence without touching memory (valid only once the whole
    /// buffer is known to be zero: `clear`).
    pub fn skip(&mut self, n: usize) -> usize {
        let start = self.w;
        self.w += n;
        start
    }

    /// Read positions for a block as 32.32 fixed point relative to `start`: sample j sits at
    /// start + j - (d0 + step (j+1)). Integer stepping keeps the per-sample cost low.
    #[inline(always)]
    fn positions(n: usize, d0: f64, d1: f64, min: f64) -> (i64, i64) {
        let (d0, d1) = (d0.max(min), d1.max(min));
        let step = (d1 - d0) / n as f64;
        const ONE: f64 = 4_294_967_296.0;
        (((-d0 - step) * ONE) as i64, ((1.0 - step) * ONE) as i64)
    }

    /// Band-limited read for a block whose first sample has absolute index `start`. The delay
    /// ramps linearly from `d0` (previous block end) to `d1` (this block end), in samples.
    pub fn read_sinc(&self, start: usize, d0: f64, d1: f64, out: &mut [f32]) {
        let tab = table();
        let (mut p, rate) = Self::positions(out.len(), d0, d1, MIN_DELAY);
        let len = self.buf.len();
        const ONE: i64 = 1 << 32;
        let n = out.len();
        let first = start.wrapping_add((p >> 32) as usize).wrapping_sub(HALF as usize) & self.mask;
        if rate == ONE && first + n + TAPS <= len {
            // Constant delay over the block: one fixed FIR over contiguous samples.
            let frac = p as u32;
            let e = &tab[(frac >> 24) as usize];
            let t = ((frac >> 8) & 0xFFFF) as f32 * (1.0 / 65_536.0);
            let h: [f32; TAPS] = core::array::from_fn(|k| e[k] + e[TAPS + k] * t);
            let x = &self.buf[first..first + n + TAPS];
            let mut j = 0;
            while j + 4 <= n {
                let mut acc = F4::zero();
                for (k, hk) in h.iter().enumerate() {
                    acc = acc.fma(F4::splat(*hk), F4::load(&x[j + k..]));
                }
                acc.store(&mut out[j..]);
                j += 4;
            }
            for jj in j..n {
                out[jj] = (0..TAPS).map(|k| h[k] * x[jj + k]).sum();
            }
            return;
        }
        for o in out.iter_mut() {
            let i = start.wrapping_add((p >> 32) as usize);
            let frac = p as u32;
            let ph = (frac >> 24) as usize;
            let t = ((frac >> 8) & 0xFFFF) as f32 * (1.0 / 65_536.0);
            let e = &tab[ph];
            let base = i.wrapping_sub(HALF as usize) & self.mask;
            // Taps linearly interpolated between adjacent phases: h + dh t.
            let tv = F4::splat(t);
            let h0 = F4::load(&e[..4]).fma(F4::load(&e[8..12]), tv);
            let h1 = F4::load(&e[4..8]).fma(F4::load(&e[12..]), tv);
            *o = if base + TAPS <= len {
                let x = &self.buf[base..base + TAPS];
                h0.mul(F4::load(&x[..4])).fma(h1, F4::load(&x[4..])).sum()
            } else {
                let x: [f32; TAPS] = core::array::from_fn(|k| self.buf[(base + k) & self.mask]);
                h0.mul(F4::load(&x[..4])).fma(h1, F4::load(&x[4..])).sum()
            };
            p += rate;
        }
    }

    /// Linearly interpolated read, added into `out` with a gain ramping from `g0` to `g1`
    /// (cheap; used for the many early-reflection taps).
        pub fn read_linear_add(&self, start: usize, d0: f64, d1: f64, g0: f32, g1: f32, out: &mut [f32]) {
        let n = out.len();
        let (mut p, rate) = Self::positions(n, d0, d1, 2.0);
        let gstep = (g1 - g0) / n as f32;
        let first = start.wrapping_add((p >> 32) as usize) & self.mask;
        if rate == 1 << 32 && first + n + 1 <= self.buf.len() {
            let m = F4::splat((p as u32 >> 8) as f32 * (1.0 / 16_777_216.0));
            let x = &self.buf[first..first + n + 1];
            let ramp = F4::load(&[1.0, 2.0, 3.0, 4.0]);
            let mut g = F4::splat(g0).fma(F4::splat(gstep), ramp);
            let dg = F4::splat(4.0 * gstep);
            let mut j = 0;
            while j + 4 <= n {
                let x0 = F4::load(&x[j..]);
                let x1 = F4::load(&x[j + 1..]);
                let v = x0.fma(x1.fms(F4::splat(1.0), x0), m);
                F4::load(&out[j..]).fma(g, v).store(&mut out[j..]);
                g = g.add(dg);
                j += 4;
            }
            let m = (p as u32 >> 8) as f32 * (1.0 / 16_777_216.0);
            for jj in j..n {
                out[jj] += (g0 + gstep * (jj + 1) as f32) * (x[jj] + (x[jj + 1] - x[jj]) * m);
            }
            return;
        }
        for (j, o) in out.iter_mut().enumerate() {
            let i = start.wrapping_add((p >> 32) as usize);
            let m = (p as u32 >> 8) as f32 * (1.0 / 16_777_216.0);
            let x0 = self.buf[i & self.mask];
            let x1 = self.buf[i.wrapping_add(1) & self.mask];
            *o += (g0 + gstep * (j + 1) as f32) * (x0 + (x1 - x0) * m);
            p += rate;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_and_fractional_delays_are_exact_for_low_frequencies() {
        init();
        let mut dl = DelayLine::new(4096);
        let f = 500.0 / 48_000.0;
        let x: Vec<f32> = (0..2048).map(|i| (2.0 * PI * f * i as f64).sin() as f32).collect();
        let start = dl.write(&x);
        let mut out = vec![0.0f32; 64];
        let d = 100.37;
        // Read the last 64 samples' worth at a constant delay.
        dl.read_sinc(start + 1984, d, d, &mut out);
        for (j, v) in out.iter().enumerate() {
            let want = (2.0 * PI * f * ((1984 + j) as f64 - d)).sin();
            assert!((*v as f64 - want).abs() < 2e-3, "j={j}: {v} vs {want}");
        }
        let mut acc = vec![0.0f32; 64];
        dl.read_linear_add(start + 1984, d, d, 1.0, 1.0, &mut acc);
        for (j, v) in acc.iter().enumerate() {
            let want = (2.0 * PI * f * ((1984 + j) as f64 - d)).sin();
            assert!((*v as f64 - want).abs() < 5e-3, "linear j={j}: {v} vs {want}");
        }
    }

    #[test]
    fn a_cleared_line_reads_silence_after_skipping() {
        init();
        // Sound, then a sleep (clear + skips past a buffer's length), then a wake: the reads
        // before the new writes must be silent, not what was there before.
        let mut dl = DelayLine::new(4096);
        let x: Vec<f32> = (0..3000).map(|i| (i as f32 * 0.1).sin()).collect();
        dl.write(&x);
        dl.write(&[0.0; 600]);
        dl.clear();
        // 103 blocks: the ring (8192) wraps so that the read lands on the old sound.
        for _ in 0..103 {
            dl.skip(64);
        }
        let start = dl.write(&[0.0; 64]);
        let mut out = vec![1.0f32; 64];
        dl.read_sinc(start, 900.0, 900.0, &mut out);
        assert!(out.iter().all(|v| *v == 0.0), "stale samples: {:?}", &out[..4]);
    }

    #[test]
    fn sinc_passband_is_flat() {
        init();
        // Response of the 0.5-sample-delay filter at 16 kHz / 48 kHz.
        let h = &table()[PHASES / 2][..TAPS];
        let w = 2.0 * PI * 16_000.0 / 48_000.0;
        let (mut re, mut im) = (0.0f64, 0.0f64);
        for (k, v) in h.iter().enumerate() {
            re += *v as f64 * (w * k as f64).cos();
            im -= *v as f64 * (w * k as f64).sin();
        }
        let db = 10.0 * (re * re + im * im).log10();
        assert!(db > -3.0, "16 kHz at mu=0.5: {db} dB");
    }
}
