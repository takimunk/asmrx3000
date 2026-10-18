//! Ambisonics -> binaural decoder.
//!
//! M0 uses an analytic HRTF: the Brown & Duda (1998) structural model of a spherical head
//! (Woodworth ITD + one-pole/one-zero head shadow + pinna echoes). It needs no measured data
//! and gets the main cues right: ITD, ILD, a front/back and elevation colouring.
//! M1 replaces `hrir()` with measured SOFA HRTFs (SADIE II / HUTUBS); nothing else changes.
//!
//! The decoder is folded into the SH domain: for each of the 16 Ambisonic channels and each
//! ear we precompute one FIR = sum over virtual speakers of (decoder gain x speaker HRIR).
//! Runtime cost is 32 short FIRs per sample, independent of the number of sources.

use core::f32::consts::{FRAC_PI_2, PI};

use crate::ambisonics::{self, CHANNELS};
use crate::dsp::dot;
use crate::engine::BLOCK;
use crate::math::Vec3;

pub const TAPS: usize = 128;

const HEAD_RADIUS: f32 = 0.0875;
const SPEED_OF_SOUND: f32 = 343.0;
const SINC_HALF: f32 = 8.0;
const VIRTUAL_SPEAKERS: usize = 50;

// Brown & Duda head-shadow parameters.
const ALPHA_MIN: f32 = 0.1;
const THETA_MIN: f32 = 150.0 * PI / 180.0;

// Brown & Duda pinna echo model (delays in samples at 44.1 kHz).
const PINNA_RHO: [f32; 5] = [0.5, -1.0, 0.5, -0.25, 0.25];
const PINNA_A: [f32; 5] = [1.0, 5.0, 5.0, 5.0, 5.0];
const PINNA_B: [f32; 5] = [2.0, 4.0, 7.0, 11.0, 13.0];
const PINNA_D: [f32; 5] = [1.0, 0.5, 0.5, 0.5, 0.5];
const PINNA_GAIN: f32 = 0.5;

/// Near-uniform points on the unit sphere.
pub fn fibonacci_sphere(n: usize) -> Vec<Vec3> {
    let golden = PI * (3.0 - 5.0f32.sqrt());
    (0..n)
        .map(|i| {
            let z = 1.0 - 2.0 * (i as f32 + 0.5) / n as f32;
            let r = (1.0 - z * z).max(0.0).sqrt();
            let phi = golden * i as f32;
            Vec3::new(r * phi.cos(), r * phi.sin(), z)
        })
        .collect()
}

fn sinc(t: f32) -> f32 {
    if t.abs() < 1e-6 {
        1.0
    } else {
        (PI * t).sin() / (PI * t)
    }
}

/// Head-related impulse response for a head-frame direction (Brown & Duda model).
pub fn hrir(dir: Vec3, left: bool, sample_rate: f32) -> [f32; TAPS] {
    let a_c = HEAD_RADIUS / SPEED_OF_SOUND;
    let ear = Vec3::new(0.0, if left { 1.0 } else { -1.0 }, 0.0);
    // Angle between the source and the ear axis: 0 = straight into this ear.
    let theta = dir.dot(ear).clamp(-1.0, 1.0).acos();

    // Woodworth ray-tracing ITD, shifted so the earliest possible arrival is t = 0.
    let itd = if theta < FRAC_PI_2 { -a_c * theta.cos() } else { a_c * (theta - FRAC_PI_2) };
    let delay = (itd + a_c) * sample_rate + SINC_HALF;

    // Fractional delay as a Hann-windowed sinc.
    let mut h = [0.0f32; TAPS];
    for (n, v) in h.iter_mut().enumerate() {
        let t = n as f32 - delay;
        if t.abs() < SINC_HALF {
            let w = 0.5 + 0.5 * (PI * t / SINC_HALF).cos();
            *v = sinc(t) * w;
        }
    }

    // Head shadow: H(s) = (1 + alpha*tau*s) / (1 + tau*s), tau = a / (2c), bilinear transform.
    let alpha = (1.0 + ALPHA_MIN / 2.0) + (1.0 - ALPHA_MIN / 2.0) * (theta / THETA_MIN * PI).cos();
    let tau = a_c / 2.0;
    let k = 2.0 * sample_rate;
    let norm = 1.0 + tau * k;
    let b0 = (1.0 + alpha * tau * k) / norm;
    let b1 = (1.0 - alpha * tau * k) / norm;
    let a1 = (1.0 - tau * k) / norm;
    let (mut x1, mut y1) = (0.0f32, 0.0f32);
    for v in h.iter_mut() {
        let x = *v;
        let y = b0 * x + b1 * x1 - a1 * y1;
        x1 = x;
        y1 = y;
        *v = y;
    }

    // Pinna echoes: delays depend on azimuth (front/back) and elevation.
    let azimuth = dir.y.atan2(dir.x);
    let elevation = dir.z.clamp(-1.0, 1.0).asin();
    let scale = sample_rate / 44_100.0;
    let src = h;
    for e in 0..PINNA_RHO.len() {
        let tau_e = (PINNA_A[e] * (azimuth / 2.0).cos() * (PINNA_D[e] * (FRAC_PI_2 - elevation)).sin()
            + PINNA_B[e])
            * scale;
        let whole = tau_e.floor() as usize;
        let frac = tau_e - tau_e.floor();
        let rho = PINNA_RHO[e] * PINNA_GAIN;
        for n in (whole + 1)..TAPS {
            let s = src[n - whole] * (1.0 - frac) + src[n - whole - 1] * frac;
            h[n] += rho * s;
        }
    }
    h
}

fn invert(mut a: Vec<Vec<f64>>) -> Vec<Vec<f64>> {
    let n = a.len();
    let mut inv: Vec<Vec<f64>> = (0..n).map(|i| (0..n).map(|j| (i == j) as u8 as f64).collect()).collect();
    for col in 0..n {
        let pivot = (col..n).max_by(|&i, &j| a[i][col].abs().total_cmp(&a[j][col].abs())).unwrap();
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
    inv
}

pub struct BinauralDecoder {
    /// Index `ch * 2 + ear` (0 = left), time-reversed so convolution is a plain dot product.
    filters: Vec<[f32; TAPS]>,
    /// Doubled history per channel so the last TAPS samples are always one contiguous slice.
    history: Vec<[f32; 2 * TAPS]>,
    pos: usize,
}

impl BinauralDecoder {
    pub fn new(sample_rate: f32) -> Self {
        let speakers = fibonacci_sphere(VIRTUAL_SPEAKERS);
        let l = speakers.len();

        // Y: per speaker, its SH vector.
        let y: Vec<[f64; CHANNELS]> = speakers
            .iter()
            .map(|d| {
                let mut sh = [0.0f32; CHANNELS];
                ambisonics::encode(*d, &mut sh);
                sh.map(|v| v as f64)
            })
            .collect();

        // Mode-matching decoder D = Y^T (Y Y^T)^-1.
        let mut g = vec![vec![0.0f64; CHANNELS]; CHANNELS];
        for row in &y {
            for i in 0..CHANNELS {
                for j in 0..CHANNELS {
                    g[i][j] += row[i] * row[j];
                }
            }
        }
        let ginv = invert(g);
        let weights = ambisonics::max_re_weights();

        let mut acc = vec![[0.0f64; TAPS]; CHANNELS * 2];
        for s in 0..l {
            let hl = hrir(speakers[s], true, sample_rate);
            let hr = hrir(speakers[s], false, sample_rate);
            for ch in 0..CHANNELS {
                let d: f64 = (0..CHANNELS).map(|j| y[s][j] * ginv[j][ch]).sum::<f64>()
                    * weights[ambisonics::order_of(ch)] as f64;
                for t in 0..TAPS {
                    acc[ch * 2][t] += d * hl[t] as f64;
                    acc[ch * 2 + 1][t] += d * hr[t] as f64;
                }
            }
        }

        let filters = acc
            .iter()
            .map(|h| {
                let mut rev = [0.0f32; TAPS];
                for j in 0..TAPS {
                    rev[j] = h[TAPS - 1 - j] as f32;
                }
                rev
            })
            .collect();

        Self { filters, history: vec![[0.0; 2 * TAPS]; CHANNELS], pos: 0 }
    }

    pub fn process(
        &mut self,
        bus: &[[f32; BLOCK]; CHANNELS],
        len: usize,
        left: &mut [f32],
        right: &mut [f32],
    ) {
        for i in 0..len {
            self.pos = (self.pos + 1) % TAPS;
            let p = self.pos;
            let (mut l, mut r) = (0.0f32, 0.0f32);
            for ch in 0..CHANNELS {
                let h = &mut self.history[ch];
                let x = bus[ch][i];
                h[p] = x;
                h[p + TAPS] = x;
                let window = &h[p + 1..p + 1 + TAPS];
                l += dot(window, &self.filters[ch * 2]);
                r += dot(window, &self.filters[ch * 2 + 1]);
            }
            left[i] = l;
            right[i] = r;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Energy reaching (left, right) ear for an impulse from `dir`.
    fn ear_energy(dir: Vec3) -> (f64, f64) {
        let mut dec = BinauralDecoder::new(48_000.0);
        let mut sh = [0.0f32; CHANNELS];
        ambisonics::encode(dir, &mut sh);
        let mut bus = [[0.0f32; BLOCK]; CHANNELS];
        for ch in 0..CHANNELS {
            bus[ch][0] = sh[ch];
        }
        let (mut el, mut er) = (0.0f64, 0.0f64);
        let mut l = [0.0f32; BLOCK];
        let mut r = [0.0f32; BLOCK];
        for block in 0..4 {
            dec.process(&bus, BLOCK, &mut l, &mut r);
            el += l.iter().map(|v| (*v as f64).powi(2)).sum::<f64>();
            er += r.iter().map(|v| (*v as f64).powi(2)).sum::<f64>();
            if block == 0 {
                bus = [[0.0f32; BLOCK]; CHANNELS];
            }
        }
        (el, er)
    }

    #[test]
    fn interaural_level_difference() {
        let (l, r) = ear_energy(Vec3::new(0.0, 1.0, 0.0));
        assert!(l > 2.0 * r, "left source: L={l} R={r}");
        let (l, r) = ear_energy(Vec3::new(0.0, -1.0, 0.0));
        assert!(r > 2.0 * l, "right source: L={l} R={r}");
        let (l, r) = ear_energy(Vec3::new(1.0, 0.0, 0.0));
        assert!((l / r - 1.0).abs() < 0.1, "front source: L={l} R={r}");
    }

    #[test]
    fn hrir_has_unity_dc_gain() {
        let h = hrir(Vec3::new(1.0, 0.0, 0.0), true, 48_000.0);
        let dc: f32 = h.iter().sum();
        assert!((dc - 1.0).abs() < 0.05, "dc = {dc}");
    }
}
