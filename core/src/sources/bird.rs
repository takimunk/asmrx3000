//! Procedural, species-parameterised bird voice. Species with a trained neural voice use it
//! for song (director/voices.rs); this one sings for the rest and does every short call
//! (flight / contact calls, alarm calls) for all species.
//!
//! A phrase is built up front into a fixed array of syllables (frequency glide, duration,
//! gap, trill, amplitude, noisiness) by a per-species pattern, then played back sample by
//! sample: sine + harmonics 2–4 + optional band-limited breath noise under a smooth
//! envelope, with optional roughness (amplitude modulation: the raven's croak, the gull's
//! nasal edge). Flight calls (`Controls::calling`) are short species-typical notes; with
//! `Controls::alarm` they become the species' alarm (blackbird "chink" and rattle, robin
//! "tic", chaffinch "pink", …).
//!
//! Timing reacts at once to `activity`: the wait before the next song runs faster the
//! more eager the bird is, so a bird asked to answer a neighbour does so within a second or
//! two instead of finishing a long pause chosen earlier.

use core::f32::consts::TAU;

use super::{Controls, Generator};
use crate::dsp::Svf;
use crate::rng::Rng;

const MAX_SYL: usize = 48;
const NOISE_UPDATE: usize = 16;

/// Song pattern family. Each maps to a recognisable rhythm/pitch shape, not a recording.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VoiceStyle {
    /// Descending run of syllables ending in a rising flourish (~4 kHz).
    Chaffinch,
    /// Thin, wistful warble with large random pitch jumps (~5 kHz).
    Robin,
    /// Short motifs, each repeated 2-4 times (~3 kHz).
    SongThrush,
    /// Crescendo of "piu" whistles, then a fast trill (~2 kHz). Sings at night.
    Nightingale,
    /// Long breathy hoot, pause, tremolo, long wavering hoot (~600 Hz); females "ke-wick".
    TawnyOwl,
    /// "cu-coo" falling third, repeated, sometimes "cu-cu-coo" (~700 Hz). Carries far.
    Cuckoo,
    /// Soft husky "coo-COO-coo, coo-coo" ending in "cuk" (~450 Hz).
    WoodPigeon,
    /// Fast twittering with a creaky terminal rattle (~4.5 kHz).
    Swallow,
    /// Slow, mellow fluted phrases (~2 kHz) ending in a quiet squeaky twitter.
    Blackbird,
    /// Endless stream of fast trills, chirps and warbles (~3.4 kHz), sung in flight.
    Skylark,
    /// Nasal "aaow", then a laughing series of "kyow" (~1 kHz, harmonic-rich).
    Gull,
    /// Deep rough "kraa" croaks and hollow "klong" knocks (~550 Hz).
    Raven,
}

#[derive(Clone, Copy, Default)]
struct Syl {
    f0: f32,
    f1: f32,
    dur: f32,
    gap: f32,
    amp: f32,
    trill_rate: f32,
    trill_depth: f32,
    /// Extra breath noise for this syllable (clicks, "chuk"s), on top of the timbre's.
    noise: f32,
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Idle,
    Gap,
    Syllable,
}

struct Timbre {
    h2: f32,
    h3: f32,
    h4: f32,
    noise: f32,
    gain: f32,
    /// Mean seconds between song phrases at full activity.
    phrase_gap: f32,
    /// Roughness: amplitude modulation depth 0..1 at `rough_hz`.
    rough: f32,
    rough_hz: f32,
}

fn timbre(style: VoiceStyle) -> Timbre {
    use VoiceStyle::*;
    let t = |h2, h3, noise, gain, phrase_gap| Timbre { h2, h3, h4: 0.0, noise, gain, phrase_gap, rough: 0.0, rough_hz: 0.0 };
    match style {
        Chaffinch => t(0.12, 0.0, 0.0, 0.3, 2.5),
        Robin => t(0.08, 0.0, 0.0, 0.25, 3.0),
        SongThrush => t(0.15, 0.04, 0.0, 0.3, 2.0),
        Nightingale => t(0.1, 0.03, 0.02, 0.35, 1.5),
        TawnyOwl => t(0.25, 0.08, 0.35, 0.5, 9.0),
        Cuckoo => t(0.12, 0.03, 0.08, 0.45, 6.0),
        WoodPigeon => t(0.35, 0.12, 0.25, 0.35, 5.0),
        Swallow => t(0.1, 0.0, 0.03, 0.3, 0.3),
        Blackbird => t(0.16, 0.04, 0.01, 0.32, 3.5),
        Skylark => t(0.1, 0.02, 0.02, 0.25, 0.12),
        Gull => Timbre { h4: 0.35, rough: 0.15, rough_hz: 28.0, ..t(0.7, 0.5, 0.12, 0.3, 6.0) },
        Raven => Timbre { h4: 0.3, rough: 0.6, rough_hz: 55.0, ..t(0.6, 0.45, 0.35, 0.35, 5.0) },
    }
}

pub struct Bird {
    sample_rate: f32,
    rng: Rng,
    style: VoiceStyle,
    timbre: Timbre,
    base: f32,
    tempo: f32,
    syl: [Syl; MAX_SYL],
    n: usize,
    idx: usize,
    mode: Mode,
    /// Samples until the next phrase; for songs counted at "activity-scaled" speed.
    timer: f32,
    timer_scaled: bool,
    t: f32,
    phase: f32,
    rough_phase: f32,
    freq_now: f32,
    noise_bp: Svf,
    counter: usize,
}

impl Bird {
    /// `base` is this individual's characteristic pitch in Hz, `tempo` scales syllable speed.
    pub fn new(sample_rate: f32, seed: u64, style: VoiceStyle, base: f32, tempo: f32) -> Self {
        let mut rng = Rng::new(seed);
        let timer = sample_rate * rng.range(0.3, 2.0);
        Self {
            sample_rate,
            rng,
            style,
            timbre: timbre(style),
            base,
            tempo,
            syl: [Syl::default(); MAX_SYL],
            n: 0,
            idx: 0,
            mode: Mode::Idle,
            timer,
            timer_scaled: false,
            t: 0.0,
            phase: 0.0,
            rough_phase: 0.0,
            freq_now: 0.0,
            noise_bp: Svf::new(base, 4.0, sample_rate),
            counter: 0,
        }
    }

    /// Typical base pitch for a style (Hz).
    pub fn default_pitch(style: VoiceStyle) -> f32 {
        use VoiceStyle::*;
        match style {
            Chaffinch => 4200.0,
            Robin => 5000.0,
            SongThrush => 3000.0,
            Nightingale => 2200.0,
            TawnyOwl => 600.0,
            Cuckoo => 700.0,
            WoodPigeon => 450.0,
            Swallow => 4500.0,
            Blackbird => 2000.0,
            Skylark => 3400.0,
            Gull => 1000.0,
            Raven => 550.0,
        }
    }

    /// Instantaneous pitch while vocalising, 0 when silent.
    pub fn freq(&self) -> f32 {
        if self.mode == Mode::Syllable {
            self.freq_now
        } else {
            0.0
        }
    }

    fn push(&mut self, f0: f32, f1: f32, dur: f32, gap: f32, amp: f32) {
        if self.n < MAX_SYL {
            let k = self.tempo.max(0.1);
            self.syl[self.n] = Syl { f0, f1, dur: dur / k, gap: gap / k, amp, trill_rate: 0.0, trill_depth: 0.0, noise: 0.0 };
            self.n += 1;
        }
    }

    fn trill_last(&mut self, rate: f32, depth: f32) {
        if self.n > 0 {
            self.syl[self.n - 1].trill_rate = rate;
            self.syl[self.n - 1].trill_depth = depth;
        }
    }

    fn noise_last(&mut self, noise: f32) {
        if self.n > 0 {
            self.syl[self.n - 1].noise = noise;
        }
    }

    fn build_song(&mut self) {
        use VoiceStyle::*;
        let b = self.base;
        self.n = 0;
        match self.style {
            Chaffinch => {
                let len = 4 + self.rng.next_u32() % 7;
                for k in 0..len - 1 {
                    let p = k as f32 / len as f32;
                    let f0 = b * (1.25 - 0.4 * p) * self.rng.range(0.96, 1.04);
                    let f1 = f0 * self.rng.range(0.7, 0.92);
                    let (d, g, a) = (self.rng.range(0.045, 0.1), self.rng.range(0.025, 0.07), self.rng.range(0.7, 1.0));
                    self.push(f0, f1, d, g, a);
                    if self.rng.uniform() < 0.2 {
                        let r = self.rng.range(22.0, 40.0);
                        self.trill_last(r, 0.07);
                    }
                }
                self.push(b * 0.9, b * 1.45, 0.2, 0.05, 1.0);
                self.trill_last(32.0, 0.04);
            }
            Robin => {
                let len = 6 + self.rng.next_u32() % 9;
                for _ in 0..len {
                    let f0 = b * self.rng.range(0.55, 1.45);
                    let f1 = f0 * self.rng.range(0.7, 1.35);
                    let (d, g, a) = (self.rng.range(0.03, 0.16), self.rng.range(0.015, 0.06), self.rng.range(0.4, 1.0));
                    self.push(f0, f1, d, g, a);
                    if self.rng.uniform() < 0.25 {
                        let r = self.rng.range(30.0, 60.0);
                        self.trill_last(r, 0.05);
                    }
                }
            }
            SongThrush => {
                let motifs = 2 + self.rng.next_u32() % 3;
                for _ in 0..motifs {
                    let parts = 1 + self.rng.next_u32() % 3;
                    let reps = 2 + self.rng.next_u32() % 3;
                    let mut motif = [(0.0f32, 0.0f32, 0.0f32); 3];
                    for m in motif.iter_mut().take(parts as usize) {
                        let f0 = b * self.rng.range(0.6, 1.6);
                        *m = (f0, f0 * self.rng.range(0.6, 1.4), self.rng.range(0.04, 0.12));
                    }
                    for r in 0..reps {
                        for (i, m) in motif.iter().take(parts as usize).enumerate() {
                            let last = r == reps - 1 && i == parts as usize - 1;
                            let gap = if last { self.rng.range(0.25, 0.45) } else { 0.035 };
                            self.push(m.0, m.1, m.2, gap, 0.9);
                        }
                    }
                }
            }
            Nightingale => {
                let pius = 3 + self.rng.next_u32() % 4;
                for k in 0..pius {
                    let a = 0.3 + 0.7 * k as f32 / (pius - 1).max(1) as f32;
                    self.push(b * 1.3, b * 0.8, 0.18, 0.12, a);
                }
                let trill = 12 + self.rng.next_u32() % 17;
                let f0 = b * self.rng.range(1.4, 1.8);
                for _ in 0..trill {
                    self.push(f0, f0 * 0.75, 0.025, 0.012, 1.0);
                }
                if self.rng.uniform() < 0.5 {
                    for _ in 0..3 {
                        self.push(b * 0.55, b * 0.5, 0.05, 0.06, 0.8);
                    }
                }
            }
            TawnyOwl => {
                if self.rng.uniform() < 0.25 {
                    // Female contact call "ke-wick", a few times.
                    let k = 1 + self.rng.next_u32() % 3;
                    for _ in 0..k {
                        let gap = self.rng.range(0.6, 1.2);
                        self.push(b * 2.2, b * 3.0, 0.25, gap, 0.9);
                        self.noise_last(0.25);
                    }
                } else {
                    // Male: "hooo ... hu, hu-hu-hu-hoooooo" with a wavering end.
                    let pause = self.rng.range(2.0, 3.5);
                    self.push(b, b * 0.92, 0.7, pause, 1.0);
                    self.push(b * 0.95, b * 0.9, 0.15, 0.25, 0.7);
                    let stutter = 2 + self.rng.next_u32() % 3;
                    for _ in 0..stutter {
                        self.push(b * 1.0, b * 0.97, 0.07, 0.05, 0.75);
                    }
                    self.push(b * 1.02, b * 0.9, 1.0, 0.1, 1.0);
                    self.trill_last(11.0, 0.03);
                }
            }
            Cuckoo => {
                let pairs = 5 + self.rng.next_u32() % 8;
                let mut f = b;
                for _ in 0..pairs {
                    if self.rng.uniform() < 0.15 {
                        // "cu-cu-coo" stutter.
                        self.push(f, f * 0.98, 0.1, 0.08, 0.9);
                    }
                    self.push(f, f * 0.98, 0.14, 0.12, 1.0);
                    let gap = self.rng.range(0.5, 0.65);
                    self.push(f * 0.82, f * 0.8, 0.22, gap, 0.9);
                    f *= 0.997;
                }
            }
            WoodPigeon => {
                let phrases = 3 + self.rng.next_u32() % 3;
                for _ in 0..phrases {
                    self.push(b * 0.95, b, 0.25, 0.12, 0.7);
                    self.push(b * 1.05, b * 0.95, 0.45, 0.12, 1.0);
                    self.push(b, b * 0.95, 0.25, 0.25, 0.7);
                    self.push(b, b * 0.97, 0.25, 0.2, 0.75);
                    self.push(b, b * 0.95, 0.25, 0.6, 0.7);
                }
                // The phrase series ends with a short "cuk".
                self.push(b * 1.05, b * 0.9, 0.08, 0.1, 0.6);
            }
            Swallow => {
                let len = 10 + self.rng.next_u32() % 15;
                for _ in 0..len {
                    let f0 = b * self.rng.range(0.6, 1.6);
                    let f1 = f0 * self.rng.range(0.5, 1.6);
                    let (d, g) = (self.rng.range(0.015, 0.06), self.rng.range(0.01, 0.04));
                    let a = self.rng.range(0.5, 1.0);
                    self.push(f0, f1, d, g, a);
                    if self.rng.uniform() < 0.15 {
                        self.trill_last(80.0, 0.1);
                    }
                }
                if self.rng.uniform() < 0.6 {
                    // The creaky terminal "zerrrr".
                    self.push(b * 0.9, b * 0.8, 0.25, 0.05, 0.7);
                    self.trill_last(140.0, 0.25);
                    self.noise_last(0.2);
                }
            }
            Blackbird => {
                let notes = 3 + self.rng.next_u32() % 5;
                for _ in 0..notes {
                    let f0 = b * self.rng.range(0.8, 1.5);
                    let f1 = f0 * self.rng.range(0.85, 1.2);
                    let (d, g, a) = (self.rng.range(0.12, 0.3), self.rng.range(0.03, 0.08), self.rng.range(0.6, 1.0));
                    self.push(f0, f1, d, g, a);
                    if self.rng.uniform() < 0.3 {
                        self.trill_last(8.0, 0.02);
                    }
                }
                if self.rng.uniform() < 0.6 {
                    // Quiet, squeaky twitter at the end of the phrase.
                    let tw = 3 + self.rng.next_u32() % 5;
                    for _ in 0..tw {
                        let f0 = b * self.rng.range(2.6, 3.6);
                        let f1 = f0 * self.rng.range(0.7, 1.3);
                        let (d, a) = (self.rng.range(0.02, 0.05), self.rng.range(0.25, 0.4));
                        self.push(f0, f1, d, 0.015, a);
                    }
                }
            }
            Skylark => {
                let len = 28 + self.rng.next_u32() % 21;
                for _ in 0..len {
                    let kind = self.rng.uniform();
                    let gap = self.rng.range(0.006, 0.03);
                    if kind < 0.35 {
                        let f = b * self.rng.range(0.8, 1.6);
                        let d = self.rng.range(0.04, 0.09);
                        let (f1, a) = (f * self.rng.range(0.9, 1.1), self.rng.range(0.6, 1.0));
                        self.push(f, f1, d, gap, a);
                        let (r, dep) = (self.rng.range(50.0, 90.0), self.rng.range(0.05, 0.1));
                        self.trill_last(r, dep);
                    } else if kind < 0.65 {
                        let f0 = b * self.rng.range(0.7, 1.2);
                        let f1 = f0 * self.rng.range(1.3, 1.8);
                        let (d, a) = (self.rng.range(0.02, 0.04), self.rng.range(0.5, 0.9));
                        self.push(f0, f1, d, gap, a);
                    } else {
                        let f0 = b * self.rng.range(0.6, 1.8);
                        let f1 = f0 * self.rng.range(0.6, 1.5);
                        let (d, a) = (self.rng.range(0.025, 0.07), self.rng.range(0.4, 0.9));
                        self.push(f0, f1, d, gap, a);
                    }
                }
            }
            Gull => {
                let long = 1 + self.rng.next_u32() % 2;
                for _ in 0..long {
                    self.push(b * 1.05, b * 0.8, 0.35, 0.15, 0.9);
                }
                let kyow = 4 + self.rng.next_u32() % 6;
                let mut d = 0.14;
                let mut a = 1.0;
                for _ in 0..kyow {
                    self.push(b * 1.25, b * 0.85, d, 0.07, a);
                    d *= 0.93;
                    a *= 0.95;
                }
            }
            Raven => {
                let calls = 1 + self.rng.next_u32() % 4;
                for _ in 0..calls {
                    let gap = self.rng.range(0.4, 0.8);
                    if self.rng.uniform() < 0.7 {
                        let d = self.rng.range(0.22, 0.4);
                        self.push(b, b * 0.88, d, gap, 1.0);
                    } else {
                        self.push(b * 1.7, b * 1.5, 0.07, gap, 0.8);
                        self.noise_last(-0.3);
                    }
                }
            }
        }
    }

    /// Short call (or alarm). Returns false if this species is silent in that situation.
    fn build_call(&mut self, alarm: bool) -> bool {
        use VoiceStyle::*;
        let b = self.base;
        self.n = 0;
        match (self.style, alarm) {
            (Chaffinch, false) => self.push(b * 1.25, b * 1.12, 0.04, 0.05, 1.0),
            // "pink".
            (Chaffinch, true) => {
                self.push(b * 1.35, b * 1.28, 0.035, 0.05, 1.0);
                self.noise_last(0.1);
            }
            (Robin, false) => {
                self.push(b * 1.5, b * 1.45, 0.015, 0.08, 0.8);
                self.push(b * 1.5, b * 1.45, 0.015, 0.05, 0.8);
            }
            // Hard "tic-tic".
            (Robin, true) => {
                let k = 2 + self.rng.next_u32() % 2;
                for _ in 0..k {
                    self.push(b * 1.6, b * 1.5, 0.012, 0.09, 1.0);
                    self.noise_last(0.6);
                }
            }
            (SongThrush, false) => self.push(b * 2.3, b * 2.1, 0.03, 0.05, 0.9),
            // Dry "tchuk-tchuk".
            (SongThrush, true) => {
                let k = 2 + self.rng.next_u32() % 3;
                for _ in 0..k {
                    self.push(b * 1.3, b * 0.9, 0.025, 0.1, 1.0);
                    self.noise_last(0.4);
                }
            }
            (Nightingale, false) => self.push(b, b * 1.5, 0.1, 0.05, 0.8),
            // "hweet" + croaking "kerr".
            (Nightingale, true) => {
                self.push(b * 0.9, b * 1.4, 0.12, 0.1, 0.9);
                self.push(b * 0.5, b * 0.45, 0.1, 0.05, 0.8);
                self.noise_last(0.5);
            }
            (TawnyOwl, _) => self.push(b * 2.2, b * 3.0, 0.25, 0.1, 1.0),
            (Swallow, false) => {
                self.push(b * 1.1, b * 0.9, 0.03, 0.05, 1.0);
                self.push(b * 1.15, b * 0.9, 0.03, 0.05, 1.0);
            }
            // Sharp "flitt-flitt".
            (Swallow, true) => {
                self.push(b * 1.4, b * 1.1, 0.03, 0.06, 1.0);
                self.push(b * 1.4, b * 1.1, 0.03, 0.05, 1.0);
            }
            // Thin "tsii".
            (Blackbird, false) => self.push(b * 3.6, b * 3.4, 0.12, 0.05, 0.6),
            // "chink-chink-chink", or the panicked "pli-pli-pli" rattle.
            (Blackbird, true) => {
                if self.rng.uniform() < 0.35 {
                    let k = 8 + self.rng.next_u32() % 7;
                    for i in 0..k {
                        let f = b * (2.0 - 0.4 * i as f32 / k as f32);
                        self.push(f, f * 0.85, 0.03, 0.04, 1.0);
                    }
                } else {
                    let k = 3 + self.rng.next_u32() % 4;
                    for _ in 0..k {
                        self.push(b * 2.6, b * 2.4, 0.025, 0.09, 1.0);
                        self.noise_last(0.25);
                    }
                }
            }
            // "chirrup".
            (Skylark, _) => {
                self.push(b * 0.9, b * 1.3, 0.04, 0.02, 0.9);
                self.push(b * 1.3, b * 1.0, 0.05, 0.05, 0.9);
            }
            (Gull, false) => self.push(b * 1.3, b * 0.9, 0.15, 0.08, 1.0),
            // Laughing "ha-ha-ha".
            (Gull, true) => {
                let k = 4 + self.rng.next_u32() % 3;
                for _ in 0..k {
                    self.push(b * 1.1, b * 1.0, 0.08, 0.07, 1.0);
                }
            }
            (Raven, false) => self.push(b, b * 0.9, 0.3, 0.1, 1.0),
            (Raven, true) => {
                let k = 3 + self.rng.next_u32() % 3;
                for _ in 0..k {
                    self.push(b * 1.1, b * 0.95, 0.15, 0.1, 1.0);
                }
            }
            (Cuckoo | WoodPigeon, _) => return false,
        }
        true
    }

    fn start_syllable(&mut self) {
        self.t = 0.0;
        self.mode = Mode::Syllable;
    }

    fn rest(&mut self, c: &Controls) {
        let a = c.activity.clamp(0.02, 1.0);
        self.mode = Mode::Idle;
        if c.calling {
            let secs = if c.alarm { self.rng.range(0.25, 0.7) / (0.4 + a) } else { self.rng.range(0.2, 0.9) / (0.3 + a) };
            self.timer = secs * self.sample_rate;
            self.timer_scaled = false;
        } else {
            // Counted down at `1 / (1 + 4 (1 - activity))`: a more eager bird sings sooner.
            self.timer = self.timbre.phrase_gap * self.rng.range(0.6, 1.4) * self.sample_rate;
            self.timer_scaled = true;
        }
    }
}

#[inline]
fn smoothstep(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

impl Generator for Bird {
    fn render(&mut self, out: &mut [f32], c: &Controls) {
        let inv_sr = 1.0 / self.sample_rate;
        if self.mode == Mode::Idle && (c.activity <= 0.02 || (c.wait && !c.calling)) {
            // Held (nothing to say, or the mix's phrase permit is withheld): the pause keeps
            // running, but an expired one is re-drawn short, so the bird neither bursts out the
            // moment activity rises nor starves while permits go to others block after block.
            self.timer -= out.len() as f32;
            if self.timer <= 0.0 {
                self.timer = self.rng.range(0.1, 1.5) * self.sample_rate;
                self.timer_scaled = false;
            }
            out.fill(0.0);
            return;
        }
        let tb = &self.timbre;
        let (h2, h3, h4, noise_amt, gain) = (tb.h2, tb.h3, tb.h4, tb.noise, tb.gain);
        let (rough, rough_inc) = (tb.rough, tb.rough_hz * inv_sr);
        if c.reply && self.mode == Mode::Idle && !c.calling && self.timer_scaled {
            let soon = self.rng.range(0.3, 1.4) * self.sample_rate;
            self.timer = self.timer.min(soon);
        }
        let idle_dec = if self.timer_scaled { 1.0 / (1.0 + 4.0 * (1.0 - c.activity.clamp(0.0, 1.0))) } else { 1.0 };
        // A pause that outlasts the block: no sample would sound, so skip the loop.
        if self.mode == Mode::Idle && self.timer > idle_dec * out.len() as f32 {
            self.timer -= idle_dec * out.len() as f32;
            self.counter = self.counter.wrapping_add(out.len() as _);
            out.fill(0.0);
            return;
        }
        for o in out.iter_mut() {
            let mut s = 0.0;
            match self.mode {
                Mode::Idle => {
                    self.timer -= idle_dec;
                    if self.timer <= 0.0 {
                        let ok = if c.calling {
                            self.build_call(c.alarm)
                        } else {
                            self.build_song();
                            true
                        };
                        if ok && self.n > 0 && c.activity > 0.02 {
                            self.idx = 0;
                            self.start_syllable();
                        } else {
                            self.rest(c);
                        }
                    }
                }
                Mode::Gap => {
                    self.timer -= 1.0;
                    if self.timer <= 0.0 {
                        if self.idx < self.n {
                            self.start_syllable();
                        } else {
                            self.rest(c);
                        }
                    }
                }
                Mode::Syllable => {
                    let sy = self.syl[self.idx];
                    if self.t >= sy.dur {
                        self.idx += 1;
                        self.mode = Mode::Gap;
                        self.timer = sy.gap * self.sample_rate;
                    } else {
                        let u = self.t / sy.dur;
                        let att = (0.3 * sy.dur).min(0.08);
                        let env = smoothstep(self.t.min(sy.dur - self.t) / att);
                        let env = env * env;
                        let mut f = sy.f0 * (sy.f1 / sy.f0).powf(u);
                        if sy.trill_depth > 0.0 {
                            f *= 1.0 + sy.trill_depth * (TAU * sy.trill_rate * self.t).sin();
                        }
                        self.phase += f * inv_sr;
                        if self.phase >= 1.0 {
                            self.phase -= 1.0;
                        }
                        let ph = TAU * self.phase;
                        let mut v = ph.sin() + h2 * (2.0 * ph).sin() + h3 * (3.0 * ph).sin();
                        if h4 > 0.0 {
                            v += h4 * (4.0 * ph).sin();
                        }
                        let na = (noise_amt + sy.noise).clamp(0.0, 0.9);
                        if na > 0.0 {
                            if self.counter % NOISE_UPDATE == 0 {
                                self.noise_bp.set(f, 3.0, self.sample_rate);
                            }
                            let (_, bp, _) = self.noise_bp.process(self.rng.bipolar());
                            v = v * (1.0 - 0.5 * na) + bp * na * 3.0;
                        }
                        if rough > 0.0 {
                            self.rough_phase += rough_inc;
                            if self.rough_phase >= 1.0 {
                                self.rough_phase -= 1.0;
                            }
                            v *= 1.0 - rough * (0.5 + 0.5 * (TAU * self.rough_phase).sin());
                        }
                        s = sy.amp * env * v;
                        self.freq_now = f;
                        self.t += inv_sr;
                    }
                }
            }
            self.counter = self.counter.wrapping_add(1);
            *o = s * gain;
        }
    }

    fn busy(&self) -> bool {
        self.mode != Mode::Idle
    }

    fn pitch(&self) -> f32 {
        self.freq()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [VoiceStyle; 12] = {
        use VoiceStyle::*;
        [Chaffinch, Robin, SongThrush, Nightingale, TawnyOwl, Cuckoo, WoodPigeon, Swallow, Blackbird, Skylark, Gull, Raven]
    };

    fn energy(style: VoiceStyle, c: &Controls, secs: usize) -> f32 {
        let mut b = Bird::new(48_000.0, 7, style, Bird::default_pitch(style), 1.0);
        let mut buf = [0.0f32; 64];
        let mut energy = 0.0f32;
        for _ in 0..(48_000 * secs / 64) {
            b.render(&mut buf, c);
            for v in buf {
                assert!(v.is_finite() && v.abs() < 2.0, "{style:?} {v}");
                energy += v * v;
            }
        }
        energy
    }

    #[test]
    fn every_style_sings_and_stays_bounded() {
        for style in ALL {
            let e = energy(style, &Controls { activity: 1.0, ..Default::default() }, 20);
            assert!(e > 1.0, "{style:?} silent: {e}");
        }
    }

    #[test]
    fn alarm_calls() {
        use VoiceStyle::*;
        for style in [Chaffinch, Robin, SongThrush, Blackbird, Swallow, Gull, Raven, Skylark] {
            let c = Controls { activity: 0.8, calling: true, alarm: true, ..Default::default() };
            assert!(energy(style, &c, 5) > 0.05, "{style:?} has no alarm");
        }
        // Pigeons and cuckoos stay silent (the pigeon's alarm is its wing clatter).
        let c = Controls { activity: 0.8, calling: true, alarm: true, ..Default::default() };
        assert_eq!(energy(WoodPigeon, &c, 3), 0.0);
    }

    #[test]
    fn eager_bird_sings_sooner() {
        // Starting a song after a phrase: with activity 1 the next phrase comes ~5x sooner
        // than with activity 0.05.
        let starts = |a: f32| {
            let mut b = Bird::new(48_000.0, 3, VoiceStyle::Blackbird, 2000.0, 1.0);
            let c = Controls { activity: a, ..Default::default() };
            let mut buf = [0.0f32; 64];
            let (mut n, mut was) = (0, false);
            for _ in 0..(48_000 * 60 / 64) {
                b.render(&mut buf, &c);
                if b.busy() && !was {
                    n += 1;
                }
                was = b.busy();
            }
            n
        };
        assert!(starts(1.0) > 2 * starts(0.05), "{} vs {}", starts(1.0), starts(0.05));
    }

    #[test]
    fn silent_without_activity() {
        let mut b = Bird::new(48_000.0, 1, VoiceStyle::Robin, 5000.0, 1.0);
        let c = Controls::default();
        let mut buf = [1.0f32; 64];
        for _ in 0..2000 {
            b.render(&mut buf, &c);
            assert!(buf.iter().all(|v| *v == 0.0));
        }
    }
}
