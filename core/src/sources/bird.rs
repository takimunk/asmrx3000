//! Procedural, species-parameterised bird voice (placeholder until the neural DDSP voices of
//! M2 land behind the same `Generator` interface).
//!
//! A phrase is built up front into a fixed array of syllables (frequency glide, duration,
//! gap, trill, amplitude) by a per-species pattern, then played back sample by sample:
//! sine + 2nd/3rd harmonic + optional breath noise under a smooth envelope. Flight calls
//! (`Controls::calling`) are short species-typical notes instead of full song.

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
    /// Long breathy hoot, pause, wavering second hoot (~600 Hz).
    TawnyOwl,
    /// "cu-coo" falling minor third, repeated (~700 Hz). Carries far.
    Cuckoo,
    /// Soft husky "coo-COO-coo, coo-coo" (~450 Hz).
    WoodPigeon,
    /// Fast twittering with buzzy trills (~4.5 kHz).
    Swallow,
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
    noise: f32,
    gain: f32,
    /// Mean seconds between song phrases at full activity.
    phrase_gap: f32,
}

fn timbre(style: VoiceStyle) -> Timbre {
    use VoiceStyle::*;
    let t = |h2, h3, noise, gain, phrase_gap| Timbre { h2, h3, noise, gain, phrase_gap };
    match style {
        Chaffinch => t(0.12, 0.0, 0.0, 0.3, 2.5),
        Robin => t(0.08, 0.0, 0.0, 0.25, 3.0),
        SongThrush => t(0.15, 0.04, 0.0, 0.3, 2.0),
        Nightingale => t(0.1, 0.03, 0.02, 0.35, 1.5),
        TawnyOwl => t(0.25, 0.08, 0.35, 0.5, 9.0),
        Cuckoo => t(0.12, 0.03, 0.08, 0.45, 6.0),
        WoodPigeon => t(0.35, 0.12, 0.25, 0.35, 5.0),
        Swallow => t(0.1, 0.0, 0.03, 0.3, 0.3),
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
    timer: f32,
    t: f32,
    phase: f32,
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
            t: 0.0,
            phase: 0.0,
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
            self.syl[self.n] = Syl { f0, f1, dur: dur / k, gap: gap / k, amp, trill_rate: 0.0, trill_depth: 0.0 };
            self.n += 1;
        }
    }

    fn trill_last(&mut self, rate: f32, depth: f32) {
        if self.n > 0 {
            self.syl[self.n - 1].trill_rate = rate;
            self.syl[self.n - 1].trill_depth = depth;
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
                let pause = self.rng.range(2.0, 3.5);
                self.push(b, b * 0.92, 0.7, pause, 1.0);
                self.push(b * 0.95, b * 0.9, 0.15, 0.25, 0.7);
                self.push(b * 1.02, b * 0.9, 1.0, 0.1, 1.0);
                self.trill_last(11.0, 0.03);
            }
            Cuckoo => {
                let pairs = 5 + self.rng.next_u32() % 8;
                for _ in 0..pairs {
                    self.push(b, b * 0.98, 0.14, 0.12, 1.0);
                    let gap = self.rng.range(0.5, 0.65);
                    self.push(b * 0.82, b * 0.8, 0.22, gap, 0.9);
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
            }
        }
    }

    /// Returns false if this species is silent in flight.
    fn build_call(&mut self) -> bool {
        use VoiceStyle::*;
        let b = self.base;
        self.n = 0;
        match self.style {
            Chaffinch => self.push(b * 1.25, b * 1.12, 0.04, 0.05, 1.0),
            Robin => {
                self.push(b * 1.5, b * 1.45, 0.015, 0.08, 0.8);
                self.push(b * 1.5, b * 1.45, 0.015, 0.05, 0.8);
            }
            SongThrush => self.push(b * 2.3, b * 2.1, 0.03, 0.05, 0.9),
            Nightingale => self.push(b, b * 1.5, 0.1, 0.05, 0.8),
            TawnyOwl => self.push(b * 2.2, b * 3.0, 0.25, 0.1, 1.0),
            Swallow => {
                self.push(b * 1.1, b * 0.9, 0.03, 0.05, 1.0);
                self.push(b * 1.15, b * 0.9, 0.03, 0.05, 1.0);
            }
            Cuckoo | WoodPigeon => return false,
        }
        true
    }

    fn start_syllable(&mut self) {
        self.t = 0.0;
        self.mode = Mode::Syllable;
    }

    fn rest(&mut self, c: &Controls) {
        let a = c.activity.clamp(0.02, 1.0);
        let secs = if c.calling {
            self.rng.range(0.2, 0.9) / (0.3 + a)
        } else {
            self.timbre.phrase_gap * (1.0 + 4.0 * (1.0 - a)) * self.rng.range(0.6, 1.4)
        };
        self.mode = Mode::Idle;
        self.timer = secs * self.sample_rate;
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
        if self.mode == Mode::Idle && c.activity <= 0.02 {
            // Nothing to say; keep the phrase timer from bursting out the moment activity rises.
            self.timer = self.timer.max(self.rng.range(0.2, 1.5) * self.sample_rate);
            out.fill(0.0);
            return;
        }
        let tb = &self.timbre;
        let (h2, h3, noise_amt, gain) = (tb.h2, tb.h3, tb.noise, tb.gain);
        for o in out.iter_mut() {
            let mut s = 0.0;
            match self.mode {
                Mode::Idle => {
                    self.timer -= 1.0;
                    if self.timer <= 0.0 {
                        let ok = if c.calling { self.build_call() } else {
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
                        if noise_amt > 0.0 {
                            if self.counter % NOISE_UPDATE == 0 {
                                self.noise_bp.set(f, 3.0, self.sample_rate);
                            }
                            let (_, bp, _) = self.noise_bp.process(self.rng.bipolar());
                            v = v * (1.0 - 0.5 * noise_amt) + bp * noise_amt * 3.0;
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

    fn pitch(&self) -> f32 {
        self.freq()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_style_sings_and_stays_bounded() {
        use VoiceStyle::*;
        for style in [Chaffinch, Robin, SongThrush, Nightingale, TawnyOwl, Cuckoo, WoodPigeon, Swallow] {
            let mut b = Bird::new(48_000.0, 7, style, Bird::default_pitch(style), 1.0);
            let c = Controls { activity: 1.0, ..Default::default() };
            let mut buf = [0.0f32; 64];
            let mut energy = 0.0f32;
            for _ in 0..(48_000 * 20 / 64) {
                b.render(&mut buf, &c);
                for v in buf {
                    assert!(v.is_finite() && v.abs() < 2.0);
                    energy += v * v;
                }
            }
            assert!(energy > 1.0, "{style:?} silent: {energy}");
        }
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
