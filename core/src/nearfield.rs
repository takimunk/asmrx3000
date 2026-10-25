//! Near-field compensated Higher-Order Ambisonics (J. Daniel, AES 23rd conf. 2003), used as
//! HRTF range extrapolation (Duraiswami, Zotkin & Gumerov 2004; Pollow et al. 2012).
//!
//! A point source at distance r has SH coefficients proportional to h_m(kr); the HRTFs our
//! decoder is built from were measured at radius R. Re-weighting order m by h_m(kr)/h_m(kR)
//! therefore turns "source at R" into "source at r" for every ear position the order-3
//! expansion resolves (f below ~1.9 kHz for a human head) -- the region where near-field ILD
//! and proximity bass live. With s = jw:
//!     H_m(s) = prod_i (s - c/(r x_i)) / (s - c/(R x_i)),   x_i = roots of the Bessel polynomial y_m,
//! stable for any r, DC gain (R/r)^m, identity at r = R. Bilinear transform per section.
//! Regularisation: one common minimum distance R_MIN_NFC = 0.2 m for all orders (the
//! expansion is only valid outside the head, and clamping orders separately would give them
//! inconsistent phases).
//!
//! Low frequencies of orders >= 2: there the true HRTF coefficients are tiny (~(ka)^m) and the
//! measured ones are mostly fitting error, which the extrapolation's DC gain (R/r)^m (+31 dB
//! for order 2, +47 dB for order 3 at 0.2 m) blew up: a source 0.3 m to the left was louder in
//! the right ear at 110 Hz, and 0.2 m ahead came out +10..+17 dB over the point-source law.
//! So orders >= 2 are extrapolated only above SPLIT_HZ, through a Linkwitz-Riley crossover
//! (LP4 + HP4 = a 2nd-order all-pass): below it they stay as measured at R. Orders 0-1 go
//! through that same all-pass (always, near or far, so nothing jumps at the zone's edge), so
//! every order keeps one common phase and they sum coherently. Orders 0-1 carry the
//! low-frequency near field (level and interaural gradient), which then follows the
//! 1/distance-to-each-ear law within ~1 dB (test `near_field_table`).

use crate::ambisonics::ORDER;
use crate::num::{poly_roots, C64};

/// Closest distance the extrapolation is applied for (must stay outside the head).
pub const R_MIN_NFC: f64 = 0.2;

/// Crossover below which orders >= 2 are not extrapolated, Hz.
pub const SPLIT_HZ: f64 = 400.0;

#[derive(Clone, Copy, Default, Debug)]
pub struct Biquad {
    b: [f32; 3],
    a: [f32; 2],
    s: [f32; 2],
}

impl Biquad {
    pub const IDENTITY: [f32; 5] = [1.0, 0.0, 0.0, 0.0, 0.0];

    fn coeffs(&self) -> [f32; 5] {
        [self.b[0], self.b[1], self.b[2], self.a[0], self.a[1]]
    }

    /// Filters `buf` in place, ramping coefficients linearly to `to` across the block
    /// (stable: the biquad stability region is convex in (a1, a2)).
    pub fn process(&mut self, buf: &mut [f32], to: [f32; 5]) {
        let from = self.coeffs();
        let n = buf.len().max(1) as f32;
        let step: [f32; 5] = core::array::from_fn(|i| (to[i] - from[i]) / n);
        let mut c = from;
        let [mut s1, mut s2] = self.s;
        for x in buf.iter_mut() {
            for i in 0..5 {
                c[i] += step[i];
            }
            let v = *x;
            let y = c[0] * v + s1;
            s1 = c[1] * v - c[3] * y + s2;
            s2 = c[2] * v - c[4] * y;
            *x = y;
        }
        self.s = [s1, s2];
        self.b = [to[0], to[1], to[2]];
        self.a = [to[3], to[4]];
    }

    pub fn reset(&mut self) {
        *self = Biquad { b: [1.0, 0.0, 0.0], a: [0.0, 0.0], s: [0.0; 2] };
    }

    /// Sets coefficients without a ramp (for a filter that starts from silence).
    pub fn set(&mut self, c: [f32; 5]) {
        self.b = [c[0], c[1], c[2]];
        self.a = [c[3], c[4]];
    }
}

/// Bessel polynomial roots per order, grouped into real roots and upper-half-plane roots.
pub struct NfcDesign {
    sections: [[Section; 2]; ORDER + 1],
    pub ref_radius: f64,
    fs: f64,
    c: f64,
    /// Crossover biquads (Butterworth, Q = 1/sqrt 2): low-pass, high-pass, and the all-pass
    /// their 4th-order Linkwitz-Riley sum equals.
    lp: [f32; 5],
    hp: [f32; 5],
    ap: [f32; 5],
}

#[derive(Clone, Copy, Debug, Default)]
enum Section {
    #[default]
    None,
    Real(f64),
    Pair(C64),
}

impl NfcDesign {
    pub fn new(sample_rate: f32, ref_radius: f64) -> Self {
        let mut sections = [[Section::None; 2]; ORDER + 1];
        for m in 1..=ORDER {
            // y_m(x) = sum_k (m+k)! / ((m-k)! k!) (x/2)^k
            let fact = |n: usize| (1..=n).map(|v| v as f64).product::<f64>();
            let coef: Vec<f64> = (0..=m).map(|k| fact(m + k) / (fact(m - k) * fact(k)) / 2f64.powi(k as i32)).collect();
            let roots = poly_roots(&coef);
            let mut idx = 0;
            for r in roots.iter().filter(|r| r.im.abs() < 1e-9) {
                sections[m][idx] = Section::Real(r.re);
                idx += 1;
            }
            for r in roots.iter().filter(|r| r.im > 1e-9) {
                sections[m][idx] = Section::Pair(*r);
                idx += 1;
            }
        }
        let fs = sample_rate as f64;
        let w = 2.0 * core::f64::consts::PI * SPLIT_HZ / fs;
        let (cw, alpha) = (w.cos(), w.sin() / (2.0 * core::f64::consts::FRAC_1_SQRT_2));
        let a0 = 1.0 + alpha;
        let q = |b: [f64; 3]| [(b[0] / a0) as f32, (b[1] / a0) as f32, (b[2] / a0) as f32, (-2.0 * cw / a0) as f32, ((1.0 - alpha) / a0) as f32];
        let lp = q([(1.0 - cw) / 2.0, 1.0 - cw, (1.0 - cw) / 2.0]);
        let hp = q([(1.0 + cw) / 2.0, -(1.0 + cw), (1.0 + cw) / 2.0]);
        let ap = q([1.0 - alpha, -2.0 * cw, 1.0 + alpha]);
        Self { sections, ref_radius, fs, c: 343.0, lp, hp, ap }
    }

    pub fn set_speed_of_sound(&mut self, c: f64) {
        self.c = c;
    }

    /// Digital sections for order m at source distance r. Returns identity at r >= R.
    pub fn coeffs(&self, m: usize, r: f64) -> [[f32; 5]; 2] {
        let mut out = [Biquad::IDENTITY; 2];
        let rr = self.ref_radius;
        let r = r.max(R_MIN_NFC);
        if r >= rr {
            return out;
        }
        let k = 2.0 * self.fs;
        for (slot, sec) in self.sections[m].iter().enumerate() {
            out[slot] = match *sec {
                Section::None => Biquad::IDENTITY,
                Section::Real(x) => {
                    // (s - al) / (s - be)
                    let al = self.c / (r * x);
                    let be = self.c / (rr * x);
                    let n0 = k - be;
                    [((k - al) / n0) as f32, (-(k + al) / n0) as f32, 0.0, (-(k + be) / n0) as f32, 0.0]
                }
                Section::Pair(x) => {
                    // (s^2 - 2 Re(al) s + |al|^2) / (s^2 - 2 Re(be) s + |be|^2)
                    let al = (x.scale(r)).inv().scale(self.c);
                    let be = (x.scale(rr)).inv().scale(self.c);
                    let (n1, n0) = (-2.0 * al.re, al.norm_sqr());
                    let (d1, d0) = (-2.0 * be.re, be.norm_sqr());
                    let bz = [k * k + n1 * k + n0, -2.0 * k * k + 2.0 * n0, k * k - n1 * k + n0];
                    let az = [k * k + d1 * k + d0, -2.0 * k * k + 2.0 * d0, k * k - d1 * k + d0];
                    [
                        (bz[0] / az[0]) as f32,
                        (bz[1] / az[0]) as f32,
                        (bz[2] / az[0]) as f32,
                        (az[1] / az[0]) as f32,
                        (az[2] / az[0]) as f32,
                    ]
                }
            };
        }
        out
    }
}

/// Per-emitter near-field state: two sections for each order 1..=3.
#[derive(Clone, Default)]
pub struct NearField {
    sec: [[Biquad; 2]; ORDER + 1],
    /// Per order >= 2: the crossover's two low-pass and two high-pass stages.
    xo: [[Biquad; 4]; ORDER + 1],
    /// The all-pass every order goes through (kept across the near zone's edge).
    ap: Biquad,
    ap_ready: bool,
    active: bool,
}

impl NearField {
    pub fn new() -> Self {
        let mut n = Self::default();
        n.reset();
        n
    }

    pub fn reset(&mut self) {
        for o in self.sec.iter_mut() {
            for b in o.iter_mut() {
                b.reset();
            }
        }
        for o in self.xo.iter_mut() {
            for b in o.iter_mut() {
                b.reset();
            }
        }
        self.active = false;
    }

    /// Runs the crossovers of orders >= 2 on `dry` and drops the result: far from the head
    /// they keep their state warm, so entering the near zone does not start them cold (a cold
    /// LR4 pair on a running tone summed to a crackle 15 dB over the signal's own top end,
    /// heard where a path crosses R).
    fn warm_crossovers(&mut self, design: &NfcDesign, dry: &[f32]) {
        let n = dry.len();
        let mut buf = [0.0f32; crate::engine::BLOCK];
        for m in 2..=ORDER {
            let [l1, l2, h1, h2] = &mut self.xo[m];
            for (a, b, c) in [(l1, l2, design.lp), (h1, h2, design.hp)] {
                buf[..n].copy_from_slice(dry);
                a.process(&mut buf[..n], c);
                b.process(&mut buf[..n], c);
            }
        }
    }

    /// Puts `x` through the crossover's all-pass (every order's common phase) and, near the
    /// head, fills `orders[m]` (m = 1..=3) with the order-m extrapolated version of it.
    /// Returns false (and leaves `orders` untouched) when the source is beyond R, i.e. all
    /// orders equal `x`.
    pub fn process(&mut self, design: &NfcDesign, r: f64, x: &mut [f32], orders: &mut [[f32; crate::engine::BLOCK]; ORDER]) -> bool {
        let n = x.len();
        let mut dry = [0.0f32; crate::engine::BLOCK];
        dry[..n].copy_from_slice(x);
        if !self.ap_ready {
            self.ap.set(design.ap);
            self.ap_ready = true;
        }
        self.ap.process(x, design.ap);
        let near = r < design.ref_radius;
        if !near && !self.active {
            self.warm_crossovers(design, &dry[..n]);
            return false;
        }
        for m in 1..=ORDER {
            let c = design.coeffs(m, r);
            let fresh = !self.active;
            let buf = &mut orders[m - 1][..n];
            let mut low = [0.0f32; crate::engine::BLOCK];
            if m >= 2 {
                // Crossover: below SPLIT_HZ the order stays as measured, above it is extrapolated.
                let low = &mut low[..n];
                low.copy_from_slice(&dry[..n]);
                buf.copy_from_slice(&dry[..n]);
                let [l1, l2, h1, h2] = &mut self.xo[m];
                for (b, c) in [(l1, design.lp), (l2, design.lp)] {
                    if fresh {
                        b.set(c);
                    }
                    b.process(low, c);
                }
                for (b, c) in [(h1, design.hp), (h2, design.hp)] {
                    if fresh {
                        b.set(c);
                    }
                    b.process(buf, c);
                }
            } else {
                buf.copy_from_slice(x);
            }
            for (s, cs) in self.sec[m].iter_mut().zip(c) {
                if fresh {
                    // Fresh start: no ramp from identity across the first block.
                    s.set(cs);
                }
                s.process(buf, cs);
            }
            if m >= 2 {
                for (b, l) in buf.iter_mut().zip(&low[..n]) {
                    *b += l;
                }
            }
        }
        // Stay active for one block after leaving the near zone so the filters settle onto
        // identity, then drop back to the free path. The crossovers keep running (warm).
        if !near {
            for o in self.sec.iter_mut() {
                for b in o.iter_mut() {
                    b.reset();
                }
            }
            self.active = false;
        } else {
            self.active = true;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::f64::consts::PI;

    fn gain(c: [[f32; 5]; 2], f: f64, fs: f64) -> f64 {
        let z = C64::from_polar(1.0, -2.0 * PI * f / fs);
        let mut h = C64::ONE;
        for s in c {
            let num = C64::new(s[0] as f64, 0.0) + z.scale(s[1] as f64) + (z * z).scale(s[2] as f64);
            let den = C64::ONE + z.scale(s[3] as f64) + (z * z).scale(s[4] as f64);
            h = h * num / den;
        }
        h.abs()
    }

    #[test]
    fn matches_hankel_ratio() {
        // |h_1(kr)/h_1(kR)| for order 1: h_1(x) ~ e^{ix}/x (1 + i/x)  ->  |.| = sqrt(1+x^2)/x^2.
        let d = NfcDesign::new(48_000.0, 1.5);
        let r = 0.5;
        let c = d.coeffs(1, r);
        for f in [50.0, 200.0, 1000.0] {
            let k = 2.0 * PI * f / 343.0;
            let mag = |x: f64| (1.0 + x * x).sqrt() / (x * x);
            // Normalised by the order-0 term (|h_0| = 1/x), which is the 1/r spreading we
            // apply separately.
            let want = (mag(k * r) / mag(k * 1.5)) / ((1.0 / (k * r)) / (1.0 / (k * 1.5)));
            let got = gain(c, f, 48_000.0);
            assert!((got / want - 1.0).abs() < 0.03, "f={f}: {got} vs {want}");
        }
        // Identity beyond R; regularised DC boost below.
        assert!((gain(d.coeffs(3, 2.0), 100.0, 48_000.0) - 1.0).abs() < 1e-6);
        assert!(gain(d.coeffs(3, 0.1), 5.0, 48_000.0) < 1.01 * (1.5 / R_MIN_NFC).powi(3));
    }
}

#[cfg(test)]
mod zone_edge {
    use super::*;
    use crate::engine::BLOCK;

    /// A tone that crosses into the near zone keeps a continuous waveform in every order: just
    /// inside R the filters are close to identity, so order 2..3 must stay close to order 0.
    #[test]
    fn entering_the_near_zone_is_seamless() {
        let fs = 48_000.0;
        let d = NfcDesign::new(fs, 1.2);
        let mut nf = NearField::new();
        let mut orders = [[0.0f32; BLOCK]; ORDER];
        let mut worst = 0.0f32;
        for b in 0..200 {
            let mut x = [0.0f32; BLOCK];
            for (i, v) in x.iter_mut().enumerate() {
                let t = (b * BLOCK + i) as f32 / fs;
                *v = (core::f32::consts::TAU * 220.0 * t).sin() * 0.5 + (core::f32::consts::TAU * 1300.0 * t).sin() * 0.2;
            }
            // Far for 100 blocks, then 1 mm inside R.
            let r = if b < 100 { 1.5 } else { 1.199 };
            let near = nf.process(&d, r, &mut x, &mut orders);
            if near && b < 110 {
                for o in &orders[1..] {
                    for (a, b) in o.iter().zip(x.iter()) {
                        worst = worst.max((a - b).abs());
                    }
                }
            }
        }
        assert!(worst < 0.02, "orders differ from the all-passed signal by {worst} on entry");
    }
}

#[cfg(test)]
mod range_extrapolation {
    use super::*;
    use crate::ambisonics::{self, CHANNELS};
    use crate::binaural::fibonacci_sphere;
    use crate::math::Vec3;
    use crate::num::pinv;
    use core::f64::consts::PI;

    /// Range extrapolation is exact in principle: with ideal point receivers at +-a (no head),
    /// "HRTFs" measured at R = 1.2 m, an LS order-3 decoder and the NFC filters, the rendered
    /// ILD of a source at r < R matches the exact free-field value (1/r parallax).
    #[test]
    fn range_extrapolation_free_field() {
        let (a, rr, c, fs) = (0.0875f64, 1.2f64, 343.0f64, 48_000.0f64);
        let dirs = fibonacci_sphere(600);
        let y: Vec<Vec<f64>> = dirs
            .iter()
            .map(|d| {
                let mut s = [0.0f32; CHANNELS];
                ambisonics::encode(*d, &mut s);
                s.iter().map(|v| *v as f64).collect()
            })
            .collect();
        let p = pinv(&y, None, 1e-9);
        let design = NfcDesign::new(fs as f32, rr);
        let field = |src: [f64; 3], ear: f64, k: f64, r0: f64| {
            let d = ((src[0]).powi(2) + (src[1] - ear).powi(2) + (src[2]).powi(2)).sqrt();
            // Relative to the field at the head centre (delay and 1/r removed), DSP convention.
            C64::from_polar(r0 / d, -k * (d - r0))
        };
        for f in [300.0, 500.0, 800.0] {
            let k = 2.0 * PI * f / c;
            for r in [0.3, 0.6, 2.0] {
                let mut ears = [C64::ZERO; 2];
                for (e, ear) in [a, -a].iter().enumerate() {
                    // LS decoder for this ear from sources at R.
                    let h: Vec<C64> = dirs.iter().map(|d| field([rr * d.x as f64, rr * d.y as f64, rr * d.z as f64], *ear, k, rr)).collect();
                    let mut dec = [C64::ZERO; CHANNELS];
                    for ch in 0..CHANNELS {
                        for q in 0..dirs.len() {
                            dec[ch] = dec[ch] + h[q].scale(p[ch][q]);
                        }
                    }
                    let mut sh = [0.0f32; CHANNELS];
                    ambisonics::encode(Vec3::new(0.0, 1.0, 0.0), &mut sh);
                    let mut sum = C64::ZERO;
                    for ch in 0..CHANNELS {
                        let m = ambisonics::order_of(ch);
                        let nfc = if m == 0 { C64::ONE } else { resp(design.coeffs(m, r), f, fs) };
                        sum = sum + dec[ch] * nfc.scale(sh[ch] as f64);
                    }
                    ears[e] = sum;
                }
                let exact = |ear: f64| field([0.0, r, 0.0], ear, k, r).abs();
                let got = 20.0 * (ears[0].abs() / ears[1].abs()).log10();
                let want = 20.0 * (exact(a) / exact(-a)).log10();
                eprintln!("free-field range extrapolation f={f} r={r}: {got:.2} dB vs exact {want:.2} dB");
                if r < rr {
                    assert!((got - want).abs() < 0.5, "f={f} r={r}: rendered ILD {got:.2} dB, exact {want:.2} dB");
                }
            }
        }
    }

    fn resp(c: [[f32; 5]; 2], f: f64, fs: f64) -> C64 {
        let z = C64::from_polar(1.0, -2.0 * PI * f / fs);
        let mut h = C64::ONE;
        for s in c {
            let num = C64::new(s[0] as f64, 0.0) + z.scale(s[1] as f64) + (z * z).scale(s[2] as f64);
            let den = C64::ONE + z.scale(s[3] as f64) + (z * z).scale(s[4] as f64);
            h = h * num / den;
        }
        h
    }
}
