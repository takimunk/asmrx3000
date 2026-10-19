//! What the wind does to sound on its way: refraction by the wind-speed gradient and level
//! fluctuations by turbulence. Engineering estimates on the shaping grid, kept modest.
//!
//! **Refraction.** Sound travels at c + u(z)·cos φ (effective sound speed; φ between the
//! propagation direction and the wind). Wind grows with height (logarithmic profile
//! u(z) = U·ln(z/z₀)/ln(z_ref/z₀), z₀ = 0.1 m over vegetation, z_ref = 10 m), so
//!  * **downwind** (cos φ > 0) rays bend down: energy that would leave upwards returns, the
//!    ground-effect dip fills in; levels rise by a few dB at a few hundred metres. ISO 9613-2
//!    is defined for this "favourable" condition; its long-term correction C_met (0–5 dB) is
//!    roughly the downwind excess over average weather;
//!  * **upwind** (cos φ < 0) rays bend up and leave a **shadow zone** beyond
//!    r_s = √(2R)(√h_s + √h_r), R = c/|du_eff/dz| the ray radius of curvature (circular-ray
//!    approximation of a linear profile, gradient taken at the mean path height; Pierce 1981
//!    §8.3, Embleton 1996 §6, Salomons 2001 ch. 7). In the shadow only diffraction and
//!    turbulent scattering reach the listener: measured excess attenuation rises to 10–25 dB,
//!    strongest at high frequency (long waves diffract into the shadow) and limited by
//!    scattering (Wiener & Keast 1959; Embleton, Piercy & Olson 1976; Daigle 1979);
//!  * **crosswind** (cos φ ≈ 0) nothing: R → ∞, the shadow recedes to infinity.
//! Temperature gradients act the same way (day lapse = upwind everywhere, night inversion =
//! downwind everywhere); not modelled here, the atmosphere is taken as neutral.
//!
//! **Turbulence.** Wind-driven eddies scatter sound and make the received level wander. The
//! standard deviation grows with distance and turbulence strength (Rytov variance
//! ∝ C²·k^{7/6}·r^{11/6} in weak scattering) and saturates at 5.6 dB (Rayleigh statistics,
//! fully saturated scattering; Daigle et al. 1983, Wilson 2000, Salomons 2001 ch. 9). The
//! fluctuations are slow: the eddies are carried past the path by the wind (τ ≈ L/U).

use core::f64::consts::LN_2;

/// Aerodynamic roughness length, m (low vegetation / forest edge).
const Z0: f64 = 0.1;
/// Height the WIND control refers to, m.
const Z_REF: f64 = 10.0;
/// Deepest shadow, dB (turbulent scattering floor).
const SHADOW_MAX_DB: f64 = 20.0;
/// Frequency where the shadow reaches half its depth, Hz.
const SHADOW_F_HALF: f64 = 600.0;
/// Saturated level fluctuation, dB.
const SIGMA_SAT_DB: f32 = 5.6;

/// Path geometry seen by the wind.
#[derive(Clone, Copy, Debug)]
pub struct WindPath {
    /// Component of the unit propagation direction (source → listener) along the wind
    /// (+1 = straight downwind; steep paths give |cos| < 1 automatically).
    pub cos_phi: f64,
    /// Source–listener distance, m.
    pub r: f64,
    /// Source and listener heights above their ground, m.
    pub hs: f64,
    pub hr: f64,
    /// Wind speed at `Z_REF`, m/s.
    pub u: f64,
    pub c: f64,
}

/// How a path is affected: shadow penetration 0..1 and downwind enhancement weight 0..1.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Refraction {
    pub shadow: f64,
    pub favour: f64,
    /// Shadow-boundary distance, m (∞ downwind / calm).
    pub r_shadow: f64,
}

/// Effective sound-speed gradient (1/s) at the mean height of the path.
fn gradient(p: &WindPath) -> f64 {
    let z = (0.5 * (p.hs + p.hr)).clamp(1.0, 30.0);
    p.cos_phi * p.u / (z * (Z_REF / Z0).ln())
}

pub fn refraction(p: &WindPath) -> Refraction {
    if p.u < 0.3 || p.r < 5.0 {
        return Refraction { r_shadow: f64::INFINITY, ..Default::default() };
    }
    let a = gradient(p);
    let (hs, hr) = (p.hs.max(0.1), p.hr.max(0.1));
    if a < 0.0 {
        let radius = p.c / -a;
        let r_s = (2.0 * radius).sqrt() * (hs.sqrt() + hr.sqrt());
        // Penumbra from 0.6 r_s, nearly full shadow by 1.6 r_s.
        let q = (p.r / r_s - 0.6).max(0.0);
        Refraction { shadow: 1.0 - (-3.0 * q * q).exp(), favour: 0.0, r_shadow: r_s }
    } else {
        // Downward refraction matters once the rays' sag over the path (r²/8R) is comparable
        // with the heights: that is the same √(2R)(√h_s+√h_r) scale.
        let radius = p.c / a.max(1e-9);
        let r_c = (2.0 * radius).sqrt() * (hs.sqrt() + hr.sqrt());
        let x = p.r / r_c;
        Refraction { shadow: 0.0, favour: (x * x / (1.0 + x * x)).min(1.0), r_shadow: f64::INFINITY }
    }
}

/// Spectral excess gain of the path, dB (negative in the shadow), at `f` Hz.
pub fn excess_db(f: f64, rf: &Refraction) -> f64 {
    let mut db = 0.0;
    if rf.shadow > 0.0 {
        let x = f / SHADOW_F_HALF;
        db -= rf.shadow * SHADOW_MAX_DB * x / (1.0 + x);
    }
    if rf.favour > 0.0 {
        // Broadband gain plus the ground-effect dip filling in (~300–800 Hz over soft ground).
        let o = (f / 500.0).ln() / LN_2;
        db += rf.favour * (2.0 + 3.0 * (-o * o / (2.0 * 1.2 * 1.2)).exp());
    }
    db
}

/// Standard deviation of the turbulent level fluctuation, dB, for a path of `r` metres in a
/// wind of `u` m/s, deeper (scattering-dominated) in a shadow.
pub fn fluctuation_db(r: f32, u: f32, shadow: f32) -> f32 {
    if u <= 0.0 || r <= 0.0 {
        return 0.0;
    }
    let x = (r / 250.0).powf(0.92) * u / 6.0;
    SIGMA_SAT_DB * (x * (1.0 + 0.5 * shadow)).tanh()
}

/// Correlation time of the fluctuation, s: eddies of ~10 m carried past at U.
pub fn fluctuation_tau(u: f32) -> f32 {
    (10.0 / u.max(0.1)).clamp(0.5, 4.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(cos_phi: f64, r: f64, u: f64) -> WindPath {
        WindPath { cos_phi, r, hs: 1.5, hr: 1.5, u, c: 343.0 }
    }

    #[test]
    fn shadow_distance_shrinks_with_wind() {
        let r = |u| refraction(&path(-1.0, 500.0, u)).r_shadow;
        eprintln!("shadow boundary at 1.5 m heights: 3 m/s {:.0} m, 5 m/s {:.0} m, 10 m/s {:.0} m", r(3.0), r(5.0), r(10.0));
        assert!(r(10.0) < r(5.0) && r(5.0) < r(3.0));
        // Of the order of 100 m in a moderate wind (Embleton 1996: 'a few hundred metres'
        // over open ground at moderate gradients).
        assert!(r(5.0) > 50.0 && r(5.0) < 200.0);
        // Raised ends push it out.
        let high = refraction(&WindPath { hr: 20.0, ..path(-1.0, 500.0, 5.0) }).r_shadow;
        assert!(high > 1.8 * r(5.0));
    }

    #[test]
    fn upwind_darker_downwind_brighter_crosswind_neutral() {
        let at = |c: f64, f: f64| excess_db(f, &refraction(&path(c, 300.0, 6.0)));
        eprintln!("300 m, 6 m/s: upwind 125 Hz {:.1} / 1 kHz {:.1} / 4 kHz {:.1} dB; downwind 500 Hz {:+.1} dB; crosswind 1 kHz {:.2} dB",
            at(-1.0, 125.0), at(-1.0, 1000.0), at(-1.0, 4000.0), at(1.0, 500.0), at(0.0, 1000.0));
        assert!(at(-1.0, 4000.0) < -12.0 && at(-1.0, 4000.0) > -21.0);
        assert!(at(-1.0, 125.0) > at(-1.0, 4000.0) + 8.0);
        assert!(at(1.0, 500.0) > 2.0 && at(1.0, 500.0) < 6.0);
        assert!(at(0.0, 1000.0).abs() < 0.1);
    }

    #[test]
    fn fluctuations_grow_with_distance_and_saturate() {
        let s = |r| fluctuation_db(r, 6.0, 0.0);
        eprintln!("level fluctuation sd at 6 m/s: 10 m {:.2}, 100 m {:.2}, 300 m {:.2}, 2 km {:.2} dB", s(10.0), s(100.0), s(300.0), s(2000.0));
        assert!(s(10.0) < 0.5 && s(100.0) > 1.5 && s(300.0) > 3.0 && s(2000.0) <= SIGMA_SAT_DB);
    }
}
