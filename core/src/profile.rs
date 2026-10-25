//! Opt-in CPU profile of the render path (`examples/profile.rs`, `ASMRX3000 --bench` with
//! `ASMR_PROFILE=1`). Off by default: every hook is then one relaxed atomic load. When on,
//! `lap(section)` charges the time since the previous lap on this thread to `section`, so
//! consecutive laps cover a render without gaps and the sections add up to the whole of it.
//! Totals are global atomics, readable from any thread; the lap clock is per thread.

use std::cell::Cell;
use std::fmt::Write;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};

static ON: AtomicBool = AtomicBool::new(false);

/// Emitter kinds, as `SourceKind` (generator and propagation time are charged per kind).
pub const KINDS: usize = 7;
pub const KIND_NAMES: [&str; KINDS] = ["water", "wind", "bird", "rain", "thunder", "fire", "insect"];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Section {
    /// Glides, controls, head pose, director, scene, environment update.
    Control,
    /// Generator render, per emitter kind.
    Generate(usize),
    /// Mix layer voice + feed, level and scope bookkeeping per emitter.
    MixVoice,
    /// Propagation staging (delay/Doppler, refits, sends), per emitter kind.
    Propagate(usize),
    /// Path filter bank (all emitters, staged or silent).
    PathFilters,
    /// Near field + 3rd-order ambisonic encode of both paths.
    Encode,
    /// Early reflections and late reverb.
    Reverb,
    /// Sound-field rotation into the head frame.
    Rotate,
    /// Binaural (or 7.1.4 bed) decode.
    Decode,
    /// Mix finish, master dynamics, output loop, scope.
    Master,
    /// Spectrum ring and publishing telemetry for the UI.
    Publish,
}

pub const SECTIONS: usize = 9 + 2 * KINDS;

impl Section {
    pub fn index(self) -> usize {
        match self {
            Section::Control => 0,
            Section::Generate(k) => 1 + k,
            Section::MixVoice => 1 + KINDS,
            Section::Propagate(k) => 2 + KINDS + k,
            Section::PathFilters => 2 + 2 * KINDS,
            Section::Encode => 3 + 2 * KINDS,
            Section::Reverb => 4 + 2 * KINDS,
            Section::Rotate => 5 + 2 * KINDS,
            Section::Decode => 6 + 2 * KINDS,
            Section::Master => 7 + 2 * KINDS,
            Section::Publish => 8 + 2 * KINDS,
        }
    }

    pub fn name(i: usize) -> String {
        match i {
            0 => "control + director".into(),
            i if i <= KINDS => format!("generate: {}", KIND_NAMES[i - 1]),
            i if i == 1 + KINDS => "mix voice/feed".into(),
            i if i < 2 + 2 * KINDS => format!("propagate: {}", KIND_NAMES[i - 2 - KINDS]),
            i => ["path filters", "near field + encode", "reverb", "rotate", "decode", "master", "publish"]
                [i - 2 - 2 * KINDS]
                .into(),
        }
    }
}

#[allow(clippy::declare_interior_mutable_const)]
const ZERO: AtomicU64 = AtomicU64::new(0);
static NANOS: [AtomicU64; SECTIONS] = [ZERO; SECTIONS];
static BLOCKS: [AtomicU64; KINDS] = [ZERO; KINDS];
static SOUNDING: [AtomicU64; KINDS] = [ZERO; KINDS];
static EMITTERS: AtomicU64 = ZERO;
static RENDER_BLOCKS: AtomicU64 = ZERO;
static PATHS: AtomicU64 = ZERO;
/// Audio rendered, in nanoseconds of signal (frames / sample rate).
static AUDIO_NANOS: AtomicU64 = ZERO;

thread_local! {
    static LAST: Cell<Option<std::time::Instant>> = const { Cell::new(None) };
}

/// Totals since `enable`.
#[derive(Clone, Default, Debug)]
pub struct Report {
    pub secs: Vec<f64>,
    /// Emitter-blocks rendered per kind, and how many of them produced any signal.
    pub blocks: [u64; KINDS],
    pub sounding: [u64; KINDS],
    /// Emitters rendered, summed over blocks, and render blocks.
    pub emitters: u64,
    pub render_blocks: u64,
    /// Propagators the filter bank ran (staged or not), summed over blocks.
    pub paths: u64,
    /// Seconds of audio rendered.
    pub audio: f64,
}

#[inline]
pub fn on() -> bool {
    ON.load(Relaxed)
}

/// Clears the totals and starts profiling every render from now on.
pub fn enable() {
    for a in NANOS.iter().chain(&BLOCKS).chain(&SOUNDING) {
        a.store(0, Relaxed);
    }
    for a in [&EMITTERS, &RENDER_BLOCKS, &PATHS, &AUDIO_NANOS] {
        a.store(0, Relaxed);
    }
    ON.store(true, Relaxed);
}

pub fn report() -> Report {
    let load = |a: &[AtomicU64; KINDS]| core::array::from_fn(|k| a[k].load(Relaxed));
    Report {
        secs: NANOS.iter().map(|a| a.load(Relaxed) as f64 * 1e-9).collect(),
        blocks: load(&BLOCKS),
        sounding: load(&SOUNDING),
        emitters: EMITTERS.load(Relaxed),
        render_blocks: RENDER_BLOCKS.load(Relaxed),
        paths: PATHS.load(Relaxed),
        audio: AUDIO_NANOS.load(Relaxed) as f64 * 1e-9,
    }
}

/// Start of a render: restarts the lap clock without charging anyone.
#[inline]
pub fn start() {
    if on() {
        LAST.with(|l| l.set(Some(std::time::Instant::now())));
    }
}

/// End of a render of `frames` at `sample_rate`.
#[inline]
pub fn rendered(frames: usize, sample_rate: f32) {
    if on() {
        AUDIO_NANOS.fetch_add((frames as f64 / sample_rate as f64 * 1e9) as u64, Relaxed);
    }
}

#[inline]
pub fn lap(section: Section) {
    if !on() {
        return;
    }
    let now = std::time::Instant::now();
    if let Some(t) = LAST.with(|l| l.replace(Some(now))) {
        NANOS[section.index()].fetch_add((now - t).as_nanos() as u64, Relaxed);
    }
}

/// One block's emitter census (`emitters` rendered, `paths` filtered).
#[inline]
pub fn census(emitters: usize, paths: usize) {
    if on() {
        RENDER_BLOCKS.fetch_add(1, Relaxed);
        EMITTERS.fetch_add(emitters as u64, Relaxed);
        PATHS.fetch_add(paths as u64, Relaxed);
    }
}

/// An emitter of `kind` rendered a block; `sounding` if it produced any signal.
#[inline]
pub fn emitter(kind: usize, sounding: bool) {
    if on() {
        BLOCKS[kind].fetch_add(1, Relaxed);
        SOUNDING[kind].fetch_add(sounding as u64, Relaxed);
    }
}

/// The report as a table: percent of one core in real time per section, then per emitter kind.
pub fn format(title: &str) -> String {
    let rep = report();
    let audio = rep.audio.max(1e-9);
    let total: f64 = rep.secs.iter().sum();
    let blocks = rep.render_blocks.max(1) as f64;
    let mut s = String::new();
    let _ = writeln!(s, "━━ {title}: {:.1} % of one core over {:.0} s of audio, {:.0} emitters, {:.0} path filters per block",
        total / audio * 100.0, rep.audio, rep.emitters as f64 / blocks, rep.paths as f64 / blocks);
    let mut rows: Vec<(usize, f64)> = (0..SECTIONS).map(|i| (i, rep.secs[i])).filter(|(_, t)| *t / audio >= 5e-4).collect();
    rows.sort_by(|a, b| b.1.total_cmp(&a.1));
    for (i, t) in rows {
        let bar = "█".repeat((t / total * 50.0).round() as usize);
        let _ = writeln!(s, "  {:<22} {:>6.2} %  {:>4.1} % of DSP  {bar}", Section::name(i), t / audio * 100.0, t / total * 100.0);
    }
    let _ = writeln!(s, "  per kind: present / sounding emitters, % of one core per present emitter");
    for k in 0..KINDS {
        if rep.blocks[k] == 0 {
            continue;
        }
        let present = rep.blocks[k] as f64 / blocks;
        let each = |sec: Section| rep.secs[sec.index()] / audio * 100.0 / present;
        let _ = writeln!(s, "    {:<8} {:>5.1} / {:>5.1}   generate {:.3} %  propagate {:.3} %",
            KIND_NAMES[k], present, rep.sounding[k] as f64 / blocks, each(Section::Generate(k)), each(Section::Propagate(k)));
    }
    s
}
