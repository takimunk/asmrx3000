//! The sound director's mix layer (docs/sound-director.md): keeps a dense scene legible.
//!
//! Layer 1, deterministic and real-time, here:
//! 1. **Calibration** (calibration.rs): every emitter class has a literature source level;
//!    signals are pascals at 1 m.
//! 2. **Measurement** (analysis.rs, loudness.rs): per mix group and per soloist, Bark-band
//!    powers at the listener, excitation, specific and partial loudness; BS.1770 LUFS of
//!    the output.
//! 3. **Attention**: a salience score per emitter (loudness over the bed, onset/novelty,
//!    motion, rarity, intent focus) picks at most N foreground soloists with hysteresis;
//!    the rest of the event-like sounds are demoted into the bed (lower, drier-to-wetter,
//!    darker: distance cues). Bird phrases are throttled to a concurrency limit and staggered
//!    (acoustic niche partitioning) through the director's `activity` control.
//! 4. **Masking-aware ducking**: the bed groups are cut by a few dB only in the critical
//!    bands where they mask a foreground source (fed into the propagation path filters, so
//!    it costs no extra per-sample filtering).
//! 5. **Dynamics** (dynamics.rs): leveller to a headphone loudness target, comfort
//!    compressor, limiter.
//! 6. **Scene focus** (focus.rs): perceptual distance (level, HF, DRR), attention that
//!    follows the head pose (front focus), a loudness budget for broadband textures (a few
//!    protected streams, the low-priority tail steps back; texture groups keep their own
//!    bands) and a stationary refocus (when the listener stops, the nearest water and one
//!    bird come forward). Per-group level trims (`<group>.level`) are plain gains applied
//!    before the analysis.
//!
//! Layer 2 is the intent in the control tree (group MIX: clarity, density, soloists,
//! loudness target, dynamics, per-group focus). A language model may set it on a scene
//! change; it never touches per-block levels.
//!
//! Engine contract, per control block: `begin` → for each emitter `voice` (before its
//! generator renders: gain, path EQ, reverb send, phrase permit) and `feed` (after: the
//! dry signal) → `finish` (analysis and decisions) → `master` (output gains). All of it is
//! allocation-free after `new`.

pub mod analysis;
pub mod calibration;
pub mod dynamics;
pub mod focus;
pub mod loudness;
#[cfg(test)]
mod tests;

use core::sync::atomic::Ordering;
use std::sync::atomic::AtomicU32;

use analysis::{db_gain_sq, Analysis, Curve, DuckParams, HOP};
pub use calibration::{group, SoundClass};
use dynamics::{DynParams, Dynamics};
use loudness::{db_spl, BANDS};

use crate::controls::{id, ControlState};
use crate::engine::{SourceKind, BLOCK};
use crate::math::{Mat3, Vec3};
use focus::{Budget, Cue, Stillness};
use crate::shaping::NF;

/// Emitter slots the mix keeps state for (scene emitters first; flight mode can use more).
pub const MAX_SLOTS: usize = 256;
pub const MAX_SOLOISTS: usize = 4;
/// Analysis tracks: one per group (bed), one per soloist, and the texture tail (a copy of
/// the low-priority textures, also in their group tracks), padded to pairs.
const TRACKS: usize = group::COUNT + MAX_SOLOISTS + 1;
const TAIL: usize = group::COUNT + MAX_SOLOISTS;
const _: () = assert!(TAIL < TRACKS);
/// Bed groups that hold broadband textures (budget, spectral partition).
const TEXTURE_GROUPS: [usize; 4] = [group::WATER, group::WIND, group::RAIN, group::INSECTS];

/// Per-group level trims (`<group>.level` controls), index of `Intent::level_db`.
pub mod level {
    pub const WATER: usize = 0;
    pub const WIND: usize = 1;
    pub const FOLIAGE: usize = 2;
    pub const RAIN: usize = 3;
    pub const BIRDS: usize = 4;
    pub const INSECTS: usize = 5;
    pub const FIRE: usize = 6;
    pub const COUNT: usize = 7;
}

/// Level-trim index of a class (thunder follows the rain).
pub fn level_index(c: SoundClass) -> usize {
    match c {
        SoundClass::Water => level::WATER,
        SoundClass::Wind => level::WIND,
        SoundClass::Foliage => level::FOLIAGE,
        SoundClass::Rain | SoundClass::Thunder => level::RAIN,
        SoundClass::Bird(_) => level::BIRDS,
        SoundClass::Insects => level::INSECTS,
        SoundClass::Fire => level::FIRE,
    }
}

/// Texture class index (budget: the strongest stream of each is protected), None for events.
fn texture_kind(c: SoundClass) -> Option<usize> {
    match c {
        SoundClass::Water => Some(0),
        SoundClass::Foliage => Some(1),
        SoundClass::Wind => Some(2),
        SoundClass::Rain => Some(3),
        SoundClass::Insects => Some(4),
        _ => None,
    }
}
const TEXTURE_KINDS: usize = 5;
/// A level trim at or below this mutes.
const MUTE_DB: f32 = -39.99;

/// The intent (layer 2), read from the MIX controls every block.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Intent {
    /// 0 = leave the physical mix alone, 1 = strongest separation.
    pub clarity: f32,
    /// 0 = one bird phrase at a time, 1 = no limit.
    pub density: f32,
    pub soloists: usize,
    pub target_lufs: f32,
    pub dynamics: f32,
    /// Per mix group, −1 (background) .. 0 (neutral) .. +1 (foreground).
    pub focus: [f32; group::COUNT],
    /// Attention follows the head (mix.front_focus, 0..1).
    pub front_focus: f32,
    /// Perceptual distance emphasis (mix.distance, 0..1).
    pub distance: f32,
    /// Texture budget (mix.texture_budget, 0 = only the protected streams, 1 = no limit).
    pub texture_budget: f32,
    /// Stationary refocus strength (mix.refocus, 0..1).
    pub refocus: f32,
    /// Per-group level trims, dB (`level::*`; −40 mutes).
    pub level_db: [f32; level::COUNT],
}

impl Default for Intent {
    fn default() -> Self {
        Self {
            clarity: 0.6,
            density: 0.54,
            soloists: 2,
            target_lufs: -22.0,
            dynamics: 0.7,
            focus: [0.0; group::COUNT],
            front_focus: 0.5,
            distance: 0.6,
            texture_budget: 0.35,
            refocus: 0.5,
            level_db: [0.0; level::COUNT],
        }
    }
}

impl Intent {
    pub fn from_controls(st: &ControlState) -> Self {
        let f = |k: u32| st.get(k).clamp(-1.0, 1.0);
        let rain = f(id::MIX_FOCUS_RAIN);
        let mut focus = [0.0; group::COUNT];
        focus[group::WATER] = f(id::MIX_FOCUS_WATER);
        focus[group::WIND] = f(id::MIX_FOCUS_WIND);
        focus[group::BIRDS] = f(id::MIX_FOCUS_BIRDS);
        focus[group::RAIN] = rain;
        // Thunder belongs to the weather: it follows the rain focus.
        focus[group::THUNDER] = rain;
        focus[group::INSECTS] = f(id::MIX_FOCUS_INSECTS);
        focus[group::FIRE] = f(id::MIX_FOCUS_FIRE);
        let u = |k: u32| st.get(k).clamp(0.0, 1.0);
        let lv = |k: u32| {
            let v = st.get(k);
            if v.is_finite() {
                v.clamp(-40.0, 12.0)
            } else {
                0.0
            }
        };
        let mut level_db = [0.0; level::COUNT];
        level_db[level::WATER] = lv(id::WATER_LEVEL);
        level_db[level::WIND] = lv(id::WIND_LEVEL);
        level_db[level::FOLIAGE] = lv(id::FOLIAGE_LEVEL);
        level_db[level::RAIN] = lv(id::RAIN_LEVEL);
        level_db[level::BIRDS] = lv(id::BIRDS_LEVEL);
        level_db[level::INSECTS] = lv(id::INSECTS_LEVEL);
        level_db[level::FIRE] = lv(id::FIRE_LEVEL);
        Self {
            front_focus: u(id::MIX_FRONT_FOCUS),
            distance: u(id::MIX_DISTANCE),
            texture_budget: u(id::MIX_TEXTURE_BUDGET),
            refocus: u(id::MIX_REFOCUS),
            level_db,
            clarity: st.get(id::MIX_CLARITY).clamp(0.0, 1.0),
            density: st.get(id::MIX_DENSITY).clamp(0.0, 1.0),
            soloists: (st.get(id::MIX_SOLOISTS).round() as usize).clamp(1, MAX_SOLOISTS),
            target_lufs: st.get(id::MIX_LOUDNESS).clamp(-36.0, -10.0),
            dynamics: st.get(id::MIX_DYNAMICS).clamp(0.0, 1.0),
            focus,
        }
    }

    /// Concurrent bird phrases allowed (None = unlimited).
    pub fn max_phrases(&self) -> Option<usize> {
        if self.density >= 0.99 || self.clarity <= 0.0 {
            None
        } else {
            Some(1 + (self.density * 6.0) as usize)
        }
    }

    /// Minimum gap between two phrase starts, s (staggering).
    pub fn stagger_s(&self) -> f32 {
        if self.max_phrases().is_none() {
            0.0
        } else {
            (0.15 + 0.85 * (1.0 - self.density)) * self.clarity.min(1.0)
        }
    }
}

/// What the mix tells the engine for one emitter and block.
#[derive(Clone, Copy, Debug)]
pub struct Voice<'a> {
    /// Linear gain: calibration × mix level (pascals at 1 m out).
    pub gain: f32,
    /// Path EQ on the propagation fit grid, dB.
    pub eq_db: &'a [f32; NF],
    /// Reverb send multiplier (1 = physical).
    pub send: f32,
    /// May start a new phrase this block (false: the director's activity is held at 0).
    pub permit: bool,
}

/// Mixing role of an emitter right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Bed,
    /// Event-like but not in the foreground: lower, wetter, darker.
    Demoted,
    Soloist(usize),
}

#[derive(Clone, Copy)]
struct Slot {
    key: u64,
    used: bool,
    class: SoundClass,
    group: usize,
    cal: f32,
    dist: f32,
    rel: Vec3,
    speed: f32,
    /// Listener power estimates (Pa²): fast (≈ 60 ms) and slow (≈ 2 s).
    fast: f32,
    slow: f32,
    novelty: f32,
    salience: f32,
    since_solo: f32,
    role: Role,
    role_t: f32,
    quiet_t: f32,
    busy: bool,
    wants: bool,
    flying: bool,
    permit: bool,
    waiting: f32,
    /// Smoothed mix gain (dB) and send.
    gain_db: f32,
    send: f32,
    seen: bool,
    /// Smoothed level trim, dB.
    trim_db: f32,
    /// Smoothed scene-focus gain (front focus, refocus, texture tail), dB: the controller
    /// outputs, divided out of `phys`.
    ctl_db: f32,
    /// Its attention part (front focus, refocus), dB.
    attn_db: f32,
    /// Angular velocity seen from the listener, rad/s (smoothed): flybys.
    omega: f32,
    /// Cosine between the head's forward axis and the source; front weight 0..1.
    cos: f32,
    front: f32,
    /// Slow listener power without the scene-focus controller gains (Pa²): texture priority.
    phys: f32,
    /// Slow power while singing (birds, ~10 s): the bird anchor of the stationary refocus.
    presence: f32,
    /// Texture budget: protected stream, or the share of the tail cut it takes (0..1).
    protected: bool,
    tail_w: f32,
    /// Stationary-refocus anchor.
    anchor: bool,
    /// Event ceiling (birds): the loudest recent phrase before the ceiling, referred to 1 m
    /// (Pa²·m², held ~30 s), and the smoothed cut, dB (≤ 0).
    peak_raw: f32,
    cap_db: f32,
    /// ≈ 10 ms listener power (Pa²) for that peak.
    burst: f32,
}

impl Slot {
    fn empty() -> Self {
        Self {
            key: u64::MAX,
            used: false,
            class: SoundClass::Water,
            group: 0,
            cal: 1.0,
            dist: 1.0,
            rel: Vec3::default(),
            speed: 0.0,
            fast: 0.0,
            slow: 0.0,
            novelty: 0.0,
            salience: 0.0,
            since_solo: 60.0,
            role: Role::Bed,
            role_t: 0.0,
            quiet_t: 0.0,
            busy: false,
            wants: false,
            flying: false,
            permit: true,
            waiting: 0.0,
            gain_db: 0.0,
            send: 1.0,
            seen: false,
            trim_db: 0.0,
            ctl_db: 0.0,
            attn_db: 0.0,
            omega: 0.0,
            cos: 1.0,
            front: 0.0,
            phys: 0.0,
            presence: 0.0,
            protected: true,
            tail_w: 0.0,
            anchor: false,
            peak_raw: 0.0,
            cap_db: 0.0,
            burst: 0.0,
        }
    }
}

/// Per-emitter description the engine hands to `voice`.
#[derive(Clone, Copy, Debug)]
pub struct EmitterView {
    /// Identity: when it changes the slot's history resets (flight mode reuses slots).
    pub key: u64,
    pub class: SoundClass,
    pub kind: SourceKind,
    /// Position relative to the listener's head centre (world frame), m.
    pub rel: Vec3,
    /// The director wants it to sing (activity > 0).
    pub wants: bool,
    /// In a phrase right now, before this block renders (`Generator::busy`).
    pub busy: bool,
    /// Flying (calls in flight are never held).
    pub flying: bool,
}

/// Legacy per-kind gains (before calibration), for A/B renders.
pub fn legacy_gain(kind: SourceKind) -> f32 {
    match kind {
        SourceKind::Water => 1.0,
        SourceKind::Wind => 0.6,
        SourceKind::Bird => 0.8,
        SourceKind::Rain => 1.0,
        SourceKind::Thunder => crate::director::THUNDER_PROXY_M,
        SourceKind::Fire => 1.0,
        SourceKind::Insect => 0.8,
    }
}

/// Snapshot for telemetry (`asmr_get_mix`, mirrors `AsmrMixInfo`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct MixInfo {
    pub loudness_momentary: f32,
    pub loudness_short: f32,
    pub loudness_integrated: f32,
    /// Dynamics gain (leveller + compressor + limiter), dB.
    pub gain_db: f32,
    pub group_lufs: [f32; 8],
    pub group_sone: [f32; 8],
    /// Fraction of the group's loudness masked by everything else, 0..1.
    pub group_masking: [f32; 8],
    /// Mean duck on the group over the bands it occupies, dB (≤ 0).
    pub group_duck_db: [f32; 8],
    pub soloists: u32,
    /// Source indices (`asmr_get_sources` order) of the soloists, `u32::MAX` = none.
    pub soloist_source: [u32; 4],
    pub soloist_salience: [f32; 4],
    pub soloist_masking: [f32; 4],
    /// Bird phrases sounding now, and the limit (0 = none).
    pub phrases: u32,
    pub max_phrases: u32,
    /// Foreground loudness lost to masking (0..1), before and after ducking.
    pub masking_before: f32,
    pub masking_after: f32,
    /// Signal-to-mask ratio of the soloists in the bands that carry their loudness
    /// (loudness-weighted mean, dB), before and after ducking.
    pub smr_before_db: f32,
    pub smr_after_db: f32,
    /// Texture budget: joint loudness (sone) of all broadband textures, of the protected
    /// streams alone, and the cut on the tail (dB, ≤ 0).
    pub texture_sone: f32,
    pub texture_protected_sone: f32,
    pub texture_cut_db: f32,
    /// ... and the cut of the absolute budget on the whole bed (dB ≤ 0; half on protected).
    pub texture_bed_cut_db: f32,
    /// Sounding texture emitters, and how many of them are protected streams.
    pub textures: u32,
    pub textures_protected: u32,
    /// Listener stillness 0 (moving) .. 1 (still for a few seconds): stationary refocus.
    pub stillness: f32,
    /// Stationary-refocus anchors (source indices like `soloist_source`): water / texture,
    /// bird. `u32::MAX` = none.
    pub anchor_source: [u32; 2],
}

const INFO_WORDS: usize = core::mem::size_of::<MixInfo>() / 4;

/// Lock-free telemetry store (any thread reads, the audio thread writes).
pub struct MixShared {
    words: Box<[AtomicU32]>,
}

impl MixShared {
    pub fn new() -> Self {
        Self { words: (0..INFO_WORDS).map(|_| AtomicU32::new(0)).collect() }
    }

    fn store(&self, info: &MixInfo) {
        // SAFETY: MixInfo is repr(C), all 4-byte fields.
        let w: &[u32; INFO_WORDS] = unsafe { &*(info as *const MixInfo as *const [u32; INFO_WORDS]) };
        for (a, v) in self.words.iter().zip(w) {
            a.store(*v, Ordering::Relaxed);
        }
    }

    pub fn load(&self) -> MixInfo {
        let mut w = [0u32; INFO_WORDS];
        for (v, a) in w.iter_mut().zip(self.words.iter()) {
            *v = a.load(Ordering::Relaxed);
        }
        // SAFETY: as above; every bit pattern is a valid f32/u32.
        unsafe { core::mem::transmute::<[u32; INFO_WORDS], MixInfo>(w) }
    }
}

impl Default for MixShared {
    fn default() -> Self {
        Self::new()
    }
}

/// Opt-in per-emitter sound level probes (calibration and tests; costly, off by default).
pub struct Probe {
    pub class: SoundClass,
    pub key: u64,
    /// A-weighted meter of the calibrated signal at 1 m (Pa).
    pub meter: loudness::SoundLevelMeter,
    /// Mean of 1/r² over the run (to estimate the level at the listener).
    pub inv_r2: f64,
    pub blocks: u64,
}

pub struct Mix {
    fs: f32,
    slots: Vec<Slot>,
    pub intent: Intent,
    /// Legacy kind gains, no attention, no ducking, no dynamics (A/B "before").
    pub bypass: bool,
    analysis: Analysis,
    taps: Vec<[f32; BLOCK]>,
    /// Smoothed duck per track (bed groups), dB.
    duck: Vec<Curve>,
    duck_target: Vec<Curve>,
    /// Path EQ per group: [normal, demoted].
    eq: Vec<[[f32; NF]; 2]>,
    flat: [f32; NF],
    /// Path EQ per emitter slot: its group curve plus its own scene-focus tilt.
    slot_eq: Vec<[f32; NF]>,
    /// High-frequency tilt shape on the propagation grid, 0 below 2 kHz .. 1 towards 10 kHz.
    tilt_grid: [f32; NF],
    tilt: Curve,
    grid: [f64; NF],
    dynamics: Dynamics,
    soloist: [Option<usize>; MAX_SOLOISTS],
    time: f32,
    last_start: f32,
    phrases: usize,
    bed_db: f32,
    /// The same ambience unweighted, dB SPL (the thunder ceiling compares like with like).
    bed_lin_db: f32,
    info: MixInfo,
    probes: Option<Vec<Probe>>,
    a_fast: f32,
    a_slow: f32,
    a_speed: f32,
    a_gain: f32,
    a_presence: f32,
    /// ≈ 10 ms: the phrase-peak detector of the event ceiling (a chirp is 20–50 ms).
    a_burst: f32,
    timing: bool,
    /// Seconds spent in the mix layer since `enable_timing`.
    pub cpu_s: f64,
    /// Head -> world rotation of the listener (front focus) and listener speed, m/s.
    head: Mat3,
    speed: f32,
    stillness: Stillness,
    budget: Budget,
    /// Absolute loudness budget of the whole texture bed.
    bed_budget: Budget,
    /// Stationary-refocus anchors: slot index and key (texture, bird).
    anchors: [Option<(usize, u64)>; 2],
    /// Power share of each bed group in the frontal cone (less ducking in front).
    front_share: [f32; group::COUNT],
    /// Per analysis track this block: power with and without the user's level trims, and
    /// their smoothed ratio (the texture budget measures the untrimmed scene, so a trim stays
    /// exactly what the fader says).
    trim_pw: [[f32; 2]; TRACKS],
    untrim: [f32; TRACKS],
}

#[cfg(not(target_arch = "wasm32"))]
type Clock = std::time::Instant;
#[cfg(target_arch = "wasm32")]
type Clock = ();

/// Rough boost of the ground reflection on the listener power (+2 dB, incoherent).
const GROUND_POWER: f32 = 1.6;
/// Below this listener level nothing counts as sounding, dB SPL.
const QUIET_DB: f32 = 12.0;
/// Event ceiling: a bird's phrases (LAFmax-like, 60 ms) stay within this many dB of the
/// A-weighted ambience (water, wind, rain) at the listener; docs/comfort.md. Physically a
/// chaffinch 7 m away sings ~20 dB over a brook 4 m away: a startle, not a forest to work in.
const EVENT_CEIL_DB: f32 = 9.0;
/// Most the ceiling takes off a phrase, dB: enough for a bird calling as it flies past at
/// 3 m (a nightingale there is ~30 dB over the brook), while the nearness cues stay.
const EVENT_MAX_CUT_DB: f32 = 18.0;
/// Hold of the phrase-peak memory, s.
const EVENT_PEAK_HOLD_S: f32 = 30.0;
/// Thunder ceiling: each peal (10 ms power at the listener, unweighted) within this many dB
/// of the unweighted ambience, as a per-emitter compressor. The thunder proxy sits ~40 m away,
/// so the mix hears a peal ~0.1 s before it is heard: the cut is in place at the onset.
const THUNDER_CEIL_DB: f32 = 9.0;
const THUNDER_MAX_CUT_DB: f32 = 20.0;

impl Mix {
    pub fn new(sample_rate: f32) -> Self {
        let dt = BLOCK as f32 / sample_rate;
        let coef = |tau: f32| 1.0 - (-dt / tau).exp();
        let grid = crate::shaping::grid_freqs(sample_rate);
        // Distance cue of a demoted source: up to −4 dB towards 10 kHz (air, foliage).
        let shape = |f: f32| ((f / 2000.0).log2() / 2.3).clamp(0.0, 1.0);
        let mut tilt = [0.0f32; BANDS];
        for (z, t) in tilt.iter_mut().enumerate() {
            *t = -4.0 * shape(loudness::centre(z));
        }
        let tilt_grid: [f32; NF] = core::array::from_fn(|i| shape(grid[i] as f32));
        Self {
            fs: sample_rate,
            slots: vec![Slot::empty(); MAX_SLOTS],
            intent: Intent::default(),
            bypass: false,
            analysis: Analysis::new(sample_rate, TRACKS),
            taps: vec![[0.0; BLOCK]; TRACKS + 1],
            duck: vec![[0.0; BANDS]; TRACKS + 1],
            duck_target: vec![[0.0; BANDS]; TRACKS + 1],
            eq: vec![[[0.0; NF]; 2]; group::COUNT],
            flat: [0.0; NF],
            slot_eq: vec![[0.0; NF]; MAX_SLOTS],
            tilt_grid,
            tilt,
            grid,
            dynamics: Dynamics::new(sample_rate),
            soloist: [None; MAX_SOLOISTS],
            time: 0.0,
            last_start: -10.0,
            phrases: 0,
            bed_db: 0.0,
            bed_lin_db: 0.0,
            info: MixInfo::default(),
            probes: None,
            a_fast: coef(0.06),
            a_slow: coef(2.0),
            a_speed: coef(0.3),
            a_gain: coef(0.25),
            a_presence: coef(10.0),
            a_burst: coef(0.01),
            timing: false,
            cpu_s: 0.0,
            head: Mat3::from_head(0.0, 0.0, 0.0),
            speed: 0.0,
            stillness: Stillness::default(),
            budget: Budget::default(),
            bed_budget: Budget::default(),
            anchors: [None; 2],
            front_share: [0.0; group::COUNT],
            trim_pw: [[0.0; 2]; TRACKS],
            untrim: [1.0; TRACKS],
        }
    }

    /// The listener for this block: head -> world rotation (the frame of `EmitterView::rel`)
    /// and travel speed, m/s (0 when not flying). Call before the `voice` calls.
    pub fn set_listener(&mut self, head: &Mat3, speed: f32) {
        self.head = *head;
        self.speed = if speed.is_finite() { speed.max(0.0) } else { 0.0 };
    }

    /// How strongly the scene focus acts (fades in with clarity: 0 = raw physical mix).
    fn focus_on(&self) -> f32 {
        if self.bypass {
            0.0
        } else {
            (self.intent.clarity / 0.3).clamp(0.0, 1.0)
        }
    }

    /// Stationary refocus amount now, 0..1.
    fn refocus_now(&self) -> f32 {
        self.intent.refocus * self.focus_on() * self.stillness.still
    }

    pub fn set_bypass(&mut self, on: bool) {
        self.bypass = on;
        self.dynamics.bypass = on;
    }

    /// Starts per-emitter probes (allocates; not on the audio thread in production).
    pub fn enable_probes(&mut self) {
        self.probes = Some(Vec::with_capacity(MAX_SLOTS));
    }

    pub fn probes(&self) -> &[Probe] {
        self.probes.as_deref().unwrap_or(&[])
    }

    pub fn info(&self) -> &MixInfo {
        &self.info
    }

    pub fn role(&self, idx: usize) -> Role {
        self.slots.get(idx).map_or(Role::Bed, |s| s.role)
    }

    /// Start of a control block.
    pub fn begin(&mut self, intent: Intent, dt: f32) {
        self.intent = intent;
        self.time += dt;
        self.stillness.update(self.speed, dt);
        for t in self.taps.iter_mut() {
            *t = [0.0; BLOCK];
        }
        for s in self.slots.iter_mut() {
            s.seen = false;
        }
        self.phrases = 0;
        for t in self.trim_pw.iter_mut() {
            *t = [0.0; 2];
        }
    }

    /// Before emitter `idx` renders: its gain, path EQ, send and phrase permit.
    pub fn voice(&mut self, idx: usize, v: &EmitterView) -> Voice<'_> {
        let t0 = self.clock();
        let (gain, eq, tilt, send, permit) = self.voice_inner(idx, v);
        let own = idx < MAX_SLOTS && (tilt != 0.0 || eq.is_some());
        if own {
            let base = match eq {
                None => self.flat,
                Some((g, d)) => self.eq[g][d as usize],
            };
            let out = &mut self.slot_eq[idx];
            for i in 0..NF {
                out[i] = base[i] + tilt * self.tilt_grid[i];
            }
        }
        self.lap(t0);
        let eq_db = if own { &self.slot_eq[idx] } else { &self.flat };
        Voice { gain, eq_db, send, permit }
    }

    #[inline]
    fn clock(&self) -> Option<Clock> {
        #[cfg(not(target_arch = "wasm32"))]
        if self.timing {
            return Some(std::time::Instant::now());
        }
        None
    }

    #[inline]
    fn lap(&mut self, t0: Option<Clock>) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(t) = t0 {
            self.cpu_s += t.elapsed().as_secs_f64();
        }
        #[cfg(target_arch = "wasm32")]
        let _ = t0;
    }

    /// Starts accumulating the mix layer's own CPU time (`cpu_s`).
    pub fn enable_timing(&mut self) {
        self.timing = true;
    }

    #[allow(clippy::type_complexity)]
    fn voice_inner(&mut self, idx: usize, v: &EmitterView) -> (f32, Option<(usize, bool)>, f32, f32, bool) {
        let on = self.focus_on();
        let refocus = self.refocus_now();
        let head = self.head;
        let tail_cut = self.budget.cut_db;
        let bed_cut = self.bed_budget.cut_db;
        let Some(s) = self.slots.get_mut(idx) else {
            return (v.class.gain(), None, 0.0, 1.0, true);
        };
        if s.key != v.key || !s.used {
            *s = Slot::empty();
            s.key = v.key;
            s.used = true;
            s.class = v.class;
            s.group = v.class.group();
            s.cal = v.class.gain();
            s.rel = v.rel;
        }
        s.seen = true;
        let dt = BLOCK as f32 / self.fs;
        let dist = v.rel.length().max(0.3);
        let dv = (v.rel - s.rel) * (1.0 / dt);
        let vel = dv.length();
        s.speed += (vel.min(60.0) - s.speed) * self.a_speed;
        // Angular velocity seen from the listener: what makes a flyby (fast panning).
        let rhat = v.rel * (1.0 / dist);
        let tangential = (dv - rhat * dv.dot(rhat)).length();
        s.omega += ((tangential / dist).min(10.0) - s.omega) * self.a_speed;
        s.rel = v.rel;
        s.dist = dist;
        s.wants = v.wants;
        s.flying = v.flying;
        s.busy = v.busy;
        if self.bypass {
            s.gain_db = 0.0;
            // The old propagation fed the late reverb without the emitter gain: keep that.
            let g = legacy_gain(v.kind);
            return (g, None, 0.0, 1.0 / g, true);
        }

        // Level and cues from the role and the intent.
        let c = self.intent.clarity;
        let focus = self.intent.focus[s.group];
        let mut gain_db = if focus >= 0.0 { 3.0 * focus } else { 6.0 * focus };
        let mut send = 1.0;
        let mut demoted = focus < -0.2;
        match s.role {
            Role::Soloist(_) => {
                gain_db += 1.5 * c;
                send = 1.0 - 0.3 * c;
            }
            Role::Demoted => {
                gain_db -= 2.0 + 6.0 * c;
                send = 1.0 + c;
                demoted = true;
            }
            Role::Bed => {}
        }
        if focus < 0.0 {
            send *= 1.0 - 0.5 * focus;
        }
        let solo = matches!(s.role, Role::Soloist(_));

        // Scene focus (focus.rs): direction relative to the head, distance, refocus, budget.
        let local = head.apply_transposed(v.rel);
        s.cos = (local.x / dist).clamp(-1.0, 1.0);
        let (fw, _, _) = focus::direction_weights(s.cos);
        s.front = fw;
        // Textures of a group in focus are foreground: no budget or refocus pushback.
        let tex = texture_kind(s.class).is_some() && !solo && focus <= 0.25;
        // Thunder is a direction proxy (its distance physics is in the generator). An anchor of
        // the stationary refocus is brought closer: attending to a sound shrinks its distance.
        let closer = if s.anchor { 1.0 - 0.7 * refocus } else { 1.0 };
        let dcue = if s.class == SoundClass::Thunder { Cue::NONE } else { focus::distance_cue(dist, self.intent.distance * on * closer) };
        // Looking at a texture lifts it half as much as an event (attention selects objects).
        let fcue = focus::front_cue(s.cos, self.intent.front_focus * on, solo);
        let fcue = if tex { Cue { db: if fcue.db > 0.0 { 0.5 * fcue.db } else { fcue.db }, ..fcue } } else { fcue };
        let bird = matches!(s.class, SoundClass::Bird(_));
        // Stationary refocus: the anchors come forward and drier, the rest steps back.
        let rcue = if refocus <= 0.0 {
            Cue::NONE
        } else if s.anchor {
            Cue { db: 1.5 * refocus, tilt_db: 0.0, send: 1.0 - 0.3 * refocus }
        } else if tex || (bird && !solo) {
            Cue { db: if tex { -3.0 } else { -2.0 } * refocus, tilt_db: -2.0 * refocus, send: 1.0 + 0.5 * refocus }
        } else {
            Cue::NONE
        };
        let tail_db = if !tex {
            0.0
        } else if s.protected || s.anchor {
            0.5 * bed_cut * on
        } else {
            (tail_cut * s.tail_w + bed_cut) * on
        };
        s.ctl_db += (fcue.db + rcue.db + tail_db - s.ctl_db) * self.a_gain;
        s.attn_db += (fcue.db + rcue.db - s.attn_db) * self.a_gain;
        let trim = self.intent.level_db[level_index(s.class)];
        s.trim_db += (trim - s.trim_db) * self.a_gain;

        s.gain_db += (gain_db - s.gain_db) * self.a_gain;
        s.send += (send * rcue.send - s.send) * self.a_gain;
        // Event ceiling (birds): the phrase peak at the listener — measured once the bird has
        // sung, predicted from its calibrated LAFmax and distance before — stays within
        // EVENT_CEIL_DB of the ambience. Set before a phrase starts, so onsets are covered too.
        // Fades in with the dynamics intent like the comfort ceiling (0.3..0.6).
        let k_dyn = ((self.intent.dynamics - 0.3) / 0.3).clamp(0.0, 1.0);
        let thunder = s.class == SoundClass::Thunder;
        let k_cap = k_dyn * k_dyn * (3.0 - 2.0 * k_dyn);
        let want_cap = if k_dyn <= 0.0 || self.bed_db <= QUIET_DB {
            0.0
        } else if bird {
            let level = if s.peak_raw > 0.0 {
                db_spl(s.peak_raw / (dist * dist))
            } else {
                calibration::entry(s.class).target_db + s.gain_db + s.trim_db + s.ctl_db + dcue.db - 20.0 * dist.log10()
            };
            (self.bed_db + EVENT_CEIL_DB - level).clamp(-EVENT_MAX_CUT_DB, 0.0) * k_cap
        } else if thunder {
            let level = db_spl(s.burst * db_gain_sq(-s.cap_db));
            (self.bed_lin_db + THUNDER_CEIL_DB - level).clamp(-THUNDER_MAX_CUT_DB, 0.0) * k_cap
        } else {
            0.0
        };
        // Down fast (a bird: before the next note; thunder: within ~10 ms), up slowly.
        let a_cap = if want_cap >= s.cap_db {
            self.a_slow
        } else if thunder {
            self.a_burst
        } else {
            self.a_fast
        };
        s.cap_db += (want_cap - s.cap_db) * a_cap;
        // Calls in flight are short and carry motion: never held.
        let permit = s.permit || v.flying;
        let gain = if trim <= MUTE_DB && s.trim_db < -39.0 {
            0.0
        } else {
            s.cal * 10f32.powf((s.gain_db + s.trim_db + s.ctl_db + dcue.db + s.cap_db) / 20.0)
        };
        let eq = match s.role {
            Role::Soloist(_) => None,
            _ => Some((s.group, demoted)),
        };
        let tilt = dcue.tilt_db + fcue.tilt_db + rcue.tilt_db;
        (gain, eq, tilt, s.send * dcue.send, permit)
    }

    /// After emitter `idx` rendered `signal` (raw generator output; `gain` = what goes to the
    /// propagation); `busy` = in a phrase after this block.
    pub fn feed(&mut self, idx: usize, signal: &[f32], gain: f32, busy: bool) {
        let t0 = self.clock();
        self.feed_inner(idx, signal, gain, busy);
        self.lap(t0);
    }

    fn feed_inner(&mut self, idx: usize, signal: &[f32], gain: f32, busy: bool) {
        let pb = if self.bypass { 1.0 / self.dynamics_playback_nominal() } else { 1.0 };
        let Some(s) = self.slots.get_mut(idx) else { return };
        if busy && !s.busy {
            self.last_start = self.time;
        }
        s.busy = busy;
        if busy && matches!(s.class, SoundClass::Bird(_)) && !s.flying {
            self.phrases += 1;
        }
        let len = signal.len().min(BLOCK);
        // Listener estimate: 1/r spreading plus the ground reflection.
        let k = gain / s.dist;
        let k = k * pb;
        let energy: f32 = signal[..len].iter().map(|x| x * x).sum::<f32>() / len.max(1) as f32;
        let p = energy * k * k * GROUND_POWER;
        s.fast += (p - s.fast) * self.a_fast;
        s.slow += (p - s.slow) * self.a_slow;
        if matches!(s.class, SoundClass::Thunder) {
            s.burst += (p - s.burst) * self.a_burst;
        }
        if matches!(s.class, SoundClass::Bird(_)) {
            // Phrase peak before the event ceiling (what the bird would be without it).
            s.burst += (p - s.burst) * self.a_burst;
            let hold = (-(len as f32) / (EVENT_PEAK_HOLD_S * self.fs)).exp();
            // Referred to 1 m (× r²): a bird that flies in or hops closer is judged at its new
            // distance at once.
            let raw = if busy { s.burst * db_gain_sq(-s.cap_db) * s.dist * s.dist } else { 0.0 };
            s.peak_raw = (s.peak_raw * hold).max(raw);
        }
        // Without the scene-focus controller gains: what the texture priority ranks.
        let phys = p * db_gain_sq(-s.ctl_db);
        s.phys += (phys - s.phys) * self.a_slow;
        let sing = if busy { phys } else { 0.0 };
        s.presence += (sing - s.presence) * self.a_presence;
        let track = match s.role {
            Role::Soloist(i) => group::COUNT + i,
            _ => s.group,
        };
        let tail = !s.protected && !s.anchor && texture_kind(s.class).is_some() && !matches!(s.role, Role::Soloist(_));
        if energy > 0.0 {
            // The scene as the budget judges it: without the user's trims and the attention gains
            // (front focus, refocus), so neither is undone by the bed budget.
            let raw = p * db_gain_sq(-(s.trim_db + s.attn_db));
            self.trim_pw[track][0] += p;
            self.trim_pw[track][1] += raw;
            if tail {
                self.trim_pw[TAIL][0] += p;
                self.trim_pw[TAIL][1] += raw;
            }
            let k = k * GROUND_POWER.sqrt();
            let tap = &mut self.taps[track];
            for (t, x) in tap[..len].iter_mut().zip(signal) {
                *t += x * k;
            }
            if tail {
                let tap = &mut self.taps[TAIL];
                for (t, x) in tap[..len].iter_mut().zip(signal) {
                    *t += x * k;
                }
            }
        }
        if let Some(probes) = self.probes.as_mut() {
            let key = s.key;
            let pi = match probes.iter().position(|q| q.key == key) {
                Some(i) => i,
                None => {
                    probes.push(Probe { class: s.class, key, meter: loudness::SoundLevelMeter::new(self.fs), inv_r2: 0.0, blocks: 0 });
                    probes.len() - 1
                }
            };
            let q = &mut probes[pi];
            q.meter.process(&signal[..len], gain * pb);
            q.inv_r2 += 1.0 / (s.dist * s.dist) as f64;
            q.blocks += 1;
        }
    }

    fn dynamics_playback_nominal(&self) -> f32 {
        10f32.powf((calibration::PA_DB - calibration::PLAYBACK_REF_DB) / 20.0)
    }

    /// End of the emitter loop: analysis, attention, ducking, phrase permits.
    pub fn finish(&mut self, len: usize) {
        let t0 = self.clock();
        self.finish_inner(len);
        self.lap(t0);
    }

    fn finish_inner(&mut self, len: usize) {
        // Slots not rendered this block (flight mode churn) drop out.
        for s in self.slots.iter_mut() {
            if s.used && !s.seen {
                if let Role::Soloist(k) = s.role {
                    self.soloist[k] = None;
                }
                *s = Slot::empty();
            }
        }
        let dt = len as f32 / self.fs;
        self.update_salience(dt);
        self.update_permits(dt);
        if self.analysis.push(&self.taps[..TRACKS], len) {
            self.analysis.analyse();
            self.select_soloists();
            self.update_textures();
            self.update_ducking();
            self.update_info();
        }
    }

    /// Output gains for the block (per sample, linear, includes the playback mapping).
    pub fn master(&mut self, l: &[f32], r: &[f32], out: &mut [f32]) {
        let t0 = self.clock();
        let p = DynParams { target_lufs: self.intent.target_lufs, strength: self.intent.dynamics };
        self.dynamics.process(l, r, &p, out);
        self.lap(t0);
    }

    /// Full telemetry snapshot, also stored into `shared`.
    pub fn publish(&self, shared: &MixShared) -> MixInfo {
        let d = &self.dynamics;
        let mut info = self.info;
        info.loudness_momentary = d.post.momentary;
        info.loudness_short = d.post.short_term;
        info.loudness_integrated = d.post.integrated;
        info.gain_db = d.gain_db();
        shared.store(&info);
        info
    }

    pub fn reset_integrated(&mut self) {
        self.dynamics.post.reset_integrated();
    }

    // ---------------------------------------------------------------------------------
    // Attention
    // ---------------------------------------------------------------------------------

    fn update_salience(&mut self, dt: f32) {
        let decay = (-dt / 1.5).exp();
        let bed_db = self.bed_db;
        let on = self.focus_on();
        let front_w = 0.3 * self.intent.front_focus * on;
        // While travelling, flybys (fast panning nearby) weigh more; when still, the anchor.
        let motion_w = 0.2 + 0.3 * self.stillness.moving * on;
        let anchor_w = 0.3 * self.refocus_now();
        for s in self.slots.iter_mut().filter(|s| s.used) {
            // Attention judges the sound as it is, before the event ceiling (a level trim only).
            let uncap = db_gain_sq(-s.cap_db);
            let l_fast = db_spl(s.fast * uncap);
            let l_slow = db_spl(s.slow * uncap);
            let onset = ((l_fast - l_slow - 3.0) / 12.0).clamp(0.0, 1.0);
            s.novelty = (s.novelty * decay).max(onset);
            let prox = ((l_fast - bed_db + 6.0) / 18.0).clamp(0.0, 1.0);
            let near = (1.5 - s.dist / 20.0).clamp(0.0, 1.0);
            let linear = ((s.speed - 1.0) / 8.0).clamp(0.0, 1.0);
            let angular = focus::smoothstep(0.2, 1.2, s.omega);
            let motion = linear.max(angular) * near;
            let rarity = 0.7 * s.class.rarity() + 0.3 * (s.since_solo / 45.0).min(1.0);
            let focus = self.intent.focus[s.group];
            let anchor = if s.anchor { anchor_w } else { 0.0 };
            let sounding = l_fast > (bed_db - 10.0).max(QUIET_DB) && (s.busy || !matches!(s.class, SoundClass::Bird(_)));
            s.salience = if sounding {
                (0.35 * prox + 0.25 * s.novelty + motion_w * motion + 0.2 * rarity + 0.3 * focus + front_w * s.front + anchor).clamp(0.0, 1.0)
            } else {
                0.0
            };
            s.quiet_t = if sounding { 0.0 } else { s.quiet_t + dt };
            s.role_t += dt;
            if matches!(s.role, Role::Soloist(_)) {
                s.since_solo = 0.0;
            } else {
                s.since_solo += dt;
            }
        }
    }

    fn select_soloists(&mut self) {
        // In bypass the roles are still tracked (analysis only: nothing is applied), so A/B
        // numbers describe the same sources.
        let n = if self.intent.clarity > 0.0 { self.intent.soloists } else { 0 };
        // Release: silent for a while, or faded and held long enough; or slots shrank.
        for k in 0..MAX_SOLOISTS {
            let Some(i) = self.soloist[k] else { continue };
            let s = &self.slots[i];
            if k >= n || s.quiet_t > 0.8 || (s.salience < 0.12 && s.role_t > 2.0) {
                self.slots[i].role = Role::Bed;
                self.slots[i].role_t = 0.0;
                self.soloist[k] = None;
            }
        }
        // Candidates by salience.
        loop {
            let mut best: Option<usize> = None;
            for (i, s) in self.slots.iter().enumerate() {
                if !s.used || !s.class.soloable() || matches!(s.role, Role::Soloist(_)) || s.salience < 0.3 {
                    continue;
                }
                if best.map_or(true, |b| s.salience > self.slots[b].salience) {
                    best = Some(i);
                }
            }
            let Some(b) = best else { break };
            let free = (0..n).find(|&k| self.soloist[k].is_none());
            let k = match free {
                Some(k) => k,
                None => {
                    // Replace the weakest if clearly beaten and held long enough.
                    let Some(weak) = (0..n).filter_map(|k| self.soloist[k].map(|i| (k, i))).min_by(|a, b| self.slots[a.1].salience.total_cmp(&self.slots[b.1].salience)) else { break };
                    let w = &self.slots[weak.1];
                    if self.slots[b].salience > w.salience + 0.2 && w.role_t > 1.5 {
                        self.slots[weak.1].role = Role::Bed;
                        self.slots[weak.1].role_t = 0.0;
                        weak.0
                    } else {
                        break;
                    }
                }
            };
            self.soloist[k] = Some(b);
            self.slots[b].role = Role::Soloist(k);
            self.slots[b].role_t = 0.0;
        }
        // Demote the other event-like sources that sound while the foreground is full.
        let full = n > 0 && (0..n).all(|k| self.soloist[k].is_some());
        let sounding_events = self.slots.iter().filter(|s| s.used && s.class.soloable() && s.salience > 0.0).count();
        let crowded = full && sounding_events > n;
        for s in self.slots.iter_mut().filter(|s| s.used) {
            match s.role {
                Role::Soloist(_) => {}
                _ if crowded && s.class.soloable() => {
                    if s.role != Role::Demoted {
                        s.role = Role::Demoted;
                        s.role_t = 0.0;
                    }
                }
                Role::Demoted if s.role_t > 1.0 => {
                    s.role = Role::Bed;
                    s.role_t = 0.0;
                }
                _ => {}
            }
        }
    }

    /// Phrase permits for the next block: at most `max_phrases` bird phrases at once, starts
    /// at least `stagger_s` apart, the most salient waiting bird first.
    fn update_permits(&mut self, dt: f32) {
        let limit = if self.bypass { None } else { self.intent.max_phrases() };
        for s in self.slots.iter_mut().filter(|s| s.used) {
            s.waiting = if s.wants && !s.busy { s.waiting + dt } else { 0.0 };
            s.permit = true;
        }
        let Some(limit) = limit else { return };
        let gap_ok = self.time - self.last_start >= self.intent.stagger_s();
        let free = if gap_ok { limit.saturating_sub(self.phrases) } else { 0 };
        // Rank the waiting birds; permit the best `free` ones.
        let mut granted = 0;
        let mut taken = [usize::MAX; 8];
        let front_w = 0.5 * self.intent.front_focus * self.focus_on();
        let anchor_w = self.refocus_now();
        while granted < free.min(taken.len()) {
            let mut best: Option<(usize, f32)> = None;
            for (i, s) in self.slots.iter().enumerate() {
                if !s.used || !matches!(s.class, SoundClass::Bird(_)) || s.busy || !s.wants || taken[..granted].contains(&i) {
                    continue;
                }
                let solo = if matches!(s.role, Role::Soloist(_)) { 1.0 } else { 0.0 };
                let anchor = if s.anchor { anchor_w } else { 0.0 };
                let prio = 1.0 / (1.0 + s.dist / 10.0) + s.class.rarity() + (s.waiting / 20.0).min(1.0) + 0.5 * self.intent.focus[s.group].max(0.0) + solo + front_w * s.front + anchor;
                if best.map_or(true, |(_, p)| prio > p) {
                    best = Some((i, prio));
                }
            }
            let Some((i, _)) = best else { break };
            taken[granted] = i;
            granted += 1;
        }
        for (i, s) in self.slots.iter_mut().enumerate() {
            if s.used && matches!(s.class, SoundClass::Bird(_)) && !s.busy {
                s.permit = taken[..granted].contains(&i);
            }
        }
    }

    // ---------------------------------------------------------------------------------
    // Scene focus: texture budget and stationary-refocus anchors (focus.rs)
    // ---------------------------------------------------------------------------------

    /// Texture priority (dB-like): the physical level at the listener (after the distance
    /// law, without the controller gains), intent focus, the head's front and, while
    /// travelling, motion past the listener.
    fn texture_priority(&self, s: &Slot) -> f32 {
        let on = self.focus_on();
        db_spl(s.phys)
            + 6.0 * self.intent.focus[s.group]
            + 3.0 * self.intent.front_focus * on * s.front
            + 6.0 * self.stillness.moving * on * focus::smoothstep(0.2, 1.2, s.omega)
    }

    fn update_textures(&mut self) {
        let hop_s = HOP as f32 / self.fs;
        for (u, [pw, raw]) in self.untrim.iter_mut().zip(self.trim_pw) {
            let f = if pw > 0.0 { (raw / pw).min(1e4) } else { *u };
            *u += (f - *u) * 0.2;
        }
        let on = self.focus_on();
        let focus_g = self.intent.focus;
        let sounding = |s: &Slot| s.used && texture_kind(s.class).is_some() && !matches!(s.role, Role::Soloist(_)) && focus_g[s.group] <= 0.25 && db_spl(s.phys) > QUIET_DB;

        // Anchors of the stationary refocus, with hysteresis (3 dB better to take over).
        // 0: the strongest water (the nearest brook), 1: the bird heard most over ~10 s.
        for (a, pick) in [(0usize, 0u8), (1, 1)] {
            let score = |s: &Slot| -> Option<f32> {
                match pick {
                    0 if s.used && s.class == SoundClass::Water && sounding(s) && focus_g[s.group] >= -0.2 => Some(self.texture_priority(s)),
                    1 if s.used && matches!(s.class, SoundClass::Bird(_)) && focus_g[s.group] >= -0.2 && db_spl(s.presence) > QUIET_DB => Some(db_spl(s.presence) + 6.0 * self.intent.focus[s.group]),
                    _ => None,
                }
            };
            let current = self.anchors[a].filter(|&(i, key)| self.slots[i].key == key).and_then(|(i, _)| score(&self.slots[i]).map(|v| (i, v)));
            let mut best: Option<(usize, f32)> = None;
            for (i, s) in self.slots.iter().enumerate() {
                if let Some(v) = score(s) {
                    if best.map_or(true, |(_, b)| v > b) {
                        best = Some((i, v));
                    }
                }
            }
            let next = match (current, best) {
                (Some((ci, cv)), Some((bi, bv))) if bi != ci && bv > cv + 3.0 => Some(bi),
                (Some((ci, _)), _) => Some(ci),
                (None, b) => b.map(|(i, _)| i),
            };
            self.anchors[a] = next.map(|i| (i, self.slots[i].key));
        }
        for s in self.slots.iter_mut() {
            s.anchor = false;
        }
        for (i, _) in self.anchors.iter().flatten() {
            self.slots[*i].anchor = true;
        }

        // Ranking: protected = anchors, the strongest stream of each texture class, and the
        // KEEP_STREAMS strongest overall. The rest is the tail, ranked again for its weights.
        // Sounding textures, sorted by priority (insertion sort on the stack: tens of them).
        let mut order = [(0u16, 0.0f32); MAX_SLOTS];
        let mut n = 0;
        let mut class_best = [usize::MAX; TEXTURE_KINDS];
        for i in 0..MAX_SLOTS {
            // Not a sounding texture (or not ranked yet): protected, no tail cut.
            self.slots[i].protected = true;
            self.slots[i].tail_w = 0.0;
            if !sounding(&self.slots[i]) {
                continue;
            }
            let p = self.texture_priority(&self.slots[i]);
            let mut k = n;
            while k > 0 && order[k - 1].1 < p {
                order[k] = order[k - 1];
                k -= 1;
            }
            order[k] = (i as u16, p);
            n += 1;
        }
        for &(i, _) in &order[..n] {
            let c = texture_kind(self.slots[i as usize].class).unwrap_or(0);
            if class_best[c] == usize::MAX {
                class_best[c] = i as usize;
            }
        }
        let (mut n_prot, mut tail_rank) = (0u32, 0usize);
        for (rank, &(i, _)) in order[..n].iter().enumerate() {
            let i = i as usize;
            let s = &mut self.slots[i];
            let best_of_class = class_best[texture_kind(s.class).unwrap_or(0)] == i;
            s.protected = s.anchor || best_of_class || rank < focus::KEEP_STREAMS || on <= 0.0;
            if s.protected {
                n_prot += 1;
            } else {
                s.tail_w = focus::tail_weight(tail_rank);
                tail_rank += 1;
            }
        }
        let n_tex = n as u32;

        // Joint loudness of every texture vs the protected streams alone (the tail track is a
        // copy of the tail inside the group tracks).
        let a = &self.analysis;
        let bed_tex = |g: usize| self.intent.focus[g] <= 0.25;
        let (mut n_all, mut n_prot_sone) = (0.0f32, 0.0f32);
        for z in 0..BANDS {
            let e_all: f32 = TEXTURE_GROUPS.iter().filter(|g| bed_tex(**g)).map(|g| a.exc[*g][z] * self.untrim[*g]).sum();
            if e_all <= 0.0 {
                continue;
            }
            let e_prot = (e_all - a.exc[TAIL][z] * self.untrim[TAIL]).max(0.0);
            n_all += a.model.specific(z, e_all, 0.0);
            n_prot_sone += a.model.specific(z, e_prot, 0.0);
        }
        // The protected streams count at least TAIL_FLOOR_SONE: a soft, quiet bed keeps its tail.
        let ratio = if n_all > 0.05 { n_all / n_prot_sone.max(focus::TAIL_FLOOR_SONE) } else { 1.0 };
        // Standing still tightens the budget (the rest steps further back).
        let refocus = self.refocus_now();
        let limit = if on > 0.0 { focus::budget_limit(self.intent.texture_budget).map(|l| (l - 0.2 * refocus).max(1.02)) } else { None };
        self.budget.update(ratio, limit, focus::TAIL_MAX_DB, hop_s);
        // The whole bed against the absolute budget (sone): loud textures step back and the
        // leveller lifts the scene, so events gain on the bed. A slow integrator on the
        // measured (already cut) loudness: no oscillation.
        let sone = if on > 0.0 { focus::budget_sone(self.intent.texture_budget).map(|b| b * (1.0 - 0.15 * refocus)) } else { None };
        let bed_ratio = match sone {
            Some(b) if n_all > 0.05 => n_all / b,
            _ => 1.0,
        };
        // Under budget the cut is released towards 0 (it never boosts).
        let max_db = focus::BED_MAX_DB * (1.0 + ((0.35 - self.intent.texture_budget) / 0.35).clamp(0.0, 1.0));
        self.bed_budget.update(bed_ratio, sone.map(|_| 1.0), max_db, hop_s);
        let info = &mut self.info;
        info.texture_sone += (n_all - info.texture_sone) * 0.05;
        info.texture_protected_sone += (n_prot_sone - info.texture_protected_sone) * 0.05;
        info.texture_cut_db = self.budget.cut_db;
        info.texture_bed_cut_db = self.bed_budget.cut_db;
        info.textures = n_tex;
        info.textures_protected = n_prot;
        info.stillness = self.stillness.still;
        for (k, a) in self.anchors.iter().enumerate() {
            info.anchor_source[k] = a.map_or(u32::MAX, |(i, _)| i as u32);
        }

        // Share of each group's power in the frontal cone.
        let mut pf = [0.0f32; group::COUNT];
        let mut pt = [0.0f32; group::COUNT];
        for s in self.slots.iter().filter(|s| s.used) {
            pf[s.group] += s.slow * s.front;
            pt[s.group] += s.slow;
        }
        for g in 0..group::COUNT {
            self.front_share[g] = if pt[g] > 0.0 { pf[g] / pt[g] } else { 0.0 };
        }
    }

    // ---------------------------------------------------------------------------------
    // Masking
    // ---------------------------------------------------------------------------------

    fn update_ducking(&mut self) {
        let a = &self.analysis;
        let c = if self.bypass { 0.0 } else { self.intent.clarity };
        // Foreground: soloists fully, groups in focus by their focus.
        let mut fg = [0.0f32; TRACKS + 1];
        let mut bed = [false; TRACKS + 1];
        for g in 0..group::COUNT {
            let f = self.intent.focus[g];
            if f > 0.25 {
                fg[g] = f;
            } else {
                bed[g] = true;
            }
        }
        // A soloist behind the head asks for less unmasking than one in front.
        let ff = self.intent.front_focus * self.focus_on();
        for k in 0..MAX_SOLOISTS {
            if let Some(i) = self.soloist[k] {
                let (_, rear, _) = focus::direction_weights(self.slots[i].cos);
                fg[group::COUNT + k] = 1.0 - 0.5 * ff * rear;
            }
        }
        // The tail track is a copy, never a masker or a foreground of its own.
        bed[TAIL] = false;
        fg[TAIL] = 0.0;
        // Ambience level (A-weighted textures: water, wind, rain) for the salience reference:
        // how far an event stands out of the background it is heard against.
        let (mut bed_p, mut bed_lin) = (0.0, 0.0);
        for g in [group::WATER, group::WIND, group::RAIN] {
            if bed[g] {
                bed_p += (0..BANDS).map(|z| a.fast[g][z] * a.model.a_w[z]).sum::<f32>();
                bed_lin += a.fast[g].iter().sum::<f32>();
            }
        }
        self.bed_db = db_spl(bed_p);
        self.bed_lin_db = db_spl(bed_lin);

        let p = DuckParams { target_smr_db: 3.0 + 9.0 * c, depth_db: 1.0 + 7.0 * c };
        if c > 0.0 {
            analysis::duck_targets(a, &fg[..TRACKS], &bed[..TRACKS], &p, &mut self.duck_target[..TRACKS]);
        } else {
            for d in self.duck_target.iter_mut() {
                *d = [0.0; BANDS];
            }
        }
        if c > 0.0 {
            // Attended (frontal) event-like bed sources (birds in the bed, fire) are ducked less;
            // textures in front still step aside for the soloists.
            for g in (0..group::COUNT).filter(|g| !TEXTURE_GROUPS.contains(g)) {
                let k = 1.0 - 0.5 * ff * self.front_share[g];
                for v in self.duck_target[g].iter_mut() {
                    *v *= k;
                }
            }
            // Spectral niches of the textures (analysis::niche_targets): each background
            // layer keeps the bands where it is relatively strongest, the others step aside
            // there (4 dB at the default clarity, up to 7 when over the texture budget).
            let over = (-(self.budget.cut_db + self.bed_budget.cut_db) / 12.0).clamp(0.0, 1.0);
            let depth = (self.focus_on() * c.min(1.0) / 0.6 * (4.0 + 3.0 * over)).min(7.0);
            analysis::niche_targets(a, &TEXTURE_GROUPS, &bed[..TRACKS], depth, &mut self.duck_target[..TRACKS]);
            for g in TEXTURE_GROUPS {
                for v in self.duck_target[g].iter_mut() {
                    *v = v.max(-(p.depth_db + 7.0));
                }
            }
        }
        // Attack ~50 ms, release ~400 ms at the hop rate.
        let hop_s = HOP as f32 / self.fs;
        let (att, rel) = (1.0 - (-hop_s / 0.05).exp(), 1.0 - (-hop_s / 0.4).exp());
        for t in 0..TRACKS {
            for z in 0..BANDS {
                let (d, tg) = (&mut self.duck[t][z], self.duck_target[t][z]);
                *d += (tg - *d) * if tg < *d { att } else { rel };
                if d.abs() < 0.01 {
                    *d = 0.0;
                }
            }
        }
        // Group curves on the propagation grid, plain and with the demotion tilt.
        let cc = c.min(1.0);
        for g in 0..group::COUNT {
            let duck = self.duck[g];
            analysis::to_grid(&duck, &self.grid, &mut self.eq[g][0]);
            let mut dem = duck;
            for z in 0..BANDS {
                dem[z] += self.tilt[z] * cc.max(0.5 * (-self.intent.focus[g]).max(0.0));
            }
            analysis::to_grid(&dem, &self.grid, &mut self.eq[g][1]);
        }
    }

    fn update_info(&mut self) {
        let a = &self.analysis;
        let pb = self.dynamics_playback_nominal();
        let g = 10f32.powf(self.dynamics.gain_db() / 10.0) * pb * pb;
        let mut info = self.info;
        // The tail track duplicates emitters of the group tracks: never counted.
        let all: [bool; TRACKS] = core::array::from_fn(|t| t != TAIL && a.fast[t].iter().any(|v| *v > 0.0));
        for t in 0..group::COUNT {
            // A group is its bed track plus its soloists (comparable with and without the
            // director, whose roles only move emitters between tracks).
            let mut members = [false; TRACKS];
            members[t] = all[t];
            for k in 0..MAX_SOLOISTS {
                if let Some(i) = self.soloist[k] {
                    members[group::COUNT + k] = self.slots[i].group == t && all[group::COUNT + k];
                }
            }
            let k: f32 = (0..TRACKS).filter(|&m| members[m]).map(|m| (0..BANDS).map(|z| a.slow[m][z] * a.model.k_w[z]).sum::<f32>()).sum();
            info.group_lufs[t] = loudness::lufs(2.0 * k * g).max(-70.0);
            let others: [bool; TRACKS] = core::array::from_fn(|m| all[m] && !members[m]);
            let (n, np) = a.partial_union(&members, &others, Some(&self.duck[..TRACKS]));
            info.group_sone[t] = n;
            info.group_masking[t] = if n > 0.01 { (1.0 - np / n).clamp(0.0, 1.0) } else { 0.0 };
            // Mean duck over the bands that carry the group.
            let (mut wsum, mut dsum) = (0.0, 0.0);
            for z in 0..BANDS {
                let w = a.exc[t][z];
                wsum += w;
                dsum += w * self.duck[t][z];
            }
            info.group_duck_db[t] = if wsum > 0.0 { dsum / wsum } else { 0.0 };
        }
        // Soloists and the foreground masking before / after ducking.
        let mut nb = 0;
        let (mut lost_before, mut lost_after, mut total) = (0.0, 0.0, 0.0);
        let (mut smr0, mut smr1, mut smr_w) = (0.0, 0.0, 0.0);
        let bed: [bool; TRACKS] = core::array::from_fn(|t| t < group::COUNT && self.intent.focus[t] <= 0.25 && all[t]);
        for k in 0..MAX_SOLOISTS {
            info.soloist_source[k] = u32::MAX;
            info.soloist_salience[k] = 0.0;
            info.soloist_masking[k] = 0.0;
            let Some(i) = self.soloist[k] else { continue };
            let t = group::COUNT + k;
            let (n, before) = a.partial(t, &bed, None);
            let (_, after) = a.partial(t, &bed, Some(&self.duck[..TRACKS]));
            let (s0, s1, w) = a.smr(t, &bed, &self.duck[..TRACKS]);
            if w > 0.0 {
                smr0 += s0 * w;
                smr1 += s1 * w;
                smr_w += w;
            }
            info.soloist_source[k] = i as u32;
            info.soloist_salience[k] = self.slots[i].salience;
            info.soloist_masking[k] = if n > 0.01 { (1.0 - after / n).clamp(0.0, 1.0) } else { 0.0 };
            if n > 0.01 {
                total += n;
                lost_before += n - before;
                lost_after += n - after;
            }
            nb += 1;
        }
        info.soloists = nb;
        let mb = if total > 0.0 { lost_before / total } else { 0.0 };
        let ma = if total > 0.0 { lost_after / total } else { 0.0 };
        // Smooth for display.
        info.masking_before += (mb - info.masking_before) * 0.05;
        info.masking_after += (ma - info.masking_after) * 0.05;
        if smr_w > 0.0 {
            info.smr_before_db += (smr0 / smr_w - info.smr_before_db) * 0.05;
            info.smr_after_db += (smr1 / smr_w - info.smr_after_db) * 0.05;
        }
        info.phrases = self.phrases as u32;
        info.max_phrases = if self.bypass { 0 } else { self.intent.max_phrases().map_or(0, |m| m as u32) };
        self.info = info;
    }
}

/// The calibrated (or legacy) mix gain of a class, for code paths without a mix.
pub fn class_gain(class: SoundClass, kind: SourceKind, bypass: bool) -> f32 {
    if bypass {
        legacy_gain(kind)
    } else {
        class.gain()
    }
}
