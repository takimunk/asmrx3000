//! Global wind field.
//!
//! Gusts are an Ornstein-Uhlenbeck process around the mean wind. The field is not uniform:
//! a gust front travels across the scene from the listener's left (+y) to the right (-y),
//! so every tree, wind source and the UI animation sees the same gust, delayed by its position.

use crate::engine::{BLOCK, WIND_SPAN};
use crate::rng::Rng;

const HISTORY: usize = 16_384; // ~21 s at 750 Hz control rate
const GUST_TIMESCALE: f32 = 2.8; // seconds

pub struct WindField {
    history: Vec<f32>,
    pos: usize,
    deviation: f32,
    gust: f32,
    front_speed: f32,
    control_rate: f32,
}

impl WindField {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            history: vec![0.0; HISTORY],
            pos: 0,
            deviation: 0.0,
            gust: 0.0,
            front_speed: 4.0,
            control_rate: sample_rate / BLOCK as f32,
        }
    }

    /// Advance by one control block. `wind` is the mean wind in 0..1.
    pub fn step(&mut self, wind: f32, dt: f32, rng: &mut Rng) {
        let sigma = 0.35 + 0.45 * wind;
        self.deviation += -self.deviation / GUST_TIMESCALE * dt
            + sigma * (2.0 * dt / GUST_TIMESCALE).sqrt() * rng.gauss();
        self.gust = (wind * (1.0 + self.deviation)).max(0.0);
        self.front_speed = 3.0 + 6.0 * wind; // m/s
        self.pos = (self.pos + 1) % HISTORY;
        self.history[self.pos] = self.gust;
    }

    pub fn gust(&self) -> f32 {
        self.gust
    }

    /// Gust strength at lateral world position `y` (metres, +left).
    pub fn local(&self, y: f32) -> f32 {
        let lag_s = (WIND_SPAN - y).max(0.0) / self.front_speed;
        let lag = ((lag_s * self.control_rate) as usize).min(HISTORY - 1);
        self.history[(self.pos + HISTORY - lag) % HISTORY]
    }
}
