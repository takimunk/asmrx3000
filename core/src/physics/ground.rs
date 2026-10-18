//! Ground reflection: locally reacting porous ground with the Miki (1990) impedance model
//! (a physically realisable refit of Delany & Bazley 1970), and the spherical-wave reflection
//! coefficient Q = Rp + (1 - Rp) F(w) (Chien & Soroka 1975 / Attenborough et al. 1980).
//!
//! Convention inside this module: physics time dependence e^{-i w t}. The DSP code wants
//! H(e^{jw}) with e^{+jwt}; callers conjugate (see `reflection_dsp`).

use crate::num::{faddeeva, C64};
use core::f64::consts::PI;

#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroundType {
    ForestFloor = 0,
    Grass = 1,
    Water = 2,
    CompactSoil = 3,
    Snow = 4,
    Asphalt = 5,
    /// Loose dry sand (dunes, beaches).
    Sand = 6,
    /// Bare rock (cliffs, crags, scree faces).
    Rock = 7,
}

pub const GROUND_TYPES: usize = 8;

impl GroundType {
    pub fn from_index(i: u32) -> Self {
        match i {
            1 => GroundType::Grass,
            2 => GroundType::Water,
            3 => GroundType::CompactSoil,
            4 => GroundType::Snow,
            5 => GroundType::Asphalt,
            6 => GroundType::Sand,
            7 => GroundType::Rock,
            _ => GroundType::ForestFloor,
        }
    }

    /// Effective flow resistivity in Pa s m^-2 (Embleton, Piercy & Daigle 1983; Attenborough,
    /// Li & Horoshenkov, "Predicting Outdoor Sound", Table 5.1). None = acoustically rigid.
    pub fn flow_resistivity(self) -> Option<f64> {
        match self {
            GroundType::ForestFloor => Some(40e3),  // pine/hemlock litter 20-80 kPa s/m^2
            GroundType::Grass => Some(200e3),       // pasture / lawn 150-300
            GroundType::Water => None,              // rho c of water is 3600x that of air
            GroundType::CompactSoil => Some(2000e3), // hard-packed dirt / sandy silt
            GroundType::Snow => Some(20e3),         // fresh snow 10-30
            GroundType::Asphalt => None,            // 20 MPa s/m^2 and up: rigid in practice
            GroundType::Sand => Some(250e3),        // loose dry sand 100-500 (dunes, beaches)
            GroundType::Rock => Some(3000e3),       // exposed rock 2000-4000: nearly rigid
        }
    }
}

/// Normalised characteristic impedance Z/(rho c) of a porous ground (Miki), e^{-iwt}.
pub fn impedance(f: f64, sigma: f64) -> C64 {
    let x = (1000.0 * f / sigma).max(1e-6);
    let p = x.powf(-0.632);
    C64::new(1.0 + 5.50 * p, 8.43 * p)
}

/// Normalised admittance beta = rho c / Z, or 0 for rigid ground.
pub fn admittance(ground: GroundType, f: f64) -> C64 {
    match ground.flow_resistivity() {
        Some(s) => impedance(f, s).inv(),
        None => C64::ZERO,
    }
}

/// Plane-wave reflection coefficient of a locally reacting surface at grazing angle psi.
pub fn plane_wave_reflection(beta: C64, sin_psi: f64) -> C64 {
    (C64::new(sin_psi, 0.0) - beta) / (C64::new(sin_psi, 0.0) + beta)
}

/// Spherical-wave reflection coefficient Q for wavenumber k, image path length r2 and grazing
/// angle psi, e^{-iwt}.
pub fn spherical_reflection(beta: C64, k: f64, r2: f64, sin_psi: f64) -> C64 {
    if beta == C64::ZERO {
        return C64::ONE;
    }
    let rp = plane_wave_reflection(beta, sin_psi);
    // Numerical distance w = (1 + i)/2 sqrt(k r2) (sin psi + beta).
    let w = C64::new(0.5, 0.5).scale((k * r2).sqrt()) * (beta + sin_psi);
    // Boundary-loss factor F(w) = 1 + i sqrt(pi) w W(w).
    let f = C64::ONE + C64::I * w * faddeeva(w) * PI.sqrt();
    rp + (C64::ONE - rp) * f
}

/// Q as a DSP transfer function value (e^{+jwt} convention).
pub fn reflection_dsp(beta: C64, k: f64, r2: f64, sin_psi: f64) -> C64 {
    spherical_reflection(beta, k, r2, sin_psi).conj()
}

/// Random-incidence (Paris) absorption coefficient of a locally reacting ground.
pub fn random_incidence_absorption(ground: GroundType, f: f64) -> f64 {
    let beta = admittance(ground, f);
    if beta == C64::ZERO {
        return 0.0;
    }
    // alpha = int_0^1 (1 - |R(mu)|^2) 2 mu dmu, mu = cos(theta from the normal).
    let n = 64;
    let mut s = 0.0;
    for i in 0..n {
        let mu = (i as f64 + 0.5) / n as f64;
        let r = plane_wave_reflection(beta, mu);
        s += (1.0 - r.norm_sqr()) * 2.0 * mu / n as f64;
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rigid_ground_reflects_fully() {
        let q = spherical_reflection(C64::ZERO, 10.0, 20.0, 0.1);
        assert_eq!(q, C64::ONE);
    }

    #[test]
    fn grazing_plane_wave_tends_to_minus_one() {
        let beta = admittance(GroundType::Grass, 500.0);
        let rp = plane_wave_reflection(beta, 1e-4);
        assert!((rp + C64::ONE).abs() < 1e-2, "{rp:?}");
    }

    #[test]
    fn spherical_coefficient_limits() {
        let beta = admittance(GroundType::Grass, 300.0);
        let k = 2.0 * PI * 300.0 / 343.0;
        // Far, high above: the ground wave vanishes and Q -> Rp.
        let q = spherical_reflection(beta, k * 100.0, 1000.0, 0.5);
        let rp = plane_wave_reflection(beta, 0.5);
        assert!((q - rp).abs() < 0.05, "q={q:?} rp={rp:?}");
        // Near grazing at short range the boundary-loss factor keeps |Q| well above |Rp|.
        let q = spherical_reflection(beta, k, 10.0, 0.05);
        let rp = plane_wave_reflection(beta, 0.05);
        assert!(q.abs() > rp.abs() + 0.1, "q={q:?} rp={rp:?}");
    }

    /// Attenborough et al. benchmark style check: classic grassland dip. Source 1 m, receiver
    /// 1.5 m, 50 m range over grass: the first ground-effect minimum of |1 + Q r1/r2 e^{ik dr}|
    /// lies at a few hundred Hz (far below the rigid-ground c/(2 dr)).
    #[test]
    fn grass_dip_is_low_frequency() {
        let (hs, hr, d) = (1.0f64, 1.5f64, 50.0f64);
        let r1 = (d * d + (hs - hr).powi(2)).sqrt();
        let r2 = (d * d + (hs + hr).powi(2)).sqrt();
        let sin_psi = (hs + hr) / r2;
        let mut best = (0.0, f64::MAX);
        let mut f = 50.0;
        while f < 3000.0 {
            let k = 2.0 * PI * f / 343.0;
            let q = spherical_reflection(admittance(GroundType::Grass, f), k, r2, sin_psi);
            let p = C64::ONE + q * C64::from_polar(r1 / r2, k * (r2 - r1));
            let level = 20.0 * p.abs().log10();
            if level < best.1 {
                best = (f, level);
            }
            f *= 1.01;
        }
        let rigid_dip = 343.0 / (2.0 * (r2 - r1));
        assert!(best.0 > 200.0 && best.0 < 900.0, "dip at {} Hz ({} dB)", best.0, best.1);
        assert!(best.0 < 0.5 * rigid_dip);
        assert!(best.1 < -10.0, "dip depth {}", best.1);
    }

    #[test]
    fn forest_floor_absorbs_more_at_high_frequency() {
        let lo = random_incidence_absorption(GroundType::ForestFloor, 125.0);
        let hi = random_incidence_absorption(GroundType::ForestFloor, 2000.0);
        assert!(lo < hi && hi > 0.5 && hi < 1.0, "lo {lo} hi {hi}");
        assert_eq!(random_incidence_absorption(GroundType::Water, 1000.0), 0.0);
    }
}
