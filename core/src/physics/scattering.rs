//! Scattering of sound by tree trunks: exact partial-wave solution for an infinitely long rigid
//! circular cylinder at normal incidence (2-D; Morse & Ingard, "Theoretical Acoustics" 8.1).
//!
//! Scattering amplitude f(theta) ~ sum_n e^{i d_n} sin(d_n) e^{i n theta}, tan d_n = J_n'(ka)/Y_n'(ka).
//!   scattering width     sigma_s  = 4/k sum_n sin^2 d_n                       (n over all integers)
//!   transport width      sigma_tr = sigma_s - 4/k Re sum_n a_n conj(a_{n+1})  (= int |f|^2 (1 - cos))
//! Limits: ka << 1 -> sigma_s = 3 pi^2/4 k^3 a^4 (Rayleigh); ka >> 1 -> sigma_s = 4a (extinction
//! paradox: half reflection, half forward diffraction), sigma_tr -> 8a/3.
//!
//! The forward-diffracted half of sigma_s keeps travelling with the direct wave, so the coherent
//! direct sound in a forest decays with the *transport* width: I(r) = I0 exp(-n sigma_tr r), and
//! that energy feeds the diffuse field (reverb).

use crate::num::{bessel_j, bessel_y, C64};

#[derive(Clone, Copy, Debug)]
pub struct CylinderScattering {
    /// Scattering width (m).
    pub sigma_s: f64,
    /// Transport (momentum-transfer) width (m).
    pub sigma_tr: f64,
}

pub fn rigid_cylinder(ka: f64, a: f64) -> CylinderScattering {
    let k = ka / a;
    let nmax = (ka + 4.0 * ka.cbrt() + 12.0) as usize;
    let j = bessel_j(nmax + 1, ka);
    let y = bessel_y(nmax + 1, ka);
    // Derivatives: Z_0' = -Z_1, Z_n' = Z_{n-1} - n/x Z_n.
    let d = |z: &[f64], n: usize| if n == 0 { -z[1] } else { z[n - 1] - n as f64 / ka * z[n] };
    let amps: Vec<C64> = (0..=nmax)
        .map(|n| {
            let delta = d(&j, n).atan2(d(&y, n));
            C64::from_polar(delta.sin(), delta)
        })
        .collect();
    // a_{-n} = a_n.
    let amp = |n: i64| amps.get(n.unsigned_abs() as usize).copied().unwrap_or(C64::ZERO);
    let mut s = 0.0;
    let mut c = 0.0;
    let nm = nmax as i64;
    for n in -nm..=nm {
        s += amp(n).norm_sqr();
        c += (amp(n) * amp(n + 1).conj()).re;
    }
    CylinderScattering { sigma_s: 4.0 / k * s, sigma_tr: 4.0 / k * (s - c) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::f64::consts::PI;

    #[test]
    fn rayleigh_limit() {
        let a = 0.15;
        let ka = 0.05;
        let k = ka / a;
        let s = rigid_cylinder(ka, a);
        let want = 3.0 * PI * PI / 4.0 * k.powi(3) * a.powi(4);
        assert!((s.sigma_s / want - 1.0).abs() < 0.02, "{} vs {}", s.sigma_s, want);
    }

    #[test]
    fn geometric_limit() {
        let a = 0.2;
        let s = rigid_cylinder(60.0, a);
        assert!((s.sigma_s / (4.0 * a) - 1.0).abs() < 0.05, "sigma_s {}", s.sigma_s);
        assert!((s.sigma_tr / (8.0 * a / 3.0) - 1.0).abs() < 0.1, "sigma_tr {}", s.sigma_tr);
    }
}
