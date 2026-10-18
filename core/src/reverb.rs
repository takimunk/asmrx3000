//! Forest reverberation: single-scatter early reflections from the trunks nearest the
//! listener, plus a late tail from a 16-line feedback delay network whose decay, density and
//! level are derived from the forest geometry.
//!
//! Early reflections. A trunk is a rigid convex cylinder; the geometric-optics specular
//! reflection of a point source at distance r1 from it, heard at r2, has amplitude
//!     A = sqrt(a cos(theta_i) / 2) / sqrt(r1 r2 (r1 + r2))
//! (flat mirror in the vertical plane, convex mirror of focal length a/2 in the horizontal),
//! high-passed below ka ~ 1 where scattering falls into the Rayleigh regime. Every emitter
//! adds its delayed, scaled signal into one mono bus per trunk (`Sends::trunks`); since a
//! trunk's direction seen from the listener does not depend on the emitter, each trunk bus
//! is filtered and encoded once (order 3). Cost per emitter: one interpolated read per trunk.
//!
//! Late tail (diffuse field). The forest is modelled as a slab between an absorbing floor and
//! a leaky canopy, filled with scatterers. Energy density decays as e^{-G t} with
//!     G(f) = c [ (alpha_floor(f) + e_canopy(f)) / (4 H) + n sigma_s(f) (pi/4) alpha_bark + m_air(f) ]
//! (slab mean free path 2H between floor and canopy; trunk hits at rate n sigma_s per metre of
//! horizontal travel), and spreads horizontally by diffusion, which adds the 1/(1 + t/t0)
//! factor (t0 = transport mean free time). T60 at 500 Hz and 4 kHz sets the FDN's per-line
//! absorption (Jot & Chaigne 1991, exact match at both frequencies). Line lengths scale with
//! the scattering mean free path. The level follows from energy balance: scattered power
//! n sigma_tr P feeds a diffuse field of decay rate G spread over area pi L^2 and height H,
//! giving a W-channel power ratio 4 n sigma_tr c / (G L H) relative to the emitter's
//! 1-metre level (critical distance r_c^2 = G L H / (4 n sigma_tr c)).
//! The 16 line outputs are encoded at 16 directions concentrated towards the horizon
//! (open sky above, absorbing floor below), decorrelated over the sphere.

use crate::ambisonics::{self, CHANNELS};
use crate::engine::BLOCK;
use crate::math::Vec3;
use crate::nearfield::Biquad;
use crate::shaping::{Coeffs, Design, ParallelFilter, NF};
use crate::simd::F4;
use core::f64::consts::PI;

pub const ER_TRUNKS: usize = 8;
const N: usize = 16;

/// Per-block accumulators the propagators write into.
pub struct Sends {
    pub reverb: [f32; BLOCK],
    pub trunks: [[f32; BLOCK]; ER_TRUNKS],
}

impl Sends {
    pub fn new() -> Self {
        Self { reverb: [0.0; BLOCK], trunks: [[0.0; BLOCK]; ER_TRUNKS] }
    }
    pub fn clear(&mut self, len: usize) {
        self.reverb[..len].fill(0.0);
        for t in self.trunks.iter_mut() {
            t[..len].fill(0.0);
        }
    }
}

/// Geometry-derived reverb parameters (computed off the per-sample path).
#[derive(Clone, Copy, Debug)]
pub struct TailParams {
    pub t60_low: f64,
    pub t60_mid: f64,
    pub t60_high: f64,
    /// Scattering mean free path (m) at 1 kHz.
    pub mean_free_path: f64,
    /// Amplitude factor from an emitter's 1-m level to the FDN input.
    pub send: f64,
    /// Horizontal extent L (m) of the diffuse field around the listener.
    pub extent: f64,
}

pub struct ForestReverb {
    sr: f32,
    lines: Vec<f32>,
    cap: usize,
    lens: [usize; N],
    w: usize,
    g: [f32; N],
    b: [f32; N],
    lp: [f32; N],
    in_filter: ParallelFilter,
    send: f32,
    out_sh: [[f32; CHANNELS]; N],
    trunk_hp: [Biquad; ER_TRUNKS],
    trunk_hp_c: [[f32; 5]; ER_TRUNKS],
    trunk_lp: [f32; ER_TRUNKS],
    trunk_lp_a: [f32; ER_TRUNKS],
    trunk_sh: [[f32; CHANNELS]; ER_TRUNKS],
    trunk_sh_prev: [[f32; CHANNELS]; ER_TRUNKS],
    pub params: TailParams,
    tmp: [f32; BLOCK],
    ybuf: [[f32; BLOCK]; N],
}

impl ForestReverb {
    pub fn new(sample_rate: f32, design: &Design) -> Self {
        let cap = ((0.12 * sample_rate) as usize).next_power_of_two();
        // Output directions: Fibonacci sphere squashed towards the horizon (z scaled by 0.4).
        let mut out_sh = [[0.0f32; CHANNELS]; N];
        for (i, d) in crate::binaural::fibonacci_sphere(N).iter().enumerate() {
            let v = Vec3::new(d.x, d.y, 0.4 * d.z);
            let v = v * (1.0 / v.length());
            ambisonics::encode(v, &mut out_sh[i]);
            for c in out_sh[i].iter_mut() {
                *c *= 0.25; // 1/sqrt(N): the W channel carries the mean line power
            }
        }
        Self {
            sr: sample_rate,
            lines: vec![0.0; N * cap],
            cap,
            lens: [1000; N],
            w: 0,
            g: [0.0; N],
            b: [0.0; N],
            lp: [0.0; N],
            in_filter: ParallelFilter::new(design),
            send: 0.0,
            out_sh,
            trunk_hp: [Biquad::default(); ER_TRUNKS],
            trunk_hp_c: [Biquad::IDENTITY; ER_TRUNKS],
            trunk_lp: [0.0; ER_TRUNKS],
            trunk_lp_a: [0.0; ER_TRUNKS],
            trunk_sh: [[0.0; CHANNELS]; ER_TRUNKS],
            trunk_sh_prev: [[0.0; CHANNELS]; ER_TRUNKS],
            params: TailParams { t60_low: 1.0, t60_mid: 1.0, t60_high: 1.0, mean_free_path: 50.0, send: 0.0, extent: 30.0 },
            tmp: [0.0; BLOCK],
            ybuf: [[0.0; BLOCK]; N],
        }
    }

    /// Sets the late tail. `input_shape`: fitted sqrt(sigma_tr(f)/sigma_tr(1 kHz)) response.
    pub fn configure_tail(&mut self, p: TailParams, input_shape: Coeffs, wet: f32) {
        let fs = self.sr as f64;
        let mfp_t = p.mean_free_path / 343.0;
        let d_min = (0.25 * mfp_t).clamp(0.009, 0.03);
        let d_max = (1.0 * mfp_t).clamp(0.035, 0.1);
        let mut g2 = 0.0;
        for i in 0..N {
            let d = d_min * (d_max / d_min).powf(i as f64 / (N - 1) as f64);
            // Odd, distinct lengths.
            let len = ((d * fs) as usize | 1) + 2 * i;
            self.lens[i] = len.min(self.cap - 1);
            let dl = self.lens[i] as f64;
            let g_lo = 10f64.powf(-3.0 * dl / (fs * p.t60_low));
            let g_hi = 10f64.powf(-3.0 * dl / (fs * p.t60_high));
            let w = 2.0 * PI * 4000.0 / fs;
            let a = g_lo * g_lo - g_hi * g_hi;
            let bb = g_lo * g_lo - g_hi * g_hi * w.cos();
            let b = if a > 1e-12 { (bb - (bb * bb - a * a).max(0.0).sqrt()) / a } else { 0.0 };
            self.g[i] = (g_lo * (1.0 - b)) as f32;
            self.b[i] = b as f32;
            let g_mid = 10f64.powf(-3.0 * dl / (fs * p.t60_mid));
            g2 += g_mid * g_mid / N as f64;
        }
        // Steady-state line power = mean(b_in^2) p_in / (1 - mean g^2), b_in = 1/4.
        self.send = (p.send * (16.0 * (1.0 - g2)).sqrt() * wet as f64) as f32;
        self.in_filter.set(input_shape);
        self.params = p;
    }

    /// Sets the early-reflection trunks: horizontal direction from the listener, radius and
    /// mean reflection path length (for air absorption, via `air_np_8k`, amplitude Np/m @ 8 kHz).
    pub fn configure_trunks(&mut self, trunks: &[(Vec3, f32, f32)], air_np_8k: f64, c: f64) {
        let fs = self.sr as f64;
        for j in 0..ER_TRUNKS {
            let Some(&(dir, radius, path)) = trunks.get(j) else {
                self.trunk_sh[j] = [0.0; CHANNELS];
                continue;
            };
            ambisonics::encode(dir, &mut self.trunk_sh[j]);
            // 2nd-order Butterworth high-pass at ka = 1.
            let fc = (c / (2.0 * PI * radius as f64)).min(0.4 * fs);
            let k = (PI * fc / fs).tan();
            let q = 0.7071;
            let norm = 1.0 / (1.0 + k / q + k * k);
            self.trunk_hp_c[j] = [
                norm as f32,
                (-2.0 * norm) as f32,
                norm as f32,
                (2.0 * (k * k - 1.0) * norm) as f32,
                ((1.0 - k / q + k * k) * norm) as f32,
            ];
            // One-pole low-pass matching the air absorption of the path at 8 kHz.
            let target = (-air_np_8k * path as f64).exp().clamp(0.05, 0.999);
            let w = 2.0 * PI * 8000.0 / fs;
            // |(1-a)/(1 - a e^{-jw})|^2 = t^2  ->  quadratic in a.
            let t2 = target * target;
            let (qa, qb, qc) = (1.0 - t2, -2.0 + 2.0 * t2 * w.cos(), 1.0 - t2);
            let a = if qa.abs() < 1e-12 { 0.0 } else { (-qb - (qb * qb - 4.0 * qa * qc).max(0.0).sqrt()) / (2.0 * qa) };
            self.trunk_lp_a[j] = a.clamp(0.0, 0.99) as f32;
        }
    }

    /// Adds the early reflections and the late tail into the (world-frame) bus.
    pub fn render(&mut self, sends: &Sends, bus: &mut [[f32; BLOCK]; CHANNELS], len: usize) {
        // Early reflections.
        for j in 0..ER_TRUNKS {
            let target = self.trunk_sh[j];
            let prev = self.trunk_sh_prev[j];
            if target.iter().all(|v| *v == 0.0) && prev.iter().all(|v| *v == 0.0) {
                continue;
            }
            let x = &mut self.tmp[..len];
            x.copy_from_slice(&sends.trunks[j][..len]);
            self.trunk_hp[j].process(x, self.trunk_hp_c[j]);
            let a = self.trunk_lp_a[j];
            let mut s = self.trunk_lp[j];
            for v in x.iter_mut() {
                s = (1.0 - a) * *v + a * s;
                *v = s;
            }
            self.trunk_lp[j] = s;
            let inv = 1.0 / len as f32;
            for ch in 0..CHANNELS {
                let (g0, g1) = (prev[ch], target[ch]);
                let step = (g1 - g0) * inv;
                let mut g = g0;
                let out = &mut bus[ch];
                for i in 0..len {
                    g += step;
                    out[i] += x[i] * g;
                }
            }
            self.trunk_sh_prev[j] = target;
        }

        // Late tail.
        let x = &mut self.tmp[..len];
        x.copy_from_slice(&sends.reverb[..len]);
        self.in_filter.process(x);
        let mask = self.cap - 1;
        let mut y = [0.0f32; N];
        for i in 0..len {
            let input = x[i] * self.send * 0.25;
            for l in 0..N {
                let o = self.lines[l * self.cap + ((self.w + self.cap - self.lens[l]) & mask)];
                self.lp[l] = self.g[l] * o + self.b[l] * self.lp[l];
                y[l] = self.lp[l];
            }
            // Orthonormal 16x16 Hadamard mixing (fast Walsh-Hadamard, scaled 1/4).
            let mut v = y;
            let mut h = 1;
            while h < N {
                for i0 in (0..N).step_by(2 * h) {
                    for k in i0..i0 + h {
                        let (a, b) = (v[k], v[k + h]);
                        v[k] = a + b;
                        v[k + h] = a - b;
                    }
                }
                h *= 2;
            }
            for l in 0..N {
                let sign = if l % 2 == 0 { 1.0 } else { -1.0 };
                self.lines[l * self.cap + (self.w & mask)] = 0.25 * v[l] + sign * input;
            }
            self.w = self.w.wrapping_add(1);
            for l in 0..N {
                self.ybuf[l][i] = y[l];
            }
        }
        // Encode the 16 line outputs, channel-major and 4 samples at a time.
        for ch in 0..CHANNELS {
            let out = &mut bus[ch];
            let mut i = 0;
            while i + 4 <= len {
                let mut acc = F4::load(&out[i..]);
                for l in 0..N {
                    acc = acc.fma(F4::splat(self.out_sh[l][ch]), F4::load(&self.ybuf[l][i..]));
                }
                acc.store(&mut out[i..]);
                i += 4;
            }
            for j in i..len {
                out[j] += (0..N).map(|l| self.out_sh[l][ch] * self.ybuf[l][j]).sum::<f32>();
            }
        }
    }
}

/// Forest geometry and materials that the tail and the statistical attenuation derive from.
#[derive(Clone, Copy, Debug)]
pub struct ForestGeometry {
    /// Trunks per square metre.
    pub density: f64,
    /// Height of the trunk zone (crown base) above the floor, m.
    pub crown_base: f64,
    /// Bark absorption coefficient.
    pub bark_absorption: f64,
}

/// Late-tail parameters from geometry. `sigma_s`, `sigma_tr`: mean trunk scattering widths
/// (m) at grid frequencies; `floor_alpha`: random-incidence absorption of the ground;
/// `air_np`: amplitude Np/m; `canopy_escape`: fraction of energy leaving through the canopy.
pub fn tail_params(
    geo: &ForestGeometry,
    freqs: &[f64; NF],
    sigma_s: &[f64; NF],
    sigma_tr: &[f64; NF],
    floor_alpha: &[f64; NF],
    air_np: &[f64; NF],
    foliage: f64,
    c: f64,
) -> TailParams {
    let n = geo.density.max(1e-6);
    let h = geo.crown_base.max(2.0);
    let at = |arr: &[f64; NF], f: f64| -> f64 {
        let i = freqs.iter().position(|x| *x >= f).unwrap_or(NF - 1).max(1);
        let t = (f.ln() - freqs[i - 1].ln()) / (freqs[i].ln() - freqs[i - 1].ln());
        arr[i - 1] + (arr[i] - arr[i - 1]) * t.clamp(0.0, 1.0)
    };
    let t60 = |f: f64| -> f64 {
        // Foliage scatters high frequencies back down; low frequencies escape.
        let back = foliage * 0.5 * (f / 2000.0).min(1.0);
        let escape = 1.0 - back;
        let g = c
            * ((at(floor_alpha, f) + escape) / (4.0 * h)
                + n * at(sigma_s, f) * PI / 4.0 * geo.bark_absorption
                + 2.0 * at(air_np, f));
        let t0 = 1.0 / (c * n * at(sigma_tr, f)).max(1e-3);
        // Solve e^{-g t} / (1 + t/t0) = 1e-6 by bisection.
        let (mut lo, mut hi) = (0.0f64, 13.8 / g.max(1e-3));
        for _ in 0..60 {
            let m = 0.5 * (lo + hi);
            if -g * m - (1.0 + m / t0).ln() > -13.8155 {
                lo = m;
            } else {
                hi = m;
            }
        }
        0.5 * (lo + hi)
    };
    let (t_lo, t_mid, t_hi) = (t60(500.0), t60(1000.0), t60(4000.0));
    let mfp = 1.0 / (n * at(sigma_s, 1000.0)).max(1e-4);
    let mtp = 1.0 / (n * at(sigma_tr, 1000.0)).max(1e-4);
    let extent = mtp.clamp(10.0, 40.0);
    let g_mid = 13.8155 / t_mid;
    let ratio = 4.0 * n * at(sigma_tr, 1000.0) * c / (g_mid * extent * h);
    TailParams { t60_low: t_lo, t60_mid: t_mid, t60_high: t_hi, mean_free_path: mfp, send: ratio.sqrt(), extent }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The FDN's measured energy decay matches the T60 it was configured with.
    #[test]
    fn fdn_decay_time_matches_target() {
        let fs = 48_000.0f32;
        let design = Design::new(fs);
        let mut rv = ForestReverb::new(fs, &design);
        let p = TailParams { t60_low: 1.2, t60_mid: 1.2, t60_high: 1.2, mean_free_path: 40.0, send: 1.0, extent: 30.0 };
        rv.configure_tail(p, Coeffs::identity(), 1.0);
        rv.configure_trunks(&[], 0.0, 343.0);
        let mut sends = Sends::new();
        let mut bus = [[0.0f32; BLOCK]; CHANNELS];
        let blocks = (2.0 * fs as f64 / BLOCK as f64) as usize;
        let mut energy = vec![0.0f64; blocks];
        for b in 0..blocks {
            sends.clear(BLOCK);
            if b == 0 {
                sends.reverb[0] = 1.0;
            }
            for ch in bus.iter_mut() {
                ch.fill(0.0);
            }
            rv.render(&sends, &mut bus, BLOCK);
            energy[b] = bus[0].iter().map(|v| (*v as f64).powi(2)).sum();
        }
        // Schroeder backward integration, fit -5 .. -25 dB (T20).
        let mut edc = vec![0.0f64; blocks];
        let mut acc = 0.0;
        for b in (0..blocks).rev() {
            acc += energy[b];
            edc[b] = acc;
        }
        let db = |b: usize| 10.0 * (edc[b] / edc[0]).log10();
        let t = |level: f64| (0..blocks).find(|&b| db(b) < level).unwrap() as f64 * BLOCK as f64 / fs as f64;
        let t60 = 3.0 * (t(-25.0) - t(-5.0));
        eprintln!("FDN T60 measured {t60:.3} s (target 1.2 s)");
        assert!((t60 / 1.2 - 1.0).abs() < 0.1, "measured T60 {t60}");
    }

    #[test]
    fn denser_forest_reverberates_more() {
        let freqs: [f64; NF] = core::array::from_fn(|i| 20.0 * 1000f64.powf(i as f64 / (NF - 1) as f64));
        let s = [0.5f64; NF];
        let tr = [0.35f64; NF];
        let floor = [0.6f64; NF];
        let air = [1e-4f64; NF];
        let sparse = ForestGeometry { density: 0.01, crown_base: 8.0, bark_absorption: 0.05 };
        let dense = ForestGeometry { density: 0.1, ..sparse };
        let a = tail_params(&sparse, &freqs, &s, &tr, &floor, &air, 0.5, 343.0);
        let b = tail_params(&dense, &freqs, &s, &tr, &floor, &air, 0.5, 343.0);
        eprintln!("tail sparse {a:?}\ntail dense {b:?}");
        assert!(b.send > a.send);
        assert!(a.t60_mid > 0.3 && a.t60_mid < 3.0, "{a:?}");
        assert!(b.mean_free_path < a.mean_free_path);
    }
}
