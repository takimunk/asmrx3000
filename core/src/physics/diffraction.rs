//! Occlusion and diffraction by obstacles (trunks, boulders, ridges) from scalar
//! Fresnel-Kirchhoff theory with Babinet's principle.
//!
//! Each obstacle is projected onto the plane through it perpendicular to the source-listener
//! line and approximated there by a rectangle [x1,x2] x [z1,z2] (transverse offsets, metres).
//! The field relative to free field is
//!     U/U0 = 1 - Fx Fz,   Fq = (1-i)/2 [ (C(v2)-C(v1)) + i (S(v2)-S(v1)) ],
//!     v = q sqrt(2 (d1+d2) / (lambda d1 d2))
//! (C, S: Fresnel integrals). This single formula covers the lit zone (small ripples), the
//! shadow boundary (-6 dB for a half plane, the knife-edge value), the deep shadow (the
//! Fresnel-Kirchhoff knife-edge curve, close to Maekawa/Kurze-Anderson) and low-frequency
//! transparency of thin trunks (a 30 cm trunk does not shadow 100 Hz). Two-edge interference
//! (Arago spot) is band-averaged over 1/3 octave because the per-path filter is smooth.
//!
//! In the deep shadow sound arrives via the shortest edge detour; that extra path length is
//! returned separately so it can go into the propagation delay (continuous: 0 at the shadow
//! boundary).

use crate::math::Vec3;
use crate::num::{fresnel, C64};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Obstacle {
    /// Vertical cylinder (tree trunk): centre (x, y), radius, from z0 to z1 (world z).
    Cylinder { x: f32, y: f32, radius: f32, z0: f32, z1: f32 },
    /// Sphere (boulder).
    Sphere { center: Vec3, radius: f32 },
    /// Vertical thin screen (terrain ridge, wall, fallen log): from (x0,y0) to (x1,y1), z0..z1.
    Wall { x0: f32, y0: f32, x1: f32, y1: f32, z0: f32, z1: f32 },
}

/// Obstacle footprint seen from one source-listener pair.
#[derive(Clone, Copy, Debug)]
pub struct Aperture {
    pub d1: f64,
    pub d2: f64,
    pub x1: f64,
    pub x2: f64,
    pub z1: f64,
    pub z2: f64,
}

impl Aperture {
    /// sqrt(2 (d1+d2) / (d1 d2)) : multiply by sqrt(f/c) to get v per metre of offset.
    fn scale(&self) -> f64 {
        (2.0 * (self.d1 + self.d2) / (self.d1 * self.d2)).sqrt()
    }

    pub fn blocks_line_of_sight(&self) -> bool {
        self.x1 < 0.0 && self.x2 > 0.0 && self.z1 < 0.0 && self.z2 > 0.0
    }

    /// Extra path length (m) of the shortest detour when the line of sight is blocked.
    pub fn detour(&self) -> f64 {
        if !self.blocks_line_of_sight() {
            return 0.0;
        }
        let e = (-self.x1).min(self.x2).min(-self.z1).min(self.z2);
        e * e * (self.d1 + self.d2) / (2.0 * self.d1 * self.d2)
    }

    /// Complex field relative to free field at one frequency.
    pub fn field(&self, f: f64, c: f64) -> C64 {
        let s = self.scale() * (f / c).sqrt();
        let fq = |q1: f64, q2: f64| {
            let (c1, s1) = fresnel(q1 * s);
            let (c2, s2) = fresnel(q2 * s);
            C64::new(0.5, -0.5) * C64::new(c2 - c1, s2 - s1)
        };
        C64::ONE - fq(self.x1, self.x2) * fq(self.z1, self.z2)
    }

    /// |U/U0|^2 averaged over a 1/3-octave band around f.
    pub fn band_power(&self, f: f64, c: f64) -> f64 {
        const R: [f64; 3] = [0.890_898_718, 1.0, 1.122_462_048]; // 2^(-1/6), 1, 2^(1/6)
        R.iter().map(|r| self.field(f * r, c).norm_sqr()).sum::<f64>() / 3.0
    }

    /// Whether the obstacle is so far off-axis that it cannot matter below `f_max`.
    pub fn negligible(&self, f_max: f64, c: f64) -> bool {
        let v = self.scale() * (f_max / c).sqrt();
        let near = |a: f64, b: f64| if a > 0.0 { a } else if b < 0.0 { -b } else { 0.0 };
        // Beyond v ~ 6 the Fresnel integrals have converged (< 0.1 dB effect).
        near(self.x1, self.x2) * v > 6.0 || near(self.z1, self.z2) * v > 6.0
    }
}

const FAR: f64 = 1.0e4;

/// Projects an obstacle onto the plane perpendicular to the path s -> l.
/// `ground_z`: obstacles that touch the ground are extended to -infinity (the ground plane
/// itself is handled by the image source).
pub fn aperture(ob: &Obstacle, s: Vec3, l: Vec3, ground_z: f32) -> Option<Aperture> {
    let sd = [s.x as f64, s.y as f64, s.z as f64];
    let ld = [l.x as f64, l.y as f64, l.z as f64];
    let u = [ld[0] - sd[0], ld[1] - sd[1], ld[2] - sd[2]];
    let dist = (u[0] * u[0] + u[1] * u[1] + u[2] * u[2]).sqrt();
    if dist < 1e-3 {
        return None;
    }
    let u = [u[0] / dist, u[1] / dist, u[2] / dist];
    // Transverse basis: h horizontal perpendicular to the path, w = u x h ("up-ish").
    let hl = (u[0] * u[0] + u[1] * u[1]).sqrt();
    let h = if hl > 1e-6 { [-u[1] / hl, u[0] / hl, 0.0] } else { [0.0, 1.0, 0.0] };
    let w = [u[1] * h[2] - u[2] * h[1], u[2] * h[0] - u[0] * h[2], u[0] * h[1] - u[1] * h[0]];
    let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let rel = |p: [f64; 3]| [p[0] - sd[0], p[1] - sd[1], p[2] - sd[2]];
    let gz = ground_z as f64;
    let on_ground = |z0: f32| (z0 as f64) <= gz + 0.05;

    let (t, x1, x2, z1, z2) = match *ob {
        Obstacle::Cylinder { x, y, radius, z0, z1 } => {
            let c = rel([x as f64, y as f64, 0.0]);
            // Along-path position from the horizontal projection.
            let t = if hl > 1e-6 { (c[0] * u[0] + c[1] * u[1]) / (hl * hl) / dist } else { return None };
            let e = dot(c, h);
            let zl = sd[2] + t * dist * u[2];
            // Vertical extents projected onto w (w.z = cos(elevation)).
            let wz = w[2].abs().max(1e-3);
            let bot = if on_ground(z0) { -FAR } else { (z0 as f64 - zl) * wz };
            let top = (z1 as f64 - zl) * wz;
            let r = radius as f64;
            (t, e - r, e + r, bot, top)
        }
        Obstacle::Sphere { center, radius } => {
            let c = rel([center.x as f64, center.y as f64, center.z as f64]);
            let t = dot(c, u) / dist;
            let half = radius as f64 * 0.886_226_925; // equal-area square: r sqrt(pi)/2
            let (e, f) = (dot(c, h), dot(c, w));
            let bot = if (center.z - radius) as f64 <= gz + 0.05 { -FAR } else { f - half };
            (t, e - half, e + half, bot, f + half)
        }
        Obstacle::Wall { x0, y0, x1, y1, z0, z1 } => {
            // Intersect the horizontal projection of the path with the wall line.
            let (px, py) = (x0 as f64 - sd[0], y0 as f64 - sd[1]);
            let (qx, qy) = (x1 as f64 - x0 as f64, y1 as f64 - y0 as f64);
            let (ux, uy) = (u[0] * dist, u[1] * dist);
            let den = ux * qy - uy * qx;
            if den.abs() < 1e-9 {
                return None;
            }
            let t = (px * qy - py * qx) / den;
            let e0 = dot(rel([x0 as f64, y0 as f64, 0.0]), h);
            let e1 = dot(rel([x1 as f64, y1 as f64, 0.0]), h);
            let zl = sd[2] + t * dist * u[2];
            let wz = w[2].abs().max(1e-3);
            let bot = if on_ground(z0) { -FAR } else { (z0 as f64 - zl) * wz };
            (t, e0.min(e1), e0.max(e1), bot, (z1 as f64 - zl) * wz)
        }
    };
    if !(0.0..=1.0).contains(&t) {
        return None;
    }
    let d1 = (t * dist).max(0.05);
    let d2 = ((1.0 - t) * dist).max(0.05);
    Some(Aperture { d1, d2, x1, x2, z1, z2 })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn half_plane(edge: f64) -> Aperture {
        // Screen covering x < edge, infinite in z.
        Aperture { d1: 10.0, d2: 10.0, x1: -FAR, x2: edge, z1: -FAR, z2: FAR }
    }

    #[test]
    fn knife_edge_shadow_boundary_is_minus_6_db() {
        let p = half_plane(0.0).field(1000.0, 343.0).norm_sqr();
        assert!((10.0 * p.log10() + 6.02).abs() < 0.2, "{}", 10.0 * p.log10());
    }

    /// Deep shadow vs the ITU-R P.526 knife-edge approximation
    /// J(v) = 6.9 + 20 log10(sqrt((v-0.1)^2 + 1) + v - 0.1).
    #[test]
    fn knife_edge_matches_itu_r_p526() {
        let ap = half_plane(0.5);
        for f in [250.0, 1000.0, 4000.0] {
            let v = 0.5 * ap.scale() * (f / 343.0f64).sqrt();
            let want = 6.9 + 20.0 * (((v - 0.1) * (v - 0.1) + 1.0).sqrt() + v - 0.1).log10();
            let got = -20.0 * ap.field(f, 343.0).abs().log10();
            assert!((got - want).abs() < 1.0, "f={f} v={v:.2}: {got:.2} vs {want:.2}");
        }
    }

    #[test]
    fn thin_trunk_is_transparent_at_low_frequency() {
        let trunk = Obstacle::Cylinder { x: 10.0, y: 0.0, radius: 0.15, z0: -1.5, z1: 10.0 };
        let s = Vec3::new(20.0, 0.0, 0.0);
        let l = Vec3::new(0.0, 0.0, 0.0);
        let ap = aperture(&trunk, s, l, -1.5).unwrap();
        assert!(ap.blocks_line_of_sight());
        let lo = 10.0 * ap.band_power(100.0, 343.0).log10();
        let hi = 10.0 * ap.band_power(8000.0, 343.0).log10();
        assert!(lo > -1.0, "100 Hz: {lo} dB");
        assert!(hi < -3.0, "8 kHz: {hi} dB");
        // Listener right behind the trunk: deep high-frequency shadow.
        let near = Obstacle::Cylinder { x: 0.6, y: 0.0, radius: 0.15, z0: -1.5, z1: 10.0 };
        let ap = aperture(&near, Vec3::new(20.0, 0.0, 0.0), l, -1.5).unwrap();
        let hi = 10.0 * ap.band_power(8000.0, 343.0).log10();
        assert!(hi < -8.0, "close trunk 8 kHz: {hi} dB");
    }

    #[test]
    fn obstacle_behind_listener_is_ignored() {
        let trunk = Obstacle::Cylinder { x: -3.0, y: 0.0, radius: 0.3, z0: -1.5, z1: 10.0 };
        assert!(aperture(&trunk, Vec3::new(10.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0), -1.5).is_none());
    }
}
