//! Prompt control: the scene as a JSON state for a language model, and a JSON patch back.
//!
//! * [`state_json`] — every control (key, value, unit, range, preset names, overridden flag),
//!   a few environment readings and a compact list of scene objects.
//! * [`apply_patch`] — `{"glide_s": 4, "changes": [{"key": "water.flow", "value": 0.3}, …],
//!   "moves": [{"object": 3, "position": [x, y, z]}]}`. Keys are the control keys the sliders
//!   use plus switches (`flight.on`); values are clamped, presets may be given by name,
//!   `"reset"` hands a child back to its macro. Parsing is tolerant of what language models
//!   write: aliases and near-miss keys, `<group>.level` ↔ `<group>.gain`, loose preset names
//!   (also Russian place words for landscapes), numbers with units, `delta` / `factor`, a
//!   `{key: value}` map, JSON inside prose. Continuous controls glide (eased) instead of
//!   jumping. The reply carries an `undo` patch that restores what the patch touched.
//! * [`scene_facts`] — "what is around": place, time, weather, water, singing birds relative to
//!   the head, what is loud; structured and as short Russian sentences (in the state and the
//!   prompt context).
//! * [`prompt_json`] — the static system prompt (what every control means physically, rules,
//!   few-shot examples), the dynamic context (current values) and the tool schema generated
//!   from the control tree, so a provider adapter only has to transport them.
//!
//! Glides live in [`Glides`]: atomic slots, one per control, advanced by the audio thread
//! every block and by readers (so they land even while nothing renders). Wall-clock based.
//! Writers serialize control transactions; audio/read ticks skip a busy transaction.

use std::fmt::Write as _;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

use serde_json::{json, Map, Value};

use crate::controls::{self, flags, ControlBank, COUNT};
use crate::engine::AtomicF32;
use crate::engine::{SourceInfo, SourceKind};
use crate::mix::MixInfo;
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
    editing: AtomicBool,
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
            editing: AtomicBool::new(false),
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

    /// Control writers may wait; the audio thread only ever uses `try_edit`.
    pub(crate) fn edit(&self) -> EditGuard<'_> {
        loop {
            if let Some(guard) = self.try_edit() { return guard; }
            std::thread::yield_now();
        }
    }

    pub(crate) fn try_edit(&self) -> Option<EditGuard<'_>> {
        self.editing.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .ok().map(|_| EditGuard(&self.editing))
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

    fn target(&self, i: usize) -> Option<Value> {
        let slot = &self.slots[i];
        match slot.state.load(Ordering::Acquire) & 3 {
            VALUE => Some(json!(wrap(i, slot.to.load()))),
            RESET => Some(json!("reset")),
            _ => None,
        }
    }

    /// Moves every gliding control to its current eased value. Any thread, real-time safe.
    pub(crate) fn tick(&self, bank: &ControlBank, params: &[AtomicF32]) {
        let Some(_guard) = self.try_edit() else { return };
        self.tick_locked(bank, params);
    }

    pub(crate) fn tick_locked(&self, bank: &ControlBank, params: &[AtomicF32]) {
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

pub(crate) struct EditGuard<'a>(&'a AtomicBool);

impl Drop for EditGuard<'_> {
    fn drop(&mut self) { self.0.store(false, Ordering::Release); }
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
    /// Sources, objects on stage and loudness for the scene description.
    pub live: Live,
}

impl View<'_> {
    fn values(&self) -> [f32; COUNT] {
        self.bank.state(self.params).v
    }

    fn objects(&self) -> Vec<ObjectInfo> {
        let mut out = vec![ObjectInfo::default(); MAX_OBJECTS];
        let n = self.objects.read_requested(&mut out);
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
        if let Some(target) = v.glides.target(i) {
            o.insert("target".into(), target);
        }
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
        if m.flags & LEVEL_FLAG != 0 || m.key.ends_with(".level") {
            // Loudness of one group (dB), as opposed to its physical knobs.
            o.insert("loudness".into(), json!(true));
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
    let switches: Vec<Value> = Switch::ALL.iter().map(|s| json!({"key": s.key(), "value": if s.on(v) { "on" } else { "off" }, "options": ["off", "on"]})).collect();
    let (facts, parts) = scene_facts(v);
    json!({
        "controls": ctl,
        "switches": switches,
        "environment": env,
        "scene": facts,
        "summary_ru": summary_text(&parts),
        "summary_topics_ru": parts.iter().map(|(k, s)| (k.to_string(), json!(s))).collect::<Map<String, Value>>(),
        "objects": objects,
        "frame": "listener at the origin; +x forward, +y left, +z up; metres",
        "gliding": v.glides.active(),
    })
    .to_string()
}

// ---------------------------------------------------------------------------------------
// Patch: tolerant parsing (aliases, fuzzy keys and presets, relative values, switches)
// ---------------------------------------------------------------------------------------

/// Sound-director per-group loudness controls (`<group>.level`, dB) carry this flag.
const LEVEL_FLAG: u32 = 1 << 4;

/// On/off switches a patch may flip besides the controls (they are engine parameters).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Switch {
    /// Flight mode (`ASMR_PARAM_FLIGHT`): the listener travels through generated landscapes.
    Flight,
}

impl Switch {
    pub(crate) const ALL: [Switch; 1] = [Switch::Flight];

    pub(crate) fn key(self) -> &'static str {
        match self {
            Switch::Flight => "flight.on",
        }
    }

    fn param(self) -> u32 {
        match self {
            Switch::Flight => crate::engine::param::FLIGHT,
        }
    }

    fn on(self, v: &View) -> bool {
        v.params[self.param() as usize].load() > 0.5
    }
}

/// How a value given for an alias maps onto the control it resolved to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Conv {
    Same,
    /// The model wrote decibels, the control is a linear gain (`water.level` → `water.gain`).
    DbToGain,
    /// The model wrote a linear gain, the control is in decibels (`water.gain` → `water.level`).
    GainToDb,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Resolved {
    Control(usize, Conv),
    Switch(Switch),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Op {
    Abs(f32),
    Add(f32),
    Mul(f32),
    Reset,
}

struct Change {
    i: usize,
    op: Op,
    glide: f32,
}

/// Errors in English (fed back to a model) and Russian (shown to the user).
#[derive(Default)]
struct Errs {
    en: Vec<String>,
    ru: Vec<String>,
}

impl Errs {
    fn push(&mut self, en: String, ru: String) {
        self.en.push(en);
        self.ru.push(ru);
    }
}

/// Lowercase, words joined by `_`, no quotes or stray punctuation.
fn norm(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.trim().trim_matches(|c| c == '"' || c == '\'' || c == '`').chars().flat_map(char::to_lowercase) {
        let ch = match ch {
            ' ' | '-' | '/' | '\\' => '_',
            'ё' => 'е',
            c => c,
        };
        if ch == '_' && (out.is_empty() || out.ends_with('_') || out.ends_with('.')) {
            continue;
        }
        if ch == '.' && out.ends_with('_') {
            out.pop();
        }
        out.push(ch);
    }
    while out.ends_with('_') || out.ends_with('.') {
        out.pop();
    }
    out
}

fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for i in 1..=a.len() {
        cur[0] = i;
        for j in 1..=b.len() {
            let sub = prev[j - 1] + usize::from(a[i - 1] != b[j - 1]);
            cur[j] = sub.min(prev[j] + 1).min(cur[j - 1] + 1);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// The closest of `names` within a small edit distance, if it is the only one that close.
fn fuzzy<'a>(raw: &str, names: impl Iterator<Item = &'a str>) -> Option<&'a str> {
    let limit = if raw.chars().count() <= 5 { 1 } else { 2 };
    let mut best: Option<(&str, usize)> = None;
    let mut tie = false;
    for n in names {
        let d = levenshtein(raw, n);
        if d > limit {
            continue;
        }
        match best {
            Some((_, b)) if d > b => {}
            Some((_, b)) if d == b => tie = true,
            _ => {
                best = Some((n, d));
                tie = false;
            }
        }
    }
    if tie { None } else { best.map(|(n, _)| n) }
}

fn closest<'a>(raw: &str, names: impl Iterator<Item = &'a str>) -> Option<&'a str> {
    names.min_by_key(|n| levenshtein(raw, n))
}

fn keys() -> impl Iterator<Item = &'static str> {
    (0..COUNT).map(|i| controls::meta(i).key)
}

/// Group words a model (or a person) may use instead of the control tree's group names.
fn group_alias(g: &str) -> &str {
    match g {
        "river" | "stream" | "brook" | "creek" | "lake" | "sea" | "waves" | "waterfall" | "река" | "вода" => "water",
        "bird" | "birdsong" | "songbirds" | "птицы" => "birds",
        "insect" | "crickets" | "cricket" | "cicadas" | "bugs" | "насекомые" | "сверчки" => "insects",
        "campfire" | "bonfire" | "костер" => "fire",
        "leaves" | "trees" | "forest" | "листва" => "foliage",
        "thunder" | "storm" | "weather" | "дождь" => "rain",
        "time" | "environment" | "world" | "climate" => "env",
        "volume" | "overall" | "global" | "output" => "master",
        "fly" | "flying" | "landscape" | "land" | "terrain" | "полет" => "flight",
        "ветер" => "wind",
        g => g,
    }
}

/// Whole-key aliases.
fn alias(k: &str) -> Option<&'static str> {
    Some(match k {
        "time" | "time_of_day" | "hour" | "clock" | "env.time" | "env.hour" | "время" => "env.time_of_day",
        "temperature" | "temp" | "env.temp" => "env.temperature",
        "humidity" => "env.humidity",
        "season" => "env.season",
        "volume" | "master" | "master.gain" | "master.level" | "loudness_overall" | "громкость" => "master.volume",
        "biome" | "landscape" | "place" | "location" | "terrain" | "flight.landscape" | "flight.place" | "flight.terrain" => "flight.biome",
        "thunder" | "storm" | "rain.thunder" | "thunderstorm" | "rain.thunderstorm" => "rain.storm",
        "speed" | "flight.velocity" => "flight.speed",
        "altitude" | "height" | "flight.height" => "flight.altitude",
        "regime" | "water.type" | "water.character" | "water.preset" | "water.mode" => "water.regime",
        "focus.water" | "focus.wind" | "focus.birds" | "focus.rain" | "focus.insects" | "focus.fire" => return None,
        _ => return None,
    })
}

fn is_switch_word(k: &str) -> Option<Switch> {
    match k {
        "flight.on" | "flight" | "flight.enabled" | "flight.enable" | "flight.active" | "flight.mode" | "flight_mode" | "flying" | "fly" | "flight.state"
        | "flight.flying" | "flight.toggle" | "param.flight" | "params.flight" => Some(Switch::Flight),
        _ => None,
    }
}

/// Resolves what a patch entry's key means; `Err` carries (English, Russian) messages.
fn resolve_key(raw: &str) -> Result<Resolved, (String, String)> {
    if let Some(i) = controls::index_of_key(raw) {
        return Ok(Resolved::Control(i, Conv::Same));
    }
    let mut k = norm(raw);
    for prefix in ["controls.", "control.", "scene.", "params.", "set."] {
        if let Some(rest) = k.strip_prefix(prefix) {
            if !rest.is_empty() && is_switch_word(&k).is_none() {
                k = rest.to_string();
            }
        }
    }
    if let Some(i) = controls::index_of_key(&k) {
        return Ok(Resolved::Control(i, Conv::Same));
    }
    if let Some(s) = is_switch_word(&k) {
        return Ok(Resolved::Switch(s));
    }
    if let Some(a) = alias(&k) {
        if let Some(i) = controls::index_of_key(a) {
            return Ok(Resolved::Control(i, Conv::Same));
        }
    }
    // `focus.water` → `mix.focus.water`.
    if let Some(i) = controls::index_of_key(&format!("mix.{k}")) {
        return Ok(Resolved::Control(i, Conv::Same));
    }
    let (g, rest) = match k.split_once('.') {
        Some((g, r)) => (group_alias(g).to_string(), r.to_string()),
        None => (group_alias(&k).to_string(), String::new()),
    };
    if !rest.is_empty() {
        let full = format!("{g}.{rest}");
        if let Some(i) = controls::index_of_key(&full) {
            return Ok(Resolved::Control(i, Conv::Same));
        }
        if let Some(s) = is_switch_word(&full) {
            return Ok(Resolved::Switch(s));
        }
        if let Some(a) = alias(&full) {
            if let Some(i) = controls::index_of_key(a) {
                return Ok(Resolved::Control(i, Conv::Same));
            }
        }
        // Loudness of one group: `<group>.level` (dB) if the sound director has it, else the
        // linear `<group>.gain`.
        let loud = matches!(rest.as_str(), "level" | "volume" | "loudness" | "gain" | "db" | "level_db" | "loud" | "громкость");
        if loud && g != "mix" && g != "master" {
            let gain_word = rest == "gain";
            if let Some(i) = controls::index_of_key(&format!("{g}.level")) {
                return Ok(Resolved::Control(i, if gain_word { Conv::GainToDb } else { Conv::Same }));
            }
            if let Some(i) = controls::index_of_key(&format!("{g}.gain")) {
                return Ok(Resolved::Control(i, if gain_word { Conv::Same } else { Conv::DbToGain }));
            }
            if let Some(i) = (0..COUNT).find(|&i| controls::meta(i).flags & flags::MACRO != 0 && controls::meta(i).key.starts_with(&format!("{g}."))) {
                let mk = controls::meta(i).key;
                return Err((
                    format!("unknown key `{raw}`: `{g}` has no loudness control; use its amount `{mk}` instead"),
                    format!("у «{g}» нет отдельной громкости — меняется количество ({mk})"),
                ));
            }
        }
    } else {
        // A bare group: its macro.
        if let Some(i) = (0..COUNT).find(|&i| {
            let m = controls::meta(i);
            m.flags & flags::MACRO != 0 && controls::group_name(m.group).starts_with(g.as_str()) && m.key.starts_with(&format!("{g}."))
        }) {
            return Ok(Resolved::Control(i, Conv::Same));
        }
    }
    // A unique key ending (`rock_size`, `time_of_day`, `curviness`).
    let tail = if rest.is_empty() { k.clone() } else { rest.clone() };
    let mut ends = keys().filter(|key| key.ends_with(&format!(".{tail}")));
    if let (Some(one), None) = (ends.next(), ends.next()) {
        return Ok(Resolved::Control(controls::index_of_key(one).unwrap_or(0), Conv::Same));
    }
    if let Some(one) = fuzzy(&k, keys().chain(Switch::ALL.iter().map(|s| s.key()))) {
        if let Some(i) = controls::index_of_key(one) {
            return Ok(Resolved::Control(i, Conv::Same));
        }
        if let Some(s) = Switch::ALL.iter().find(|s| s.key() == one) {
            return Ok(Resolved::Switch(*s));
        }
    }
    let near = closest(&k, keys()).unwrap_or("");
    Err((format!("unknown key `{raw}` (closest: {near})"), format!("нет параметра «{raw}» (ближе всего {near})")))
}

/// Words that pick a landscape preset, best candidate first. Only names that exist count.
const PLACE_WORDS: &[(&[&str], &[&str])] = &[
    (&["desert", "dune", "sand", "sahara", "пустын", "дюн", "песк", "пески"], &["desert"]),
    (&["snow", "снег", "снеж", "alpine", "glacier", "ледник"], &["snowy_mountains", "highlands"]),
    (&["mountain", "hill", "ridge", "гор", "скал", "нагорь"], &["highlands", "snowy_mountains"]),
    (&["sea", "ocean", "coast", "beach", "shore_sea", "мор", "океан", "побереж", "пляж"], &["sea_coast", "lake_shore"]),
    (&["lake", "pond", "озер", "пруд"], &["lake_shore", "taiga"]),
    (&["big_river", "wide_river", "большая_рек", "широк"], &["big_river", "river_valley"]),
    (&["river", "valley", "brook", "рек", "долин", "ручь", "ручей"], &["river_valley", "big_river"]),
    (&["taiga", "conifer", "spruce", "pine", "тайг", "хвой", "ель", "елов", "сосн"], &["taiga"]),
    (&["meadow", "field", "steppe", "prairie", "grass", "луг", "поле", "степ", "трав"], &["meadows"]),
    (&["oak", "birch", "deciduous", "broadleaf", "дуб", "берез", "листв"], &["broadleaf"]),
    (&["forest", "wood", "лес"], &["mixed"]),
];

/// Matches a preset name loosely: exact, as-built words, prefix / substring, edit distance,
/// then (landscapes only) place words in English or Russian.
fn match_option(id: u32, raw: &str) -> Result<usize, (String, String)> {
    let opts = controls::options(id);
    let s = norm(raw);
    if let Some(k) = opts.iter().position(|o| *o == s) {
        return Ok(k);
    }
    if matches!(s.as_str(), "as_built" | "default" | "original" | "normal" | "как_было" | "как_в_сцене") {
        if let Some(k) = opts.iter().position(|o| *o == "scene") {
            return Ok(k);
        }
    }
    let containing: Vec<usize> = (0..opts.len()).filter(|&k| s.len() >= 3 && (opts[k].contains(s.as_str()) || s.contains(opts[k]))).collect();
    if containing.len() == 1 || (!containing.is_empty() && id != controls::id::FLIGHT_BIOME) {
        return Ok(containing[0]);
    }
    if let Some(one) = fuzzy(&s, opts.iter().copied()) {
        return Ok(opts.iter().position(|o| *o == one).unwrap_or(0));
    }
    if id == controls::id::FLIGHT_BIOME {
        for (words, picks) in PLACE_WORDS {
            if words.iter().any(|w| s.contains(w)) {
                if let Some(k) = picks.iter().find_map(|p| opts.iter().position(|o| o == p)) {
                    return Ok(k);
                }
            }
        }
    }
    let key = controls::index_of_id(id).map_or("", |i| controls::meta(i).key);
    Err((
        format!("{key}: unknown option `{raw}` (one of {})", opts.join(", ")),
        format!("у «{key}» нет варианта «{raw}» (есть: {})", opts.join(", ")),
    ))
}

/// A number in a string: "0.3", "-6 dB", "25 мм/ч", "23:30" (hours), "30%" (= 0.3).
fn number_in(s: &str) -> Option<f32> {
    let t = s.trim().replace(',', ".");
    if let Some((h, m)) = t.split_once(':') {
        if let (Ok(h), Ok(m)) = (h.trim().parse::<f32>(), m.trim().trim_end_matches(|c: char| !c.is_ascii_digit()).parse::<f32>()) {
            return Some(h + m / 60.0);
        }
    }
    let end = t.char_indices().find(|&(i, c)| !(c.is_ascii_digit() || c == '.' || ((c == '-' || c == '+') && i == 0) || c == 'e' && i > 0)).map_or(t.len(), |(i, _)| i);
    let x: f32 = t[..end].parse().ok()?;
    Some(if t[end..].trim_start().starts_with('%') { x / 100.0 } else { x })
}

fn as_f32(v: &Value) -> Option<f32> {
    match v {
        Value::Number(n) => n.as_f64().map(|x| x as f32),
        Value::String(s) => number_in(s),
        _ => None,
    }
}

fn parse_value(i: usize, val: &Value) -> Result<Op, (String, String)> {
    let m = controls::meta(i);
    let discrete = m.flags & flags::DISCRETE != 0;
    let bad = || (format!("{}: expected a number, an option name or \"reset\", got {val}", m.key), format!("у «{}» непонятное значение {val}", m.key));
    match val {
        Value::Null => Ok(Op::Reset),
        Value::String(s) if matches!(norm(s).as_str(), "reset" | "auto" | "follow" | "unset" | "сброс") => Ok(Op::Reset),
        Value::String(s) if discrete => match number_in(s) {
            Some(x) if s.trim().chars().all(|c| c.is_ascii_digit() || c == '.') => Ok(Op::Abs(x)),
            _ => {
                let o = controls::options(m.id);
                match norm(s).as_str() {
                    "on" | "true" | "yes" if o.first() == Some(&"off") => Ok(Op::Abs(1.0)),
                    "off" | "false" | "no" | "none" if o.first() == Some(&"off") => Ok(Op::Abs(0.0)),
                    _ => match_option(m.id, s).map(|k| Op::Abs(k as f32)),
                }
            }
        },
        Value::String(s) => number_in(s).map(Op::Abs).ok_or_else(|| (format!("{}: expected a number, got `{s}`", m.key), format!("у «{}» ждали число, а пришло «{s}»", m.key))),
        Value::Number(_) => as_f32(val).map(Op::Abs).ok_or_else(bad),
        Value::Bool(b) if discrete && controls::options(m.id).first() == Some(&"off") => Ok(Op::Abs(if *b { 1.0 } else { 0.0 })),
        Value::Object(o) => {
            // {"value": …} nested, or {"delta": …}.
            if let Some(x) = o.get("value") {
                return parse_value(i, x);
            }
            parse_relative(o).ok_or_else(bad)
        }
        _ => Err(bad()),
    }
}

/// `delta` / `by` / `add` (in the control's units) or `factor` / `scale` / `times`.
fn parse_relative(o: &Map<String, Value>) -> Option<Op> {
    for k in ["delta", "by", "add", "change", "offset"] {
        if let Some(x) = o.get(k).and_then(as_f32) {
            return Some(Op::Add(x));
        }
    }
    for k in ["factor", "scale", "times", "multiply", "mul"] {
        if let Some(x) = o.get(k).and_then(as_f32) {
            return Some(Op::Mul(x));
        }
    }
    None
}

fn convert(op: Op, conv: Conv) -> Op {
    let db_to_gain = |d: f32| 10f32.powf(d / 20.0);
    let gain_to_db = |g: f32| 20.0 * g.max(1e-4).log10();
    match (conv, op) {
        (Conv::Same, op) | (_, op @ Op::Reset) => op,
        (Conv::DbToGain, Op::Abs(d)) => Op::Abs(db_to_gain(d)),
        (Conv::DbToGain, Op::Add(d)) => Op::Mul(db_to_gain(d)),
        (Conv::DbToGain, Op::Mul(f)) => Op::Mul(f),
        (Conv::GainToDb, Op::Abs(g)) => Op::Abs(gain_to_db(g)),
        (Conv::GainToDb, Op::Mul(f)) => Op::Add(gain_to_db(f)),
        (Conv::GainToDb, Op::Add(d)) => Op::Add(d),
    }
}

fn parse_switch(val: &Value) -> Option<bool> {
    match val {
        Value::Bool(b) => Some(*b),
        Value::Number(n) => n.as_f64().map(|x| x > 0.5),
        Value::String(s) => match norm(s).as_str() {
            "on" | "true" | "yes" | "start" | "fly" | "go" | "1" | "вкл" | "да" => Some(true),
            "off" | "false" | "no" | "stop" | "land" | "0" | "выкл" | "нет" => Some(false),
            _ => None,
        },
        Value::Object(o) => o.get("value").and_then(parse_switch),
        _ => None,
    }
}

/// Finds the JSON in a model reply that wrapped it in prose or a code fence.
fn extract_json(text: &str) -> Option<Value> {
    if let Ok(v) = serde_json::from_str::<Value>(text) {
        return Some(v);
    }
    let start = text.find(['{', '['])?;
    let close = if text[start..].starts_with('{') { '}' } else { ']' };
    let end = text.rfind(close)?;
    (end > start).then(|| serde_json::from_str(&text[start..=end]).ok()).flatten()
}

/// Top-level fields of a patch that are not changes.
const PATCH_FIELDS: &[&str] = &["explanation", "glide_s", "changes", "moves", "answer", "intent", "say", "reply", "message", "params", "switches", "notes"];

/// The change list in whatever shape it came: an array of entries, a `{key: value}` map,
/// wrapped in `patch` / `input` / `set_scene`, or loose keys at the top level.
fn change_list(o: &Map<String, Value>) -> Vec<Value> {
    let from_map = |m: &Map<String, Value>| m.iter().map(|(k, v)| json!({"key": k, "value": v})).collect::<Vec<_>>();
    let mut out = match o.get("changes").or_else(|| o.get("set")).or_else(|| o.get("controls")) {
        Some(Value::Array(a)) => a.clone(),
        Some(Value::Object(m)) => from_map(m),
        _ => Vec::new(),
    };
    for k in ["params", "switches"] {
        if let Some(Value::Object(m)) = o.get(k) {
            out.extend(from_map(m));
        }
    }
    if o.get("changes").is_none() {
        out.extend(o.iter().filter(|(k, _)| !PATCH_FIELDS.contains(&k.as_str()) && !["set", "controls", "patch", "input", "set_scene", "tool_input"].contains(&k.as_str())).map(|(k, v)| json!({"key": k, "value": v})));
    }
    out
}

fn unwrap_patch(root: Value) -> Value {
    match root {
        Value::Object(ref o) => {
            for k in ["patch", "input", "set_scene", "tool_input", "arguments"] {
                match o.get(k) {
                    Some(inner @ Value::Object(_)) => return unwrap_patch(inner.clone()),
                    Some(Value::String(s)) => {
                        if let Some(v) = extract_json(s) {
                            return unwrap_patch(v);
                        }
                    }
                    _ => {}
                }
            }
            root
        }
        root => root,
    }
}

/// Applies a patch (see the module doc). Returns the result JSON and the engine parameters
/// to set afterwards (switches):
/// `{"ok", "applied": [{"key", "value" | "reset", "clamped"?, "from"?}], "switched": {..},
///   "moved": [ids], "errors": [..], "errors_ru": [..], "undo": <patch>}`.
/// Valid entries are applied even when others are rejected.
pub(crate) fn apply_patch(v: &View, patch: &str) -> (String, Vec<(u32, f32)>) {
    let mut errs = Errs::default();
    let Some(root) = extract_json(patch) else {
        let e = serde_json::from_str::<Value>(patch).err().map_or_else(String::new, |e| e.to_string());
        let out = json!({"ok": false, "applied": [], "moved": [], "errors": [format!("patch is not JSON: {e}")],
            "errors_ru": ["ответ модели не JSON"], "undo": {"changes": []}});
        return (out.to_string(), Vec::new());
    };
    let root = unwrap_patch(root);
    let (default_glide, changes, moves) = match &root {
        Value::Array(a) => (DEFAULT_GLIDE_S, a.clone(), Vec::new()),
        Value::Object(o) => {
            let g = o.get("glide_s").and_then(as_f32).unwrap_or(DEFAULT_GLIDE_S);
            let moves = match o.get("moves") {
                Some(Value::Array(a)) => a.clone(),
                _ => Vec::new(),
            };
            (g, change_list(o), moves)
        }
        _ => {
            errs.push("patch must be an object or an array".into(), "патч должен быть объектом или списком".into());
            (DEFAULT_GLIDE_S, Vec::new(), Vec::new())
        }
    };
    let default_glide = if default_glide.is_finite() { default_glide.clamp(0.0, MAX_GLIDE_S) } else { DEFAULT_GLIDE_S };

    // Parse everything first.
    let mut list: Vec<Change> = Vec::new();
    let mut switches: Vec<(Switch, bool)> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    for c in &changes {
        let Some(o) = c.as_object() else {
            errs.push(format!("change is not an object: {c}"), format!("непонятное изменение {c}"));
            continue;
        };
        let Some(key) = ["key", "control", "name", "param", "parameter", "id"].iter().find_map(|k| o.get(*k).and_then(Value::as_str)) else {
            errs.push(format!("change without a key: {c}"), format!("изменение без параметра: {c}"));
            continue;
        };
        let resolved = match resolve_key(key) {
            Ok(r) => r,
            Err((en, ru)) => {
                errs.push(en, ru);
                continue;
            }
        };
        let value = ["value", "to", "target", "set", "state"].iter().find_map(|k| o.get(*k));
        let i = match resolved {
            Resolved::Switch(s) => {
                match value.and_then(parse_switch).or_else(|| o.get("on").and_then(parse_switch)) {
                    Some(on) => {
                        switches.retain(|(x, _)| *x != s);
                        switches.push((s, on));
                    }
                    None => errs.push(format!("{}: expected \"on\" or \"off\"", s.key()), format!("«{}»: нужно on или off", s.key())),
                }
                continue;
            }
            Resolved::Control(i, conv) => {
                if key != controls::meta(i).key {
                    notes.push(format!("{key} → {}", controls::meta(i).key));
                }
                let reset = o.get("reset").and_then(Value::as_bool).unwrap_or(false);
                let op = if reset {
                    Ok(Op::Reset)
                } else if let Some(rel) = parse_relative(o) {
                    Ok(rel)
                } else {
                    parse_value(i, value.unwrap_or(&Value::Null))
                };
                match op {
                    Ok(op) => (i, convert(op, conv)),
                    Err((en, ru)) => {
                        errs.push(en, ru);
                        continue;
                    }
                }
            }
        };
        let (i, op) = i;
        let glide = o.get("glide_s").and_then(as_f32).unwrap_or(default_glide);
        let glide = if glide.is_finite() { glide.clamp(0.0, MAX_GLIDE_S) } else { default_glide };
        match op {
            Op::Abs(x) | Op::Add(x) | Op::Mul(x) if !x.is_finite() => {
                let k = controls::meta(i).key;
                errs.push(format!("{k}: value is not finite"), format!("у «{k}» значение не число"));
            }
            op => {
                list.retain(|c| c.i != i);
                list.push(Change { i, op, glide });
            }
        }
    }

    // Serialize the entire control transaction with other writers and glide ticks.
    let guard = v.glides.edit();
    // Undo restores what is there now.
    v.glides.tick_locked(v.bank, v.params);
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
    for (s, _) in &switches {
        undo.push(json!({"key": s.key(), "value": if s.on(v) { "on" } else { "off" }}));
    }

    let mut applied = Vec::new();
    for c in &list {
        let m = controls::meta(c.i);
        let discrete = m.flags & flags::DISCRETE != 0;
        let b = before[c.i];
        let target = match c.op {
            Op::Reset => {
                // Reset: a child glides back onto its macro's curve; a macro to its default.
                let child = m.flags & flags::MACRO == 0;
                if discrete {
                    v.glides.cancel(c.i);
                    v.bank.set(m.id, v.bank.default_at(c.i), v.params);
                    if child {
                        v.bank.set(m.id, f32::NAN, v.params);
                    }
                } else if child {
                    if c.glide <= 0.0 || v.bank.followed(c.i, v.params).is_none() {
                        v.glides.cancel(c.i);
                        v.bank.set(m.id, f32::NAN, v.params);
                    } else {
                        v.glides.start(c.i, RESET, b, b, c.glide);
                    }
                } else {
                    start_value(v, c.i, b, v.bank.default_at(c.i), c.glide);
                }
                applied.push(json!({"key": m.key, "value": "reset"}));
                continue;
            }
            Op::Abs(x) => x,
            Op::Add(d) => b + d,
            Op::Mul(f) => b * f,
        };
        let mut x = target.clamp(m.min, m.max);
        if is_time(c.i) {
            x = target.rem_euclid(24.0);
        }
        if discrete {
            x = x.round().clamp(0.0, controls::options(m.id).len().saturating_sub(1) as f32);
            v.glides.cancel(c.i);
            v.bank.set(m.id, x, v.params);
        } else {
            start_value(v, c.i, b, x, c.glide);
        }
        let mut a = Map::new();
        a.insert("key".into(), json!(m.key));
        a.insert("value".into(), if discrete { json!(option_of(m.id, x)) } else { r3(x) });
        a.insert("from".into(), if discrete { json!(option_of(m.id, b)) } else { r3(b) });
        if (x - target).abs() > 1e-4 * (1.0 + target.abs()) && !(discrete && (x - target).abs() < 0.5) && !is_time(c.i) {
            a.insert("clamped".into(), json!(true));
        }
        applied.push(Value::Object(a));
    }
    drop(guard);

    // Switches: engine parameters, set by the caller after this transaction.
    let mut params = Vec::new();
    let mut switched = Map::new();
    for (s, on) in &switches {
        params.push((s.param(), if *on { 1.0 } else { 0.0 }));
        switched.insert(s.key().into(), json!(if *on { "on" } else { "off" }));
        applied.push(json!({"key": s.key(), "value": if *on { "on" } else { "off" }, "from": if s.on(v) { "on" } else { "off" }}));
    }

    // Object moves (immediate; the stage animates its own way).
    let objects = v.objects();
    let mut moved = Vec::new();
    let mut undo_moves = Vec::new();
    for mv in &moves {
        let id = mv.get("object").or_else(|| mv.get("id")).and_then(Value::as_u64).map(|x| x as u32);
        let pos: Option<Vec<f32>> = mv.get("position").or_else(|| mv.get("pos")).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_f64).map(|x| x as f32).collect());
        let (Some(id), Some(pos)) = (id, pos) else {
            errs.push(format!("move needs `object` and `position`: {mv}"), "перемещение без объекта или позиции".into());
            continue;
        };
        let Some(o) = objects.iter().find(|o| o.id == id) else {
            errs.push(format!("unknown object {id}"), format!("нет объекта {id}"));
            continue;
        };
        if pos.len() != 3 || pos.iter().any(|x| !x.is_finite()) {
            errs.push(format!("object {id}: position must be [x, y, z]"), format!("объект {id}: позиция должна быть [x, y, z]"));
            continue;
        }
        if o.flags & object_flags::MOVABLE == 0 {
            errs.push(format!("object {id} cannot be moved"), format!("объект {id} нельзя двигать"));
            continue;
        }
        // Keep things on stage: within 40 m, at most 30 m up.
        let (x, y, z) = (pos[0].clamp(-40.0, 40.0), pos[1].clamp(-40.0, 40.0), pos[2].clamp(-5.0, 30.0));
        v.objects.request_move(id, crate::math::Vec3::new(x, y, z));
        undo_moves.push(json!({"object": id, "position": [r3(o.x), r3(o.y), r3(o.z)]}));
        moved.push(id);
    }

    let mut out = json!({
        "ok": errs.en.is_empty(),
        "applied": applied,
        "moved": moved,
        "errors": errs.en,
        "errors_ru": errs.ru,
        "undo": {"glide_s": default_glide.min(2.0), "changes": undo, "moves": undo_moves},
    });
    if !switched.is_empty() {
        out["switched"] = Value::Object(switched);
    }
    if !notes.is_empty() {
        out["notes"] = json!(notes);
    }
    (out.to_string(), params)
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
    v.glides.tick_locked(v.bank, v.params);
}

// ---------------------------------------------------------------------------------------
// What is around: the scene in words (structured facts + a short Russian summary)
// ---------------------------------------------------------------------------------------

/// A source counts as sounding above this RMS.
const AUDIBLE: f32 = 1e-4;

/// Russian names of landscape presets (unknown ones are shown by their machine name).
fn biome_ru(name: &str) -> String {
    match name {
        "mixed" => "смешанный лес",
        "broadleaf" => "лиственный лес",
        "taiga" => "тайга",
        "river_valley" => "речная долина",
        "meadows" => "луга",
        "highlands" => "нагорье",
        "desert" => "пустыня",
        "snowy_mountains" => "снежные горы",
        "lake_shore" => "берег озера",
        "sea_coast" => "морское побережье",
        "big_river" => "большая река",
        other => return other.replace('_', " "),
    }
    .to_string()
}

/// English blurb of a landscape preset for the system prompt.
fn biome_en(name: &str) -> String {
    match name {
        "mixed" => "mixed forest, brooks, a lake now and then".into(),
        "broadleaf" => "oak, birch and aspen woods".into(),
        "taiga" => "spruce and pine, lakes, rock".into(),
        "river_valley" => "many brooks, willows, alders, lakes".into(),
        "meadows" => "open parkland and fields, crickets, camps".into(),
        "highlands" => "ridges, rock, waterfalls".into(),
        "desert" => "dry sand and dunes, almost no trees, hot wind".into(),
        "snowy_mountains" => "snowy peaks and ridges, cold wind, sparse conifers".into(),
        "lake_shore" => "a lake shore with lapping water".into(),
        "sea_coast" => "a sea coast with surf".into(),
        "big_river" => "a wide big river".into(),
        other => other.replace('_', " "),
    }
}

fn regime_ru(name: &str) -> &'static str {
    match name {
        "still_lake" => "тихое озеро",
        "lake_lapping" => "плеск озера",
        "slow_river" => "медленная река",
        "brook_pebbles" => "ручей по гальке",
        "brook_rocks" => "ручей по камням",
        "rapids" => "пороги",
        "small_waterfall" => "водопад",
        "drips" => "капель",
        "trickle" => "струйка воды",
        _ => "река",
    }
}

fn group_ru(g: &str) -> &'static str {
    match g {
        "water" => "вода",
        "wind" => "ветер и листва",
        "birds" => "птицы",
        "rain" => "дождь",
        "thunder" => "гром",
        "insects" => "насекомые",
        "fire" => "костёр",
        _ => "прочее",
    }
}

fn part_of_day(h: f32) -> (&'static str, &'static str) {
    match h {
        h if h < 4.0 => ("deep night", "глубокая ночь"),
        h if h < 5.5 => ("before dawn", "предрассветный час"),
        h if h < 7.0 => ("dawn", "рассвет"),
        h if h < 11.0 => ("morning", "утро"),
        h if h < 14.0 => ("midday", "полдень"),
        h if h < 18.0 => ("afternoon", "день"),
        h if h < 20.0 => ("evening", "вечер"),
        h if h < 21.5 => ("dusk", "закат"),
        _ => ("night", "ночь"),
    }
}

fn season_ru(s: &str) -> &'static str {
    match s {
        "spring" => "весна",
        "summer" => "лето",
        "autumn" => "осень",
        "winter" => "зима",
        _ => "",
    }
}

fn rain_words(mm_h: f32) -> (&'static str, &'static str) {
    match mm_h {
        r if r < 0.05 => ("dry", "сухо"),
        r if r < 0.6 => ("drizzle", "моросит"),
        r if r < 2.0 => ("light rain", "лёгкий дождь"),
        r if r < 7.0 => ("steady rain", "ровный дождь"),
        r if r < 18.0 => ("heavy rain", "сильный дождь"),
        r if r < 45.0 => ("downpour", "ливень"),
        _ => ("cloudburst", "стена воды"),
    }
}

fn wind_words(ms: f32) -> (&'static str, &'static str) {
    match ms {
        w if w < 0.8 => ("calm", "безветрие"),
        w if w < 3.0 => ("light air", "лёгкий ветерок"),
        w if w < 6.0 => ("gentle breeze", "слабый ветер"),
        w if w < 10.0 => ("fresh wind", "свежий ветер"),
        w if w < 15.0 => ("strong wind", "сильный ветер"),
        w if w < 22.0 => ("gale", "штормовой ветер"),
        _ => ("storm", "буря"),
    }
}

/// Direction of a listener-relative point (+x ahead, +y left), English and Russian.
fn direction(x: f32, y: f32) -> (&'static str, &'static str) {
    let a = y.atan2(x).to_degrees();
    let side = |l: (&'static str, &'static str), r: (&'static str, &'static str)| if a >= 0.0 { l } else { r };
    match a.abs() {
        d if d <= 22.5 => ("ahead", "впереди"),
        d if d <= 67.5 => side(("ahead left", "впереди слева"), ("ahead right", "впереди справа")),
        d if d <= 112.5 => side(("left", "слева"), ("right", "справа")),
        d if d <= 157.5 => side(("behind left", "сзади слева"), ("behind right", "сзади справа")),
        _ => ("behind", "сзади"),
    }
}

fn distance_ru(d: f32) -> &'static str {
    match d {
        d if d < 5.0 => "совсем рядом",
        d if d < 15.0 => "рядом",
        d if d < 40.0 => "недалеко",
        _ => "вдали",
    }
}

/// Live readings the description needs besides the control values.
#[derive(Default)]
pub(crate) struct Live {
    /// Sounding sources (`asmr_get_sources`): kind, listener-relative position, level, pitch.
    pub sources: Vec<SourceInfo>,
    /// What is on stage now (the streamed landscape while flying).
    pub objects: Vec<ObjectInfo>,
    pub mix: MixInfo,
}

/// "What is in front of us": place, time, weather, water, birds, other life, what is loud.
/// Returns the structured facts and a few Russian sentences by topic.
pub(crate) fn scene_facts(v: &View) -> (Value, Vec<(&'static str, String)>) {
    use crate::engine::{param, telemetry};
    let vals = v.values();
    let get = |k: &str| controls::index_of_key(k).map(|i| vals[i]);
    let opt = |k: &str| controls::index_of_key(k).and_then(|i| option_of(controls::meta(i).id, vals[i]));
    let p = |k: u32| v.params[k as usize].load();
    let t = |k: u32| v.telemetry[k as usize].load();
    let mut parts: Vec<(&'static str, String)> = Vec::new();

    // Place.
    let flying = p(param::FLIGHT) > 0.5;
    let playing = p(param::PLAYING) > 0.5;
    let biome = opt("flight.biome").unwrap_or("mixed");
    let kinds = |k: u32| v.live.objects.iter().filter(|o| o.kind == k && o.flags & object_flags::ABSENT == 0).count();
    let (trees, rivers, lakes, rocks) = (kinds(0), kinds(1), kinds(4), kinds(5));
    let sounding = |kind: SourceKind| v.live.sources.iter().filter(move |s| s.kind == kind as u32 && s.level > AUDIBLE);
    let fire = sounding(SourceKind::Fire).count() > 0;
    let speed = t(telemetry::FLIGHT_SPEED);
    let altitude = t(telemetry::FLIGHT_ALTITUDE);
    let place = if flying {
        json!({"mode": "flight", "biome": biome, "speed_m_s": r3(speed), "altitude_m": r3(altitude), "set_speed_m_s": r3(get("flight.speed").unwrap_or(0.0))})
    } else {
        json!({"mode": "scene", "trees": trees, "rivers": rivers, "lakes": lakes, "fire": fire})
    };
    if flying {
        let pace = if speed < 0.3 { "зависли на месте".to_string() } else { format!("{:.0} м/с", speed) };
        let height = if altitude >= 0.5 { format!(", высота {:.0} м", altitude) } else { String::new() };
        parts.push(("place", format!("Полёт, ландшафт «{}»: {pace}{height}.", biome_ru(biome))));
    } else {
        let mut what: Vec<&str> = Vec::new();
        if trees > 0 {
            what.push(if trees > 12 { "лес" } else { "деревья" });
        }
        if rivers > 0 {
            what.push("река");
        }
        if lakes > 0 {
            what.push("озеро");
        }
        if rocks > 0 {
            what.push("скалы");
        }
        if fire {
            what.push("костёр");
        }
        parts.push(("place", if what.is_empty() { "Стоим на месте.".into() } else { format!("Стоим на месте: {}.", what.join(", ")) }));
    }

    // Time.
    let hour = get("env.time_of_day").unwrap_or(12.0).rem_euclid(24.0);
    let (pod_en, pod_ru) = part_of_day(hour);
    let season = opt("env.season").unwrap_or("summer");
    let temp = get("env.temperature").unwrap_or(15.0);
    let clock = format!("{:02}:{:02}", hour as u32, ((hour.fract() * 60.0).round() as u32).min(59));
    parts.push(("time", format!("{} ({clock}), {}, {:+.0} °C.", capitalize(pod_ru), season_ru(season), temp)));

    // Weather.
    let rain_now = t(telemetry::RAIN_RATE);
    let rain_set = get("rain.rate").unwrap_or(0.0);
    let storm = get("rain.storm").unwrap_or(0.0);
    let wind = get("wind.speed").unwrap_or(0.0);
    let wet = t(telemetry::WETNESS);
    let (rain_en, rain_ru) = rain_words(rain_now.max(rain_set.min(rain_now + 0.5)));
    let (wind_en, wind_ru) = wind_words(wind);
    let mut w = if rain_set < 0.05 && rain_now >= 0.05 {
        "Дождь стихает".to_string()
    } else if rain_ru == "сухо" {
        if wet > 0.2 { "Дождя нет, с листьев ещё капает".to_string() } else { "Дождя нет".to_string() }
    } else {
        format!("{} ({:.1} мм/ч)", capitalize(rain_ru), rain_now.max(rain_set))
    };
    if storm > 0.5 {
        w.push_str(", гроза рядом");
    } else if storm > 0.12 {
        w.push_str(", гром вдали");
    }
    w.push_str(&format!(", {wind_ru} ({:.0} м/с).", wind));
    parts.push(("weather", w));
    let weather = json!({"rain_mm_h": r3(rain_now), "rain_target_mm_h": r3(rain_set), "rain": rain_en, "storm": r3(storm),
        "wind_m_s": r3(wind), "wind": wind_en, "temperature_c": r3(temp), "leaf_wetness": r3(wet)});

    // Water, nearest sounding source relative to the head.
    let regime = opt("water.regime").unwrap_or("scene");
    let nearest = |kind: SourceKind| {
        sounding(kind).map(|s| (s, (s.rel_x * s.rel_x + s.rel_y * s.rel_y + s.rel_z * s.rel_z).sqrt())).min_by(|a, b| a.1.total_cmp(&b.1))
    };
    let water = match nearest(SourceKind::Water) {
        Some((s, d)) => {
            let (dir_en, dir_ru) = direction(s.rel_x, s.rel_y);
            let name = if !flying || regime != "scene" { regime_ru(regime) } else if lakes > 0 && rivers == 0 { "озеро" } else { "река" };
            parts.push(("water", format!("{}: {} {dir_ru}, ~{:.0} м.", capitalize(name), distance_ru(d), d)));
            json!({"sounding": true, "regime": regime, "nearest_m": r3(d), "direction": dir_en, "flow": r3(get("water.flow").unwrap_or(0.0))})
        }
        None if !playing => {
            // Paused: nothing sounds, but the water is still there.
            let near = v.live.objects.iter().filter(|o| (o.kind == 1 || o.kind == 4) && o.flags & object_flags::ABSENT == 0)
                .map(|o| (o, (o.x * o.x + o.y * o.y).sqrt())).min_by(|a, b| a.1.total_cmp(&b.1));
            match near {
                Some((o, d)) => {
                    let (dir_en, dir_ru) = direction(o.x, o.y);
                    parts.push(("water", format!("{}: {dir_ru}, ~{:.0} м (звук на паузе).", if o.kind == 4 { "Озеро" } else { "Река" }, d)));
                    json!({"sounding": false, "regime": regime, "nearest_m": r3(d), "direction": dir_en, "flow": r3(get("water.flow").unwrap_or(0.0))})
                }
                None => {
                    parts.push(("water", "Воды рядом нет.".into()));
                    json!({"sounding": false, "regime": regime, "flow": r3(get("water.flow").unwrap_or(0.0))})
                }
            }
        }
        None => {
            parts.push(("water", "Воды не слышно.".into()));
            json!({"sounding": false, "regime": regime, "flow": r3(get("water.flow").unwrap_or(0.0))})
        }
    };

    // Birds singing now, by direction.
    let mut birds: Vec<(f32, &SourceInfo)> = sounding(SourceKind::Bird).map(|s| ((s.rel_x * s.rel_x + s.rel_y * s.rel_y + s.rel_z * s.rel_z).sqrt(), s)).collect();
    birds.sort_by(|a, b| b.1.level.total_cmp(&a.1.level));
    let bird_list: Vec<Value> = birds.iter().take(8).map(|(d, s)| {
        let (dir_en, _) = direction(s.rel_x, s.rel_y);
        json!({"direction": dir_en, "distance_m": r3(*d), "height_m": r3(s.rel_z), "pitch_hz": r3(s.freq), "level": r3(s.level)})
    }).collect();
    if birds.is_empty() {
        let a = get("birds.activity").unwrap_or(0.0);
        parts.push(("birds", if a < 0.05 { "Птиц нет.".into() } else { "Птицы сейчас молчат.".into() }));
    } else {
        let mut by_dir: Vec<(&str, usize, f32)> = Vec::new();
        for (d, s) in &birds {
            let (_, ru) = direction(s.rel_x, s.rel_y);
            match by_dir.iter_mut().find(|x| x.0 == ru) {
                Some(x) => {
                    x.1 += 1;
                    x.2 = x.2.min(*d);
                }
                None => by_dir.push((ru, 1, *d)),
            }
        }
        let n = birds.len();
        let verb = if n == 1 { "Поёт" } else { "Поют" };
        let noun = match n % 10 {
            1 if n % 100 != 11 => "птица",
            2..=4 if !(12..=14).contains(&(n % 100)) => "птицы",
            _ => "птиц",
        };
        let wheres: Vec<String> = by_dir.iter().take(3).map(|(dir, k, d)| if *k > 1 { format!("{k} {dir} ({})", distance_ru(*d)) } else { format!("{dir} {}", distance_ru(*d)) }).collect();
        parts.push(("birds", format!("{verb} {n} {noun}: {}.", wheres.join(", "))));
    }

    // Other life.
    let insects = sounding(SourceKind::Insect).count();
    let mut life: Vec<&str> = Vec::new();
    if insects > 0 {
        life.push("стрекочут насекомые");
    }
    if fire {
        life.push("трещит костёр");
    }
    if sounding(SourceKind::Thunder).count() > 0 {
        life.push("гремит гром");
    }
    if !life.is_empty() {
        parts.push(("life", format!("{}.", capitalize(&life.join(", ")))));
    }

    // What is loud (sound-director measurement, LUFS per group).
    let names = crate::mix::group::NAMES;
    let mut loud: Vec<(&str, f32)> = names.iter().enumerate().map(|(i, n)| (*n, v.live.mix.group_lufs.get(i).copied().unwrap_or(-100.0))).filter(|(_, l)| l.is_finite() && *l > -60.0).collect();
    loud.sort_by(|a, b| b.1.total_cmp(&a.1));
    if let Some((first, _)) = loud.first() {
        let rest: Vec<&str> = loud.iter().skip(1).take(2).map(|(n, _)| group_ru(n)).collect();
        parts.push(("loudest", if rest.is_empty() { format!("Громче всего {}.", group_ru(first)) } else { format!("Громче всего {}, потом {}.", group_ru(first), rest.join(" и ")) }));
    }
    if !playing {
        parts.insert(0, ("playing", "Звук на паузе.".into()));
    }

    let facts = json!({
        "playing": playing,
        "place": place,
        "time": {"hour": r3(hour), "clock": clock, "part_of_day": pod_en, "season": season},
        "weather": weather,
        "water": water,
        "birds": {"singing": birds.len(), "activity": r3(get("birds.activity").unwrap_or(0.0)), "nearest": bird_list},
        "insects_sounding": insects,
        "fire": fire,
        "loudest": loud.iter().take(4).map(|(n, l)| json!({"group": n, "lufs": r3(*l)})).collect::<Vec<_>>(),
        "frame": "directions relative to the head: ahead, left, right, behind",
    });
    (facts, parts)
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// The whole summary, one paragraph.
fn summary_text(parts: &[(&str, String)]) -> String {
    parts.iter().map(|(_, s)| s.as_str()).collect::<Vec<_>>().join(" ")
}

// ---------------------------------------------------------------------------------------
// Prompt
// ---------------------------------------------------------------------------------------

/// Plain-language meaning of each control for the system prompt. Controls added later get a
/// generic line (`<group>.level`), landscapes are listed from the control tree.
fn describe(key: &str) -> String {
    if key == "flight.biome" {
        let list: Vec<String> = controls::options(controls::id::FLIGHT_BIOME).iter().map(|n| format!("{n} ({})", biome_en(n))).collect();
        return format!(
            "Landscape preset of flight mode: {}. Exists only while flying: set flight.on = \"on\" with it. A change fades out and regenerates. Pick the closest one when no preset matches the request exactly.",
            list.join(", ")
        );
    }
    let s = describe_static(key);
    if !s.is_empty() {
        return s.to_string();
    }
    if let Some(g) = key.strip_suffix(".level") {
        return format!(
            "LOUDNESS of the {g} sound only, dB relative to its calibrated level (0 = as calibrated, −3 a bit quieter, −8 clearly quieter, −20 far in the background, −40 muted, +4 a bit louder, +8 clearly louder, +12 max). Use it for 'louder / quieter {g}' (громче / тише): the {g} itself stays as it is."
        );
    }
    String::new()
}

fn describe_static(key: &str) -> &'static str {
    match key {
        "water.flow" => "River discharge, the main PHYSICAL water knob (how much water there is). 0 = no flow (silent, still), 0.3 = low summer brook, 0.6 = as built, 0.8 = high water, 1 = spring flood. Moves speed, depth and width together. Not for 'quieter / louder water' alone: that is loudness (water.level, or water.gain).",
        "water.regime" => "Character of all water: still_lake, lake_lapping (small waves), slow_river (deep, quiet, rare gulps), brook_pebbles (babbling), brook_rocks (louder, splashy), rapids (roaring white water), small_waterfall, drips (cave drips), trickle. `scene` = as built.",
        "water.speed" => "Surface velocity, m/s (0.3 lazy, 1 brisk, 2.5 rapids).",
        "water.depth" => "Depth, m. Shallow riffles babble, deep pools are quiet.",
        "water.width" => "Channel width, m.",
        "water.rock_size" => "Median bed rock size, m. Bigger rocks = deeper gurgles and more splashes (0.02 gravel, 0.1 cobbles, 0.5 boulders).",
        "water.aeration" => "Air entrainment: white-water hiss and fizz (0 glassy, 1 foamy, 1.5 waterfall).",
        "water.turbulence" => "How chaotic the flow is: more chirpy bubbles and splash variety.",
        "water.gain" => "Linear level multiplier of the water sound only (1 = physical, 0.5 ≈ −6 dB, 2 ≈ +6 dB). Loudness, not physics: use for 'quieter / louder river' when the flow itself should stay (prefer water.level if it exists).",
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
        "rain.gain" => "Linear level multiplier of the rain sound only (loudness, not how much it rains; prefer rain.level if it exists).",
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
        "mix.clarity" => "Mix intent (art direction, not levels): how hard the sound director separates sounds. 0 = raw physical mix, 0.6 default, 1 = very clear (few soloists, strong masking reduction, fewer simultaneous bird phrases). Raise for 'clearer', 'less mush', 'focus'; lower for 'wild', 'immersive wall of sound'.",
        "mix.density" => "How many bird phrases may overlap (0 = one at a time, staggered; 1 = no limit). Follows clarity unless set. Lower for 'calm', 'sparse', 'one bird at a time'.",
        "mix.soloists" => "How many sounds may stand in the foreground at once, 1..4 (default 2). The rest blend into the background bed.",
        "mix.loudness" => "Target loudness of the mix for headphones, LUFS (−22 default; −26 quiet, relaxing; −18 lively). Change only on requests about overall level or calm vs energetic.",
        "mix.dynamics" => "How strongly loudness is evened out (0 = natural dynamics, thunder at full force; 0.7 default; 1 = very even, sleep-friendly).",
        "mix.focus.water" => "Foreground (+1) .. neutral (0) .. background (−1) for the river and water. +1 for 'listen to the river', −1 for 'birds over the river'.",
        "mix.focus.wind" => "Foreground .. background for wind and leaves (−1..1).",
        "mix.focus.birds" => "Foreground .. background for birds (−1..1). +1 for 'birdsong', 'birdwatching'.",
        "mix.focus.rain" => "Foreground .. background for rain and thunder (−1..1).",
        "mix.focus.insects" => "Foreground .. background for crickets and insects (−1..1).",
        "mix.focus.fire" => "Foreground .. background for the campfire (−1..1).",
        "mix.front_focus" => "Attention follows the head: sounds in front of the listener's face are lifted ~2 dB and kept clear, sounds behind are a little lower and darker (0 off, 0.5 default, 1 strong). Raise for 'listen where I look'.",
        "mix.distance" => "How strongly distance is heard: far sounds recede (quieter, darker, more reverb), near ones are present (0 physical only, 0.6 default, 1 strong depth).",
        "mix.texture_budget" => "How much the background textures (rain patches, leaves, water segments, wind, insects) may pile up beyond the few nearest/most important ones (0 = only those few, 0.35 default, 1 = everything summed). Lower for 'less noise', 'less hiss', 'cleaner'; raise for 'enveloping', 'wall of rain'.",
        "mix.refocus" => "When the listener stands still, over a few seconds the nearest brook and one bird area come forward and the rest steps back (0 off, 0.5 default, 1 strong). Raise for 'meditative', 'calm focus'; lower for 'keep everything'.",
        "water.level" | "wind.level" | "foliage.level" | "rain.level" | "birds.level" | "insects.level" | "fire.level" => "Loudness trim of this sound group in the mix, dB (−40 mutes, 0 default, +12). A plain fader: use it only for explicit 'louder/quieter/mute the …' requests about one group; prefer the physical macro or mix.focus otherwise.",
        "flight.speed" => "Flight mode only (the user switches it on with the flight button): forward speed through an endless generated landscape, m/s. 0 hover, 1.5 walk, 5 run, 12 glide, 25 fast flight. Altitude follows unless set.",
        "flight.altitude" => "Flight mode: ears above the ground, m, logarithmic (1.7 walking among trunks, 5 under the crowns, 20 just above the canopy, 40 high). Follows the speed unless set.",
        "flight.curviness" => "Flight mode: how much the path bends, 0 straight .. 1 (turn radius 250 m). Takes effect ~200 m ahead.",
        "flight.density" => "Flight mode: forest density multiplier, 0 open .. 1 as generated .. 2 dense; applies at once.",
        "flight.seed" => "Flight mode: which landscape, 0..999 (another number = another landscape; regenerates).",
        "flight.steer_by_head" => "Flight mode: fly where the head looks (AirPods head tracking). off = keep the course (head only looks around), yaw = turning the head turns the flight, yaw_pitch = also climb when looking up and descend when looking down.",
        "flight.turn_rate" => "Flight mode, steering by head: fastest turn, degrees per second (10 lazy glider, 30 default, 60+ agile). Reached when the head is turned ~45° from the course.",
        "flight.airflow" => "Flight mode: wind noise of motion at the ears, 0 off, 1 physical (subtle below ~10 m/s), 3 strong.",
        "flight.relief" => "Flight mode: how high the hills, mountains and dunes are, 0 flat .. 1 as the biome .. 2 twice as steep (regenerates). Relief shapes the sound: hills hide sources, valleys echo.",
        "flight.stops" => "Flight mode: how often the flight eases down and lingers at a place (lake shore, waterfall, surf, river bank, viewpoint), 0 never .. 0.5 now and then .. 1 at every one, longer.",
        _ => "",
    }
}

const RULES: &str = "\
You control a real-time, physically based sound simulation heard binaurally by one listener: a \
forest scene by a river, or (flight mode) an endless generated landscape the listener flies \
through. The user writes a short request in any language (usually Russian). Answer ONLY by calling \
the tool `set_scene` exactly once, never with prose outside it.

Rules:
- Change only what the request implies. Prefer the macro of a group (water.flow, wind.speed, \
rain.rate, birds.activity, env.time_of_day, insects.activity, master.volume); touch children only \
for specific details (\"bigger rocks\", \"dry leaves\", \"waterfall\").
- Use only keys from the list below, spelled exactly. Values are absolute targets in the control's \
units and range; compute them from the current values (\"Current scene\") for relative requests. \
\"a bit\" ≈ 15–25 %, plain comparative ≈ 30–50 %, \"much\" ≈ ×2 or ÷2. Presets are given by name. \
\"reset\" hands a child back to its macro.
- LOUDNESS vs PHYSICS. \"громче / тише <something>\", \"louder / quieter water\" is about how loud \
it sounds: use that group's loudness control `<group>.level` (dB; −8 clearly quieter, +6 clearly \
louder) if it exists, else `<group>.gain` (linear multiplier). Change the physical knob (water.flow, \
rain.rate, wind.speed, birds.activity) only when the thing itself should change: \"река \
полноводнее\", \"дождь сильнее\", \"больше птиц\", \"убери дождь\". \"тише\" alone (no object) = calmer \
scene. master.volume is the overall volume only.
- Switches: `flight.on` (\"on\" / \"off\") starts or stops flight mode. \"полетели\", \"летим\", \
\"fly\" → on (and a flight.speed if none is set: 8 m/s); \"приземлись\", \"стоп\", \"land\" → off. \
flight.* controls only act while flying.
- Scene recipes. A place request (\"перенеси меня в пустыню\", \"в горы, где идёт снег\", \"к морю\", \
\"на озеро ночью в дождь\") is one patch built from blocks: place (flight.on on + flight.biome, the \
closest preset; while flying, keep a gentle flight.speed 3–8 unless asked to stop) + weather \
(rain.rate, rain.storm, wind.speed, env.temperature, env.humidity, env.season for snow/winter) + time \
(env.time_of_day) + life (birds.activity, insects.activity) + mix intent (mix.focus.*). Fill in what \
the place implies even when not said: desert = hot, dry, few birds, no rain; snowy mountains = \
winter, frost, wind, no insects; sea or lake = water in front.
- glide_s is how long the change takes: 4 by default, 10–40 for \"slowly\", \"gradually\", \
\"fading\", \"стихает\", 0.5–1 for \"now\", \"suddenly\".
- Plausibility matters: night means few birds (owls) and more insects in warm weather; heavy rain \
comes with wind; a storm needs rain. Keep the rest of the scene as it is.
- Positions: listener at the origin, +x forward, +y left, +z up, metres. Only objects marked \
movable can be moved (moves). Birds live on trees: to bring birds to one side, move trees there or \
raise birds.activity.
- Mix intent (mix.*) is art direction, not a fader: when the request names what to listen to \
(\"слушать реку\", \"focus on the birds\") or how busy it should feel, set mix.focus.* for the groups \
involved (+0.6..1 in front, −0.4..−1 behind) and mix.clarity / mix.density; when the scene changes \
character (storm, night, dawn chorus), set a fitting focus too (storm: rain +0.5; dawn: birds +0.5). \
Leave mix.loudness and mix.dynamics alone unless asked about overall level or calm. The sound \
director does the real-time levelling itself. \"Less noise / hiss / mush\" → lower mix.texture_budget \
(0.1–0.2); \"listen where I look\" → mix.front_focus 0.8; \"stay with the brook\", \"meditative\" → \
mix.refocus 0.8–1; more or less depth → mix.distance. <group>.level (dB) are plain faders: only for \
explicit \"quieter/louder/mute the …\" about one group.
- Questions about the scene (\"что вокруг?\", \"где птицы?\", \"what is that sound?\") change nothing: \
answer in `explanation`, in the user's language, from \"Scene now\" (up to ~200 characters, plain \
prose, directions relative to the head).
- If the request is already satisfied (\"убери дождь\" when it is dry), change nothing and say so \
(\"Дождя и так нет.\"). If it is not about the sound scene, change nothing and say what you can do.
- explanation: one short line (≤ 70 characters, longer only for answers) in the user's language, \
saying what you changed, plain words, no numbers unless asked. Never leave it empty.";

/// The first landscape preset of `names` that exists, else the first preset.
fn pick_biome(names: &[&str]) -> &'static str {
    let opts = controls::options(controls::id::FLIGHT_BIOME);
    names.iter().find_map(|n| opts.iter().find(|o| *o == n).copied()).or_else(|| opts.first().copied()).unwrap_or("mixed")
}

/// The loudness change for "+db on water": `water.level` when the sound director has it,
/// else the linear `water.gain`.
fn level_change(group: &str, db: f32) -> String {
    if controls::index_of_key(&format!("{group}.level")).is_some() {
        format!("{{\"key\":\"{group}.level\",\"value\":{db}}}")
    } else {
        let g = (10f32.powf(db / 20.0) * 100.0).round() / 100.0;
        format!("{{\"key\":\"{group}.gain\",\"value\":{g}}}")
    }
}

/// Few-shot examples. Generated, so they only use keys and presets that exist (each one is
/// checked to apply cleanly by a test).
fn examples() -> String {
    let desert = pick_biome(&["desert", "meadows"]);
    let snowy = pick_biome(&["snowy_mountains", "highlands"]);
    let sea = pick_biome(&["sea_coast", "lake_shore", "river_valley"]);
    let lake = pick_biome(&["lake_shore", "taiga"]);
    format!("\
Examples (current values matter; these assume wind.speed 6, rain.rate 4, water.flow 0.7, \
birds.activity 0.8, master.volume 0.8, env.time_of_day 14, flight off):

«поспокойнее» →
{{\"explanation\":\"Тише ветер и дождь, река медленнее, птицы реже.\",\"glide_s\":6,\"changes\":[\
{{\"key\":\"wind.speed\",\"value\":3}},{{\"key\":\"rain.rate\",\"value\":1.2}},{{\"key\":\"water.flow\",\"value\":0.5}},\
{{\"key\":\"birds.activity\",\"value\":0.5}},{{\"key\":\"master.volume\",\"value\":0.72}}]}}

«сделай реку тише» →
{{\"explanation\":\"Река звучит тише, сама она прежняя.\",\"changes\":[{quieter}]}}

«громче воду» →
{{\"explanation\":\"Вода громче.\",\"changes\":[{louder}]}}

«больше птиц» →
{{\"explanation\":\"Птиц больше.\",\"changes\":[{{\"key\":\"birds.activity\",\"value\":1}}]}}

«убери дождь» →
{{\"explanation\":\"Дождь прекратился.\",\"glide_s\":8,\"changes\":[{{\"key\":\"rain.rate\",\"value\":0}},\
{{\"key\":\"rain.storm\",\"value\":0}}]}}

«убери дождь» (rain.rate is already 0) →
{{\"explanation\":\"Дождя и так нет.\",\"changes\":[]}}

«хочу ночь» →
{{\"explanation\":\"Ночь: птицы стихли, стрекочут сверчки.\",\"glide_s\":10,\"changes\":[\
{{\"key\":\"env.time_of_day\",\"value\":23.5}},{{\"key\":\"birds.activity\",\"value\":0.25}},\
{{\"key\":\"insects.activity\",\"value\":1.3}}]}}

«3 a.m.» →
{{\"explanation\":\"Three in the morning: owls, crickets, cool air.\",\"glide_s\":8,\"changes\":[\
{{\"key\":\"env.time_of_day\",\"value\":3}},{{\"key\":\"birds.activity\",\"value\":0.3}}]}}

«гроза» →
{{\"explanation\":\"Гроза: ливень, порывистый ветер, гром рядом.\",\"glide_s\":8,\"changes\":[\
{{\"key\":\"rain.rate\",\"value\":25}},{{\"key\":\"rain.storm\",\"value\":0.8}},{{\"key\":\"wind.speed\",\"value\":11}},\
{{\"key\":\"wind.gustiness\",\"value\":1.5}},{{\"key\":\"birds.activity\",\"value\":0.1}},{{\"key\":\"mix.focus.rain\",\"value\":0.5}}]}}

«ночь, лёгкий дождь стихает» →
{{\"explanation\":\"Ночь, редкий дождь медленно стихает.\",\"glide_s\":30,\"changes\":[\
{{\"key\":\"env.time_of_day\",\"value\":23.5}},{{\"key\":\"rain.rate\",\"value\":0.2}},{{\"key\":\"rain.storm\",\"value\":0}},\
{{\"key\":\"birds.activity\",\"value\":0.25}},{{\"key\":\"wind.speed\",\"value\":2}}]}}

«как будто у водопада» →
{{\"explanation\":\"Рядом шумит небольшой водопад.\",\"changes\":[\
{{\"key\":\"water.regime\",\"value\":\"small_waterfall\"}},{{\"key\":\"water.flow\",\"value\":0.9}},\
{{\"key\":\"water.aeration\",\"value\":1.3}}]}}

«полетели» →
{{\"explanation\":\"Летим над лесом.\",\"changes\":[{{\"key\":\"flight.on\",\"value\":\"on\"}},\
{{\"key\":\"flight.speed\",\"value\":8}}]}}

«перенеси меня в пустыню» →
{{\"explanation\":\"Пустыня: жарко, сухо, горячий ветер.\",\"glide_s\":6,\"changes\":[\
{{\"key\":\"flight.on\",\"value\":\"on\"}},{{\"key\":\"flight.biome\",\"value\":\"{desert}\"}},{{\"key\":\"flight.speed\",\"value\":4}},\
{{\"key\":\"rain.rate\",\"value\":0}},{{\"key\":\"rain.storm\",\"value\":0}},{{\"key\":\"wind.speed\",\"value\":6}},\
{{\"key\":\"env.temperature\",\"value\":35}},{{\"key\":\"env.humidity\",\"value\":0.15}},\
{{\"key\":\"birds.activity\",\"value\":0.15}},{{\"key\":\"insects.activity\",\"value\":0.4}}]}}

«в горы, где идёт снег» →
{{\"explanation\":\"Снежные горы: зима, мороз, ветер на гребнях.\",\"glide_s\":6,\"changes\":[\
{{\"key\":\"flight.on\",\"value\":\"on\"}},{{\"key\":\"flight.biome\",\"value\":\"{snowy}\"}},{{\"key\":\"flight.speed\",\"value\":5}},\
{{\"key\":\"env.season\",\"value\":\"winter\"}},{{\"key\":\"env.temperature\",\"value\":-8}},{{\"key\":\"rain.rate\",\"value\":0}},\
{{\"key\":\"wind.speed\",\"value\":9}},{{\"key\":\"insects.activity\",\"value\":0}},{{\"key\":\"birds.activity\",\"value\":0.1}}]}}

«к морю» →
{{\"explanation\":\"Летим к морю, вода впереди.\",\"changes\":[\
{{\"key\":\"flight.on\",\"value\":\"on\"}},{{\"key\":\"flight.biome\",\"value\":\"{sea}\"}},{{\"key\":\"flight.speed\",\"value\":5}},\
{{\"key\":\"wind.speed\",\"value\":7}},{{\"key\":\"mix.focus.water\",\"value\":0.7}}]}}

«на озеро ночью в дождь» →
{{\"explanation\":\"Ночное озеро под ровным дождём.\",\"glide_s\":8,\"changes\":[\
{{\"key\":\"flight.on\",\"value\":\"on\"}},{{\"key\":\"flight.biome\",\"value\":\"{lake}\"}},{{\"key\":\"flight.speed\",\"value\":2}},\
{{\"key\":\"env.time_of_day\",\"value\":23}},{{\"key\":\"rain.rate\",\"value\":4}},{{\"key\":\"birds.activity\",\"value\":0.15}},\
{{\"key\":\"insects.activity\",\"value\":0.6}},{{\"key\":\"mix.focus.rain\",\"value\":0.4}}]}}

«приземлись» →
{{\"explanation\":\"Приземлились, снова лес у реки.\",\"changes\":[{{\"key\":\"flight.on\",\"value\":\"off\"}}]}}

«больше птиц слева» (tree 4 is movable at [6, -3, 0]) →
{{\"explanation\":\"Больше птиц, дерево с птицами слева.\",\"changes\":[\
{{\"key\":\"birds.activity\",\"value\":0.95}}],\"moves\":[{{\"object\":4,\"position\":[4,6,0]}}]}}

«хочу слушать реку, птицы пусть будут фоном» →
{{\"explanation\":\"Река впереди, птицы отошли в фон.\",\"changes\":[\
{{\"key\":\"mix.focus.water\",\"value\":0.8}},{{\"key\":\"mix.focus.birds\",\"value\":-0.5}},\
{{\"key\":\"mix.clarity\",\"value\":0.75}}]}}

«верни реку как была» →
{{\"explanation\":\"Река снова как в сцене.\",\"changes\":[{{\"key\":\"water.regime\",\"value\":\"scene\"}},\
{{\"key\":\"water.flow\",\"value\":0.6}},{{\"key\":\"water.aeration\",\"value\":\"reset\"}}]}}

«что вокруг?» →
{{\"explanation\":\"День, лес у реки. Река справа рядом, слева поют две птицы, лёгкий ветер.\",\"changes\":[]}}

«what's the weather in Paris?» →
{{\"explanation\":\"I can only change the sound scene: water, weather, time, birds, flight.\",\"changes\":[]}}",
        quieter = level_change("water", -8.0),
        louder = level_change("water", 6.0),
    )
}

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
        let level = c["unit"] == json!("dB") && key.ends_with(".level");
        let flag = if c["overridden"] == json!(true) { ", set explicitly" } else if c["parent"].is_string() && !level { ", follows macro" } else { "" };
        let _ = writeln!(s, "{key} = {val}{range}{flag}");
    }
    let _ = writeln!(s, "[switches]");
    for sw in state["switches"].as_array().into_iter().flatten() {
        let _ = writeln!(s, "{} = {} (on / off)", sw["key"].as_str().unwrap_or(""), sw["value"].as_str().unwrap_or(""));
    }
    let e = &state["environment"];
    let _ = writeln!(
        s,
        "[readings] playing {}, rain falling now {} mm/h, leaf wetness {}, ground {}",
        e["playing"], e["rain_now_mm_h"], e["leaf_wetness"], e["ground"]
    );
    let _ = writeln!(s, "[scene now] {}", state["summary_ru"].as_str().unwrap_or(""));
    let _ = writeln!(s, "[objects] {}", state["frame"].as_str().unwrap_or(""));
    for o in state["objects"].as_array().into_iter().flatten().take(40) {
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
    let _ = writeln!(s, "- flight.on: switch, \"on\" / \"off\": flight mode (the listener flies through generated landscapes; off = the scene where the user stands).");
    s.push('\n');
    s.push_str(&examples());
    s
}

/// JSON schema of the `set_scene` tool input, generated from the control tree.
pub fn tool_schema() -> Value {
    let mut keys: Vec<&str> = (0..COUNT).map(|i| controls::meta(i).key).collect();
    keys.extend(Switch::ALL.iter().map(|s| s.key()));
    let mut names: Vec<&str> = vec!["reset", "on", "off"];
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
            "explanation": {"type": "string", "description": "One short line in the user's language: what changed, why nothing changed, or the answer to a question about the scene."},
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
                            "description": "Target in the control's units, a preset name for preset controls, \"on\"/\"off\" for switches, or \"reset\"."
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

/// `{"system", "context", "tool": {"name", "description", "input_schema"}, "scene", "summary_ru"}`.
/// `system` is static (cacheable); `context` is the current state, sent with the request.
pub(crate) fn prompt_json(v: &View) -> String {
    let (facts, parts) = scene_facts(v);
    let topics: Map<String, Value> = parts.iter().map(|(k, s)| (k.to_string(), json!(s))).collect();
    json!({
        "system": system_text(),
        "context": context_text(v),
        "tool": {
            "name": "set_scene",
            "description": "Change the sound scene: absolute targets for controls and switches, optional object moves, and a one-line explanation (or an answer about the scene) for the user.",
            "input_schema": tool_schema(),
        },
        "scene": facts,
        "summary_ru": summary_text(&parts),
        "summary_topics_ru": topics,
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
    fn snapshots_include_glide_targets_and_pending_moves() {
        let e = engine();
        let before = parse(&e.state_json());
        let tree = before["objects"].as_array().unwrap().iter()
            .find(|o| o["kind"] == "tree" && o["movable"] == true).unwrap();
        let id = tree["id"].as_u64().unwrap() as u32;
        e.set_object_position(id, 12.0, -9.0, -1.7);
        e.apply_patch_json(r#"{"glide_s":120,"changes":[{"key":"master.volume","value":0.2}]}"#);
        let state = parse(&e.state_json());
        let volume = state["controls"].as_array().unwrap().iter().find(|c| c["key"] == "master.volume").unwrap();
        assert!((volume["target"].as_f64().unwrap() - 0.2).abs() < 1e-6);
        let moved = state["objects"].as_array().unwrap().iter().find(|o| o["id"] == id).unwrap();
        assert_eq!(moved["pos"], json!([12.0, -9.0, -1.7]));
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
        assert_eq!(keys.len(), crate::controls::COUNT + super::Switch::ALL.len());
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

    fn applied(r: &Value, key: &str) -> Option<Value> {
        r["applied"].as_array().unwrap().iter().find(|a| a["key"] == key).map(|a| a["value"].clone())
    }

    #[test]
    fn keys_are_resolved_tolerantly() {
        use super::{resolve_key, Conv, Resolved, Switch};
        let idx = |k: &str| crate::controls::index_of_key(k).unwrap();
        for (raw, key) in [
            ("water.flow", "water.flow"),
            ("Water Flow", "water.flow"),
            ("river.flow", "water.flow"),
            ("river", "water.flow"),
            ("rain", "rain.rate"),
            ("time", "env.time_of_day"),
            ("time_of_day", "env.time_of_day"),
            ("rock_size", "water.rock_size"),
            ("biome", "flight.biome"),
            ("focus.birds", "mix.focus.birds"),
            ("wind.sped", "wind.speed"),
            ("birds.activty", "birds.activity"),
            ("controls.rain.rate", "rain.rate"),
            ("thunder", "rain.storm"),
        ] {
            match resolve_key(raw) {
                Ok(Resolved::Control(i, Conv::Same)) => assert_eq!(i, idx(key), "{raw}"),
                other => panic!("{raw}: {other:?}"),
            }
        }
        for raw in ["flight.on", "flight", "flying", "params.flight"] {
            assert_eq!(resolve_key(raw), Ok(Resolved::Switch(Switch::Flight)), "{raw}");
        }
        // Loudness: `<group>.level` when it exists, else the linear gain in dB.
        let lvl = resolve_key("river.volume").unwrap();
        match crate::controls::index_of_key("water.level") {
            Some(i) => assert_eq!(lvl, Resolved::Control(i, Conv::Same)),
            None => assert_eq!(lvl, Resolved::Control(idx("water.gain"), Conv::DbToGain)),
        }
        assert!(resolve_key("nope.x").is_err());
        assert!(resolve_key("banana").is_err());
        let e = resolve_key("weather.sunshine").unwrap_err();
        assert!(e.0.contains("unknown key"), "{e:?}");
    }

    #[test]
    fn presets_and_values_are_parsed_loosely() {
        let e = engine();
        let r = parse(&e.apply_patch_json(r#"{"glide_s":0,"changes":[
            {"key":"water.regime","value":"Small Waterfall"},
            {"key":"rain.rate","value":"6 mm/h"},
            {"key":"env.time_of_day","value":"23:30"},
            {"key":"env.season","value":"Winter"},
            {"key":"flight.biome","value":"тайга"},
            {"key":"flight.steer_by_head","value":false}]}"#));
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(e.control(id::WATER_REGIME), crate::controls::options(id::WATER_REGIME).iter().position(|o| *o == "small_waterfall").unwrap() as f32);
        assert!((e.control(id::RAIN_RATE) - 6.0).abs() < 1e-4);
        assert!((e.control(id::ENV_TIME) - 23.5).abs() < 1e-4);
        assert_eq!(applied(&r, "flight.biome"), Some(json!("taiga")));
        assert_eq!(applied(&r, "flight.steer_by_head"), Some(json!("off")));
        // Russian place words pick the closest landscape that exists.
        let r = parse(&e.apply_patch_json(r#"{"changes":[{"key":"flight.biome","value":"горы"}]}"#));
        let b = applied(&r, "flight.biome").unwrap();
        assert!(b == "highlands" || b == "snowy_mountains", "{r}");
        // Relative values.
        e.apply_patch_json(r#"{"glide_s":0,"changes":[{"key":"wind.speed","value":6}]}"#);
        e.apply_patch_json(r#"{"glide_s":0,"changes":[{"key":"wind.speed","factor":0.5},{"key":"birds.activity","value":0.5}]}"#);
        e.apply_patch_json(r#"{"glide_s":0,"changes":[{"key":"birds.activity","delta":0.2}]}"#);
        assert!((e.control(id::WIND_SPEED) - 3.0).abs() < 1e-3);
        assert!((e.control(id::BIRDS_ACTIVITY) - 0.7).abs() < 1e-4);
    }

    #[test]
    fn patch_shapes_models_write_are_accepted() {
        let e = engine();
        // JSON in prose / a code fence, a {key: value} map, a wrapper.
        let prose = "Sure! Here is the patch:\n```json\n{\"explanation\":\"x\",\"glide_s\":0,\"changes\":{\"water.flow\":0.2}}\n```";
        assert_eq!(parse(&e.apply_patch_json(prose))["ok"], true);
        assert!((e.control(id::WATER_FLOW) - 0.2).abs() < 1e-4);
        let wrapped = r#"{"set_scene":{"glide_s":0,"changes":[{"key":"birds","value":0.1}]}}"#;
        assert_eq!(parse(&e.apply_patch_json(wrapped))["ok"], true);
        assert!((e.control(id::BIRDS_ACTIVITY) - 0.1).abs() < 1e-4);
        let loose = r#"{"explanation":"x","glide_s":0,"rain.rate":3}"#;
        assert_eq!(parse(&e.apply_patch_json(loose))["ok"], true);
        assert!((e.control(id::RAIN_RATE) - 3.0).abs() < 1e-4);
        // Errors come in both languages.
        let r = parse(&e.apply_patch_json(r#"{"changes":[{"key":"nope.x","value":1}]}"#));
        assert!(r["errors_ru"][0].as_str().unwrap().contains("нет параметра"));
    }

    #[test]
    fn level_alias_converts_decibels() {
        let e = engine();
        let r = parse(&e.apply_patch_json(r#"{"glide_s":0,"changes":[{"key":"water.volume","value":-6}]}"#));
        assert_eq!(r["ok"], true, "{r}");
        if crate::controls::index_of_key("water.level").is_none() {
            let g = e.control(id::WATER_GAIN);
            assert!((g - 0.501).abs() < 0.01, "{g}");
        }
    }

    #[test]
    fn flight_switch_toggles_and_undoes() {
        let e = engine();
        assert!(e.param(crate::param::FLIGHT) < 0.5);
        let r = parse(&e.apply_patch_json(r#"{"changes":[{"key":"flight.on","value":"on"},{"key":"flight.biome","value":"taiga"}]}"#));
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(r["switched"]["flight.on"], "on");
        assert!(e.param(crate::param::FLIGHT) > 0.5);
        let s = parse(&e.state_json());
        assert_eq!(s["switches"][0]["value"], "on");
        let undo = r["undo"].to_string();
        assert!(undo.contains("flight.on"));
        e.apply_patch_json(&undo);
        assert!(e.param(crate::param::FLIGHT) < 0.5);
        // Bad switch values are reported.
        let r = parse(&e.apply_patch_json(r#"{"changes":[{"key":"flight.on","value":"maybe"}]}"#));
        assert_eq!(r["ok"], false);
    }

    #[test]
    fn state_describes_the_scene_in_words() {
        let e = engine();
        e.set_param(crate::param::PLAYING, 1.0);
        e.apply_patch_json(r#"{"glide_s":0,"changes":[{"key":"env.time_of_day","value":23},{"key":"rain.rate","value":5},{"key":"birds.activity","value":1}]}"#);
        render(&e, 3.0);
        let s = parse(&e.state_json());
        let sum = s["summary_ru"].as_str().unwrap();
        assert!(sum.contains("Ночь") && sum.contains("23:00"), "{sum}");
        assert!(sum.contains("дождь"), "{sum}");
        assert_eq!(s["scene"]["place"]["mode"], "scene");
        assert_eq!(s["scene"]["time"]["part_of_day"], "night");
        assert!(s["scene"]["weather"]["rain_target_mm_h"].as_f64().unwrap() > 4.0);
        for t in ["place", "time", "weather", "water", "birds"] {
            assert!(s["summary_topics_ru"][t].is_string(), "{t} missing: {sum}");
        }
        let p = parse(&e.prompt_json());
        assert!(p["context"].as_str().unwrap().contains("[scene now]"));
        assert!(p["context"].as_str().unwrap().contains("flight.on = off"));
        assert_eq!(super::direction(1.0, 0.0).1, "впереди");
        assert_eq!(super::direction(0.0, 1.0).1, "слева");
        assert_eq!(super::direction(0.0, -1.0).1, "справа");
        assert_eq!(super::direction(-1.0, 0.1).1, "сзади");
    }

    #[test]
    fn system_prompt_lists_landscapes_from_the_tree() {
        let sys = parse(&engine().prompt_json())["system"].as_str().unwrap().to_string();
        for b in crate::controls::options(id::FLIGHT_BIOME) {
            assert!(sys.contains(b), "{b}");
        }
        assert!(sys.contains("LOUDNESS vs PHYSICS"));
        assert!(sys.contains("- flight.on"));
    }
}
