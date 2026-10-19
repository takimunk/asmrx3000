//! Scene-level acoustics: environment (weather, ground, forest), per-emitter propagators,
//! forest reverb, SH-domain head rotation and binaural decoding.
//!
//! Engine integration (two calls, see engine.rs):
//!   * per emitter per block: `acoustics.propagate(id, &emitter, mono, &mut bus)`
//!   * once per block:        `acoustics.render(&mut bus, &head, len, left, right)`
//! The bus is in the world frame; `render` adds reverb, rotates into the head frame and
//! decodes. All of it is allocation- and lock-free; `new`, `ensure_emitters`, `set_trees` and
//! `set_obstacles` (beyond their reserved capacity) are not and belong off the audio thread.

use crate::ambisonics::{CHANNELS, ORDER};
use crate::bed;
use crate::binaural::BinauralDecoder;
use crate::engine::BLOCK;
use crate::hrtf::{self, HrirSet};
use crate::math::{Mat3, Vec3};
use crate::nearfield::NfcDesign;
use crate::num::C64;
use crate::physics::{
    air,
    diffraction::Obstacle,
    directivity::{self, Directivity},
    ground::{self, GroundType, GROUND_TYPES},
    scattering, wind,
};
use crate::propagation::Propagator;
use crate::reverb::{self, ForestGeometry, ForestReverb, Sends, ER_TRUNKS};
use crate::rng::Rng;
use crate::rotation::ShRotator;
use crate::shaping::{Coeffs, Design, FilterBank, NF};

/// Trunks closer than this to the listener are explicit diffracting obstacles; beyond it the
/// forest acts statistically (excess attenuation), so nothing is counted twice.
pub const EXPLICIT_RADIUS: f32 = 15.0;
const PROCEDURAL_RADIUS: f32 = 22.0;
const MAX_TREES: usize = 512;
const MAX_OBSTACLES: usize = 128;
const CANOPY_TOP: f32 = 20.0;
const BARK_ABSORPTION: f64 = 0.05;

#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HrtfModel {
    Measured = 0,
    Analytic = 1,
}

/// Weather, ground and forest. Units: deg C, % RH, kPa, trunks per hectare, metres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Environment {
    pub temperature_c: f32,
    pub humidity_pct: f32,
    pub pressure_kpa: f32,
    pub ground: GroundType,
    pub tree_density_per_ha: f32,
    /// 0 = bare branches, 1 = dense summer foliage.
    pub foliage: f32,
    /// Late-reverb level relative to the physical estimate (1 = physical).
    pub reverb: f32,
    /// Listener ear height above the ground.
    pub ear_height: f32,
    pub hrtf: HrtfModel,
}

impl Default for Environment {
    fn default() -> Self {
        Self {
            temperature_c: 15.0,
            humidity_pct: 70.0,
            pressure_kpa: 101.325,
            ground: GroundType::ForestFloor,
            tree_density_per_ha: 400.0,
            foliage: 0.6,
            reverb: 1.0,
            ear_height: 1.5,
            hrtf: HrtfModel::Measured,
        }
    }
}

impl Environment {
    fn sanitized(mut self) -> Self {
        let fix = |v: f32, lo: f32, hi: f32, d: f32| if v.is_finite() { v.clamp(lo, hi) } else { d };
        self.temperature_c = fix(self.temperature_c, -30.0, 45.0, 15.0);
        self.humidity_pct = fix(self.humidity_pct, 1.0, 100.0, 70.0);
        self.pressure_kpa = fix(self.pressure_kpa, 50.0, 110.0, 101.325);
        self.tree_density_per_ha = fix(self.tree_density_per_ha, 0.0, 3000.0, 400.0);
        self.foliage = fix(self.foliage, 0.0, 1.0, 0.6);
        self.reverb = fix(self.reverb, 0.0, 4.0, 1.0);
        // Up to 60 m: flight mode flies above the canopy.
        self.ear_height = fix(self.ear_height, 0.1, 60.0, 1.5);
        self
    }
}

/// One sound emitter as the propagation sees it, in the world frame.
#[derive(Clone, Copy, Debug)]
pub struct Emitter {
    pub position: Vec3,
    /// m/s. None = estimated from successive positions.
    pub velocity: Option<Vec3>,
    /// Amplitude of the emitted signal at 1 m.
    pub gain: f32,
    /// Facing direction (directivity axis). None = omnidirectional.
    pub forward: Option<Vec3>,
    pub directivity: Directivity,
    /// Ground under this emitter's reflection path, if it differs from the scene's
    /// (e.g. water under a river segment).
    pub ground: Option<GroundType>,
    /// Height of the ground plane under this emitter's reflection path relative to the
    /// listener, if it differs from the scene's flat ground (terrain: a source in a valley
    /// below or on a slope reflects off its own ground, not off the listener's).
    pub ground_z: Option<f32>,
    /// Extra spectral gain of the direct and ground paths on the fit grid
    /// (`shaping::grid_freqs`), dB: the mix layer's masking ducks and distance cues.
    pub eq_db: [f32; NF],
    /// Reverb send multiplier (1 = physical).
    pub send: f32,
}

impl Emitter {
    pub fn at(position: Vec3, gain: f32) -> Self {
        Self { position, velocity: None, gain, forward: None, directivity: Directivity::Omni, ground: None, ground_z: None, eq_db: [0.0; NF], send: 1.0 }
    }
}

/// A tree trunk: centre (x, y) on the ground, radius, height above the ground.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Trunk {
    pub x: f32,
    pub y: f32,
    pub radius: f32,
    pub height: f32,
}

/// Switches for A/B listening and tests.
#[derive(Clone, Copy, Debug)]
pub struct Features {
    pub ground: bool,
    pub reverb: bool,
    pub early_reflections: bool,
    pub occlusion: bool,
    pub forest: bool,
    pub near_field: bool,
    /// Wind refraction (upwind shadow, downwind enhancement) and turbulent level wander.
    pub wind: bool,
}

impl Default for Features {
    fn default() -> Self {
        Self { ground: true, reverb: true, early_reflections: true, occlusion: true, forest: true, near_field: true, wind: true }
    }
}

/// Read-only state shared by all propagators during a block.
pub struct Context {
    pub sample_rate: f32,
    pub design: Design,
    pub nfc: NfcDesign,
    pub listener: Vec3,
    pub ground_z: f32,
    pub ground: GroundType,
    pub speed_of_sound: f32,
    pub epoch: u32,
    pub features: Features,
    /// Amplitude attenuation, Np per metre, at the shaping grid frequencies.
    pub air_np: [f64; NF],
    pub forest_np: [f64; NF],
    pub foliage_np: [f64; NF],
    pub k: [f64; NF],
    pub beta: [[C64; NF]; GROUND_TYPES],
    pub obstacles: Vec<Obstacle>,
    pub er_trunks: Vec<Trunk>,
    pub reverb_send: f32,
    pub tail_extent: f32,
    /// Unit vector the wind blows towards, in this context's frame (x, y).
    pub wind_to: (f32, f32),
    /// Wind speed at 10 m driving refraction, m/s (mean, following the gusts at the
    /// listener with a 2 s lag), and the mean alone (turbulence).
    pub wind_ms: f32,
    pub wind_mean_ms: f32,
    /// Spectral shapes of the wind excess (dB on the grid) for a full shadow and a fully
    /// favourable (downwind) path; a path's wind EQ is shadow·S + favour·F.
    pub wind_shadow_db: [f32; NF],
    pub wind_favour_db: [f32; NF],
    crown_base: f32,
    dir_radii: [f64; 16],
    dir_mean: [[f64; NF]; 16],
}

impl Context {
    /// Sphere-averaged directivity power for radiator radius `radius` at grid index i.
    pub fn directivity_mean(&self, i: usize, radius: f64) -> f64 {
        let x = (radius / self.dir_radii[0]).ln() / (self.dir_radii[15] / self.dir_radii[0]).ln() * 15.0;
        let x = x.clamp(0.0, 15.0);
        let j = (x as usize).min(14);
        let t = x - j as f64;
        self.dir_mean[j][i] * (1.0 - t) + self.dir_mean[j + 1][i] * t
    }

    /// Path lengths (m) from s to l through (trunk zone beyond EXPLICIT_RADIUS, canopy).
    pub fn forest_lengths(&self, s: Vec3, l: Vec3) -> (f64, f64) {
        let d = l - s;
        let total = d.length() as f64;
        let rh = ((d.x * d.x + d.y * d.y) as f64).sqrt();
        let t_far = if rh > EXPLICIT_RADIUS as f64 { 1.0 - EXPLICIT_RADIUS as f64 / rh } else { 0.0 };
        let z_at = |z: f32| z as f64;
        // t-interval where z(t) lies in [lo, hi].
        let band = |lo: f64, hi: f64| -> (f64, f64) {
            let (z0, z1) = (z_at(s.z), z_at(l.z));
            if (z1 - z0).abs() < 1e-6 {
                return if z0 >= lo && z0 <= hi { (0.0, 1.0) } else { (0.0, 0.0) };
            }
            let ta = (lo - z0) / (z1 - z0);
            let tb = (hi - z0) / (z1 - z0);
            (ta.min(tb).max(0.0), ta.max(tb).min(1.0))
        };
        let gz = self.ground_z as f64;
        let (a0, a1) = band(gz - 10.0, gz + self.crown_base as f64);
        let trunk = (a1.min(t_far) - a0).max(0.0);
        let (c0, c1) = band(gz + self.crown_base as f64, gz + CANOPY_TOP as f64);
        let canopy = (c1 - c0).max(0.0);
        (trunk * total, canopy * total)
    }
}

/// Scratch buffers for propagators (kept out of the per-emitter state).
pub struct Scratch {
    pub a: [f32; BLOCK],
    pub b: [f32; BLOCK],
    pub c: [f32; BLOCK],
    pub orders: [[f32; BLOCK]; ORDER],
}

pub struct Acoustics {
    ctx: Context,
    sends: Sends,
    scratch: Scratch,
    props: Vec<Propagator>,
    bank: FilterBank,
    staged: Vec<bool>,
    /// Filter-bank slot (lanes 2s, 2s + 1) of each emitter, and the emitter in each slot: a
    /// permutation. Slots pair up into SIMD groups; a group whose two emitters sleep is not
    /// filtered, so waking emitters move next to an awake one (`place`).
    slot_of: Vec<usize>,
    occupant: Vec<usize>,
    reverb: ForestReverb,
    rotator: ShRotator,
    bed: bed::BedDecoder,
    measured: Option<BinauralDecoder>,
    analytic: BinauralDecoder,
    active: HrtfModel,
    env: Option<Environment>,
    environment_elapsed: f32,
    trees: Vec<Trunk>,
    procedural: bool,
    fixed_obstacles: Vec<Obstacle>,
    sigma_s: [f64; NF],
    sigma_tr: [f64; NF],
    floor_alpha: [[f64; NF]; GROUND_TYPES],
    foliage_iso: [f64; NF],
    reverb_shape: Coeffs,
    block: u64,
    stereo: [[f32; BLOCK]; 2],
}

/// ISO 9613-2 Annex A, attenuation through dense foliage (dB/m, 20..200 m), octave bands
/// 63 Hz .. 8 kHz.
const FOLIAGE_DB_PER_M: [f64; 8] = [0.02, 0.03, 0.04, 0.05, 0.06, 0.08, 0.09, 0.12];

impl Acoustics {
    pub fn new(sample_rate: f32, emitters: usize) -> Self {
        crate::delay::init();
        let design = Design::new(sample_rate);
        let freqs = design.freqs;

        // Ground admittances and random-incidence absorption per ground type.
        let mut beta = [[C64::ZERO; NF]; GROUND_TYPES];
        let mut floor_alpha = [[0.0f64; NF]; GROUND_TYPES];
        for g in 0..GROUND_TYPES {
            let gt = GroundType::from_index(g as u32);
            for i in 0..NF {
                beta[g][i] = ground::admittance(gt, freqs[i]);
                floor_alpha[g][i] = ground::random_incidence_absorption(gt, freqs[i]);
            }
        }

        // Trunk scattering widths averaged over a radius distribution (diameters 16..56 cm).
        const RADII: [(f64, f64); 5] = [(0.08, 0.15), (0.11, 0.25), (0.15, 0.3), (0.2, 0.2), (0.28, 0.1)];
        let mut sigma_s = [0.0f64; NF];
        let mut sigma_tr = [0.0f64; NF];
        for (a, w) in RADII {
            for i in 0..NF {
                let ka = 2.0 * core::f64::consts::PI * freqs[i] / 343.0 * a;
                let s = scattering::rigid_cylinder(ka, a);
                sigma_s[i] += w * s.sigma_s;
                sigma_tr[i] += w * s.sigma_tr;
            }
        }

        // ISO foliage table interpolated in log frequency.
        let mut foliage_iso = [0.0f64; NF];
        for i in 0..NF {
            let x = (freqs[i] / 63.0).log2().clamp(0.0, 7.0);
            let j = (x as usize).min(6);
            let t = x - j as f64;
            foliage_iso[i] = FOLIAGE_DB_PER_M[j] * (1.0 - t) + FOLIAGE_DB_PER_M[j + 1] * t;
        }

        // Directivity normalisation table.
        let mut dir_radii = [0.0f64; 16];
        let mut dir_mean = [[0.0f64; NF]; 16];
        for j in 0..16 {
            dir_radii[j] = 0.005 * 100f64.powf(j as f64 / 15.0);
            for i in 0..NF {
                dir_mean[j][i] = directivity::mean_power(freqs[i], dir_radii[j], 343.0);
            }
        }

        let analytic = BinauralDecoder::new(sample_rate);
        let measured = HrirSet::parse(hrtf::DEFAULT_SET)
            .ok()
            .map(|set| BinauralDecoder::from_filters(hrtf::magls_filters(&set, sample_rate), hrtf::DECODER_TAPS));

        let ctx = Context {
            sample_rate,
            nfc: NfcDesign::new(sample_rate, hrtf::DEFAULT_RADIUS),
            listener: Vec3::default(),
            ground_z: -1.5,
            ground: GroundType::ForestFloor,
            speed_of_sound: 343.0,
            epoch: 0,
            features: Features::default(),
            air_np: [0.0; NF],
            forest_np: [0.0; NF],
            foliage_np: [0.0; NF],
            k: [0.0; NF],
            beta,
            obstacles: Vec::with_capacity(MAX_OBSTACLES),
            er_trunks: Vec::with_capacity(ER_TRUNKS),
            reverb_send: 0.0,
            tail_extent: 30.0,
            wind_to: crate::windfield::blow_vector(270.0),
            wind_ms: 0.0,
            wind_mean_ms: 0.0,
            wind_shadow_db: core::array::from_fn(|i| {
                let rf = wind::Refraction { shadow: 1.0, ..Default::default() };
                wind::excess_db(freqs[i], &rf) as f32
            }),
            wind_favour_db: core::array::from_fn(|i| {
                let rf = wind::Refraction { favour: 1.0, ..Default::default() };
                wind::excess_db(freqs[i], &rf) as f32
            }),
            crown_base: 8.0,
            dir_radii,
            dir_mean,
            design,
        };
        let reverb = ForestReverb::new(sample_rate, &ctx.design);
        let props = (0..emitters).map(|i| Propagator::new(&ctx, i as u64)).collect();
        let bank = FilterBank::new(&ctx.design, 2 * emitters);
        // This spectral shape depends only on the fixed trunk model and fitting grid.
        let i1k = freqs.iter().position(|f| *f >= 1000.0).unwrap_or(NF / 2);
        let shape = std::array::from_fn(|i| 0.5 * (sigma_tr[i] / sigma_tr[i1k]).max(1e-6).ln());
        let reverb_shape = ctx.design.fit_min_phase(&shape, None);
        let mut a = Self {
            ctx,
            sends: Sends::new(),
            scratch: Scratch { a: [0.0; BLOCK], b: [0.0; BLOCK], c: [0.0; BLOCK], orders: [[0.0; BLOCK]; ORDER] },
            props,
            bank,
            staged: vec![false; emitters],
            slot_of: (0..emitters).collect(),
            occupant: (0..emitters).collect(),
            reverb,
            rotator: ShRotator::new(),
            bed: bed::BedDecoder::new(),
            measured,
            analytic,
            active: HrtfModel::Measured,
            env: None,
            environment_elapsed: 0.0,
            trees: Vec::with_capacity(MAX_TREES),
            procedural: true,
            fixed_obstacles: Vec::with_capacity(MAX_OBSTACLES),
            sigma_s,
            sigma_tr,
            floor_alpha,
            foliage_iso,
            reverb_shape,
            block: 0,
            stereo: [[0.0; BLOCK]; 2],
        };
        a.set_environment(&Environment::default());
        a
    }

    /// Makes room for `n` emitters (allocates: not on the audio thread).
    pub fn ensure_emitters(&mut self, n: usize) {
        while self.props.len() < n {
            let i = self.props.len() as u64;
            self.slot_of.push(self.props.len());
            self.occupant.push(self.props.len());
            self.props.push(Propagator::new(&self.ctx, i));
            self.staged.push(false);
        }
        self.bank.ensure(2 * n);
    }

    pub fn emitters(&self) -> usize {
        self.props.len()
    }

    pub fn context(&self) -> &Context {
        &self.ctx
    }

    pub fn tail(&self) -> reverb::TailParams {
        self.reverb.params
    }

    pub fn set_features(&mut self, f: Features) {
        self.ctx.features = f;
        self.ctx.epoch = self.ctx.epoch.wrapping_add(1);
    }

    /// Replaces the procedural forest with explicit trees (None = procedural from density).
    pub fn set_trees(&mut self, trees: Option<&[Trunk]>) {
        self.trees.clear();
        self.procedural = trees.is_none();
        if let Some(t) = trees {
            self.trees.extend_from_slice(&t[..t.len().min(MAX_TREES)]);
        }
        self.rebuild_geometry();
    }

    /// Real-time safe update for a streaming world (flight mode): replaces the explicit trunks
    /// and the extra obstacles (both listener-relative, truncated to the reserved capacity)
    /// without resetting the environment. With `refit` false the propagators keep their
    /// spectral fits and refit on their own motion thresholds, so a forest that moves past the
    /// listener does not force every emitter to refit at every update.
    pub fn update_geometry(&mut self, trees: &[Trunk], obstacles: &[Obstacle], refit: bool) {
        self.procedural = false;
        self.trees.clear();
        self.trees.extend_from_slice(&trees[..trees.len().min(MAX_TREES)]);
        self.fixed_obstacles.clear();
        self.fixed_obstacles.extend_from_slice(&obstacles[..obstacles.len().min(MAX_OBSTACLES / 2)]);
        let epoch = self.ctx.epoch;
        self.rebuild_geometry();
        if !refit {
            self.ctx.epoch = epoch;
        }
    }

    /// Extra obstacles (boulders, ridges, logs) in addition to the trunks.
    pub fn set_obstacles(&mut self, obstacles: &[Obstacle]) {
        self.fixed_obstacles.clear();
        self.fixed_obstacles.extend_from_slice(&obstacles[..obstacles.len().min(MAX_OBSTACLES / 2)]);
        self.rebuild_geometry();
    }

    /// Moves the listener's ears away from the rest position (head translation about the
    /// neck, world frame, metres). Real-time safe; affects delays, directions and near field.
    pub fn set_listener_offset(&mut self, offset: Vec3) {
        self.ctx.listener = offset;
    }

    /// The wind, once per block: (`to_x`, `to_y`) the unit vector it blows towards in this
    /// context's frame (world x north / y west for the scene; the heading frame in flight),
    /// `mean_ms` the mean speed and `now_ms` the speed at the listener with its gusts.
    /// Real-time safe.
    pub fn set_wind(&mut self, to_x: f32, to_y: f32, mean_ms: f32, now_ms: f32) {
        let n = (to_x * to_x + to_y * to_y).sqrt();
        if n > 1e-6 && n.is_finite() {
            self.ctx.wind_to = (to_x / n, to_y / n);
        }
        let fix = |v: f32| if v.is_finite() { v.clamp(0.0, 40.0) } else { 0.0 };
        let (mean, now) = (fix(mean_ms), fix(now_ms));
        self.ctx.wind_mean_ms = mean;
        // The near-ground profile adjusts to a gust over a couple of seconds.
        let a = (BLOCK as f32 / self.ctx.sample_rate / 2.0).min(1.0);
        self.ctx.wind_ms += (now - self.ctx.wind_ms) * a;
    }

    /// Applies weather/ground/forest parameters. Cheap when nothing changed; real-time safe.
    pub fn set_environment(&mut self, env: &Environment) {
        let env = env.sanitized();
        if self.env == Some(env) {
            return;
        }
        let old = self.env;
        self.env = Some(env);
        let ctx = &mut self.ctx;
        let c = air::speed_of_sound(env.temperature_c as f64);
        ctx.speed_of_sound = c as f32;
        ctx.nfc.set_speed_of_sound(c);
        ctx.ground = env.ground;
        ctx.ground_z = -env.ear_height;
        let n = env.tree_density_per_ha as f64 / 10_000.0;
        for i in 0..NF {
            let f = ctx.design.freqs[i];
            ctx.air_np[i] = air::absorption_db_per_m(f, env.temperature_c as f64, env.humidity_pct as f64, env.pressure_kpa as f64) / 8.686;
            ctx.k[i] = 2.0 * core::f64::consts::PI * f / c;
            ctx.forest_np[i] = 0.5 * n * self.sigma_tr[i];
            ctx.foliage_np[i] = env.foliage as f64 * self.foliage_iso[i] / 8.686;
        }

        // Late tail from geometry.
        let geo = ForestGeometry { density: n, crown_base: ctx.crown_base as f64, bark_absorption: BARK_ABSORPTION };
        let tp = reverb::tail_params(
            &geo,
            &ctx.design.freqs,
            &self.sigma_s,
            &self.sigma_tr,
            &self.floor_alpha[env.ground as usize],
            &ctx.air_np,
            env.foliage as f64,
            c,
        );
        self.reverb.configure_tail(tp, self.reverb_shape, env.reverb);
        ctx.reverb_send = if n > 0.0 { 1.0 } else { 0.0 };
        ctx.tail_extent = tp.extent as f32;

        let density_changed = old.map_or(true, |o| o.tree_density_per_ha != env.tree_density_per_ha || o.ear_height != env.ear_height);
        if density_changed {
            self.rebuild_geometry();
        }
        self.ctx.epoch = self.ctx.epoch.wrapping_add(1);
    }

    /// Continuous weather controls only need spectral redesign at 50 Hz. Path coefficients
    /// interpolate at audio rate; discrete renderer/ground and geometry edits land immediately.
    /// Streaming flight geometry continues to use `set_environment` directly.
    pub fn update_environment(&mut self, env: &Environment, dt: f32) {
        const INTERVAL: f32 = 0.02;
        self.environment_elapsed = (self.environment_elapsed + dt.max(0.0)).min(INTERVAL);
        let env = env.sanitized();
        if self.env == Some(env) { return; }
        let immediate = self.env.map_or(true, |old| {
            old.hrtf != env.hrtf || old.ground != env.ground || old.ear_height != env.ear_height
                || old.tree_density_per_ha != env.tree_density_per_ha
        });
        if immediate || self.environment_elapsed >= INTERVAL {
            self.set_environment(&env);
            self.environment_elapsed = 0.0;
        }
    }

    /// Trunks -> explicit obstacles near the listener + the early-reflection set.
    fn rebuild_geometry(&mut self) {
        let env = self.env.unwrap_or_default();
        let gz = -env.ear_height;
        if self.procedural {
            generate_forest(&mut self.trees, env.tree_density_per_ha as f64 / 10_000.0);
        }
        let l = self.ctx.listener;
        let dist = |t: &Trunk| ((t.x - l.x).powi(2) + (t.y - l.y).powi(2)).sqrt();
        let ctx = &mut self.ctx;
        ctx.obstacles.clear();
        ctx.obstacles.extend_from_slice(&self.fixed_obstacles);
        for t in self.trees.iter().filter(|t| dist(t) < EXPLICIT_RADIUS) {
            if ctx.obstacles.len() >= MAX_OBSTACLES {
                break;
            }
            ctx.obstacles.push(Obstacle::Cylinder { x: t.x, y: t.y, radius: t.radius, z0: gz, z1: gz + t.height });
        }
        // Nearest trunks for early reflections (selection without allocation).
        ctx.er_trunks.clear();
        let mut used = [usize::MAX; ER_TRUNKS];
        for slot in 0..ER_TRUNKS {
            let mut best: Option<(usize, f32)> = None;
            for (i, t) in self.trees.iter().enumerate() {
                if used.contains(&i) {
                    continue;
                }
                let d = dist(t);
                if best.map_or(true, |(_, bd)| d < bd) {
                    best = Some((i, d));
                }
            }
            if let Some((i, _)) = best {
                used[slot] = i;
                ctx.er_trunks.push(self.trees[i]);
            }
        }
        let mut cfg = [(Vec3::default(), 0.0f32, 0.0f32); ER_TRUNKS];
        for (j, t) in ctx.er_trunks.iter().enumerate() {
            let d = Vec3::new(t.x - l.x, t.y - l.y, 0.0);
            let r = d.length().max(0.1);
            cfg[j] = (d * (1.0 / r), t.radius, 2.0 * r + 10.0);
        }
        let i8k = ctx.design.freqs.iter().position(|f| *f >= 8000.0).unwrap_or(NF - 1);
        self.reverb.configure_trunks(&cfg[..ctx.er_trunks.len()], ctx.air_np[i8k], ctx.speed_of_sound as f64);
        ctx.ground_z = gz;
        ctx.epoch = ctx.epoch.wrapping_add(1);
    }

    /// Propagates one emitter's block. Its contribution is added to the bus by the next
    /// `render` (path filters run there, batched across emitters); `_bus` is the same bus.
    pub fn propagate(&mut self, id: usize, em: &Emitter, input: &[f32], _bus: &mut [[f32; BLOCK]; CHANNELS]) {
        let phase = self.block.wrapping_add(id as u64);
        if id >= self.props.len() {
            return;
        }
        if self.props[id].asleep() && input.iter().any(|v| *v != 0.0) {
            self.place(id);
        }
        let path = 2 * self.slot_of[id];
        self.props[id].stage(&self.ctx, em, input, &mut self.sends, &mut self.scratch, phase, &mut self.bank, path);
        self.staged[id] = true;
    }

    /// Emitter `id` is about to wake: give it a slot whose partner is awake, taken from a
    /// sleeper (they swap), so the pair's filter group is shared. Its lanes start cleared.
    fn place(&mut self, id: usize) {
        let awake = |s: usize, props: &[Propagator], occupant: &[usize]| {
            occupant.get(s).is_some_and(|&e| e != id && !props[e].asleep())
        };
        let own = self.slot_of[id];
        if !awake(own ^ 1, &self.props, &self.occupant) {
            let free = (0..self.occupant.len()).find(|&s| {
                let e = self.occupant[s];
                (e == id || self.props[e].asleep()) && awake(s ^ 1, &self.props, &self.occupant)
            });
            if let Some(s) = free {
                let other = self.occupant[s];
                self.occupant.swap(own, s);
                self.slot_of[id] = s;
                self.slot_of[other] = own;
            }
        }
        let path = 2 * self.slot_of[id];
        self.bank.reset(path);
        self.bank.reset(path + 1);
    }

    /// Adds reverb, rotates the bus into the head frame (`head`: head -> world) and decodes.
    pub fn render(&mut self, bus: &mut [[f32; BLOCK]; CHANNELS], head: &Mat3, len: usize, left: &mut [f32], right: &mut [f32]) {
        self.complete_world_bus(bus, len);
        self.rotate_to_head(bus, head, len);
        self.decode_binaural(bus, len, left, right);
        crate::profile::lap(crate::profile::Section::Decode);
        self.block = self.block.wrapping_add(1);
    }

    /// Like `render`, but for an external binaural renderer (Apple's AUSpatialMixer with the
    /// listener's personalised HRTF): decodes to a 7.1.4 loudspeaker bed (`bed::LAYOUT`).
    /// `head` = Some: rotated by our head pose (the bed is head-locked); None: world-locked,
    /// for a renderer that applies AirPods head tracking itself.
    pub fn render_bed(&mut self, bus: &mut [[f32; BLOCK]; CHANNELS], head: Option<&Mat3>, len: usize, out: &mut [[f32; BLOCK]; bed::CHANNELS]) {
        self.complete_world_bus(bus, len);
        if let Some(h) = head {
            self.rotate_to_head(bus, h, len);
        }
        self.bed.decode(bus, len, out);
        crate::profile::lap(crate::profile::Section::Decode);
        self.block = self.block.wrapping_add(1);
    }

    /// Path filters (batched), near field, encode, reverb: the full world-frame bus.
    fn complete_world_bus(&mut self, bus: &mut [[f32; BLOCK]; CHANNELS], len: usize) {
        use crate::profile::{lap, Section};
        // Path filters for all emitters, four paths per SIMD group, then near field + encode.
        for (id, staged) in self.staged.iter().enumerate() {
            // Awake but not rendered this block: its filters ring out on silence.
            if !*staged && !self.props[id].asleep() {
                self.bank.silence(2 * self.slot_of[id], len);
                self.bank.silence(2 * self.slot_of[id] + 1, len);
            }
        }
        // A group of two emitters is skipped when both sleep: their filter state is cleared,
        // their input silent and `finish` reads nothing back (most birds, rain and insects).
        let mut filtered = 0;
        for (g, pair) in self.occupant.chunks(2).enumerate() {
            if pair.iter().all(|&e| self.props[e].asleep()) {
                continue;
            }
            filtered += pair.len();
            for &e in pair {
                // A sleeper beside an awake partner: zero input keeps its cleared lanes at zero.
                if self.props[e].asleep() {
                    self.bank.silence(2 * self.slot_of[e], len);
                    self.bank.silence(2 * self.slot_of[e] + 1, len);
                }
            }
            let ramps = match *pair {
                [a, b] => {
                    let [pa, pb] = self.props.get_disjoint_mut([a, b]).expect("slots are a permutation");
                    let ((ad, ag), (bd, bg)) = (pa.coeffs_mut(), pb.coeffs_mut());
                    [Some(ad), Some(ag), Some(bd), Some(bg)]
                }
                [a] => {
                    let (ad, ag) = self.props[a].coeffs_mut();
                    [Some(ad), Some(ag), None, None]
                }
                _ => unreachable!(),
            };
            self.bank.process_group(g, len, ramps);
        }
        crate::profile::census(self.props.iter().filter(|p| !p.asleep()).count(), filtered);
        lap(Section::PathFilters);
        for (id, p) in self.props.iter_mut().enumerate() {
            p.finish(&self.ctx, &self.bank, 2 * self.slot_of[id], bus, &mut self.scratch);
            self.staged[id] = false;
        }
        lap(Section::Encode);

        if self.ctx.features.reverb || self.ctx.features.early_reflections {
            self.reverb.render(&self.sends, bus, len);
        }
        self.sends.clear(len);
        lap(Section::Reverb);

    }

    fn rotate_to_head(&mut self, bus: &mut [[f32; BLOCK]; CHANNELS], head: &Mat3, len: usize) {
        let m = head.m;
        let world_to_head = Mat3 { m: [[m[0][0], m[1][0], m[2][0]], [m[0][1], m[1][1], m[2][1]], [m[0][2], m[1][2], m[2][2]]] };
        let rot = self.rotator.matrix(&world_to_head);
        self.rotator.apply(bus, len, rot);
        crate::profile::lap(crate::profile::Section::Rotate);

    }

    fn decode_binaural(&mut self, bus: &mut [[f32; BLOCK]; CHANNELS], len: usize, left: &mut [f32], right: &mut [f32]) {
        let want = match (self.env.map_or(HrtfModel::Measured, |e| e.hrtf), self.measured.is_some()) {
            (HrtfModel::Measured, true) => HrtfModel::Measured,
            _ => HrtfModel::Analytic,
        };
        let decode = |model: HrtfModel, me: &mut Self, l: &mut [f32], r: &mut [f32]| match model {
            HrtfModel::Measured => me.measured.as_mut().unwrap().process(bus, len, l, r),
            HrtfModel::Analytic => me.analytic.process(bus, len, l, r),
        };
        if want == self.active {
            decode(want, self, left, right);
        } else {
            // Crossfade decoders over one block.
            let [mut ol, mut or] = self.stereo;
            decode(self.active, self, &mut ol[..len], &mut or[..len]);
            decode(want, self, left, right);
            for i in 0..len {
                let w = (i + 1) as f32 / len as f32;
                left[i] = left[i] * w + ol[i] * (1.0 - w);
                right[i] = right[i] * w + or[i] * (1.0 - w);
            }
            match self.active {
                HrtfModel::Measured => self.measured.as_mut().unwrap().reset(),
                HrtfModel::Analytic => self.analytic.reset(),
            }
            self.active = want;
        }
    }
}

/// Deterministic Poisson forest around the listener for the default "forest by the river"
/// scene: a small clearing around the listener and no trunks in the river channel ahead
/// (x in 2.5..7.5 m). The scene graph replaces this with `set_trees`.
fn generate_forest(out: &mut Vec<Trunk>, density: f64) {
    out.clear();
    let r = PROCEDURAL_RADIUS;
    let mut rng = Rng::new(0x7EE5);
    let count = (density * (2.0 * r as f64).powi(2)).round() as usize;
    for _ in 0..count {
        let x = rng.range(-r, r);
        let y = rng.range(-r, r);
        let radius = (0.15 * (0.35 * rng.gauss()).exp()).clamp(0.06, 0.4);
        if out.len() >= MAX_TREES || x * x + y * y > r * r || x * x + y * y < 2.5 * 2.5 || (2.5..7.5).contains(&x) {
            continue;
        }
        out.push(Trunk { x, y, radius, height: 15.0 });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacing_trees_preserves_the_environment() {
        let mut acoustics = Acoustics::new(48_000.0, 0);
        let environment = Environment { temperature_c: 35.0, ear_height: 2.4,
            humidity_pct: 40.0, reverb: 0.3, ..Environment::default() };
        acoustics.set_environment(&environment);
        let speed = acoustics.context().speed_of_sound;
        acoustics.set_trees(Some(&[]));
        assert_eq!(acoustics.env, Some(environment));
        assert_eq!(acoustics.context().speed_of_sound, speed);
        assert_eq!(acoustics.context().ground_z, -2.4);
        acoustics.set_trees(None);
        assert_eq!(acoustics.env, Some(environment));
    }

    #[test]
    fn weather_redesign_is_bounded_but_renderer_changes_are_immediate() {
        let mut acoustics = Acoustics::new(48_000.0, 0);
        let epoch = acoustics.context().epoch;
        let mut environment = Environment::default();
        for step in 1..20 {
            environment.temperature_c = 15.0 + step as f32 * 0.1;
            acoustics.update_environment(&environment, 0.001);
        }
        assert_eq!(acoustics.context().epoch, epoch);
        acoustics.update_environment(&environment, 0.002);
        assert_ne!(acoustics.context().epoch, epoch);
        assert_eq!(acoustics.env, Some(environment));
        environment.hrtf = HrtfModel::Analytic;
        acoustics.update_environment(&environment, 0.001);
        assert_eq!(acoustics.env.unwrap().hrtf, HrtfModel::Analytic);
    }
    use crate::physics::ground;
    use core::f64::consts::PI;

    const FS: f32 = 48_000.0;

    fn quiet() -> Features {
        Features { ground: false, reverb: false, early_reflections: false, occlusion: false, forest: false, near_field: false, wind: false }
    }

    /// Runs `blocks` blocks with input from `src(sample)` and emitter state from `em(block)`;
    /// returns the W (omni) channel of the world-frame bus.
    fn run_w(ac: &mut Acoustics, blocks: usize, src: impl Fn(usize) -> f32, em: impl Fn(usize) -> Emitter) -> Vec<f32> {
        let mut out = Vec::with_capacity(blocks * BLOCK);
        let mut l = [0.0f32; BLOCK];
        let mut r = [0.0f32; BLOCK];
        for b in 0..blocks {
            let x: [f32; BLOCK] = core::array::from_fn(|i| src(b * BLOCK + i));
            let mut bus = [[0.0f32; BLOCK]; CHANNELS];
            ac.propagate(0, &em(b), &x, &mut bus);
            ac.render(&mut bus, &Mat3::from_head(0.0, 0.0, 0.0), BLOCK, &mut l, &mut r);
            out.extend_from_slice(&bus[0]);
        }
        out
    }

    /// Magnitude (dB) of a real impulse response at frequency f.
    fn level_db(h: &[f32], f: f64) -> f64 {
        let (mut re, mut im) = (0.0f64, 0.0f64);
        for (n, v) in h.iter().enumerate() {
            let w = 2.0 * PI * f * n as f64 / FS as f64;
            re += *v as f64 * w.cos();
            im -= *v as f64 * w.sin();
        }
        10.0 * (re * re + im * im).log10()
    }

    /// Frequency of a signal from interpolated rising zero crossings.
    fn frequency(x: &[f32]) -> f64 {
        let mut crossings = Vec::new();
        for n in 1..x.len() {
            if x[n - 1] < 0.0 && x[n] >= 0.0 {
                crossings.push(n as f64 - 1.0 + (-x[n - 1] / (x[n] - x[n - 1])) as f64);
            }
        }
        let k = crossings.len() - 1;
        k as f64 * FS as f64 / (crossings[k] - crossings[0])
    }

    /// A source approaching at v is heard at f c / (c - v) (moving-source Doppler), not at
    /// f (1 + v/c) (moving listener): the retarded-time delay gets the asymmetry right.
    #[test]
    fn doppler_shift_of_a_moving_source() {
        let mut ac = Acoustics::new(FS, 1);
        ac.set_features(quiet());
        let c = ac.context().speed_of_sound as f64;
        let v = 20.0f32;
        let f0 = 1000.0f64;
        let em = |b: usize| {
            let t = (b * BLOCK) as f32 / FS;
            let mut e = Emitter::at(Vec3::new(80.0 - v * t, 0.0, 0.0), 1.0);
            e.velocity = Some(Vec3::new(-v, 0.0, 0.0));
            e
        };
        let w = run_w(&mut ac, 1500, |n| (2.0 * PI * f0 * n as f64 / FS as f64).sin() as f32, em);
        let got = frequency(&w[48_000..96_000]);
        let want = f0 * c / (c - v as f64);
        eprintln!("doppler: heard {got:.2} Hz, moving source {want:.2} Hz, moving-listener formula {:.2} Hz", f0 * (1.0 + v as f64 / c));
        assert!((got - want).abs() < 0.3, "heard {got:.2} Hz, moving-source Doppler {want:.2} Hz");
        assert!((got - f0 * (1.0 + v as f64 / c)).abs() > 2.0);
    }

    fn impulse_response(ac: &mut Acoustics, em: Emitter, blocks: usize) -> Vec<f32> {
        run_w(ac, blocks, |n| (n == 0) as u8 as f32, move |_| em)
    }

    /// Over rigid ground the direct and image paths cancel first at f = c / (2 dr).
    #[test]
    fn ground_effect_dip_over_rigid_ground() {
        let mut ac = Acoustics::new(FS, 1);
        ac.set_features(Features { ground: true, ..quiet() });
        let c = ac.context().speed_of_sound as f64;
        // Ear 1.5 m above ground; source at the same height, 10 m away.
        let mut em = Emitter::at(Vec3::new(10.0, 0.0, 0.0), 1.0);
        em.ground = Some(GroundType::Water);
        let h = impulse_response(&mut ac, em, 64);
        let dr = (100.0f64 + 9.0).sqrt() - 10.0;
        let want = c / (2.0 * dr);
        let mut best = (0.0, f64::MAX);
        let mut f = 200.0;
        while f < 700.0 {
            let l = level_db(&h, f);
            if l < best.1 {
                best = (f, l);
            }
            f += 0.5;
        }
        eprintln!("rigid ground dip: {} Hz (theory {want:.1} Hz), depth {:.1} dB re 2f", best.0, best.1 - level_db(&h, 2.0 * want));
        assert!((best.0 / want - 1.0).abs() < 0.02, "dip at {} Hz, theory {want:.1} Hz", best.0);
        assert!(best.1 < level_db(&h, 2.0 * want) - 20.0, "dip depth");
    }

    /// Over grass the rendered response follows |1 + Q r1/r2 e^{-ik dr}| / r1 with the
    /// spherical-wave reflection coefficient.
    #[test]
    fn ground_effect_over_grass_matches_theory() {
        let mut ac = Acoustics::new(FS, 1);
        let env = Environment { ground: GroundType::Grass, ..Environment::default() };
        ac.set_environment(&env);
        ac.set_features(Features { ground: true, ..quiet() });
        let c = ac.context().speed_of_sound as f64;
        let (hs, hr, d) = (0.5f64, 1.5f64, 20.0f64);
        let em = Emitter::at(Vec3::new(d as f32, 0.0, (hs - hr) as f32), 1.0);
        let h = impulse_response(&mut ac, em, 64);
        let r1 = (d * d + (hs - hr).powi(2)).sqrt();
        let r2 = (d * d + (hs + hr).powi(2)).sqrt();
        let sin_psi = (hs + hr) / r2;
        for f in [100.0, 200.0, 300.0, 500.0, 800.0, 1200.0, 2000.0, 4000.0] {
            let k = 2.0 * PI * f / c;
            let air = |r: f64| 10f64.powf(-air::absorption_db_per_m(f, 15.0, 70.0, 101.325) * r / 20.0);
            let q = ground::spherical_reflection(ground::admittance(GroundType::Grass, f), k, r2, sin_psi);
            let p = C64::new(air(r1) / r1, 0.0) + q * C64::from_polar(air(r2) / r2, k * (r2 - r1));
            let want = 20.0 * p.abs().log10();
            let got = level_db(&h, f);
            eprintln!("grass f={f}: rendered {got:.2} dB, theory {want:.2} dB");
            assert!((got - want).abs() < 1.5, "f={f}: rendered {got:.2} dB, theory {want:.2} dB");
        }
    }

    /// End to end, a source 100 m away is attenuated as ISO 9613-1 says.
    #[test]
    fn air_absorption_end_to_end() {
        let mut ac = Acoustics::new(FS, 1);
        ac.set_features(quiet());
        let h = impulse_response(&mut ac, Emitter::at(Vec3::new(100.0, 0.0, 0.0), 1.0), 320);
        let base = -40.0; // 1/r at 100 m
        for f in [125.0, 1000.0, 4000.0, 8000.0, 12_000.0] {
            let want = base - air::absorption_db_per_m(f, 15.0, 70.0, 101.325) * 100.0;
            let got = level_db(&h, f);
            eprintln!("air 100 m f={f}: {got:.2} dB vs ISO {want:.2} dB");
            assert!((got - want).abs() < 0.6, "f={f}: {got:.2} vs {want:.2}");
        }
    }

    /// A trunk between source and listener shadows high frequencies but not low ones.
    #[test]
    fn trunk_occludes_high_frequencies() {
        let mut ac = Acoustics::new(FS, 1);
        ac.set_trees(Some(&[Trunk { x: 1.0, y: 0.0, radius: 0.25, height: 15.0 }]));
        ac.set_features(Features { occlusion: true, ..quiet() });
        let blocked = impulse_response(&mut ac, Emitter::at(Vec3::new(10.0, 0.0, 0.0), 1.0), 64);
        let mut ac = Acoustics::new(FS, 1);
        ac.set_trees(Some(&[]));
        ac.set_features(Features { occlusion: true, ..quiet() });
        let free = impulse_response(&mut ac, Emitter::at(Vec3::new(10.0, 0.0, 0.0), 1.0), 64);
        let loss = |f: f64| level_db(&free, f) - level_db(&blocked, f);
        eprintln!("trunk occlusion: loss 100 Hz {:.2} dB, 6 kHz {:.2} dB", loss(100.0), loss(6000.0));
        assert!(loss(100.0) < 2.5, "100 Hz loss {}", loss(100.0));
        assert!(loss(6000.0) > 8.0, "6 kHz loss {}", loss(6000.0));
    }

    /// Ear signals: energy in a band, and the interaural level difference.
    fn ild_db(ac: &mut Acoustics, pos: Vec3, head: Mat3, band: (f64, f64)) -> f64 {
        let mut l_all = Vec::new();
        let mut r_all = Vec::new();
        let mut l = [0.0f32; BLOCK];
        let mut r = [0.0f32; BLOCK];
        for b in 0..64 {
            let x: [f32; BLOCK] = core::array::from_fn(|i| (b == 0 && i == 0) as u8 as f32);
            let mut bus = [[0.0f32; BLOCK]; CHANNELS];
            ac.propagate(0, &Emitter::at(pos, 1.0), &x, &mut bus);
            ac.render(&mut bus, &head, BLOCK, &mut l, &mut r);
            l_all.extend_from_slice(&l);
            r_all.extend_from_slice(&r);
        }
        let e = |h: &[f32]| {
            let mut s = 0.0;
            let mut f = band.0;
            while f <= band.1 {
                s += 10f64.powf(level_db(h, f) / 10.0);
                f *= 1.05;
            }
            s
        };
        10.0 * (e(&l_all) / e(&r_all)).log10()
    }

    /// Near-field: a source at 0.3 m to the left has a much larger low-frequency ILD than
    /// the same direction at 2 m (NFC-HOA range extrapolation of the measured HRTFs).
    #[test]
    fn near_field_increases_low_frequency_ild() {
        let mut ac = Acoustics::new(FS, 1);
        ac.set_features(Features { near_field: true, ..quiet() });
        let head = Mat3::from_head(0.0, 0.0, 0.0);
        let far = ild_db(&mut ac, Vec3::new(0.0, 2.0, 0.0), head, (300.0, 900.0));
        let mut ac = Acoustics::new(FS, 1);
        ac.set_features(Features { near_field: true, ..quiet() });
        let near = ild_db(&mut ac, Vec3::new(0.0, 0.3, 0.0), head, (300.0, 900.0));
        eprintln!("near-field ILD 300-900 Hz: 0.3 m {near:.2} dB, 2 m {far:.2} dB");
        assert!(far > 2.0, "far ILD {far}");
        assert!(near > far + 3.0, "near ILD {near} dB vs far {far} dB");
    }

    /// Turning the head 90 degrees left puts a source in front on the right (SH rotation).
    #[test]
    fn head_rotation_moves_the_image() {
        let mut ac = Acoustics::new(FS, 1);
        ac.set_features(quiet());
        let front = Vec3::new(3.0, 0.0, 0.0);
        let straight = ild_db(&mut ac, front, Mat3::from_head(0.0, 0.0, 0.0), (500.0, 8000.0));
        let mut ac = Acoustics::new(FS, 1);
        ac.set_features(quiet());
        let turned = ild_db(&mut ac, front, Mat3::from_head(core::f32::consts::FRAC_PI_2, 0.0, 0.0), (500.0, 8000.0));
        assert!(straight.abs() < 2.5, "front ILD {straight}");
        assert!(turned < -8.0, "after turning left the source is on the right: ILD {turned}");
    }

    /// Measured and analytic decoders give similar loudness (switching must not jump).
    #[test]
    fn decoders_are_level_matched() {
        let mut lv = [0.0f64; 2];
        for (i, model) in [HrtfModel::Measured, HrtfModel::Analytic].into_iter().enumerate() {
            let mut ac = Acoustics::new(FS, 1);
            ac.set_environment(&Environment { hrtf: model, ..Environment::default() });
            ac.set_features(quiet());
            let mut e = 0.0f64;
            let mut l = [0.0f32; BLOCK];
            let mut r = [0.0f32; BLOCK];
            let mut seed = 1u32;
            for _ in 0..400 {
                let x: [f32; BLOCK] = core::array::from_fn(|_| {
                    seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    (seed >> 8) as f32 / 16_777_216.0 - 0.5
                });
                let mut bus = [[0.0f32; BLOCK]; CHANNELS];
                ac.propagate(0, &Emitter::at(Vec3::new(2.0, 1.0, 0.3), 1.0), &x, &mut bus);
                ac.render(&mut bus, &Mat3::from_head(0.0, 0.0, 0.0), BLOCK, &mut l, &mut r);
                e += l.iter().chain(&r).map(|v| (*v as f64).powi(2)).sum::<f64>();
            }
            lv[i] = 10.0 * e.log10();
        }
        assert!((lv[0] - lv[1]).abs() < 3.0, "measured {:.1} dB vs analytic {:.1} dB", lv[0], lv[1]);
    }

    /// Wind from the north (blowing to −x), 6 m/s. Sound travelling upwind (source 150 m
    /// south, downwind of the listener) is in the refractive shadow: much darker than the same
    /// source 150 m north (sound carried downwind); crosswind lies in between.
    #[test]
    fn sound_travelling_upwind_is_darker_than_downwind() {
        let tilt = |pos: Vec3| {
            let mut ac = Acoustics::new(FS, 1);
            ac.set_features(Features { wind: true, ..quiet() });
            for _ in 0..2000 {
                ac.set_wind(-1.0, 0.0, 6.0, 6.0);
            }
            let h = impulse_response(&mut ac, Emitter::at(pos, 1.0), 420);
            let (lo, hi) = (level_db(&h, 250.0), level_db(&h, 4000.0));
            (hi, hi - lo)
        };
        let (hi_up, up) = tilt(Vec3::new(-150.0, 0.0, 0.0));
        let (hi_down, down) = tilt(Vec3::new(150.0, 0.0, 0.0));
        let (hi_cross, cross) = tilt(Vec3::new(0.0, 150.0, 0.0));
        eprintln!("150 m, 6 m/s: 4 kHz re 250 Hz upwind {up:.1} dB, crosswind {cross:.1} dB, downwind {down:.1} dB; 4 kHz level upwind {hi_up:.1}, crosswind {hi_cross:.1}, downwind {hi_down:.1} dB");
        // (Downwind lifts the low-mid ground-effect band most, so its tilt is not above the
        // crosswind one; its level is.)
        assert!(up < cross - 6.0 && up < down - 6.0, "up {up:.1} cross {cross:.1} down {down:.1}");
        assert!(hi_up < hi_cross - 6.0 && hi_cross < hi_down + 3.0);
        assert!(down - up > 8.0 && down - up < 25.0);
        // Broadband too (the turbulent wander of one realisation is a few dB at most).
        assert!(hi_up < hi_down - 8.0);
    }

    /// Far paths wander in level with the turbulence (a few dB at 150 m), near ones do not.
    #[test]
    fn turbulence_makes_far_sources_come_and_go() {
        let spread = |r: f32| {
            let mut ac = Acoustics::new(FS, 1);
            ac.set_features(Features { wind: true, ..quiet() });
            ac.set_wind(0.0, -1.0, 6.0, 6.0);
            let blocks = (40.0 * FS) as usize / BLOCK;
            let w = run_w(&mut ac, blocks, |n| (n as f32 * 0.13).sin(), move |_| Emitter::at(Vec3::new(r, 0.0, 0.0), 1.0));
            let lv: Vec<f32> = w.chunks(BLOCK * 64).skip(12).map(|c| 10.0 * (c.iter().map(|v| v * v).sum::<f32>() / c.len() as f32).log10()).collect();
            let m = lv.iter().sum::<f32>() / lv.len() as f32;
            (lv.iter().map(|x| (x - m).powi(2)).sum::<f32>() / lv.len() as f32).sqrt()
        };
        let (near, far) = (spread(5.0), spread(150.0));
        eprintln!("level sd over 40 s at 6 m/s crosswind: 5 m {near:.2} dB, 150 m {far:.2} dB");
        assert!(near < 0.4 && far > 1.5 && far < 6.0);
    }

    /// Sleeping emitters are not filtered and a waking one moves next to an awake partner; an
    /// emitter's sound must not depend on that: with every effect linear, the scene equals
    /// the sum of each emitter rendered alone.
    #[test]
    fn slot_moves_on_wake_keep_superposition() {
        // 0 and 2 never sound; 1 sounds throughout; 3 wakes after sleeping for 0.5 s.
        let wake = (0.5 * FS) as usize / BLOCK;
        let blocks = wake + 120;
        let noise = |seed: u32, n: usize| {
            let mut x = seed.wrapping_mul(0x9E37_79B9) ^ n as u32;
            x ^= x >> 15;
            x = x.wrapping_mul(0x2C1B_3C6D);
            x ^= x >> 12;
            (x as f32 / u32::MAX as f32) * 2.0 - 1.0
        };
        let at = [Vec3::new(3.0, 1.0, 0.0), Vec3::new(6.0, -4.0, 1.0), Vec3::new(-8.0, 2.0, 0.5), Vec3::new(-2.0, -7.0, 2.0)];
        let run = |a: bool, b: bool| -> (Vec<[f32; CHANNELS]>, Acoustics) {
            let mut ac = Acoustics::new(FS, 4);
            let mut l = [0.0f32; BLOCK];
            let mut r = [0.0f32; BLOCK];
            let mut out = Vec::new();
            for blk in 0..blocks {
                let mut bus = [[0.0f32; BLOCK]; CHANNELS];
                for (id, p) in at.iter().enumerate() {
                    let on = (id == 1 && a) || (id == 3 && b && blk >= wake);
                    let x: [f32; BLOCK] = core::array::from_fn(|i| if on { noise(id as u32 + 1, blk * BLOCK + i) * 0.3 } else { 0.0 });
                    ac.propagate(id, &Emitter::at(*p, 1.0), &x, &mut bus);
                }
                ac.render(&mut bus, &Mat3::from_head(0.0, 0.0, 0.0), BLOCK, &mut l, &mut r);
                if blk >= wake {
                    for i in 0..BLOCK {
                        out.push(core::array::from_fn(|ch| bus[ch][i]));
                    }
                }
            }
            (out, ac)
        };
        let (both, ac) = run(true, true);
        let (only_a, _) = run(true, false);
        let (only_b, _) = run(false, true);
        assert_eq!(ac.slot_of[3] ^ 1, ac.slot_of[1], "the waking emitter pairs with the awake one");
        assert!(ac.props[0].asleep() && ac.props[2].asleep());
        let peak = both.iter().flatten().fold(0.0f32, |m, v| m.max(v.abs()));
        let err = both.iter().zip(only_a.iter().zip(&only_b))
            .flat_map(|(x, (a, b))| (0..CHANNELS).map(move |ch| (x[ch] - a[ch] - b[ch]).abs()))
            .fold(0.0f32, f32::max);
        let b_peak = only_b.iter().flatten().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(b_peak > 0.01 * peak, "the woken emitter is heard ({b_peak} vs {peak})");
        assert!(err < 1e-4 * peak, "superposition error {err} vs peak {peak}");
    }

    /// An emitter sleeps soon after its sound has left the delay line (not after the whole
    /// buffer), and what sleep cuts off is already below -80 dB.
    #[test]
    fn sleep_follows_the_path_and_cuts_nothing_audible() {
        for (dist, ground) in [(2.0f32, true), (40.0, true), (120.0, false)] {
            let mut ac = Acoustics::new(FS, 1);
            // The shared reverb rings on by itself; measure the emitter's own paths.
            ac.set_features(Features { ground, reverb: false, early_reflections: false, ..ac.ctx.features });
            let mut l = [0.0f32; BLOCK];
            let mut r = [0.0f32; BLOCK];
            let on = (0.3 * FS) as usize / BLOCK;
            let (mut peak, mut last, mut slept) = (0.0f32, 0.0f32, None);
            for blk in 0..(1.5 * FS) as usize / BLOCK {
                let x: [f32; BLOCK] = core::array::from_fn(|i| if blk < on { ((blk * BLOCK + i) as f32 * 0.37).sin() * 0.5 } else { 0.0 });
                let mut bus = [[0.0f32; BLOCK]; CHANNELS];
                ac.propagate(0, &Emitter::at(Vec3::new(dist, 0.0, 0.0), 1.0), &x, &mut bus);
                ac.render(&mut bus, &Mat3::from_head(0.0, 0.0, 0.0), BLOCK, &mut l, &mut r);
                let level = bus.iter().flat_map(|c| c.iter()).fold(0.0f32, |m, v| m.max(v.abs()));
                if ac.props[0].asleep() {
                    slept = Some(blk);
                    break;
                }
                peak = peak.max(level);
                last = level;
            }
            let slept = slept.expect("the emitter sleeps") as f32 * BLOCK as f32 / FS - 0.3;
            let flight = dist / ac.ctx.speed_of_sound;
            assert!(slept < flight + 0.3, "{dist} m: slept {slept:.2} s after the sound stopped");
            assert!(last < 1e-4 * peak, "{dist} m: cut at {:.0} dB", 20.0 * (last / peak).log10());
        }
    }
}
