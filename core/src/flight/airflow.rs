//! Wind noise of motion: the turbulent airflow over the head and pinnae of someone moving
//! at speed U through still air.
//!
//! Flow separation at the helix and the head sheds vortices at f ≈ St·U/d (St ≈ 0.2, edge
//! scale d ≈ 1 cm, so ~20·U Hz: 120 Hz at 6 m/s, 500 Hz at 25 m/s) with a broadband
//! turbulent spectrum falling ~12 dB/octave above it. The radiated pressure of these
//! aerodynamic dipoles grows like U^3 (power ~U^6); the level saturates gently at high speed
//! (the ear canal is partly shielded by the tragus). The ear facing the flow hears more of
//! it, so the balance follows the head yaw relative to the direction of travel. It is
//! head-locked (it happens at the ears), so it bypasses the spatial renderer.

use crate::rng::Rng;

pub struct Airflow {
    rng: Rng,
    lp: [[f32; 2]; 2],
    hiss: [f32; 2],
    gust: [f32; 2],
    target: [f32; 2],
    timer: f32,
    level: f32,
}

/// Output RMS at U = 10 m/s and amount 1 (about -44 dBFS: subtle).
const A10: f32 = 0.006;

impl Airflow {
    pub fn new(seed: u64) -> Self {
        Self { rng: Rng::new(seed ^ 0xA1F7), lp: [[0.0; 2]; 2], hiss: [0.0; 2], gust: [1.0; 2], target: [1.0; 2], timer: 0.0, level: 0.0 }
    }

    /// Target RMS for airspeed `u` (m/s) and user amount.
    pub fn rms(u: f32, amount: f32) -> f32 {
        let x = (u / 10.0).max(0.0);
        let x3 = x * x * x;
        amount.max(0.0) * A10 * x3 / (1.0 + x3 / 8.0)
    }

    /// Adds the noise to `left`/`right`. `yaw`: head yaw relative to the direction of
    /// travel (+ = turned left), radians.
    pub fn render(&mut self, u: f32, amount: f32, yaw: f32, sr: f32, left: &mut [f32], right: &mut [f32]) {
        let len = left.len().min(right.len());
        let target = Self::rms(u, amount);
        let dt = len as f32 / sr;
        self.level += (target - self.level) * (1.0 - (-dt / 0.3).exp());
        if self.level < 1e-6 {
            return;
        }
        // Turbulent gustiness: a new random level per ear every 0.15..0.6 s, smoothed.
        self.timer -= dt;
        if self.timer <= 0.0 {
            self.timer = self.rng.range(0.15, 0.6);
            for t in self.target.iter_mut() {
                *t = (0.35 * self.rng.gauss()).exp();
            }
        }
        let g = 1.0 - (-dt / 0.12).exp();
        for e in 0..2 {
            self.gust[e] += (self.target[e] - self.gust[e]) * g;
        }
        // Flow arrives from straight ahead in the body frame, i.e. from azimuth -yaw in the
        // head frame; the windward ear (+90° = left) gets more.
        let side = (-yaw).sin();
        let exposure = [0.65 + 0.35 * side, 0.65 - 0.35 * side];
        let fc = (20.0 * u).clamp(40.0, 0.2 * sr);
        let p = (-core::f32::consts::TAU * fc / sr).exp();
        // Unit-RMS normalisation of white noise through two one-pole stages: the first passes
        // a variance fraction (1-p)/(1+p), the second (already band-limited input) about half.
        let norm = ((1.0 + p) / (1.0 - p).max(1e-6)).sqrt() * core::f32::consts::SQRT_2;
        let hp = (-core::f32::consts::TAU * 3000.0 / sr).exp();
        for i in 0..len {
            for e in 0..2 {
                let w = self.rng.gauss();
                let s = &mut self.lp[e];
                s[0] = (1.0 - p) * w + p * s[0];
                s[1] = (1.0 - p) * s[0] + p * s[1];
                // A little hiss from the small-scale turbulence (high-passed white noise).
                self.hiss[e] = hp * self.hiss[e] + (1.0 - hp) * w;
                let v = s[1] * norm + 0.12 * (w - self.hiss[e]);
                let out = v * self.level * self.gust[e] * exposure[e];
                if e == 0 {
                    left[i] += out;
                } else {
                    right[i] += out;
                }
            }
        }
    }
}
