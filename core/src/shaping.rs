//! Per-path spectral shaping with a fixed-pole parallel filter (B. Bank, "Direct design of
//! parallel second-order filters", 2007; "Logarithmic frequency scale parallel filter design
//! with complex and magnitude-only specifications", IEEE SPL 2011).
//!
//! Every propagation effect that colours a path (air absorption, ground reflection
//! coefficient, diffraction, forest scattering, foliage, directivity) is multiplied into one
//! complex target response on a log-spaced frequency grid, then fitted by
//!     H(z) = d + sum_k [ c_k Re + e_k Im ] 1 / (1 - p_k z^-1),
//! with K fixed complex poles p_k spread logarithmically over the audio band. Because the
//! poles are fixed:
//!  * the fit is linear least squares, i.e. one precomputed matrix-vector product per update;
//!  * every coefficient set is stable, and interpolating coefficients interpolates the
//!    frequency response exactly (the output is a linear combination of fixed basis signals),
//!    so fast source motion cannot produce zipper noise or transient instability;
//!  * cost is K complex one-pole recursions (4 MAC) + 2K+1 MAC per sample.
//! Magnitude-only effects get their minimum phase from a precomputed discrete Hilbert operator
//! on the same grid, so one fit captures exact complex effects (ground) and min-phase ones.

use crate::num::{fft, pinv, C64};
use crate::engine::BLOCK;
use crate::simd::F4;
use core::f64::consts::PI;

/// Fixed pole pairs.
pub const K: usize = 12;
/// Fit-grid frequencies.
pub const NF: usize = 40;
const NP: usize = 2 * K + 1;
/// Least-squares weighting profiles (see `Design::new`).
const PROFILES: usize = 12;

/// The log-spaced fit grid (20 Hz .. min(0.45 fs, 20 kHz)) of `Design::freqs`.
pub fn grid_freqs(sample_rate: f32) -> [f64; NF] {
    let f_hi = (0.45 * sample_rate as f64).min(20_000.0);
    let mut freqs = [0.0f64; NF];
    for (i, f) in freqs.iter_mut().enumerate() {
        *f = 20.0 * (f_hi / 20.0).powf(i as f64 / (NF - 1) as f64);
    }
    freqs
}

pub struct Design {
    pub freqs: [f64; NF],
    pole_re: [f32; K],
    pole_im: [f32; K],
    /// (2K+1) x (2 NF) least-squares operator.
    fit: Vec<f64>,
    /// Corner frequency f_j of each weighting profile (inf = uniform).
    profile_hz: [f64; PROFILES],
    /// NF x NF: minimum phase (rad) from natural-log magnitude.
    minphase: Vec<f64>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Coeffs {
    pub c: [f32; K],
    pub e: [f32; K],
    pub d: f32,
}

impl Coeffs {
    pub fn identity() -> Self {
        Self { c: [0.0; K], e: [0.0; K], d: 1.0 }
    }
}

impl Design {
    pub fn new(sample_rate: f32) -> Self {
        let fs = sample_rate as f64;
        let freqs = grid_freqs(sample_rate);

        // Poles: log-spaced, bandwidth from the spacing (Bank 2007).
        let p_lo = 30.0;
        let p_hi = (0.42 * fs).min(18_000.0);
        let theta: Vec<f64> = (0..K).map(|k| 2.0 * PI * p_lo * (p_hi / p_lo).powf(k as f64 / (K - 1) as f64) / fs).collect();
        let mut poles = [C64::ZERO; K];
        for k in 0..K {
            let dt = if k == 0 {
                theta[1] - theta[0]
            } else if k == K - 1 {
                theta[K - 1] - theta[K - 2]
            } else {
                0.5 * (theta[k + 1] - theta[k - 1])
            };
            poles[k] = C64::from_polar((-0.5 * dt).exp(), theta[k]);
        }

        // Basis responses at the grid: columns [c_0..c_K, e_0..e_K, d].
        let mut a = vec![vec![0.0f64; NP]; 2 * NF];
        for (i, f) in freqs.iter().enumerate() {
            let zinv = C64::from_polar(1.0, -2.0 * PI * f / fs);
            for k in 0..K {
                let g = (C64::ONE - poles[k] * zinv).inv();
                let gc = (C64::ONE - poles[k].conj() * zinv).inv();
                let re_part = (g + gc).scale(0.5);
                let im_part = (g - gc) * C64::new(0.0, -0.5);
                a[2 * i][k] = re_part.re;
                a[2 * i + 1][k] = re_part.im;
                a[2 * i][K + k] = im_part.re;
                a[2 * i + 1][K + k] = im_part.im;
            }
            a[2 * i][2 * K] = 1.0;
        }
        // One LS operator per weighting profile. Profile 0 is uniform (absolute error);
        // profile j>0 weights by 1/|T_j|^2 for a Gaussian low-pass T_j(f) = exp(-(f/f_j)^2)
        // (the shape of air absorption), i.e. it minimises *relative* error for targets that
        // roll off like that, so a -60 dB stop band is not swamped by absolute-error slack.
        let mut fit = vec![0.0f64; PROFILES * NP * 2 * NF];
        let mut profile_hz = [0.0f64; PROFILES];
        for j in 0..PROFILES {
            let fj = if j == 0 {
                f64::INFINITY
            } else {
                1500.0 * 32.0f64.powf((j - 1) as f64 / (PROFILES - 2) as f64)
            };
            profile_hz[j] = fj;
            let w: Vec<f64> = (0..2 * NF)
                .map(|r| {
                    let t = (-(freqs[r / 2] / fj).powi(2)).exp().max(1e-3);
                    1.0 / (t * t)
                })
                .collect();
            let p = pinv(&a, Some(&w), 1e-7);
            for r in 0..NP {
                for c in 0..2 * NF {
                    fit[(j * NP + r) * 2 * NF + c] = p[r][c];
                }
            }
        }

        Self {
            freqs,
            pole_re: poles.map(|p| p.re as f32),
            pole_im: poles.map(|p| p.im as f32),
            fit,
            profile_hz,
            minphase: minphase_operator(&freqs, fs),
        }
    }

    /// Minimum phase (rad) for a natural-log magnitude response on the grid.
    pub fn min_phase(&self, ln_mag: &[f64; NF], out: &mut [f64; NF]) {
        for i in 0..NF {
            let row = &self.minphase[i * NF..(i + 1) * NF];
            out[i] = row.iter().zip(ln_mag).map(|(m, l)| m * l).sum();
        }
    }

    /// Least-squares fit of a complex target response on the grid.
    pub fn fit(&self, target: &[C64; NF]) -> Coeffs {
        // Weighting profile from where the target first falls 20 dB below its low-frequency
        // level: exp(-(f/f_j)^2) = 0.1 at f = 1.517 f_j.
        let reference = target[0].abs().max(target[1].abs()).max(1e-9);
        let mut j = 0;
        if let Some(i) = (0..NF).find(|&i| target[i].abs() < 0.1 * reference) {
            let fj = self.freqs[i] / 1.517;
            let dist = |a: usize| (self.profile_hz[a] / fj).ln().abs();
            j = (1..PROFILES).min_by(|&a, &b| dist(a).total_cmp(&dist(b))).unwrap_or(0);
        }
        let mut out = Coeffs::default();
        for r in 0..NP {
            let row = &self.fit[(j * NP + r) * 2 * NF..(j * NP + r + 1) * 2 * NF];
            let mut s = 0.0;
            for i in 0..NF {
                s += row[2 * i] * target[i].re + row[2 * i + 1] * target[i].im;
            }
            let s = s as f32;
            if r < K {
                out.c[r] = s;
            } else if r < 2 * K {
                out.e[r - K] = s;
            } else {
                out.d = s;
            }
        }
        out
    }

    /// Fit of exp(ln_mag + i (minphase(ln_mag) + extra_phase)) times an optional complex factor.
    pub fn fit_min_phase(&self, ln_mag: &[f64; NF], factor: Option<&[C64; NF]>) -> Coeffs {
        let mut ph = [0.0f64; NF];
        self.min_phase(ln_mag, &mut ph);
        let mut t = [C64::ZERO; NF];
        for i in 0..NF {
            t[i] = C64::from_polar(ln_mag[i].exp(), ph[i]);
            if let Some(f) = factor {
                t[i] = t[i] * f[i];
            }
        }
        self.fit(&t)
    }

    /// Response of a coefficient set at frequency f (for tests and diagnostics).
    pub fn response(&self, co: &Coeffs, f: f64, fs: f64) -> C64 {
        let zinv = C64::from_polar(1.0, -2.0 * PI * f / fs);
        let mut h = C64::new(co.d as f64, 0.0);
        for k in 0..K {
            let p = C64::new(self.pole_re[k] as f64, self.pole_im[k] as f64);
            let g = (C64::ONE - p * zinv).inv();
            let gc = (C64::ONE - p.conj() * zinv).inv();
            h = h + (g + gc).scale(0.5 * co.c[k] as f64) + (g - gc) * C64::new(0.0, -0.5 * co.e[k] as f64);
        }
        h
    }
}

/// Linear operator log|H| -> arg H_min on the grid via the real cepstrum on a fine FFT grid.
/// Log-magnitude between grid points is interpolated linearly in log frequency and held
/// constant outside the grid.
fn minphase_operator(freqs: &[f64; NF], fs: f64) -> Vec<f64> {
    const N: usize = 16384;
    let mut op = vec![0.0f64; NF * NF];
    let lf: Vec<f64> = freqs.iter().map(|f| f.ln()).collect();
    let hat = |j: usize, f: f64| -> f64 {
        let x = f.max(1e-3).ln();
        if x <= lf[0] {
            return (j == 0) as u8 as f64;
        }
        if x >= lf[NF - 1] {
            return (j == NF - 1) as u8 as f64;
        }
        let left = if j > 0 { lf[j - 1] } else { f64::NEG_INFINITY };
        let right = if j + 1 < NF { lf[j + 1] } else { f64::INFINITY };
        if x >= left && x <= lf[j] && j > 0 {
            (x - left) / (lf[j] - left)
        } else if x >= lf[j] && x <= right && j + 1 < NF {
            (right - x) / (right - lf[j])
        } else {
            0.0
        }
    };
    let mut buf = vec![C64::ZERO; N];
    for j in 0..NF {
        for b in 0..=N / 2 {
            let v = hat(j, b as f64 * fs / N as f64);
            buf[b] = C64::new(v, 0.0);
            if b > 0 && b < N / 2 {
                buf[N - b] = C64::new(v, 0.0);
            }
        }
        fft(&mut buf, true); // real cepstrum
        for n in 1..N / 2 {
            buf[n] = buf[n].scale(2.0);
        }
        for v in buf.iter_mut().skip(N / 2 + 1) {
            *v = C64::ZERO;
        }
        buf[0] = C64::new(buf[0].re, 0.0);
        buf[N / 2] = C64::new(buf[N / 2].re, 0.0);
        fft(&mut buf, false);
        for (i, f) in freqs.iter().enumerate() {
            let x = f * N as f64 / fs;
            let b0 = x.floor() as usize;
            let t = x - b0 as f64;
            op[i * NF + j] = buf[b0].im * (1.0 - t) + buf[b0 + 1].im * t;
        }
    }
    op
}

/// Lanes of 4 sections.
const KV: usize = K / 4;

fn lanes(v: &[f32; K]) -> [F4; KV] {
    core::array::from_fn(|c| F4::load(&v[4 * c..4 * c + 4]))
}

/// A single parallel filter (lanes = sections), for one-off signals such as the reverb input.
/// Per-path filtering uses the batched `FilterBank`.
#[derive(Clone)]
pub struct ParallelFilter {
    pr: [F4; KV],
    pi: [F4; KV],
    sr: [F4; KV],
    si: [F4; KV],
    co: Coeffs,
    target: Coeffs,
    blocks_left: u32,
}

/// Per-block coefficient start and per-sample increment.
struct Ramp {
    c0: [F4; KV],
    e0: [F4; KV],
    dc: [F4; KV],
    de: [F4; KV],
    d0: f32,
    dd: f32,
}

impl ParallelFilter {
    pub fn new(design: &Design) -> Self {
        Self {
            pr: lanes(&design.pole_re),
            pi: lanes(&design.pole_im),
            sr: [F4::zero(); KV],
            si: [F4::zero(); KV],
            co: Coeffs::identity(),
            target: Coeffs::identity(),
            blocks_left: 0,
        }
    }

    /// Jump to a coefficient set (no ramp).
    pub fn set(&mut self, co: Coeffs) {
        self.co = co;
        self.target = co;
        self.blocks_left = 0;
    }

    fn begin(&mut self, len: usize) -> Ramp {
        let c0 = self.co;
        let end = if self.blocks_left > 0 {
            let f = 1.0 / self.blocks_left as f32;
            let mut e = c0;
            for k in 0..K {
                e.c[k] += (self.target.c[k] - c0.c[k]) * f;
                e.e[k] += (self.target.e[k] - c0.e[k]) * f;
            }
            e.d += (self.target.d - c0.d) * f;
            self.blocks_left -= 1;
            if self.blocks_left == 0 {
                e = self.target;
            }
            e
        } else {
            c0
        };
        self.co = end;
        let inv = 1.0 / len.max(1) as f32;
        let dc: [f32; K] = core::array::from_fn(|k| (end.c[k] - c0.c[k]) * inv);
        let de: [f32; K] = core::array::from_fn(|k| (end.e[k] - c0.e[k]) * inv);
        Ramp { c0: lanes(&c0.c), e0: lanes(&c0.e), dc: lanes(&dc), de: lanes(&de), d0: c0.d, dd: (end.d - c0.d) * inv }
    }

    /// Filters `buf` in place.
    pub fn process(&mut self, buf: &mut [f32]) {
        let r = self.begin(buf.len());
        let (mut sr, mut si) = (self.sr, self.si);
        for (i, x) in buf.iter_mut().enumerate() {
            *x = tick(&self.pr, &self.pi, &r, &mut sr, &mut si, *x, (i + 1) as f32);
        }
        self.sr = sr;
        self.si = si;
    }
}

/// One sample: K complex one-pole recursions (independent across k, 4 per SIMD lane group)
/// and the interpolated linear combination of their outputs. State lives in registers.
#[inline(always)]
fn tick(pr: &[F4; KV], pi: &[F4; KV], r: &Ramp, sr: &mut [F4; KV], si: &mut [F4; KV], x: f32, t: f32) -> f32 {
    let xv = F4::splat(x);
    let tv = F4::splat(t);
    let mut acc = F4::zero();
    for c in 0..KV {
        let (a, b) = (sr[c], si[c]);
        let nr = xv.fma(pr[c], a).fms(pi[c], b);
        let ni = pi[c].mul(a).fma(pr[c], b);
        sr[c] = nr;
        si[c] = ni;
        acc = acc.fma(r.c0[c].fma(tv, r.dc[c]), nr).fma(r.e0[c].fma(tv, r.de[c]), ni);
    }
    (r.d0 + t * r.dd) * x + acc.sum()
}


/// Coefficient state of one path inside a `FilterBank`: current set, target, ramp length.
#[derive(Clone, Copy)]
pub struct CoeffRamp {
    co: Coeffs,
    target: Coeffs,
    blocks_left: u32,
}

impl CoeffRamp {
    pub fn new() -> Self {
        Self { co: Coeffs::identity(), target: Coeffs::identity(), blocks_left: 0 }
    }

    pub fn set(&mut self, co: Coeffs) {
        self.co = co;
        self.target = co;
        self.blocks_left = 0;
    }

    /// Ramp linearly to `co` over the next `blocks` blocks.
    pub fn ramp_to(&mut self, co: Coeffs, blocks: u32) {
        if blocks == 0 {
            return self.set(co);
        }
        self.target = co;
        self.blocks_left = blocks;
    }

    /// Jumps to the target (a path whose filter state is cleared: nothing to glide over).
    pub fn settle(&mut self) {
        self.set(self.target);
    }

    /// Advances one block: returns (start, end) coefficient sets.
    fn advance(&mut self) -> (Coeffs, Coeffs) {
        let c0 = self.co;
        if self.blocks_left == 0 {
            return (c0, c0);
        }
        let f = 1.0 / self.blocks_left as f32;
        let mut e = c0;
        for k in 0..K {
            e.c[k] += (self.target.c[k] - c0.c[k]) * f;
            e.e[k] += (self.target.e[k] - c0.e[k]) * f;
        }
        e.d += (self.target.d - c0.d) * f;
        self.blocks_left -= 1;
        if self.blocks_left == 0 {
            e = self.target;
        }
        self.co = e;
        (c0, e)
    }
}

impl Default for CoeffRamp {
    fn default() -> Self {
        Self::new()
    }
}

/// Parallel filters of many paths, batched four paths per SIMD group (lanes = paths). All
/// paths share the fixed poles, so the K section recursions of four paths run side by side;
/// unlike lanes = sections, this hides the recursion latency and keeps the units busy.
/// Usage per block: write inputs with `input(path)`, call `process_group` for every group
/// (passing the four paths' `CoeffRamp`s), read outputs with `output(path)`.
pub struct FilterBank {
    pr: [f32; K],
    pi: [f32; K],
    sr: Vec<[F4; K]>,
    si: Vec<[F4; K]>,
    /// io[group][sample][lane]
    io: Vec<[[f32; 4]; BLOCK]>,
}

impl FilterBank {
    pub fn new(design: &Design, paths: usize) -> Self {
        let groups = paths.div_ceil(4);
        Self {
            pr: design.pole_re,
            pi: design.pole_im,
            sr: vec![[F4::zero(); K]; groups],
            si: vec![[F4::zero(); K]; groups],
            io: vec![[[0.0; 4]; BLOCK]; groups],
        }
    }

    /// Grows to `paths` (allocates: not on the audio thread).
    pub fn ensure(&mut self, paths: usize) {
        let groups = paths.div_ceil(4);
        while self.io.len() < groups {
            self.sr.push([F4::zero(); K]);
            self.si.push([F4::zero(); K]);
            self.io.push([[0.0; 4]; BLOCK]);
        }
    }

    pub fn write(&mut self, path: usize, x: &[f32]) {
        let (g, l) = (path / 4, path % 4);
        for (i, v) in x.iter().enumerate() {
            self.io[g][i][l] = *v;
        }
    }

    pub fn read(&self, path: usize, out: &mut [f32]) {
        let (g, l) = (path / 4, path % 4);
        for (i, o) in out.iter_mut().enumerate() {
            *o = self.io[g][i][l];
        }
    }

    /// Zeroes a path's input for this block (inactive emitter).
    pub fn silence(&mut self, path: usize, len: usize) {
        let (g, l) = (path / 4, path % 4);
        for frame in self.io[g][..len].iter_mut() {
            frame[l] = 0.0;
        }
    }

    pub fn reset(&mut self, path: usize) {
        let (g, l) = (path / 4, path % 4);
        for k in 0..K {
            let mut a = [0.0f32; 4];
            let mut b = [0.0f32; 4];
            self.sr[g][k].store(&mut a);
            self.si[g][k].store(&mut b);
            a[l] = 0.0;
            b[l] = 0.0;
            self.sr[g][k] = F4::load(&a);
            self.si[g][k] = F4::load(&b);
        }
    }

    /// Filters group `g` in place. `ramps[l]` is the coefficient state of lane l (None = an
    /// unused lane, filtered with zero coefficients).
    pub fn process_group(&mut self, g: usize, len: usize, ramps: [Option<&mut CoeffRamp>; 4]) {
        let mut c0 = [[0.0f32; 4]; K];
        let mut e0 = [[0.0f32; 4]; K];
        let mut dc = [[0.0f32; 4]; K];
        let mut de = [[0.0f32; 4]; K];
        let mut d0 = [0.0f32; 4];
        let mut dd = [0.0f32; 4];
        let inv = 1.0 / len.max(1) as f32;
        let mut ramping = false;
        for (l, r) in ramps.into_iter().enumerate() {
            let Some(r) = r else { continue };
            let (a, b) = r.advance();
            for k in 0..K {
                c0[k][l] = a.c[k];
                e0[k][l] = a.e[k];
                dc[k][l] = (b.c[k] - a.c[k]) * inv;
                de[k][l] = (b.e[k] - a.e[k]) * inv;
                ramping |= dc[k][l] != 0.0 || de[k][l] != 0.0;
            }
            d0[l] = a.d;
            dd[l] = (b.d - a.d) * inv;
            ramping |= dd[l] != 0.0;
        }
        let c0: [F4; K] = core::array::from_fn(|k| F4::load(&c0[k]));
        let e0: [F4; K] = core::array::from_fn(|k| F4::load(&e0[k]));
        let pr: [F4; K] = core::array::from_fn(|k| F4::splat(self.pr[k]));
        let pi: [F4; K] = core::array::from_fn(|k| F4::splat(self.pi[k]));
        let (d0, dd) = (F4::load(&d0), F4::load(&dd));
        let (mut sr, mut si) = (self.sr[g], self.si[g]);
        let io = &mut self.io[g];
        if ramping {
            let dc: [F4; K] = core::array::from_fn(|k| F4::load(&dc[k]));
            let de: [F4; K] = core::array::from_fn(|k| F4::load(&de[k]));
            for (i, frame) in io[..len].iter_mut().enumerate() {
                let x = F4::load(frame);
                let t = F4::splat((i + 1) as f32);
                // Four independent accumulators: a single chain would serialise 2K FMAs.
                let mut acc = [d0.fma(t, dd).mul(x), F4::zero(), F4::zero(), F4::zero()];
                for k in 0..K {
                    let (a, b) = (sr[k], si[k]);
                    let nr = x.fma(pr[k], a).fms(pi[k], b);
                    let ni = pi[k].mul(a).fma(pr[k], b);
                    sr[k] = nr;
                    si[k] = ni;
                    let j = 2 * (k % 2);
                    acc[j] = acc[j].fma(c0[k].fma(t, dc[k]), nr);
                    acc[j + 1] = acc[j + 1].fma(e0[k].fma(t, de[k]), ni);
                }
                acc[0].add(acc[1]).add(acc[2].add(acc[3])).store(frame);
            }
        } else {
            for frame in io[..len].iter_mut() {
                let x = F4::load(frame);
                let mut acc = [d0.mul(x), F4::zero(), F4::zero(), F4::zero()];
                for k in 0..K {
                    let (a, b) = (sr[k], si[k]);
                    let nr = x.fma(pr[k], a).fms(pi[k], b);
                    let ni = pi[k].mul(a).fma(pr[k], b);
                    sr[k] = nr;
                    si[k] = ni;
                    let j = 2 * (k % 2);
                    acc[j] = acc[j].fma(c0[k], nr);
                    acc[j + 1] = acc[j + 1].fma(e0[k], ni);
                }
                acc[0].add(acc[1]).add(acc[2].add(acc[3])).store(frame);
            }
        }
        self.sr[g] = sr;
        self.si[g] = si;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physics::{air, ground};

    fn db(c: C64) -> f64 {
        20.0 * c.abs().log10()
    }

    #[test]
    fn identity_fit() {
        let d = Design::new(48_000.0);
        let co = d.fit_min_phase(&[0.0; NF], None);
        for f in [30.0, 300.0, 3000.0, 15000.0] {
            let h = d.response(&co, f, 48_000.0);
            assert!((h - C64::ONE).abs() < 0.02, "f={f} h={h:?}");
        }
    }

    /// Air absorption over 200 m at 15 C / 50 % RH: a steep low-pass (~ -26 dB at 8 kHz).
    #[test]
    fn fits_air_absorption() {
        let d = Design::new(48_000.0);
        let mut ln = [0.0f64; NF];
        for i in 0..NF {
            ln[i] = -air::absorption_db_per_m(d.freqs[i], 15.0, 50.0, 101.325) * 200.0 / 8.686;
        }
        let co = d.fit_min_phase(&ln, None);
        for f in [100.0, 1000.0, 2000.0, 4000.0, 6000.0, 8000.0, 10000.0, 12000.0, 16000.0] {
            let want = -air::absorption_db_per_m(f, 15.0, 50.0, 101.325) * 200.0;
            let got = db(d.response(&co, f, 48_000.0));
            assert!((got - want).abs() < 0.75f64.max(-want * 0.05), "f={f}: {got:.2} vs {want:.2}");
        }
    }

    /// Complex ground reflection coefficient Q (grass, grazing, 30 m): magnitude and phase.
    #[test]
    fn fits_ground_reflection() {
        let d = Design::new(48_000.0);
        let (r2, sin_psi) = (30.0, 0.1);
        let q = |f: f64| {
            let k = 2.0 * PI * f / 343.0;
            ground::reflection_dsp(ground::admittance(ground::GroundType::Grass, f), k, r2, sin_psi)
        };
        let mut t = [C64::ZERO; NF];
        for i in 0..NF {
            t[i] = q(d.freqs[i]);
        }
        let co = d.fit(&t);
        let mut f = 40.0;
        while f < 16_000.0 {
            let err = (d.response(&co, f, 48_000.0) - q(f)).abs();
            assert!(err < 0.06, "f={f}: err {err}");
            f *= 1.3;
        }
    }

    /// A bank lane computes exactly what a single filter with the same coefficients does,
    /// including across a coefficient ramp.
    #[test]
    fn filter_bank_matches_single_filter() {
        let d = Design::new(48_000.0);
        let mut ln = [0.0f64; NF];
        for i in 0..NF {
            ln[i] = -(d.freqs[i] / 3000.0).powi(2);
        }
        let co1 = d.fit_min_phase(&ln, None);
        let co2 = d.fit_min_phase(&[0.0; NF], None);
        let mut single = ParallelFilter::new(&d);
        single.set(co1);
        let mut bank = FilterBank::new(&d, 6);
        let mut ramps = [CoeffRamp::new(); 6];
        ramps[5].set(co1);
        let mut x = [0.0f32; BLOCK];
        for b in 0..20 {
            for (i, v) in x.iter_mut().enumerate() {
                *v = ((b * BLOCK + i) as f32 * 0.37).sin();
            }
            if b == 5 {
                ramps[5].ramp_to(co2, 3);
                single.blocks_left = 3;
                single.target = co2;
            }
            bank.write(5, &x);
            let [_, _, _, _, r4, r5] = &mut ramps;
            bank.process_group(1, BLOCK, [Some(r4), Some(r5), None, None]);
            let mut y = [0.0f32; BLOCK];
            bank.read(5, &mut y);
            single.process(&mut x);
            for i in 0..BLOCK {
                assert!((x[i] - y[i]).abs() < 1e-4, "block {b} sample {i}: {} vs {}", x[i], y[i]);
            }
        }
    }

    #[test]
    #[ignore]
    fn perf_filter_bank() {
        let d = Design::new(48_000.0);
        let paths = 128;
        let mut bank = FilterBank::new(&d, paths);
        let mut ramps = vec![CoeffRamp::new(); paths];
        let x = [0.1f32; BLOCK];
        let n = 20_000;
        let t = std::time::Instant::now();
        for _ in 0..n {
            for p in 0..paths {
                bank.write(p, &x);
            }
            for (g, r) in ramps.chunks_mut(4).enumerate() {
                let [a, b, c, e] = r else { unreachable!() };
                bank.process_group(g, BLOCK, [Some(a), Some(b), Some(c), Some(e)]);
            }
        }
        let ns = t.elapsed().as_secs_f64() * 1e9 / (n as f64 * (paths * BLOCK) as f64);
        eprintln!("filter bank: {ns:.2} ns per path-sample");
    }

    #[test]
    fn runtime_filter_matches_design_response() {
        let fs = 48_000.0;
        let d = Design::new(fs as f32);
        let mut ln = [0.0f64; NF];
        for i in 0..NF {
            ln[i] = -(d.freqs[i] / 4000.0).powi(2); // smooth low-pass
        }
        let co = d.fit_min_phase(&ln, None);
        let mut pf = ParallelFilter::new(&d);
        pf.set(co);
        let f = 2000.0;
        let n = 9600;
        let mut buf: Vec<f32> = (0..n).map(|i| (2.0 * PI * f * i as f64 / fs).sin() as f32).collect();
        pf.process(&mut buf);
        let rms_out = (buf[n / 2..].iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / (n / 2) as f64).sqrt();
        let want = d.response(&co, f, fs).abs() / 2f64.sqrt();
        assert!((rms_out / want - 1.0).abs() < 0.01, "{rms_out} vs {want}");
    }
}
