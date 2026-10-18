//! Rotation of the Ambisonic bus (SH domain), so that everything on the bus -- direct
//! sources, reflections and the diffuse forest tail -- turns with the listener's head.
//!
//! Real SH of order <= N span a rotation-invariant space: y(R d) = D(R) y(d) with D block
//! diagonal (one (2l+1)^2 block per order). Instead of the Ivanic-Ruedenberg recursion we
//! obtain D exactly by projection: for a fixed set of Q >= 16 directions with full-rank
//! Y (Q x 16), D^T = pinv(Y) Y_R where Y_R holds y(R d_q). This is exact for order-limited
//! functions (no quadrature error), independent of sign/normalisation conventions, and costs
//! ~3k MAC per head update. Within a block the matrix is interpolated per sample.

use crate::ambisonics::{self, order_of, CHANNELS};
use crate::binaural::fibonacci_sphere;
use crate::engine::BLOCK;
use crate::math::{Mat3, Vec3};
use crate::num::pinv;

const Q: usize = 36;

pub type ShMatrix = [[f32; CHANNELS]; CHANNELS];

pub struct ShRotator {
    dirs: Vec<Vec3>,
    /// 16 x Q
    pinv: Vec<[f64; Q]>,
    current: ShMatrix,
}

pub fn identity() -> ShMatrix {
    let mut m = [[0.0f32; CHANNELS]; CHANNELS];
    for (i, row) in m.iter_mut().enumerate() {
        row[i] = 1.0;
    }
    m
}

impl ShRotator {
    pub fn new() -> Self {
        let dirs = fibonacci_sphere(Q);
        let y: Vec<Vec<f64>> = dirs
            .iter()
            .map(|d| {
                let mut s = [0.0f32; CHANNELS];
                ambisonics::encode(*d, &mut s);
                s.iter().map(|v| *v as f64).collect()
            })
            .collect();
        let p = pinv(&y, None, 0.0);
        let pinv = p.iter().map(|row| core::array::from_fn(|q| row[q])).collect();
        Self { dirs, pinv, current: identity() }
    }

    /// D such that D y(d) = y(R d).
    pub fn matrix(&self, r: &Mat3) -> ShMatrix {
        let mut yr = [[0.0f32; CHANNELS]; Q];
        for (q, d) in self.dirs.iter().enumerate() {
            ambisonics::encode(r.apply(*d), &mut yr[q]);
        }
        let mut m = [[0.0f32; CHANNELS]; CHANNELS];
        for i in 0..CHANNELS {
            for j in 0..CHANNELS {
                if order_of(i) != order_of(j) {
                    continue;
                }
                // D[i][j] = (D^T)[j][i] = sum_q pinv[j][q] yr[q][i]
                let s: f64 = (0..Q).map(|q| self.pinv[j][q] * yr[q][i] as f64).sum();
                m[i][j] = s as f32;
            }
        }
        m
    }

    /// Rotates the bus in place, interpolating from the previous block's matrix to `next`.
    pub fn apply(&mut self, bus: &mut [[f32; BLOCK]; CHANNELS], len: usize, next: ShMatrix) {
        let prev = self.current;
        let inv = 1.0 / len.max(1) as f32;
        let mut x = [0.0f32; CHANNELS];
        for n in 0..len {
            let t = (n + 1) as f32 * inv;
            for ch in 0..CHANNELS {
                x[ch] = bus[ch][n];
            }
            bus[0][n] = x[0];
            for l in 1..=ambisonics::ORDER {
                let lo = l * l;
                let hi = (l + 1) * (l + 1);
                for i in lo..hi {
                    let mut acc = 0.0f32;
                    for j in lo..hi {
                        let m = prev[i][j] + (next[i][j] - prev[i][j]) * t;
                        acc += m * x[j];
                    }
                    bus[i][n] = acc;
                }
            }
        }
        self.current = next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotating_the_bus_equals_rotating_the_source() {
        let rot = ShRotator::new();
        let r = Mat3::from_head(0.7, -0.3, 0.2);
        let m = rot.matrix(&r);
        for d in [Vec3::new(1.0, 0.0, 0.0), Vec3::new(0.3, -0.6, 0.74), Vec3::new(-0.5, 0.5, -0.707)] {
            let len = d.length();
            let d = d * (1.0 / len);
            let mut y = [0.0f32; CHANNELS];
            let mut want = [0.0f32; CHANNELS];
            ambisonics::encode(d, &mut y);
            ambisonics::encode(r.apply(d), &mut want);
            for i in 0..CHANNELS {
                let got: f32 = (0..CHANNELS).map(|j| m[i][j] * y[j]).sum();
                assert!((got - want[i]).abs() < 1e-4, "ch {i}: {got} vs {}", want[i]);
            }
        }
    }
}
