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
pub mod loudness;
#[cfg(test)]
mod tests;

use core::sync::atomic::Ordering;
use std::sync::atomic::AtomicU32;

use analysis::{Analysis, Curve, DuckParams, HOP};
pub use calibration::{group, SoundClass};
use dynamics::{DynParams, Dynamics};
use loudness::{db_spl, BANDS};

use crate::controls::{id, ControlState};
use crate::engine::{SourceKind, BLOCK};
use crate::math::Vec3;
use crate::shaping::NF;

/// Emitter slots the mix keeps state for (scene emitters first; flight mode can use more).
pub const MAX_SLOTS: usize = 256;
pub const MAX_SOLOISTS: usize = 4;
/// Analysis tracks: one per group (bed), one per soloist, padded to pairs.
const TRACKS: usize = group::COUNT + MAX_SOLOISTS + 1;

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
}

impl Default for Intent {
    fn default() -> Self {
        Self { clarity: 0.6, density: 0.54, soloists: 2, target_lufs: -22.0, dynamics: 0.7, focus: [0.0; group::COUNT] }
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
        Self {
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
    /// In a phrase right now (`Generator::busy`).
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
    tilt: Curve,
    grid: [f64; NF],
    dynamics: Dynamics,
    soloist: [Option<usize>; MAX_SOLOISTS],
    time: f32,
    last_start: f32,
    phrases: usize,
    bed_db: f32,
    info: MixInfo,
    probes: Option<Vec<Probe>>,
    a_fast: f32,
    a_slow: f32,
    a_speed: f32,
    a_gain: f32,
}

/// Rough boost of the ground reflection on the listener power (+2 dB, incoherent).
const GROUND_POWER: f32 = 1.6;
/// Below this listener level nothing counts as sounding, dB SPL.
const QUIET_DB: f32 = 12.0;

impl Mix {
    pub fn new(sample_rate: f32) -> Self {
        let dt = BLOCK as f32 / sample_rate;
        let coef = |tau: f32| 1.0 - (-dt / tau).exp();
        let grid = crate::shaping::grid_freqs(sample_rate);
        // Distance cue of a demoted source: up to −4 dB towards 10 kHz (air, foliage).
        let mut tilt = [0.0f32; BANDS];
        for (z, t) in tilt.iter_mut().enumerate() {
            let f = loudness::centre(z);
            *t = -4.0 * ((f / 2000.0).log2() / 2.3).clamp(0.0, 1.0);
        }
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
            tilt,
            grid,
            dynamics: Dynamics::new(sample_rate),
            soloist: [None; MAX_SOLOISTS],
            time: 0.0,
            last_start: -10.0,
            phrases: 0,
            bed_db: 0.0,
            info: MixInfo::default(),
            probes: None,
            a_fast: coef(0.06),
            a_slow: coef(2.0),
            a_speed: coef(0.3),
            a_gain: coef(0.25),
        }
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
        for t in self.taps.iter_mut() {
            *t = [0.0; BLOCK];
        }
        for s in self.slots.iter_mut() {
            s.seen = false;
        }
        self.phrases = 0;
    }

    /// Before emitter `idx` renders: its gain, path EQ, send and phrase permit.
    pub fn voice(&mut self, idx: usize, v: &EmitterView) -> Voice<'_> {
        let Some(s) = self.slots.get_mut(idx) else {
            return Voice { gain: v.class.gain(), eq_db: &self.flat, send: 1.0, permit: true };
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
        let vel = (v.rel - s.rel).length() / dt;
        s.speed += (vel.min(60.0) - s.speed) * self.a_speed;
        s.rel = v.rel;
        s.dist = dist;
        s.wants = v.wants;
        s.flying = v.flying;
        if v.busy && !s.busy {
            self.last_start = self.time;
        }
        s.busy = v.busy;
        if v.busy {
            self.phrases += 1;
        }
        if self.bypass {
            s.gain_db = 0.0;
            return Voice { gain: legacy_gain(v.kind), eq_db: &self.flat, send: 1.0, permit: true };
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
                gain_db -= 2.0 + 4.0 * c;
                send = 1.0 + c;
                demoted = true;
            }
            Role::Bed => {}
        }
        if focus < 0.0 {
            send *= 1.0 - 0.5 * focus;
        }
        s.gain_db += (gain_db - s.gain_db) * self.a_gain;
        s.send += (send - s.send) * self.a_gain;
        let permit = s.permit || v.flying || matches!(s.role, Role::Soloist(_));
        let gain = s.cal * 10f32.powf(s.gain_db / 20.0);
        let eq = match s.role {
            Role::Soloist(_) => &self.flat,
            _ => &self.eq[s.group][demoted as usize],
        };
        Voice { gain, eq_db: eq, send: s.send, permit }
    }

    /// After emitter `idx` rendered `signal` (raw generator output, before `Voice::gain`).
    pub fn feed(&mut self, idx: usize, signal: &[f32], gain: f32) {
        let pb = if self.bypass { 1.0 / self.dynamics_playback_nominal() } else { 1.0 };
        let Some(s) = self.slots.get_mut(idx) else { return };
        let len = signal.len().min(BLOCK);
        // Listener estimate: 1/r spreading plus the ground reflection.
        let k = gain / s.dist;
        let k = k * pb;
        let energy: f32 = signal[..len].iter().map(|x| x * x).sum::<f32>() / len.max(1) as f32;
        let p = energy * k * k * GROUND_POWER;
        s.fast += (p - s.fast) * self.a_fast;
        s.slow += (p - s.slow) * self.a_slow;
        let track = match s.role {
            Role::Soloist(i) => group::COUNT + i,
            _ => s.group,
        };
        if energy > 0.0 {
            let tap = &mut self.taps[track];
            let k = k * GROUND_POWER.sqrt();
            for (t, x) in tap[..len].iter_mut().zip(signal) {
                *t += x * k;
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
            self.update_ducking();
            self.update_info();
        }
    }

    /// Output gains for the block (per sample, linear, includes the playback mapping).
    pub fn master(&mut self, l: &[f32], r: &[f32], out: &mut [f32]) {
        let p = DynParams { target_lufs: self.intent.target_lufs, strength: self.intent.dynamics };
        self.dynamics.process(l, r, &p, out);
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
        for s in self.slots.iter_mut().filter(|s| s.used) {
            let l_fast = db_spl(s.fast);
            let l_slow = db_spl(s.slow);
            let onset = ((l_fast - l_slow - 3.0) / 12.0).clamp(0.0, 1.0);
            s.novelty = (s.novelty * decay).max(onset);
            let prox = ((l_fast - bed_db + 6.0) / 18.0).clamp(0.0, 1.0);
            let motion = ((s.speed - 1.0) / 8.0).clamp(0.0, 1.0) * (1.5 - s.dist / 20.0).clamp(0.0, 1.0);
            let rarity = 0.7 * s.class.rarity() + 0.3 * (s.since_solo / 45.0).min(1.0);
            let focus = self.intent.focus[s.group];
            let sounding = l_fast > (bed_db - 10.0).max(QUIET_DB) && (s.busy || !matches!(s.class, SoundClass::Bird(_)));
            s.salience = if sounding { (0.35 * prox + 0.25 * s.novelty + 0.2 * motion + 0.2 * rarity + 0.3 * focus).clamp(0.0, 1.0) } else { 0.0 };
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
        let n = if self.intent.clarity > 0.0 && !self.bypass { self.intent.soloists } else { 0 };
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
        while granted < free.min(taken.len()) {
            let mut best: Option<(usize, f32)> = None;
            for (i, s) in self.slots.iter().enumerate() {
                if !s.used || !matches!(s.class, SoundClass::Bird(_)) || s.busy || !s.wants || taken[..granted].contains(&i) {
                    continue;
                }
                let prio = 1.0 / (1.0 + s.dist / 10.0) + s.class.rarity() + (s.waiting / 20.0).min(1.0) + 0.5 * self.intent.focus[s.group].max(0.0);
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
        for k in 0..MAX_SOLOISTS {
            if self.soloist[k].is_some() {
                fg[group::COUNT + k] = 1.0;
            }
        }
        // Bed level (A-weighted) for the salience reference.
        let mut bed_p = 0.0;
        for g in 0..group::COUNT {
            if bed[g] {
                bed_p += (0..BANDS).map(|z| a.fast[g][z] * a.model.a_w[z]).sum::<f32>();
            }
        }
        self.bed_db = db_spl(bed_p);

        let p = DuckParams { target_smr_db: 3.0 + 9.0 * c, depth_db: 1.0 + 7.0 * c };
        if c > 0.0 {
            analysis::duck_targets(a, &fg[..TRACKS], &bed[..TRACKS], &p, &mut self.duck_target[..TRACKS]);
        } else {
            for d in self.duck_target.iter_mut() {
                *d = [0.0; BANDS];
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
        let all: [bool; TRACKS] = core::array::from_fn(|t| a.fast[t].iter().any(|v| *v > 0.0));
        for t in 0..group::COUNT {
            let k: f32 = (0..BANDS).map(|z| a.slow[t][z] * a.model.k_w[z]).sum();
            info.group_lufs[t] = loudness::lufs(2.0 * k * g).max(-70.0);
            let mut others = all;
            others[t] = false;
            let (n, np) = a.partial(t, &others, Some(&self.duck[..TRACKS]));
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
        let bed: [bool; TRACKS] = core::array::from_fn(|t| t < group::COUNT && self.intent.focus[t] <= 0.25 && all[t]);
        for k in 0..MAX_SOLOISTS {
            info.soloist_source[k] = u32::MAX;
            info.soloist_salience[k] = 0.0;
            info.soloist_masking[k] = 0.0;
            let Some(i) = self.soloist[k] else { continue };
            let t = group::COUNT + k;
            let (n, before) = a.partial(t, &bed, None);
            let (_, after) = a.partial(t, &bed, Some(&self.duck[..TRACKS]));
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
        info.phrases = self.phrases as u32;
        info.max_phrases = self.intent.max_phrases().map_or(0, |m| m as u32);
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
