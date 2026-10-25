//! Generic controls: every multi-parameter model exposed as a tree of sliders.
//!
//! Each sound group has one **macro** (water flow, wind speed, rain rate, …) and **children**
//! that are physical parameters of the models (depth, rock size, gustiness, …). Moving a
//! macro moves its children along a physical curve (for water: Leopold–Maddock hydraulic
//! geometry) unless the user has set a child explicitly; setting a child to NaN hands it back
//! to its macro. Presets are discrete children (index 0 = "as in the scene").
//!
//! Macros that existed as plain parameters (WATER, WIND, RAIN, STORM, BIRDS, ORBIT, VOLUME,
//! TIME_OF_DAY, TEMPERATURE, HUMIDITY) are stored *in* those parameters, so the old ABI and
//! the controls stay one source of truth. Lock-free: atomics only.
//!
//! The audio thread takes a [`ControlState`] snapshot every block and, when it changed,
//! reconfigures the generators with [`apply`] (copies plain config structs, no allocation).

use std::sync::atomic::{AtomicU32, Ordering};

use crate::engine::{param, AtomicF32};
use crate::scene::{vary_water, ObjectKind, SceneDesc, Season, World};
use crate::sources::common::{smoothstep, wind_slider, wind_speed_ms};
use crate::sources::{FireConfig, Fire, ModelConfig, Rain, WaterConfig, Water, Wind, WindConfig};

/// Mirrors `ASMR_GROUP_*`.
pub mod group {
    pub const WATER: u32 = 0;
    pub const WIND: u32 = 1;
    pub const FOLIAGE: u32 = 2;
    pub const RAIN: u32 = 3;
    pub const BIRDS: u32 = 4;
    pub const MASTER: u32 = 5;
    pub const ENVIRONMENT: u32 = 6;
    pub const INSECTS: u32 = 7;
    pub const FIRE: u32 = 8;
    // 9: reserved (formerly flight mode).
    /// Mix intent (the sound director, docs/sound-director.md).
    pub const MIX: u32 = 10;
    /// The world's own course (direction/, docs/direction.md): how far it goes and which way.
    pub const DIRECTION: u32 = 11;
}

/// Mirrors `ASMR_CONTROL_FLAG_*`.
pub mod flags {
    pub const MACRO: u32 = 1 << 0;
    pub const LOG: u32 = 1 << 1;
    pub const DISCRETE: u32 = 1 << 2;
    /// Child currently set by the user (does not follow its macro).
    pub const OVERRIDDEN: u32 = 1 << 3;
    /// Per-group loudness trim in dB (`<group>.level`): a pure gain on the group in the mix,
    /// after calibration and before the director's analysis. Never follows its macro; NaN
    /// resets it to 0 dB. −40 dB mutes.
    pub const LEVEL: u32 = 1 << 4;
}

pub const NO_PARENT: u32 = u32::MAX;

/// Mirrors `AsmrControlInfo` in asmr.h.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ControlInfo {
    pub id: u32,
    pub group: u32,
    pub parent: u32,
    pub flags: u32,
    pub min: f32,
    pub max: f32,
    pub value: f32,
    pub default_value: f32,
    pub key: [u8; 24],
    pub unit: [u8; 8],
}

impl Default for ControlInfo {
    fn default() -> Self {
        Self { id: 0, group: 0, parent: NO_PARENT, flags: 0, min: 0.0, max: 0.0, value: 0.0, default_value: 0.0, key: [0; 24], unit: [0; 8] }
    }
}

impl ControlInfo {
    pub fn key_str(&self) -> &str {
        let n = self.key.iter().position(|&b| b == 0).unwrap_or(self.key.len());
        core::str::from_utf8(&self.key[..n]).unwrap_or("")
    }
}

/// Control ids (stable across runs; also in asmr.h).
pub mod id {
    pub const WATER_FLOW: u32 = 100;
    pub const WATER_REGIME: u32 = 101;
    pub const WATER_SPEED: u32 = 102;
    pub const WATER_DEPTH: u32 = 103;
    pub const WATER_WIDTH: u32 = 104;
    pub const WATER_ROCK: u32 = 105;
    pub const WATER_AERATION: u32 = 106;
    pub const WATER_TURBULENCE: u32 = 107;
    pub const WATER_GAIN: u32 = 108;
    pub const WIND_SPEED: u32 = 200;
    pub const WIND_GUSTINESS: u32 = 201;
    pub const WIND_EXPOSURE: u32 = 202;
    pub const WIND_CHARACTER: u32 = 203;
    pub const WIND_WHISTLE: u32 = 204;
    pub const FOLIAGE_DENSITY: u32 = 300;
    pub const FOLIAGE_DRYNESS: u32 = 301;
    pub const FOLIAGE_FLUTTER: u32 = 302;
    pub const FOLIAGE_CREAK: u32 = 303;
    pub const FOLIAGE_EXPOSURE: u32 = 304;
    pub const RAIN_RATE: u32 = 400;
    pub const RAIN_STORM: u32 = 401;
    pub const RAIN_SURFACE: u32 = 402;
    pub const RAIN_DRYING: u32 = 403;
    pub const RAIN_GAIN: u32 = 404;
    pub const BIRDS_ACTIVITY: u32 = 500;
    pub const BIRDS_ORBIT: u32 = 501;
    pub const MASTER_VOLUME: u32 = 600;
    pub const ENV_TIME: u32 = 700;
    pub const ENV_TEMPERATURE: u32 = 701;
    pub const ENV_HUMIDITY: u32 = 702;
    pub const ENV_SEASON: u32 = 703;
    /// Where the wind blows from, degrees clockwise from north (270 = westerly, from the
    /// scene's left). World-fixed: 
    pub const ENV_WIND_DIRECTION: u32 = 704;
    pub const INSECTS_ACTIVITY: u32 = 800;
    pub const INSECTS_WARMTH: u32 = 801;
    pub const FIRE_HEAT: u32 = 900;
    pub const FIRE_MOISTURE: u32 = 901;
    pub const FIRE_RESIN: u32 = 902;
    pub const MIX_CLARITY: u32 = 1100;
    pub const MIX_DENSITY: u32 = 1101;
    pub const MIX_SOLOISTS: u32 = 1102;
    pub const MIX_LOUDNESS: u32 = 1103;
    pub const MIX_DYNAMICS: u32 = 1104;
    pub const MIX_FOCUS_WATER: u32 = 1105;
    pub const MIX_FOCUS_WIND: u32 = 1106;
    pub const MIX_FOCUS_BIRDS: u32 = 1107;
    pub const MIX_FOCUS_RAIN: u32 = 1108;
    pub const MIX_FOCUS_INSECTS: u32 = 1109;
    pub const MIX_FOCUS_FIRE: u32 = 1110;
    /// Attention follows the head: frontal cone lifted and clearer, rear a little lower and
    /// darker (0 = off).
    pub const MIX_FRONT_FOCUS: u32 = 1111;
    /// Perceptual distance: far sources recede (level, HF, wetter), near ones are present.
    pub const MIX_DISTANCE: u32 = 1112;
    /// Texture budget: how much the low-priority tail of broadband textures may add on top of
    /// the few protected texture streams (0 = cull hard, 1 = no limit).
    pub const MIX_TEXTURE_BUDGET: u32 = 1113;
    /// Stationary refocus: when the listener stops, a few constant sounds come forward and
    /// the rest steps back (0 = off).
    pub const MIX_REFOCUS: u32 = 1114;
    /// Direction (docs/direction.md): 0 = the scene stands still, 1 = it runs its full course.
    pub const DIRECTION_AMOUNT: u32 = 1200;
    /// −1 fair .. +1 wet and stormy.
    pub const DIRECTION_WEATHER: u32 = 1201;
    /// −1 still .. +1 lively (wind, birds, encounters).
    pub const DIRECTION_ENERGY: u32 = 1202;
    /// 0 steady .. 1 eventful (spells, their contrast and texture).
    pub const DIRECTION_CHANGE: u32 = 1203;
    /// Scene hours per real hour (0 = the clock stands, 1 = real time).
    pub const DIRECTION_CLOCK: u32 = 1204;
    /// Minutes a value set by hand is left to the user before the direction takes it back.
    pub const DIRECTION_HOLD: u32 = 1205;
    /// Ask for a spell (an impulse: the engine sets it back to "none" once taken).
    pub const DIRECTION_CUE: u32 = 1206;
    /// Per-group loudness trims, dB (flags::LEVEL), `<group>.level`: id = group base + 50.
    pub const WATER_LEVEL: u32 = 150;
    pub const WIND_LEVEL: u32 = 250;
    pub const FOLIAGE_LEVEL: u32 = 350;
    pub const RAIN_LEVEL: u32 = 450;
    pub const BIRDS_LEVEL: u32 = 550;
    pub const INSECTS_LEVEL: u32 = 850;
    pub const FIRE_LEVEL: u32 = 950;
}

#[derive(Clone, Copy, PartialEq)]
enum Store {
    /// Value lives in an engine parameter (with a unit conversion).
    Param(u32, Conv),
    Local,
}

#[derive(Clone, Copy, PartialEq)]
enum Conv {
    Same,
    /// Control in m/s, parameter in slider units.
    WindMs,
}

struct Def {
    id: u32,
    group: u32,
    parent: u32,
    flags: u32,
    min: f32,
    max: f32,
    key: &'static str,
    unit: &'static str,
    store: Store,
}

const fn def(id: u32, group: u32, parent: u32, flags: u32, min: f32, max: f32, key: &'static str, unit: &'static str, store: Store) -> Def {
    Def { id, group, parent, flags, min, max, key, unit, store }
}

use flags::{DISCRETE as D, LEVEL as LV, LOG as L, MACRO as M};
use group as g;
use id::*;

const N: usize = 66;
#[rustfmt::skip]
static DEFS: [Def; N] = [
    def(WATER_FLOW, g::WATER, NO_PARENT, M, 0.0, 1.0, "water.flow", "", Store::Param(param::WATER, Conv::Same)),
    def(WATER_REGIME, g::WATER, WATER_FLOW, D, 0.0, Water::PRESETS.len() as f32, "water.regime", "", Store::Local),
    def(WATER_SPEED, g::WATER, WATER_FLOW, 0, 0.0, 6.0, "water.speed", "m/s", Store::Local),
    def(WATER_DEPTH, g::WATER, WATER_FLOW, L, 0.01, 5.0, "water.depth", "m", Store::Local),
    def(WATER_WIDTH, g::WATER, WATER_FLOW, L, 0.05, 50.0, "water.width", "m", Store::Local),
    def(WATER_ROCK, g::WATER, WATER_FLOW, L, 0.002, 2.0, "water.rock_size", "m", Store::Local),
    def(WATER_AERATION, g::WATER, WATER_FLOW, 0, 0.0, 1.5, "water.aeration", "", Store::Local),
    def(WATER_TURBULENCE, g::WATER, WATER_FLOW, 0, 0.0, 1.0, "water.turbulence", "", Store::Local),
    def(WATER_GAIN, g::WATER, WATER_FLOW, 0, 0.0, 4.0, "water.gain", "", Store::Local),
    def(WIND_SPEED, g::WIND, NO_PARENT, M, 0.0, 30.0, "wind.speed", "m/s", Store::Param(param::WIND, Conv::WindMs)),
    def(WIND_GUSTINESS, g::WIND, WIND_SPEED, 0, 0.0, 2.0, "wind.gustiness", "", Store::Local),
    def(WIND_EXPOSURE, g::WIND, WIND_SPEED, 0, 0.0, 2.5, "wind.exposure", "", Store::Local),
    def(WIND_CHARACTER, g::WIND, WIND_SPEED, D, 0.0, Wind::PRESETS.len() as f32, "wind.character", "", Store::Local),
    def(WIND_WHISTLE, g::WIND, WIND_SPEED, 0, 0.0, 3.0, "wind.whistle", "", Store::Local),
    def(FOLIAGE_DENSITY, g::FOLIAGE, NO_PARENT, M, 0.0, 2.0, "foliage.density", "", Store::Local),
    def(FOLIAGE_DRYNESS, g::FOLIAGE, FOLIAGE_DENSITY, 0, -1.0, 1.0, "foliage.dryness", "", Store::Local),
    def(FOLIAGE_FLUTTER, g::FOLIAGE, FOLIAGE_DENSITY, 0, 0.0, 2.0, "foliage.flutter", "", Store::Local),
    def(FOLIAGE_CREAK, g::FOLIAGE, FOLIAGE_DENSITY, 0, 0.0, 3.0, "foliage.creak", "", Store::Local),
    def(FOLIAGE_EXPOSURE, g::FOLIAGE, FOLIAGE_DENSITY, 0, 0.0, 2.5, "foliage.exposure", "", Store::Local),
    def(RAIN_RATE, g::RAIN, NO_PARENT, M | L, 0.0, 100.0, "rain.rate", "mm/h", Store::Param(param::RAIN, Conv::Same)),
    def(RAIN_STORM, g::RAIN, RAIN_RATE, 0, 0.0, 1.0, "rain.storm", "", Store::Param(param::STORM, Conv::Same)),
    def(RAIN_SURFACE, g::RAIN, RAIN_RATE, D, 0.0, Rain::PRESETS.len() as f32, "rain.surface", "", Store::Local),
    def(RAIN_DRYING, g::RAIN, RAIN_RATE, L, 0.1, 200.0, "rain.drying", "x", Store::Local),
    def(RAIN_GAIN, g::RAIN, RAIN_RATE, 0, 0.0, 4.0, "rain.gain", "", Store::Local),
    def(BIRDS_ACTIVITY, g::BIRDS, NO_PARENT, M, 0.0, 1.0, "birds.activity", "", Store::Param(param::BIRDS, Conv::Same)),
    def(BIRDS_ORBIT, g::BIRDS, BIRDS_ACTIVITY, 0, 0.0, 1.0, "birds.orbit", "", Store::Param(param::ORBIT, Conv::Same)),
    def(MASTER_VOLUME, g::MASTER, NO_PARENT, M, 0.0, 1.5, "master.volume", "", Store::Param(param::VOLUME, Conv::Same)),
    def(ENV_TIME, g::ENVIRONMENT, NO_PARENT, M, 0.0, 24.0, "env.time_of_day", "h", Store::Param(param::TIME_OF_DAY, Conv::Same)),
    def(ENV_TEMPERATURE, g::ENVIRONMENT, ENV_TIME, 0, -20.0, 40.0, "env.temperature", "\u{b0}C", Store::Param(param::TEMPERATURE, Conv::Same)),
    def(ENV_HUMIDITY, g::ENVIRONMENT, ENV_TIME, 0, 0.0, 1.0, "env.humidity", "", Store::Param(param::HUMIDITY, Conv::Same)),
    def(ENV_SEASON, g::ENVIRONMENT, ENV_TIME, D, 0.0, 3.0, "env.season", "", Store::Local),
    def(ENV_WIND_DIRECTION, g::ENVIRONMENT, ENV_TIME, 0, 0.0, 360.0, "env.wind_direction", "\u{b0}", Store::Local),
    def(INSECTS_ACTIVITY, g::INSECTS, NO_PARENT, M, 0.0, 2.0, "insects.activity", "", Store::Local),
    def(INSECTS_WARMTH, g::INSECTS, INSECTS_ACTIVITY, 0, -10.0, 10.0, "insects.warmth", "\u{b0}C", Store::Local),
    def(FIRE_HEAT, g::FIRE, NO_PARENT, M | L, 1.0, 1000.0, "fire.heat", "kW", Store::Local),
    def(FIRE_MOISTURE, g::FIRE, FIRE_HEAT, 0, 0.02, 0.6, "fire.moisture", "", Store::Local),
    def(FIRE_RESIN, g::FIRE, FIRE_HEAT, 0, 0.0, 1.0, "fire.resin", "", Store::Local),
    def(MIX_CLARITY, g::MIX, NO_PARENT, M, 0.0, 1.0, "mix.clarity", "", Store::Local),
    def(MIX_DENSITY, g::MIX, MIX_CLARITY, 0, 0.0, 1.0, "mix.density", "", Store::Local),
    def(MIX_SOLOISTS, g::MIX, MIX_CLARITY, 0, 1.0, 4.0, "mix.soloists", "", Store::Local),
    def(MIX_LOUDNESS, g::MIX, MIX_CLARITY, 0, -36.0, -10.0, "mix.loudness", "LUFS", Store::Local),
    def(MIX_DYNAMICS, g::MIX, MIX_CLARITY, 0, 0.0, 1.0, "mix.dynamics", "", Store::Local),
    def(MIX_FOCUS_WATER, g::MIX, MIX_CLARITY, 0, -1.0, 1.0, "mix.focus.water", "", Store::Local),
    def(MIX_FOCUS_WIND, g::MIX, MIX_CLARITY, 0, -1.0, 1.0, "mix.focus.wind", "", Store::Local),
    def(MIX_FOCUS_BIRDS, g::MIX, MIX_CLARITY, 0, -1.0, 1.0, "mix.focus.birds", "", Store::Local),
    def(MIX_FOCUS_RAIN, g::MIX, MIX_CLARITY, 0, -1.0, 1.0, "mix.focus.rain", "", Store::Local),
    def(MIX_FOCUS_INSECTS, g::MIX, MIX_CLARITY, 0, -1.0, 1.0, "mix.focus.insects", "", Store::Local),
    def(MIX_FOCUS_FIRE, g::MIX, MIX_CLARITY, 0, -1.0, 1.0, "mix.focus.fire", "", Store::Local),
    def(MIX_FRONT_FOCUS, g::MIX, MIX_CLARITY, 0, 0.0, 1.0, "mix.front_focus", "", Store::Local),
    def(MIX_DISTANCE, g::MIX, MIX_CLARITY, 0, 0.0, 1.0, "mix.distance", "", Store::Local),
    def(MIX_TEXTURE_BUDGET, g::MIX, MIX_CLARITY, 0, 0.0, 1.0, "mix.texture_budget", "", Store::Local),
    def(MIX_REFOCUS, g::MIX, MIX_CLARITY, 0, 0.0, 1.0, "mix.refocus", "", Store::Local),
    def(WATER_LEVEL, g::WATER, WATER_FLOW, LV, -40.0, 12.0, "water.level", "dB", Store::Local),
    def(WIND_LEVEL, g::WIND, WIND_SPEED, LV, -40.0, 12.0, "wind.level", "dB", Store::Local),
    def(FOLIAGE_LEVEL, g::FOLIAGE, FOLIAGE_DENSITY, LV, -40.0, 12.0, "foliage.level", "dB", Store::Local),
    def(RAIN_LEVEL, g::RAIN, RAIN_RATE, LV, -40.0, 12.0, "rain.level", "dB", Store::Local),
    def(BIRDS_LEVEL, g::BIRDS, BIRDS_ACTIVITY, LV, -40.0, 12.0, "birds.level", "dB", Store::Local),
    def(INSECTS_LEVEL, g::INSECTS, INSECTS_ACTIVITY, LV, -40.0, 12.0, "insects.level", "dB", Store::Local),
    def(FIRE_LEVEL, g::FIRE, FIRE_HEAT, LV, -40.0, 12.0, "fire.level", "dB", Store::Local),
    def(DIRECTION_AMOUNT, g::DIRECTION, NO_PARENT, M, 0.0, 1.0, "direction.amount", "", Store::Local),
    def(DIRECTION_WEATHER, g::DIRECTION, DIRECTION_AMOUNT, 0, -1.0, 1.0, "direction.weather", "", Store::Local),
    def(DIRECTION_ENERGY, g::DIRECTION, DIRECTION_AMOUNT, 0, -1.0, 1.0, "direction.energy", "", Store::Local),
    def(DIRECTION_CHANGE, g::DIRECTION, DIRECTION_AMOUNT, 0, 0.0, 1.0, "direction.change", "", Store::Local),
    def(DIRECTION_CLOCK, g::DIRECTION, DIRECTION_AMOUNT, L, 0.0, 120.0, "direction.clock", "x", Store::Local),
    def(DIRECTION_HOLD, g::DIRECTION, DIRECTION_AMOUNT, L, 0.5, 240.0, "direction.hold", "min", Store::Local),
    def(DIRECTION_CUE, g::DIRECTION, DIRECTION_AMOUNT, D, 0.0, 33.0, "direction.cue", "", Store::Local),
];

/// Options of `direction.cue`: nothing, wrap up and move on, or a built-in spell.
pub const CUES: [&str; 13] = {
    let b = crate::direction::BUILTIN_NAMES;
    ["none", "next", b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7], b[8], b[9], b[10]]
};

const SEASONS: [&str; 4] = ["spring", "summer", "autumn", "winter"];

fn index(id: u32) -> Option<usize> {
    DEFS.iter().position(|d| d.id == id)
}

/// Option names of a discrete control (index 0 = "scene").
pub fn option_name(id: u32, i: u32) -> Option<&'static str> {
    let i = i as usize;
    let list: &[&str] = match id {
        WATER_REGIME => Water::PRESETS,
        WIND_CHARACTER => Wind::PRESETS,
        RAIN_SURFACE => Rain::PRESETS,
        ENV_SEASON => return SEASONS.get(i).copied(),
        DIRECTION_CUE => return CUES.get(i).copied(),
        _ => return None,
    };
    if i == 0 {
        Some("scene")
    } else {
        list.get(i - 1).copied()
    }
}

/// What the scene was built with: the reference the children's curves start from.
#[derive(Clone, Copy, Debug)]
pub struct Bases {
    pub water: WaterConfig,
    pub wind: WindConfig,
    pub fire: FireConfig,
    pub temperature: f32,
    pub humidity: f32,
    pub hour: f32,
    pub season: u32,
    pub drying: f32,
    /// The scene's direction and its starting steer (None: the scene stands still).
    pub direction: Option<crate::direction::Steer>,
    /// Its episodes: `direction.cue` offers none, next and these.
    pub cues: usize,
    /// The scene's loudness target (mix.loudness), LUFS.
    pub loudness: f32,
}

impl Bases {
    pub fn new(desc: &SceneDesc, world: &World) -> Self {
        let first = |pick: &dyn Fn(&ModelConfig) -> bool| world.emitters.iter().filter_map(|e| e.base.as_ref()).find(|c| pick(c)).copied();
        let water = match first(&|c| matches!(c, ModelConfig::Water(_))) {
            Some(ModelConfig::Water(c)) => {
                // The unvaried reference of the first water emitter.
                let v = world.emitters.iter().find(|e| matches!(e.base, Some(ModelConfig::Water(_)))).map(|e| e.variation).unwrap_or([1.0, 1.0, 0.0]);
                WaterConfig { depth_m: c.depth_m / v[0], rock_d50_m: c.rock_d50_m / v[1], turbulence: (c.turbulence - v[2]).clamp(0.0, 1.0), ..c }
            }
            _ => Water::brook_pebbles(),
        };
        let wind = match first(&|c| matches!(c, ModelConfig::Wind(_))) {
            Some(ModelConfig::Wind(c)) => c,
            _ => Wind::breeze(),
        };
        let fire = match first(&|c| matches!(c, ModelConfig::Fire(_))) {
            Some(ModelConfig::Fire(c)) => c,
            _ => Fire::campfire(),
        };
        let e = &desc.environment;
        let season = match e.season {
            Season::Spring => 0,
            Season::Summer => 1,
            Season::Autumn => 2,
            Season::Winter => 3,
        };
        let direction = desc.direction.as_ref().map(crate::direction::Steer::from_desc);
        let cues = desc.direction.as_ref().and_then(|d| d.catalogue().ok()).map_or(CUES.len() - 2, |e| e.len());
        Self { water, wind, fire, temperature: e.temperature_c, humidity: e.humidity, hour: e.time_of_day, season, drying: e.weather_time_scale.clamp(0.1, 200.0), direction, cues, loudness: e.loudness_lufs.unwrap_or(-22.0).clamp(-36.0, -10.0) }
    }

    fn regime(&self, idx: f32) -> WaterConfig {
        let i = idx.round() as usize;
        if i == 0 {
            self.water
        } else {
            Water::PRESETS.get(i - 1).and_then(|p| Water::preset(p)).unwrap_or(self.water)
        }
    }
}

/// Discharge ratio for the WATER macro: 0.6 = as built, quadratic so the slider is even.
pub fn discharge_ratio(flow: f32) -> f32 {
    (flow.max(0.0) / 0.6).powi(2)
}

/// Density of bird phrases that follows the clarity macro.
fn mix_density(clarity: f32) -> f32 {
    (0.9 - 0.6 * clarity).clamp(0.0, 1.0)
}

/// Foreground soloists that follow the clarity macro: 3 for a loose mix, 2 for a clear one.
fn mix_soloists(clarity: f32) -> f32 {
    if clarity < 0.4 {
        3.0
    } else {
        2.0
    }
}

/// Diurnal cycle shape, 1 at 15:00, −1 at 03:00.
fn diurnal(hour: f32) -> f32 {
    (core::f32::consts::TAU * (hour - 15.0) / 24.0).cos()
}

/// The control values the audio thread acts on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ControlState {
    pub v: [f32; N],
}

impl Default for ControlState {
    fn default() -> Self {
        Self { v: [0.0; N] }
    }
}

impl ControlState {
    pub fn get(&self, id: u32) -> f32 {
        index(id).map_or(0.0, |i| self.v[i])
    }
}

/// Atomic store of local controls and override flags.
pub(crate) struct ControlBank {
    local: Box<[AtomicF32]>,
    overridden: Box<[AtomicU32]>,
    pub bases: Bases,
}

impl ControlBank {
    pub(crate) fn new(bases: Bases, desc: &SceneDesc) -> Self {
        let bank = Self {
            local: (0..N).map(|_| AtomicF32::new(0.0)).collect(),
            overridden: (0..N).map(|_| AtomicU32::new(0)).collect(),
            bases,
        };
        for (i, d) in DEFS.iter().enumerate() {
            if d.store == Store::Local {
                bank.local[i].store(bank.default_of(d.id));
            }
        }
        // A scene that states the storm sets it explicitly.
        if desc.environment.storm > 0.0 {
            bank.overridden[index(RAIN_STORM).unwrap()].store(1, Ordering::Relaxed);
        }
        bank
    }

    fn default_of(&self, id: u32) -> f32 {
        let b = &self.bases;
        match id {
            WATER_FLOW => 0.6,
            WATER_REGIME | WIND_CHARACTER | RAIN_SURFACE => 0.0,
            WATER_SPEED => b.water.flow_ms,
            WATER_DEPTH => b.water.depth_m,
            WATER_WIDTH => b.water.width_m,
            WATER_ROCK => b.water.rock_d50_m,
            WATER_AERATION => b.water.aeration,
            WATER_TURBULENCE => b.water.turbulence,
            WATER_GAIN | WIND_EXPOSURE | FOLIAGE_DENSITY | FOLIAGE_FLUTTER | FOLIAGE_CREAK | FOLIAGE_EXPOSURE | RAIN_GAIN | INSECTS_ACTIVITY => 1.0,
            WIND_SPEED => wind_speed_ms(0.45),
            WIND_GUSTINESS => 1.0,
            WIND_WHISTLE => b.wind.gap,
            FOLIAGE_DRYNESS => -1.0,
            RAIN_RATE | RAIN_STORM | INSECTS_WARMTH => 0.0,
            RAIN_DRYING => b.drying,
            BIRDS_ACTIVITY => 0.6,
            BIRDS_ORBIT => 0.0,
            MASTER_VOLUME => 0.7,
            ENV_TIME => b.hour,
            ENV_TEMPERATURE => b.temperature,
            ENV_HUMIDITY => b.humidity,
            ENV_SEASON => b.season as f32,
            ENV_WIND_DIRECTION => 270.0,
            FIRE_HEAT => b.fire.heat_kw,
            FIRE_MOISTURE => b.fire.moisture,
            FIRE_RESIN => b.fire.resin,
            MIX_CLARITY => 0.6,
            MIX_DENSITY => mix_density(0.6),
            MIX_SOLOISTS => mix_soloists(0.6),
            MIX_LOUDNESS => b.loudness,
            MIX_DYNAMICS => 0.7,
            MIX_FRONT_FOCUS => 0.5,
            MIX_DISTANCE => 0.6,
            MIX_TEXTURE_BUDGET => 0.35,
            MIX_REFOCUS => 0.5,
            DIRECTION_AMOUNT => b.direction.map_or(0.0, |d| d.amount),
            DIRECTION_WEATHER => b.direction.map_or(0.0, |d| d.weather),
            DIRECTION_ENERGY => b.direction.map_or(0.0, |d| d.energy),
            DIRECTION_CHANGE => b.direction.map_or(0.35, |d| d.change),
            DIRECTION_CLOCK => b.direction.map_or(1.0, |d| d.clock),
            DIRECTION_HOLD => b.direction.map_or(20.0, |d| d.hold_s / 60.0),
            // Level trims (flags::LEVEL) start at 0 dB.
            _ => 0.0,
        }
    }

    fn stored(&self, i: usize, params: &[AtomicF32]) -> f32 {
        match DEFS[i].store {
            Store::Param(p, Conv::Same) => params[p as usize].load(),
            Store::Param(p, Conv::WindMs) => wind_speed_ms(params[p as usize].load()),
            Store::Local => self.local[i].load(),
        }
    }

    fn is_overridden(&self, i: usize) -> bool {
        self.overridden[i].load(Ordering::Relaxed) != 0
    }

    /// Value of a child that follows its macro (`m` = the macro values snapshot).
    fn follow(&self, id: u32, get: &dyn Fn(u32) -> f32) -> Option<f32> {
        let b = &self.bases;
        let q = || discharge_ratio(get(WATER_FLOW));
        let reg = || b.regime(get(WATER_REGIME));
        Some(match id {
            // Leopold & Maddock (1953) at-a-station hydraulic geometry: as discharge Q grows,
            // velocity ∝ Q^0.34, depth ∝ Q^0.40, width ∝ Q^0.26.
            WATER_SPEED => reg().flow_ms * q().powf(0.34),
            WATER_DEPTH => (reg().depth_m * q().powf(0.40)).max(0.01),
            WATER_WIDTH => (reg().width_m * q().powf(0.26)).max(0.05),
            WATER_ROCK => reg().rock_d50_m,
            WATER_AERATION => reg().aeration,
            WATER_TURBULENCE => reg().turbulence,
            // Turbulence intensity falls slightly in strong wind (gust factor ~1.6 → ~1.4).
            WIND_GUSTINESS => 1.15 - 0.3 * smoothstep(2.0, 16.0, get(WIND_SPEED)),
            WIND_WHISTLE => {
                let i = get(WIND_CHARACTER).round() as usize;
                if i == 0 {
                    b.wind.gap
                } else {
                    Wind::PRESETS.get(i - 1).and_then(|p| Wind::preset(p)).map_or(b.wind.gap, |c| c.gap)
                }
            }
            // Heavy rain usually comes from convective cells.
            RAIN_STORM => 0.9 * smoothstep(10.0, 40.0, get(RAIN_RATE)),
            // Diurnal cycle around the scene's values (±5 °C, humidity opposite).
            ENV_TEMPERATURE => b.temperature + 5.0 * (diurnal(get(ENV_TIME)) - diurnal(b.hour)),
            ENV_HUMIDITY => (b.humidity - 0.15 * (diurnal(get(ENV_TIME)) - diurnal(b.hour))).clamp(0.0, 1.0),
            FIRE_MOISTURE => b.fire.moisture,
            FIRE_RESIN => b.fire.resin,
            // Clearer mixes allow fewer simultaneous phrases and soloists.
            MIX_DENSITY => mix_density(get(MIX_CLARITY)),
            MIX_SOLOISTS => mix_soloists(get(MIX_CLARITY)),
            _ => return None,
        })
    }

    fn value_at(&self, i: usize, params: &[AtomicF32], depth: u32) -> f32 {
        let d = &DEFS[i];
        if d.flags & flags::MACRO != 0 || self.is_overridden(i) || depth > 2 {
            return self.stored(i, params);
        }
        let get = |id: u32| index(id).map_or(0.0, |k| self.value_at(k, params, depth + 1));
        self.follow(d.id, &get).unwrap_or_else(|| self.stored(i, params))
    }

    /// Effective value of every control. Any thread.
    pub(crate) fn state(&self, params: &[AtomicF32]) -> ControlState {
        let mut s = ControlState::default();
        for i in 0..N {
            s.v[i] = self.value_at(i, params, 0);
        }
        s
    }

    /// Any thread. NaN on a child hands it back to its macro.
    pub(crate) fn set(&self, id: u32, value: f32, params: &[AtomicF32]) {
        let Some(i) = index(id) else { return };
        let d = &DEFS[i];
        if value.is_nan() {
            self.overridden[i].store(0, Ordering::Relaxed);
            // A level trim has no macro curve: "follow the macro" means back to 0 dB.
            if d.flags & flags::LEVEL != 0 {
                self.local[i].store(self.default_of(d.id));
            }
            return;
        }
        if !value.is_finite() {
            return;
        }
        let mut v = value.clamp(d.min, d.max);
        if d.flags & flags::DISCRETE != 0 {
            v = v.round();
        }
        match d.store {
            Store::Param(p, Conv::Same) => params[p as usize].store(v),
            Store::Param(p, Conv::WindMs) => params[p as usize].store(wind_slider(v)),
            Store::Local => self.local[i].store(v),
        }
        if d.flags & flags::MACRO == 0 {
            self.overridden[i].store(1, Ordering::Relaxed);
        }
    }

    /// The legacy parameter `p` was set directly: if it backs a child, that child is now
    /// explicit (e.g. `asmr_set_param(STORM, …)`).
    pub(crate) fn param_set(&self, p: u32) {
        for (i, d) in DEFS.iter().enumerate() {
            if d.store == Store::Param(p, Conv::Same) && d.flags & flags::MACRO == 0 {
                self.overridden[i].store(1, Ordering::Relaxed);
            }
        }
    }

    /// Writes follower values of parameter-backed children back into their parameters so
    /// `asmr_get_param` reports what is heard. Audio thread.
    pub(crate) fn publish_params(&self, st: &ControlState, params: &[AtomicF32]) {
        for (i, d) in DEFS.iter().enumerate() {
            if let Store::Param(p, Conv::Same) = d.store {
                if d.flags & flags::MACRO == 0 && !self.is_overridden(i) {
                    params[p as usize].store(st.v[i]);
                }
            }
        }
    }

    /// Fills `out` with the control tree. Any thread.
    pub(crate) fn read(&self, params: &[AtomicF32], out: &mut [ControlInfo]) -> usize {
        let st = self.state(params);
        let n = N.min(out.len());
        for (i, o) in out.iter_mut().take(n).enumerate() {
            let d = &DEFS[i];
            let mut key = [0u8; 24];
            let mut unit = [0u8; 8];
            key[..d.key.len().min(23)].copy_from_slice(&d.key.as_bytes()[..d.key.len().min(23)]);
            unit[..d.unit.len().min(7)].copy_from_slice(&d.unit.as_bytes()[..d.unit.len().min(7)]);
            let over = if self.is_overridden(i) { flags::OVERRIDDEN } else { 0 };
            *o = ControlInfo {
                id: d.id,
                group: d.group,
                parent: d.parent,
                flags: d.flags | over,
                min: d.min,
                max: if d.id == DIRECTION_CUE { (self.bases.cues + 1) as f32 } else { d.max },
                value: st.v[i],
                default_value: self.default_of(d.id),
                key,
                unit,
            };
        }
        n
    }

    // --- Prompt control (core/src/prompt.rs) ---

    /// Effective value of control `i` if it followed its macro (None for macros and
    /// children without a curve). Any thread.
    pub(crate) fn followed(&self, i: usize, params: &[AtomicF32]) -> Option<f32> {
        let d = &DEFS[i];
        if d.flags & flags::MACRO != 0 {
            return None;
        }
        let get = |id: u32| index(id).map_or(0.0, |k| self.value_at(k, params, 1));
        self.follow(d.id, &get)
    }

    pub(crate) fn overridden_at(&self, i: usize) -> bool {
        self.is_overridden(i)
    }

    pub(crate) fn default_at(&self, i: usize) -> f32 {
        self.default_of(DEFS[i].id)
    }
}

/// Static description of control `i` (0..COUNT), for the prompt layer.
#[derive(Clone, Copy, Debug)]
pub struct Meta {
    pub id: u32,
    pub group: u32,
    pub parent: u32,
    pub flags: u32,
    pub min: f32,
    pub max: f32,
    pub key: &'static str,
    pub unit: &'static str,
}

pub fn meta(i: usize) -> Meta {
    let d = &DEFS[i];
    Meta { id: d.id, group: d.group, parent: d.parent, flags: d.flags, min: d.min, max: d.max, key: d.key, unit: d.unit }
}

/// Index (0..COUNT) of the control with machine name `key`.
pub fn index_of_key(key: &str) -> Option<usize> {
    DEFS.iter().position(|d| d.key == key)
}

pub fn index_of_id(id: u32) -> Option<usize> {
    index(id)
}

/// Indices of controls stored in legacy parameter `p`.
pub fn indices_of_param(p: u32) -> impl Iterator<Item = usize> {
    DEFS.iter().enumerate().filter(move |(_, d)| matches!(d.store, Store::Param(q, _) if q == p)).map(|(i, _)| i)
}

/// All option names of a discrete control, index order.
pub fn options(id: u32) -> Vec<&'static str> {
    (0..).map_while(|i| option_name(id, i)).collect()
}

pub fn group_name(g: u32) -> &'static str {
    ["water", "wind", "foliage", "rain", "birds", "master", "environment", "insects", "fire", "reserved", "mix", "direction"].get(g as usize).copied().unwrap_or("other")
}

/// Number of controls.
pub const COUNT: usize = N;

/// Reconfigures every physical generator for the control state `st`. Audio thread: copies
/// plain config structs, no allocation.
pub(crate) fn apply(world: &mut World, st: &ControlState, bases: &Bases) {
    let get = |id: u32| st.get(id);
    let rep = bases.regime(get(WATER_REGIME));
    let regime = get(WATER_REGIME).round() as usize;
    let ratio = |eff: f32, base: f32| if base > 1e-6 { eff / base } else { 1.0 };
    let wind_char = get(WIND_CHARACTER).round() as usize;
    let surface = get(RAIN_SURFACE).round() as usize;
    for em in world.emitters.iter_mut() {
        let Some(base) = em.base else { continue };
        // Clips with a physical model are configured by their group (clips.rs).
        if matches!(world.entities[em.owner].geometry, crate::scene::Geometry::Clip { .. }) {
            continue;
        }
        let owner_is_source = world.entities[em.owner].kind == ObjectKind::Source;
        let cfg = match base {
            // No panel controls of its own yet: keeps the scene's configuration.
            ModelConfig::Whale(_) | ModelConfig::Dolphins(_) | ModelConfig::Reef(_) | ModelConfig::Sea(_) => base,
            ModelConfig::Water(own) => {
                let v = em.variation;
                // Preset regime replaces the scene's; geometry (length) stays.
                let own = if regime > 0 { WaterConfig { length_m: own.length_m, ..vary_water(&rep, v) } } else { own };
                let mut c = own;
                c.flow_ms = (own.flow_ms * ratio(get(WATER_SPEED), rep.flow_ms) * em.flow.sqrt()).clamp(0.0, 6.0);
                c.depth_m = (own.depth_m * ratio(get(WATER_DEPTH), rep.depth_m)).clamp(0.01, 5.0);
                c.width_m = (own.width_m * ratio(get(WATER_WIDTH), rep.width_m)).clamp(0.05, 50.0);
                c.rock_d50_m = (own.rock_d50_m * ratio(get(WATER_ROCK), rep.rock_d50_m)).clamp(0.002, 2.0);
                c.aeration = (own.aeration + get(WATER_AERATION) - rep.aeration).clamp(0.0, 1.5);
                c.turbulence = (own.turbulence + get(WATER_TURBULENCE) - rep.turbulence).clamp(0.0, 1.0);
                c.gain = own.gain * get(WATER_GAIN);
                // Lake waves and drips do not depend on the stream's discharge.
                ModelConfig::Water(c)
            }
            ModelConfig::Wind(own) => {
                let mut c = if wind_char > 0 { Wind::PRESETS.get(wind_char - 1).and_then(|p| Wind::preset(p)).unwrap_or(own) } else { own };
                c.exposure = (c.exposure * get(WIND_EXPOSURE)).clamp(0.0, 2.5);
                let w = get(WIND_WHISTLE);
                c.gap = if bases.wind.gap > 1e-6 { c.gap * w / bases.wind.gap } else { w }.clamp(0.0, 3.0);
                c.gain = own.gain * c.gain;
                ModelConfig::Wind(c)
            }
            ModelConfig::Foliage(own) => {
                let mut c = own;
                c.density = (own.density * get(FOLIAGE_DENSITY)).clamp(0.0, 3.0);
                let dry = get(FOLIAGE_DRYNESS);
                c.dryness = if dry < 0.0 { own.dryness } else { dry };
                c.flutter = (own.flutter * get(FOLIAGE_FLUTTER)).clamp(0.0, 1.0);
                c.creak = (own.creak * get(FOLIAGE_CREAK)).clamp(0.0, 1.0);
                c.exposure = (own.exposure * get(FOLIAGE_EXPOSURE)).clamp(0.0, 2.5);
                ModelConfig::Foliage(c)
            }
            ModelConfig::Rain(own) => {
                let mut c = own;
                if surface > 0 && owner_is_source {
                    if let Some(p) = Rain::PRESETS.get(surface - 1).and_then(|p| Rain::preset(p)) {
                        c.surface = p.surface;
                    }
                }
                // Below freezing the precipitation falls as snow.
                if get(ENV_TEMPERATURE) < 0.0 {
                    c.snow = 1.0;
                }
                c.gain = own.gain * get(RAIN_GAIN);
                ModelConfig::Rain(c)
            }
            ModelConfig::Insects(own) => {
                let mut c = own;
                c.gain = own.gain * get(INSECTS_ACTIVITY);
                c.temperature_offset_c = (own.temperature_offset_c + get(INSECTS_WARMTH)).clamp(-10.0, 10.0);
                ModelConfig::Insects(c)
            }
            ModelConfig::Fire(own) => {
                let mut c = own;
                c.heat_kw = (own.heat_kw * ratio(get(FIRE_HEAT), bases.fire.heat_kw)).clamp(1.0, 2000.0);
                c.moisture = get(FIRE_MOISTURE);
                c.resin = get(FIRE_RESIN);
                ModelConfig::Fire(c)
            }
            ModelConfig::Thunder(own) => ModelConfig::Thunder(own),
        };
        em.generator.set_model(&cfg);
    }
}

pub fn season_of(i: f32) -> Season {
    match i.round() as i32 {
        0 => Season::Spring,
        2 => Season::Autumn,
        3 => Season::Winter,
        _ => Season::Summer,
    }
}
