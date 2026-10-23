//! The encounter director: what a scene meant for hours of focused work lets happen, how
//! often, and when it keeps quiet (the rules and their sources: docs/scenes/forest.md).
//!
//! Without it every animal keeps its own clock: residents sing whenever their daily curve
//! says so, each fly-by, circling bird and visitor comes back after its own `interval`, flocks
//! leave on a timer. Each is natural alone; together the default forest started ~40 songs a
//! minute with a bird somewhere in the air every half minute, which is a lively morning walk,
//! not a background to work in. Three layers turn that into a directed scene:
//! * **Song budget.** Residents still decide *who* wants to sing (daily curves, territories,
//!   counter-singing); a token bucket decides *how many* phrases start per minute. Its rate
//!   follows the hour (dawn chorus ~20/min, mid-morning ~9, midday ~5, evening ~9, night ~3)
//!   and the forest's slow pulse. A bird that wants to sing while the bucket is empty waits
//!   (`Controls::wait`, as for a singing neighbour), so phrases are spaced, never cut.
//! * **Pulse and lulls.** Activity wanders on three slow Ornstein–Uhlenbeck processes (2, 10
//!   and 30 min): together close to 1/f over minutes to half an hour, so there are livelier
//!   and quieter stretches without any period to recognise. Every 3–6 min a lull of 20–60 s
//!   lets only the water and the wind be heard.
//! * **Encounters.** Fly-bys, circling birds, visitors, flocks and gusts sweeping the crowns
//!   wait for a cue. The director draws the next one after a gap (about 1 to 4 min, shorter
//!   when the pulse is high, never during a lull), picks the kind by weight with novelty (a
//!   kind just used is unlikely again soon, never twice in a row) and lets the forest hush for
//!   a few seconds after a bird passes, as a real one does.
//!
//! Deterministic per scene seed; real-time safe (no allocation after construction).

use serde::{Deserialize, Serialize};

use super::Environment;
use crate::rng::Rng;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct EncountersDesc {
    /// Seconds between encounters, drawn uniformly, then shortened or stretched by the pulse.
    pub gap: [f32; 2],
    /// Seconds before the first encounter.
    pub first: [f32; 2],
    /// Seconds between lulls (birds silent, no encounters) and their length.
    pub lull_every: [f32; 2],
    pub lull: [f32; 2],
    /// Scale of the song budget (1 = the day curve above).
    pub songs: f32,
    /// Weight of gusts sweeping the crowns among the encounters (0 = none).
    pub gusts: f32,
}

impl Default for EncountersDesc {
    fn default() -> Self {
        Self { gap: [70.0, 230.0], first: [40.0, 90.0], lull_every: [180.0, 360.0], lull: [20.0, 60.0], songs: 1.0, gusts: 1.0 }
    }
}

/// What the director should make happen now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cue {
    /// Cue this agent's entrance (index into the director's agents).
    Performer(usize),
    Flock,
    Gust(u32),
}

/// Kinds of encounter, for novelty and telemetry.
pub const FLYBY: usize = 0;
pub const CIRCLE: usize = 1;
pub const VISITOR: usize = 2;
pub const FLOCK: usize = 3;
pub const GUST: usize = 4;
pub const KINDS: usize = 5;
pub const KIND_NAMES: [&str; KINDS] = ["fly-by", "circling", "visitor", "flock", "gust"];

/// Base weight of each kind (before novelty and availability).
const WEIGHT: [f32; KINDS] = [1.0, 0.6, 0.45, 0.8, 0.7];
/// Time constants of the pulse, s, and their weights.
const PULSE_TAU: [f32; 3] = [120.0, 600.0, 1800.0];
const PULSE_W: [f32; 3] = [0.5, 0.6, 0.6];
const PULSE_SIGMA: f32 = 0.35;
const PULSE_STEP: f32 = 0.5;
/// Song-bucket capacity: at most this many phrases in a burst after a quiet stretch.
const BUCKET: f32 = 1.5;

/// Phrases per minute at BIRDS 0.6 by hour of day: dawn chorus, a busy early morning, the
/// working day, a smaller evening peak, the night's few singers.
const SONG_CURVE: [(f32, f32); 14] = [
    (0.0, 3.0),
    (3.5, 3.0),
    (4.5, 14.0),
    (5.5, 22.0),
    (7.0, 16.0),
    (8.5, 10.0),
    (10.0, 8.0),
    (12.0, 5.0),
    (15.0, 5.0),
    (17.0, 7.0),
    (19.5, 10.0),
    (21.0, 8.0),
    (22.5, 4.0),
    (24.0, 3.0),
];

/// Songs per minute the budget allows at `hour` (BIRDS 0.6, no pulse).
pub fn songs_per_min(hour: f32) -> f32 {
    let h = hour.rem_euclid(24.0);
    for w in SONG_CURVE.windows(2) {
        let ((h0, a), (h1, b)) = (w[0], w[1]);
        if h <= h1 {
            return a + (b - a) * ((h - h0) / (h1 - h0)).clamp(0.0, 1.0);
        }
    }
    SONG_CURVE[0].1
}

pub struct Encounters {
    desc: EncountersDesc,
    rng: Rng,
    pulse_x: [f32; 3],
    pulse_t: f32,
    pulse: f32,
    tokens: f32,
    next: f32,
    lull_in: f32,
    lull_left: f32,
    since: [f32; KINDS],
    last: Option<usize>,
    pending: Option<(Cue, usize)>,
    /// From the direction: the wait for the next encounter runs this much faster.
    liveliness: f32,
    /// Encounters cued so far, per kind; phrases let through.
    pub counts: [u32; KINDS],
    pub songs: u32,
}

impl Encounters {
    pub fn new(desc: &EncountersDesc, seed: u64) -> Self {
        let mut rng = Rng::new(seed ^ 0xE7C0_0E75);
        let pulse_x = [rng.gauss() * 0.7, rng.gauss() * 0.7, rng.gauss() * 0.7];
        let next = rng.range(desc.first[0], desc.first[1].max(desc.first[0]));
        let lull_in = rng.range(desc.lull_every[0], desc.lull_every[1].max(desc.lull_every[0]));
        let mut e = Self {
            desc: desc.clone(),
            rng,
            pulse_x,
            pulse_t: 0.0,
            pulse: 1.0,
            tokens: 0.0,
            next,
            lull_in,
            lull_left: 0.0,
            since: [f32::INFINITY; KINDS],
            last: None,
            pending: None,
            liveliness: 1.0,
            counts: [0; KINDS],
            songs: 0,
        };
        e.pulse = e.pulse_value();
        e
    }

    fn pulse_value(&self) -> f32 {
        let norm = PULSE_W.iter().map(|w| w * w).sum::<f32>().sqrt();
        let x: f32 = self.pulse_x.iter().zip(PULSE_W).map(|(x, w)| x * w).sum::<f32>() / norm;
        (PULSE_SIGMA * x).exp().clamp(0.4, 1.9)
    }

    /// The forest's slow activity (mean ~1).
    pub fn pulse(&self) -> f32 {
        self.pulse
    }

    pub fn in_lull(&self) -> bool {
        self.lull_left > 0.0
    }

    /// Phrases per minute allowed right now.
    pub fn song_rate(&self, env: &Environment) -> f32 {
        if self.in_lull() {
            return 0.0;
        }
        songs_per_min(env.hour) * self.desc.songs * (env.birds / 0.6).clamp(0.0, 1.67) * self.pulse
    }

    /// The direction's liveliness (1 = as directed; 0.2 in a downpour .. ~1.5 after rain).
    pub fn set_liveliness(&mut self, l: f32) {
        self.liveliness = if l.is_finite() { l.clamp(0.1, 4.0) } else { 1.0 };
    }

    /// May a resident start a phrase now?
    pub fn gate_open(&self) -> bool {
        self.tokens >= 1.0 && !self.in_lull()
    }

    /// A resident started a phrase.
    pub fn song_started(&mut self) {
        self.tokens -= 1.0;
        self.songs += 1;
    }

    /// Advance by `dt`; `candidates` yields (agent index, kind) of the performers waiting for
    /// a cue and awake now; `gusts_ok` says whether the weather allows a gust (some wind, no
    /// storm). Returns the cue to execute, if any; report the outcome with [`Self::done`].
    pub fn step(&mut self, dt: f32, env: &Environment, candidates: impl Iterator<Item = (usize, usize)>, gusts_ok: bool) -> Option<Cue> {
        // Pulse: three OU processes, updated twice a second.
        self.pulse_t += dt;
        while self.pulse_t >= PULSE_STEP {
            self.pulse_t -= PULSE_STEP;
            for (x, tau) in self.pulse_x.iter_mut().zip(PULSE_TAU) {
                *x += -*x * PULSE_STEP / tau + (2.0 * PULSE_STEP / tau).sqrt() * self.rng.gauss();
            }
            self.pulse = self.pulse_value();
        }
        for s in self.since.iter_mut() {
            *s += dt;
        }
        // Lulls.
        if self.lull_left > 0.0 {
            self.lull_left -= dt;
            if self.lull_left <= 0.0 {
                // Birds come back one by one, not all at once.
                self.tokens = self.tokens.min(0.0);
                self.lull_in = self.range(self.desc.lull_every);
            }
        } else {
            self.lull_in -= dt;
            if self.lull_in <= 0.0 {
                self.lull_left = self.range(self.desc.lull);
                // Nothing big right after the silence either.
                self.next = self.next.max(self.lull_left + 15.0);
            }
        }
        let rate = self.song_rate(env) / 60.0;
        self.tokens = (self.tokens + rate * dt).min(BUCKET);

        self.next -= dt * self.liveliness;
        if self.next > 0.0 || self.in_lull() {
            return None;
        }
        // Choose: weight × novelty × availability.
        let mut best: Option<(Cue, usize)> = None;
        let mut total = 0.0f32;
        let gusts = self.desc.gusts;
        let mut consider = |cue: Cue, kind: usize, rng: &mut Rng, since: &[f32; KINDS], last: Option<usize>| {
            let novelty = 0.15 + 0.85 * crate::sources::common::smoothstep(0.0, 600.0, since[kind]);
            let w = if last == Some(kind) { 0.0 } else { WEIGHT[kind] * novelty };
            let w = if kind == GUST { w * gusts } else { w };
            if w <= 0.0 {
                return;
            }
            total += w;
            // Reservoir choice: each candidate wins with probability w / total so far.
            if rng.uniform() * total < w {
                best = Some((cue, kind));
            }
        };
        for (i, kind) in candidates {
            consider(Cue::Performer(i), kind, &mut self.rng, &self.since, self.last);
        }
        consider(Cue::Flock, FLOCK, &mut self.rng, &self.since, self.last);
        if gusts_ok {
            consider(Cue::Gust(0), GUST, &mut self.rng, &self.since, self.last);
        }
        let Some((cue, kind)) = best else {
            self.next = self.rng.range(10.0, 30.0);
            return None;
        };
        self.pending = Some((cue, kind));
        Some(match cue {
            Cue::Gust(_) => Cue::Gust(self.rng.next_u32()),
            c => c,
        })
    }

    /// Outcome of the last cue: `ok` = it happened (a flock may find no birds to lead).
    pub fn done(&mut self, ok: bool) {
        let Some((_, kind)) = self.pending.take() else { return };
        if !ok {
            self.next = self.rng.range(5.0, 15.0);
            return;
        }
        self.counts[kind] += 1;
        self.since[kind] = 0.0;
        self.last = Some(kind);
        // Next gap: livelier stretches bring encounters sooner. A visitor lingers for a minute
        // or two and comes close: leave more room after it.
        let gap = self.range(self.desc.gap) / self.pulse.powf(0.7);
        self.next = gap + if kind == VISITOR { 90.0 } else { 0.0 };
        // The forest hushes for a moment while a bird passes or a gust goes through.
        self.tokens = self.tokens.min(if kind == GUST { 0.0 } else { -1.0 });
    }

    fn range(&mut self, r: [f32; 2]) -> f32 {
        self.rng.range(r[0], r[1].max(r[0]))
    }
}
