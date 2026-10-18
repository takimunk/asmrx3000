//! Thunder: the sum of the pressure pulses radiated by every piece of a tortuous lightning
//! channel, each arriving at its own time.
//!
//! * The channel is a 3D random walk from the ground strike point up to the cloud base
//!   (segments of ~15 m, mean direction change ~16°, after Few 1969 and Ribner & Roy 1982),
//!   plus side branches and optionally an in-cloud horizontal part.
//! * Each segment's sudden heating radiates a weak shock, an N-wave whose initial length
//!   follows Few's relaxation radius R₀ = √(E_l/(π·p₀)) (E_l = energy per metre of channel,
//!   ~10⁵–10⁶ J/m): the dominant frequency is ≈ 0.63·c/R₀, about 100–200 Hz near the channel.
//!   Weak-shock propagation lengthens the N-wave slowly with distance (∝ r^¼ asymptotically).
//! * Radiation is strongest broadside to a segment, so a channel seen side-on gives a loud
//!   clap and one seen end-on a long roll.
//! * Arrival time r/c spreads the pulses over seconds: the near part of the channel arrives
//!   first (the crack for a close strike), the far part later (the rumble). Spherical
//!   spreading and air absorption (cut-off frequency falling with distance, ~5 dB/km at
//!   1 kHz) make far thunder a low rumble without crack, and beyond ~20 km upward refraction
//!   puts the listener in the acoustic shadow.
//! * Echoes from terrain and forest edges add delayed, darker copies.
//!
//! The emitter is a direction proxy (the director places it ~40 m away in the direction of
//! the strike, so the spatialiser gives the right bearing); the long-range physics is done
//! here. Sound starts r/c after the flash (3 s per km).

use serde::{Deserialize, Serialize};

use super::common::*;
use super::rain::check;
use super::{Controls, Generator};
use crate::rng::Rng;

const MAX_PULSES: usize = 2048;
const RING: usize = 1 << 15;
const LOOKAHEAD: f32 = 0.25;
/// Listener ear height above ground, m.
const EAR: f32 = 1.7;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct ThunderConfig {
    /// Height of the channel top (cloud base), m (500..8000).
    pub cloud_base_m: f32,
    /// Mean change of direction between channel segments, degrees (3..45).
    pub tortuosity_deg: f32,
    /// Mean channel segment length, m (3..60).
    pub segment_m: f32,
    /// Energy per metre of channel, J/m (1e4..5e6): sets the N-wave length (pitch).
    pub energy_j_per_m: f32,
    /// Side branches (0..12).
    pub branches: u32,
    /// Share of in-cloud horizontal channel (0..1): longer, softer rolls.
    pub intracloud: f32,
    /// Strike distance, m (200..30000), or negative to take strikes from the environment.
    pub distance_m: f32,
    /// With a fixed distance: mean seconds between strikes (5..600).
    pub interval_s: f32,
    /// Linear output gain (0..8).
    pub gain: f32,
}

impl Default for ThunderConfig {
    fn default() -> Self {
        Self {
            cloud_base_m: 3000.0,
            tortuosity_deg: 16.0,
            segment_m: 8.0,
            energy_j_per_m: 5e5,
            branches: 5,
            intracloud: 0.2,
            distance_m: -1.0,
            interval_s: 30.0,
            gain: 1.0,
        }
    }
}

impl ThunderConfig {
    pub fn validate(&self) -> Result<(), String> {
        check("cloud_base_m", self.cloud_base_m, 500.0, 8000.0)?;
        check("tortuosity_deg", self.tortuosity_deg, 3.0, 45.0)?;
        check("segment_m", self.segment_m, 3.0, 60.0)?;
        check("energy_j_per_m", self.energy_j_per_m, 1e4, 5e6)?;
        check("branches", self.branches as f32, 0.0, 12.0)?;
        check("intracloud", self.intracloud, 0.0, 1.0)?;
        if self.distance_m >= 0.0 {
            check("distance_m", self.distance_m, 200.0, 30_000.0)?;
        }
        check("interval_s", self.interval_s, 5.0, 600.0)?;
        check("gain", self.gain, 0.0, 8.0)
    }
}

#[derive(Clone, Copy, Default)]
struct Pulse {
    /// Arrival time relative to `clock` origin, s.
    t: f32,
    amp: f32,
    /// N-wave length, s.
    period: f32,
    /// Air-absorption cut-off, Hz.
    fc: f32,
    /// Time spread by turbulent scattering, s.
    spread: f32,
}

#[derive(Clone, Copy)]
struct V {
    x: f32,
    y: f32,
    z: f32,
}

impl V {
    fn add(self, o: V, s: f32) -> V {
        V { x: self.x + o.x * s, y: self.y + o.y * s, z: self.z + o.z * s }
    }
    fn len(self) -> f32 {
        (self.x * self.x + self.y * self.y + self.z * self.z).sqrt()
    }
    fn norm(self) -> V {
        let l = self.len().max(1e-9);
        V { x: self.x / l, y: self.y / l, z: self.z / l }
    }
    fn cross_len(self, o: V) -> f32 {
        let c = V { x: self.y * o.z - self.z * o.y, y: self.z * o.x - self.x * o.z, z: self.x * o.y - self.y * o.x };
        c.len()
    }
}

pub struct Thunder {
    sample_rate: f32,
    rng: Rng,
    cfg: ThunderConfig,
    pulses: Box<[Pulse]>,
    n: usize,
    next: usize,
    clock: f32,
    ring: Box<[f32]>,
    read: usize,
    last_strike: u32,
    timer: f32,
    distance: f32,
    temperature: f32,
}

impl Thunder {
    pub fn new(sample_rate: f32, seed: u64, cfg: ThunderConfig) -> Self {
        let mut rng = Rng::new(seed);
        let timer = rng.range(0.5, 2.0);
        Self {
            sample_rate,
            rng,
            pulses: vec![Pulse::default(); MAX_PULSES].into_boxed_slice(),
            n: 0,
            next: 0,
            clock: 0.0,
            ring: vec![0.0; RING].into_boxed_slice(),
            read: 0,
            last_strike: 0,
            timer,
            distance: 0.0,
            temperature: 15.0,
            cfg,
        }
    }

    pub fn config(&self) -> &ThunderConfig {
        &self.cfg
    }

    pub fn set_config(&mut self, cfg: ThunderConfig) {
        self.cfg = cfg;
    }

    /// Distance of the most recent strike, m.
    pub fn last_distance(&self) -> f32 {
        self.distance
    }

    /// Seconds until the first sound of the pending strike (negative once it is heard).
    pub fn time_to_first_arrival(&self) -> f32 {
        if self.next < self.n {
            self.pulses[self.next].t - self.clock
        } else {
            -1.0
        }
    }

    /// The storm overhead follows the director's lightning.
    pub fn storm() -> ThunderConfig {
        ThunderConfig::default()
    }
    pub fn close_strike() -> ThunderConfig {
        ThunderConfig { distance_m: 700.0, interval_s: 25.0, branches: 7, ..Default::default() }
    }
    pub fn summer_storm() -> ThunderConfig {
        ThunderConfig { distance_m: 3000.0, interval_s: 20.0, ..Default::default() }
    }
    pub fn distant_rumble() -> ThunderConfig {
        ThunderConfig { distance_m: 12_000.0, interval_s: 18.0, intracloud: 0.5, ..Default::default() }
    }
    /// Long in-cloud rolls of a mature storm.
    pub fn rolling() -> ThunderConfig {
        ThunderConfig { distance_m: 5000.0, interval_s: 25.0, intracloud: 0.8, branches: 9, ..Default::default() }
    }

    pub fn preset(name: &str) -> Option<ThunderConfig> {
        Some(match name {
            "storm" => Self::storm(),
            "close_strike" => Self::close_strike(),
            "summer_storm" => Self::summer_storm(),
            "distant_rumble" => Self::distant_rumble(),
            "rolling" => Self::rolling(),
            _ => return None,
        })
    }

    pub const PRESETS: &'static [&'static str] = &["storm", "close_strike", "summer_storm", "distant_rumble", "rolling"];

    /// Starts a strike at horizontal distance `distance` m. Real-time safe (fixed buffers).
    pub fn strike(&mut self, distance: f32) {
        let distance = distance.clamp(50.0, 40_000.0);
        self.distance = distance;
        // Keep the not yet rendered pulses of a previous strike, re-based to now.
        let keep = self.n - self.next;
        for i in 0..keep {
            let mut p = self.pulses[self.next + i];
            p.t -= self.clock;
            self.pulses[i] = p;
        }
        self.n = keep;
        self.next = 0;
        self.clock = 0.0;

        let c = sound_speed(self.temperature);
        let cfg = self.cfg.clone();
        let ear = V { x: 0.0, y: 0.0, z: EAR };
        let ground = V { x: distance, y: 0.0, z: 0.0 };
        let sigma = cfg.tortuosity_deg.to_radians();
        let budget = (MAX_PULSES - self.n) / 2;
        let mut out = 0usize;

        // Walk: main channel, then branches from random points of it, then in-cloud part.
        let mut main_pts = [ground; 64];
        let mut main_n = 0;
        let mut emit = |th: &mut Thunder, mid: V, dir: V, len: f32, strength: f32| {
            if out >= budget {
                return;
            }
            let to = V { x: ear.x - mid.x, y: ear.y - mid.y, z: ear.z - mid.z };
            let r = to.len().max(1.0);
            let los = to.norm();
            let side = dir.cross_len(los);
            let directivity = 0.15 + 0.85 * side * side;
            let e = cfg.energy_j_per_m * strength;
            let r0 = (e / (core::f32::consts::PI * P0)).sqrt().max(0.05);
            let t0 = r0 / (0.63 * c);
            let period = t0 * (1.0 + r / (50.0 * r0)).powf(0.25);
            let horizontal = (mid.x * mid.x + mid.y * mid.y).sqrt();
            let shadow = 1.0 - smoothstep(15_000.0, 25_000.0, horizontal);
            let fc = (1000.0 * (1200.0 / r).powf(0.625)).clamp(60.0, 12_000.0);
            let amp = directivity * (len / 15.0).sqrt() * (e / 5e5).sqrt() * shadow * 300.0 / r * (0.3 * th.rng.gauss()).exp();
            let t = r / c;
            // Turbulence scatters part of each pulse into a coda that grows with range.
            let spread = (0.015 + r / 40_000.0).min(0.25);
            th.pulses[th.n] = Pulse { t, amp, period, fc, spread };
            th.n += 1;
            // Terrain / forest-edge echo.
            let delay = th.rng.range(0.4, 3.5);
            th.pulses[th.n] = Pulse { t: t + delay, amp: amp * th.rng.range(0.1, 0.3), period: period * 1.2, fc: fc * 0.5, spread: spread * 1.5 };
            th.n += 1;
            out += 1;
        };

        let mut pos = ground;
        let mut dir = V { x: 0.0, y: 0.0, z: 1.0 };
        let mut guard = 0;
        while pos.z < cfg.cloud_base_m && guard < 600 {
            guard += 1;
            dir = V { x: dir.x + sigma * self.rng.gauss(), y: dir.y + sigma * self.rng.gauss(), z: dir.z + sigma * self.rng.gauss() + 0.12 }.norm();
            let len = cfg.segment_m * (0.5 * self.rng.gauss()).exp();
            let mid = pos.add(dir, 0.5 * len);
            emit(self, mid, dir, len, 1.0);
            pos = pos.add(dir, len);
            if main_n < main_pts.len() && guard % 4 == 0 {
                main_pts[main_n] = pos;
                main_n += 1;
            }
        }
        let top = pos;
        for _ in 0..cfg.branches {
            if main_n == 0 {
                break;
            }
            let k = main_n / 3 + (self.rng.next_u32() as usize % (main_n - main_n / 3).max(1));
            let mut p = main_pts[k.min(main_n - 1)];
            let ang = self.rng.range(0.0, core::f32::consts::TAU);
            let mut d = V { x: ang.cos(), y: ang.sin(), z: -0.6 }.norm();
            let segs = 10 + self.rng.next_u32() % 30;
            for _ in 0..segs {
                d = V { x: d.x + sigma * self.rng.gauss(), y: d.y + sigma * self.rng.gauss(), z: d.z + sigma * self.rng.gauss() - 0.05 }.norm();
                let len = cfg.segment_m * (0.5 * self.rng.gauss()).exp();
                emit(self, p.add(d, 0.5 * len), d, len, 0.3);
                p = p.add(d, len);
                if p.z < 0.0 {
                    break;
                }
            }
        }
        let ic = (cfg.intracloud * 150.0) as u32;
        let mut p = top;
        let ang = self.rng.range(0.0, core::f32::consts::TAU);
        let mut d = V { x: ang.cos(), y: ang.sin(), z: 0.0 };
        for _ in 0..ic {
            d = V { x: d.x + sigma * self.rng.gauss(), y: d.y + sigma * self.rng.gauss(), z: 0.3 * (d.z + sigma * self.rng.gauss()) }.norm();
            let len = cfg.segment_m * 2.0 * (0.5 * self.rng.gauss()).exp();
            emit(self, p.add(d, 0.5 * len), d, len, 0.5);
            p = p.add(d, len);
        }
        self.pulses[..self.n].sort_unstable_by(|a, b| a.t.partial_cmp(&b.t).unwrap_or(core::cmp::Ordering::Equal));
    }

    /// Writes one N-wave, low-passed by air absorption, into the ring at `offset` samples.
    fn write_pulse(&mut self, p: Pulse, offset: usize) {
        let sr = self.sample_rate;
        let len = ((p.period * sr) as usize).max(2);
        let a = one_pole_coef(p.fc, sr);
        let tail = (4.0 * sr / (core::f32::consts::TAU * p.fc)) as usize;
        let (mut y1, mut y2) = (0.0f32, 0.0f32);
        let coda = (4.0 * p.spread * sr) as usize;
        let total = (len + tail).max(coda).min(RING - offset.min(RING) - 1).min(RING / 2);
        // The coherent part fades with range (e-folding ~8 km); its energy goes to the coda.
        let coherent = (-p.spread * 5.0).exp();
        let direct = p.amp * coherent;
        let scatter = p.amp * (1.0 - coherent * coherent).sqrt() * (len as f32 / (p.spread * sr)).sqrt().min(1.0);
        let decay = (-1.0 / (p.spread * sr)).exp();
        let attack = (-1.0 / (0.004 * sr)).exp();
        let (mut e1, mut e2) = (scatter, scatter);
        for i in 0..total {
            let mut x = if i < len { direct * (1.0 - 2.0 * i as f32 / len as f32) } else { 0.0 };
            x += (e1 - e2) * self.rng.bipolar() * 1.7;
            e1 *= decay;
            e2 *= attack;
            y1 += (x - y1) * a;
            y2 += (y1 - y2) * a;
            let k = (self.read + offset + i) & (RING - 1);
            self.ring[k] += y2;
        }
    }
}

impl Generator for Thunder {
    fn render(&mut self, out: &mut [f32], c: &Controls) {
        let sr = self.sample_rate;
        let len = out.len();
        let dt = len as f32 / sr;
        self.temperature = c.env.temperature_c;
        if self.cfg.distance_m < 0.0 {
            if c.env.strike != self.last_strike {
                self.last_strike = c.env.strike;
                if c.env.strike_distance_m > 0.0 {
                    self.strike(c.env.strike_distance_m);
                }
            }
        } else {
            self.timer -= dt;
            if self.timer <= 0.0 {
                self.timer = self.cfg.interval_s * self.rng.range(0.5, 1.5);
                let d = self.cfg.distance_m * self.rng.range(0.8, 1.25);
                self.strike(d);
            }
        }
        while self.next < self.n && self.pulses[self.next].t <= self.clock + LOOKAHEAD {
            let p = self.pulses[self.next];
            let offset = ((p.t - self.clock).max(0.0) * sr) as usize;
            self.write_pulse(p, offset);
            self.next += 1;
        }
        let g = self.cfg.gain;
        for o in out.iter_mut() {
            let k = self.read & (RING - 1);
            *o = flush(self.ring[k]) * g;
            self.ring[k] = 0.0;
            self.read = (self.read + 1) & (RING - 1);
        }
        self.clock += dt;
        if self.next >= self.n {
            self.n = 0;
            self.next = 0;
            self.clock = 0.0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(distance: f32, seconds: f32) -> (Vec<f32>, f32) {
        let mut t = Thunder::new(48_000.0, 4, ThunderConfig { distance_m: -1.0, ..Default::default() });
        t.strike(distance);
        let first = t.time_to_first_arrival();
        let mut buf = vec![0.0f32; (seconds * 48_000.0) as usize];
        for chunk in buf.chunks_mut(64) {
            t.render(chunk, &Controls::default());
        }
        (buf, first)
    }

    #[test]
    fn thunder_arrives_after_flash_and_darkens_with_distance() {
        let (near, t_near) = render(1000.0, 14.0);
        let (far, t_far) = render(8000.0, 30.0);
        // 3 s per km.
        assert!((t_near - 1000.0 / 341.0).abs() < 0.3, "{t_near}");
        assert!((t_far - 8000.0 / 341.0).abs() < 1.0, "{t_far}");
        let energy = |b: &[f32]| b.iter().map(|v| (*v as f64).powi(2)).sum::<f64>();
        assert!(energy(&near) > 10.0 * energy(&far));
        // High-frequency share (first difference) drops with distance.
        let hf = |b: &[f32]| b.windows(2).map(|w| ((w[1] - w[0]) as f64).powi(2)).sum::<f64>() / energy(b).max(1e-30);
        assert!(hf(&near) > 2.0 * hf(&far), "{} vs {}", hf(&near), hf(&far));
        assert!(near.iter().chain(&far).all(|v| v.is_finite()));
    }
}
