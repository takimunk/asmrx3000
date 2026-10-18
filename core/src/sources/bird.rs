//! Placeholder parametric songbird for M0 (M2 replaces the voice with per-species DDSP models).
//!
//! A phrase is a run of frequency-swept syllables that descend in pitch and end with a rising
//! flourish, loosely chaffinch-like. Sine plus a weak second harmonic, sin^2 envelopes,
//! optional trills.

use core::f32::consts::{PI, TAU};

use crate::rng::Rng;

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Idle,
    Gap,
    Syllable,
}

pub struct Bird {
    sample_rate: f32,
    rng: Rng,
    base: f32,
    tempo: f32,
    mode: Mode,
    timer: f32,
    phrase_len: u32,
    syllables_left: u32,
    t: f32,
    dur: f32,
    f0: f32,
    f1: f32,
    trill_rate: f32,
    trill_depth: f32,
    amp: f32,
    phase: f32,
    freq_now: f32,
}

impl Bird {
    /// `base` is the voice's characteristic pitch in Hz, `tempo` scales syllable speed.
    pub fn new(sample_rate: f32, seed: u64, base: f32, tempo: f32) -> Self {
        let mut rng = Rng::new(seed);
        let timer = sample_rate * rng.range(0.5, 3.0);
        Self {
            sample_rate,
            rng,
            base,
            tempo,
            mode: Mode::Idle,
            timer,
            phrase_len: 0,
            syllables_left: 0,
            t: 0.0,
            dur: 0.1,
            f0: base,
            f1: base,
            trill_rate: 0.0,
            trill_depth: 0.0,
            amp: 0.0,
            phase: 0.0,
            freq_now: 0.0,
        }
    }

    /// Instantaneous pitch while singing, 0 when silent.
    pub fn freq(&self) -> f32 {
        if self.mode == Mode::Syllable {
            self.freq_now
        } else {
            0.0
        }
    }

    fn start_phrase(&mut self) {
        self.phrase_len = 4 + self.rng.next_u32() % 7;
        self.syllables_left = self.phrase_len;
        self.start_syllable();
    }

    fn start_syllable(&mut self) {
        let k = (self.phrase_len - self.syllables_left) as f32;
        let progress = k / self.phrase_len.max(1) as f32;
        if self.syllables_left == 1 {
            // Closing flourish: longer, rising, slightly trilled.
            self.f0 = self.base * 0.9;
            self.f1 = self.base * 1.45;
            self.dur = 0.2 / self.tempo;
            self.trill_depth = 0.04;
            self.trill_rate = 32.0;
        } else {
            self.f0 = self.base * (1.25 - 0.4 * progress) * self.rng.range(0.96, 1.04);
            self.f1 = self.f0 * self.rng.range(0.7, 0.92);
            self.dur = self.rng.range(0.045, 0.1) / self.tempo;
            if self.rng.uniform() < 0.2 {
                self.trill_depth = 0.07;
                self.trill_rate = self.rng.range(22.0, 40.0);
            } else {
                self.trill_depth = 0.0;
            }
        }
        self.amp = self.rng.range(0.7, 1.0);
        self.t = 0.0;
        self.mode = Mode::Syllable;
    }

    fn next_interval(&mut self, activity: f32) -> f32 {
        let mean = 1.5 + 9.0 * (1.0 - activity.clamp(0.0, 1.0));
        mean * self.rng.range(0.5, 1.5)
    }

    /// `activity` (0..1) controls how often the bird sings.
    pub fn render(&mut self, out: &mut [f32], activity: f32) {
        let inv_sr = 1.0 / self.sample_rate;
        for o in out.iter_mut() {
            let mut s = 0.0;
            match self.mode {
                Mode::Idle => {
                    self.timer -= 1.0;
                    if self.timer <= 0.0 {
                        if activity > 0.02 {
                            self.start_phrase();
                        } else {
                            self.timer = self.sample_rate;
                        }
                    }
                }
                Mode::Gap => {
                    self.timer -= 1.0;
                    if self.timer <= 0.0 {
                        if self.syllables_left > 0 {
                            self.start_syllable();
                        } else {
                            self.mode = Mode::Idle;
                            self.timer = self.next_interval(activity) * self.sample_rate;
                        }
                    }
                }
                Mode::Syllable => {
                    let u = self.t / self.dur;
                    if u >= 1.0 {
                        self.syllables_left -= 1;
                        self.mode = Mode::Gap;
                        self.timer = self.rng.range(0.025, 0.07) / self.tempo * self.sample_rate;
                    } else {
                        let env = (PI * u).sin();
                        let env = env * env;
                        let mut f = self.f0 + (self.f1 - self.f0) * u;
                        if self.trill_depth > 0.0 {
                            f *= 1.0 + self.trill_depth * (TAU * self.trill_rate * self.t).sin();
                        }
                        self.phase += f * inv_sr;
                        if self.phase >= 1.0 {
                            self.phase -= 1.0;
                        }
                        let ph = TAU * self.phase;
                        s = self.amp * env * (ph.sin() + 0.12 * (2.0 * ph).sin());
                        self.freq_now = f;
                        self.t += inv_sr;
                    }
                }
            }
            *o = s * 0.3;
        }
    }
}
