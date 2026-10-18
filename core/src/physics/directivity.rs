//! Source directivity: a point source on a rigid sphere (head/body of radius a), the reciprocal
//! of the Brown & Duda (1998) head-shadow model: H(w, theta) = (1 + i alpha(theta) w/(2 w0)) /
//! (1 + i w/(2 w0)), w0 = c/a, alpha(theta) = 1.05 + 0.95 cos(theta/150deg * 180deg).
//! On axis this rises +6 dB at high frequency (pressure doubling), behind the body it falls.
//! Normalised by the sphere-averaged power so directivity redistributes energy but never adds any.

use crate::num::C64;
use core::f64::consts::PI;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Directivity {
    Omni,
    /// Radiator on a rigid sphere of this radius (m): songbird ~0.03, human head ~0.0875.
    Sphere { radius: f32 },
}

const ALPHA_MIN: f64 = 0.1;
const THETA_MIN: f64 = 150.0 * PI / 180.0;

fn alpha(theta: f64) -> f64 {
    (1.0 + ALPHA_MIN / 2.0) + (1.0 - ALPHA_MIN / 2.0) * (theta / THETA_MIN * PI).cos()
}

/// Unnormalised |H|^2.
fn power(f: f64, theta: f64, radius: f64, c: f64) -> f64 {
    let x = PI * f * radius / c; // w / (2 w0)
    C64::new(1.0, alpha(theta) * x).norm_sqr() / (1.0 + x * x)
}

/// Sphere-averaged |H|^2 (for normalisation).
pub fn mean_power(f: f64, radius: f64, c: f64) -> f64 {
    let n = 64;
    let mut s = 0.0;
    for i in 0..n {
        let mu = -1.0 + (i as f64 + 0.5) * 2.0 / n as f64;
        s += power(f, mu.acos(), radius, c);
    }
    s / n as f64
}

/// Natural-log magnitude of the normalised directivity at angle `theta` off the facing axis.
/// `mean` = `mean_power(f, radius, c)` (precomputed by the caller).
pub fn ln_gain(f: f64, theta: f64, radius: f64, c: f64, mean: f64) -> f64 {
    0.5 * (power(f, theta, radius, c) / mean).ln()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn front_louder_than_back_at_high_frequency() {
        let (a, c) = (0.05, 343.0);
        let m = mean_power(8000.0, a, c);
        let front = ln_gain(8000.0, 0.0, a, c, m) * 8.686;
        let back = ln_gain(8000.0, PI * 0.83, a, c, m) * 8.686;
        assert!(front > 3.0 && back < -10.0, "front {front} back {back}");
        // Omni at low frequency.
        let m = mean_power(100.0, a, c);
        assert!((ln_gain(100.0, PI, a, c, m) * 8.686).abs() < 0.3);
    }
}
