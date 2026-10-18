//! Campfire: turbulent combustion roar, crackle from bursting moisture and resin pockets,
//! steam hiss from wet wood, settling embers, and rain sizzling on the coals.
//!
//! * **Roar**: combustion noise of a turbulent diffusion flame is low-frequency (peak ~100–
//!   300 Hz, lower for bigger fires), with power ∝ heat release. Pool fires "puff" at the
//!   flicker frequency f ≈ 1.5/√D Hz (D = fire diameter in m; Cetegen & Ahmed 1993), which
//!   modulates the roar. Wind feeds the fire and raises it.
//! * **Crackle**: water and resin trapped in the wood flash to steam and burst the cell walls.
//!   Pops are clustered (one fracture opens the next): a self-exciting (Hawkes) process
//!   whose base rate peaks at moderate fuel moisture (too dry: little steam; too wet: the
//!   fire smoulders). Pop strength is heavy-tailed (log-normal), each a sub-millisecond
//!   broadband burst with a tiny ring of the charcoal fragment.
//! * **Hiss**: steam jetting from the end grain of wet logs, intermittent.
//! * **Embers**: charcoal pieces settling: dull thunks and glassy tinkles.
//! * **Rain on the fire**: drops hitting embers flash-boil, a short sizzle each.

use serde::{Deserialize, Serialize};

use super::common::*;
use super::rain::{check, flux_above};
use super::{Controls, Generator, BLOCK};
use crate::rng::Rng;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct FireConfig {
    /// Heat release rate, kW (1..2000): small campfire 20, bonfire 500.
    pub heat_kw: f32,
    /// Fuel bed diameter, m (0.1..5).
    pub diameter_m: f32,
    /// Fuel moisture content, fraction of dry mass (0.02..0.6).
    pub moisture: f32,
    /// Resin content (0..1): pine and spruce crackle and spit.
    pub resin: f32,
    /// Burn stage (0..1): 0 fresh flames, 1 glowing embers.
    pub embers: f32,
    /// Linear output gain (0..8).
    pub gain: f32,
}

impl Default for FireConfig {
    fn default() -> Self {
        Self { heat_kw: 20.0, diameter_m: 0.6, moisture: 0.15, resin: 0.3, embers: 0.2, gain: 1.0 }
    }
}

impl FireConfig {
    pub fn validate(&self) -> Result<(), String> {
        check("heat_kw", self.heat_kw, 1.0, 2000.0)?;
        check("diameter_m", self.diameter_m, 0.1, 5.0)?;
        check("moisture", self.moisture, 0.02, 0.6)?;
        check("resin", self.resin, 0.0, 1.0)?;
        check("embers", self.embers, 0.0, 1.0)?;
        check("gain", self.gain, 0.0, 8.0)
    }
}

pub struct Fire {
    sample_rate: f32,
    rng: Rng,
    cfg: FireConfig,
    roar: [NoiseBand; 2],
    flicker: f32,
    flicker_ou: Ou,
    hiss: NoiseBand,
    hiss_on: bool,
    hiss_timer: f32,
    hiss_level: Smooth,
    grains: GrainPool<40>,
    ev_pop: Events,
    ev_ember: Events,
    ev_sizzle: Events,
    excitation: f32,
    pop_rate: f32,
    ember_rate: f32,
    sizzle_rate: f32,
    roar_mod: Ramp,
    out: Ramp,
}

impl Fire {
    pub fn new(sample_rate: f32, seed: u64, cfg: FireConfig) -> Self {
        let sr = sample_rate;
        let mut rng = Rng::new(seed);
        Self {
            sample_rate,
            roar: [NoiseBand::new(150.0, 0.7, sr), NoiseBand::new(600.0, 0.6, sr)],
            flicker: rng.uniform(),
            flicker_ou: Ou::default(),
            hiss: NoiseBand::new(5000.0, 1.2, sr),
            hiss_on: false,
            hiss_timer: rng.range(0.5, 3.0),
            hiss_level: Smooth::new(0.0),
            grains: GrainPool::new(),
            ev_pop: Events::new(&mut rng),
            ev_ember: Events::new(&mut rng),
            ev_sizzle: Events::new(&mut rng),
            excitation: 0.0,
            pop_rate: 0.0,
            ember_rate: 0.0,
            sizzle_rate: 0.0,
            roar_mod: Ramp::new(0.0),
            out: Ramp::new(cfg.gain),
            rng,
            cfg,
        }
    }

    pub fn config(&self) -> &FireConfig {
        &self.cfg
    }

    pub fn set_config(&mut self, cfg: FireConfig) {
        self.cfg = cfg;
    }

    /// Flame flicker (puffing) frequency for a fire of diameter `d` m, Hz.
    pub fn flicker_hz(d: f32) -> f32 {
        1.5 / d.max(0.05).sqrt()
    }

    pub fn campfire() -> FireConfig {
        FireConfig::default()
    }
    pub fn bonfire() -> FireConfig {
        FireConfig { heat_kw: 500.0, diameter_m: 2.0, moisture: 0.18, resin: 0.4, embers: 0.1, ..Default::default() }
    }
    pub fn wet_wood() -> FireConfig {
        FireConfig { moisture: 0.4, resin: 0.1, heat_kw: 12.0, ..Default::default() }
    }
    pub fn resinous_pine() -> FireConfig {
        FireConfig { resin: 0.9, moisture: 0.12, ..Default::default() }
    }
    pub fn embers() -> FireConfig {
        FireConfig { heat_kw: 6.0, embers: 0.95, moisture: 0.05, resin: 0.1, ..Default::default() }
    }

    pub fn preset(name: &str) -> Option<FireConfig> {
        Some(match name {
            "campfire" => Self::campfire(),
            "bonfire" => Self::bonfire(),
            "wet_wood" => Self::wet_wood(),
            "resinous_pine" => Self::resinous_pine(),
            "embers" => Self::embers(),
            _ => return None,
        })
    }

    pub const PRESETS: &'static [&'static str] = &["campfire", "bonfire", "wet_wood", "resinous_pine", "embers"];
}

impl Generator for Fire {
    fn render(&mut self, out: &mut [f32], c: &Controls) {
        for chunk in out.chunks_mut(BLOCK) {
            self.render_block(chunk, c);
        }
    }
}

impl Fire {
    fn render_block(&mut self, out: &mut [f32], c: &Controls) {
        let sr = self.sample_rate;
        let len = out.len();
        let dt = len as f32 / sr;
        let cfg = self.cfg.clone();
        let flames = 1.0 - cfg.embers;
        let wind = 1.0 + 0.12 * wind_speed_ms(c.gust).min(15.0);
        let heat = cfg.heat_kw * wind;

        // Roar: lower and louder for bigger fires, puffing at the flicker frequency.
        let f_roar = (220.0 / cfg.diameter_m.sqrt()).clamp(60.0, 400.0);
        self.roar[0].svf.set(f_roar, 0.7, sr);
        self.roar[1].svf.set(f_roar * 4.0, 0.6, sr);
        let roar = 0.008 * heat.sqrt() * flames;
        self.flicker = (self.flicker + dt * Self::flicker_hz(cfg.diameter_m) * (1.0 + 0.2 * self.flicker_ou.step(dt, 0.5, 1.0, &mut self.rng))) % 1.0;
        let puff = 0.65 + 0.35 * (core::f32::consts::TAU * self.flicker).sin();
        self.roar[0].gain.to(roar * band_norm(f_roar, 0.7, sr), len);
        self.roar[1].gain.to(roar * 0.25 * band_norm(f_roar * 4.0, 0.6, sr), len);
        self.roar_mod.to(puff, len);

        // Crackle base rate peaks at moderate moisture.
        let m = cfg.moisture;
        let steam = (m / 0.15) * (-(m / 0.3).powi(2)).exp() * 1.9;
        self.pop_rate = (0.5 + 2.5 * steam + 4.0 * cfg.resin) * (cfg.heat_kw / 20.0).sqrt() * (0.3 + flames);
        self.ember_rate = 0.4 + 2.0 * cfg.embers;
        // Hiss bouts from wet wood.
        self.hiss_timer -= dt;
        if self.hiss_timer <= 0.0 {
            self.hiss_on = self.rng.uniform() < (m * 2.5).min(0.9);
            self.hiss_timer = if self.hiss_on { self.rng.range(0.5, 4.0) } else { self.rng.range(1.0, 6.0) };
        }
        let h = self.hiss_level.step(if self.hiss_on { 0.01 * m.sqrt() * flames.max(0.2) } else { 0.0 }, dt, 0.2);
        self.hiss.gain.to(h * band_norm(5000.0, 1.2, sr), len);
        // Rain on the coals: drops of the fire's area, each a sizzle.
        let area = core::f32::consts::PI * 0.25 * cfg.diameter_m * cfg.diameter_m;
        self.sizzle_rate = flux_above(c.env.rain_mm_h, 1.0) * area;

        let hawkes_decay = (-1.0 / (0.06 * sr)).exp();
        let mut white = [0.0f32; BLOCK];
        for w in white[..len].iter_mut() {
            *w = self.rng.bipolar();
        }
        let white = &white[..len];
        out.fill(0.0);
        for i in 0..len as u32 {
            let rate = (self.pop_rate + self.excitation) / sr;
            for _ in 0..self.ev_pop.tick(rate, &mut self.rng) {
                // Each pop raises the chance of the next for ~60 ms (clustered crackle); the
                // branching ratio (expected offspring per pop) stays below 1.
                self.excitation += (0.5 + 0.4 * cfg.resin) / 0.06;
                let a = 0.12 * (0.9 * self.rng.gauss()).exp().min(12.0);
                self.grains.spawn(
                    &GrainSpec {
                        amp: a,
                        fc: self.rng.range(2500.0, 9000.0),
                        q: 0.6,
                        attack: 0.00005,
                        decay: self.rng.range(0.0002, 0.0012),
                        modes: [(self.rng.range(2000.0, 6000.0), self.rng.range(0.002, 0.006), a * 0.15), (0.0, 0.0, 0.0)],
                        delay: i,
                    },
                    sr,
                );
            }
            self.excitation *= hawkes_decay;
            for _ in 0..self.ev_ember.tick(self.ember_rate / sr, &mut self.rng) {
                let tinkle = self.rng.uniform() < 0.5;
                let a = 0.05 * (1.0 + 3.0 * cfg.embers) * self.rng.range(0.3, 1.0);
                let spec = if tinkle {
                    GrainSpec { amp: a * 0.3, fc: 4000.0, q: 1.0, decay: 0.0005, modes: [(self.rng.range(2500.0, 5500.0), 0.03, a * 0.25), (self.rng.range(6000.0, 9000.0), 0.015, a * 0.12)], delay: i, ..Default::default() }
                } else {
                    GrainSpec { amp: a, fc: self.rng.range(250.0, 600.0), q: 0.8, attack: 0.002, decay: 0.02, delay: i, ..Default::default() }
                };
                self.grains.spawn(&spec, sr);
            }
            for _ in 0..self.ev_sizzle.tick(self.sizzle_rate / sr, &mut self.rng) {
                self.grains.spawn(
                    &GrainSpec { amp: 0.02 * self.rng.range(0.5, 1.0), fc: self.rng.range(4000.0, 7000.0), q: 0.9, attack: 0.003, decay: self.rng.range(0.03, 0.09), delay: i, ..Default::default() },
                    sr,
                );
            }
        }
        // Roar, puffing at the flicker rate.
        let mut roar = [0.0f32; BLOCK];
        let roar = &mut roar[..len];
        self.roar[0].render_add(white, roar);
        self.roar[1].render_add(white, roar);
        for (o, r) in out.iter_mut().zip(roar.iter()) {
            *o += r * self.roar_mod.next();
        }
        self.roar_mod.finish();
        self.hiss.render_add(white, out);
        self.grains.render_add(out);
        self.out.to(cfg.gain, len);
        for o in out.iter_mut() {
            *o *= self.out.next() * 1.2;
        }
        self.out.finish();
        self.excitation = flush(self.excitation);
    }
}
