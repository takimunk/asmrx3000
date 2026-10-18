//! Flight mode: the listener travels forward through an endless, procedurally generated
//! landscape (docs/flight-mode.md).
//!
//! * [`land`] streams a deterministic landscape in chunks ahead of the listener and retires
//!   it behind: forest stands, clearings and meadows, brooks and rivers crossing the path,
//!   lakes, ridges and boulders, waterfalls, campfires, cricket meadows, and the birds that
//!   live in the trees.
//! * The world is rigid; the listener moves along the path at the flight speed (with gentle
//!   curves and altitude changes). Every emitter position is listener-relative, with the
//!   exact relative velocity, so the propagation produces Doppler, parallax, approach and
//!   recession, changing occlusion and ground effect by itself.
//! * Level of detail: fixed pools of pre-built emitters (generators are reconfigured in
//!   place with `Generator::set_model`, never rebuilt). The nearest trees get their own
//!   crown emitter, the rest of the forest is voiced by aggregate cells; water, insects,
//!   fire and birds take the nearest / loudest sites. Slots cross-fade when they change hands.
//! * Nothing on the audio thread allocates: storage is sized in [`Flight::new`], which runs
//!   on whichever thread turns flight mode on (see `Engine::set_param`).

pub mod airflow;
pub mod bridge;
pub mod land;
#[cfg(test)]
mod tests;

use crate::acoustics::{Acoustics, Emitter as Source, Environment as Acoustic, Trunk};
use crate::ambisonics::CHANNELS;
use crate::controls::{self, id, Bases, ControlState};
use crate::director::Species;
use crate::engine::{scope, scope_track, SourceKind, BLOCK, SCOPE_TRACKS};
use crate::mix::{EmitterView, Mix, SoundClass};
use crate::math::{Mat3, Vec3};
use crate::physics::diffraction::Obstacle;
use crate::physics::ground::GroundType;
use crate::rng::Rng;
use crate::scene::{object_flags, vary_water, Emitter, Entity, Geometry, ObjectInfo, ObjectKind, World};
use crate::sources::{
    Ambient, Bird as Voice, Controls, Fire, Flyer, Foliage, Insects, ModelConfig, ModelKind, Rain, Water, Wings,
};
use crate::windfield::WindField;

use airflow::Airflow;
use bridge::{FlightBridge, Snapshot};
use land::{BirdState, FeatureKind, Land, SiteKind, BANDS, CHUNK, MAX_CHUNKS};

/// The listener starts this far along the path (so there is landscape behind, too).
const START_S: f64 = 120.0;
/// Level-of-detail / geometry update period, blocks (~5 ms at 48 kHz).
const LOD_EVERY: u64 = 4;
/// Cross-fade time of a slot changing hands, s.
const FADE_S: f32 = 0.35;
/// Fade-out before a landscape change (seed / biome) is applied, s.
const REGEN_S: f32 = 0.8;
/// Speed changes are limited to this acceleration, m/s².
const ACCEL: f32 = 2.5;
/// Farthest distance any emitter is voiced from (delay lines carry ~235 m).
const MAX_RANGE: f32 = 220.0;

/// Ids of objects reported to the UI that have a shape: see `asmr_get_object_path`.
pub use bridge::PATH_POINTS;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Pool {
    Near,
    Cell,
    Bird,
    Water,
    Insects,
    Fire,
    Rain,
}

/// (pool, slots, voiced at once). Spare slots let outgoing voices fade while new ones fade in.
const POOLS: [(Pool, usize, usize); 7] = [
    (Pool::Near, 14, 10),
    (Pool::Cell, 8, 6),
    (Pool::Bird, 10, 8),
    (Pool::Water, 8, 6),
    (Pool::Insects, 4, 3),
    (Pool::Fire, 2, 1),
    (Pool::Rain, 4, 4),
];

/// Emitters in flight mode (fixed: the level-of-detail bound). At most 38 sound at once
/// (the voiced counts above); the rest are spares that fade out while others fade in.
pub const EMITTERS: usize = 50;

/// Species of the bird slots (a streamed bird takes a slot of its species when one is free).
const BIRD_SLOTS: [Species; 10] = [
    Species::Chaffinch,
    Species::Chaffinch,
    Species::Robin,
    Species::Robin,
    Species::SongThrush,
    Species::SongThrush,
    Species::Nightingale,
    Species::WoodPigeon,
    Species::Cuckoo,
    Species::TawnyOwl,
];

#[derive(Clone, Copy, PartialEq, Debug)]
enum Target {
    None,
    Tree { abs: u64, id: u32 },
    Cell { k: i64, band: u8 },
    Bird { idx: u16, id: u32 },
    Site { idx: u16, id: u32 },
    Rain(u8),
}

#[derive(Clone, Copy, Debug)]
struct Slot {
    pool: Pool,
    target: Target,
    /// Fading in (true) or out.
    want: bool,
    fade: f32,
    /// Extra gain (aggregate cells: share of their forest not voiced by near trees).
    gain: f32,
    /// Crown area a cell slot was configured with.
    ref_area: f32,
    /// Size of the sounding area (crown radius, half a river segment, a patch), m: an
    /// extended source is not louder than its surface when you are inside or next to it.
    extent: f32,
    species: Species,
    vel: Vec3,
}

/// Mix-layer slots of the flight emitters start after the scene's.
const MIX_SLOT0: usize = crate::engine::MAX_SOURCES;
const _: () = assert!(MIX_SLOT0 + EMITTERS <= crate::mix::MAX_SLOTS);

/// Identity of what a slot voices (the mix layer resets its history when it changes).
fn target_key(t: &Target) -> u64 {
    match *t {
        Target::None => 0,
        Target::Tree { abs, id } => 1 << 60 | abs << 20 | id as u64,
        Target::Cell { k, band } => 2 << 60 | ((k as u64) << 8 & 0x0FFF_FFFF_FFFF_FF00) | band as u64,
        Target::Bird { idx, id } => 3 << 60 | (idx as u64) << 32 | id as u64,
        Target::Site { idx, id } => 4 << 60 | (idx as u64) << 32 | id as u64,
        Target::Rain(i) => 5 << 60 | i as u64,
    }
}

/// Best-`N` candidates by priority (descending), fixed size.
struct Best<const N: usize> {
    items: [(f32, Target); N],
    n: usize,
}

impl<const N: usize> Best<N> {
    fn new() -> Self {
        Self { items: [(0.0, Target::None); N], n: 0 }
    }

    fn push(&mut self, prio: f32, t: Target) {
        if !(prio > 0.0) || (self.n == N && prio <= self.items[N - 1].0) {
            return;
        }
        let mut i = self.n.min(N - 1);
        if self.n < N {
            self.n += 1;
        }
        while i > 0 && self.items[i - 1].0 < prio {
            self.items[i] = self.items[i - 1];
            i -= 1;
        }
        self.items[i] = (prio, t);
    }

    fn find(&self, t: Target) -> Option<f32> {
        self.items[..self.n].iter().find(|x| x.1 == t).map(|x| x.0)
    }
}

/// What the engine hands the flight renderer each block.
pub struct Inputs<'a> {
    pub st: &'a ControlState,
    pub bases: &'a Bases,
    pub hour: f32,
    /// BIRDS slider 0..1.
    pub birds: f32,
    /// Rain suppression of birds 0..1.
    pub rain: f32,
    pub ambient: Ambient,
    pub head: Mat3,
    /// Head yaw relative to the direction of travel, rad (+ = left).
    pub head_yaw: f32,
    pub acoustic: Acoustic,
    pub wind: &'a WindField,
    /// WIND slider 0..1.
    pub wind_slider: f32,
    pub gustiness: f32,
    pub water: f32,
    pub neural: bool,
}

/// Listener pose on the path.
#[derive(Clone, Copy, Debug, Default)]
pub struct Pose {
    /// Distance travelled along the path, m.
    pub s: f64,
    pub x: f64,
    pub y: f64,
    pub heading: f64,
    /// Ears above the ground, m.
    pub height: f32,
    pub speed: f32,
    /// Turn rate, rad/s (+ = turning left).
    pub turn: f32,
    /// Vertical speed, m/s.
    pub climb: f32,
    /// sin / cos of the heading.
    sin: f64,
    cos: f64,
}

pub struct Flight {
    sample_rate: f32,
    pub land: Land,
    /// Slot entities / emitters (positions are listener-relative).
    pub world: World,
    pub acoustics: Acoustics,
    slots: Box<[Slot]>,
    pose: Pose,
    altitude: f32,
    curviness: f32,
    density: f32,
    seed: u64,
    seed_ctl: u32,
    biome_ctl: u32,
    regen: f32,
    blocks: u64,
    airflow: Airflow,
    bus: [[f32; BLOCK]; CHANNELS],
    bed: [[f32; BLOCK]; crate::bed::CHANNELS],
    scratch: [f32; BLOCK],
    left: [f32; BLOCK],
    right: [f32; BLOCK],
    trunks: Box<[Trunk]>,
    obstacles: Box<[Obstacle]>,
    /// Crown area voiced by near-tree slots, per (chunk ring slot, band).
    voiced: [f32; MAX_CHUNKS * BANDS],
    env_key: (u32, f32, f32),
    refit_s: f64,
    ctl: Option<ControlState>,
    dirty: bool,
    birds_level: f32,
}

/// Crown sound of `area` m². `exposure`: wind in the crown relative to the open canopy
/// top. Inside a closed stand the wind decays into the canopy (u(z) = u_h·e^{a(z/h - 1)},
/// a ≈ 2..3, Inoue 1963 / Cionco 1965), so interior crowns see a fraction of it; lone and
/// edge trees the full wind.
fn foliage(species: crate::scene::TreeSpecies, area: f32, exposure: f32) -> ModelConfig {
    let mut c = Foliage::preset(species.foliage_preset()).unwrap_or_default();
    c.area_m2 = area.clamp(0.5, 2000.0);
    c.exposure = (c.exposure * exposure).clamp(0.0, 2.5);
    ModelConfig::Foliage(c)
}

/// Exposure of a crown standing in forest cover `cover` (0 open .. 1 closed stand).
fn exposure(cover: f32) -> f32 {
    1.1 - 0.45 * cover.clamp(0.0, 1.0)
}

/// Exposure of a whole stand seen as one source: mostly interior crowns.
const STAND_EXPOSURE: f32 = 0.7;

fn water(preset: u8, length: f32, variation: [f32; 3]) -> ModelConfig {
    let name = Water::PRESETS.get(preset as usize).copied().unwrap_or("brook_pebbles");
    let mut c = Water::preset(name).unwrap_or_else(Water::brook_pebbles);
    c.length_m = length.clamp(0.1, 100.0);
    ModelConfig::Water(vary_water(&c, variation))
}

fn rain(ground: u32) -> ModelConfig {
    let name = match ground {
        2 => "on_water",
        1 => "meadow",
        _ => "under_canopy",
    };
    let mut c = Rain::preset(name).unwrap_or_default();
    c.area_m2 = 60.0;
    ModelConfig::Rain(c)
}

impl Flight {
    /// Builds the landscape around the start and every pooled generator. Allocates: call off
    /// the audio thread.
    pub fn new(sample_rate: f32, seed: u64) -> Self {
        let sr = sample_rate;
        let mut land = Land::new(seed, 0);
        land.stream(START_S, 0.35, usize::MAX);
        let mut rng = Rng::new(seed ^ 0xF1_1647);
        let mut entities = Vec::with_capacity(EMITTERS);
        let mut emitters = Vec::with_capacity(EMITTERS);
        let mut slots = Vec::with_capacity(EMITTERS);
        for (pool, n, _) in POOLS {
            for j in 0..n {
                let index = emitters.len();
                let species = if pool == Pool::Bird { BIRD_SLOTS[j % BIRD_SLOTS.len()] } else { Species::Chaffinch };
                let (kind, cfg): (SourceKind, Option<ModelConfig>) = match pool {
                    Pool::Near | Pool::Cell => (SourceKind::Wind, Some(foliage(crate::scene::TreeSpecies::Birch, 10.0, 1.0))),
                    Pool::Water => (SourceKind::Water, Some(water(3, 10.0, [1.0, 1.0, 0.0]))),
                    Pool::Insects => (SourceKind::Insect, Insects::preset("field_cricket").map(ModelConfig::Insects)),
                    Pool::Fire => (SourceKind::Fire, Fire::preset("campfire").map(ModelConfig::Fire)),
                    Pool::Rain => (SourceKind::Rain, Some(rain(0))),
                    Pool::Bird => (SourceKind::Bird, None),
                };
                let generator: Box<dyn crate::sources::Generator> = match cfg {
                    Some(c) => c.build(sr, rng.next_u64()),
                    None => {
                        let def = species.def();
                        let pitch = Voice::default_pitch(def.voice) * rng.range(0.93, 1.07);
                        let procedural = Voice::new(sr, rng.next_u64(), def.voice, pitch, 1.0);
                        let voice = crate::director::bird_voice(species, procedural, sr, rng.next_u64());
                        Box::new(Flyer::new(voice, Wings::new(sr, rng.next_u64(), def.wings), 1.0))
                    }
                };
                let (okind, geometry) = match pool {
                    Pool::Bird => (ObjectKind::Bird, Geometry::Point),
                    _ => (ObjectKind::Source, Geometry::Source { model: model_of(kind), path: None }),
                };
                entities.push(Entity { id: index as u32, kind: okind, position: Vec3::default(), geometry, flags: 0, emitters: index..index + 1 });
                emitters.push(Emitter {
                    owner: index,
                    kind,
                    offset: Vec3::default(),
                    position: Vec3::new(0.0, 0.0, -1000.0),
                    generator,
                    controls: Controls::default(),
                    flow: 1.0,
                    base: cfg,
                    variation: [1.0, 1.0, 0.0],
                    level: 0.0,
                    rel: Vec3::default(),
                    class: match cfg {
                        Some(c) => SoundClass::of_model(c.kind()),
                        None => SoundClass::Bird(species),
                    },
                });
                let target = if pool == Pool::Rain { Target::Rain(j as u8) } else { Target::None };
                slots.push(Slot { pool, target, want: pool == Pool::Rain, fade: 0.0, gain: 1.0, ref_area: 1.0, extent: 5.0, species, vel: Vec3::default() });
            }
        }
        debug_assert_eq!(emitters.len(), EMITTERS);
        let mut acoustics = Acoustics::new(sr, EMITTERS);
        acoustics.set_trees(Some(&[]));
        let pose = Pose { s: START_S, height: controls::flight_altitude(6.0), ..Default::default() };
        let mut f = Self {
            sample_rate: sr,
            land,
            world: World { entities, emitters, ground_z: -pose.height },
            acoustics,
            slots: slots.into_boxed_slice(),
            pose,
            altitude: pose.height,
            curviness: 0.35,
            density: 1.0,
            seed,
            seed_ctl: 0,
            biome_ctl: 0,
            regen: 0.0,
            blocks: 0,
            airflow: Airflow::new(seed),
            bus: [[0.0; BLOCK]; CHANNELS],
            bed: [[0.0; BLOCK]; crate::bed::CHANNELS],
            scratch: [0.0; BLOCK],
            left: [0.0; BLOCK],
            right: [0.0; BLOCK],
            trunks: vec![Trunk { x: 0.0, y: 0.0, radius: 0.0, height: 0.0 }; 128].into_boxed_slice(),
            obstacles: vec![Obstacle::Sphere { center: Vec3::default(), radius: 0.0 }; 64].into_boxed_slice(),
            voiced: [0.0; MAX_CHUNKS * BANDS],
            env_key: (u32::MAX, -1.0, -1.0),
            refit_s: f64::NEG_INFINITY,
            ctl: None,
            dirty: true,
            birds_level: 0.6,
        };
        f.locate();
        f
    }

    /// Starts already moving at `speed` (a scene that begins in the air).
    pub fn cruise(&mut self, speed: f32) {
        self.pose.speed = speed.clamp(0.0, 30.0);
        self.altitude = controls::flight_altitude(self.pose.speed);
        self.pose.height = self.altitude;
    }

    pub fn pose(&self) -> Pose {
        self.pose
    }

    /// Slot range of a pool.
    fn range(pool: Pool) -> core::ops::Range<usize> {
        let mut start = 0;
        for (p, n, _) in POOLS {
            if p == pool {
                return start..start + n;
            }
            start += n;
        }
        0..0
    }

    fn voiced_max(pool: Pool) -> usize {
        POOLS.iter().find(|x| x.0 == pool).map_or(0, |x| x.2)
    }

    /// Number of slots currently sounding (fading in, on, or fading out).
    pub fn active_emitters(&self) -> usize {
        self.slots.iter().filter(|s| s.target != Target::None && (s.want || s.fade > 0.0)).count()
    }

    fn locate(&mut self) {
        let p = self.land.path.at(self.pose.s);
        self.pose.x = p.x;
        self.pose.y = p.y;
        self.pose.heading = p.heading;
        (self.pose.sin, self.pose.cos) = p.heading.sin_cos();
    }

    /// Listener-relative position of a world point `z` metres above the ground.
    #[inline]
    fn rel(&self, x: f64, y: f64, z: f32) -> Vec3 {
        let (s, c) = (self.pose.sin, self.pose.cos);
        let (dx, dy) = (x - self.pose.x, y - self.pose.y);
        Vec3::new((c * dx + s * dy) as f32, (-s * dx + c * dy) as f32, z - self.pose.height)
    }

    /// Approximate path-frame (s, u) of a world point near the listener.
    fn land_frame(&self, x: f64, y: f64) -> (f64, f32) {
        let r = self.rel(x, y, 0.0);
        (self.pose.s + r.x as f64, r.y)
    }

    /// Velocity in the listener frame of a point at `rel` moving with `own` (world frame).
    #[inline]
    fn rel_vel(&self, rel: Vec3, own: [f32; 3]) -> Vec3 {
        let (s, c) = (self.pose.sin as f32, self.pose.cos as f32);
        let p = &self.pose;
        Vec3::new(
            -p.speed + p.turn * rel.y + c * own[0] + s * own[1],
            -p.turn * rel.x - s * own[0] + c * own[1],
            -p.climb + own[2],
        )
    }

    // -----------------------------------------------------------------------------------
    // Audio thread
    // -----------------------------------------------------------------------------------

    /// Renders one block of the flight and mixes it into `out_l/out_r` (which hold the scene's
    /// output): scene × cos, flight × sin of the cross-fade `mix` (0..1). Audio thread.
    /// `bed`: the engine is rendering a 7.1.4 bed (`Engine::render_bed`) instead of binaural:
    /// the flight is decoded into it too (head-locked with Some(head), world-locked with None).
    pub fn process(
        &mut self,
        inp: &Inputs,
        mix: f32,
        out_l: &mut [f32],
        out_r: &mut [f32],
        mut bed: Option<(&mut [[f32; BLOCK]; crate::bed::CHANNELS], Option<&Mat3>)>,
        peaks: &mut [f32; SCOPE_TRACKS],
        mixer: &mut Mix,
    ) {
        let len = out_l.len().min(out_r.len()).min(BLOCK);
        let sr = self.sample_rate;
        let dt = len as f32 / sr;
        self.blocks += 1;

        self.birds_level = inp.birds;
        self.controls(inp, dt);
        self.advance(dt);
        self.step_birds(dt, inp);
        if self.blocks % LOD_EVERY == 1 || self.dirty {
            self.lod(inp);
            self.geometry();
        }
        if self.dirty {
            controls::apply(&mut self.world, inp.st, inp.bases);
            self.dirty = false;
        }
        self.environment(inp);

        for ch in self.bus.iter_mut() {
            ch[..len].fill(0.0);
        }
        let inv_len = 1.0 / len as f32;
        let fade_step = dt / FADE_S;
        let bird_density = (inp.birds / 0.6).clamp(0.0, 1.67);
        for idx in 0..self.slots.len() {
            // Fades and target bookkeeping.
            let mut slot = self.slots[idx];
            slot.fade = if slot.want { (slot.fade + fade_step).min(1.0) } else { (slot.fade - fade_step).max(0.0) };
            let place = self.place(&slot);
            match place {
                None => {
                    slot.target = Target::None;
                    slot.want = false;
                    slot.fade = 0.0;
                }
                Some(_) if !slot.want && slot.fade == 0.0 => slot.target = Target::None,
                _ => {}
            }
            if slot.target == Target::None {
                self.slots[idx] = slot;
                continue;
            }
            let (rel, vel, flying) = place.unwrap_or_default();
            slot.vel = vel;
            self.slots[idx] = slot;

            let ent = &mut self.world.entities[idx];
            ent.position = rel;
            ent.flags = if flying { object_flags::MOVING } else { 0 };
            let em = &mut self.world.emitters[idx];
            em.position = rel;
            em.controls.env = inp.ambient;
            if let Target::Bird { idx: b, .. } = slot.target {
                let bird = &self.land.birds[b as usize];
                let (activity, calling, flap) = bird_voice(bird, inp, bird_density);
                em.controls.activity = activity;
                em.controls.calling = calling;
                em.controls.flap = flap;
            }
            let tmp = &mut self.scratch[..len];
            let mut c = em.controls;
            c.water = inp.water * em.flow;
            c.neural = inp.neural;
            c.legacy_levels = mixer.bypass;
            c.gust = (inp.wind_slider + (inp.wind.local(rel.y) - inp.wind_slider) * inp.gustiness).max(0.0);
            // Sound director: calibrated level, role, masking EQ, phrase permits.
            let view = EmitterView {
                key: target_key(&slot.target),
                class: em.class,
                kind: em.kind,
                rel,
                wants: c.activity > 0.02,
                busy: em.generator.busy(),
                flying,
            };
            let voice = mixer.voice(MIX_SLOT0 + idx, &view);
            let (gain, eq_db, send) = (voice.gain, *voice.eq_db, voice.send);
            if !voice.permit {
                c.activity = 0.0;
            }
            em.generator.render(tmp, &c);
            let energy: f32 = tmp.iter().map(|v| v * v).sum();
            em.level = em.level * 0.8 + (energy * inv_len).sqrt() * 0.2 * slot.fade * slot.gain;
            let peak = tmp.iter().fold(0.0f32, |m, v| m.max(v.abs())) * gain * slot.fade * slot.gain;
            let track = scope_track(em.kind);
            peaks[track] = peaks[track].max(peak * mix);
            if flying {
                peaks[scope::FLIGHT] = peaks[scope::FLIGHT].max(peak * mix);
            }
            em.rel = inp.head.apply_transposed(rel);
            // Extended source of radius a seen from d: intensity ~ 1/(d² + a²), not 1/d².
            let d = rel.length();
            let spread = d / (d * d + slot.extent * slot.extent).sqrt().max(1e-3);
            let g = gain * slot.fade * slot.gain * spread;
            mixer.feed(MIX_SLOT0 + idx, tmp, g * (mix * core::f32::consts::FRAC_PI_2).sin());
            let mut src = Source::at(rel, g);
            src.eq_db = eq_db;
            src.send = send;
            src.velocity = Some(vel);
            if em.kind == SourceKind::Water {
                src.ground = Some(GroundType::Water);
            }
            self.acoustics.propagate(idx, &src, tmp, &mut self.bus);
        }
        match bed.as_mut() {
            None => self.acoustics.render(&mut self.bus, &inp.head, len, &mut self.left[..len], &mut self.right[..len]),
            Some((_, head)) => {
                self.acoustics.render_bed(&mut self.bus, *head, len, &mut self.bed);
                self.left[..len].copy_from_slice(&self.bed[0][..len]);
                self.right[..len].copy_from_slice(&self.bed[1][..len]);
            }
        }

        // Wind noise of motion, at the ears.
        let wind_ms = inp.ambient.wind_ms;
        let airspeed = (self.pose.speed * self.pose.speed + 0.16 * wind_ms * wind_ms).sqrt();
        self.airflow.render(airspeed, inp.st.get(id::FLIGHT_AIRFLOW), inp.head_yaw, sr, &mut self.left[..len], &mut self.right[..len]);

        let (gs, gf) = ((mix * core::f32::consts::FRAC_PI_2).cos(), (mix * core::f32::consts::FRAC_PI_2).sin());
        for i in 0..len {
            out_l[i] = out_l[i] * gs + self.left[i] * gf;
            out_r[i] = out_r[i] * gs + self.right[i] * gf;
        }
        if let Some((out, _)) = bed {
            for (o, f) in out.iter_mut().zip(self.bed.iter()) {
                for i in 0..len {
                    o[i] = o[i] * gs + f[i] * gf;
                }
            }
            // The wind noise happens at the ears: on the front pair.
            out[0][..len].copy_from_slice(&out_l[..len]);
            out[1][..len].copy_from_slice(&out_r[..len]);
        }
    }

    /// Reads the FLIGHT controls; seed / biome changes fade everything out and rebuild.
    fn controls(&mut self, inp: &Inputs, dt: f32) {
        let st = inp.st;
        if self.ctl.as_ref() != Some(st) {
            self.ctl = Some(*st);
            self.dirty = true;
        }
        let target = st.get(id::FLIGHT_SPEED).clamp(0.0, 30.0);
        let dv = (target - self.pose.speed).clamp(-ACCEL * dt, ACCEL * dt);
        self.pose.speed += dv;
        let alt = st.get(id::FLIGHT_ALTITUDE).clamp(1.2, 40.0);
        self.altitude += (alt - self.altitude) * (1.0 - (-dt / 2.5).exp());
        self.curviness = st.get(id::FLIGHT_CURVINESS).clamp(0.0, 1.0);
        self.density = st.get(id::FLIGHT_DENSITY).clamp(0.0, 2.0);
        let seed = st.get(id::FLIGHT_SEED).round().max(0.0) as u32;
        let biome = st.get(id::FLIGHT_BIOME).round().max(0.0) as u32;
        if (seed != self.seed_ctl || biome != self.biome_ctl) && self.regen == 0.0 {
            self.regen = REGEN_S;
            for s in self.slots.iter_mut() {
                if s.pool != Pool::Rain {
                    s.want = false;
                }
            }
        }
        if self.regen > 0.0 {
            self.regen -= dt;
            if self.regen <= 0.0 {
                self.regen = 0.0;
                self.seed_ctl = seed;
                self.biome_ctl = biome;
                for s in self.slots.iter_mut() {
                    if s.pool != Pool::Rain {
                        *s = Slot { target: Target::None, want: false, fade: 0.0, ..*s };
                    }
                }
                let seed = self.seed ^ (seed as u64).wrapping_mul(0x9E37_79B9);
                self.land.reset(seed, biome, self.pose.s);
                self.land.stream(self.pose.s, self.curviness, usize::MAX);
                self.dirty = true;
            }
        }
    }

    /// Moves the listener along the path.
    fn advance(&mut self, dt: f32) {
        self.pose.s += (self.pose.speed * dt) as f64;
        if self.regen == 0.0 {
            self.land.stream(self.pose.s, self.curviness, 1);
        }
        self.locate();
        self.pose.turn = (self.land.path.curvature(self.pose.s) * self.pose.speed as f64) as f32;
        // Gentle swells of the flight level above walking height (none when walking).
        let swell = (self.altitude - 2.5).clamp(0.0, 12.0) * 0.12;
        let n = land::noise2(self.land.seed, 0xA17, self.pose.s / 160.0, 0.5) - 0.5;
        let h = (self.altitude + 2.0 * swell * n).max(1.0);
        self.pose.climb = if dt > 0.0 { (h - self.pose.height) / dt } else { 0.0 };
        self.pose.height = h;
        self.world.ground_z = -h;
    }

    /// Birds: perched singing, flushed by the passing listener, flying off to a new perch.
    fn step_birds(&mut self, dt: f32, inp: &Inputs) {
        let pose = self.pose;
        let (hs, hc) = (pose.sin, pose.cos);
        let speed = pose.speed;
        let seed = self.land.seed;
        let threshold = 0.5 * (inp.birds / 0.6).clamp(0.0, 1.67);
        for i in 0..self.land.birds.len() {
            let b = self.land.birds[i];
            if !b.alive {
                continue;
            }
            let mut b = b;
            b.alarm = (b.alarm - dt).max(0.0);
            match b.state {
                BirdState::Flying => {
                    b.t += dt;
                    let u = ease((b.t / b.dur).min(1.0));
                    let p = bezier(b.from, b.c1, b.c2, b.to, u);
                    let inv = if dt > 0.0 { 1.0 / dt } else { 0.0 };
                    b.vel = [((p[0] - b.pos[0]) as f32) * inv, ((p[1] - b.pos[1]) as f32) * inv, ((p[2] - b.pos[2]) as f32) * inv];
                    b.pos = p;
                    if b.t >= b.dur {
                        b.state = BirdState::Perched;
                        b.vel = [0.0; 3];
                    }
                }
                BirdState::Perched if !b.checked && b.rank < threshold => {
                    let (dx, dy) = (b.pos[0] - pose.x, b.pos[1] - pose.y);
                    let ahead = hc * dx + hs * dy;
                    let side = -hs * dx + hc * dy;
                    let dz = b.pos[2] - pose.height as f64;
                    let d = (dx * dx + dy * dy + dz * dz).sqrt() as f32;
                    let flush_at = 6.0 + 0.9 * speed;
                    if d < flush_at && ahead > -4.0 {
                        b.checked = true;
                        let mut r = land::Stream::new(land::hash(seed, b.id as i64, 0, 0xF1A5));
                        let shy = match b.species() {
                            Species::WoodPigeon => 0.85,
                            Species::TawnyOwl => 0.6,
                            Species::SongThrush | Species::Cuckoo => 0.5,
                            Species::Chaffinch => 0.4,
                            Species::Robin => 0.25,
                            _ => 0.2,
                        };
                        let p = shy * (0.3 + 0.7 * (speed / 8.0).min(1.0));
                        if speed > 0.5 && r.uniform() < p {
                            // Away from the path, a little forward or back, up and down to a
                            // new perch 25..55 m off.
                            let lat = if side >= 0.0 { 1.0 } else { -1.0 };
                            let fwd = r.range(-0.7, 0.9) as f64;
                            let (nx, ny) = (-hs * lat, hc * lat);
                            let (mut ex, mut ey) = (nx + fwd * hc, ny + fwd * hs);
                            let el = (ex * ex + ey * ey).sqrt().max(1e-6);
                            ex /= el;
                            ey /= el;
                            let dist = r.range(25.0, 55.0) as f64;
                            let def = b.species().def();
                            let v = r.range(def.speed[0], def.speed[1]);
                            let to = [b.pos[0] + ex * dist, b.pos[1] + ey * dist, r.range(3.0, 9.0) as f64];
                            let up = r.range(3.0, 7.0) as f64;
                            b.from = b.pos;
                            b.c1 = [b.pos[0] + ex * 0.3 * dist, b.pos[1] + ey * 0.3 * dist, b.pos[2] + up];
                            b.c2 = [to[0] - ex * 0.3 * dist, to[1] - ey * 0.3 * dist, to[2] + 0.5 * up];
                            b.to = to;
                            b.t = 0.0;
                            b.dur = (1.15 * dist as f32 / v).max(1.5);
                            b.state = BirdState::Flying;
                            b.alarm = 8.0;
                        }
                    }
                }
                _ => {}
            }
            self.land.birds[i] = b;
        }
    }

    /// Listener-relative position, velocity and "flying" of a slot's target; None if gone.
    fn place(&self, slot: &Slot) -> Option<(Vec3, Vec3, bool)> {
        match slot.target {
            Target::None => Some((Vec3::default(), Vec3::default(), false)),
            Target::Tree { abs, id } => {
                if abs < self.land.tree_head || abs >= self.land.tree_tail {
                    return None;
                }
                let t = self.land.tree(abs);
                (t.id == id).then(|| {
                    let r = self.rel(t.x, t.y, t.crown_z);
                    (r, self.rel_vel(r, [0.0; 3]), false)
                })
            }
            Target::Cell { k, band } => {
                if k < self.land.chunk_head || k >= self.land.chunk_tail {
                    return None;
                }
                let c = &self.land.chunk(k).cells[band as usize];
                let r = self.rel(c.x, c.y, c.canopy_z);
                Some((r, self.rel_vel(r, [0.0; 3]), false))
            }
            Target::Bird { idx, id } => {
                let b = &self.land.birds[idx as usize];
                (b.alive && b.id == id).then(|| {
                    let r = self.rel(b.pos[0], b.pos[1], b.pos[2] as f32);
                    (r, self.rel_vel(r, b.vel), b.state == BirdState::Flying)
                })
            }
            Target::Site { idx, id } => {
                let x = &self.land.sites[idx as usize];
                (x.alive && x.id == id).then(|| {
                    let r = self.rel(x.x, x.y, x.z);
                    (r, self.rel_vel(r, [0.0; 3]), false)
                })
            }
            Target::Rain(k) => {
                let (sx, sy) = (if k & 1 == 0 { 7.0 } else { -7.0 }, if k & 2 == 0 { 7.0 } else { -7.0 });
                Some((Vec3::new(sx, sy, -self.pose.height), Vec3::new(0.0, 0.0, -self.pose.climb), false))
            }
        }
    }

    /// Chooses which streamed things the pools voice.
    fn lod(&mut self, inp: &Inputs) {
        let density = self.density;
        let tree_rank = 0.5 * density;
        let s = self.pose.s;
        let regen = self.regen > 0.0;

        // Nearest trees.
        let mut near = Best::<24>::new();
        let k0 = ((s - 45.0) / CHUNK).floor() as i64;
        let k1 = ((s + 45.0) / CHUNK).floor() as i64;
        for k in k0.max(self.land.chunk_head)..=k1.min(self.land.chunk_tail - 1) {
            let (a, b) = self.land.chunk(k).trees;
            for abs in a.max(self.land.tree_head)..b {
                let t = self.land.tree(abs);
                if t.rank >= tree_rank {
                    continue;
                }
                let r = self.rel(t.x, t.y, t.crown_z);
                let d = r.length();
                if d < 40.0 {
                    near.push(1.0 / d.max(0.5), Target::Tree { abs, id: t.id });
                }
            }
        }
        if !regen {
            self.assign(Pool::Near, &near);
        }

        // Crown area now voiced by near trees, per cell.
        self.voiced = [0.0; MAX_CHUNKS * BANDS];
        for i in Self::range(Pool::Near) {
            if let Target::Tree { abs, .. } = self.slots[i].target {
                let t = *self.land.tree(abs);
                let k = ((self.land.chunk_head..self.land.chunk_tail).find(|&k| {
                    let c = self.land.chunk(k);
                    abs >= c.trees.0 && abs < c.trees.1
                }))
                .unwrap_or(self.land.chunk_head);
                let cell = k.rem_euclid(MAX_CHUNKS as i64) as usize * BANDS + t.band as usize;
                self.voiced[cell] += t.crown_area() * self.slots[i].fade;
            }
        }

        // Aggregate cells.
        let mut cells = Best::<16>::new();
        for k in self.land.chunk_head..self.land.chunk_tail {
            let ch = self.land.chunk(k);
            for (band, c) in ch.cells.iter().enumerate() {
                let area = c.area_at(density);
                if area < 15.0 {
                    continue;
                }
                let d = self.rel(c.x, c.y, c.canopy_z).length();
                if d < MAX_RANGE {
                    cells.push(area.sqrt() / d.max(5.0), Target::Cell { k, band: band as u8 });
                }
            }
        }
        if !regen {
            self.assign(Pool::Cell, &cells);
        }
        for i in Self::range(Pool::Cell) {
            if let Target::Cell { k, band } = self.slots[i].target {
                if k >= self.land.chunk_head && k < self.land.chunk_tail {
                    let area = self.land.chunk(k).cells[band as usize].area_at(density);
                    let left = (area - self.voiced[k.rem_euclid(MAX_CHUNKS as i64) as usize * BANDS + band as usize]).max(0.0);
                    self.slots[i].gain = (left / self.slots[i].ref_area.max(1.0)).sqrt().min(1.5);
                }
            }
        }

        // Birds.
        let threshold = 0.5 * (inp.birds / 0.6).clamp(0.0, 1.67);
        let mut birds = Best::<16>::new();
        for (i, b) in self.land.birds.iter().enumerate() {
            if !b.alive || b.rank >= threshold {
                continue;
            }
            let d = self.rel(b.pos[0], b.pos[1], b.pos[2] as f32).length();
            if d < 150.0 {
                let song = b.species().song_activity(inp.hour).max(0.05);
                let boost = if b.state == BirdState::Flying { 4.0 } else { 1.0 };
                birds.push(song * boost / d.max(2.0), Target::Bird { idx: i as u16, id: b.id });
            }
        }
        if !regen {
            self.assign(Pool::Bird, &birds);
        }

        // Water, insects, fire.
        let mut water = Best::<16>::new();
        let mut insects = Best::<8>::new();
        let mut fire = Best::<4>::new();
        for (i, x) in self.land.sites.iter().enumerate() {
            if !x.alive {
                continue;
            }
            let d = self.rel(x.x, x.y, x.z).length();
            let t = Target::Site { idx: i as u16, id: x.id };
            match x.kind {
                SiteKind::Water if d < MAX_RANGE => water.push(x.weight / d.max(2.0), t),
                SiteKind::Insects if d < 90.0 => insects.push(x.weight / d.max(2.0), t),
                SiteKind::Fire if d < 150.0 => fire.push(x.weight / d.max(2.0), t),
                _ => {}
            }
        }
        if !regen {
            self.assign(Pool::Water, &water);
            self.assign(Pool::Insects, &insects);
            self.assign(Pool::Fire, &fire);
        }

        // Rain patches follow the listener; their surface follows the ground below.
        let ground = self.land.ground_at(s, 0.0);
        for i in Self::range(Pool::Rain) {
            let cfg = rain(ground);
            if self.world.emitters[i].base != Some(cfg) {
                self.world.emitters[i].base = Some(cfg);
                self.dirty = true;
            }
        }
    }

    /// Keeps the best candidates voiced (with hysteresis), releases the rest.
    fn assign<const N: usize>(&mut self, pool: Pool, best: &Best<N>) {
        let range = Self::range(pool);
        let voiced = Self::voiced_max(pool);
        let keep = if best.n >= voiced { best.items[voiced - 1].0 * 0.6 } else { 0.0 };
        let mut active = 0;
        for i in range.clone() {
            let s = &mut self.slots[i];
            if s.want {
                match best.find(s.target) {
                    Some(p) if p >= keep => active += 1,
                    _ => s.want = false,
                }
            }
        }
        for &(_, t) in &best.items[..best.n.min(voiced)] {
            if active >= voiced {
                break;
            }
            if let Some(i) = range.clone().find(|&i| self.slots[i].target == t) {
                if !self.slots[i].want {
                    self.slots[i].want = true;
                    active += 1;
                }
                continue;
            }
            let free = |i: &usize| self.slots[*i].target == Target::None;
            let slot = match t {
                Target::Bird { idx, .. } => {
                    let sp = self.land.birds[idx as usize].species();
                    range.clone().filter(free).find(|&i| self.slots[i].species == sp).or_else(|| range.clone().find(free))
                }
                _ => range.clone().find(free),
            };
            let Some(i) = slot else { break };
            self.configure(i, t);
            active += 1;
        }
    }

    /// Points slot `i` at `t` and sets its model configuration.
    fn configure(&mut self, i: usize, t: Target) {
        let s = &mut self.slots[i];
        s.target = t;
        s.want = true;
        s.fade = 0.0;
        s.gain = 1.0;
        s.extent = 0.0;
        let cfg = match t {
            Target::Tree { abs, .. } => {
                let tr = *self.land.tree(abs);
                s.extent = tr.crown_r;
                let p = self.land_frame(tr.x, tr.y);
                Some(foliage(tr.species(), tr.crown_area(), exposure(self.land.base_cover(p.0, p.1))))
            }
            Target::Cell { k, band } => {
                let c = self.land.chunk(k).cells[band as usize];
                let area = c.area_at(self.density).max(1.0);
                s.ref_area = area;
                s.extent = 0.6 * (area / core::f32::consts::PI).sqrt();
                Some(foliage(land::SPECIES[c.species as usize], area, STAND_EXPOSURE))
            }
            Target::Site { idx, .. } => {
                let x = self.land.sites[idx as usize];
                s.extent = 0.5 * x.length;
                match x.kind {
                    SiteKind::Water => Some(water(x.preset, x.length, x.variation)),
                    SiteKind::Insects => Insects::preset(Insects::PRESETS.get(x.preset as usize).copied().unwrap_or("field_cricket")).map(ModelConfig::Insects),
                    SiteKind::Fire => Fire::preset("campfire").map(ModelConfig::Fire),
                }
            }
            Target::Bird { idx, .. } => {
                // A bird takes the voice of the slot it got.
                let sp = s.species;
                if let Some(k) = land::BIRD_SPECIES.iter().position(|x| *x == sp) {
                    self.land.birds[idx as usize].species = k as u8;
                }
                None
            }
            Target::Rain(_) | Target::None => None,
        };
        if cfg.is_some() {
            self.world.emitters[i].base = cfg;
            self.dirty = true;
        }
    }

    /// Feeds trunks, boulders and ridges near the listener to the propagation (real-time
    /// safe: fixed buffers, no environment reset).
    fn geometry(&mut self) {
        let s = self.pose.s;
        let tree_rank = 0.5 * self.density;
        let mut n = 0;
        let k0 = ((s - 35.0) / CHUNK).floor() as i64;
        let k1 = ((s + 35.0) / CHUNK).floor() as i64;
        let mut radius = 30.0f32;
        'scan: for pass in 0..2 {
            n = 0;
            for k in k0.max(self.land.chunk_head)..=k1.min(self.land.chunk_tail - 1) {
                let (a, b) = self.land.chunk(k).trees;
                for abs in a.max(self.land.tree_head)..b {
                    let t = *self.land.tree(abs);
                    if t.rank >= tree_rank {
                        continue;
                    }
                    let r = self.rel(t.x, t.y, 0.0);
                    if r.x * r.x + r.y * r.y < radius * radius {
                        if n == self.trunks.len() {
                            if pass == 0 {
                                radius = 20.0;
                                continue 'scan;
                            }
                            break 'scan;
                        }
                        self.trunks[n] = Trunk { x: r.x, y: r.y, radius: t.trunk_r, height: t.top() };
                        n += 1;
                    }
                }
            }
            break;
        }
        let trunks = n;
        let mut m = 0;
        let h = self.pose.height;
        for f in self.land.features.iter() {
            if !f.alive || m >= self.obstacles.len() {
                continue;
            }
            match f.kind {
                FeatureKind::Rock => {
                    let r = self.rel(f.x, f.y, 0.7 * f.size);
                    if r.length() < 80.0 {
                        self.obstacles[m] = Obstacle::Sphere { center: r, radius: f.size };
                        m += 1;
                    }
                }
                FeatureKind::Ridge => {
                    for w in f.path[..f.path_len as usize].windows(2) {
                        if m >= self.obstacles.len() {
                            break;
                        }
                        let a = self.rel(w[0][0], w[0][1], 0.0);
                        let b = self.rel(w[1][0], w[1][1], 0.0);
                        if a.length().min(b.length()) < 150.0 {
                            self.obstacles[m] = Obstacle::Wall { x0: a.x, y0: a.y, x1: b.x, y1: b.y, z0: -h - 1.0, z1: -h + f.z };
                            m += 1;
                        }
                    }
                }
                _ => {}
            }
        }
        // The trunks move with the emitters (both are rigid in the world), and each
        // propagator refits its spectra when its own path moved enough (0.15 m when
        // obstructed, 1 m otherwise), so a refit of everything is forced only when the
        // landscape was rebuilt or the listener stood still while it changed.
        let refit = self.dirty || ((s - self.refit_s).abs() > 4.0);
        if refit {
            self.refit_s = s;
        }
        self.acoustics.update_geometry(&self.trunks[..trunks], &self.obstacles[..m], refit);
    }

    /// Ground, stand density and ear height for the propagation (quantised, so the forest
    /// tail and ground model are rebuilt only when they really change).
    fn environment(&mut self, inp: &Inputs) {
        let ground = self.land.ground_at(self.pose.s, 0.0);
        let b = self.land.biome_def();
        let local = b.density_ha * self.density * self.land.cover(self.pose.s, 0.0).max(self.land.cover(self.pose.s, 15.0));
        let q = (local / 50.0).round() * 50.0;
        let ear = self.pose.height;
        let (g0, d0, e0) = self.env_key;
        let ear_q = if (ear - e0).abs() > (0.04 * e0).max(0.05) { ear } else { e0 };
        let dens = if (q - d0).abs() >= 50.0 { q } else { d0 };
        self.env_key = (ground, dens, ear_q);
        let _ = g0;
        let mut env = inp.acoustic;
        env.ground = match ground {
            2 => GroundType::Water,
            1 => GroundType::Grass,
            _ => GroundType::ForestFloor,
        };
        env.tree_density_per_ha = dens.max(0.0);
        env.ear_height = ear_q;
        self.acoustics.set_environment(&env);
    }

    // -----------------------------------------------------------------------------------
    // Telemetry
    // -----------------------------------------------------------------------------------

    /// Source telemetry: (emitter, its fade) for every sounding slot.
    pub fn sounding(&self) -> impl Iterator<Item = &Emitter> {
        self.world.emitters.iter().zip(self.slots.iter()).filter(|(_, s)| s.target != Target::None && s.fade > 0.0).map(|(e, _)| e)
    }

    /// Publishes the streamed objects for the UI (audio thread).
    pub fn publish(&self, bridge: &FlightBridge) {
        let mut snap = bridge.begin();
        let h = self.pose.height;
        let ground = -h;
        // Features first: they are few and big.
        for f in self.land.features.iter() {
            if !f.alive {
                continue;
            }
            let r = self.rel(f.x, f.y, 0.0);
            if r.x * r.x + r.y * r.y > 260.0 * 260.0 {
                continue;
            }
            let (kind, z) = match f.kind {
                FeatureKind::River => (ObjectKind::River, ground),
                FeatureKind::Lake => (ObjectKind::Lake, ground),
                FeatureKind::Ridge | FeatureKind::Rock => (ObjectKind::Rock, ground + f.z),
                FeatureKind::Waterfall => (ObjectKind::Source, ground + f.z),
                FeatureKind::Campfire | FeatureKind::Meadow => (ObjectKind::Source, ground),
            };
            snap.object(ObjectInfo { id: f.id, kind: kind as u32, x: r.x, y: r.y, z, flags: 0 });
            if f.path_len > 0 {
                let pts = f.path[..f.path_len as usize].iter().map(|p| self.rel(p[0], p[1], 0.0));
                snap.path(f.id, pts);
            }
        }
        // Birds that exist at the current BIRDS level.
        let threshold = 0.5 * (self.birds_level / 0.6).clamp(0.0, 1.67);
        for b in self.land.birds.iter() {
            if !b.alive || b.rank >= threshold {
                continue;
            }
            let r = self.rel(b.pos[0], b.pos[1], b.pos[2] as f32);
            if r.length() < 160.0 {
                let flags = if b.state == BirdState::Flying { object_flags::MOVING } else { 0 };
                snap.object(ObjectInfo { id: b.id, kind: ObjectKind::Bird as u32, x: r.x, y: r.y, z: r.z, flags });
            }
        }
        self.publish_trees(&mut snap, ground);
    }


    /// The nearest trees that fit in the snapshot (radius from a distance histogram).
    fn publish_trees(&self, snap: &mut Snapshot, ground: f32) {
        let room = bridge::OBJECTS.saturating_sub(snap.len());
        if room == 0 {
            return;
        }
        const BIN: f32 = 5.0;
        const BINS: usize = 20;
        let mut hist = [0usize; BINS];
        let s = self.pose.s;
        let tree_rank = 0.5 * self.density;
        let k0 = ((s - BIN as f64 * BINS as f64) / CHUNK).floor() as i64;
        let k1 = ((s + BIN as f64 * BINS as f64) / CHUNK).floor() as i64;
        let ks = k0.max(self.land.chunk_head)..=k1.min(self.land.chunk_tail - 1);
        for k in ks.clone() {
            let (a, b) = self.land.chunk(k).trees;
            for abs in a.max(self.land.tree_head)..b {
                let t = self.land.tree(abs);
                if t.rank < tree_rank {
                    let r = self.rel(t.x, t.y, 0.0);
                    let d = (r.x * r.x + r.y * r.y).sqrt();
                    if d < BIN * BINS as f32 {
                        hist[(d / BIN) as usize] += 1;
                    }
                }
            }
        }
        let mut total = 0;
        let mut bins = 0;
        while bins < BINS && total + hist[bins] <= room {
            total += hist[bins];
            bins += 1;
        }
        let radius = bins as f32 * BIN;
        for k in ks {
            let (a, b) = self.land.chunk(k).trees;
            for abs in a.max(self.land.tree_head)..b {
                let t = self.land.tree(abs);
                if t.rank < tree_rank {
                    let r = self.rel(t.x, t.y, 0.0);
                    if r.x * r.x + r.y * r.y < radius * radius {
                        snap.object(ObjectInfo { id: t.id, kind: ObjectKind::Tree as u32, x: r.x, y: r.y, z: ground, flags: 0 });
                    }
                }
            }
        }
    }
}

fn model_of(kind: SourceKind) -> ModelKind {
    match kind {
        SourceKind::Water => ModelKind::Water,
        SourceKind::Wind => ModelKind::Foliage,
        SourceKind::Rain => ModelKind::Rain,
        SourceKind::Fire => ModelKind::Fire,
        SourceKind::Insect => ModelKind::Insects,
        SourceKind::Thunder => ModelKind::Thunder,
        SourceKind::Bird => ModelKind::Wind,
    }
}

/// Song / calls / wingbeats of a streamed bird (mirrors the director's agents).
fn bird_voice(b: &land::Bird, inp: &Inputs, density: f32) -> (f32, bool, f32) {
    let sp = b.species();
    let def = sp.def();
    let rain = inp.rain.clamp(0.0, 1.0);
    match b.state {
        BirdState::Perched => {
            let song = sp.song_activity(inp.hour) * (1.0 - 0.8 * rain);
            let calm = if b.alarm > 0.0 { 0.3 } else { 1.0 };
            ((song * density * calm).min(1.0), false, 0.0)
        }
        BirdState::Flying => {
            let awake = sp.flight_activity(inp.hour) * (1.0 - 0.9 * rain);
            let activity = (def.call_rate.max(0.3) * awake.max(0.3) * density).min(1.0);
            let flap = if b.t < 0.6 {
                1.0
            } else if b.dur - b.t < 0.5 {
                0.85
            } else {
                0.5
            };
            (activity, !def.sings_in_flight, flap)
        }
    }
}

fn ease(u: f32) -> f64 {
    let u = u.clamp(0.0, 1.0) as f64;
    u * u * (3.0 - 2.0 * u)
}

fn bezier(p0: [f64; 3], p1: [f64; 3], p2: [f64; 3], p3: [f64; 3], u: f64) -> [f64; 3] {
    let v = 1.0 - u;
    let (a, b, c, d) = (v * v * v, 3.0 * v * v * u, 3.0 * v * u * u, u * u * u);
    [
        a * p0[0] + b * p1[0] + c * p2[0] + d * p3[0],
        a * p0[1] + b * p1[1] + c * p2[1] + d * p3[1],
        a * p0[2] + b * p1[2] + c * p2[2] + d * p3[2],
    ]
}

/// Flight settings in a scene description: `"flight": {"speed": 12, "biome": "taiga"}`.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FlightDesc {
    /// Start in flight mode (default true when the block is present).
    #[serde(default = "yes")]
    pub start: bool,
    /// m/s, 0..25.
    #[serde(default)]
    pub speed: Option<f32>,
    /// Ears above the ground, m (omitted = follows the speed).
    #[serde(default)]
    pub altitude: Option<f32>,
    #[serde(default)]
    pub curviness: Option<f32>,
    /// One of `land::BIOMES` by name.
    #[serde(default)]
    pub biome: Option<String>,
    #[serde(default)]
    pub seed: Option<u32>,
    #[serde(default)]
    pub density: Option<f32>,
    /// Wind noise of motion, 0..3 (1 = default).
    #[serde(default)]
    pub airflow: Option<f32>,
}

fn yes() -> bool {
    true
}

impl FlightDesc {
    /// Sets the FLIGHT controls on `engine`; returns whether to start in the air.
    pub fn apply(&self, engine: &crate::Engine) -> Result<bool, String> {
        let biome = match &self.biome {
            None => None,
            Some(name) => Some(
                land::BIOMES.iter().position(|b| b.name == name).ok_or_else(|| {
                    let names: Vec<&str> = land::BIOMES.iter().map(|b| b.name).collect();
                    format!("flight: unknown biome `{name}` (valid: {})", names.join(", "))
                })? as f32,
            ),
        };
        let set = |control: u32, v: Option<f32>| {
            if let Some(v) = v {
                engine.set_control(control, v);
            }
        };
        set(id::FLIGHT_SPEED, self.speed);
        set(id::FLIGHT_ALTITUDE, self.altitude);
        set(id::FLIGHT_CURVINESS, self.curviness);
        set(id::FLIGHT_BIOME, biome);
        set(id::FLIGHT_SEED, self.seed.map(|s| s as f32));
        set(id::FLIGHT_DENSITY, self.density);
        set(id::FLIGHT_AIRFLOW, self.airflow);
        if self.start {
            engine.set_param(crate::engine::param::FLIGHT, 1.0);
        }
        Ok(self.start)
    }
}
