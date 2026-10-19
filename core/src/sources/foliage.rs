//! Foliage: the sound of a tree crown, a grass patch or a reed bed in the wind and rain.
//!
//! * **Broad leaves** flutter at f ≈ St_leaf·U/c (c = leaf chord, St_leaf ≈ 0.12) once the
//!   wind exceeds a species-specific onset; neighbouring leaves collide, and each collision
//!   is a short noise burst whose band and length follow leaf size and state: fresh leaves
//!   swish (soft, 2–4 kHz), dry autumn leaves are stiff and crackle (bright, short,
//!   multi-click), wet leaves are heavy and stick together (fewer, duller, longer taps).
//!   Collision rate ∝ leaf count × flutter frequency × engagement(U); collision speed ∝ U.
//!   Aspen's flattened petioles make it flutter in the faintest breeze, in coherent groups,
//!   which is its pattering "applause".
//! * **Needles** do not collide; a conifer "sighs" by aeolian shedding from a cloud of
//!   needles, each at St·U/d (d ≈ 1–2 mm): a soft hiss centred around 0.2·U/d that rises
//!   with the wind (≈ 800 Hz at 5 m/s for spruce).
//! * **Grass** is blade-on-blade friction: a high, dense hiss modulated by the waves gusts
//!   draw across the field (honami, wavelength ~2 m). **Reeds** add hollow-stalk clacks and
//!   low aeolian tones from the stalks.
//! * **Branch creak**: the trunk sways as a damped oscillator at its natural frequency,
//!   driven by the gusts; when two branches rub at high sway velocity they stick-slip, a
//!   pulse train whose rate follows the sliding speed, filtered by wood resonances.
//! * **Rain on the crown** (via [`RainLayer`]): the intercepted share of the rain taps on the
//!   leaves; wet leaves change the timbre of both rain and rustle.
//!
//! The crown is not one point in one wind: the gust crosses it (extent √area at the
//! gust-front speed, [`GustTrail`]), so the leaf collisions, each needle band and the trunk
//! sway hear it at their own moments, each with its own small-scale turbulence (leaves
//! answer fast eddies, ~0.2 s; the trunk the slow ones). The rustle swells in parts instead
//! of as one gain.
//!
//! Large numbers of collisions are rendered as in rain: up to a budget individually, the
//! rest as a noise bed of equal energy (Campbell's theorem).

use core::f32::consts::TAU;

use serde::{Deserialize, Serialize};

use super::common::*;
use super::rain::{check, drips_per_mm_h, RainLayer, Surface};
use super::{Controls, Generator, BLOCK};
use crate::rng::Rng;

const NEEDLE_BANDS: usize = 5;
const U_REF: f32 = 5.0;
/// Overall level of wind noises relative to rain on the leaves.
const RUSTLE_LEVEL: f32 = 3.0;
/// Individually rendered leaf collisions per second (the rest goes into the bed).
const RESOLVED: f32 = 250.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FoliageKind {
    #[default]
    Broadleaf,
    Needles,
    Grass,
    Reeds,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct FoliageConfig {
    pub kind: FoliageKind,
    /// Leaf chord, m (0.005..0.3): birch 0.05, oak 0.1, willow 0.02.
    pub leaf_size_m: f32,
    /// Needle (or grass blade / reed stalk) diameter, m (0.0005..0.02).
    pub needle_diameter_m: f32,
    /// Leaf density, relative leaf area index (0..3).
    pub density: f32,
    /// Area of crown / patch this emitter represents, m² (0.5..2000).
    pub area_m2: f32,
    /// Local wind relative to the canopy mean (0..2.5): edge trees and hilltops > 1.
    pub exposure: f32,
    /// Wind speed at which leaves start to collide, m/s (0.2..10). Aspen 0.5, oak 3.
    pub onset_ms: f32,
    /// Petiole flutter (0..1): coherent quasi-periodic pattering in light wind (aspen 1).
    pub flutter: f32,
    /// Leaf dryness 0 (fresh, supple) .. 1 (dry autumn, crisp); negative = from the season.
    pub dryness: f32,
    /// Branch creak propensity (0..1).
    pub creak: f32,
    /// Natural sway frequency of the trunk, Hz (0.1..3).
    pub sway_hz: f32,
    /// Level of rain hitting the crown (0..3).
    pub rain_on_leaves: f32,
    /// Linear output gain (0..8).
    pub gain: f32,
}

impl Default for FoliageConfig {
    fn default() -> Self {
        Self {
            kind: FoliageKind::Broadleaf,
            leaf_size_m: 0.06,
            needle_diameter_m: 0.0012,
            density: 1.0,
            area_m2: 12.0,
            exposure: 1.0,
            onset_ms: 1.5,
            flutter: 0.2,
            dryness: -1.0,
            creak: 0.1,
            sway_hz: 0.4,
            rain_on_leaves: 1.0,
            gain: 1.0,
        }
    }
}

impl FoliageConfig {
    pub fn validate(&self) -> Result<(), String> {
        check("leaf_size_m", self.leaf_size_m, 0.005, 0.3)?;
        check("needle_diameter_m", self.needle_diameter_m, 0.0005, 0.02)?;
        check("density", self.density, 0.0, 3.0)?;
        check("area_m2", self.area_m2, 0.5, 2000.0)?;
        check("exposure", self.exposure, 0.0, 2.5)?;
        check("onset_ms", self.onset_ms, 0.2, 10.0)?;
        check("flutter", self.flutter, 0.0, 1.0)?;
        check("dryness", self.dryness, -1.0, 1.0)?;
        check("creak", self.creak, 0.0, 1.0)?;
        check("sway_hz", self.sway_hz, 0.1, 3.0)?;
        check("rain_on_leaves", self.rain_on_leaves, 0.0, 3.0)?;
        check("gain", self.gain, 0.0, 8.0)
    }
}

/// Stick-slip creak of rubbing branches, driven by trunk sway.
#[derive(Clone, Copy, Default)]
struct Creak {
    x: f32,
    v: f32,
    force_ou: Ou,
    active: bool,
    dir: f32,
    phase: f32,
    rate: f32,
    amp: f32,
    modes: [Mode; 3],
    freqs: [f32; 3],
}

pub struct Foliage {
    sample_rate: f32,
    rng: Rng,
    cfg: FoliageConfig,
    gust: Smooth,
    trail: GustTrail,
    leaf_turb: Ou,
    /// The inner leaves: their own place in the crown and turbulence, a duller spectrum.
    inner_gust: Smooth,
    inner_turb: Ou,
    spec_inner: GrainSpec,
    inner_share: f32,
    needle_gust: [Smooth; NEEDLE_BANDS],
    needle_turb: [Ou; NEEDLE_BANDS],
    sway_gust: Smooth,
    u: f32,
    t: f32,
    flutter_phase: [f32; 2],
    ev: Events,
    grains: GrainPool<40>,
    bed: NoiseBand,
    swish: NoiseBand,
    needles: [NoiseBand; NEEDLE_BANDS],
    rate: f32,
    rate_mod: [f32; 2],
    spec: GrainSpec,
    crackle: f32,
    creak: Creak,
    rain: RainLayer,
    out: Ramp,
    collisions: f32,
}

impl Foliage {
    pub fn new(sample_rate: f32, seed: u64, cfg: FoliageConfig) -> Self {
        let sr = sample_rate;
        let mut rng = Rng::new(seed);
        let surface = match cfg.kind {
            FoliageKind::Needles => Surface::Needles,
            FoliageKind::Grass | FoliageKind::Reeds => Surface::Grass,
            FoliageKind::Broadleaf => Surface::Leaves,
        };
        let mut rain = RainLayer::new(surface, cfg.area_m2, sr, &mut rng);
        rain.leaf_size = cfg.leaf_size_m;
        rain.resolved_per_s = 60.0;
        let mut creak = Creak::default();
        let base = rng.range(380.0, 620.0);
        creak.freqs = [base, base * rng.range(2.3, 2.9), base * rng.range(4.1, 5.3)];
        Self {
            sample_rate,
            gust: Smooth::new(0.0),
            trail: GustTrail::default(),
            leaf_turb: Ou::default(),
            inner_gust: Smooth::new(0.0),
            inner_turb: Ou::default(),
            spec_inner: GrainSpec::default(),
            inner_share: 0.0,
            needle_gust: [Smooth::new(0.0); NEEDLE_BANDS],
            needle_turb: [Ou::default(); NEEDLE_BANDS],
            sway_gust: Smooth::new(0.0),
            u: 0.0,
            t: 0.0,
            flutter_phase: [rng.uniform(), rng.uniform()],
            ev: Events::new(&mut rng),
            grains: GrainPool::new(),
            bed: NoiseBand::new(3000.0, 1.0, sr),
            swish: NoiseBand::new(1800.0, 0.8, sr),
            needles: [NoiseBand::new(800.0, 4.0, sr); NEEDLE_BANDS],
            rate: 0.0,
            rate_mod: [0.0; 2],
            spec: GrainSpec::default(),
            crackle: 0.0,
            creak,
            rain,
            out: Ramp::new(cfg.gain),
            collisions: 0.0,
            rng,
            cfg,
        }
    }

    pub fn config(&self) -> &FoliageConfig {
        &self.cfg
    }

    /// Change parameters at run time (rates and levels glide).
    pub fn set_config(&mut self, cfg: FoliageConfig) {
        self.rain.area = cfg.area_m2;
        self.rain.leaf_size = cfg.leaf_size_m;
        self.cfg = cfg;
    }

    /// Leaf collisions per second at the current wind (telemetry/tests).
    pub fn collision_rate(&self) -> f32 {
        self.collisions
    }

    /// Flutter frequency of the leaves at wind speed `u`, Hz: St_leaf·U/c, saturating around
    /// 25 Hz (in strong wind leaves fold and streamline instead of fluttering faster).
    pub fn flutter_hz(&self, u: f32) -> f32 {
        let f = 0.12 * u / self.cfg.leaf_size_m;
        f / (1.0 + f / 25.0)
    }

    pub fn birch() -> FoliageConfig {
        FoliageConfig { leaf_size_m: 0.05, onset_ms: 1.2, flutter: 0.35, ..Default::default() }
    }
    pub fn oak() -> FoliageConfig {
        FoliageConfig { leaf_size_m: 0.1, density: 1.3, area_m2: 25.0, onset_ms: 3.0, flutter: 0.05, creak: 0.35, sway_hz: 0.3, ..Default::default() }
    }
    /// Trembling aspen: flattened petioles flutter even in a faint breeze.
    pub fn aspen() -> FoliageConfig {
        FoliageConfig { leaf_size_m: 0.06, onset_ms: 0.4, flutter: 1.0, density: 1.1, ..Default::default() }
    }
    pub fn willow() -> FoliageConfig {
        FoliageConfig { leaf_size_m: 0.02, density: 1.4, onset_ms: 1.8, flutter: 0.1, sway_hz: 0.5, ..Default::default() }
    }
    pub fn alder() -> FoliageConfig {
        FoliageConfig { leaf_size_m: 0.08, onset_ms: 2.2, flutter: 0.15, ..Default::default() }
    }
    pub fn spruce() -> FoliageConfig {
        FoliageConfig { kind: FoliageKind::Needles, needle_diameter_m: 0.0012, density: 1.5, onset_ms: 2.0, creak: 0.3, sway_hz: 0.3, ..Default::default() }
    }
    /// Scots pine: longer, thicker needles, lower sigh.
    pub fn pine() -> FoliageConfig {
        FoliageConfig { kind: FoliageKind::Needles, needle_diameter_m: 0.0016, density: 1.2, area_m2: 20.0, onset_ms: 2.0, creak: 0.4, sway_hz: 0.25, ..Default::default() }
    }
    /// Old pine in a storm: groaning creaks.
    pub fn old_pine() -> FoliageConfig {
        FoliageConfig { creak: 1.0, sway_hz: 0.2, ..Self::pine() }
    }
    /// Oak in autumn: dry, crisp, crackling leaves.
    pub fn autumn_oak() -> FoliageConfig {
        FoliageConfig { dryness: 1.0, density: 0.8, ..Self::oak() }
    }
    pub fn autumn_birch() -> FoliageConfig {
        FoliageConfig { dryness: 0.9, density: 0.7, ..Self::birch() }
    }
    /// Fresh spring leaves: supple, soft swish.
    pub fn fresh_birch() -> FoliageConfig {
        FoliageConfig { dryness: 0.0, ..Self::birch() }
    }
    pub fn grass() -> FoliageConfig {
        FoliageConfig { kind: FoliageKind::Grass, needle_diameter_m: 0.002, area_m2: 60.0, onset_ms: 1.0, creak: 0.0, ..Default::default() }
    }
    pub fn dry_grass() -> FoliageConfig {
        FoliageConfig { dryness: 1.0, ..Self::grass() }
    }
    pub fn reeds() -> FoliageConfig {
        FoliageConfig { kind: FoliageKind::Reeds, needle_diameter_m: 0.007, leaf_size_m: 0.015, area_m2: 40.0, onset_ms: 1.5, creak: 0.0, dryness: 0.6, ..Default::default() }
    }

    pub fn preset(name: &str) -> Option<FoliageConfig> {
        Some(match name {
            "birch" => Self::birch(),
            "oak" => Self::oak(),
            "aspen" => Self::aspen(),
            "willow" => Self::willow(),
            "alder" => Self::alder(),
            "spruce" => Self::spruce(),
            "pine" => Self::pine(),
            "old_pine" => Self::old_pine(),
            "autumn_oak" => Self::autumn_oak(),
            "autumn_birch" => Self::autumn_birch(),
            "fresh_birch" => Self::fresh_birch(),
            "grass" => Self::grass(),
            "dry_grass" => Self::dry_grass(),
            "reeds" => Self::reeds(),
            _ => return None,
        })
    }

    pub const PRESETS: &'static [&'static str] = &[
        "birch",
        "oak",
        "aspen",
        "willow",
        "alder",
        "spruce",
        "pine",
        "old_pine",
        "autumn_oak",
        "autumn_birch",
        "fresh_birch",
        "grass",
        "dry_grass",
        "reeds",
    ];

    fn update(&mut self, c: &Controls, len: usize) {
        let sr = self.sample_rate;
        let dt = len as f32 / sr;
        let cfg = self.cfg.clone();
        let e = &c.env;
        let arriving = c.gust.max(0.0);
        if self.t == 0.0 {
            self.gust = Smooth::new(arriving);
            self.sway_gust = Smooth::new(arriving);
            self.inner_gust = Smooth::new(arriving);
            for g in self.needle_gust.iter_mut() {
                *g = Smooth::new(arriving);
            }
        }
        self.trail.push(arriving, dt);
        let mean = wind_slider(e.wind_ms);
        let cross_s = cfg.area_m2.sqrt().clamp(1.0, 40.0) / (3.0 + 6.0 * mean.min(1.5));
        // Outer leaves on the windward side, inner leaves deeper in the crown, each with fast
        // leaf-scale turbulence of their own.
        let g = self.gust.step(self.trail.at(0.15 * cross_s), dt, 0.25);
        let lt = self.leaf_turb.step(dt, 0.2, 0.15, &mut self.rng);
        self.u = wind_speed_ms(g) * cfg.exposure * lt.exp();
        let u = self.u;
        let gi = self.inner_gust.step(self.trail.at(0.7 * cross_s), dt, 0.3);
        let it = self.inner_turb.step(dt, 0.3, 0.15, &mut self.rng);
        // Sheltered by the outer foliage.
        let u_inner = 0.85 * wind_speed_ms(gi) * cfg.exposure * it.exp();
        let gs = self.sway_gust.step(self.trail.at(0.8 * cross_s), dt, 0.6);
        let u_sway = wind_speed_ms(gs) * cfg.exposure;
        self.t += dt;
        let wet = e.leaf_wetness.clamp(0.0, 1.0);
        let dry = if cfg.dryness >= 0.0 { cfg.dryness } else { e.dryness } * (1.0 - wet);
        let cover = if cfg.kind == FoliageKind::Broadleaf { e.leaf_cover.clamp(0.0, 1.0) } else { 1.0 };
        let leaves = cfg.density * cfg.area_m2 * cover;

        // Leaf collisions (broadleaf, grass blades, reed stalks).
        let collide = |u: f32| match cfg.kind {
            FoliageKind::Broadleaf => {
                let f_fl = self.flutter_hz(u);
                let onset = cfg.onset_ms;
                let engage = ((u - 0.6 * onset) / (2.0 * onset)).clamp(0.0, 1.0).powf(1.5);
                let rate = leaves * 35.0 * f_fl * engage * (1.0 - 0.55 * wet);
                let size = (0.06 / cfg.leaf_size_m).powf(0.4);
                let spec = GrainSpec {
                    amp: 0.05 * (u / U_REF).min(4.0) * (1.0 + 0.5 * dry),
                    fc: 2600.0 * size * (1.0 + 0.9 * dry) * (1.0 - 0.35 * wet),
                    q: 0.9 + 0.6 * dry,
                    attack: lerp(0.0008, 0.0002, dry),
                    decay: 0.003 * (cfg.leaf_size_m / 0.06).sqrt() * (1.0 - 0.45 * dry) * (1.0 + 0.6 * wet),
                    ..Default::default()
                };
                let swish = 0.004 * cfg.density * cover * cfg.area_m2.sqrt() * flow_level(u, U_REF, 4.6) * engage.sqrt();
                (rate, spec, swish)
            }
            FoliageKind::Grass | FoliageKind::Reeds => {
                let engage = ((u - 0.5 * cfg.onset_ms) / (2.0 * cfg.onset_ms)).clamp(0.0, 1.0);
                // Blades rub at a rate ∝ U² per unit area; honami waves at U / 2 m.
                let honami = 1.0 + 0.6 * (TAU * self.t * u / 2.0).sin() * engage;
                let rate = cfg.density * cfg.area_m2 * 60.0 * (u / U_REF).powi(2) * engage * honami * (1.0 - 0.4 * wet);
                let spec = GrainSpec {
                    amp: 0.025 * (u / U_REF).min(4.0),
                    fc: if cfg.kind == FoliageKind::Grass { 6000.0 } else { 3500.0 } * (1.0 + 0.4 * dry) * (1.0 - 0.3 * wet),
                    q: 0.7,
                    attack: 0.0006,
                    decay: 0.002,
                    ..Default::default()
                };
                (rate, spec, 0.0)
            }
            FoliageKind::Needles => (0.0, GrainSpec::default(), 0.0),
        };
        let (ra, mut spec, wa) = collide(u);
        let (rb, mut spec_b, wb) = collide(u_inner);
        spec.fc *= 1.12;
        spec_b.fc *= 0.88;
        let rate = 0.5 * (ra + rb);
        let swish = (0.5 * (wa * wa + wb * wb)).sqrt();
        self.spec_inner = spec_b;
        self.inner_share = if ra + rb > 0.0 { rb / (ra + rb) } else { 0.0 };
        // Coherent flutter groups (aspen): the collision rate pulses near the flutter rate.
        let f_fl = self.flutter_hz(u).min(12.0);
        for k in 0..2 {
            self.flutter_phase[k] = (self.flutter_phase[k] + dt * f_fl * [1.0, 1.37][k]) % 1.0;
            self.rate_mod[k] = (TAU * self.flutter_phase[k]).sin();
        }
        self.collisions = rate;
        self.rate = rate;
        self.spec = spec;
        self.crackle = dry;
        let resolved = rate.min(RESOLVED);
        let bed_rate = (rate - resolved).max(0.0) / sr;
        self.bed.svf.set(spec.fc, spec.q, sr);
        self.bed.gain.to(campbell_amp(bed_rate, spec.energy(sr)), len);
        self.swish.svf.set(1800.0 * (0.06 / cfg.leaf_size_m).powf(0.3), 0.8, sr);
        self.swish.gain.to(swish * band_norm(1800.0, 0.8, sr), len);

        // Needle cloud: aeolian shedding around St·U/d.
        for k in 0..NEEDLE_BANDS {
            let target = if cfg.kind == FoliageKind::Needles || cfg.kind == FoliageKind::Reeds {
                let d = cfg.needle_diameter_m * (0.7f32).powf(1.0 - 2.0 * k as f32 / (NEEDLE_BANDS - 1) as f32);
                // Each band from its own part of the crown, with its own turbulence.
                let at = (k as f32 * 0.618_034 + 0.1).fract();
                let gk = self.needle_gust[k].step(self.trail.at(at * cross_s), dt, 0.25);
                let tk = self.needle_turb[k].step(dt, 0.15 + 0.1 * k as f32, 0.15, &mut self.rng);
                let u = wind_speed_ms(gk) * cfg.exposure * tk.exp();
                let f = strouhal_hz(0.2, u, d).clamp(20.0, 16_000.0);
                let q = if cfg.kind == FoliageKind::Needles { 3.5 } else { 6.0 };
                self.needles[k].svf.set(f, q, sr);
                let shed = smoothstep(40.0, 120.0, reynolds(u, d));
                let lvl = if cfg.kind == FoliageKind::Needles { 0.002 } else { 0.0004 };
                lvl * cfg.density * cfg.area_m2.sqrt() * flow_level(u, U_REF, 4.6) * shed * band_norm(f, q, sr) * (1.0 - 0.3 * wet)
            } else {
                0.0
            };
            self.needles[k].gain.to(target, len);
        }

        // Trunk sway (control rate) and creak triggering.
        let cr = &mut self.creak;
        let w0 = TAU * cfg.sway_hz;
        let force = (u_sway / 10.0).powi(2) * (1.0 + cr.force_ou.step(dt, 0.8, 0.8, &mut self.rng));
        let acc = force * w0 * w0 * 0.05 - 2.0 * 0.06 * w0 * cr.v - w0 * w0 * cr.x;
        cr.v += acc * dt;
        cr.x += cr.v * dt;
        let vmax = (u_sway / 10.0).powi(2) * w0 * 0.05 + 1e-6;
        let speed = (cr.v.abs() / vmax).min(2.0);
        if cr.active {
            if cr.v.signum() != cr.dir || speed < 0.15 {
                cr.active = false;
            }
        } else if cfg.creak > 0.0 && speed > 0.5 && u > 4.0 && self.rng.uniform() < cfg.creak * (u / 12.0).powi(2) * dt * 4.0 * (1.0 - 0.5 * wet) {
            cr.active = true;
            cr.dir = cr.v.signum();
        }
        cr.rate = if cr.active { 40.0 + 220.0 * speed } else { 0.0 };
        cr.amp = if cr.active { 0.03 * speed * (u / 10.0) } else { 0.0 };
        for k in 0..3 {
            let f = cr.freqs[k];
            cr.modes[k].tune(f, f / 25.0, sr);
        }

        // Rain on the crown: the intercepted share, plus drips from upper to lower leaves.
        let rain = e.rain_mm_h * 0.75 * cover.max(0.2);
        let drips = e.drip_mm_h * cfg.area_m2 * 0.3 * drips_per_mm_h();
        self.rain.update(rain, drips, 1.0, wet, 0.0, len, sr);
    }
}

impl Generator for Foliage {
    fn set_model(&mut self, cfg: &super::ModelConfig) {
        if let super::ModelConfig::Foliage(c) = cfg {
            if *c != self.cfg {
                self.set_config(*c);
            }
        }
    }

    fn render(&mut self, out: &mut [f32], c: &Controls) {
        for chunk in out.chunks_mut(BLOCK) {
            self.render_block(chunk, c);
        }
    }
}

impl Foliage {
    fn render_block(&mut self, out: &mut [f32], c: &Controls) {
        let sr = self.sample_rate;
        let len = out.len();
        self.update(c, len);
        let quiet = self.rate <= 0.0
            && self.grains.active() == 0
            && self.bed.gain.cur < 1e-7
            && self.swish.gain.cur < 1e-7
            && self.needles.iter().all(|g| g.gain.cur < 1e-7)
            && self.u < 0.05
            && !self.creak.active
            && self.creak.modes.iter().all(|m| m.energy() < 1e-10)
            && self.rain.is_silent();
        if quiet {
            out.fill(0.0);
            for b in self.needles.iter_mut().chain([&mut self.bed, &mut self.swish]) {
                b.gain.finish();
            }
            return;
        }
        let mut white = [0.0f32; BLOCK];
        for w in white[..len].iter_mut() {
            *w = self.rng.bipolar();
        }
        let white = &white[..len];
        out.fill(0.0);

        // Leaf collisions, spawned at their sample offsets.
        let resolved = self.rate.min(RESOLVED) / sr;
        let base_mod = 1.0 + self.cfg.flutter * 0.7 * (0.6 * self.rate_mod[0] + 0.4 * self.rate_mod[1]);
        let (spec_outer, spec_inner, inner) = (self.spec, self.spec_inner, self.inner_share);
        let crackle = self.crackle;
        if resolved > 0.0 {
            for i in 0..len as u32 {
                for _ in 0..self.ev.tick(resolved * base_mod.max(0.0), &mut self.rng) {
                    let mut s = if self.rng.uniform() < inner { spec_inner } else { spec_outer };
                    // Log-normal collision strength.
                    s.amp *= (0.6 * self.rng.gauss()).exp() * 0.7;
                    s.fc *= self.rng.range(0.75, 1.3);
                    s.decay *= self.rng.range(0.7, 1.4);
                    s.delay = i;
                    self.grains.spawn(&s, sr);
                    if crackle > 0.4 && self.rng.uniform() < crackle * 0.6 {
                        let mut k = s;
                        k.amp *= 0.6;
                        k.fc *= 1.4;
                        k.delay = i + (self.rng.range(0.0004, 0.002) * sr) as u32;
                        self.grains.spawn(&k, sr);
                    }
                }
            }
        }
        self.grains.render_add(out);
        self.bed.render_add(white, out);
        self.swish.render_add(white, out);
        for b in self.needles.iter_mut() {
            b.render_add(white, out);
        }
        // Creak: one stick-slip pulse per phase wrap, with jittered period.
        let cr = &mut self.creak;
        if cr.rate > 0.0 || cr.modes.iter().any(|m| m.energy() > 1e-10) {
            for o in out.iter_mut() {
                if cr.rate > 0.0 {
                    cr.phase += cr.rate / sr;
                    if cr.phase >= 1.0 {
                        cr.phase -= 1.0 + 0.15 * self.rng.bipolar();
                        let a = cr.amp * self.rng.range(0.6, 1.0);
                        cr.modes[0].strike(a);
                        cr.modes[1].strike(a * 0.5);
                        cr.modes[2].strike(a * 0.25);
                    }
                }
                *o += cr.modes[0].tick() + cr.modes[1].tick() + cr.modes[2].tick();
            }
        }
        // Rain on the crown.
        let mut rain = [0.0f32; BLOCK];
        let rain = &mut rain[..len];
        self.rain.render_add(rain, white, &mut self.rng, sr);
        let rain_level = self.cfg.rain_on_leaves;
        self.out.to(self.cfg.gain, len);
        for (o, r) in out.iter_mut().zip(rain.iter()) {
            *o = (*o * RUSTLE_LEVEL + r * rain_level) * self.out.next();
        }
        self.out.finish();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::Ambient;

    fn controls(gust: f32) -> Controls {
        Controls { gust, env: Ambient { wind_ms: wind_speed_ms(gust), ..Default::default() }, ..Default::default() }
    }

    #[test]
    fn aspen_flutters_before_oak() {
        let mut aspen = Foliage::new(48_000.0, 1, Foliage::aspen());
        let mut oak = Foliage::new(48_000.0, 1, Foliage::oak());
        let c = controls(wind_slider(1.2));
        let mut buf = [0.0f32; 64];
        for _ in 0..800 {
            aspen.render(&mut buf, &c);
            oak.render(&mut buf, &c);
        }
        assert!(aspen.collision_rate() > 50.0, "{}", aspen.collision_rate());
        assert_eq!(oak.collision_rate(), 0.0);
        // Flutter frequency is Strouhal-like: ∝ U / chord.
        assert!((aspen.flutter_hz(1.0) / aspen.flutter_hz(0.5) - 2.0).abs() < 0.1);
    }

    #[test]
    fn wet_leaves_collide_less() {
        let run = |wet: f32| {
            let mut f = Foliage::new(48_000.0, 2, Foliage::birch());
            let mut c = controls(0.6);
            c.env.leaf_wetness = wet;
            let mut buf = [0.0f32; 64];
            for _ in 0..400 {
                f.render(&mut buf, &c);
            }
            f.collision_rate()
        };
        assert!(run(1.0) < 0.6 * run(0.0));
    }
}
