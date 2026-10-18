//! Small vector/matrix helpers.
//!
//! Coordinate convention everywhere in the engine (world and head frame):
//! +x forward, +y left, +z up (right-handed, same as Ambisonics ACN/SN3D/N3D).

use core::ops::{Add, Mul, Sub};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3 {
    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    #[inline]
    pub fn dot(self, o: Vec3) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }

    #[inline]
    pub fn length(self) -> f32 {
        self.dot(self).sqrt()
    }
}

impl Add for Vec3 {
    type Output = Vec3;
    fn add(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}

impl Sub for Vec3 {
    type Output = Vec3;
    fn sub(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}

impl Mul<f32> for Vec3 {
    type Output = Vec3;
    fn mul(self, s: f32) -> Vec3 {
        Vec3::new(self.x * s, self.y * s, self.z * s)
    }
}

/// Row-major 3x3 rotation matrix.
#[derive(Clone, Copy, Debug)]
pub struct Mat3 {
    pub m: [[f32; 3]; 3],
}

impl Mat3 {
    pub fn rot_x(a: f32) -> Self {
        let (s, c) = a.sin_cos();
        Self { m: [[1.0, 0.0, 0.0], [0.0, c, -s], [0.0, s, c]] }
    }

    pub fn rot_y(a: f32) -> Self {
        let (s, c) = a.sin_cos();
        Self { m: [[c, 0.0, s], [0.0, 1.0, 0.0], [-s, 0.0, c]] }
    }

    pub fn rot_z(a: f32) -> Self {
        let (s, c) = a.sin_cos();
        Self { m: [[c, -s, 0.0], [s, c, 0.0], [0.0, 0.0, 1.0]] }
    }

    pub fn mul(&self, o: &Mat3) -> Mat3 {
        let mut r = [[0.0f32; 3]; 3];
        for (i, row) in r.iter_mut().enumerate() {
            for (j, cell) in row.iter_mut().enumerate() {
                *cell = (0..3).map(|k| self.m[i][k] * o.m[k][j]).sum();
            }
        }
        Mat3 { m: r }
    }

    #[allow(dead_code)] // used by tests and upcoming propagation code
    #[inline]
    pub fn apply(&self, v: Vec3) -> Vec3 {
        let m = &self.m;
        Vec3::new(
            m[0][0] * v.x + m[0][1] * v.y + m[0][2] * v.z,
            m[1][0] * v.x + m[1][1] * v.y + m[1][2] * v.z,
            m[2][0] * v.x + m[2][1] * v.y + m[2][2] * v.z,
        )
    }

    /// Multiplies by the transpose (= inverse for rotations).
    #[inline]
    pub fn apply_transposed(&self, v: Vec3) -> Vec3 {
        let m = &self.m;
        Vec3::new(
            m[0][0] * v.x + m[1][0] * v.y + m[2][0] * v.z,
            m[0][1] * v.x + m[1][1] * v.y + m[2][1] * v.z,
            m[0][2] * v.x + m[1][2] * v.y + m[2][2] * v.z,
        )
    }

    /// Head orientation (head frame -> world frame).
    /// yaw > 0 turns left, pitch > 0 looks up, roll > 0 lowers the right ear.
    pub fn from_head(yaw: f32, pitch: f32, roll: f32) -> Mat3 {
        Mat3::rot_z(yaw).mul(&Mat3::rot_y(-pitch)).mul(&Mat3::rot_x(roll))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Vec3, b: Vec3) -> bool {
        (a - b).length() < 1e-5
    }

    #[test]
    fn head_rotation_conventions() {
        let fwd = Vec3::new(1.0, 0.0, 0.0);
        let half = core::f32::consts::FRAC_PI_2;
        // Turning left by 90 deg: the nose points to world +y.
        assert!(close(Mat3::from_head(half, 0.0, 0.0).apply(fwd), Vec3::new(0.0, 1.0, 0.0)));
        // Looking up by 90 deg: the nose points to world +z.
        assert!(close(Mat3::from_head(0.0, half, 0.0).apply(fwd), Vec3::new(0.0, 0.0, 1.0)));
        // A source straight ahead appears on the right after turning left.
        let rel = Mat3::from_head(half, 0.0, 0.0).apply_transposed(fwd);
        assert!(close(rel, Vec3::new(0.0, -1.0, 0.0)));
    }
}
