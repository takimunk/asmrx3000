//! Prompt control: the scene as a JSON state for a language model, and a JSON patch back.
//!
//! * [`state_json`] — every control (key, value, unit, range, preset names, overridden flag),
//!   a few environment readings and a compact list of scene objects.
//! * [`apply_patch`] — `{"glide_s": 4, "changes": [{"key": "water.flow", "value": 0.3}, …],
//!   "moves": [{"object": 3, "position": [x, y, z]}]}`. Keys are the control keys the sliders
//!   use; values are clamped, presets may be given by name, `"reset"` hands a child back to its
//!   macro. Continuous controls glide (eased) instead of jumping. The reply carries an `undo`
//!   patch that restores what the patch touched.
//! * [`prompt_json`] — the static system prompt (what every control means physically, rules,
//!   few-shot examples), the dynamic context (current values) and the tool schema generated
//!   from the control tree, so a provider adapter only has to transport them.
//!
//! Glides live in [`Glides`]: lock-free slots, one per control, advanced by the audio thread
//! every block and by readers (so they land even while nothing renders). Wall-clock based.

use std::fmt::Write as _;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

use serde_json::{json, Map, Value};

use crate::controls::{self, flags, ControlBank, COUNT};
use crate::engine::AtomicF32;
use crate::scene::{object_flags, ObjectBridge, ObjectInfo, World, MAX_OBJECTS};

/// Default glide when a patch does not say.
pub const DEFAULT_GLIDE_S: f32 = 4.0;
pub const MAX_GLIDE_S: f32 = 120.0;

const IDLE: u32 = 0;
const VALUE: u32 = 1;
const RESET: u32 = 2;

struct Slot {
    /// generation << 2 | kind
    state: AtomicU32,
    from: AtomicF32,
    to: AtomicF32,
    start: AtomicU64,
    dur: AtomicF32,
}

/// In-flight glides, one slot per control.
pub(crate) struct Glides {
    slots: Box<[Slot]>,
    #[cfg(not(target_arch = "wasm32"))]
    epoch: Instant,
    /// Seconds rendered (clock on wasm, where `Instant` traps).
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    rendered: AtomicU64,
}

impl Glides {
    pub(crate) fn new() -> Self {
        Self {
            slots: (0..COUNT)
                .map(|_| Slot { state: AtomicU32::new(0), from: AtomicF32::new(0.0), to: AtomicF32::new(0.0), start: AtomicU64::new(0), dur: AtomicF32::new(0.0) })
                .collect(),
            #[cfg(not(target_arch = "wasm32"))]
            epoch: Instant::now(),
            rendered: AtomicU64::new(0f64.to_bits()),
        }
    }

    fn now(&self) -> f64 {
        #[cfg(not(target_arch = "wasm32"))]
        return self.epoch.elapsed().as_secs_f64();
        #[cfg(target_arch = "wasm32")]
        return f64::from_bits(self.rendered.load(Ordering::Relaxed));
    }

    /// Audio thread: `dt` seconds were rendered.
    pub(crate) fn advance(&self, dt: f32) {
        let t = f64::from_bits(self.rendered.load(Ordering::Relaxed)) + dt as f64;
        self.rendered.store(t.to_bits(), Ordering::Relaxed);
    }

    fn start(&self, i: usize, kind: u32, from: f32, to: f32, dur: f32) {
        let s = &self.slots[i];
        let gen = (s.state.load(Ordering::Relaxed) >> 2).wrapping_add(1);
        s.state.store(gen << 2, Ordering::Release);
        s.from.store(from);
        s.to.store(to);
        s.dur.store(dur.max(1e-3));
        s.start.store(self.now().to_bits(), Ordering::Relaxed);
        s.state.store(gen << 2 | kind, Ordering::Release);
    }

    /// Stops the glide on control `i` (the user grabbed the slider).
    pub(crate) fn cancel(&self, i: usize) {
        let s = &self.slots[i];
        let gen = (s.state.load(Ordering::Relaxed) >> 2).wrapping_add(1);
        s.state.store(gen << 2, Ordering::Release);
    }

    pub(crate) fn active(&self) -> usize {
        self.slots.iter().filter(|s| s.state.load(Ordering::Relaxed) & 3 != IDLE).count()
    }

    /// Moves every gliding control to its current eased value. Any thread, real-time safe.
    pub(crate) fn tick(&self, bank: &ControlBank, params: &[AtomicF32]) {
        let now = self.now();
        for (i, s) in self.slots.iter().enumerate() {
            let st = s.state.load(Ordering::Acquire);
            let kind = st & 3;
            if kind == IDLE {
                continue;
            }
            let id = controls::meta(i).id;
            let (from, dur) = (s.from.load(), s.dur.load());
            let to = if kind == RESET { bank.followed(i, params).unwrap_or(s.to.load()) } else { s.to.load() };
            let t = ((now - f64::from_bits(s.start.load(Ordering::Relaxed))) as f32 / dur).clamp(0.0, 1.0);
            if t >= 1.0 {
                if s.state.compare_exchange(st, st & !3, Ordering::AcqRel, Ordering::Relaxed).is_ok() {
                    bank.set(id, if kind == RESET { f32::NAN } else { wrap(i, to) }, params);
                }
                continue;
            }
            // Smoothstep: no click at either end.
            let e = t * t * (3.0 - 2.0 * t);
            bank.set(id, wrap(i, from + (to - from) * e), params);
        }
    }
}

fn is_time(i: usize) -> bool {
    controls::meta(i).id == controls::id::ENV_TIME
}

fn wrap(i: usize, v: f32) -> f32 {
    if is_time(i) {
        v.rem_euclid(24.0)
    } else {
        v
    }
}

/// What the prompt layer reads from the engine.
pub(crate) struct View<'a> {
    pub bank: &'a ControlBank,
    pub params: &'a [AtomicF32],
    pub telemetry: &'a [AtomicF32],
    pub glides: &'a Glides,
    pub objects: &'a ObjectBridge,
    pub labels: &'a [String],
}

impl View<'_> {
    fn values(&self) -> [f32; COUNT] {
        self.bank.state(self.params).v
    }

    fn objects(&self) -> Vec<ObjectInfo> {
        let mut out = vec![ObjectInfo::default(); MAX_OBJECTS];
        let n = self.objects.read(&mut out);
        out.truncate(n);
        out
    }
}

/// Label of every scene object (index order): "tree", "river", bird species, source model.
pub(crate) fn labels(world: &World) -> Vec<String> {
    use crate::scene::Geometry;
    world
        .entities
        .iter()
        .map(|e| match &e.geometry {
            Geometry::Tree { .. } => "tree".to_string(),
            Geometry::River { .. } => "river".to_string(),
            Geometry::Source { model, .. } => model.name().to_string(),
            Geometry::Point => "bird".to_string(),
        })
        .collect()
}

fn r3(v: f32) -> Value {
    let x = (v as f64 * 1000.0).round() / 1000.0;
    json!(x)
}

fn option_of(id: u32, v: f32) -> Option<&'static str> {
    controls::option_name(id, v.round().max(0.0) as u32)
}

fn ground_name(g: f32) -> &'static str {
    ["forest_floor", "grass", "water", "compact_soil", "snow", "asphalt"].get(g.round().max(0.0) as usize).copied().unwrap_or("forest_floor")
}

/// The current state as JSON (see the module doc).
pub(crate) fn state_json(v: &View) -> String {
    v.glides.tick(v.bank, v.params);
    let vals = v.values();
    let mut ctl = Vec::with_capacity(COUNT);
    for (i, &value) in vals.iter().enumerate() {
        let m = controls::meta(i);
        let mut o = Map::new();
        o.insert("key".into(), json!(m.key));
        o.insert("group".into(), json!(controls::group_name(m.group)));
        if m.flags & flags::MACRO != 0 {
            o.insert("macro".into(), json!(true));
        } else if let Some(p) = controls::index_of_id(m.parent) {
            o.insert("parent".into(), json!(controls::meta(p).key));
        }
        o.insert("value".into(), r3(value));
        if m.flags & flags::DISCRETE != 0 {
            o.insert("option".into(), json!(option_of(m.id, value)));
            o.insert("options".into(), json!(controls::options(m.id)));
        } else {
            o.insert("min".into(), r3(m.min));
            o.insert("max".into(), r3(m.max));
            if m.flags & flags::LOG != 0 {
                o.insert("log".into(), json!(true));
            }
        }
        if !m.unit.is_empty() {
            o.insert("unit".into(), json!(m.unit));
        }
        o.insert("default".into(), r3(v.bank.default_at(i)));
        if m.flags & flags::MACRO == 0 {
            o.insert("overridden".into(), json!(v.bank.overridden_at(i)));
        }
        ctl.push(Value::Object(o));
    }
    let p = |k: u32| v.params[k as usize].load();
    let t = |k: u32| v.telemetry[k as usize].load();
    use crate::engine::{param, telemetry};
    let env = json!({
        "playing": p(param::PLAYING) > 0.5,
        "rain_now_mm_h": r3(t(telemetry::RAIN_RATE)),
        "leaf_wetness": r3(t(telemetry::WETNESS)),
        "canopy_drip_mm_h": r3(t(telemetry::DRIP)),
        "ground": ground_name(p(param::GROUND)),
        "tree_density_per_ha": r3(p(param::TREE_DENSITY)),
        "reverb": r3(p(param::REVERB)),
        "neural_birds": p(param::NEURAL_BIRDS) > 0.5,
    });
    let objects: Vec<Value> = v
        .objects()
        .iter()
        .enumerate()
        .filter(|(_, o)| o.flags & object_flags::ABSENT == 0)
        .map(|(i, o)| {
            let mut m = Map::new();
            m.insert("id".into(), json!(o.id));
            m.insert("kind".into(), json!(v.labels.get(i).map_or("object", |s| s.as_str())));
            m.insert("pos".into(), json!([r3(o.x).as_f64(), r3(o.y).as_f64(), r3(o.z).as_f64()]));
            if o.flags & object_flags::MOVABLE != 0 {
                m.insert("movable".into(), json!(true));
            }
            if o.flags & object_flags::MOVING != 0 {
                m.insert("moving".into(), json!(true));
            }
            Value::Object(m)
        })
        .collect();
    json!({
        "controls": ctl,
        "environment": env,
        "objects": objects,
        "frame": "listener at the origin; +x forward, +y left, +z up; metres",
        "gliding": v.glides.active(),
    })
    .to_string()
}

struct Change {
    i: usize,
    /// None = reset.
    value: Option<f32>,
    glide: f32,
}

fn parse_value(i: usize, val: &Value) -> Result<Option<f32>, String> {
    let m = controls::meta(i);
    match val {
        Value::Null => Ok(None),
        Value::String(s) if s == "reset" => Ok(None),
        Value::String(s) if m.flags & flags::DISCRETE != 0 => controls::options(m.id)
            .iter()
            .position(|o| o == s)
            .map(|k| Some(k as f32))
            .ok_or_else(|| format!("{}: unknown option `{s}` (one of {})", m.key, controls::options(m.id).join(", "))),
        Value::String(s) => s.trim().parse::<f32>().map(Some).map_err(|_| format!("{}: expected a number, got `{s}`", m.key)),
        Value::Number(n) => n.as_f64().map(|x| Some(x as f32)).ok_or_else(|| format!("{}: bad number", m.key)),
        _ => Err(format!("{}: expected a number, an option name or \"reset\"", m.key)),
    }
}

/// Applies a patch (see the module doc). Returns the result JSON:
/// `{"ok", "applied": [{"key", "value" | "reset", "clamped"?}], "moved": [ids], "errors": [..],
///   "undo": <patch>}`. Valid entries are applied even when others are rejected.
pub(crate) fn apply_patch(v: &View, patch: &str) -> String {
    let mut errors: Vec<String> = Vec::new();
    let root: Value = match serde_json::from_str(patch) {
        Ok(x) => x,
        Err(e) => return json!({"ok": false, "applied": [], "moved": [], "errors": [format!("patch is not JSON: {e}")], "undo": {"changes": []}}).to_string(),
    };
    let (default_glide, changes, moves) = match &root {
        Value::Array(a) => (DEFAULT_GLIDE_S, a.clone(), Vec::new()),
        Value::Object(o) => {
            let g = o.get("glide_s").and_then(Value::as_f64).map_or(DEFAULT_GLIDE_S, |g| g as f32);
            let list = |k: &str| match o.get(k) {
                Some(Value::Array(a)) => a.clone(),
                _ => Vec::new(),
            };
            (g, list("changes"), list("moves"))
        }
        _ => (DEFAULT_GLIDE_S, Vec::new(), Vec::new()),
    };
    if !root.is_array() && !root.is_object() {
        errors.push("patch must be an object or an array".into());
    }
    let default_glide = if default_glide.is_finite() { default_glide.clamp(0.0, MAX_GLIDE_S) } else { DEFAULT_GLIDE_S };

    // Parse everything first.
    let mut list: Vec<Change> = Vec::new();
    for c in &changes {
        let Some(key) = c.get("key").and_then(Value::as_str) else {
            errors.push(format!("change without a key: {c}"));
            continue;
        };
        let Some(i) = controls::index_of_key(key) else {
            errors.push(format!("unknown key `{key}`"));
            continue;
        };
        let reset = c.get("reset").and_then(Value::as_bool).unwrap_or(false);
        let value = if reset { Ok(None) } else { parse_value(i, c.get("value").unwrap_or(&Value::Null)) };
        let glide = c.get("glide_s").and_then(Value::as_f64).map_or(default_glide, |g| g as f32);
        let glide = if glide.is_finite() { glide.clamp(0.0, MAX_GLIDE_S) } else { default_glide };
        match value {
            Ok(Some(x)) if !x.is_finite() => errors.push(format!("{key}: value is not finite")),
            Ok(value) => {
                list.retain(|o| o.i != i);
                list.push(Change { i, value, glide });
            }
            Err(e) => errors.push(e),
        }
    }

    // Undo restores what is there now.
    v.glides.tick(v.bank, v.params);
    let before = v.values();
    let mut undo = Vec::new();
    for c in &list {
        let m = controls::meta(c.i);
        let child = m.flags & flags::MACRO == 0;
        let back = if child && !v.bank.overridden_at(c.i) {
            json!("reset")
        } else if m.flags & flags::DISCRETE != 0 {
            json!(option_of(m.id, before[c.i]))
        } else {
            r3(before[c.i])
        };
        undo.push(json!({"key": m.key, "value": back}));
    }

    let mut applied = Vec::new();
    for c in &list {
        let m = controls::meta(c.i);
        let discrete = m.flags & flags::DISCRETE != 0;
        let Some(target) = c.value else {
            // Reset: a child glides back onto its macro's curve; a macro to its default.
            let child = m.flags & flags::MACRO == 0;
            if discrete {
                v.glides.cancel(c.i);
                if child {
                    v.bank.set(m.id, v.bank.default_at(c.i), v.params);
                    v.bank.set(m.id, f32::NAN, v.params);
                } else {
                    v.bank.set(m.id, v.bank.default_at(c.i), v.params);
                }
            } else if child {
                if c.glide <= 0.0 || v.bank.followed(c.i, v.params).is_none() {
                    v.glides.cancel(c.i);
                    v.bank.set(m.id, f32::NAN, v.params);
                } else {
                    v.glides.start(c.i, RESET, before[c.i], before[c.i], c.glide);
                }
            } else {
                start_value(v, c.i, before[c.i], v.bank.default_at(c.i), c.glide);
            }
            applied.push(json!({"key": m.key, "value": "reset"}));
            continue;
        };
        let mut x = target.clamp(m.min, m.max);
        if is_time(c.i) {
            x = target.rem_euclid(24.0);
        }
        if discrete {
            x = x.round().min(controls::options(m.id).len().saturating_sub(1) as f32);
            v.glides.cancel(c.i);
            v.bank.set(m.id, x, v.params);
        } else {
            start_value(v, c.i, before[c.i], x, c.glide);
        }
        let mut a = Map::new();
        a.insert("key".into(), json!(m.key));
        a.insert("value".into(), if discrete { json!(option_of(m.id, x)) } else { r3(x) });
        if (x - target).abs() > 1e-4 * (1.0 + target.abs()) && !(discrete && (x - target).abs() < 0.5) && !is_time(c.i) {
            a.insert("clamped".into(), json!(true));
        }
        applied.push(Value::Object(a));
    }

    // Object moves (immediate; the stage animates its own way).
    let objects = v.objects();
    let mut moved = Vec::new();
    let mut undo_moves = Vec::new();
    for mv in &moves {
        let id = mv.get("object").and_then(Value::as_u64).map(|x| x as u32);
        let pos: Option<Vec<f32>> = mv.get("position").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_f64).map(|x| x as f32).collect());
        let (Some(id), Some(pos)) = (id, pos) else {
            errors.push(format!("move needs `object` and `position`: {mv}"));
            continue;
        };
        let Some(o) = objects.iter().find(|o| o.id == id) else {
            errors.push(format!("unknown object {id}"));
            continue;
        };
        if pos.len() != 3 || pos.iter().any(|x| !x.is_finite()) {
            errors.push(format!("object {id}: position must be [x, y, z]"));
            continue;
        }
        if o.flags & object_flags::MOVABLE == 0 {
            errors.push(format!("object {id} cannot be moved"));
            continue;
        }
        // Keep things on stage: within 40 m, at most 30 m up.
        let (x, y, z) = (pos[0].clamp(-40.0, 40.0), pos[1].clamp(-40.0, 40.0), pos[2].clamp(-5.0, 30.0));
        v.objects.request_move(id, crate::math::Vec3::new(x, y, z));
        undo_moves.push(json!({"object": id, "position": [r3(o.x), r3(o.y), r3(o.z)]}));
        moved.push(id);
    }

    json!({
        "ok": errors.is_empty(),
        "applied": applied,
        "moved": moved,
        "errors": errors,
        "undo": {"glide_s": default_glide.min(2.0), "changes": undo, "moves": undo_moves},
    })
    .to_string()
}

fn start_value(v: &View, i: usize, from: f32, mut to: f32, glide: f32) {
    let m = controls::meta(i);
    if glide <= 0.0 {
        v.glides.cancel(i);
        v.bank.set(m.id, to, v.params);
        return;
    }
    if is_time(i) {
        // The short way round the clock.
        if to - from > 12.0 {
            to -= 24.0;
        } else if from - to > 12.0 {
            to += 24.0;
        }
    }
    v.glides.start(i, VALUE, from, to, glide);
    // The first step lands now (latency), the rest follows the clock.
    v.glides.tick(v.bank, v.params);
}

// ---------------------------------------------------------------------------------------
// Prompt
// ---------------------------------------------------------------------------------------

/// Plain-language meaning of each control for the system prompt.
fn describe(key: &str) -> &'static str {
    match key {
        "water.flow" => "River discharge, the main water knob. 0 = no flow (silent, still), 0.3 = low summer brook, 0.6 = as built, 0.8 = high water, 1 = spring flood. Moves speed, depth and width together.",
        "water.regime" => "Character of all water: still_lake, lake_lapping (small waves), slow_river (deep, quiet, rare gulps), brook_pebbles (babbling), brook_rocks (louder, splashy), rapids (roaring white water), small_waterfall, drips (cave drips), trickle. `scene` = as built.",
        "water.speed" => "Surface velocity, m/s (0.3 lazy, 1 brisk, 2.5 rapids).",
        "water.depth" => "Depth, m. Shallow riffles babble, deep pools are quiet.",
        "water.width" => "Channel width, m.",
        "water.rock_size" => "Median bed rock size, m. Bigger rocks = deeper gurgles and more splashes (0.02 gravel, 0.1 cobbles, 0.5 boulders).",
        "water.aeration" => "Air entrainment: white-water hiss and fizz (0 glassy, 1 foamy, 1.5 waterfall).",
        "water.turbulence" => "How chaotic the flow is: more chirpy bubbles and splash variety.",
        "water.gain" => "Level multiplier of the water sound only (1 = physical). Use for 'quieter river' when the flow itself should stay.",
        "wind.speed" => "Mean wind, m/s. 0 calm, 1.5 light air, 4 gentle breeze, 8 fresh, 12 strong, 17 gale, 25+ storm. Drives leaves, gusts, whistles.",
        "wind.gustiness" => "Relative gust strength (1 = natural, 0.3 steady, 1.8 very gusty).",
        "wind.exposure" => "How exposed the place is to wind (0 sheltered hollow, 1 as built, 2.5 open ridge).",
        "wind.character" => "Preset of the distant wind sources: breeze, forest_roar, storm, power_lines, fence_wire, gap_howl, hollow_tree, whistle.",
        "wind.whistle" => "Level of tonal whistling through gaps and hollows (0 none).",
        "foliage.density" => "Leaf amount multiplier (0 bare branches, 1 as built, 2 lush). Rustle level in wind.",
        "foliage.dryness" => "-1 = by season; 0 fresh green leaves (soft rustle) .. 1 dry crackly autumn leaves.",
        "foliage.flutter" => "Leaf flutter multiplier (aspen-like shimmering).",
        "foliage.creak" => "Branch and trunk creaking multiplier.",
        "foliage.exposure" => "How much wind reaches the crowns (multiplier).",
        "rain.rate" => "Rain rate in the open, mm/h, logarithmic. 0 dry, 0.3 drizzle, 1 light, 4 steady, 10 heavy, 25 downpour, 50+ cloudburst. Wet leaves keep dripping after it stops.",
        "rain.storm" => "Thunderstorm activity 0..1: lightning rate and closeness (0.2 distant rumbles, 0.6 storm nearby, 0.9 overhead). Follows heavy rain unless set.",
        "rain.surface" => "What the open rain patches fall on: light_drizzle, steady_rain, downpour, on_leaves, on_soil, on_rock, on_water, puddles, tin_roof, wood_deck, meadow, under_canopy, under_conifer.",
        "rain.drying" => "How fast leaves and ground dry after rain (1 = real time, 50 = within minutes).",
        "rain.gain" => "Level multiplier of the rain sound only.",
        "birds.activity" => "How much birds sing, 0 silent .. 1 full chorus. Species follow the time of day and season (dawn chorus, owls at night).",
        "birds.orbit" => "Speed of a bird circling the listener, 0 = it stays perched.",
        "master.volume" => "Overall volume, 0.7 default. Change only when asked for louder/quieter overall, or a little for 'calmer'.",
        "env.time_of_day" => "Local time, hours 0..24 (3 a.m. = 3, noon = 12, 20:30 = 20.5). Picks active species; temperature and humidity follow a daily cycle. Dawn ~5.5, dusk ~20.5, night 22..4.",
        "env.temperature" => "Air temperature, °C. Crickets chirp faster when warm and fall silent below ~10 °C.",
        "env.humidity" => "Relative humidity 0..1 (dry air carries high frequencies further).",
        "env.season" => "spring, summer, autumn, winter: which birds sing, leaf state.",
        "insects.activity" => "Crickets and other insects, level multiplier (0 silent, 1 as built, 2 loud). Mostly evening and night.",
        "insects.warmth" => "Microclimate offset for insects, °C.",
        "fire.heat" => "Campfire heat release, kW, logarithmic (5 embers, 20 campfire, 100 big fire, 500 bonfire). Only audible if the scene has a fire.",
        "fire.moisture" => "Wood moisture 0.02..0.6: wet wood hisses and steams, dry wood cracks less often.",
        "fire.resin" => "Resin content 0..1: resinous pine pops and crackles.",
        _ => "",
    }
}

const RULES: &str = "\
You control a real-time, physically based sound simulation of a forest by a river, heard binaurally \
by one listener. The user writes a short request in any language (often Russian). Answer ONLY by \
calling the tool `set_scene` once.

Rules:
- Change only what the request implies. Prefer the macro of a group (water.flow, wind.speed, \
rain.rate, birds.activity, env.time_of_day, insects.activity, master.volume); touch children only \
for specific details (\"bigger rocks\", \"dry leaves\", \"waterfall\").
- Values are absolute targets in the control's units and range; compute them from the current values \
for relative requests. \"a bit\" ≈ 15–25 %, plain comparative ≈ 30–50 %, \"much\" ≈ ×2 or ÷2.
- Presets are given by name. \"reset\" hands a child back to its macro.
- glide_s is how long the change takes: 4 by default, 10–40 for \"slowly\", \"gradually\", \
\"fading\", \"стихает\", 0.5–1 for \"now\", \"suddenly\".
- Plausibility matters: night means few birds (owls) and more insects in warm weather; heavy rain \
comes with wind; a storm needs rain. Keep the rest of the scene as it is.
- Positions: listener at the origin, +x forward, +y left, +z up, metres. Only objects marked \
movable can be moved (moves). Birds live on trees: to bring birds to one side, move trees there or \
raise birds.activity.
- explanation: one short line (≤ 70 characters) in the user's language, saying what you changed, \
plain words, no numbers unless asked. If the request is not about the sound scene, make no changes \
and say so briefly.";

const EXAMPLES: &str = "\
Examples (current values matter; these assume wind.speed 6, rain.rate 4, water.flow 0.7, \
birds.activity 0.8, master.volume 0.8, env.time_of_day 14):

«поспокойнее» →
{\"explanation\":\"Тише ветер и дождь, река медленнее, птицы реже.\",\"glide_s\":6,\"changes\":[\
{\"key\":\"wind.speed\",\"value\":3},{\"key\":\"rain.rate\",\"value\":1.2},{\"key\":\"water.flow\",\"value\":0.5},\
{\"key\":\"birds.activity\",\"value\":0.5},{\"key\":\"master.volume\",\"value\":0.72}]}

«3 a.m.» →
{\"explanation\":\"Three in the morning: owls, crickets, cool air.\",\"glide_s\":8,\"changes\":[\
{\"key\":\"env.time_of_day\",\"value\":3},{\"key\":\"birds.activity\",\"value\":0.3}]}

«гроза» →
{\"explanation\":\"Гроза: ливень, порывистый ветер, гром рядом.\",\"glide_s\":8,\"changes\":[\
{\"key\":\"rain.rate\",\"value\":25},{\"key\":\"rain.storm\",\"value\":0.8},{\"key\":\"wind.speed\",\"value\":11},\
{\"key\":\"wind.gustiness\",\"value\":1.5},{\"key\":\"birds.activity\",\"value\":0.1}]}

«ночь, лёгкий дождь стихает» →
{\"explanation\":\"Ночь, редкий дождь медленно стихает.\",\"glide_s\":30,\"changes\":[\
{\"key\":\"env.time_of_day\",\"value\":23.5},{\"key\":\"rain.rate\",\"value\":0.2},{\"key\":\"rain.storm\",\"value\":0},\
{\"key\":\"birds.activity\",\"value\":0.25},{\"key\":\"wind.speed\",\"value\":2}]}

«как будто у водопада» →
{\"explanation\":\"Рядом шумит небольшой водопад.\",\"changes\":[\
{\"key\":\"water.regime\",\"value\":\"small_waterfall\"},{\"key\":\"water.flow\",\"value\":0.9},\
{\"key\":\"water.aeration\",\"value\":1.3}]}

«больше птиц слева» (tree 4 is movable at [6, -3, 0]) →
{\"explanation\":\"Больше птиц, дерево с птицами слева.\",\"changes\":[\
{\"key\":\"birds.activity\",\"value\":0.95}],\"moves\":[{\"object\":4,\"position\":[4,6,0]}]}

«верни реку как была» →
{\"explanation\":\"Река снова как в сцене.\",\"changes\":[{\"key\":\"water.regime\",\"value\":\"scene\"},\
{\"key\":\"water.flow\",\"value\":0.6},{\"key\":\"water.aeration\",\"value\":\"reset\"}]}

«what's the weather in Paris?» →
{\"explanation\":\"I can only change the sound scene.\",\"changes\":[]}";

/// Compact one-line-per-control text of the current values (the dynamic context).
fn context_text(v: &View) -> String {
    let state: Value = serde_json::from_str(&state_json(v)).unwrap_or(Value::Null);
    let mut s = String::from("Current scene\n");
    let mut group = "";
    for c in state["controls"].as_array().into_iter().flatten() {
        let g = c["group"].as_str().unwrap_or("");
        if g != group {
            let _ = writeln!(s, "[{g}]");
            group = g;
        }
        let key = c["key"].as_str().unwrap_or("");
        let unit = c["unit"].as_str().unwrap_or("");
        let val = match c["option"].as_str() {
            Some(o) => o.to_string(),
            None => format!("{}{}{}", c["value"], if unit.is_empty() { "" } else { " " }, unit),
        };
        let range = if c["options"].is_array() { String::new() } else { format!(" (range {}..{})", c["min"], c["max"]) };
        let flag = if c["overridden"] == json!(true) { ", set explicitly" } else if c["parent"].is_string() { ", follows macro" } else { "" };
        let _ = writeln!(s, "{key} = {val}{range}{flag}");
    }
    let e = &state["environment"];
    let _ = writeln!(
        s,
        "[readings] playing {}, rain falling now {} mm/h, leaf wetness {}, ground {}",
        e["playing"], e["rain_now_mm_h"], e["leaf_wetness"], e["ground"]
    );
    let _ = writeln!(s, "[objects] {}", state["frame"].as_str().unwrap_or(""));
    for o in state["objects"].as_array().into_iter().flatten() {
        let tags = [("movable", "movable"), ("moving", "flying")].iter().filter(|(k, _)| o[*k] == json!(true)).map(|(_, t)| *t).collect::<Vec<_>>().join(", ");
        let _ = writeln!(s, "{} {} at {}{}", o["kind"].as_str().unwrap_or(""), o["id"], o["pos"], if tags.is_empty() { String::new() } else { format!(" ({tags})") });
    }
    s
}

fn system_text() -> String {
    let mut s = String::from(RULES);
    s.push_str("\n\nControls (key: meaning):\n");
    for i in 0..COUNT {
        let m = controls::meta(i);
        let _ = writeln!(s, "- {}{}: {}", m.key, if m.unit.is_empty() { String::new() } else { format!(" [{}]", m.unit) }, describe(m.key));
    }
    s.push('\n');
    s.push_str(EXAMPLES);
    s
}

/// JSON schema of the `set_scene` tool input, generated from the control tree.
pub fn tool_schema() -> Value {
    let keys: Vec<&str> = (0..COUNT).map(|i| controls::meta(i).key).collect();
    let mut names: Vec<&str> = vec!["reset"];
    for i in 0..COUNT {
        let m = controls::meta(i);
        if m.flags & flags::DISCRETE != 0 {
            for o in controls::options(m.id) {
                if !names.contains(&o) {
                    names.push(o);
                }
            }
        }
    }
    json!({
        "type": "object",
        "properties": {
            "explanation": {"type": "string", "description": "One short line in the user's language: what changed."},
            "glide_s": {"type": "number", "description": "Seconds the change takes (default 4)."},
            "changes": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "key": {"type": "string", "enum": keys},
                        "value": {
                            "anyOf": [
                                {"type": "number"},
                                {"type": "string", "enum": names}
                            ],
                            "description": "Target in the control's units, a preset name for preset controls, or \"reset\"."
                        }
                    },
                    "required": ["key", "value"],
                    "additionalProperties": false
                }
            },
            "moves": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "object": {"type": "integer"},
                        "position": {"type": "array", "items": {"type": "number"}}
                    },
                    "required": ["object", "position"],
                    "additionalProperties": false
                }
            }
        },
        "required": ["explanation", "changes"],
        "additionalProperties": false
    })
}

/// `{"system", "context", "tool": {"name", "description", "input_schema"}}`. `system` is
/// static (cacheable); `context` is the current state, sent with the user's request.
pub(crate) fn prompt_json(v: &View) -> String {
    json!({
        "system": system_text(),
        "context": context_text(v),
        "tool": {
            "name": "set_scene",
            "description": "Change the sound scene: absolute targets for controls, optional object moves, and a one-line explanation for the user.",
            "input_schema": tool_schema(),
        },
    })
    .to_string()
}

/// Copies `s` NUL-terminated into `out` if it fits; returns its length in bytes either way.
///
/// # Safety
/// `out` must be NULL or point to `max` writable bytes.
pub(crate) unsafe fn write_c(s: &str, out: *mut std::ffi::c_char, max: u32) -> u32 {
    let n = s.len();
    if !out.is_null() && (max as usize) > n {
        let dst = std::slice::from_raw_parts_mut(out as *mut u8, n + 1);
        dst[..n].copy_from_slice(s.as_bytes());
        dst[n] = 0;
    }
    n as u32
}

#[cfg(test)]
mod tests {
    use crate::controls::id;
    use crate::engine::Engine;
    use serde_json::{json, Value};

    fn engine() -> Engine {
        Engine::new(48_000.0)
    }

    fn parse(s: &str) -> Value {
        serde_json::from_str(s).expect("valid JSON")
    }

    fn render(e: &Engine, seconds: f32) {
        let n = (48_000.0 * seconds) as usize;
        let mut l = vec![0.0; n];
        let mut r = vec![0.0; n];
        e.render(&mut l, &mut r);
    }

    #[test]
    fn state_lists_every_control() {
        let e = engine();
        let s = parse(&e.state_json());
        let c = s["controls"].as_array().unwrap();
        assert_eq!(c.len(), crate::controls::COUNT);
        let flow = c.iter().find(|x| x["key"] == "water.flow").unwrap();
        assert_eq!(flow["macro"], true);
        assert_eq!(flow["min"], 0.0);
        let regime = c.iter().find(|x| x["key"] == "water.regime").unwrap();
        assert_eq!(regime["option"], "scene");
        assert!(regime["options"].as_array().unwrap().iter().any(|o| o == "rapids"));
        let depth = c.iter().find(|x| x["key"] == "water.depth").unwrap();
        assert_eq!(depth["parent"], "water.flow");
        assert_eq!(depth["overridden"], false);
        assert_eq!(depth["unit"], "m");
        assert!(!s["objects"].as_array().unwrap().is_empty());
        assert!(s["objects"].as_array().unwrap().iter().any(|o| o["kind"] == "tree"));
    }

    #[test]
    fn immediate_patch_clamps_and_rejects_unknown_keys() {
        let e = engine();
        let r = parse(&e.apply_patch_json(
            r#"{"glide_s":0,"changes":[{"key":"water.flow","value":0.3},{"key":"rain.rate","value":500},{"key":"water.regime","value":"rapids"},{"key":"nope.x","value":1}]}"#,
        ));
        assert_eq!(r["ok"], false);
        assert!(r["errors"][0].as_str().unwrap().contains("nope.x"));
        assert_eq!(e.control(id::WATER_FLOW), 0.3);
        assert_eq!(e.control(id::RAIN_RATE), 100.0);
        assert_eq!(e.control(id::WATER_REGIME), 6.0);
        let applied = r["applied"].as_array().unwrap();
        assert_eq!(applied.len(), 3);
        assert!(applied.iter().any(|a| a["key"] == "rain.rate" && a["clamped"] == true));
        assert!(applied.iter().any(|a| a["value"] == "rapids"));
    }

    #[test]
    fn bad_input_reports_errors() {
        let e = engine();
        for bad in ["", "{", "42", r#"{"changes":[{"value":1}]}"#, r#"{"changes":[{"key":"water.regime","value":"jacuzzi"}]}"#, r#"{"changes":[{"key":"wind.speed","value":[1]}]}"#] {
            let r = parse(&e.apply_patch_json(bad));
            assert_eq!(r["ok"], false, "{bad}");
            assert!(!r["errors"].as_array().unwrap().is_empty(), "{bad}");
        }
    }

    #[test]
    fn changes_glide_instead_of_jumping() {
        let e = engine();
        e.set_param(crate::param::PLAYING, 1.0);
        let start = e.control(id::WIND_SPEED);
        let r = parse(&e.apply_patch_json(r#"{"glide_s":0.4,"changes":[{"key":"wind.speed","value":15}]}"#));
        assert_eq!(r["ok"], true);
        // Not there yet...
        let early = e.control(id::WIND_SPEED);
        assert!(early < 15.0 - 1.0, "jumped to {early}");
        // ...moves monotonically...
        std::thread::sleep(std::time::Duration::from_millis(200));
        render(&e, 0.01);
        let mid = e.control(id::WIND_SPEED);
        assert!(mid > start && mid < 15.0, "{start} {mid}");
        // ...and lands.
        std::thread::sleep(std::time::Duration::from_millis(300));
        render(&e, 0.01);
        assert!((e.control(id::WIND_SPEED) - 15.0).abs() < 1e-3);
        assert_eq!(parse(&e.state_json())["gliding"], 0);
    }

    #[test]
    fn time_of_day_glides_the_short_way() {
        let e = engine();
        e.apply_patch_json(r#"{"glide_s":0,"changes":[{"key":"env.time_of_day","value":23}]}"#);
        e.apply_patch_json(r#"{"glide_s":0.3,"changes":[{"key":"env.time_of_day","value":2}]}"#);
        std::thread::sleep(std::time::Duration::from_millis(150));
        let mid = parse(&e.state_json())["controls"].as_array().unwrap().iter().find(|c| c["key"] == "env.time_of_day").unwrap()["value"].as_f64().unwrap();
        assert!(!(3.0..22.0).contains(&mid), "went the long way: {mid}");
        std::thread::sleep(std::time::Duration::from_millis(250));
        let _ = e.state_json();
        assert!((e.control(id::ENV_TIME) - 2.0).abs() < 1e-3);
    }

    #[test]
    fn slider_cancels_glide() {
        let e = engine();
        e.apply_patch_json(r#"{"glide_s":5,"changes":[{"key":"birds.activity","value":1}]}"#);
        e.set_control(id::BIRDS_ACTIVITY, 0.1);
        std::thread::sleep(std::time::Duration::from_millis(50));
        let _ = e.state_json();
        assert_eq!(e.control(id::BIRDS_ACTIVITY), 0.1);
    }

    #[test]
    fn reset_hands_child_back_and_undo_restores() {
        let e = engine();
        let follow = e.control(id::WATER_DEPTH);
        let flow = e.control(id::WATER_FLOW);
        let r = parse(&e.apply_patch_json(r#"{"glide_s":0,"changes":[{"key":"water.depth","value":2.0},{"key":"water.flow","value":0.9}]}"#));
        assert_eq!(e.control(id::WATER_DEPTH), 2.0);
        // Undo: the macro back to its value, the child back to its macro.
        let undo = &r["undo"];
        assert!(undo["changes"].as_array().unwrap().iter().any(|c| c["key"] == "water.depth" && c["value"] == "reset"));
        let mut u = undo.clone();
        u["glide_s"] = json!(0);
        let r2 = parse(&e.apply_patch_json(&u.to_string()));
        assert_eq!(r2["ok"], true, "{r2}");
        assert!((e.control(id::WATER_FLOW) - flow).abs() < 1e-3);
        assert!((e.control(id::WATER_DEPTH) - follow).abs() < 1e-4);
        assert_eq!(parse(&e.state_json())["controls"][crate::controls::index_of_key("water.depth").unwrap()]["overridden"], false);

        // A gliding reset lands on the curve and clears the override.
        e.apply_patch_json(r#"{"glide_s":0,"changes":[{"key":"water.depth","value":3.0}]}"#);
        e.apply_patch_json(r#"{"glide_s":0.2,"changes":[{"key":"water.depth","value":"reset"}]}"#);
        std::thread::sleep(std::time::Duration::from_millis(300));
        let s = parse(&e.state_json());
        assert_eq!(s["controls"][crate::controls::index_of_key("water.depth").unwrap()]["overridden"], false);
        assert!((e.control(id::WATER_DEPTH) - follow).abs() < 1e-4);
    }

    #[test]
    fn moves_movable_objects_only() {
        let e = engine();
        let s = parse(&e.state_json());
        let objs = s["objects"].as_array().unwrap();
        let tree = objs.iter().find(|o| o["movable"] == true).expect("a movable object");
        let id = tree["id"].as_u64().unwrap();
        let r = parse(&e.apply_patch_json(&json!({"moves": [{"object": id, "position": [1.0, 7.0, 0.0]}, {"object": 99999, "position": [0, 0, 0]}]}).to_string()));
        assert_eq!(r["moved"], json!([id]));
        assert_eq!(r["errors"].as_array().unwrap().len(), 1);
        assert_eq!(r["undo"]["moves"][0]["object"], json!(id));
        render(&e, 0.01);
        let s = parse(&e.state_json());
        let moved = s["objects"].as_array().unwrap().iter().find(|o| o["id"] == json!(id)).unwrap().clone();
        assert_eq!(moved["pos"][1], 7.0);
    }

    #[test]
    fn prompt_has_schema_and_context() {
        let e = engine();
        let p = parse(&e.prompt_json());
        let sys = p["system"].as_str().unwrap();
        for i in 0..crate::controls::COUNT {
            let k = crate::controls::meta(i).key;
            assert!(sys.contains(&format!("- {k}")), "{k} missing from the system prompt");
            assert!(!super::describe(k).is_empty(), "{k} has no description");
        }
        assert!(p["context"].as_str().unwrap().contains("water.flow = 0.6"));
        let schema = &p["tool"]["input_schema"];
        let keys = schema["properties"]["changes"]["items"]["properties"]["key"]["enum"].as_array().unwrap();
        assert_eq!(keys.len(), crate::controls::COUNT);
        let names = &schema["properties"]["changes"]["items"]["properties"]["value"]["anyOf"][1]["enum"];
        assert!(names.as_array().unwrap().iter().any(|n| n == "small_waterfall"));
    }

    /// Mock provider: canned model replies (the shape a real adapter returns) go through the
    /// same path as live ones and leave a valid scene.
    #[test]
    fn mock_provider_replies_apply() {
        let e = engine();
        let sys = parse(&e.prompt_json())["system"].as_str().unwrap().to_string();
        // Every few-shot example in the prompt is itself a valid patch.
        let mut examples = 0;
        for line in sys.split("→\n").skip(1) {
            let reply = line.split("\n\n").next().unwrap();
            let r = parse(&e.apply_patch_json(reply));
            let moves_only_err = r["errors"].as_array().unwrap().iter().all(|x| x.as_str().unwrap().contains("object"));
            assert!(r["ok"] == true || moves_only_err, "{reply}: {r}");
            assert!(parse(reply)["explanation"].is_string());
            examples += 1;
        }
        assert!(examples >= 6);
        // «поспокойнее» lowers wind, rain and flow.
        let e = engine();
        e.apply_patch_json(r#"{"glide_s":0,"changes":[{"key":"wind.speed","value":6},{"key":"rain.rate","value":4},{"key":"water.flow","value":0.7}]}"#);
        let calm = r#"{"explanation":"Тише.","glide_s":0,"changes":[{"key":"wind.speed","value":3},{"key":"rain.rate","value":1.2},{"key":"water.flow","value":0.5}]}"#;
        assert_eq!(parse(&e.apply_patch_json(calm))["ok"], true);
        assert!(e.control(id::WIND_SPEED) < 6.0 && e.control(id::RAIN_RATE) < 4.0 && e.control(id::WATER_FLOW) < 0.7);
    }
}
