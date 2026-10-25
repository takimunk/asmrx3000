//! Scene graph: what exists in the world and which sound emitters each object owns.
//!
//! Two layers:
//! * [`SceneDesc`] — the declarative, serialisable description (JSON). This is what a future
//!   LLM prompt produces; it is validated by [`World::build`].
//! * [`World`] — the runtime graph built from it: entities with stable `u32` ids, a
//!   transform/geometry each, and the emitters they own (a tree owns its canopy wind, a river
//!   its segment streams, a bird its voice + wings). Emitter positions are derived from their
//!   owner every block, so moving a tree moves its rustle, moving a river translates its
//!   polyline.
//!
//! Coordinates are the engine's: listener's head at the origin, +x forward, +y left, +z up,
//! metres. The ground is at `environment.ground_z` (default -1.7, i.e. ear height 1.7 m).

use std::sync::atomic::{AtomicU32, Ordering};

use serde::{Deserialize, Serialize};

use crate::director::Species;
use crate::engine::{AtomicF32, SourceKind, MAX_SOURCES};
use crate::math::Vec3;
use crate::mix::SoundClass;
use crate::rng::Rng;
use crate::sources::common::{rain_mm_h, wind_slider};
use crate::sources::{Bird, Controls, Flyer, Generator, ModelConfig, ModelKind, SoundDesc, Wings};

/// The default scene: forest by a river.
pub const DEFAULT_SCENE_JSON: &str = include_str!("../scenes/forest_river.json");
/// Test fixture (not a shipped scene): fly-bys, a flock changing trees, an approach/recede, circling.
#[cfg(test)]
pub const DEMO_SCENE_JSON: &str = include_str!("director/fixtures/demo_flyby.json");

pub const MAX_OBJECTS: usize = 64;

// ---------------------------------------------------------------------------------------
// Description (data)
// ---------------------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneDesc {
    pub name: String,
    /// Seed for every stochastic choice (voices, timings, paths). Same seed, same scene.
    #[serde(default)]
    pub seed: u64,
    #[serde(default)]
    pub environment: EnvironmentDesc,
    #[serde(default)]
    pub director: DirectorDesc,
    /// The world's own course: clock, weather spells, the brook after rain; steered by the
    /// `direction.*` controls (direction/, docs/direction.md). Omitted = the scene stands still.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<crate::direction::DirectionDesc>,
    pub entities: Vec<EntityDesc>,
    /// Folder that relative clip file paths resolve against (clips.rs).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assets: Option<String>,
    /// Panel rows of the clip groups (names, icons, parameters shown); see clips.rs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub controls: Vec<crate::clips::GroupUi>,
    /// Scene levers: sliders with a meaning that move several clip-group parameters at once
    /// (clips.rs `LeverDesc`); the direction moves them too.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub levers: Vec<crate::clips::LeverDesc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct EnvironmentDesc {
    /// Local time, hours 0..24. Drives which species sing and fly.
    pub time_of_day: f32,
    pub season: Season,
    /// Rain intensity 0..1 (0.1 drizzle, 0.5 heavy, 1 cloudburst = 50 mm/h); `rain_mm_h`
    /// takes precedence when given.
    pub rain: f32,
    /// Rain rate in the open, mm/h (0..200).
    pub rain_mm_h: Option<f32>,
    /// Air temperature, °C.
    pub temperature_c: f32,
    /// Relative humidity 0..1.
    pub humidity: f32,
    /// Thunderstorm activity 0..1 (lightning rate and closeness).
    pub storm: f32,
    /// How wet everything is at the start, 0..1 ("after a storm" = 1): leaves, ground.
    pub wetness: f32,
    /// Speed-up of drying and draining (canopy and ground water stores), 1 = real time.
    pub weather_time_scale: f32,
    /// Mean wind at canopy height in m/s; overrides `wind` (slider units) when given.
    pub wind_ms: Option<f32>,
    /// Initial slider values (0..1); omitted = engine defaults.
    pub wind: Option<f32>,
    pub birds: Option<f32>,
    pub water: Option<f32>,
    /// Ground height relative to the listener's ears.
    pub ground_z: f32,
    /// What the listener is immersed in. `water`: the physical models sound muffled, as
    /// heard with the head under the surface (clips are left alone: they are recorded there).
    pub medium: Medium,
    /// Muffling cutoff for `water`, Hz (two-pole low-pass); omitted = 900.
    pub muffle_hz: Option<f32>,
    /// The mix's loudness target for this scene, LUFS (mix.loudness, -36..-10); omitted = -22.
    /// A meditation sits far below a forest.
    pub loudness_lufs: Option<f32>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Medium {
    #[default]
    Air,
    Water,
}

impl Default for EnvironmentDesc {
    fn default() -> Self {
        Self {
            time_of_day: 9.0,
            season: Season::Summer,
            rain: 0.0,
            rain_mm_h: None,
            temperature_c: 16.0,
            humidity: 0.75,
            storm: 0.0,
            wetness: 0.0,
            weather_time_scale: 1.0,
            wind_ms: None,
            wind: None,
            birds: None,
            water: None,
            ground_z: -1.7,
            medium: Medium::Air,
            muffle_hz: None,
            loudness_lufs: None,
        }
    }
}

impl EnvironmentDesc {
    /// Starting RAIN parameter, mm/h.
    pub fn initial_rain_mm_h(&self) -> f32 {
        self.rain_mm_h.unwrap_or_else(|| rain_mm_h(self.rain)).clamp(0.0, 200.0)
    }

    /// Starting WIND slider value, if the scene sets one.
    pub fn initial_wind(&self) -> Option<f32> {
        self.wind_ms.map(wind_slider).or(self.wind)
    }

    fn validate(&self) -> Result<(), String> {
        let ok = |v: f32, lo: f32, hi: f32| v.is_finite() && v >= lo && v <= hi;
        if !ok(self.temperature_c, -40.0, 50.0) || !ok(self.humidity, 0.0, 1.0) || !ok(self.storm, 0.0, 1.0) || !ok(self.wetness, 0.0, 1.0) {
            return Err("environment: temperature_c -40..50, humidity/storm/wetness 0..1".into());
        }
        if !ok(self.initial_rain_mm_h(), 0.0, 200.0) || !ok(self.weather_time_scale, 0.0, 1000.0) {
            return Err("environment: rain_mm_h 0..200, weather_time_scale 0..1000".into());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Season {
    Spring,
    #[default]
    Summer,
    Autumn,
    Winter,
}

/// Scene-wide director policy.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct DirectorDesc {
    /// Seconds between flock events (several birds leave one tree for another together).
    pub flock_interval: [f32; 2],
    /// Time of the first flock event; omitted = drawn from `flock_interval`.
    pub first_flock: Option<f32>,
    /// Directed scene (director/encounters.rs): a song budget by hour, slow pulse and lulls,
    /// and fly-bys, circling, visitors, flocks and gusts cued one at a time instead of each on
    /// its own timer (`flock_interval` and the performers' `first`/`interval` are then unused).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encounters: Option<crate::director::EncountersDesc>,
}

impl Default for DirectorDesc {
    fn default() -> Self {
        Self { flock_interval: [60.0, 150.0], first_flock: None, encounters: None }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EntityDesc {
    Tree(TreeDesc),
    River(RiverDesc),
    Bird(BirdDesc),
    /// Any physical sound model at a point or along a path (rain patch, waterfall, campfire,
    /// crickets, distant wind, the storm's thunder).
    Source(SourceDesc),
    /// Recorded or generated sound files, placed and moved by the choreography (clips.rs).
    Clip(crate::clips::ClipDesc),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceDesc {
    #[serde(default)]
    pub id: Option<u32>,
    /// Where the source is (required unless `path` is given).
    #[serde(default)]
    pub position: Option<[f32; 3]>,
    /// Extended source: emitters spread evenly along this polyline.
    #[serde(default)]
    pub path: Option<Vec<[f32; 3]>>,
    /// Number of decorrelated emitters along `path` (1..16).
    #[serde(default = "one_u32")]
    pub segments: u32,
    /// Model + preset + parameter overrides (docs/physical-models.md).
    pub sound: SoundDesc,
}

fn one_u32() -> u32 {
    1
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TreeDesc {
    /// Stable id; omitted = assigned (1000 + index).
    #[serde(default)]
    pub id: Option<u32>,
    /// Trunk base, normally on the ground.
    pub position: [f32; 3],
    #[serde(default)]
    pub species: TreeSpecies,
    /// Height of the canopy centre above the base, metres.
    #[serde(default = "default_canopy_height")]
    pub canopy_height: f32,
    /// Horizontal radius of the canopy; birds perch within it.
    #[serde(default = "default_canopy_radius")]
    pub canopy_radius: f32,
    /// Crown sound (model `foliage`); omitted = the species preset. `area_m2` defaults to
    /// the crown area.
    #[serde(default)]
    pub foliage: Option<SoundDesc>,
    /// Ground under the crown (model `rain`: throughfall and canopy drips); omitted =
    /// `under_canopy` (`under_conifer` for conifers).
    #[serde(default)]
    pub ground: Option<SoundDesc>,
}

fn default_canopy_height() -> f32 {
    5.0
}
fn default_canopy_radius() -> f32 {
    2.0
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TreeSpecies {
    #[default]
    Birch,
    Oak,
    Spruce,
    Willow,
    Alder,
    Aspen,
    Pine,
}

impl TreeSpecies {
    /// Foliage preset for the crown.
    pub fn foliage_preset(self) -> &'static str {
        match self {
            TreeSpecies::Birch => "birch",
            TreeSpecies::Oak => "oak",
            TreeSpecies::Spruce => "spruce",
            TreeSpecies::Willow => "willow",
            TreeSpecies::Alder => "alder",
            TreeSpecies::Aspen => "aspen",
            TreeSpecies::Pine => "pine",
        }
    }

    pub fn is_conifer(self) -> bool {
        matches!(self, TreeSpecies::Spruce | TreeSpecies::Pine)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RiverDesc {
    #[serde(default)]
    pub id: Option<u32>,
    /// Centre line, at least two points.
    pub path: Vec<[f32; 3]>,
    /// Flow scale relative to the WATER slider.
    #[serde(default = "one")]
    pub flow: f32,
    /// Number of decorrelated emitters along the path.
    #[serde(default = "default_segments")]
    pub segments: u32,
    /// Legacy: scales air entrainment relative to 350.
    #[serde(default = "default_bubble_rate")]
    pub bubble_rate: f32,
    /// Contrast between riffles and pools along the stream (0 = an even stream, 1 = the
    /// natural alternation, where one shallow riffle can be ~18 dB louder than the pools).
    #[serde(default = "one")]
    pub riffles: f32,
    /// Water model (model `water`); omitted = `brook_pebbles`. `length_m` defaults to the
    /// river length per segment.
    #[serde(default)]
    pub sound: Option<SoundDesc>,
}

fn one() -> f32 {
    1.0
}
fn default_segments() -> u32 {
    5
}
fn default_bubble_rate() -> f32 {
    350.0
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BirdDesc {
    #[serde(default)]
    pub id: Option<u32>,
    pub species: Species,
    pub behaviour: BehaviourDesc,
    /// Voice pitch override, Hz (default: species typical, jittered per individual).
    #[serde(default)]
    pub pitch: Option<f32>,
    #[serde(default = "one")]
    pub tempo: f32,
    /// Linear gain on voice and wings (1 = species default).
    #[serde(default = "one")]
    pub loudness: f32,
}

/// How a bird behaves over time. See docs/director.md for the state machines.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum BehaviourDesc {
    /// Lives in the trees: perched -> sings -> takes off -> flies a curved path -> lands on
    /// another tree. Joins flock events.
    Resident {
        /// Tree id it starts on.
        home: u32,
        /// Scales how often it changes trees (1 = species default).
        #[serde(default = "one")]
        roam: f32,
        /// Follows the ORBIT slider: circles the listener while ORBIT > 0.
        #[serde(default)]
        orbit: bool,
    },
    /// Lives in the open, without a home tree (skylark on a meadow, gulls on a shore, a raven
    /// over the mountains): feeds on the ground, sings from the ground or in a song flight,
    /// soars over the area; perches on trees inside it if there are any. docs/director.md.
    Open {
        /// Centre of the area (x, y; z is ignored: birds use the ground).
        #[serde(default)]
        center: [f32; 3],
        #[serde(default = "default_open_radius")]
        radius: f32,
        #[serde(default = "one")]
        roam: f32,
    },
    /// Off stage most of the time; now and then flies straight past the listener, calling.
    Flyby {
        #[serde(default = "default_first")]
        first: f32,
        #[serde(default = "default_flyby_interval")]
        interval: [f32; 2],
        /// m/s; omitted = species range.
        #[serde(default)]
        speed: Option<f32>,
        /// Closest approach to the listener's head, metres.
        #[serde(default = "default_min_distance")]
        min_distance: f32,
        /// Height of the pass relative to the ears, metres.
        #[serde(default = "one")]
        height: f32,
        /// Half-length of the pass, metres (starts and ends this far from the closest point).
        #[serde(default = "default_reach")]
        reach: f32,
    },
    /// Flies in, circles a point (default: the listener's head) for a few laps, leaves.
    Circle {
        #[serde(default)]
        center: [f32; 3],
        #[serde(default = "default_radius")]
        radius: f32,
        #[serde(default = "one")]
        height: f32,
        #[serde(default)]
        speed: Option<f32>,
        #[serde(default = "default_laps")]
        laps: [f32; 2],
        #[serde(default = "default_first")]
        first: f32,
        #[serde(default = "default_circle_interval")]
        interval: [f32; 2],
        /// Tree to rest on between rounds; omitted = leaves the stage.
        #[serde(default)]
        home: Option<u32>,
    },
    /// Appears far away, comes closer in hops (singing at each stop), lingers near the
    /// listener, then recedes in another direction and leaves.
    Approach {
        #[serde(default = "default_first")]
        first: f32,
        #[serde(default = "default_approach_interval")]
        interval: [f32; 2],
        #[serde(default = "default_far")]
        far: f32,
        #[serde(default = "default_near")]
        near: f32,
        #[serde(default = "default_hops")]
        hops: u32,
        /// Seconds singing at each stop.
        #[serde(default = "default_linger")]
        linger: [f32; 2],
        #[serde(default)]
        speed: Option<f32>,
    },
}

fn default_open_radius() -> f32 {
    25.0
}
fn default_first() -> f32 {
    10.0
}
fn default_flyby_interval() -> [f32; 2] {
    [30.0, 90.0]
}
fn default_min_distance() -> f32 {
    2.0
}
fn default_reach() -> f32 {
    40.0
}
fn default_radius() -> f32 {
    4.0
}
fn default_laps() -> [f32; 2] {
    [1.5, 3.0]
}
fn default_circle_interval() -> [f32; 2] {
    [40.0, 120.0]
}
fn default_approach_interval() -> [f32; 2] {
    [60.0, 180.0]
}
fn default_far() -> f32 {
    35.0
}
fn default_near() -> f32 {
    3.0
}
fn default_hops() -> u32 {
    3
}
fn default_linger() -> [f32; 2] {
    [3.0, 6.0]
}

impl SceneDesc {
    pub fn from_json(json: &str) -> Result<Self, String> {
        serde_json::from_str(json).map_err(|e| format!("scene: {e}"))
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    pub fn default_scene() -> Self {
        Self::from_json(DEFAULT_SCENE_JSON).expect("built-in scene is valid")
    }
}

// ---------------------------------------------------------------------------------------
// Runtime graph
// ---------------------------------------------------------------------------------------

/// Mirrors the `ASMR_OBJECT_*` constants in asmr.h.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ObjectKind {
    Tree = 0,
    River = 1,
    Bird = 2,
    /// A generic physical source (`EntityDesc::Source`).
    Source = 3,
}

pub mod object_flags {
    /// The user may drag it (`asmr_set_object_position`).
    pub const MOVABLE: u32 = 1 << 0;
    /// Currently moving on its own (flying).
    pub const MOVING: u32 = 1 << 1;
    /// Off stage: exists in the scene but is not audible / should not be drawn.
    pub const ABSENT: u32 = 1 << 2;
}

/// Mirrors `AsmrObjectInfo` in asmr.h.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ObjectInfo {
    pub id: u32,
    pub kind: u32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub flags: u32,
}

#[derive(Clone, Debug)]
pub enum Geometry {
    Point,
    Tree { canopy_height: f32, canopy_radius: f32 },
    /// Polyline relative to the entity position (its centroid at load time).
    River { path: Vec<Vec3>, flow: f32 },
    /// A physical model source; `path` (relative to the entity) for extended ones.
    Source { model: ModelKind, path: Option<Vec<Vec3>> },
    /// Sound files moved by the choreography (clips.rs).
    Clip { bank: std::sync::Arc<crate::clips::ClipBank> },
}

pub struct Entity {
    pub id: u32,
    pub kind: ObjectKind,
    pub position: Vec3,
    pub geometry: Geometry,
    pub flags: u32,
    /// Indices into `World::emitters`.
    pub emitters: core::ops::Range<usize>,
}

impl Entity {
    /// Centre of the canopy for trees, the position otherwise.
    pub fn canopy(&self) -> Vec3 {
        match self.geometry {
            Geometry::Tree { canopy_height, .. } => self.position + Vec3::new(0.0, 0.0, canopy_height),
            _ => self.position,
        }
    }
}

pub struct Emitter {
    pub owner: usize,
    pub kind: SourceKind,
    /// Position relative to the owner.
    pub offset: Vec3,
    /// World position, refreshed by `World::update_emitters`.
    pub position: Vec3,
    pub generator: Box<dyn Generator>,
    /// Behavioural controls written by the director (activity, calling, flap).
    pub controls: Controls,
    /// Scale of the WATER drive for river emitters.
    pub flow: f32,
    /// Physical model configuration as built (runtime controls are applied relative to it).
    pub base: Option<ModelConfig>,
    /// Per-segment variation of an extended water source: depth ×, rock size ×, turbulence +.
    pub variation: [f32; 3],
    /// Telemetry: recent RMS and head-relative position.
    pub level: f32,
    pub rel: Vec3,
    /// Calibration and attention class for the mix layer (mix/calibration.rs).
    pub class: SoundClass,
    /// Underwater muffling filter state (two one-pole stages).
    pub muffle: [f32; 2],
    /// A physical model inside a clip: the choreography's envelope (`controls.clip_env`)
    /// scales its output; `env` is the last applied value (per-sample ramp).
    pub enveloped: bool,
    pub env: f32,
}

pub struct World {
    pub entities: Vec<Entity>,
    pub emitters: Vec<Emitter>,
    pub ground_z: f32,
    /// Underwater: low-pass cutoff for the physical models, Hz (0 = in air, no muffling).
    pub muffle_hz: f32,
}

fn v3(a: [f32; 3]) -> Vec3 {
    Vec3::new(a[0], a[1], a[2])
}

fn finite(a: &[f32]) -> bool {
    a.iter().all(|v| v.is_finite())
}

impl World {
    /// Validates `desc` and instantiates every entity and its emitters. Not real-time safe
    /// (allocates); call off the audio thread.
    pub fn build(desc: &SceneDesc, sample_rate: f32) -> Result<World, String> {
        let mut rng = Rng::new(desc.seed ^ 0xA5A5_3000);
        let mut entities = Vec::new();
        let mut emitters: Vec<Emitter> = Vec::new();
        let mut ids = Vec::new();

        let id_for = |explicit: Option<u32>, index: usize| explicit.unwrap_or(1000 + index as u32);
        for (i, e) in desc.entities.iter().enumerate() {
            let id = match e {
                EntityDesc::Tree(t) => id_for(t.id, i),
                EntityDesc::River(r) => id_for(r.id, i),
                EntityDesc::Bird(b) => id_for(b.id, i),
                EntityDesc::Source(s) => id_for(s.id, i),
                EntityDesc::Clip(c) => id_for(c.id, i),
            };
            if ids.contains(&id) {
                return Err(format!("entity {i}: duplicate id {id}"));
            }
            ids.push(id);
        }

        desc.environment.validate()?;
        for (i, e) in desc.entities.iter().enumerate() {
            let id = ids[i];
            let first = emitters.len();
            let push = |emitters: &mut Vec<Emitter>, kind, offset, generator, flow| {
                emitters.push(Emitter {
                    owner: i,
                    kind,
                    offset,
                    position: Vec3::default(),
                    generator,
                    controls: Controls::default(),
                    flow,
                    base: None,
                    variation: [1.0, 1.0, 0.0],
                    level: 0.0,
                    rel: Vec3::default(),
                    class: SoundClass::of_kind(kind),
                    muffle: [0.0; 2],
                    enveloped: false,
                    env: 0.0,
                });
            };
            let set_base = |emitters: &mut Vec<Emitter>, cfg: ModelConfig| {
                if let Some(e) = emitters.last_mut() {
                    e.base = Some(cfg);
                    e.class = SoundClass::of_model(cfg.kind());
                }
            };
            let entity = match e {
                EntityDesc::Tree(t) => {
                    if !finite(&t.position) || !finite(&[t.canopy_height, t.canopy_radius]) {
                        return Err(format!("tree {id}: non-finite geometry"));
                    }
                    let r = t.canopy_radius.max(0.2);
                    let crown = core::f32::consts::PI * r * r;
                    let foliage = t.foliage.clone().unwrap_or_else(|| SoundDesc::new(ModelKind::Foliage, t.species.foliage_preset()));
                    let foliage = expect_model(foliage, ModelKind::Foliage, id)?;
                    let foliage = default_param(foliage, "area_m2", crown).resolve().map_err(|e| format!("tree {id}: {e}"))?;
                    push(&mut emitters, SourceKind::Wind, Vec3::new(0.0, 0.0, t.canopy_height), foliage.build(sample_rate, rng.next_u64()), 1.0);
                    set_base(&mut emitters, foliage);
                    let under = if t.species.is_conifer() { "under_conifer" } else { "under_canopy" };
                    let ground = t.ground.clone().unwrap_or_else(|| SoundDesc::new(ModelKind::Rain, under));
                    let ground = expect_model(ground, ModelKind::Rain, id)?;
                    let ground = default_param(default_param(ground, "area_m2", crown), "drip_height_m", t.canopy_height.clamp(0.5, 40.0));
                    let ground = ground.resolve().map_err(|e| format!("tree {id} ground: {e}"))?;
                    push(&mut emitters, SourceKind::Rain, Vec3::new(0.0, 0.0, 0.1), ground.build(sample_rate, rng.next_u64()), 1.0);
                    set_base(&mut emitters, ground);
                    Entity {
                        id,
                        kind: ObjectKind::Tree,
                        position: v3(t.position),
                        geometry: Geometry::Tree { canopy_height: t.canopy_height, canopy_radius: t.canopy_radius.max(0.2) },
                        flags: object_flags::MOVABLE,
                        emitters: 0..0,
                    }
                }
                EntityDesc::River(r) => {
                    if r.path.len() < 2 || !r.path.iter().all(|p| finite(p)) {
                        return Err(format!("river {id}: path needs at least two finite points"));
                    }
                    if r.segments == 0 || r.segments > 16 {
                        return Err(format!("river {id}: segments must be 1..16"));
                    }
                    let pts: Vec<Vec3> = r.path.iter().map(|p| v3(*p)).collect();
                    let centroid = pts.iter().fold(Vec3::default(), |a, p| a + *p) * (1.0 / pts.len() as f32);
                    let rel: Vec<Vec3> = pts.iter().map(|p| *p - centroid).collect();
                    let length: f32 = rel.windows(2).map(|w| (w[1] - w[0]).length()).sum();
                    let sound = r.sound.clone().unwrap_or_else(|| SoundDesc::new(ModelKind::Water, "brook_pebbles"));
                    let sound = expect_model(sound, ModelKind::Water, id)?;
                    let sound = default_param(sound, "length_m", (length / r.segments as f32).clamp(0.1, 100.0));
                    let mut cfg = match sound.resolve().map_err(|e| format!("river {id}: {e}"))? {
                        ModelConfig::Water(c) => c,
                        _ => unreachable!(),
                    };
                    cfg.aeration = (cfg.aeration * r.bubble_rate.max(0.0) / 350.0).min(1.5);
                    // Riffles and pools alternate along a stream (spacing ~5–7 widths): shallow,
                    // coarse riffles babble, deep pools are quiet. Deterministic per seed.
                    let phase = rng.range(0.0, core::f32::consts::TAU);
                    let spacing = (6.0 * cfg.width_m).max(2.0);
                    for k in 0..r.segments {
                        let u = (k as f32 + 0.5) / r.segments as f32;
                        let at = sample_polyline(&rel, u);
                        let s = r.riffles.clamp(0.0, 1.0) * ((core::f32::consts::TAU * u * length / spacing + phase).sin() + 0.4 * rng.gauss());
                        let variation = [(0.2 * s).exp(), (-0.15 * s + 0.1 * rng.gauss()).exp(), -0.05 * s];
                        let seg = vary_water(&cfg, variation);
                        push(&mut emitters, SourceKind::Water, at, ModelConfig::Water(seg).build(sample_rate, rng.next_u64()), r.flow.clamp(0.0, 2.0));
                        set_base(&mut emitters, ModelConfig::Water(seg));
                        emitters.last_mut().unwrap().variation = variation;
                    }
                    Entity {
                        id,
                        kind: ObjectKind::River,
                        position: centroid,
                        geometry: Geometry::River { path: rel, flow: r.flow },
                        flags: object_flags::MOVABLE,
                        emitters: 0..0,
                    }
                }
                EntityDesc::Bird(b) => {
                    let def = b.species.def();
                    let pitch = b.pitch.unwrap_or_else(|| Bird::default_pitch(def.voice) * rng.range(0.93, 1.07));
                    let procedural = Bird::new(sample_rate, rng.next_u64(), def.voice, pitch, b.tempo.clamp(0.3, 3.0));
                    let voice = crate::director::bird_voice(b.species, procedural, sample_rate, rng.next_u64());
                    let wings = Wings::new(sample_rate, rng.next_u64(), def.wings);
                    push(&mut emitters, SourceKind::Bird, Vec3::default(), Box::new(Flyer::new(voice, wings, b.loudness.clamp(0.0, 8.0))), 1.0);
                    emitters.last_mut().unwrap().class = SoundClass::Bird(b.species);
                    Entity {
                        id,
                        kind: ObjectKind::Bird,
                        position: Vec3::new(0.0, 0.0, 30.0),
                        geometry: Geometry::Point,
                        flags: 0,
                        emitters: 0..0,
                    }
                }
                EntityDesc::Source(src) => {
                    let model = src.sound.model;
                    let cfg = src.sound.resolve().map_err(|e| format!("source {id}: {e}"))?;
                    let kind = model.source_kind();
                    let (position, path) = match (&src.path, src.position) {
                        (Some(path), _) => {
                            if path.len() < 2 || !path.iter().all(|p| finite(p)) {
                                return Err(format!("source {id}: path needs at least two finite points"));
                            }
                            if src.segments == 0 || src.segments > 16 {
                                return Err(format!("source {id}: segments must be 1..16"));
                            }
                            let pts: Vec<Vec3> = path.iter().map(|p| v3(*p)).collect();
                            let centroid = pts.iter().fold(Vec3::default(), |a, p| a + *p) * (1.0 / pts.len() as f32);
                            (centroid, Some(pts.iter().map(|p| *p - centroid).collect::<Vec<_>>()))
                        }
                        (None, Some(p)) if finite(&p) => (v3(p), None),
                        _ => return Err(format!("source {id}: needs a finite position or a path")),
                    };
                    match &path {
                        Some(rel) => {
                            for k in 0..src.segments {
                                let at = sample_polyline(rel, (k as f32 + 0.5) / src.segments as f32);
                                push(&mut emitters, kind, at, cfg.build(sample_rate, rng.next_u64()), 1.0);
                                set_base(&mut emitters, cfg);
                            }
                        }
                        None => {
                            push(&mut emitters, kind, Vec3::default(), cfg.build(sample_rate, rng.next_u64()), 1.0);
                            set_base(&mut emitters, cfg);
                        }
                    }
                    // The director moves the thunder proxy; everything else can be dragged.
                    let flags = if model == ModelKind::Thunder { 0 } else { object_flags::MOVABLE };
                    Entity { id, kind: ObjectKind::Source, position, geometry: Geometry::Source { model, path }, flags, emitters: 0..0 }
                }
                EntityDesc::Clip(c) => {
                    c.validate(id)?;
                    let bank = crate::clips::ClipBank::load(&c.files, desc.assets.as_deref(), sample_rate).map_err(|e| format!("clip {id}: {e}"))?;
                    let bank = std::sync::Arc::new(bank);
                    if let Some(sound) = &c.sound {
                        // A physical model moved by the choreography; its level is the model's own.
                        let cfg = sound.resolve().map_err(|e| format!("clip {id}: {e}"))?;
                        for _ in 0..c.emitters {
                            push(&mut emitters, cfg.kind().source_kind(), Vec3::default(), cfg.build(sample_rate, rng.next_u64()), 1.0);
                            set_base(&mut emitters, cfg);
                            emitters.last_mut().unwrap().enveloped = true;
                        }
                    } else {
                        let (kind, class) = c.class.kinds();
                        // Files are at unit loudness; `level_db` is the pressure at 1 m, and the
                        // mix multiplies every emitter by its class calibration.
                        let gain = 10f32.powf((c.level_db - crate::mix::calibration::PA_DB) / 20.0) / class.gain();
                        let looped = c.playback == crate::clips::Playback::Loop;
                        for _ in 0..c.emitters {
                            let player = crate::clips::ClipPlayer::new(bank.clone(), looped, gain, rng.next_u64()).with_echo(c.echo);
                            push(&mut emitters, kind, Vec3::default(), Box::new(player), 1.0);
                            emitters.last_mut().unwrap().class = class;
                        }
                    }
                    let flags = if matches!(c.motion, crate::clips::Motion::Fixed { .. }) { 0 } else { object_flags::ABSENT };
                    Entity { id, kind: ObjectKind::Source, position: Vec3::default(), geometry: Geometry::Clip { bank }, flags, emitters: 0..0 }
                }
            };
            entities.push(Entity { emitters: first..emitters.len(), ..entity });
        }

        // Cross references.
        for e in &desc.entities {
            if let EntityDesc::Bird(b) = e {
                let home = match b.behaviour {
                    BehaviourDesc::Resident { home, .. } => Some(home),
                    BehaviourDesc::Circle { home, .. } => home,
                    _ => None,
                };
                if let Some(h) = home {
                    let ok = entities.iter().any(|t| t.id == h && t.kind == ObjectKind::Tree);
                    if !ok {
                        return Err(format!("bird {:?}: home {h} is not a tree", b.id));
                    }
                }
            }
        }
        if entities.len() > MAX_OBJECTS {
            return Err(format!("too many objects ({} > {MAX_OBJECTS})", entities.len()));
        }
        if emitters.len() > MAX_SOURCES {
            return Err(format!("too many emitters ({} > {MAX_SOURCES})", emitters.len()));
        }

        let muffle_hz = match desc.environment.medium {
            Medium::Air => 0.0,
            Medium::Water => desc.environment.muffle_hz.unwrap_or(900.0).clamp(100.0, 8000.0),
        };
        let mut world = World { entities, emitters, ground_z: desc.environment.ground_z, muffle_hz };
        world.update_emitters();
        Ok(world)
    }

    pub fn index_of(&self, id: u32) -> Option<usize> {
        self.entities.iter().position(|e| e.id == id)
    }

    /// User move. Ignored for objects that are not movable.
    pub fn set_position(&mut self, index: usize, p: Vec3) {
        if let Some(e) = self.entities.get_mut(index) {
            if e.flags & object_flags::MOVABLE != 0 && finite(&[p.x, p.y, p.z]) {
                e.position = p;
            }
        }
    }

    /// Derives emitter world positions from their owners. Real-time safe.
    pub fn update_emitters(&mut self) {
        for em in self.emitters.iter_mut() {
            em.position = self.entities[em.owner].position + em.offset;
        }
    }

    pub fn object_info(&self, index: usize) -> ObjectInfo {
        let e = &self.entities[index];
        ObjectInfo { id: e.id, kind: e.kind as u32, x: e.position.x, y: e.position.y, z: e.position.z, flags: e.flags }
    }
}

/// A water configuration with a segment's variation applied.
pub fn vary_water(c: &crate::sources::WaterConfig, v: [f32; 3]) -> crate::sources::WaterConfig {
    let mut c = *c;
    c.depth_m = (c.depth_m * v[0]).clamp(0.01, 5.0);
    c.rock_d50_m = (c.rock_d50_m * v[1]).clamp(0.002, 2.0);
    c.turbulence = (c.turbulence + v[2]).clamp(0.0, 1.0);
    c
}

fn expect_model(d: SoundDesc, want: ModelKind, id: u32) -> Result<SoundDesc, String> {
    if d.model == want {
        Ok(d)
    } else {
        Err(format!("entity {id}: sound model must be `{}`, got `{}`", want.name(), d.model.name()))
    }
}

/// Sets a parameter from the geometry unless the description already gives it.
fn default_param(mut d: SoundDesc, key: &str, value: f32) -> SoundDesc {
    if !d.params.contains_key(key) {
        d.params.insert(key.to_string(), serde_json::Value::from(value as f64));
    }
    d
}

/// Point at fraction `u` (0..1) of the polyline's arc length.
fn sample_polyline(pts: &[Vec3], u: f32) -> Vec3 {
    let total: f32 = pts.windows(2).map(|w| (w[1] - w[0]).length()).sum();
    let mut want = u.clamp(0.0, 1.0) * total;
    for w in pts.windows(2) {
        let l = (w[1] - w[0]).length();
        if want <= l && l > 0.0 {
            return w[0] + (w[1] - w[0]) * (want / l);
        }
        want -= l;
    }
    *pts.last().unwrap()
}

// ---------------------------------------------------------------------------------------
// Thread bridge: object telemetry out, user moves in. Lock-free, fixed size.
// ---------------------------------------------------------------------------------------

pub struct ObjectBridge {
    ids: Box<[u32]>,
    kinds: Box<[u32]>,
    pos: Box<[AtomicF32]>,
    flags: Box<[AtomicU32]>,
    moves: Box<[AtomicF32]>,
    /// Per object: 1 while a user move waits to be applied by the audio thread.
    pending: Box<[AtomicU32]>,
}

impl ObjectBridge {
    pub fn new(world: &World) -> Self {
        let n = world.entities.len();
        let f = |k: usize| (0..k).map(|_| AtomicF32::new(0.0)).collect::<Box<[_]>>();
        let u = |k: usize| (0..k).map(|_| AtomicU32::new(0)).collect::<Box<[_]>>();
        let bridge = Self {
            ids: world.entities.iter().map(|e| e.id).collect(),
            kinds: world.entities.iter().map(|e| e.kind as u32).collect(),
            pos: f(n * 3),
            flags: u(n),
            moves: f(n * 3),
            pending: u(n),
        };
        bridge.publish(world);
        bridge
    }

    /// Any thread.
    pub fn request_move(&self, id: u32, p: Vec3) {
        if !finite(&[p.x, p.y, p.z]) {
            return;
        }
        if let Some(i) = self.ids.iter().position(|&x| x == id) {
            self.moves[i * 3].store(p.x);
            self.moves[i * 3 + 1].store(p.y);
            self.moves[i * 3 + 2].store(p.z);
            self.pending[i].store(1, Ordering::Release);
        }
    }

    /// Audio thread.
    pub fn apply_moves(&self, world: &mut World) {
        for (i, flag) in self.pending.iter().enumerate() {
            if flag.load(Ordering::Relaxed) != 0 && flag.swap(0, Ordering::Acquire) != 0 {
                let p = Vec3::new(self.moves[i * 3].load(), self.moves[i * 3 + 1].load(), self.moves[i * 3 + 2].load());
                world.set_position(i, p);
            }
        }
    }

    /// Audio thread.
    pub fn publish(&self, world: &World) {
        for (i, e) in world.entities.iter().enumerate() {
            self.pos[i * 3].store(e.position.x);
            self.pos[i * 3 + 1].store(e.position.y);
            self.pos[i * 3 + 2].store(e.position.z);
            self.flags[i].store(e.flags, Ordering::Release);
        }
    }

    /// Any thread.
    pub fn read(&self, out: &mut [ObjectInfo]) -> usize {
        let n = self.ids.len().min(out.len());
        for (i, o) in out.iter_mut().take(n).enumerate() {
            *o = ObjectInfo {
                id: self.ids[i],
                kind: self.kinds[i],
                flags: self.flags[i].load(Ordering::Acquire),
                x: self.pos[i * 3].load(),
                y: self.pos[i * 3 + 1].load(),
                z: self.pos[i * 3 + 2].load(),
            };
        }
        n
    }

    /// Control snapshots include moves queued while audio is stopped.
    pub(crate) fn read_requested(&self, out: &mut [ObjectInfo]) -> usize {
        let n = self.read(out);
        for (i, object) in out[..n].iter_mut().enumerate() {
            if object.flags & object_flags::MOVABLE != 0 && self.pending[i].load(Ordering::Acquire) != 0 {
                object.x = self.moves[i * 3].load();
                object.y = self.moves[i * 3 + 1].load();
                object.z = self.moves[i * 3 + 2].load();
            }
        }
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_scenes_build() {
        for json in [DEFAULT_SCENE_JSON, DEMO_SCENE_JSON] {
            let desc = SceneDesc::from_json(json).unwrap();
            let world = World::build(&desc, 48_000.0).unwrap();
            assert!(world.entities.iter().any(|e| e.kind == ObjectKind::Tree));
            assert!(world.entities.iter().any(|e| e.kind == ObjectKind::Bird));
            // Round trip through JSON keeps the scene.
            let again = SceneDesc::from_json(&desc.to_json()).unwrap();
            assert_eq!(again.entities.len(), desc.entities.len());
        }
    }

    #[test]
    fn validation_rejects_bad_scenes() {
        let bad = [
            r#"{"name":"x","entities":[{"kind":"tree","id":1,"position":[0,0,0]},{"kind":"tree","id":1,"position":[1,0,0]}]}"#,
            r#"{"name":"x","entities":[{"kind":"river","path":[[0,0,0]]}]}"#,
            r#"{"name":"x","entities":[{"kind":"bird","species":"robin","behaviour":{"type":"resident","home":7}}]}"#,
            r#"{"name":"x","entities":[{"kind":"tree","position":[0,0,0],"colour":"green"}]}"#,
            r#"{"name":"x","entities":[{"kind":"bird","species":"dodo","behaviour":{"type":"flyby"}}]}"#,
        ];
        for json in bad {
            let r = SceneDesc::from_json(json).and_then(|d| World::build(&d, 48_000.0).map(|_| ()));
            assert!(r.is_err(), "accepted: {json}");
        }
    }

    #[test]
    fn physical_sources_validate() {
        let bad = [
            // Source without position or path.
            r#"{"name":"x","entities":[{"kind":"source","sound":{"model":"fire"}}]}"#,
            // Unknown preset, unknown parameter, out-of-range parameter.
            r#"{"name":"x","entities":[{"kind":"source","position":[0,0,0],"sound":{"model":"water","preset":"jacuzzi"}}]}"#,
            r#"{"name":"x","entities":[{"kind":"source","position":[0,0,0],"sound":{"model":"rain","params":{"colour":1}}}]}"#,
            r#"{"name":"x","entities":[{"kind":"source","position":[0,0,0],"sound":{"model":"insects","params":{"count":99}}}]}"#,
            // A tree's crown must be a foliage model.
            r#"{"name":"x","entities":[{"kind":"tree","position":[0,0,0],"foliage":{"model":"rain"}}]}"#,
            // Environment ranges.
            r#"{"name":"x","environment":{"humidity":2.0},"entities":[]}"#,
        ];
        for json in bad {
            let r = SceneDesc::from_json(json).and_then(|d| World::build(&d, 48_000.0).map(|_| ()));
            assert!(r.is_err(), "accepted: {json}");
        }
        let ok = r#"{"name":"x","environment":{"rain_mm_h":3,"temperature_c":12,"wind_ms":6,"wetness":1},
            "entities":[{"kind":"tree","position":[3,0,-1.7],"species":"aspen","foliage":{"model":"foliage","preset":"aspen","params":{"exposure":1.5}}},
                        {"kind":"river","path":[[4,-5,-1.5],[4,5,-1.5]],"sound":{"model":"water","preset":"rapids"}},
                        {"kind":"source","path":[[-3,-2,-1.7],[-3,2,-1.7]],"segments":2,"sound":{"model":"water","preset":"lake_lapping"}},
                        {"kind":"source","position":[0,-3,-1.5],"sound":{"model":"fire","preset":"campfire","params":{"moisture":0.3}}}]}"#;
        let desc = SceneDesc::from_json(ok).unwrap();
        let world = World::build(&desc, 48_000.0).unwrap();
        // Tree: crown + ground; river: 5 segments; lake: 2; fire: 1.
        assert_eq!(world.emitters.len(), 2 + 5 + 2 + 1);
        assert!((desc.environment.initial_wind().unwrap() - crate::sources::common::wind_slider(6.0)).abs() < 1e-6);
    }

    #[test]
    fn moving_a_river_translates_its_segments() {
        let desc = SceneDesc::default_scene();
        let mut world = World::build(&desc, 48_000.0).unwrap();
        let river = world.entities.iter().position(|e| e.kind == ObjectKind::River).unwrap();
        let before: Vec<Vec3> = world.entities[river].emitters.clone().map(|i| world.emitters[i].position).collect();
        let p = world.entities[river].position + Vec3::new(2.0, -1.0, 0.0);
        world.set_position(river, p);
        world.update_emitters();
        for (k, i) in world.entities[river].emitters.clone().enumerate() {
            let d = world.emitters[i].position - before[k];
            assert!((d - Vec3::new(2.0, -1.0, 0.0)).length() < 1e-4);
        }
    }
}
