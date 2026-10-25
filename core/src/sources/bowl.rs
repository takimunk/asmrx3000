//! Singing bowls and tingsha: modal synthesis of a struck or rubbed metal shell.
//!
//! **The bowl.** A Himalayan bowl is a thick, squat axisymmetric shell; it radiates from its
//! (n, 0) flexural modes (n nodal meridians, no nodal circles; Terwagne & Bush 2011,
//! arXiv:1106.6348). For a cylinder fixed at the base f(n,0) ∝ [(n²−1)²/(1+1/n²)]^½, which
//! gives 1 : 2.83 : 5.42 : 8.77 : 12.9; real bowls run a little flatter: 2.7–2.9, 4.8–5.7,
//! 7.5–9.1, 10.6–13.1 (Inácio, Henrique & Antunes 2006, as summarised by Gołaś & Filipek 2016).
//! Our recordings (docs/scenes/body_scan.md) average 1 : 2.79 : 5.22 : 8.2 : 11.7.
//!
//! * **Doublets.** No bowl is perfectly round, so every mode is a pair a few tenths of a hertz
//!   to a few hertz apart, oriented π/(2n) from each other. A strike excites the two unequally;
//!   their sum beats at the split, the slow "wah" that makes a bowl alive. Measured splits:
//!   0.6–2.1 Hz on the fundamental of 100–200 Hz bowls, partner 2–13 dB under the main mode.
//!   The partner here is never closer than 3 dB: equal partners cancel to silence once per beat.
//! * **Decay.** Each mode rings with its own T60, the fundamental longest (40–120 s in large
//!   bowls, 10–30 s in small ones) and the higher modes shorter, about T60 ∝ f^−0.75.
//! * **Strike.** A felt-covered mallet presses for a few milliseconds. Its force is modelled as
//!   a critically damped pulse t·e^(−t/τ) (two cascaded one-poles). Each mode takes it in its
//!   own rotating frame, so its amplitude rises smoothly along the pulse's integral (10→90 %
//!   in ≈ 3.4 τ) with no off-resonant forced response (no thump, no click), and ends exactly at
//!   its level. The modes' levels at the strike follow a tilt in dB per octave (felt: soft
//!   upper partials, the low-pass of a soft contact) and a gentle fourth-order roll-off
//!   (`tame_hz`).
//! * **Singing.** A leather-wrapped puja rubbed round the rim drives the (2,0) mode by
//!   stick-slip; it locks in with its own harmonics (2 f0 measured 29–31 dB under) and a
//!   little of the (3,0) mode (15–25 dB under). Here the sustained voice is a set of pure
//!   oscillators (no friction noise: rubbed bowls do not hiss) whose level breathes slowly;
//!   the fundamental keeps its doublet so the tone is never static.
//!
//! **Tingsha.** Two small thick cymbals on a cord, struck edge to edge. Each rings at its own
//! frequency (a pair measured at 2.86 and 2.93 kHz, with an upper mode at ×1.88, T60 7 s and
//! 3.5 s); the two beat against each other. Real tingsha sit at 2.5–6 kHz with a metallic
//! clash, the band the ear is most sensitive to. This one is tamed: a lower pair (≈2.1 kHz),
//! the second cymbal 1.6 Hz above and 4 dB under the first (a shimmer, not the 70 Hz
//! roughness of a real pair), a soft contact and the upper mode held down.
//!
//! Strikes are cued like clip events: a new `Controls::cue` value strikes (clips.rs sends one
//! per event: `first`, `interval`, `exact`). Everything is deterministic per seed, and the
//! render path does not allocate. Output is sound pressure at 1 m: `level_db` is the RMS of
//! the first two seconds of a strike at `strike` = 1 (or of the steady singing tone at
//! `sing` = 1), dB SPL, after the mix's class calibration.

use serde::{Deserialize, Serialize};

use super::common::Ramp;
use super::rain::check;
use super::{Controls, Generator, BLOCK};
use crate::rng::Rng;

/// What is struck.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BowlKind {
    /// A Himalayan singing bowl: (n, 0) modes 1 : 2.79 : 5.22 : 8.2 : 11.7, each a doublet.
    #[default]
    Bowl,
    /// A pair of tingsha cymbals: two detuned rings, each with an upper mode at ×1.88.
    Tingsha,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct BowlConfig {
    pub kind: BowlKind,
    /// Lowest mode, Hz (60..3000): the (2,0) mode of a bowl, a tingsha's main ring.
    pub f0: f32,
    /// Sound pressure at 1 m, dB SPL RMS (0..100): the first 2 s of a strike at `strike` = 1,
    /// or the singing tone at `sing` = 1.
    pub level_db: f32,
    /// Strike strength (0..2); 0 ignores the cues.
    pub strike: f32,
    /// Rubbed (singing) tone level (0..2); 0 = silent between strikes.
    pub sing: f32,
    /// T60 of the lowest mode, s (1..150).
    pub t60: f32,
    /// Higher modes ring shorter: T60 ∝ (f / f0)^−decay_slope (0..1.5).
    pub decay_slope: f32,
    /// Doublet split of the lowest mode, Hz (0..8); the higher ones split ∝ ratio^0.6.
    pub split_hz: f32,
    /// Level of each doublet's weaker partner under the main mode, dB (−30..−3).
    pub partner_db: f32,
    /// Strike spectrum: mode level per octave above f0, dB (−24..0; felt ≈ −6, metal ≈ 0).
    pub tilt_db: f32,
    /// Mallet contact time, ms (0.3..15): the strike's rise.
    pub mallet_ms: f32,
    /// Fourth-order roll-off of the mode levels above this, Hz (300..16000).
    pub tame_hz: f32,
    /// Tingsha: the second cymbal's tuning above the first, Hz (0..30).
    pub detune_hz: f32,
    /// Singing: depth of the slow level breath (0..0.5) and its period, s (3..60).
    pub breath: f32,
    pub breath_s: f32,
    /// Singing: the locked-in second harmonic, dB under the fundamental (−80..−10).
    pub harmonic_db: f32,
    /// Singing: the (3,0) mode, dB under the fundamental (−80..−6).
    pub upper_db: f32,
    /// Strike-to-strike variation of strength (0..0.5): ± share.
    pub jitter: f32,
}

impl Default for BowlConfig {
    fn default() -> Self {
        Bowl::medium()
    }
}

impl BowlConfig {
    pub fn validate(&self) -> Result<(), String> {
        check("f0", self.f0, 60.0, 3000.0)?;
        check("level_db", self.level_db, 0.0, 100.0)?;
        check("strike", self.strike, 0.0, 2.0)?;
        check("sing", self.sing, 0.0, 2.0)?;
        check("t60", self.t60, 1.0, 150.0)?;
        check("decay_slope", self.decay_slope, 0.0, 1.5)?;
        check("split_hz", self.split_hz, 0.0, 8.0)?;
        check("partner_db", self.partner_db, -30.0, -3.0)?;
        check("tilt_db", self.tilt_db, -24.0, 0.0)?;
        check("mallet_ms", self.mallet_ms, 0.3, 15.0)?;
        check("tame_hz", self.tame_hz, 300.0, 16000.0)?;
        check("detune_hz", self.detune_hz, 0.0, 30.0)?;
        check("breath", self.breath, 0.0, 0.5)?;
        check("breath_s", self.breath_s, 3.0, 60.0)?;
        check("harmonic_db", self.harmonic_db, -80.0, -10.0)?;
        check("upper_db", self.upper_db, -80.0, -6.0)?;
        check("jitter", self.jitter, 0.0, 0.5)
    }
}

/// Mode ratios of a bowl, (2,0)..(6,0): the mean of the recordings measured for the body scan
/// (docs/scenes/body_scan.md), inside the literature's ranges.
pub const BOWL_RATIOS: [f32; 5] = [1.0, 2.79, 5.22, 8.2, 11.7];
/// Tingsha: each cymbal's main ring and its upper mode (measured ×1.88).
pub const TINGSHA_RATIOS: [f32; 2] = [1.0, 1.88];

/// Damped modes: 5 bowl partials × 2 (doublet), or 2 cymbals × 2 partials × 2.
const MODES: usize = 10;

/// One damped mode: complex one-pole resonator z ← p·z + g·x, sounding Im(z). f64: a pole
/// within 1e-6 of the unit circle (T60 of a minute) must stay exact for minutes.
#[derive(Clone, Copy, Default)]
struct Mode {
    re: f64,
    im: f64,
    pr: f64,
    pi: f64,
    /// Hz and T60 (s); drive gain for the strike being played.
    f: f64,
    t60: f64,
    /// Level at the strike relative to the bowl's scale (before the doublet share).
    weight: f64,
    /// Which partial (index into the ratios) and whether it is the doublet's partner.
    partial: usize,
    partner: bool,
    drive: f64,
    /// The strike enters in the mode's own rotating frame (q = e^(jωk)), so each mode's
    /// amplitude rises along the integral of the force pulse: no forced, off-resonant response
    /// (driving the modes with the bare pulse gave a low thump 15 dB over the ring).
    qr: f64,
    qi: f64,
    /// Unit rotation per sample (cos ω, sin ω).
    cr: f64,
    ci: f64,
    on: bool,
}

/// A pure sustained oscillator (rotation, renormalised every block).
#[derive(Clone, Copy, Default)]
struct Osc {
    c: f64,
    s: f64,
    cr: f64,
    ci: f64,
    amp: f32,
}

impl Osc {
    fn new(f: f64, sr: f64, phase: f64, amp: f32) -> Self {
        let w = core::f64::consts::TAU * f / sr;
        Self { c: phase.cos(), s: phase.sin(), cr: w.cos(), ci: w.sin(), amp }
    }
    #[inline]
    fn tick(&mut self) -> f32 {
        let c = self.c * self.cr - self.s * self.ci;
        self.s = self.c * self.ci + self.s * self.cr;
        self.c = c;
        self.s as f32 * self.amp
    }
    fn renorm(&mut self) {
        let m = (self.c * self.c + self.s * self.s).sqrt();
        if m > 0.0 {
            self.c /= m;
            self.s /= m;
        }
    }
}

/// Singing oscillators: fundamental + partner, 2 f0, (3,0) + partner.
const OSCS: usize = 5;

pub struct Bowl {
    sr: f64,
    rng: Rng,
    cfg: BowlConfig,
    modes: [Mode; MODES],
    n_modes: usize,
    /// Strike force: two cascaded one-poles (t·e^(−t/τ)).
    s1: f64,
    s2: f64,
    a: f64,
    last_cue: u32,
    pending: bool,
    oscs: [Osc; OSCS],
    sing_scale: f32,
    sing_level: Ramp,
    /// Seconds since start (breath phase) and the breath's two phases.
    t: f64,
    breath_phase: [f64; 2],
}

impl Bowl {
    pub fn new(sample_rate: f32, seed: u64, cfg: BowlConfig) -> Self {
        let mut rng = Rng::new(seed);
        let breath_phase = [rng.range(0.0, core::f32::consts::TAU) as f64, rng.range(0.0, core::f32::consts::TAU) as f64];
        let mut b = Self {
            sr: sample_rate as f64,
            rng,
            cfg,
            modes: [Mode::default(); MODES],
            n_modes: 0,
            s1: 0.0,
            s2: 0.0,
            a: 0.0,
            last_cue: 0,
            pending: false,
            oscs: [Osc::default(); OSCS],
            sing_scale: 0.0,
            sing_level: Ramp::new(0.0),
            t: 0.0,
            breath_phase,
        };
        b.tune();
        b
    }

    pub fn config(&self) -> &BowlConfig {
        &self.cfg
    }

    /// A large bowl (≈ 25 cm): 110 Hz, a minute-long ring, slow beats.
    pub fn large() -> BowlConfig {
        BowlConfig { f0: 110.0, t60: 60.0, split_hz: 0.7, ..Self::medium() }
    }
    /// A medium bowl (≈ 18 cm): 220 Hz.
    pub fn medium() -> BowlConfig {
        BowlConfig {
            kind: BowlKind::Bowl,
            f0: 220.0,
            level_db: 50.0,
            strike: 1.0,
            sing: 0.0,
            t60: 40.0,
            decay_slope: 0.75,
            split_hz: 0.9,
            partner_db: -9.0,
            tilt_db: -6.0,
            mallet_ms: 4.0,
            tame_hz: 2500.0,
            detune_hz: 0.0,
            breath: 0.12,
            breath_s: 12.0,
            harmonic_db: -30.0,
            upper_db: -24.0,
            jitter: 0.12,
        }
    }
    /// A small bowl (≈ 13 cm): 330 Hz.
    pub fn small() -> BowlConfig {
        BowlConfig { f0: 330.0, t60: 30.0, split_hz: 1.2, ..Self::medium() }
    }
    /// A high, palm-sized bowl: 440 Hz.
    pub fn high() -> BowlConfig {
        BowlConfig { f0: 440.0, t60: 24.0, split_hz: 1.5, ..Self::medium() }
    }
    /// The rubbed bowl: a sustained, slowly breathing tone with a faint octave and no strike.
    /// The doublet beats once in ~12 s (a sway, not a pulse) with its partner 20 dB under
    /// (±0.9 dB), and the breath is ±6 %: together under ±1.5 dB, so a guide that travels
    /// round the head stays even (deeper sways read as the sound coming and going).
    pub fn singing() -> BowlConfig {
        BowlConfig { strike: 0.0, sing: 1.0, split_hz: 0.083, partner_db: -20.0, breath: 0.06, harmonic_db: -22.0, upper_db: -40.0, ..Self::medium() }
    }
    /// A tamed tingsha pair.
    pub fn tingsha() -> BowlConfig {
        BowlConfig {
            kind: BowlKind::Tingsha,
            f0: 2100.0,
            level_db: 40.0,
            t60: 6.0,
            decay_slope: 1.0,
            split_hz: 0.6,
            partner_db: -10.0,
            tilt_db: -15.0,
            mallet_ms: 2.0,
            tame_hz: 3000.0,
            detune_hz: 1.6,
            jitter: 0.1,
            ..Self::medium()
        }
    }

    pub fn preset(name: &str) -> Option<BowlConfig> {
        Some(match name {
            "large" => Self::large(),
            "medium" => Self::medium(),
            "small" => Self::small(),
            "high" => Self::high(),
            "singing" => Self::singing(),
            "tingsha" => Self::tingsha(),
            _ => return None,
        })
    }

    pub const PRESETS: &'static [&'static str] = &["large", "medium", "small", "high", "singing", "tingsha"];

    /// Mode frequencies, T60s and weights from the config (no allocation).
    fn tune(&mut self) {
        let c = self.cfg;
        let sr = self.sr;
        let nyq = 0.45 * sr;
        let f0 = c.f0 as f64;
        let mut n = 0;
        let mut add = |f: f64, partial: usize, partner: bool, weight: f64, modes: &mut [Mode; MODES]| {
            if n >= MODES || f >= nyq || f <= 0.0 {
                return;
            }
            let t60 = (c.t60 as f64 * (f / f0).powf(-(c.decay_slope as f64))).max(0.05);
            let r = (-6.907_755 / (t60 * sr)).exp();
            let w = core::f64::consts::TAU * f / sr;
            let m = &mut modes[n];
            // Keep a ringing mode's state when retuning (set_model glides nothing: a bowl
            // does not change pitch, so only parameters other than f0 change in practice).
            *m = Mode { pr: r * w.cos(), pi: r * w.sin(), cr: w.cos(), ci: w.sin(), f, t60, weight, partial, partner, drive: 0.0, ..*m };
            n += 1;
        };
        // Level of a partial at the strike: the tilt per octave and the fourth-order roll-off.
        let level = |f: f64| -> f64 {
            let oct = (f / f0).max(1.0).log2();
            let tilt = 10f64.powf(c.tilt_db as f64 * oct / 20.0);
            tilt / (1.0 + (f / c.tame_hz as f64).powi(4)).sqrt()
        };
        let split = c.split_hz as f64;
        match c.kind {
            BowlKind::Bowl => {
                for (k, &r) in BOWL_RATIOS.iter().enumerate() {
                    let f = f0 * r as f64;
                    let s = split * (r as f64).powf(0.6);
                    let w = level(f);
                    add(f, k, false, w, &mut self.modes);
                    add(f + s, k, true, w, &mut self.modes);
                }
            }
            BowlKind::Tingsha => {
                for cym in 0..2 {
                    let fc = f0 + cym as f64 * c.detune_hz as f64;
                    // The second cymbal quieter (they never ring evenly): the beat between them
                    // swings about 12 dB, a shimmer rather than a tremolo.
                    let g = if cym == 0 { 1.0 } else { 0.6 };
                    for (k, &r) in TINGSHA_RATIOS.iter().enumerate() {
                        let f = fc * r as f64;
                        let s = split * (r as f64).powf(0.6);
                        let w = g * level(f);
                        add(f, k, false, w, &mut self.modes);
                        add(f + s, k, true, w, &mut self.modes);
                    }
                }
            }
        }
        self.n_modes = n;
        for m in self.modes[n..].iter_mut() {
            *m = Mode::default();
        }
        // Strike force pulse: τ = half the contact time (the pulse peaks at τ and is mostly
        // over by 4τ).
        let tau = (c.mallet_ms as f64 * 1e-3 * 0.5).max(0.1e-3);
        self.a = (-1.0 / (tau * sr)).exp();
        self.tune_singing();
    }

    /// Mean square over the first 2 s of a strike with unit scale and these partner shares
    /// (per partial). The modes all start in phase, so close pairs (a doublet, the two cymbals)
    /// add coherently until they drift apart: mean of A_i A_j e^(−(b_i+b_j)t) cos(Δω t) / 2.
    fn strike_energy(&self, shares: &[f64; 5]) -> f64 {
        const T: f64 = 2.0;
        let modes = &self.modes[..self.n_modes];
        let amp = |m: &Mode| m.weight * if m.partner { shares[m.partial.min(4)] } else { 1.0 };
        let mut e = 0.0;
        for mi in modes {
            for mj in modes {
                let dw = core::f64::consts::TAU * (mi.f - mj.f);
                if dw.abs() > core::f64::consts::TAU * 40.0 {
                    continue;
                }
                let b = 6.907_755 / mi.t60 + 6.907_755 / mj.t60;
                // ∫0^T e^(−bt) cos(Δω t) dt
                let at_t = (-b * T).exp() * (-b * (dw * T).cos() + dw * (dw * T).sin());
                e += 0.5 * amp(mi) * amp(mj) * (at_t + b) / (b * b + dw * dw) / T;
            }
        }
        e
    }

    fn tune_singing(&mut self) {
        let c = self.cfg;
        let sr = self.sr;
        let f0 = c.f0 as f64;
        let k = 10f32.powf(c.partner_db / 20.0);
        let h = 10f32.powf(c.harmonic_db / 20.0);
        let u = 10f32.powf(c.upper_db / 20.0);
        let s = c.split_hz as f64;
        let r1 = BOWL_RATIOS[1] as f64;
        let spec = [(f0, 1.0), (f0 + s, k), (2.0 * f0, h), (f0 * r1, u), (f0 * r1 + s * r1.powf(0.6), u * k)];
        let mut e = 0.0f32;
        for (i, &(f, a)) in spec.iter().enumerate() {
            let a = if f < 0.45 * sr { a } else { 0.0 };
            let phase = self.oscs[i].s.atan2(self.oscs[i].c);
            let phase = if self.oscs[i].cr == 0.0 { 0.37 * i as f64 } else { phase };
            self.oscs[i] = Osc::new(f, sr, phase, a);
            e += 0.5 * a * a;
        }
        self.sing_scale = if e > 0.0 { pascals(c.level_db) / e.sqrt() } else { 0.0 };
    }

    /// Strikes now: each mode gets the share of the force pulse that brings it to its level.
    fn strike(&mut self) {
        let c = self.cfg;
        if c.strike <= 0.0 || self.n_modes == 0 {
            return;
        }
        let v = (1.0 + c.jitter as f64 * self.rng.bipolar() as f64).max(0.2);
        let a = self.a;
        let base_k = c.partner_db as f64;
        let mut partner_k = [0.0f64; 5];
        for p in partner_k.iter_mut() {
            // Each strike lands at its own angle to the doublets: the partner's share varies
            // by ±3 dB, never closer than 3 dB to the main mode.
            *p = 10f64.powf((base_k + 3.0 * self.rng.bipolar() as f64).min(-3.0) / 20.0);
        }
        let e = self.strike_energy(&partner_k);
        let scale = if e > 0.0 { pascals(c.level_db) as f64 / e.sqrt() } else { 0.0 } * c.strike as f64 * v;
        for m in self.modes[..self.n_modes].iter_mut() {
            let share = if m.partner { partner_k[m.partial.min(4)] } else { 1.0 };
            m.drive = scale * m.weight * share;
            // The injection phasor restarts at 0 rad: every mode's new part starts as a sine.
            m.qr = 1.0;
            m.qi = 0.0;
            m.on = true;
        }
        // The force cascade gets an impulse of (1 − a)²/a: its pulse then sums to 1.
        self.s1 += (1.0 - a) * (1.0 - a) / a;
    }

    fn render_block(&mut self, out: &mut [f32], c: &Controls) {
        let len = out.len();
        if c.cue != self.last_cue {
            self.last_cue = c.cue;
            self.pending = true;
        }
        if self.pending && c.activity > 0.02 {
            self.pending = false;
            self.strike();
        }
        let a = self.a;
        let pulse_on = self.s1 != 0.0 || self.s2 != 0.0;
        out.fill(0.0);
        let n = self.n_modes;
        let mut any = false;
        for m in self.modes[..n].iter_mut().filter(|m| m.on) {
            any = true;
            let (pr, pi) = (m.pr, m.pi);
            let (mut re, mut im) = (m.re, m.im);
            if pulse_on {
                // Force pulse (the same for every mode: recomputed per mode from the shared
                // state, cheaper than storing it), injected along the mode's rotation.
                let (cr, ci, g) = (m.cr, m.ci, m.drive);
                let (mut qr, mut qi) = (m.qr, m.qi);
                let (mut s1, mut s2) = (self.s1, self.s2);
                for o in out.iter_mut() {
                    s1 *= a;
                    s2 = s2 * a + s1;
                    let x = g * s2;
                    let r = re * pr - im * pi + x * qr;
                    im = re * pi + im * pr + x * qi;
                    re = r;
                    let q = qr * cr - qi * ci;
                    qi = qr * ci + qi * cr;
                    qr = q;
                    *o += im as f32;
                }
                m.qr = qr;
                m.qi = qi;
            } else {
                for o in out.iter_mut() {
                    let r = re * pr - im * pi;
                    im = re * pi + im * pr;
                    re = r;
                    *o += im as f32;
                }
            }
            m.re = re;
            m.im = im;
            if !pulse_on && re * re + im * im < 1e-20 {
                *m = Mode { re: 0.0, im: 0.0, on: false, ..*m };
            }
        }
        if pulse_on {
            // Advance the shared pulse state once for the block.
            if any {
                for _ in 0..len {
                    self.s1 *= a;
                    self.s2 = self.s2 * a + self.s1;
                }
            } else {
                self.s1 = 0.0;
                self.s2 = 0.0;
            }
            // The cascade's state is the force before scaling: flush when it is over.
            if self.s1.abs() < 1e-14 && self.s2.abs() < 1e-14 {
                self.s1 = 0.0;
                self.s2 = 0.0;
            }
        }

        // Singing: oscillators at a slowly breathing level.
        let dt = len as f64 / self.sr;
        self.t += dt;
        if self.cfg.sing > 0.0 || self.sing_level.cur > 0.0 {
            let bs = self.cfg.breath_s as f64;
            let tau = core::f64::consts::TAU;
            let b = 0.7 * (tau * self.t / bs + self.breath_phase[0]).sin() + 0.3 * (tau * self.t / (bs * 1.618) + self.breath_phase[1]).sin();
            // Swell in over the first 3 s (raised cosine), then breathe.
            let swell = if self.t < 3.0 { 0.5 - 0.5 * (core::f64::consts::PI * self.t / 3.0).cos() } else { 1.0 };
            let target = self.cfg.sing * self.sing_scale * ((1.0 + self.cfg.breath as f64 * b) * swell) as f32;
            self.sing_level.to(target, len);
            for o in out.iter_mut() {
                let g = self.sing_level.next();
                let mut s = 0.0;
                for osc in self.oscs.iter_mut() {
                    s += osc.tick();
                }
                *o += s * g;
            }
            self.sing_level.finish();
            for osc in self.oscs.iter_mut() {
                osc.renorm();
            }
        }
        for o in out.iter_mut() {
            if o.abs() < 1e-30 {
                *o = 0.0;
            }
        }
    }
}

/// Pascals at 1 m for `db` dB SPL, before the mix's class calibration (which multiplies every
/// emitter of the class, so it is undone here).
fn pascals(db: f32) -> f32 {
    use crate::mix::SoundClass;
    10f32.powf((db - crate::mix::calibration::PA_DB) / 20.0) / SoundClass::of_model(super::ModelKind::Bowl).gain()
}

impl Generator for Bowl {
    fn render(&mut self, out: &mut [f32], c: &Controls) {
        for chunk in out.chunks_mut(BLOCK) {
            self.render_block(chunk, c);
        }
    }

    fn set_model(&mut self, cfg: &super::ModelConfig) {
        if let super::ModelConfig::Bowl(c) = cfg {
            if *c != self.cfg {
                self.cfg = *c;
                self.tune();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn run(b: &mut Bowl, seconds: f32, cue_at: &[f32]) -> Vec<f32> {
        let n = (seconds * SR) as usize;
        let mut out = vec![0.0f32; n];
        let mut c = Controls { activity: 1.0, ..Default::default() };
        for (i, chunk) in out.chunks_mut(BLOCK).enumerate() {
            let t = (i * BLOCK) as f32 / SR;
            for &at in cue_at {
                if t >= at && t < at + BLOCK as f32 / SR {
                    c.cue = c.cue.wrapping_add(1);
                }
            }
            b.render(chunk, &c);
        }
        out
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / x.len().max(1) as f64).sqrt() as f32
    }

    fn db(x: f32) -> f32 {
        20.0 * x.max(1e-20).log10()
    }

    #[test]
    fn every_preset_is_finite_and_deterministic() {
        for p in Bowl::PRESETS {
            let cfg = Bowl::preset(p).unwrap();
            cfg.validate().unwrap();
            let a = run(&mut Bowl::new(SR, 9, cfg), 3.0, &[0.1, 1.5]);
            let b = run(&mut Bowl::new(SR, 9, cfg), 3.0, &[0.1, 1.5]);
            assert_eq!(a, b, "{p} not deterministic");
            assert!(a.iter().all(|v| v.is_finite() && (*v == 0.0 || v.abs() >= f32::MIN_POSITIVE)), "{p}");
            assert!(rms(&a) > 1e-5, "{p} silent");
        }
    }

    #[test]
    fn strike_level_matches_level_db() {
        for p in ["large", "medium", "small", "high", "tingsha"] {
            let cfg = BowlConfig { jitter: 0.0, ..Bowl::preset(p).unwrap() };
            let x = run(&mut Bowl::new(SR, 3, cfg), 2.5, &[0.0]);
            let got = db(rms(&x[..(2.0 * SR) as usize]));
            let want = db(pascals(cfg.level_db));
            assert!((got - want).abs() < 2.0, "{p}: {got:.1} dB vs {want:.1} dB");
        }
        let cfg = Bowl::singing();
        let x = run(&mut Bowl::new(SR, 3, cfg), 20.0, &[]);
        let got = db(rms(&x[(4.0 * SR) as usize..]));
        let want = db(pascals(cfg.level_db));
        assert!((got - want).abs() < 1.5, "singing: {got:.1} vs {want:.1}");
    }

    #[test]
    fn silent_until_struck_and_cues_strike() {
        let mut b = Bowl::new(SR, 1, Bowl::medium());
        let x = run(&mut b, 1.0, &[]);
        assert!(x.iter().all(|v| *v == 0.0), "no cue, no sound");
        let mut b = Bowl::new(SR, 1, Bowl::medium());
        let x = run(&mut b, 1.0, &[0.5]);
        let i = (0.5 * SR) as usize;
        assert!(x[..i - BLOCK].iter().all(|v| *v == 0.0));
        assert!(rms(&x[i..]) > 0.0);
        // Activity 0 holds the strike until the clip sounds.
        let mut b = Bowl::new(SR, 1, Bowl::medium());
        let mut out = [0.0f32; BLOCK];
        b.render(&mut out, &Controls { cue: 1, activity: 0.0, ..Default::default() });
        assert!(out.iter().all(|v| *v == 0.0));
        b.render(&mut out, &Controls { cue: 1, activity: 1.0, ..Default::default() });
        assert!(rms(&out) > 0.0);
    }

    #[test]
    fn modes_decay_at_their_t60_and_upper_modes_die_first() {
        let cfg = BowlConfig { jitter: 0.0, split_hz: 0.0, ..Bowl::large() };
        let x = run(&mut Bowl::new(SR, 5, cfg), 12.0, &[0.0]);
        // Broadband level falls; between 2 s and 10 s the fundamental dominates: about
        // 60 · 8 / T60 dB.
        let a = db(rms(&x[(1.5 * SR) as usize..(2.5 * SR) as usize]));
        let b = db(rms(&x[(9.5 * SR) as usize..(10.5 * SR) as usize]));
        let want = 60.0 * 8.0 / cfg.t60;
        assert!((a - b) > 0.6 * want && (a - b) < 2.0 * want, "{a:.1} -> {b:.1} dB, want ≈ {want:.1}");
        let b2 = Bowl::new(SR, 5, cfg);
        let m = &b2.modes[..b2.n_modes];
        assert!(m.windows(2).all(|w| w[1].f >= w[0].f && w[1].t60 <= w[0].t60 + 1e-9));
    }

    #[test]
    fn partials_follow_the_bowl_series() {
        // The main mode of each doublet sits exactly on the series, its partner just above.
        let cfg = Bowl::medium();
        let b = Bowl::new(SR, 1, cfg);
        assert_eq!(b.modes[0].f, cfg.f0 as f64);
        for (k, r) in BOWL_RATIOS.iter().enumerate() {
            let (main, partner) = (b.modes[2 * k], b.modes[2 * k + 1]);
            assert!((main.f / cfg.f0 as f64 - *r as f64).abs() < 1e-4, "partial {k}: {}", main.f / cfg.f0 as f64);
            assert!(partner.partner && partner.f > main.f && (partner.f - main.f - cfg.split_hz as f64 * (*r as f64).powf(0.6)).abs() < 1e-6);
        }
    }

    /// The largest sample-to-sample step relative to the signal's own slope at its loudest:
    /// a click shows as a step far above what the sinusoids allow.
    fn worst_step(x: &[f32], f_max: f32) -> f32 {
        let peak = x.iter().fold(0.0f32, |m, v| m.max(v.abs())).max(1e-12);
        let allowed = peak * core::f32::consts::TAU * f_max / SR;
        x.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0, f32::max) / allowed
    }

    #[test]
    fn no_clicks_on_strikes_restrikes_and_singing() {
        // A strike, a second strike while it rings, and the singing swell: no step bigger
        // than the highest mode's own slope at the peak level.
        for p in ["large", "medium", "high", "tingsha"] {
            let cfg = Bowl::preset(p).unwrap();
            let x = run(&mut Bowl::new(SR, 2, cfg), 4.0, &[0.2, 0.2 + 1.333, 3.0]);
            let fmax = cfg.f0 * if cfg.kind == BowlKind::Tingsha { 2.0 } else { 12.0 };
            assert!(worst_step(&x, fmax) <= 1.0, "{p}: {}", worst_step(&x, fmax));
            // The strike's first millisecond starts from zero (a soft contact, not a step).
            let i = x.iter().position(|v| *v != 0.0).unwrap();
            let peak = x.iter().fold(0.0f32, |m, v| m.max(v.abs()));
            assert!(x[i..i + 10].iter().all(|v| v.abs() < 0.05 * peak), "{p}: hard onset");
        }
        let x = run(&mut Bowl::new(SR, 2, Bowl::singing()), 6.0, &[]);
        assert!(worst_step(&x, 3.0 * 220.0) <= 1.0);
        assert!(x[0].abs() < 1e-6, "singing swells in from silence");
    }

    #[test]
    fn singing_is_steady_in_pitch_and_breathes_gently() {
        let cfg = Bowl::singing();
        let x = run(&mut Bowl::new(SR, 4, cfg), 30.0, &[]);
        // Pitch: zero crossings per second stay within 0.2 % of f0 (any vibrato or drift
        // would show here).
        let sec = SR as usize;
        for s in 4..29 {
            let w = &x[s * sec..(s + 1) * sec];
            let zc = w.windows(2).filter(|p| (p[0] <= 0.0) != (p[1] <= 0.0)).count() as f32 / 2.0;
            assert!((zc - cfg.f0).abs() <= 0.002 * cfg.f0 + 1.0, "second {s}: {zc} Hz");
        }
        // Level: 100 ms RMS stays within ±3 dB of the mean (a breath, never a pulse or a gap).
        let frames: Vec<f32> = x[4 * sec..].chunks(sec / 10).map(|c| db(rms(c))).collect();
        let mean = frames.iter().sum::<f32>() / frames.len() as f32;
        assert!(frames.iter().all(|v| (v - mean).abs() < 3.0), "{:?}", frames.iter().map(|v| v - mean).fold((0f32, 0f32), |(a, b), v| (a.min(v), b.max(v))));
    }

    #[test]
    fn tingsha_is_tamed_above_3_khz() {
        // Share of energy above 3 kHz in the first second after a strike.
        let cfg = BowlConfig { jitter: 0.0, ..Bowl::tingsha() };
        let x = run(&mut Bowl::new(SR, 8, cfg), 1.0, &[0.0]);
        let n = x.len();
        let (mut lo, mut hi) = (0.0f64, 0.0f64);
        for k in 1..n / 2 {
            let f = k as f32 * SR / n as f32;
            if !(1500.0..6000.0).contains(&f) {
                continue;
            }
            let (mut re, mut im) = (0.0f64, 0.0f64);
            // Coarse DFT on a few bins only (the test's own band check).
            if k % 8 != 0 {
                continue;
            }
            for (i, v) in x.iter().enumerate().step_by(2) {
                let p = core::f64::consts::TAU * k as f64 * i as f64 / n as f64;
                re += *v as f64 * p.cos();
                im += *v as f64 * p.sin();
            }
            let e = re * re + im * im;
            if f < 3000.0 {
                lo += e;
            } else {
                hi += e;
            }
        }
        assert!(hi < 0.1 * lo, "energy above 3 kHz {:.3} of below", hi / lo);
    }

    #[test]
    fn retuning_keeps_ringing() {
        let mut b = Bowl::new(SR, 1, Bowl::medium());
        let _ = run(&mut b, 0.5, &[0.0]);
        b.set_model(&super::super::ModelConfig::Bowl(BowlConfig { level_db: 44.0, ..Bowl::medium() }));
        let mut out = [0.0f32; BLOCK];
        b.render(&mut out, &Controls { activity: 1.0, cue: 1, ..Default::default() });
        assert!(rms(&out) > 0.0);
    }

    #[test]
    fn strike_has_no_thump() {
        // The first 100 ms (contact and rise) are never louder than the ring just after: the
        // strike enters each mode on resonance, with no forced low-frequency transient.
        for p in ["large", "medium", "high", "tingsha"] {
            // No doublets and no detune here: their beats would move the level by themselves.
            let cfg = BowlConfig { jitter: 0.0, tilt_db: 0.0, split_hz: 0.0, detune_hz: 0.0, ..Bowl::preset(p).unwrap() };
            let x = run(&mut Bowl::new(SR, 3, cfg), 0.3, &[0.0]);
            let (a, b) = (rms(&x[..4800]), rms(&x[4800..9600]));
            assert!(a < 1.2 * b, "{p}: {a:e} vs {b:e}");
        }
    }
}
