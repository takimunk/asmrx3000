//! Numerics for init-time and control-rate code (f64): complex numbers, FFT, small dense
//! linear algebra and the special functions the physical models need. Nothing here runs per
//! sample, and nothing here allocates except the explicitly Vec-returning init helpers.

use core::f64::consts::PI;
use core::ops::{Add, Div, Mul, Neg, Sub};
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct C64 {
    pub re: f64,
    pub im: f64,
}

impl C64 {
    pub const ZERO: C64 = C64 { re: 0.0, im: 0.0 };
    pub const ONE: C64 = C64 { re: 1.0, im: 0.0 };
    pub const I: C64 = C64 { re: 0.0, im: 1.0 };

    #[inline]
    pub const fn new(re: f64, im: f64) -> Self {
        Self { re, im }
    }
    #[inline]
    pub fn from_polar(r: f64, th: f64) -> Self {
        let (s, c) = th.sin_cos();
        Self::new(r * c, r * s)
    }
    #[inline]
    pub fn conj(self) -> Self {
        Self::new(self.re, -self.im)
    }
    #[inline]
    pub fn norm_sqr(self) -> f64 {
        self.re * self.re + self.im * self.im
    }
    #[inline]
    pub fn abs(self) -> f64 {
        self.re.hypot(self.im)
    }
    #[inline]
    pub fn arg(self) -> f64 {
        self.im.atan2(self.re)
    }
    #[inline]
    pub fn exp(self) -> Self {
        Self::from_polar(self.re.exp(), self.im)
    }
    /// Principal square root (branch cut on the negative real axis).
    pub fn sqrt(self) -> Self {
        let r = self.abs();
        let re = (0.5 * (r + self.re)).max(0.0).sqrt();
        let im = (0.5 * (r - self.re)).max(0.0).sqrt();
        Self::new(re, if self.im < 0.0 { -im } else { im })
    }
    #[inline]
    pub fn scale(self, s: f64) -> Self {
        Self::new(self.re * s, self.im * s)
    }
    #[inline]
    pub fn inv(self) -> Self {
        let d = self.norm_sqr();
        Self::new(self.re / d, -self.im / d)
    }
}

impl Add for C64 {
    type Output = C64;
    #[inline]
    fn add(self, o: C64) -> C64 {
        C64::new(self.re + o.re, self.im + o.im)
    }
}
impl Sub for C64 {
    type Output = C64;
    #[inline]
    fn sub(self, o: C64) -> C64 {
        C64::new(self.re - o.re, self.im - o.im)
    }
}
impl Mul for C64 {
    type Output = C64;
    #[inline]
    fn mul(self, o: C64) -> C64 {
        C64::new(self.re * o.re - self.im * o.im, self.re * o.im + self.im * o.re)
    }
}
impl Div for C64 {
    type Output = C64;
    #[inline]
    fn div(self, o: C64) -> C64 {
        self * o.inv()
    }
}
impl Neg for C64 {
    type Output = C64;
    #[inline]
    fn neg(self) -> C64 {
        C64::new(-self.re, -self.im)
    }
}
impl Mul<f64> for C64 {
    type Output = C64;
    #[inline]
    fn mul(self, s: f64) -> C64 {
        self.scale(s)
    }
}
impl Add<f64> for C64 {
    type Output = C64;
    #[inline]
    fn add(self, s: f64) -> C64 {
        C64::new(self.re + s, self.im)
    }
}
impl Sub<C64> for f64 {
    type Output = C64;
    #[inline]
    fn sub(self, o: C64) -> C64 {
        C64::new(self - o.re, -o.im)
    }
}

/// In-place radix-2 FFT, X[k] = sum x[n] e^{-2 pi i k n / N}. `inverse` uses e^{+...} and 1/N.
pub fn fft(buf: &mut [C64], inverse: bool) {
    let n = buf.len();
    assert!(n.is_power_of_two());
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            buf.swap(i, j);
        }
    }
    let sign = if inverse { 1.0 } else { -1.0 };
    let mut len = 2;
    while len <= n {
        let ang = sign * 2.0 * PI / len as f64;
        for start in (0..n).step_by(len) {
            for k in 0..len / 2 {
                let w = C64::from_polar(1.0, ang * k as f64);
                let a = buf[start + k];
                let b = buf[start + k + len / 2] * w;
                buf[start + k] = a + b;
                buf[start + k + len / 2] = a - b;
            }
        }
        len <<= 1;
    }
    if inverse {
        let s = 1.0 / n as f64;
        for v in buf.iter_mut() {
            *v = v.scale(s);
        }
    }
}

/// Gauss-Jordan inverse with partial pivoting. Returns None for a singular matrix.
pub fn invert(mut a: Vec<Vec<f64>>) -> Option<Vec<Vec<f64>>> {
    let n = a.len();
    let mut inv: Vec<Vec<f64>> = (0..n).map(|i| (0..n).map(|j| (i == j) as u8 as f64).collect()).collect();
    for col in 0..n {
        let pivot = (col..n).max_by(|&i, &j| a[i][col].abs().total_cmp(&a[j][col].abs()))?;
        if a[pivot][col].abs() < 1e-300 {
            return None;
        }
        a.swap(col, pivot);
        inv.swap(col, pivot);
        let p = a[col][col];
        for j in 0..n {
            a[col][j] /= p;
            inv[col][j] /= p;
        }
        for row in 0..n {
            if row != col {
                let f = a[row][col];
                if f != 0.0 {
                    for j in 0..n {
                        a[row][j] -= f * a[col][j];
                        inv[row][j] -= f * inv[col][j];
                    }
                }
            }
        }
    }
    Some(inv)
}

/// Least-squares pseudo-inverse of a tall matrix A (rows x cols): (A^T W A + lambda I)^-1 A^T W,
/// returned as cols x rows. `weights` are per-row (None = 1), `ridge` is relative to the
/// mean diagonal of A^T W A.
pub fn pinv(a: &[Vec<f64>], weights: Option<&[f64]>, ridge: f64) -> Vec<Vec<f64>> {
    let rows = a.len();
    let cols = a[0].len();
    let w = |r: usize| weights.map_or(1.0, |w| w[r]);
    let mut g = vec![vec![0.0f64; cols]; cols];
    for r in 0..rows {
        let wr = w(r);
        for i in 0..cols {
            let ai = a[r][i] * wr;
            for j in 0..cols {
                g[i][j] += ai * a[r][j];
            }
        }
    }
    let mean_diag = (0..cols).map(|i| g[i][i]).sum::<f64>() / cols as f64;
    for (i, row) in g.iter_mut().enumerate() {
        row[i] += ridge * mean_diag;
    }
    let ginv = invert(g).expect("pinv: singular normal matrix");
    let mut out = vec![vec![0.0f64; rows]; cols];
    for i in 0..cols {
        for r in 0..rows {
            let mut s = 0.0;
            for j in 0..cols {
                s += ginv[i][j] * a[r][j];
            }
            out[i][r] = s * w(r);
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Faddeeva function w(z) = exp(-z^2) erfc(-iz)  (Weideman 1994 rational expansion)

const WN: usize = 16;

fn weideman() -> &'static [f64; WN] {
    static A: OnceLock<[f64; WN]> = OnceLock::new();
    A.get_or_init(|| {
        let m = 2 * WN;
        let l = (WN as f64 / 2f64.sqrt()).sqrt();
        // a_n = 1/(2M) sum_{k=-M+1}^{M-1} f(theta_k) e^{-i n theta_k}, f even -> cosine sum.
        let f = |k: i64| {
            let theta = k as f64 * PI / m as f64;
            let t = l * (0.5 * theta).tan();
            (-t * t).exp() * (l * l + t * t)
        };
        let mut a = [0.0; WN];
        for (idx, v) in a.iter_mut().enumerate() {
            let n = (idx + 1) as f64;
            let mut s = f(0);
            for k in 1..m as i64 {
                s += 2.0 * f(k) * (n * k as f64 * PI / m as f64).cos();
            }
            *v = s / (2.0 * m as f64);
        }
        a
    })
}

/// Faddeeva function. Accurate to ~1e-6 in the upper half plane (N = 16); the lower half plane uses
/// w(z) = 2 exp(-z^2) - w(-z).
pub fn faddeeva(z: C64) -> C64 {
    if z.im < 0.0 {
        let e = (-(z * z)).exp();
        return e.scale(2.0) - faddeeva(-z);
    }
    let a = weideman();
    let l = (WN as f64 / 2f64.sqrt()).sqrt();
    let iz = C64::I * z;
    let lmiz = C64::new(l, 0.0) - iz;
    let zz = (C64::new(l, 0.0) + iz) / lmiz;
    let mut p = C64::ZERO;
    for n in (0..WN).rev() {
        p = p * zz + a[n];
    }
    p.scale(2.0) / (lmiz * lmiz) + (lmiz.scale(PI.sqrt())).inv()
}

/// Fresnel integrals (C(x), S(x)) = int_0^x (cos, sin)(pi t^2 / 2) dt.
/// Abramowitz & Stegun 7.3.32-33 auxiliary functions, |error| < 2e-3.
pub fn fresnel(x: f64) -> (f64, f64) {
    let ax = x.abs();
    let f = (1.0 + 0.926 * ax) / (2.0 + 1.792 * ax + 3.104 * ax * ax);
    let g = 1.0 / (2.0 + 4.142 * ax + 3.492 * ax * ax + 6.670 * ax * ax * ax);
    let (s, c) = (0.5 * PI * ax * ax).sin_cos();
    let cc = 0.5 + f * s - g * c;
    let ss = 0.5 - f * c - g * s;
    if x < 0.0 {
        (-cc, -ss)
    } else {
        (cc, ss)
    }
}

// ---------------------------------------------------------------------------------------------
// Bessel functions of integer order, real argument x > 0.

/// J_0..=J_nmax by Miller's backward recurrence, normalised with J0 + 2 sum J_2k = 1.
pub fn bessel_j(nmax: usize, x: f64) -> Vec<f64> {
    let mut out = vec![0.0f64; nmax + 1];
    if x < 1e-12 {
        out[0] = 1.0;
        return out;
    }
    let big = nmax.max(x as usize);
    let start = 2 * ((big + 20 + (40.0 * big as f64).sqrt() as usize) / 2) + 2;
    let mut j = vec![0.0f64; start + 2];
    j[start] = 1e-300;
    for n in (1..=start).rev() {
        j[n - 1] = 2.0 * n as f64 / x * j[n] - j[n + 1];
        if j[n - 1].abs() > 1e250 {
            for v in j.iter_mut() {
                *v *= 1e-250;
            }
        }
    }
    let mut norm = j[0];
    let mut k = 2;
    while k <= start {
        norm += 2.0 * j[k];
        k += 2;
    }
    for n in 0..=nmax {
        out[n] = j[n] / norm;
    }
    out
}

fn simpson(f: impl Fn(f64) -> f64, a: f64, b: f64, n: usize) -> f64 {
    let n = n + n % 2;
    let h = (b - a) / n as f64;
    let mut s = f(a) + f(b);
    for i in 1..n {
        s += f(a + i as f64 * h) * if i % 2 == 1 { 4.0 } else { 2.0 };
    }
    s * h / 3.0
}

/// Y_n(x) = 1/pi int_0^pi sin(x sin t - n t) dt - 1/pi int_0^inf (e^{nt} + (-1)^n e^{-nt}) e^{-x sinh t} dt
fn bessel_y_integral(n: i32, x: f64) -> f64 {
    let nf = n as f64;
    let first = simpson(|t| (x * t.sin() - nf * t).sin(), 0.0, PI, (64.0 * x).max(400.0) as usize);
    let tmax = (60.0 / x).asinh() + 1.0;
    let sgn = if n % 2 == 0 { 1.0 } else { -1.0 };
    let second = simpson(|t| ((nf * t).exp() + sgn * (-nf * t).exp()) * (-x * t.sinh()).exp(), 0.0, tmax, 4000);
    (first - second) / PI
}

/// Y_0..=Y_nmax: Y0, Y1 from Bessel's integrals, then upward recurrence (stable for Y).
pub fn bessel_y(nmax: usize, x: f64) -> Vec<f64> {
    let mut y = vec![0.0f64; nmax + 1];
    y[0] = bessel_y_integral(0, x);
    if nmax >= 1 {
        y[1] = bessel_y_integral(1, x);
    }
    for n in 1..nmax {
        y[n + 1] = 2.0 * n as f64 / x * y[n] - y[n - 1];
    }
    y
}

/// All roots of a real polynomial c[0] + c[1] x + ... + c[n] x^n (Durand-Kerner).
pub fn poly_roots(c: &[f64]) -> Vec<C64> {
    let n = c.len() - 1;
    let lead = c[n];
    let monic: Vec<f64> = c.iter().map(|v| v / lead).collect();
    let eval = |x: C64| {
        let mut p = C64::ZERO;
        for k in (0..=n).rev() {
            p = p * x + monic[k];
        }
        p
    };
    let mut r: Vec<C64> = (0..n).map(|k| C64::from_polar(0.9, 0.4 + 2.0 * PI * k as f64 / n as f64)).collect();
    for _ in 0..500 {
        let mut delta = 0.0f64;
        for i in 0..n {
            let mut den = C64::ONE;
            for j in 0..n {
                if i != j {
                    den = den * (r[i] - r[j]);
                }
            }
            let step = eval(r[i]) / den;
            r[i] = r[i] - step;
            delta = delta.max(step.abs());
        }
        if delta < 1e-15 {
            break;
        }
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reference: Taylor series w(z) = sum (iz)^n / Gamma(n/2 + 1), fine in f64 for |z| < 3.
    fn faddeeva_series(z: C64) -> C64 {
        let iz = C64::I * z;
        let mut term = C64::ONE;
        let mut sum = C64::ZERO;
        let mut gamma = [1.0f64, PI.sqrt() / 2.0]; // Gamma(1), Gamma(1.5)
        for n in 0..200 {
            let g = if n % 2 == 0 { gamma[0] } else { gamma[1] };
            sum = sum + term.scale(1.0 / g);
            term = term * iz;
            if n % 2 == 0 {
                gamma[0] *= (n / 2 + 1) as f64;
            } else {
                gamma[1] *= (n / 2) as f64 + 1.5;
            }
        }
        sum
    }

    #[test]
    fn faddeeva_matches_known_values() {
        let w0 = faddeeva(C64::ZERO);
        assert!((w0 - C64::ONE).abs() < 1e-5, "{w0:?}");
        let wi = faddeeva(C64::I);
        assert!((wi.re - 0.427_583_576_155_807).abs() < 1e-5, "{wi:?}");
        let w1 = faddeeva(C64::new(1.0, 0.0));
        assert!((w1 - C64::new(0.367_879_441_171_442, 0.607_157_705_841_394)).abs() < 1e-5, "{w1:?}");
        for &(re, im) in &[(0.3, 0.2), (1.5, 0.7), (2.0, 0.1), (0.5, 2.0), (-1.2, 0.4), (0.8, -0.3)] {
            let z = C64::new(re, im);
            let (a, b) = (faddeeva(z), faddeeva_series(z));
            assert!((a - b).abs() < 1e-5 * b.abs().max(1.0), "z={z:?} w={a:?} ref={b:?}");
        }
    }

    #[test]
    fn fresnel_limits() {
        let (c, s) = fresnel(0.0);
        assert!(c.abs() < 1e-3 && s.abs() < 1e-3);
        let (c, s) = fresnel(1.0); // C(1)=0.7799, S(1)=0.4383
        assert!((c - 0.7799).abs() < 3e-3 && (s - 0.4383).abs() < 3e-3, "{c} {s}");
        let (c, s) = fresnel(50.0);
        assert!((c - 0.5).abs() < 1e-2 && (s - 0.5).abs() < 1e-2);
    }

    #[test]
    fn bessel_known_values() {
        let j = bessel_j(3, 1.0);
        assert!((j[0] - 0.765_197_686_557_967).abs() < 1e-12);
        assert!((j[1] - 0.440_050_585_744_934).abs() < 1e-12);
        let y = bessel_y(2, 1.0);
        assert!((y[0] - 0.088_256_964_215_677).abs() < 1e-7, "{}", y[0]);
        assert!((y[1] + 0.781_212_821_300_289).abs() < 1e-7, "{}", y[1]);
        let j = bessel_j(1, 10.0);
        assert!((j[0] + 0.245_935_764_451_348).abs() < 1e-12);
        let y = bessel_y(1, 10.0);
        assert!((y[0] - 0.055_671_167_283_599).abs() < 1e-7, "{}", y[0]);
        assert!((y[1] - 0.249_015_424_206_954).abs() < 1e-7, "{}", y[1]);
    }

    #[test]
    fn fft_roundtrip_and_roots() {
        let mut v: Vec<C64> = (0..64).map(|i| C64::new((i as f64 * 0.3).sin(), 0.0)).collect();
        let orig = v.clone();
        fft(&mut v, false);
        fft(&mut v, true);
        assert!(v.iter().zip(&orig).all(|(a, b)| (*a - *b).abs() < 1e-12));
        // y_2(x) = 1 + 3x + 3x^2: roots -1/2 +- i sqrt(3)/6
        let r = poly_roots(&[1.0, 3.0, 3.0]);
        for z in r {
            assert!((z.re + 0.5).abs() < 1e-10 && (z.im.abs() - 3f64.sqrt() / 6.0).abs() < 1e-10);
        }
    }
}
