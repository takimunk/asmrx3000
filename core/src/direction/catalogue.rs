//! The direction as data: episodes (a weather spell with its own inner course, segment by
//! segment), the scene's script and its starting steer. The built-in catalogue of a temperate
//! forest is `episodes.json`; a scene's `direction.episodes` add to it or replace episodes of
//! the same name (`"only": true`: the scene's alone). Validated once, when the scene is built.

use serde::{Deserialize, Serialize};

use crate::scene::Season;

/// A value or a range `[lo, hi]` to draw from uniformly (a single number is both ends).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(from = "SpanIn", into = "[f32; 2]")]
pub struct Span(pub [f32; 2]);

#[derive(Deserialize)]
#[serde(untagged)]
enum SpanIn {
    One(f32),
    Two([f32; 2]),
}

impl From<SpanIn> for Span {
    fn from(s: SpanIn) -> Self {
        match s {
            SpanIn::One(v) => Span([v, v]),
            SpanIn::Two([a, b]) => Span([a.min(b), a.max(b)]),
        }
    }
}

impl From<Span> for [f32; 2] {
    fn from(s: Span) -> Self {
        s.0
    }
}

impl Span {
    fn ok(&self, lo: f32, hi: f32) -> bool {
        self.0.iter().all(|v| v.is_finite() && *v >= lo && *v <= hi)
    }
}

/// One stretch of an episode: what the weather heads for, and how fast it gets there.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Segment {
    /// What it is ("gust front"), for the timeline and the UI.
    pub label: String,
    pub minutes: Option<Span>,
    /// Mean wind at canopy height, m/s.
    pub wind: Option<Span>,
    /// Gustiness (wind.gustiness, 1 = typical).
    pub gust: Option<Span>,
    /// Rain rate in the open, mm/h.
    pub rain: Option<Span>,
    /// Thunderstorm activity 0..1.
    pub storm: Option<Span>,
    /// Air temperature relative to the day's climate at this hour, °C.
    pub temp: Option<Span>,
    /// Relative humidity 0..1.
    pub humidity: Option<Span>,
    /// Song activity relative to the scene's.
    pub birds: Option<Span>,
    /// The wind turns by this many degrees as the segment begins (+ = clockwise, veering).
    pub veer: Option<Span>,
    /// Seconds: time constant towards this segment's targets (a gust front arrives in ~15 s);
    /// omitted = each quantity's own pace.
    pub ease: Option<f32>,
    /// Scene levers (by key) this segment heads for; the ones left out go back to their default.
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub levers: std::collections::BTreeMap<String, Span>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Episode {
    pub name: String,
    /// Weight among the episodes that may come next (0 = only through `then`, a script or a cue).
    pub weight: f32,
    /// Hours it belongs to, `[from, to]` (wraps past midnight); elsewhere 0.12 as likely.
    pub hours: Option<[f32; 2]>,
    /// Seasons it belongs to (empty = all); in others 1/6 as likely.
    pub seasons: Vec<Season>,
    /// −1 dry .. +1 wet and stormy: how the weather steer favours it.
    pub wet: f32,
    /// −1 still .. +1 lively: how the energy steer favours it.
    pub energy: f32,
    /// 0 steady .. 1 eventful: how the change steer favours it (and how much a steady
    /// direction stretches it).
    pub drama: f32,
    /// It may only follow these (empty = any).
    pub after: Vec<String>,
    /// Always followed by this one (a shower by the time after the rain).
    pub then: Option<String>,
    pub segments: Vec<Segment>,
}

impl Default for Episode {
    fn default() -> Self {
        Self { name: String::new(), weight: 1.0, hours: None, seasons: Vec::new(), wet: 0.0, energy: 0.0, drama: 0.0, after: Vec::new(), then: None, segments: Vec::new() }
    }
}

/// A scripted step: this episode, optionally stretched or squeezed to `minutes` in all.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub episode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minutes: Option<f32>,
}

/// `"direction"` of a scene (docs/direction.md).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct DirectionDesc {
    /// Episodes added to the built-in catalogue (same name: replaces it).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub episodes: Vec<Episode>,
    /// Only the scene's episodes, not the built-in ones.
    pub only: bool,
    /// The first episode (omitted: drawn among the gentle ones that suit the hour).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<String>,
    /// Played first, in order; then the direction runs free (or the script repeats).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub script: Vec<Step>,
    pub repeat: bool,
    /// Starting steer (the `direction.*` controls): −1 fair .. +1 wet, −1 still .. +1 lively,
    /// 0 steady .. 1 eventful, scene hours per real hour, minutes a value set by hand is held.
    pub weather: f32,
    pub energy: f32,
    pub change: f32,
    pub clock: f32,
    pub hold_min: f32,
    /// It drives time and weather (env.*, wind, rain, birds, water). Off for scenes whose
    /// world is not weather (underwater, space): then only the scene levers move.
    pub climate: bool,
}

impl Default for DirectionDesc {
    fn default() -> Self {
        Self { episodes: Vec::new(), only: false, start: None, script: Vec::new(), repeat: false, weather: 0.0, energy: 0.0, change: 0.35, clock: 1.0, hold_min: 20.0, climate: true }
    }
}

#[derive(Deserialize)]
struct File {
    episodes: Vec<Episode>,
}

/// The built-in temperate-forest episodes (episodes.json).
pub fn builtin() -> Vec<Episode> {
    let f: File = serde_json::from_str(include_str!("episodes.json")).expect("built-in episodes are valid");
    f.episodes
}

/// Names of the built-in episodes, in catalogue order: the options of `direction.cue` after
/// "none" and "next".
pub const BUILTIN_NAMES: [&str; 11] = ["clear", "calm", "breeze", "windy", "overcast", "drizzle", "shower", "steady_rain", "thunderstorm", "after_rain", "mist"];

impl DirectionDesc {
    /// The episodes this scene runs with, checked: names unique and known, ranges physical.
    pub fn catalogue(&self) -> Result<Vec<Episode>, String> {
        let mut eps = if self.only { Vec::new() } else { builtin() };
        for e in &self.episodes {
            match eps.iter_mut().find(|x| x.name == e.name) {
                Some(x) => *x = e.clone(),
                None => eps.push(e.clone()),
            }
        }
        if eps.is_empty() {
            return Err("direction: no episodes".into());
        }
        let known = |n: &str| eps.iter().any(|e| e.name == n);
        for e in &eps {
            let at = |what: &str| format!("direction: episode `{}`: {what}", e.name);
            if e.name.is_empty() {
                return Err("direction: an episode without a name".into());
            }
            if eps.iter().filter(|x| x.name == e.name).count() > 1 {
                return Err(at("name used twice"));
            }
            if e.segments.is_empty() {
                return Err(at("no segments"));
            }
            if !(e.weight.is_finite() && e.weight >= 0.0) {
                return Err(at("weight must be ≥ 0"));
            }
            if let Some(h) = e.hours {
                if !h.iter().all(|v| (0.0..=24.0).contains(v)) {
                    return Err(at("hours are 0..24"));
                }
            }
            for n in e.after.iter().chain(e.then.iter()) {
                if !known(n) {
                    return Err(at(&format!("unknown episode `{n}` in after/then")));
                }
            }
            for (k, s) in e.segments.iter().enumerate() {
                let bad = |f: &str, r: &str| Err(at(&format!("segment {k}: {f} must be within {r}")));
                let chk = |v: &Option<Span>, lo: f32, hi: f32| v.as_ref().is_none_or(|s| s.ok(lo, hi));
                if !chk(&s.minutes, 0.05, 600.0) {
                    return bad("minutes", "0.05..600");
                }
                if !chk(&s.wind, 0.0, 30.0) {
                    return bad("wind", "0..30 m/s");
                }
                if !chk(&s.gust, 0.0, 2.0) {
                    return bad("gust", "0..2");
                }
                if !chk(&s.rain, 0.0, 100.0) {
                    return bad("rain", "0..100 mm/h");
                }
                if !chk(&s.storm, 0.0, 1.0) || !chk(&s.humidity, 0.0, 1.0) {
                    return bad("storm/humidity", "0..1");
                }
                if !chk(&s.temp, -20.0, 20.0) {
                    return bad("temp", "−20..20 °C");
                }
                if !chk(&s.birds, 0.0, 3.0) {
                    return bad("birds", "0..3");
                }
                if !chk(&s.veer, -360.0, 360.0) {
                    return bad("veer", "−360..360°");
                }
                if s.ease.is_some_and(|v| !(v.is_finite() && v > 0.0)) {
                    return bad("ease", "> 0 s");
                }
                if s.levers.values().any(|r| !r.ok(-1e6, 1e6)) {
                    return bad("levers", "finite numbers");
                }
            }
        }
        for n in self.start.iter().chain(self.script.iter().map(|s| &s.episode)) {
            if !known(n) {
                return Err(format!("direction: unknown episode `{n}`"));
            }
        }
        if self.script.iter().any(|s| s.minutes.is_some_and(|m| !(m.is_finite() && m > 0.0))) {
            return Err("direction: script minutes must be > 0".into());
        }
        let ok = |v: f32, lo: f32, hi: f32| v.is_finite() && (lo..=hi).contains(&v);
        if !ok(self.weather, -1.0, 1.0) || !ok(self.energy, -1.0, 1.0) || !ok(self.change, 0.0, 1.0) || !ok(self.clock, 0.0, 120.0) || !ok(self.hold_min, 0.5, 240.0) {
            return Err("direction: weather/energy −1..1, change 0..1, clock 0..120, hold_min 0.5..240".into());
        }
        Ok(eps)
    }
}
