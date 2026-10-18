//! Third-order Ambisonics: real spherical harmonics, ACN channel order, N3D normalisation.
//!
//! Every source is encoded into this 16-channel bus. Head rotation and binaural decoding
//! then cost the same no matter how many sources exist.

use crate::math::Vec3;

pub const ORDER: usize = 3;
pub const CHANNELS: usize = (ORDER + 1) * (ORDER + 1);

/// Encodes a unit direction into N3D real spherical harmonics (ACN order).
#[inline]
pub fn encode(d: Vec3, out: &mut [f32; CHANNELS]) {
    let (x, y, z) = (d.x, d.y, d.z);
    let (x2, y2, z2) = (x * x, y * y, z * z);

    const S3: f32 = 1.732_050_8; // sqrt(3)
    const S15: f32 = 3.872_983_3; // sqrt(15)
    const S5_2: f32 = 1.118_034; // sqrt(5)/2
    const S15_2: f32 = 1.936_491_7; // sqrt(15)/2
    const S35_8: f32 = 2.091_650_1; // sqrt(35/8)
    const S105: f32 = 10.246_951; // sqrt(105)
    const S21_8: f32 = 1.620_185_2; // sqrt(21/8)
    const S7_2: f32 = 1.322_875_7; // sqrt(7)/2
    const S105_2: f32 = 5.123_475_4; // sqrt(105)/2

    out[0] = 1.0;
    out[1] = S3 * y;
    out[2] = S3 * z;
    out[3] = S3 * x;
    out[4] = S15 * x * y;
    out[5] = S15 * y * z;
    out[6] = S5_2 * (3.0 * z2 - 1.0);
    out[7] = S15 * x * z;
    out[8] = S15_2 * (x2 - y2);
    out[9] = S35_8 * y * (3.0 * x2 - y2);
    out[10] = S105 * x * y * z;
    out[11] = S21_8 * y * (5.0 * z2 - 1.0);
    out[12] = S7_2 * z * (5.0 * z2 - 3.0);
    out[13] = S21_8 * x * (5.0 * z2 - 1.0);
    out[14] = S105_2 * z * (x2 - y2);
    out[15] = S35_8 * x * (x2 - 3.0 * y2);
}

/// Ambisonic order of an ACN channel.
#[inline]
pub fn order_of(acn: usize) -> usize {
    (acn as f32).sqrt() as usize
}

/// max-rE per-order weights: trade a little spatial sharpness for a much more stable,
/// less phasey image (Zotter & Frank).
pub fn max_re_weights() -> [f32; ORDER + 1] {
    let re = (137.9f32.to_radians() / (ORDER as f32 + 1.51)).cos();
    let p = |n: usize, x: f32| match n {
        0 => 1.0,
        1 => x,
        2 => 0.5 * (3.0 * x * x - 1.0),
        _ => 0.5 * (5.0 * x * x * x - 3.0 * x),
    };
    [p(0, re), p(1, re), p(2, re), p(3, re)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::binaural::fibonacci_sphere;

    #[test]
    fn spherical_harmonics_are_orthonormal() {
        let pts = fibonacci_sphere(20_000);
        let mut g = [[0.0f64; CHANNELS]; CHANNELS];
        let mut y = [0.0f32; CHANNELS];
        for p in &pts {
            encode(*p, &mut y);
            for i in 0..CHANNELS {
                for j in 0..CHANNELS {
                    g[i][j] += (y[i] * y[j]) as f64;
                }
            }
        }
        for i in 0..CHANNELS {
            for j in 0..CHANNELS {
                let v = g[i][j] / pts.len() as f64;
                let want = if i == j { 1.0 } else { 0.0 };
                assert!((v - want).abs() < 0.02, "G[{i}][{j}] = {v}");
            }
        }
    }

    #[test]
    fn order_mapping() {
        assert_eq!(order_of(0), 0);
        assert_eq!(order_of(3), 1);
        assert_eq!(order_of(4), 2);
        assert_eq!(order_of(8), 2);
        assert_eq!(order_of(9), 3);
        assert_eq!(order_of(15), 3);
    }
}
