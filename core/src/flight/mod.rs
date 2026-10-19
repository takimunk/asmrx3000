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

pub mod acoustic;
pub mod airflow;
pub mod bridge;
pub mod land;
pub mod terrain;
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
use crate::shaping::NF;
use crate::sources::{
    Ambient, Bird as Voice, Controls, Fire, Flyer, Foliage, Insects, ModelConfig, ModelKind, Rain, Water, Wind, Wings,
};
use crate::windfield::WindField;

use acoustic::{Echoes, PathTerrain};
use airflow::Airflow;
use bridge::{FlightBridge, FrameBridge, Snapshot};
use land::{BirdState, FeatureKind, Land, SiteKind, TILES};
use terrain::Surface;

/// Turning radius of the wandering path at curviness 1 (steering by head off), m.
const MIN_RADIUS: f64 = 250.0;
/// Head steering: gaze within this yaw / pitch of straight ahead keeps the course, rad (8°).
const DEAD_ZONE: f32 = 0.14;
/// Head steering: the gaze beyond the dead zone at which the turn rate is at its maximum,
/// rad (35°): a glance turns gently, a sustained look over the shoulder turns hard.
const FULL_TURN: f32 = 0.61;
/// Smoothing of the gaze before it steers, s: head jitter and quick glances do not wobble
/// the flight.
const GAZE_TAU: f32 = 0.35;
/// Turn-rate slew limit, rad/s²: turns build up and ease off like a glider's.
const TURN_ACCEL: f32 = 0.8;
/// Head pitch steering: climb rate at full pitch, m/s per m/s of speed (climbs at ~25°).
const CLIMB_PER_SPEED: f32 = 0.45;
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
/// Index of ~1 kHz on the propagation's fit grid (for broadband estimates of a spectral loss).
const MID_BAND: usize = 22;
/// Deceleration when easing into a stop, m/s².
const STOP_DECEL: f32 = 1.0;
/// Holding still in the world (ASMR_PARAM_FLIGHT = 2), and walking: deceleration, m/s².
/// Firm enough that a stop feels immediate, soft enough not to jolt (25 m/s → 0 in ~6 s).
const HOLD_DECEL: f32 = 4.0;
/// Walking and running pace on foot (W/S/A/D, with Shift), m/s.
const WALK: f32 = 1.5;
const RUN: f32 = 5.0;
/// Turning on foot with Q/E, rad/s (60°/s), and its slew limit, rad/s².
const KEY_TURN: f32 = 1.047;
const KEY_TURN_ACCEL: f32 = 3.0;
/// In flight, W/S change the cruise by this much per second held, m/s².
const KEY_CRUISE_RATE: f32 = 3.0;

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
    /// Wind over exposed ground: dune sand, ridge howl, blowing snow, grass on open crests.
    Air,
}

/// (pool, slots, voiced at once). Spare slots let outgoing voices fade while new ones fade in.
const POOLS: [(Pool, usize, usize); 8] = [
    (Pool::Near, 14, 10),
    (Pool::Cell, 8, 6),
    (Pool::Bird, 14, 8),
    (Pool::Water, 8, 6),
    (Pool::Insects, 4, 3),
    (Pool::Fire, 2, 1),
    (Pool::Rain, 4, 4),
    (Pool::Air, 4, 3),
];

/// Emitters in flight mode (fixed: the level-of-detail bound). At most 41 sound at once
/// (the voiced counts above); the rest are spares that fade out while others fade in.
pub const EMITTERS: usize = 58;

/// Species of the bird slots (a streamed bird takes a slot of its species when one is free).
const BIRD_SLOTS: [Species; 14] = [
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
    Species::Blackbird,
    Species::Skylark,
    Species::Gull,
    Species::Raven,
];

#[derive(Clone, Copy, PartialEq, Debug)]
enum Target {
    None,
    Tree { tile: u16, k: u16, id: u32 },
    Cell { i: i32, j: i32 },
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
        Target::Tree { tile, k, id } => 1 << 60 | (tile as u64) << 44 | (k as u64) << 28 | (id as u64 & 0x0FFF_FFFF),
        Target::Cell { i, j } => 2 << 60 | ((i as u32 as u64) << 30 & 0x0FFF_FFFF_C000_0000) | (j as u32 as u64 & 0x3FFF_FFFF),
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
    /// Head yaw relative to the direction of travel, rad (+ = left), from the predicted
    /// head pose the audio is rendered with.
    pub head_yaw: f32,
    /// Head pitch, rad (+ = looking up), same pose.
    pub head_pitch: f32,
    /// Head-pose samples are arriving (AirPods): steering by head is live only then.
    pub head_tracked: bool,
    /// Ear-centre offset of the predicted pose (neck parallax), as for the scene.
    pub listener_offset: Vec3,
    pub acoustic: Acoustic,
    pub wind: &'a WindField,
    /// WIND slider 0..1.
    pub wind_slider: f32,
    pub gustiness: f32,
    pub water: f32,
    pub neural: bool,
    /// Holding still in the world (ASMR_PARAM_FLIGHT = 2): no cruise and no stops, down to
    /// standing height; the keys walk.
    pub hold: bool,
    /// Movement keys (ASMR_PARAM_MOVE_*).
    pub moves: Moves,
}

/// Movement keys held in the app (ASMR_PARAM_MOVE_*), each −1..1.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Moves {
    /// W / S: + = forward. Standing: walk along where the head looks. Flying: cruise up / down.
    pub forward: f32,
    /// A / D: + = right. Standing: step sideways. Flying: turn.
    pub strafe: f32,
    /// Q / E: + = right. Turns the course.
    pub turn: f32,
    /// Shift: run (5 m/s) instead of walk (1.5 m/s).
    pub run: bool,
}

impl Moves {
    /// Clamped to −1..1, NaN as 0.
    pub fn new(forward: f32, strafe: f32, turn: f32, run: bool) -> Self {
        let c = |v: f32| if v.is_finite() { v.clamp(-1.0, 1.0) } else { 0.0 };
        Self { forward: c(forward), strafe: c(strafe), turn: c(turn), run }
    }
}

/// Listener pose in the world.
#[derive(Clone, Copy, Debug, Default)]
pub struct Pose {
    /// Distance travelled, m.
    pub s: f64,
    pub x: f64,
    pub y: f64,
    pub heading: f64,
    /// Ears above the ground, m.
    pub height: f32,
    /// Speed along the course, m/s (negative walking backwards).
    pub speed: f32,
    /// Sideways speed, m/s (+ = to the left of the course; stepping aside on foot).
    pub side: f32,
    /// Turn rate, rad/s (+ = turning left).
    pub turn: f32,
    /// Vertical speed, m/s.
    pub climb: f32,
    /// Ground height under the listener and height of the ears, m above sea level.
    pub ground: f32,
    pub ear: f32,
    /// Stopped at a place: 0 cruising .. 1 lingering.
    pub linger: f32,
    /// sin / cos of the heading.
    sin: f64,
    cos: f64,
}

impl Pose {
    /// Speed over the ground, m/s (along and across the course).
    pub fn ground_speed(&self) -> f32 {
        self.speed.hypot(self.side)
    }
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
    /// Crown area voiced by near-tree slots, per tile slot.
    voiced: Box<[f32]>,
    /// Smoothed gaze (yaw, pitch) relative to the direction of travel, rad.
    gaze: (f32, f32),
    /// Altitude added by looking up / down (steer by head with pitch), m.
    climb_offset: f32,
    env_key: (u32, f32, f32),
    refit_s: f64,
    ctl: Option<ControlState>,
    dirty: bool,
    birds_level: f32,
    /// Features near the listener (refreshed with the level of detail).
    near: [usize; land::LOCAL],
    n_near: usize,
    /// Block until which streaming runs with a larger budget (after a reseed).
    filled_until: u64,
    relief_ctl: f32,
    /// What the terrain does to each slot's path (refreshed with the level of detail).
    paths: Box<[PathTerrain]>,
    /// Its loss on the propagation's fit grid, dB.
    path_db: Box<[[f32; NF]]>,
    echoes: Echoes,
    route: Route,
    /// Precipitation falls as snow (snowy biome or below freezing).
    snowing: bool,
    /// Incremented whenever the landscape is regenerated (UI terrain caches key on it).
    generation: u32,
    /// Smoothed ground the flight level follows (looks ahead over rises).
    terrain_follow: f32,
}

/// Stops and slow-downs at places worth lingering (a lake shore, a waterfall, the surf, the
/// big river's bank, a viewpoint): approach, ease down, stand there a while, fly on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Visit {
    Cruise,
    Approach,
    Linger,
}

#[derive(Clone, Copy, Debug)]
struct Route {
    visit: Visit,
    /// Where to stop (world) and the site's id.
    at: (f64, f64),
    id: u32,
    /// Seconds left lingering / before looking for the next place.
    timer: f32,
    cooldown: f32,
    visited: [u32; 16],
    next_visited: usize,
    /// How far the flight level comes down to standing height (0..1, smoothed).
    down: f32,
    /// Holding still in the world (ASMR_PARAM_FLIGHT = 2) as of the last block.
    hold: bool,
    /// Cruise added by W / S in flight, m/s (reset by a new flight.speed or a stop).
    cruise_offset: f32,
    /// flight.speed the offset applies to.
    cruise_ctl: f32,
}

impl Route {
    fn new() -> Self {
        Self {
            visit: Visit::Cruise,
            at: (0.0, 0.0),
            id: 0,
            timer: 0.0,
            cooldown: 20.0,
            visited: [0; 16],
            next_visited: 0,
            down: 0.0,
            hold: false,
            cruise_offset: 0.0,
            cruise_ctl: f32::NAN,
        }
    }
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

/// Rain (or snow) patches around the listener on the ground below.
fn rain(ground: Surface, snow: bool) -> ModelConfig {
    let name = match ground {
        Surface::Water => "on_water",
        Surface::Grass => "meadow",
        Surface::Sand => "on_soil",
        Surface::Rock => "on_rock",
        Surface::Snow => "snowfall",
        Surface::Forest => "under_canopy",
    };
    let mut c = Rain::preset(name).unwrap_or_default();
    c.area_m2 = 60.0;
    if snow {
        c.snow = 1.0;
    }
    ModelConfig::Rain(c)
}

fn air(preset: u8) -> ModelConfig {
    let name = Wind::PRESETS.get(preset as usize).copied().unwrap_or("breeze");
    ModelConfig::Wind(Wind::preset(name).unwrap_or_else(Wind::breeze))
}

impl Flight {
    /// Builds the landscape around the start and every pooled generator. Allocates: call off
    /// the audio thread.
    pub fn new(sample_rate: f32, seed: u64) -> Self {
        let sr = sample_rate;
        let mut land = Land::new(seed, 0);
        land.stream(0.0, 0.0, usize::MAX);
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
                    Pool::Rain => (SourceKind::Rain, Some(rain(Surface::Forest, false))),
                    Pool::Air => (SourceKind::Wind, Some(air(0))),
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
                    Pool::Air => (ObjectKind::Source, Geometry::Source { model: ModelKind::Wind, path: None }),
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
        let height = controls::flight_altitude(6.0);
        let ground = land.ground(0.0, 0.0);
        let pose = Pose { height, ground, ear: ground + height, ..Default::default() };
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
            voiced: vec![0.0; TILES].into_boxed_slice(),
            gaze: (0.0, 0.0),
            climb_offset: 0.0,
            env_key: (u32::MAX, -1.0, -1.0),
            refit_s: f64::NEG_INFINITY,
            ctl: None,
            dirty: true,
            birds_level: 0.6,
            filled_until: 0,
            near: [0; land::LOCAL],
            n_near: 0,
            relief_ctl: 1.0,
            paths: vec![PathTerrain::NONE; EMITTERS].into_boxed_slice(),
            path_db: vec![[0.0; NF]; EMITTERS].into_boxed_slice(),
            echoes: Echoes::new(sr),
            route: Route::new(),
            snowing: false,
            generation: 0,
            terrain_follow: f32::NAN,
        };
        f.locate();
        f
    }

    /// Starts already moving at `speed` (a scene that begins in the air).
    pub fn cruise(&mut self, speed: f32) {
        self.pose.speed = speed.clamp(0.0, 30.0);
        self.altitude = controls::flight_altitude(self.pose.speed);
        self.pose.height = self.altitude;
        self.pose.ear = self.pose.ground + self.pose.height;
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
        (self.pose.sin, self.pose.cos) = self.pose.heading.sin_cos();
    }

    /// Listener-relative position of a world point at absolute height `z` (m above sea level).
    #[inline]
    fn rel(&self, x: f64, y: f64, z: f32) -> Vec3 {
        let (s, c) = (self.pose.sin, self.pose.cos);
        let (dx, dy) = (x - self.pose.x, y - self.pose.y);
        Vec3::new((c * dx + s * dy) as f32, (-s * dx + c * dy) as f32, z - self.pose.ear)
    }

    /// Listener-relative position of a point `above` metres over the ground (or the water).
    #[inline]
    fn rel_on(&self, x: f64, y: f64, above: f32) -> Vec3 {
        self.rel(x, y, self.land.top(x, y) + above)
    }

    /// Velocity in the listener frame of a point at `rel` moving with `own` (world frame).
    #[inline]
    fn rel_vel(&self, rel: Vec3, own: [f32; 3]) -> Vec3 {
        let (s, c) = (self.pose.sin as f32, self.pose.cos as f32);
        let p = &self.pose;
        Vec3::new(
            -p.speed + p.turn * rel.y + c * own[0] + s * own[1],
            -p.side - p.turn * rel.x - s * own[0] + c * own[1],
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
        self.acoustics.set_listener_offset(inp.listener_offset);
        self.advance(dt, inp);
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
        let (amb_rain, amb) = self.ambient(inp);
        let c_air = crate::physics::air::speed_of_sound(amb.temperature_c as f64) as f32;

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
            em.controls.env = if slot.pool == Pool::Rain { amb_rain } else { amb };
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
            // Terrain between it and the listener: diffraction over hills, the local ground.
            let terrain = &self.path_db[idx];
            let hidden = 10f32.powf(terrain[MID_BAND] / 20.0);
            mixer.feed(MIX_SLOT0 + idx, tmp, g * hidden * (mix * core::f32::consts::FRAC_PI_2).sin(), em.generator.busy());
            let mut src = Source::at(rel, g);
            for i in 0..NF {
                src.eq_db[i] = eq_db[i] + terrain[i];
            }
            src.send = send;
            src.velocity = Some(vel);
            let path = &self.paths[idx];
            src.ground_z = Some(path.ground - self.pose.ear);
            src.ground = Some(if em.kind == SourceKind::Water { GroundType::Water } else { path.surface.ground() });
            // Its share of the field at the listener, for the slope echoes.
            self.echoes.add(tmp, g * hidden / (d + 4.0));
            self.acoustics.propagate(idx, &src, tmp, &mut self.bus);
        }
        self.echoes.render(&mut self.bus, len, c_air);
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
        let ground_speed = self.pose.ground_speed();
        let airspeed = (ground_speed * ground_speed + 0.16 * wind_ms * wind_ms).sqrt();
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
        let cruise = self.cruise_speed(st.get(id::FLIGHT_SPEED).clamp(0.0, 30.0), inp, dt);
        let stops = st.get(id::FLIGHT_STOPS).clamp(0.0, 1.0);
        self.update_route(dt, stops, inp.hold);
        if inp.hold {
            self.walk(inp, dt);
        } else {
            let target = self.route_speed(cruise);
            let dv = (target - self.pose.speed).clamp(-ACCEL * dt, ACCEL * dt);
            self.pose.speed += dv;
            self.pose.side -= self.pose.side.clamp(-HOLD_DECEL * dt, HOLD_DECEL * dt);
        }
        let alt = st.get(id::FLIGHT_ALTITUDE).clamp(1.2, 40.0);
        // Standing at a place: down to standing height.
        let alt = alt + (alt.min(1.7) - alt) * self.route.down;
        self.altitude += (alt - self.altitude) * (1.0 - (-dt / 2.5).exp());
        self.curviness = st.get(id::FLIGHT_CURVINESS).clamp(0.0, 1.0);
        self.density = st.get(id::FLIGHT_DENSITY).clamp(0.0, 2.0);
        let seed = st.get(id::FLIGHT_SEED).round().max(0.0) as u32;
        let biome = st.get(id::FLIGHT_BIOME).round().max(0.0) as u32;
        let relief = (st.get(id::FLIGHT_RELIEF).clamp(0.0, 2.0) * 20.0).round() / 20.0;
        if (seed != self.seed_ctl || biome != self.biome_ctl || relief != self.relief_ctl) && self.regen == 0.0 {
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
                self.relief_ctl = relief;
                for s in self.slots.iter_mut() {
                    if s.pool != Pool::Rain {
                        *s = Slot { target: Target::None, want: false, fade: 0.0, ..*s };
                    }
                }
                let seed = self.seed ^ (seed as u64).wrapping_mul(0x9E37_79B9);
                self.land.reset(seed, biome, relief);
                self.generation = self.generation.wrapping_add(1);
                self.terrain_follow = f32::NAN;
                self.route = Route::new();
                // Refilled over the next blocks (6 tiles each), not all at once.
                self.land.stream(self.pose.x, self.pose.y, 6);
                self.filled_until = self.blocks + 200;
                self.dirty = true;
            }
        }
    }

    /// The cruise: flight.speed plus what W / S added in flight (ramped at
    /// KEY_CRUISE_RATE). The offset is dropped when flight.speed changes or on a stop, so
    /// flying on after a stop resumes at flight.speed.
    fn cruise_speed(&mut self, ctl: f32, inp: &Inputs, dt: f32) -> f32 {
        let r = &mut self.route;
        if ctl != r.cruise_ctl || inp.hold {
            r.cruise_ctl = ctl;
            r.cruise_offset = 0.0;
        }
        if !inp.hold {
            r.cruise_offset = (r.cruise_offset + inp.moves.forward * KEY_CRUISE_RATE * dt).clamp(-ctl, 30.0 - ctl);
        }
        (ctl + r.cruise_offset).clamp(0.0, 30.0)
    }

    /// Standing in the world: the keys walk (W/S along where the head looks when steering by
    /// head, else along the course; A/D sideways; Shift runs). Speeds up at ACCEL, slows down
    /// at HOLD_DECEL; no keys = stand still.
    fn walk(&mut self, inp: &Inputs, dt: f32) {
        let m = inp.moves;
        let pace = if m.run { RUN } else { WALK };
        let g = if self.steering(inp) { self.gaze.0 } else { 0.0 };
        let (s, c) = g.sin_cos();
        let n = m.forward.hypot(m.strafe).max(1.0);
        let (f, r) = (m.forward / n, m.strafe / n);
        let tx = pace * (f * c + r * s);
        let ty = pace * (f * s - r * c);
        let (dx, dy) = (tx - self.pose.speed, ty - self.pose.side);
        let d = dx.hypot(dy);
        let rate = if tx.hypot(ty) < self.pose.ground_speed() { HOLD_DECEL } else { ACCEL };
        let k = if d > rate * dt { rate * dt / d } else { 1.0 };
        self.pose.speed += dx * k;
        self.pose.side += dy * k;
    }

    /// Steering by head is live (a mode is on and AirPods poses arrive).
    fn steering(&self, inp: &Inputs) -> bool {
        inp.head_tracked && inp.st.get(id::FLIGHT_STEER).round() as u32 > 0
    }

    /// Steers and moves the listener. With steering by head, the course turns towards where
    /// the head looks (yaw; pitch climbs and dives with `yaw_pitch`); otherwise it wanders
    /// along a gentle curve. Both go through the same turn-rate slew limit.
    fn advance(&mut self, dt: f32, inp: &Inputs) {
        let st = inp.st;
        // Without head tracking the head never turns: keep the wandering course instead.
        let mode = if inp.head_tracked { st.get(id::FLIGHT_STEER).round() as u32 } else { 0 };
        let max_turn = st.get(id::FLIGHT_TURN_RATE).to_radians();
        let g = 1.0 - (-dt / GAZE_TAU).exp();
        let (yaw, pitch) = if mode > 0 { (inp.head_yaw, inp.head_pitch) } else { (0.0, 0.0) };
        self.gaze.0 += (wrap(yaw) - self.gaze.0) * g;
        self.gaze.1 += (pitch - self.gaze.1) * g;
        let beyond = |a: f32| a.signum() * (a.abs() - DEAD_ZONE).max(0.0);
        let by_head = (beyond(self.gaze.0) / FULL_TURN).clamp(-1.0, 1.0) * max_turn;
        let m = inp.moves;
        // Keys: + = right, the turn rate is + = left.
        let keys = -(m.turn + if inp.hold { 0.0 } else { m.strafe }).clamp(-1.0, 1.0);
        let target = if inp.hold {
            // On foot: Q/E turn; walking forward with the head turned, the course follows the
            // gaze (so the view, which looks along the course, stays behind you).
            let follow = if mode > 0 && m.forward > 0.05 { by_head } else { 0.0 };
            (follow + keys * KEY_TURN).clamp(-KEY_TURN, KEY_TURN)
        } else if keys != 0.0 {
            // Keyboard steering in flight, on top of the head.
            let head = if mode > 0 { by_head } else { 0.0 };
            (head + keys * max_turn).clamp(-max_turn, max_turn)
        } else if mode > 0 {
            by_head
        } else if self.route.visit == Visit::Approach {
            // Heading for a place to stop: turn towards it (gently).
            let r = self.rel(self.route.at.0, self.route.at.1, 0.0);
            let bearing = r.y.atan2(r.x);
            (1.2 * bearing).clamp(-0.35, 0.35)
        } else {
            let k = curvature(self.land.seed, self.pose.s) * self.curviness as f64;
            (k * self.pose.speed as f64) as f32 + self.coast_bias()
        };
        let slew = if inp.hold { KEY_TURN_ACCEL } else { TURN_ACCEL } * dt;
        self.pose.turn += (target - self.pose.turn).clamp(-slew, slew);
        if mode >= 2 && !inp.hold {
            let climb = (beyond(self.gaze.1) / FULL_TURN).clamp(-1.0, 1.0) * CLIMB_PER_SPEED * self.pose.speed;
            self.climb_offset = (self.climb_offset + climb * dt).clamp(1.2 - self.altitude, 40.0 - self.altitude);
        } else {
            self.climb_offset *= (-dt / 6.0).exp();
        }

        let mid = self.pose.heading + 0.5 * (self.pose.turn * dt) as f64;
        let (fwd, side) = ((self.pose.speed * dt) as f64, (self.pose.side * dt) as f64);
        let (ms, mc) = mid.sin_cos();
        self.pose.x += fwd * mc - side * ms;
        self.pose.y += fwd * ms + side * mc;
        self.pose.heading = (self.pose.heading + (self.pose.turn * dt) as f64).rem_euclid(core::f64::consts::TAU);
        self.pose.s += fwd.hypot(side);
        self.locate();
        if self.regen == 0.0 {
            // Quicker right after a reseed, while the landscape is still filling in.
            let budget = if self.blocks < self.filled_until { 6 } else { 1 };
            self.land.stream(self.pose.x, self.pose.y, budget);
        }
        // Gentle swells of the flight level above walking height (none when walking).
        let base = (self.altitude + self.climb_offset).clamp(1.2, 40.0);
        let swell = (base - 2.5).clamp(0.0, 12.0) * 0.12;
        let n = land::noise2(self.land.seed, 0xA17, self.pose.x / 160.0, self.pose.y / 160.0) - 0.5;
        let h = (base + 2.0 * swell * n).max(1.0);
        // Follow the terrain: the flight level rides over the highest ground just ahead (so a
        // rise is climbed before it is reached), rising quicker than it sinks into a valley.
        let here = self.land.top(self.pose.x, self.pose.y);
        let mut ahead = here;
        let v = self.pose.speed.max(1.0) as f64;
        for t in [0.6, 1.2, 2.0] {
            let d = v * t;
            ahead = ahead.max(self.land.top(self.pose.x + d * self.pose.cos, self.pose.y + d * self.pose.sin) - 0.08 * d as f32);
        }
        if !self.terrain_follow.is_finite() {
            self.terrain_follow = ahead;
        }
        let tau = if ahead > self.terrain_follow { 0.9 } else { 2.2 };
        self.terrain_follow += (ahead - self.terrain_follow) * (1.0 - (-dt / tau).exp());
        let ear = (self.terrain_follow + h).max(here + h.min(1.2));
        let prev = self.pose.ear;
        self.pose.ground = here;
        self.pose.ear = ear;
        self.pose.climb = if dt > 0.0 && prev.is_finite() { ((ear - prev) / dt).clamp(-30.0, 30.0) } else { 0.0 };
        self.pose.height = ear - here;
        self.world.ground_z = -self.pose.height;
    }

    /// On a sea coast the wandering course keeps to the shore: it turns back towards land
    /// when the sea lies ahead and drifts towards the sea far inland (turn rate, rad/s).
    fn coast_bias(&self) -> f32 {
        if self.land.params.biome.relief.sea <= 0.0 {
            return 0.0;
        }
        let d = (60.0 + 6.0 * self.pose.speed as f64).min(180.0);
        let at = |a: f64| {
            let h = self.pose.heading + a;
            self.land.ground(self.pose.x + d * h.cos(), self.pose.y + d * h.sin())
        };
        let (ahead, left, right) = (at(0.0), at(0.7), at(-0.7));
        let toward = |higher: bool| if (left > right) == higher { 1.0 } else { -1.0 };
        if ahead < 1.5 {
            0.16 * toward(true)
        } else if ahead > 18.0 && left.min(right) < ahead {
            0.05 * toward(false)
        } else {
            0.0
        }
    }

    /// Speed the route allows: the cruise, easing into a stop, nothing while lingering.
    fn route_speed(&self, cruise: f32) -> f32 {
        match self.route.visit {
            Visit::Cruise => cruise,
            Visit::Approach => {
                let d = (self.route.at.0 - self.pose.x).hypot(self.route.at.1 - self.pose.y) as f32;
                cruise.min((2.0 * STOP_DECEL * (d - 10.0).max(0.0)).sqrt())
            }
            Visit::Linger => 0.0,
        }
    }

    /// Advances the stop-and-go: approach → linger → fly on. Holding still in the world
    /// suspends it: the flight level comes down to standing height and no place is sought
    /// until a while after flying on.
    fn update_route(&mut self, dt: f32, stops: f32, hold: bool) {
        let (px, py) = (self.pose.x, self.pose.y);
        let speed = self.pose.speed;
        let seed = self.land.seed;
        let r = &mut self.route;
        r.cooldown -= dt;
        let ease = |x: &mut f32, to: f32, tau: f32| *x += (to - *x) * (1.0 - (-dt / tau).exp());
        if hold != r.hold {
            r.hold = hold;
            if hold && r.visit == Visit::Linger {
                r.visited[r.next_visited] = r.id;
                r.next_visited = (r.next_visited + 1) % r.visited.len();
            }
            r.visit = Visit::Cruise;
        }
        if hold {
            ease(&mut r.down, 1.0, 1.5);
            r.cooldown = r.cooldown.max(15.0);
            self.pose.linger += (1.0 - self.pose.linger) * (1.0 - (-dt / 1.5).exp());
            return;
        }
        match r.visit {
            Visit::Cruise => ease(&mut r.down, 0.0, 3.0),
            Visit::Approach => {
                ease(&mut r.down, 0.0, 3.0);
                let (dx, dy) = (r.at.0 - px, r.at.1 - py);
                let d = dx.hypot(dy) as f32;
                let ahead = (dx * self.pose.cos + dy * self.pose.sin) as f32;
                if d < 12.0 || (speed < 0.4 && d < 40.0) {
                    r.visit = Visit::Linger;
                    let u = land::unit(land::hash(seed, r.id as i64, 1, 0x5709));
                    r.timer = (14.0 + 30.0 * stops) * (0.7 + 0.6 * u);
                } else if ahead < -5.0 || stops <= 0.0 {
                    // Passed it (steering by head went elsewhere): fly on.
                    r.visit = Visit::Cruise;
                    r.cooldown = 10.0;
                }
            }
            Visit::Linger => {
                ease(&mut r.down, 1.0, 4.0);
                r.timer -= dt;
                if r.timer <= 0.0 || stops <= 0.0 {
                    r.visit = Visit::Cruise;
                    r.cooldown = 40.0 - 25.0 * stops;
                    r.visited[r.next_visited] = r.id;
                    r.next_visited = (r.next_visited + 1) % r.visited.len();
                }
            }
        }
        let linger = if r.visit == Visit::Linger { 1.0 } else { 0.0 };
        self.pose.linger += (linger - self.pose.linger) * (1.0 - (-dt / 1.5).exp());
    }

    /// Looks ahead for a place worth stopping at (level-of-detail rate).
    fn find_stop(&mut self, stops: f32, steering: bool) {
        if self.route.visit != Visit::Cruise || self.route.cooldown > 0.0 || stops <= 0.0 || self.pose.speed < 0.5 || self.regen > 0.0 {
            return;
        }
        let (c, s) = (self.pose.cos, self.pose.sin);
        let cone = if steering { 0.9 } else { 0.6 };
        let mut best: Option<(f64, usize)> = None;
        for (i, x) in self.land.sites.iter().enumerate() {
            if !x.alive || !x.scenic || self.route.visited.contains(&x.id) {
                continue;
            }
            let (dx, dy) = (x.x - self.pose.x, x.y - self.pose.y);
            let d = dx.hypot(dy);
            let ahead = c * dx + s * dy;
            if !(30.0..320.0).contains(&d) || ahead < cone * d {
                continue;
            }
            // Not every place: a share by the stops setting, decided once per place.
            if land::unit(land::hash(self.land.seed, x.id as i64, 0, 0x570B)) > 0.1 + 0.85 * stops {
                continue;
            }
            if best.map_or(true, |b| d < b.0) {
                best = Some((d, i));
            }
        }
        if let Some((_, i)) = best {
            let x = self.land.sites[i];
            self.route.visit = Visit::Approach;
            self.route.at = (x.x, x.y);
            self.route.id = x.id;
        }
    }

    /// Birds: perched singing, flushed by the passing listener, flying off to a new perch.
    fn step_birds(&mut self, dt: f32, inp: &Inputs) {
        let pose = self.pose;
        let (hs, hc) = (pose.sin, pose.cos);
        let speed = pose.ground_speed();
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
                    let flush_at = 2.0 + 1.2 * b.species().def().flush + 0.9 * speed;
                    if d < flush_at && ahead > -4.0 {
                        b.checked = true;
                        let mut r = land::Stream::new(land::hash(seed, b.id as i64, 0, 0xF1A5));
                        let shy = match b.species() {
                            Species::WoodPigeon => 0.85,
                            Species::TawnyOwl => 0.6,
                            Species::SongThrush | Species::Cuckoo => 0.5,
                            Species::Chaffinch => 0.4,
                            Species::Robin => 0.25,
                            Species::Gull | Species::Raven => 0.6,
                            Species::Skylark => 0.45,
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
            Target::Tree { tile, k, id } => {
                let t = self.land.tile_trees(tile as usize).get(k as usize)?;
                (t.id == id).then(|| {
                    let r = self.rel(t.x, t.y, t.ground + t.crown_z);
                    (r, self.rel_vel(r, [0.0; 3]), false)
                })
            }
            Target::Cell { i, j } => {
                let c = &self.land.tiles[self.land.tile(i, j)?].cell;
                let r = self.rel(c.x, c.y, c.ground + c.canopy_z);
                Some((r, self.rel_vel(r, [0.0; 3]), false))
            }
            Target::Bird { idx, id } => {
                let b = &self.land.birds[idx as usize];
                (b.alive && b.id == id).then(|| {
                    let r = self.rel_on(b.pos[0], b.pos[1], b.pos[2] as f32);
                    (r, self.rel_vel(r, b.vel), b.state == BirdState::Flying)
                })
            }
            Target::Site { idx, id } => {
                let x = &self.land.sites[idx as usize];
                (x.alive && x.id == id).then(|| {
                    let r = self.rel_on(x.x, x.y, x.z);
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
        let mut near = [0usize; land::LOCAL];
        self.n_near = self.land.local_features(self.pose.x, self.pose.y, 25.0, &mut near);
        self.near = near;
        let density = self.density;
        let tree_rank = 0.5 * density;
        let (px, py) = (self.pose.x, self.pose.y);
        let regen = self.regen > 0.0;

        // Nearest trees.
        let mut near = Best::<24>::new();
        for t in self.land.tiles_near(px, py, 40.0) {
            for (k, tr) in self.land.tile_trees(t).iter().enumerate() {
                if tr.rank >= tree_rank {
                    continue;
                }
                let d = self.rel(tr.x, tr.y, tr.ground + tr.crown_z).length();
                if d < 40.0 {
                    near.push(1.0 / d.max(0.5), Target::Tree { tile: t as u16, k: k as u16, id: tr.id });
                }
            }
        }
        if !regen {
            self.assign(Pool::Near, &near);
        }

        // Crown area now voiced by near trees, per tile.
        self.voiced.fill(0.0);
        for i in Self::range(Pool::Near) {
            if let Target::Tree { tile, k, .. } = self.slots[i].target {
                if let Some(tr) = self.land.tile_trees(tile as usize).get(k as usize) {
                    self.voiced[tile as usize] += tr.crown_area() * self.slots[i].fade;
                }
            }
        }

        // Aggregate cells: every live tile is a stand.
        let mut cells = Best::<16>::new();
        for tile in self.land.tiles.iter().filter(|t| t.alive) {
            let c = &tile.cell;
            let area = c.area_at(density);
            if area < 15.0 {
                continue;
            }
            let d = self.rel(c.x, c.y, c.ground + c.canopy_z).length();
            if d < MAX_RANGE {
                cells.push(area.sqrt() / d.max(5.0), Target::Cell { i: tile.i, j: tile.j });
            }
        }
        if !regen {
            self.assign(Pool::Cell, &cells);
        }
        for i in Self::range(Pool::Cell) {
            if let Target::Cell { i: ti, j: tj } = self.slots[i].target {
                if let Some(t) = self.land.tile(ti, tj) {
                    let area = self.land.tiles[t].cell.area_at(density);
                    let left = (area - self.voiced[t]).max(0.0);
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
            let (dx, dy) = (b.pos[0] - px, b.pos[1] - py);
            if dx * dx + dy * dy > 160.0 * 160.0 {
                continue;
            }
            let d = self.rel_on(b.pos[0], b.pos[1], b.pos[2] as f32).length();
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
        let mut air = Best::<8>::new();
        for (i, x) in self.land.sites.iter().enumerate() {
            if !x.alive {
                continue;
            }
            let (dx, dy) = (x.x - px, x.y - py);
            if dx * dx + dy * dy > (MAX_RANGE * MAX_RANGE) as f64 {
                continue;
            }
            let d = self.rel_on(x.x, x.y, x.z).length();
            let t = Target::Site { idx: i as u16, id: x.id };
            match x.kind {
                SiteKind::Water if d < MAX_RANGE => water.push(x.weight / d.max(2.0), t),
                SiteKind::Insects if d < 90.0 => insects.push(x.weight / d.max(2.0), t),
                SiteKind::Fire if d < 150.0 => fire.push(x.weight / d.max(2.0), t),
                SiteKind::Air if d < 160.0 => air.push(x.weight / d.max(4.0), t),
                _ => {}
            }
        }
        if !regen {
            self.assign(Pool::Water, &water);
            self.assign(Pool::Insects, &insects);
            self.assign(Pool::Fire, &fire);
            self.assign(Pool::Air, &air);
        }

        // Rain (or snow) patches follow the listener; their surface follows the ground below.
        let ground = self.land.surface_at(px, py);
        for i in Self::range(Pool::Rain) {
            let cfg = rain(ground, self.snowing);
            if self.world.emitters[i].base != Some(cfg) {
                self.world.emitters[i].base = Some(cfg);
                self.dirty = true;
            }
        }

        // Places to stop at, and what the terrain does to every voiced path.
        let steering = inp.head_tracked && inp.st.get(id::FLIGHT_STEER).round() > 0.0;
        self.find_stop(inp.st.get(id::FLIGHT_STOPS).clamp(0.0, 1.0), steering);
        self.terrain_paths();
        if self.blocks % (LOD_EVERY * 4) == 1 {
            let relief = self.relief_ctl;
            let scale = (0.5 + 0.5 * relief).min(1.2);
            self.echoes.scan(&self.land, self.pose.x, self.pose.y, self.pose.ear, self.pose.heading, scale);
        }
    }

    /// Terrain between every sounding slot and the listener (profile diffraction, local
    /// ground), turned into a spectral loss on the propagation's fit grid.
    fn terrain_paths(&mut self) {
        let lis = [self.pose.x, self.pose.y, self.pose.ear as f64];
        let freqs = self.acoustics.context().design.freqs;
        let c = self.acoustics.context().speed_of_sound;
        for i in 0..self.slots.len() {
            let slot = self.slots[i];
            if slot.target == Target::None || slot.pool == Pool::Rain {
                self.paths[i] = PathTerrain { ground: self.pose.ground, surface: self.land.surface_at(self.pose.x, self.pose.y), ..PathTerrain::NONE };
                self.path_db[i] = [0.0; NF];
                continue;
            }
            // The emitter's world position from its listener-relative one.
            let r = self.world.emitters[i].position;
            let (s, co) = (self.pose.sin, self.pose.cos);
            let wx = self.pose.x + co * r.x as f64 - s * r.y as f64;
            let wy = self.pose.y + s * r.x as f64 + co * r.y as f64;
            let src = [wx, wy, (self.pose.ear + r.z) as f64];
            let p = acoustic::terrain_path(&self.land, src, lis);
            self.paths[i] = p;
            p.loss_db(&freqs, c, &mut self.path_db[i]);
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
            Target::Tree { tile, k, .. } => {
                let tr = self.land.tile_trees(tile as usize)[k as usize];
                s.extent = tr.crown_r;
                Some(foliage(tr.species(), tr.crown_area(), exposure(self.land.base_cover(tr.x, tr.y))))
            }
            Target::Cell { i, j } => {
                let c = self.land.tile(i, j).map(|t| self.land.tiles[t].cell).unwrap_or_default();
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
                    SiteKind::Air => Some(air(x.preset)),
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
        let (px, py) = (self.pose.x, self.pose.y);
        let mut n = 0;
        let mut radius = 30.0f32;
        'scan: for pass in 0..2 {
            n = 0;
            for t in self.land.tiles_near(px, py, radius as f64) {
                for tr in self.land.tile_trees(t) {
                    if tr.rank >= tree_rank {
                        continue;
                    }
                    let r = self.rel(tr.x, tr.y, 0.0);
                    let d2 = r.x * r.x + r.y * r.y;
                    // A trunk you fly through is not an obstacle (it would wrap the ears).
                    if d2 < radius * radius && d2 > 1.0 {
                        if n == self.trunks.len() {
                            if pass == 0 {
                                radius = 20.0;
                                continue 'scan;
                            }
                            break 'scan;
                        }
                        // Trunks stand on the listener's ground plane in the propagation: keep
                        // their tops where they are.
                        let height = (tr.ground + tr.top() - self.pose.ground).max(0.5);
                        self.trunks[n] = Trunk { x: r.x, y: r.y, radius: tr.trunk_r, height };
                        n += 1;
                    }
                }
            }
            break;
        }
        let trunks = n;
        // Boulders diffract; ridges and hills are part of the relief (terrain_paths).
        let mut m = 0;
        for f in self.land.features.iter() {
            if !f.alive || m >= self.obstacles.len() || f.kind != FeatureKind::Rock {
                continue;
            }
            let (dx, dy) = (f.x - px, f.y - py);
            if dx * dx + dy * dy > 80.0 * 80.0 {
                continue;
            }
            let r = self.rel_on(f.x, f.y, 0.7 * f.size);
            self.obstacles[m] = Obstacle::Sphere { center: r, radius: f.size };
            m += 1;
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

    /// Ground, stand density, ear height, temperature and reverb for the propagation
    /// (quantised, so the forest tail and ground model are rebuilt only when they really
    /// change). The reverb follows the relief: a valley or a cirque rings longer, an open plain
    /// or a dune field is dry, snow swallows the tail.
    fn environment(&mut self, inp: &Inputs) {
        let (px, py) = (self.pose.x, self.pose.y);
        let near = &self.near[..self.n_near];
        let surface = self.land.surface_at(px, py);
        let b = self.land.biome_def();
        let ahead = (px + 15.0 * self.pose.cos, py + 15.0 * self.pose.sin);
        let local = b.density_ha * self.density * self.land.cover_near(px, py, near).max(self.land.cover_near(ahead.0, ahead.1, near));
        let q = (local / 50.0).round() * 50.0;
        let ear = self.pose.height;
        let (_, d0, e0) = self.env_key;
        let ear_q = if (ear - e0).abs() > (0.04 * e0).max(0.05) { ear } else { e0 };
        let dens = if (q - d0).abs() >= 50.0 { q } else { d0 };
        self.env_key = (surface as u32, dens, ear_q);
        let mut env = inp.acoustic;
        env.ground = surface.ground();
        env.tree_density_per_ha = dens.max(0.0);
        env.ear_height = ear_q;
        env.temperature_c = (env.temperature_c + b.temperature).clamp(-30.0, 45.0);
        self.snowing = b.snow > 0.5 || env.temperature_c < 0.5;
        let enclosure = (self.echoes.enclosure * 10.0).round() / 10.0;
        let snow = if surface == Surface::Snow { 0.6 } else { 1.0 };
        env.reverb *= (0.6 + 2.2 * enclosure).min(1.8) * snow;
        self.acoustics.set_environment(&env);
    }

    /// Weather as the generators hear it here: the biome's temperature and wind; while it
    /// snows, the precipitation reaches only the snow patches (flakes do not patter on leaves
    /// or water). Returns (for the rain/snow patches, for everything else).
    fn ambient(&self, inp: &Inputs) -> (Ambient, Ambient) {
        let b = self.land.biome_def();
        let mut a = inp.ambient;
        a.temperature_c += b.temperature;
        a.wind_ms *= b.wind;
        if b.sand > 0.5 {
            a.humidity *= 0.4;
        }
        let precip = a;
        if self.snowing {
            a.rain_mm_h = 0.0;
            a.drip_mm_h = 0.0;
            a.puddles = 0.0;
            a.leaf_wetness = 0.0;
        }
        (precip, a)
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
        // Features first: they are few and big. Heights are relative to the ears, on the
        // terrain (rivers and lakes at their water level).
        for f in self.land.features.iter() {
            if !f.alive {
                continue;
            }
            let r = self.rel(f.x, f.y, 0.0);
            if r.x * r.x + r.y * r.y > 260.0 * 260.0 {
                continue;
            }
            let mid = f.levels[f.path_len as usize / 2];
            let (kind, z) = match f.kind {
                FeatureKind::River => (ObjectKind::River, mid - self.pose.ear),
                FeatureKind::Lake => (ObjectKind::Lake, f.level - self.pose.ear),
                FeatureKind::Ridge => (ObjectKind::Rock, self.land.ground(f.x, f.y) - self.pose.ear),
                FeatureKind::Rock => (ObjectKind::Rock, self.land.ground(f.x, f.y) + f.z - self.pose.ear),
                FeatureKind::Waterfall => (ObjectKind::Source, self.land.ground(f.x, f.y) + f.z - self.pose.ear),
                FeatureKind::Campfire | FeatureKind::Meadow => (ObjectKind::Source, self.land.ground(f.x, f.y) - self.pose.ear),
            };
            snap.object(ObjectInfo { id: f.id, kind: kind as u32, x: r.x, y: r.y, z, flags: 0 });
            if f.path_len > 0 {
                let n = f.path_len as usize;
                let pts = f.path[..n].iter().enumerate().map(|(k, p)| {
                    let z = match f.kind {
                        FeatureKind::River => f.levels[k],
                        FeatureKind::Lake => f.level,
                        _ => self.land.ground(p[0], p[1]),
                    };
                    self.rel(p[0], p[1], z)
                });
                snap.path(f.id, pts);
            }
        }
        // Birds that exist at the current BIRDS level.
        let threshold = 0.5 * (self.birds_level / 0.6).clamp(0.0, 1.67);
        let mut shown = 0;
        for b in self.land.birds.iter() {
            if !b.alive || b.rank >= threshold || shown >= 64 {
                continue;
            }
            let (dx, dy) = (b.pos[0] - self.pose.x, b.pos[1] - self.pose.y);
            if dx * dx + dy * dy > 110.0 * 110.0 {
                continue;
            }
            shown += 1;
            let r = self.rel_on(b.pos[0], b.pos[1], b.pos[2] as f32);
            if r.length() < 160.0 {
                let flags = if b.state == BirdState::Flying { object_flags::MOVING } else { 0 };
                snap.object(ObjectInfo { id: b.id, kind: ObjectKind::Bird as u32, x: r.x, y: r.y, z: r.z, flags });
            }
        }
        self.publish_trees(&mut snap);
    }

    /// Publishes where the listener is in the landscape and what it looks like, for the UI's
    /// terrain (it samples the relief itself with `asmr_terrain_sample`).
    pub fn publish_frame(&self, bridge: &FrameBridge) {
        let b = self.land.biome_def();
        bridge.publish(&bridge::Frame {
            x: self.pose.x,
            y: self.pose.y,
            heading: self.pose.heading as f32,
            ear_z: self.pose.ear,
            ground_z: self.pose.ground,
            relief: self.land.relief,
            seed: self.land.seed,
            biome: self.land.biome,
            generation: self.generation,
            linger: self.pose.linger,
            snow: if self.snowing { 1.0 } else { 0.0 },
            snow_cover: b.snow,
            sand_cover: b.sand,
            rock_slope: b.rock_slope,
            sea: if b.relief.sea > 0.0 { 1.0 } else { 0.0 },
            enclosure: self.echoes.enclosure,
            _pad: 0.0,
        });
    }

    /// The nearest trees that fit in the snapshot (radius from a distance histogram).
    fn publish_trees(&self, snap: &mut Snapshot) {
        let room = bridge::OBJECTS.saturating_sub(snap.len());
        if room == 0 {
            return;
        }
        const BIN: f32 = 5.0;
        const BINS: usize = 20;
        let mut hist = [0usize; BINS];
        let tree_rank = 0.5 * self.density;
        let (px, py) = (self.pose.x, self.pose.y);
        let reach = (BIN * BINS as f32) as f64;
        for t in self.land.tiles_near(px, py, reach) {
            for tr in self.land.tile_trees(t) {
                if tr.rank < tree_rank {
                    let r = self.rel(tr.x, tr.y, 0.0);
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
        for t in self.land.tiles_near(px, py, radius as f64) {
            for tr in self.land.tile_trees(t) {
                if tr.rank < tree_rank {
                    let r = self.rel(tr.x, tr.y, 0.0);
                    if r.x * r.x + r.y * r.y < radius * radius {
                        snap.object(ObjectInfo { id: tr.id, kind: ObjectKind::Tree as u32, x: r.x, y: r.y, z: tr.ground - self.pose.ear, flags: (tr.species as u32 + 1) << 8 });
                    }
                }
            }
        }
    }
}

/// Curvature (1/m) of the wandering course at distance s, for curviness 1: three slow sines
/// (530, 310, 170 m), deterministic per seed.
fn curvature(seed: u64, s: f64) -> f64 {
    let mut r = land::Stream::new(land::hash(seed, 0, 0, 0x9A7));
    let p = [r.range(0.0, 6.283) as f64, r.range(0.0, 6.283) as f64, r.range(0.0, 6.283) as f64];
    let tau = core::f64::consts::TAU;
    (0.55 * (tau * s / 530.0 + p[0]).sin() + 0.30 * (tau * s / 310.0 + p[1]).sin() + 0.15 * (tau * s / 170.0 + p[2]).sin()) / MIN_RADIUS
}

/// Angle wrapped to [-π, π).
fn wrap(a: f32) -> f32 {
    (a + core::f32::consts::PI).rem_euclid(core::f32::consts::TAU) - core::f32::consts::PI
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
    /// Relief scale, 0 (flat) .. 2 (1 = as the biome).
    #[serde(default)]
    pub relief: Option<f32>,
    /// Stops at places worth lingering, 0 (never) .. 1 (at every one, longer).
    #[serde(default)]
    pub stops: Option<f32>,
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
        set(id::FLIGHT_RELIEF, self.relief);
        set(id::FLIGHT_STOPS, self.stops);
        if self.start {
            engine.set_param(crate::engine::param::FLIGHT, 1.0);
        }
        Ok(self.start)
    }
}
