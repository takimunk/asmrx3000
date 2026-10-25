//! Sound generators. Each produces a mono signal; the engine places it in space.
//!
//! Every generator sits behind the same small interface, [`Generator::render`], so the
//! procedural models here can be swapped for neural ones (core/src/neural/) per emitter
//! without touching the director or the engine.

mod bird;
pub mod common;
pub mod dolphin;
pub mod fire;
pub mod foliage;
pub mod insects;
mod marine;
pub mod plasma;
pub mod rain;
pub mod reef;
pub mod sea;
pub mod thunder;
pub mod water;
pub mod whale;
pub mod wind;
mod wings;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

pub use bird::{Bird, VoiceStyle};
pub use dolphin::{DolphinConfig, Dolphins};
pub use fire::{Fire, FireConfig};
pub use foliage::{Foliage, FoliageConfig, FoliageKind};
pub use insects::{InsectConfig, InsectKind, Insects};
pub use plasma::{Plasma, PlasmaConfig};
pub use rain::{Rain, RainConfig, Surface};
pub use reef::{Reef, ReefConfig};
pub use sea::{Sea, SeaConfig};
pub use thunder::{Thunder, ThunderConfig};
pub use water::{Water, WaterConfig};
pub use whale::{Whale, WhaleConfig};
pub use wind::{Wind, WindConfig};
pub use wings::{WingStyle, Wings};

use crate::engine::SourceKind;

/// Models render in chunks of at most this many samples (the engine's control block).
pub const BLOCK: usize = crate::engine::BLOCK;

/// Weather and environment as the physical models see it, written by the director every
/// block (see `director::weather`). Units are physical; every model reads what it needs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ambient {
    /// Mean wind speed at canopy height, m/s (local gusts come in `Controls::gust`).
    pub wind_ms: f32,
    /// Rain rate in the open, mm/h (smoothed, with natural fluctuation).
    pub rain_mm_h: f32,
    /// Air temperature, °C.
    pub temperature_c: f32,
    /// Relative humidity, 0..1.
    pub humidity: f32,
    /// Canopy water store relative to its capacity, 0..1 (leaves wet).
    pub leaf_wetness: f32,
    /// Ground surface wetness, 0..1.
    pub ground_wetness: f32,
    /// Share of the ground covered by puddles, 0..1.
    pub puddles: f32,
    /// Canopy drainage (dripping) under trees, mm/h of water.
    pub drip_mm_h: f32,
    /// Local time, hours 0..24.
    pub hour: f32,
    /// Seasonal leaf dryness, 0 (spring) .. 1 (late autumn).
    pub dryness: f32,
    /// Leaves on deciduous trees, 0 (winter) .. 1 (summer).
    pub leaf_cover: f32,
    /// Thunderstorm activity, 0..1.
    pub storm: f32,
    /// Lightning strike counter: changes on every strike.
    pub strike: u32,
    /// Horizontal distance of the latest strike, m.
    pub strike_distance_m: f32,
    /// Return strokes of the latest flash: 0 = intracloud, 1..=5 cloud-to-ground (with
    /// `strike` it fixes the flash's timing, `thunder::Flash::new`).
    pub strike_strokes: u8,
    /// Bearing of the latest strike, radians (0 = +x, +π/2 = +y of the world the director
    /// works in; flight mode takes it in its landscape frame).
    pub strike_bearing: f32,
}

impl Default for Ambient {
    fn default() -> Self {
        Self {
            wind_ms: 0.0,
            rain_mm_h: 0.0,
            temperature_c: 16.0,
            humidity: 0.75,
            leaf_wetness: 0.0,
            ground_wetness: 0.0,
            puddles: 0.0,
            drip_mm_h: 0.0,
            hour: 9.0,
            dryness: 0.1,
            leaf_cover: 1.0,
            storm: 0.0,
            strike: 0,
            strike_distance_m: 0.0,
            strike_strokes: 0,
            strike_bearing: 0.0,
        }
    }
}

/// Control values for one block. The engine fills the environment fields, the director the
/// behavioural ones; each generator reads only what it needs.
#[derive(Clone, Copy, Debug, Default)]
pub struct Controls {
    /// River flow at this emitter, 0..1.
    pub water: f32,
    /// Local gust strength from the wind field.
    pub gust: f32,
    /// How eager a voice is to sing, 0..1 (0 = silent).
    pub activity: f32,
    /// Voice mode: short flight / contact calls instead of full song.
    pub calling: bool,
    /// Wingbeat effort, 0 = perched / gliding, 1 = take-off clatter.
    pub flap: f32,
    /// With `calling`: alarm calls (blackbird "chink", robin "tic", …) instead of contact calls.
    pub alarm: bool,
    /// The director drives the wingbeat rhythm itself (every glide is `flap = 0`); the wings
    /// must not add their own flap/glide bouts.
    pub wings_driven: bool,
    /// Counter-singing: a neighbour just finished, reply soon (the next song starts within
    /// about a second instead of after the usual pause).
    pub reply: bool,
    /// Counter-singing: a neighbour of the same species is singing; do not start a phrase
    /// now (the bird still wants to: the mix keeps its place in the queue).
    pub wait: bool,
    /// Weather and environment (director).
    pub env: Ambient,
    /// Prefer trained neural voices where a species has one (ASMR_PARAM_NEURAL_BIRDS).
    pub neural: bool,
    /// A/B renders of the mix before the sound director: skip voice level matching.
    pub legacy_levels: bool,
    /// Clip players (clips.rs): a new value starts file `cue_file` at `cue_offset` (share
    /// of its length) and playback rate `cue_rate`.
    pub cue: u32,
    pub cue_file: u32,
    pub cue_offset: f32,
    pub cue_rate: f32,
    /// Clip players: output envelope 0..1 (appearance fades, written by the choreography).
    pub clip_env: f32,
    /// Clip players: live playback-rate factor (the group's pitch); 0 = unset = 1.
    pub clip_pitch: f32,
}

/// A mono sound generator. Called on the audio thread once per control block:
/// no allocation, no locks.
pub trait Generator: Send {
    fn render(&mut self, out: &mut [f32], controls: &Controls);
    /// Dominant pitch right now if the generator has one (voices), else 0.
    fn pitch(&self) -> f32 {
        0.0
    }
    /// Event rate for telemetry (bubbles per second for water), else 0.
    fn event_rate(&self) -> f32 {
        0.0
    }
    /// In the middle of a phrase (song or call) right now. The mix layer counts concurrent
    /// phrases and holds a new one back by setting `activity` to 0 until a slot frees;
    /// generators must never cut a phrase they started because of that.
    fn busy(&self) -> bool {
        false
    }
    /// Runtime reconfiguration (audio thread, no allocation). Physical models accept their own
    /// `ModelConfig` variant and glide to it; everything else ignores it.
    fn set_model(&mut self, _cfg: &ModelConfig) {}
}

/// What a flying animal sounds like: a voice (procedural or neural) plus its wings, both at
/// the same point in space.
pub struct Flyer {
    pub voice: Box<dyn Generator>,
    pub wings: Wings,
    /// Linear output gain (how loud this individual is at 1 m relative to the species).
    pub gain: f32,
    scratch: [f32; crate::engine::BLOCK],
}

impl Flyer {
    pub fn new(voice: Box<dyn Generator>, wings: Wings, gain: f32) -> Self {
        Self { voice, wings, gain, scratch: [0.0; crate::engine::BLOCK] }
    }
}

impl Generator for Flyer {
    fn render(&mut self, out: &mut [f32], c: &Controls) {
        self.voice.render(out, c);
        let mut off = 0;
        while off < out.len() {
            let len = (out.len() - off).min(self.scratch.len());
            let tmp = &mut self.scratch[..len];
            self.wings.render_driven(tmp, c.flap, c.wings_driven);
            for (o, w) in out[off..off + len].iter_mut().zip(tmp.iter()) {
                *o = (*o + *w) * self.gain;
            }
            off += len;
        }
    }
    fn pitch(&self) -> f32 {
        self.voice.pitch()
    }
    fn busy(&self) -> bool {
        self.voice.busy()
    }
}

// ---------------------------------------------------------------------------------------
// Model registry: scene descriptions reference a model + preset + parameter overrides.
// ---------------------------------------------------------------------------------------

/// Physical sound models available to scene descriptions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelKind {
    Wind,
    Foliage,
    Water,
    Rain,
    Thunder,
    Fire,
    Insects,
    /// Space plasma waves: hiss, whistlers, chorus, narrowband tones (plasma.rs).
    Plasma,
    /// Humpback whale song, far away underwater (whale.rs).
    Whale,
    /// A dolphin pod's whistles and echolocation clicks (dolphin.rs).
    Dolphins,
    /// Coral reef: snapping-shrimp crackle and fish calls (reef.rs).
    Reef,
    /// Open water under the surface: ambient bed, bubbles, rain above (sea.rs).
    Sea,
}

impl ModelKind {
    pub const ALL: [ModelKind; 12] = [
        ModelKind::Wind,
        ModelKind::Foliage,
        ModelKind::Water,
        ModelKind::Rain,
        ModelKind::Thunder,
        ModelKind::Fire,
        ModelKind::Insects,
        ModelKind::Plasma,
        ModelKind::Whale,
        ModelKind::Dolphins,
        ModelKind::Reef,
        ModelKind::Sea,
    ];

    pub fn presets(self) -> &'static [&'static str] {
        match self {
            ModelKind::Wind => Wind::PRESETS,
            ModelKind::Foliage => Foliage::PRESETS,
            ModelKind::Water => Water::PRESETS,
            ModelKind::Rain => Rain::PRESETS,
            ModelKind::Thunder => Thunder::PRESETS,
            ModelKind::Fire => Fire::PRESETS,
            ModelKind::Insects => Insects::PRESETS,
            ModelKind::Plasma => Plasma::PRESETS,
            ModelKind::Whale => Whale::PRESETS,
            ModelKind::Dolphins => Dolphins::PRESETS,
            ModelKind::Reef => Reef::PRESETS,
            ModelKind::Sea => Sea::PRESETS,
        }
    }

    /// Engine source kind (telemetry / UI colour, per-kind gain).
    pub fn source_kind(self) -> SourceKind {
        match self {
            ModelKind::Wind | ModelKind::Foliage | ModelKind::Plasma => SourceKind::Wind,
            ModelKind::Water | ModelKind::Reef | ModelKind::Sea => SourceKind::Water,
            ModelKind::Rain => SourceKind::Rain,
            ModelKind::Thunder => SourceKind::Thunder,
            ModelKind::Fire => SourceKind::Fire,
            ModelKind::Insects | ModelKind::Whale | ModelKind::Dolphins => SourceKind::Insect,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            ModelKind::Wind => "wind",
            ModelKind::Foliage => "foliage",
            ModelKind::Water => "water",
            ModelKind::Rain => "rain",
            ModelKind::Thunder => "thunder",
            ModelKind::Fire => "fire",
            ModelKind::Insects => "insects",
            ModelKind::Plasma => "plasma",
            ModelKind::Whale => "whale",
            ModelKind::Dolphins => "dolphins",
            ModelKind::Reef => "reef",
            ModelKind::Sea => "sea",
        }
    }

    /// Sounds made and heard under water: the engine's `medium: water` muffling (meant for
    /// air-built models heard through the surface) is not applied to them.
    pub fn underwater(self) -> bool {
        matches!(self, ModelKind::Whale | ModelKind::Dolphins | ModelKind::Reef | ModelKind::Sea)
    }
}

/// `{"model": "water", "preset": "brook_pebbles", "params": {"flow_ms": 1.2}}`: a preset
/// (omitted = the model's defaults) with individual physical parameters overridden.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SoundDesc {
    pub model: ModelKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub params: serde_json::Map<String, serde_json::Value>,
}

impl SoundDesc {
    pub fn new(model: ModelKind, preset: &str) -> Self {
        Self { model, preset: Some(preset.to_string()), params: Default::default() }
    }

    pub fn with(mut self, key: &str, value: impl Into<serde_json::Value>) -> Self {
        self.params.insert(key.to_string(), value.into());
        self
    }
}

/// Applies `params` on top of `base` (unknown keys and out-of-range values are errors).
fn configure<T: Serialize + DeserializeOwned>(base: T, params: &serde_json::Map<String, serde_json::Value>) -> Result<T, String> {
    if params.is_empty() {
        return Ok(base);
    }
    let mut v = serde_json::to_value(&base).map_err(|e| e.to_string())?;
    let obj = v.as_object_mut().ok_or("config is not an object")?;
    for (k, val) in params {
        if !obj.contains_key(k) {
            let valid: Vec<&str> = obj.keys().map(String::as_str).collect();
            return Err(format!("unknown parameter `{k}` (valid: {})", valid.join(", ")));
        }
        obj.insert(k.clone(), val.clone());
    }
    serde_json::from_value(v).map_err(|e| e.to_string())
}

fn pick<T>(model: &str, preset: &Option<String>, lookup: impl Fn(&str) -> Option<T>, default: T, names: &[&str]) -> Result<T, String> {
    match preset {
        None => Ok(default),
        Some(p) => lookup(p).ok_or_else(|| format!("{model}: unknown preset `{p}` (valid: {})", names.join(", "))),
    }
}

/// A fully resolved model configuration (plain data: copying it never allocates).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ModelConfig {
    Wind(WindConfig),
    Foliage(FoliageConfig),
    Water(WaterConfig),
    Rain(RainConfig),
    Thunder(ThunderConfig),
    Fire(FireConfig),
    Insects(InsectConfig),
    Plasma(PlasmaConfig),
    Whale(WhaleConfig),
    Dolphins(DolphinConfig),
    Reef(ReefConfig),
    Sea(SeaConfig),
}

impl SoundDesc {
    /// Resolves preset + overrides and validates ranges.
    pub fn resolve(&self) -> Result<ModelConfig, String> {
        let m = self.model.name();
        let p = &self.preset;
        let names = self.model.presets();
        let ctx = |e: String| format!("{m}: {e}");
        Ok(match self.model {
            ModelKind::Wind => {
                let c = configure(pick(m, p, Wind::preset, WindConfig::default(), names)?, &self.params).map_err(ctx)?;
                c.validate().map_err(ctx)?;
                ModelConfig::Wind(c)
            }
            ModelKind::Foliage => {
                let c = configure(pick(m, p, Foliage::preset, FoliageConfig::default(), names)?, &self.params).map_err(ctx)?;
                c.validate().map_err(ctx)?;
                ModelConfig::Foliage(c)
            }
            ModelKind::Water => {
                let c = configure(pick(m, p, Water::preset, WaterConfig::default(), names)?, &self.params).map_err(ctx)?;
                c.validate().map_err(ctx)?;
                ModelConfig::Water(c)
            }
            ModelKind::Rain => {
                let c = configure(pick(m, p, Rain::preset, RainConfig::default(), names)?, &self.params).map_err(ctx)?;
                c.validate().map_err(ctx)?;
                ModelConfig::Rain(c)
            }
            ModelKind::Thunder => {
                let c = configure(pick(m, p, Thunder::preset, ThunderConfig::default(), names)?, &self.params).map_err(ctx)?;
                c.validate().map_err(ctx)?;
                ModelConfig::Thunder(c)
            }
            ModelKind::Fire => {
                let c = configure(pick(m, p, Fire::preset, FireConfig::default(), names)?, &self.params).map_err(ctx)?;
                c.validate().map_err(ctx)?;
                ModelConfig::Fire(c)
            }
            ModelKind::Insects => {
                let c = configure(pick(m, p, Insects::preset, InsectConfig::default(), names)?, &self.params).map_err(ctx)?;
                c.validate().map_err(ctx)?;
                ModelConfig::Insects(c)
            }
            ModelKind::Plasma => {
                let c = configure(pick(m, p, Plasma::preset, PlasmaConfig::default(), names)?, &self.params).map_err(ctx)?;
                c.validate().map_err(ctx)?;
                ModelConfig::Plasma(c)
            }
            ModelKind::Whale => {
                let c = configure(pick(m, p, Whale::preset, WhaleConfig::default(), names)?, &self.params).map_err(ctx)?;
                c.validate().map_err(ctx)?;
                ModelConfig::Whale(c)
            }
            ModelKind::Dolphins => {
                let c = configure(pick(m, p, Dolphins::preset, DolphinConfig::default(), names)?, &self.params).map_err(ctx)?;
                c.validate().map_err(ctx)?;
                ModelConfig::Dolphins(c)
            }
            ModelKind::Reef => {
                let c = configure(pick(m, p, Reef::preset, ReefConfig::default(), names)?, &self.params).map_err(ctx)?;
                c.validate().map_err(ctx)?;
                ModelConfig::Reef(c)
            }
            ModelKind::Sea => {
                let c = configure(pick(m, p, Sea::preset, SeaConfig::default(), names)?, &self.params).map_err(ctx)?;
                c.validate().map_err(ctx)?;
                ModelConfig::Sea(c)
            }
        })
    }

    /// Builds the generator. Allocates: call off the audio thread.
    pub fn build(&self, sample_rate: f32, seed: u64) -> Result<Box<dyn Generator>, String> {
        Ok(self.resolve()?.build(sample_rate, seed))
    }
}

impl ModelConfig {
    pub fn kind(&self) -> ModelKind {
        match self {
            ModelConfig::Wind(_) => ModelKind::Wind,
            ModelConfig::Foliage(_) => ModelKind::Foliage,
            ModelConfig::Water(_) => ModelKind::Water,
            ModelConfig::Rain(_) => ModelKind::Rain,
            ModelConfig::Thunder(_) => ModelKind::Thunder,
            ModelConfig::Fire(_) => ModelKind::Fire,
            ModelConfig::Insects(_) => ModelKind::Insects,
            ModelConfig::Plasma(_) => ModelKind::Plasma,
            ModelConfig::Whale(_) => ModelKind::Whale,
            ModelConfig::Dolphins(_) => ModelKind::Dolphins,
            ModelConfig::Reef(_) => ModelKind::Reef,
            ModelConfig::Sea(_) => ModelKind::Sea,
        }
    }

    pub fn build(self, sr: f32, seed: u64) -> Box<dyn Generator> {
        match self {
            ModelConfig::Wind(c) => Box::new(Wind::new(sr, seed, c)),
            ModelConfig::Foliage(c) => Box::new(Foliage::new(sr, seed, c)),
            ModelConfig::Water(c) => Box::new(Water::new(sr, seed, c)),
            ModelConfig::Rain(c) => Box::new(Rain::new(sr, seed, c)),
            ModelConfig::Thunder(c) => Box::new(Thunder::new(sr, seed, c)),
            ModelConfig::Fire(c) => Box::new(Fire::new(sr, seed, c)),
            ModelConfig::Insects(c) => Box::new(Insects::new(sr, seed, c)),
            ModelConfig::Plasma(c) => Box::new(Plasma::new(sr, seed, c)),
            ModelConfig::Whale(c) => Box::new(Whale::new(sr, seed, c)),
            ModelConfig::Dolphins(c) => Box::new(Dolphins::new(sr, seed, c)),
            ModelConfig::Reef(c) => Box::new(Reef::new(sr, seed, c)),
            ModelConfig::Sea(c) => Box::new(Sea::new(sr, seed, c)),
        }
    }
}

#[cfg(test)]
mod tests;
