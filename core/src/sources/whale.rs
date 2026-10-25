//! Humpback whale song (Megaptera novaeangliae), heard from far away.
//!
//! Biology (Payne & McVay 1971; Au et al. 2006; Garland et al. 2013): a song is a fixed
//! sequence of *themes*; a theme is one *phrase* repeated several times; a phrase is a short
//! sequence of *units*, the individual sounds. The song cycles for as long as the whale
//! sings, with breaks to breathe. Units are tonal: a voiced fundamental with harmonics,
//! shaped by resonances in the whale's air spaces. What changes inside a unit is the
//! pitch contour (a slow, smooth glide: a falling moan, a rising whoop, a short upsweep) —
//! there is no vibrato and no jitter, so neither has the model.
//!
//! Fitted to the reference recordings (docs/scenes/underwater.md, measured with
//! tools/analysis/underwater_measure.py on Stimpert et al. 2012 and Garland et al. 2013,
//! PLoS ONE): units 0.15–3.5 s (median ≈ 0.45 s), gaps median 0.3–1.6 s, fundamentals in
//! clusters at ≈ 70–135 Hz (grunts, low moans), 200–420 Hz (moans, wails) and 500–800 Hz
//! (cries), with harmonics up to ≈ 2.5 kHz; 50–65 % of the energy in 200–1000 Hz; the
//! steady units wander by under 15 cents around their contour.
//!
//! The model builds a song from a seed: 4–5 themes chosen from a library of phrase
//! templates (cries in a series, moan + whoop, upsweep chirps + moan, grunts + long wail,
//! low moans, a wail with a downsweep, downsweep cries), each phrase repeated 2–4 times.
//! Repeats vary by at most ±1 % in pitch and ±4 % in timing, as real ones do. A static
//! reverberation (multipath through the water column) and a distance low-pass finish it.

use serde::{Deserialize, Serialize};

use super::marine::{envelope, harmonic_weights, Contour, Ease, Harmonic, Lowpass, Reverb, CTRL, MAX_H};
use super::rain::check;
use super::{Controls, Generator};
use crate::rng::Rng;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct WhaleConfig {
    /// Linear output gain (0..64).
    pub gain: f32,
    /// Transposition of every unit (0.6..1.6): individuals and populations differ.
    pub pitch: f32,
    /// Harmonic richness (0..1): 0 = nearly pure tones, 1 = bright, buzzy units.
    pub brightness: f32,
    /// Share of the high units (cries, chirps, downsweeps) in the song (0..1).
    pub cries: f32,
    /// Low-pass of the distance, Hz (300..6000).
    pub tone_hz: f32,
    /// Reverberation level against the direct sound (0..1).
    pub reverb: f32,
    /// Reverberation time, s (0.3..6).
    pub t60_s: f32,
    /// Mean breathing pause after each pass through the song, s (0..180).
    pub pause_s: f32,
    /// Tempo of the song: gaps between units and phrases scale with 1/`pace` (0.5..2).
    pub pace: f32,
}

impl Default for WhaleConfig {
    fn default() -> Self {
        Self { gain: 1.0, pitch: 1.0, brightness: 0.6, cries: 0.6, tone_hz: 2800.0, reverb: 0.08, t60_s: 1.8, pause_s: 15.0, pace: 1.0 }
    }
}

impl WhaleConfig {
    pub fn validate(&self) -> Result<(), String> {
        check("gain", self.gain, 0.0, 64.0)?;
        check("pitch", self.pitch, 0.6, 1.6)?;
        check("brightness", self.brightness, 0.0, 1.0)?;
        check("cries", self.cries, 0.0, 1.0)?;
        check("tone_hz", self.tone_hz, 300.0, 6000.0)?;
        check("reverb", self.reverb, 0.0, 1.0)?;
        check("t60_s", self.t60_s, 0.3, 6.0)?;
        check("pause_s", self.pause_s, 0.0, 180.0)?;
        check("pace", self.pace, 0.5, 2.0)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Kind {
    /// 0.9–3.5 s, 90–250 Hz, rich harmonics, formant near 450 Hz.
    #[default]
    Moan,
    /// 0.25–3.5 s, 300–800 Hz, few harmonics.
    Cry,
    /// 0.4–0.9 s rising sweep from ~110 Hz.
    Whoop,
    /// 0.15–0.25 s upsweep ~370 → 650 Hz.
    Chirp,
    /// 0.3–0.6 s falling 135 → 75 Hz, breathy.
    Grunt,
}

#[derive(Clone, Copy, Debug, Default)]
struct Unit {
    kind: Kind,
    dur: f32,
    contour: Contour,
    level: f32,
    /// Silence after the unit, s.
    gap: f32,
}

const UNITS: usize = 8;
const THEMES: usize = 6;

#[derive(Clone, Copy, Debug, Default)]
struct Theme {
    units: [Unit; UNITS],
    n: usize,
    reps: u32,
    /// Silence between repeats of the phrase, s.
    phrase_gap: f32,
}

/// Unit parameters per kind: (attack s, release s, tilt at brightness 0 / 1, formant Hz,
/// width Hz, boost, highest partial Hz, breath).
fn voice_of(kind: Kind) -> (f32, f32, f32, f32, f32, f32, f32, f32, f32) {
    match kind {
        Kind::Moan => (0.16, 0.35, 1.8, 0.9, 520.0, 320.0, 4.0, 2800.0, 0.03),
        Kind::Cry => (0.06, 0.12, 2.0, 1.1, 1300.0, 500.0, 1.0, 3000.0, 0.0),
        Kind::Whoop => (0.08, 0.18, 1.8, 1.0, 600.0, 320.0, 2.5, 2800.0, 0.02),
        Kind::Chirp => (0.03, 0.06, 2.2, 1.3, 900.0, 400.0, 0.8, 2800.0, 0.0),
        Kind::Grunt => (0.035, 0.15, 1.6, 0.9, 380.0, 220.0, 3.0, 1800.0, 0.12),
    }
}

pub struct Whale {
    sample_rate: f32,
    rng: Rng,
    cfg: WhaleConfig,
    song: [Theme; THEMES],
    themes: usize,
    // Position in the song.
    theme: usize,
    rep: u32,
    unit: usize,
    /// Seconds into the current unit (or negative: waiting).
    t: f32,
    wait: f32,
    singing: bool,
    cur: Unit,
    osc: Harmonic,
    breath: super::common::Svf,
    breath_lp: Lowpass,
    lp: Lowpass,
    reverb: Reverb,
    ctrl: usize,
    f_now: f32,
    out_gain: super::common::Ramp,
}

impl Whale {
    pub const PRESETS: &'static [&'static str] = &["humpback", "humpback_distant", "humpback_low"];

    pub fn humpback() -> WhaleConfig {
        WhaleConfig::default()
    }
    /// Farther: darker, more of the water's reverberation, fewer high cries.
    pub fn humpback_distant() -> WhaleConfig {
        WhaleConfig { pitch: 0.8, tone_hz: 1600.0, reverb: 0.15, t60_s: 2.6, brightness: 0.45, cries: 0.45, ..Default::default() }
    }
    /// A deeper singer: moans, whoops and grunts, few cries.
    pub fn humpback_low() -> WhaleConfig {
        WhaleConfig { pitch: 0.85, brightness: 0.5, cries: 0.2, tone_hz: 2000.0, ..Default::default() }
    }

    pub fn preset(name: &str) -> Option<WhaleConfig> {
        Some(match name {
            "humpback" => Self::humpback(),
            "humpback_distant" => Self::humpback_distant(),
            "humpback_low" => Self::humpback_low(),
            _ => return None,
        })
    }

    pub fn new(sample_rate: f32, seed: u64, cfg: WhaleConfig) -> Self {
        let sr = sample_rate;
        let mut rng = Rng::new(seed ^ 0x5717_E5A1);
        let mut song = [Theme::default(); THEMES];
        let themes = compose(&mut rng, &cfg, &mut song);
        // The first unit starts after a short, seeded delay (whales are not in sync).
        let wait = rng.range(0.5, 6.0);
        Self {
            sample_rate,
            song,
            themes,
            theme: 0,
            rep: 0,
            unit: 0,
            t: 0.0,
            wait,
            singing: false,
            cur: Unit::default(),
            osc: Harmonic::default(),
            breath: super::common::Svf::new(300.0, 1.2, sr),
            breath_lp: Lowpass::new(1200.0, sr),
            lp: Lowpass::new(cfg.tone_hz, sr),
            reverb: Reverb::new(sr, 1.8, cfg.t60_s, (cfg.tone_hz * 0.7).max(400.0)),
            ctrl: 0,
            f_now: 0.0,
            out_gain: super::common::Ramp::new(cfg.gain),
            rng,
            cfg,
        }
    }

    pub fn config(&self) -> &WhaleConfig {
        &self.cfg
    }

    /// Seconds of one pass through the song (without the breathing pause).
    pub fn song_seconds(&self) -> f32 {
        let mut s = 0.0;
        for th in &self.song[..self.themes] {
            let phrase: f32 = th.units[..th.n].iter().map(|u| u.dur + u.gap).sum();
            s += th.reps as f32 * (phrase + th.phrase_gap);
        }
        s
    }

    /// Picks the next unit (with the small repeat-to-repeat variation) and its wait.
    fn advance(&mut self) {
        let th = self.song[self.theme];
        let mut u = th.units[self.unit];
        // Repeats are close but not identical: ±1 % pitch, ±4 % length.
        u.contour = u.contour.scaled(1.0 + 0.01 * self.rng.bipolar());
        u.dur *= 1.0 + 0.04 * self.rng.bipolar();
        self.cur = u;
        self.t = 0.0;
        self.singing = true;
        self.osc.reset();
        self.ctrl = 0;
    }

    /// After a unit: where the song goes next and how long to wait.
    fn next_position(&mut self) {
        let pace = self.cfg.pace.clamp(0.5, 2.0);
        let th = self.song[self.theme];
        let mut gap = self.cur.gap;
        self.unit += 1;
        if self.unit >= th.n {
            self.unit = 0;
            self.rep += 1;
            gap += th.phrase_gap;
            if self.rep >= th.reps {
                self.rep = 0;
                self.theme += 1;
                gap += self.rng.range(2.0, 4.5);
                if self.theme >= self.themes {
                    self.theme = 0;
                    gap += self.cfg.pause_s * self.rng.range(0.6, 1.4);
                }
            }
        }
        self.wait = gap / pace;
    }
}

/// A seeded song: themes from the template library, in a fixed order.
fn compose(rng: &mut Rng, cfg: &WhaleConfig, song: &mut [Theme; THEMES]) -> usize {
    // Templates 0..=6; the high ones (cries) are drawn with weight `cries`.
    const HIGH: [bool; 7] = [true, false, true, false, false, true, true];
    let n_themes = 4 + (rng.next_u32() % 2) as usize;
    let mut used = [false; 7];
    let mut n = 0;
    let mut guard = 0;
    while n < n_themes && guard < 200 {
        guard += 1;
        let k = (rng.next_u32() % 7) as usize;
        if used[k] {
            continue;
        }
        let w = if HIGH[k] { cfg.cries.clamp(0.0, 1.0) } else { 1.0 };
        if rng.uniform() > w.max(0.02) {
            continue;
        }
        used[k] = true;
        song[n] = template(k, rng, cfg.pitch);
        n += 1;
    }
    if n == 0 {
        song[0] = template(1, rng, cfg.pitch);
        n = 1;
    }
    n
}

fn unit(kind: Kind, dur: f32, hz: &[f32], ease: Ease, level: f32, gap: f32) -> Unit {
    Unit { kind, dur, contour: Contour::through(hz, ease), level, gap }
}

/// Phrase templates, each fitted to units measured in the references (Hz before `pitch`).
fn template(k: usize, rng: &mut Rng, pitch: f32) -> Theme {
    let mut th = Theme { reps: 2 + rng.next_u32() % 3, phrase_gap: rng.range(1.2, 2.6), ..Default::default() };
    let p = pitch * rng.range(0.94, 1.06);
    let mut push = |u: Unit| {
        if th.n < UNITS {
            let mut u = u;
            u.contour = u.contour.scaled(p);
            th.units[th.n] = u;
            th.n += 1;
        }
    };
    match k {
        // A series of short cries, ~0.5 s apart (Stimpert S1, 0–3 s: 0.23–0.35 s at 650–800 Hz).
        0 => {
            // Held at one pitch: a rising-falling arch on every cry, 0.5 s apart and bridged by
            // the reverberation, was a 2 Hz pitch waver.
            let f = rng.range(620.0, 740.0);
            let count = 3 + rng.next_u32() % 3;
            for _ in 0..count {
                push(unit(Kind::Cry, rng.range(0.26, 0.36), &[f], Ease::Smooth, 0.55, rng.range(0.18, 0.28)));
            }
        }
        // A long falling moan, then a whoop.
        1 => {
            let f = rng.range(160.0, 220.0);
            push(unit(Kind::Moan, rng.range(1.5, 2.6), &[f, f * 0.88], Ease::Linear, 0.9, rng.range(0.8, 1.5)));
            let w = rng.range(100.0, 125.0);
            push(unit(Kind::Whoop, rng.range(0.55, 0.9), &[w, w * 1.25, w * 2.6], Ease::In, 0.8, rng.range(1.2, 2.0)));
        }
        // Upsweep chirps ~0.9 s apart, then a falling moan (Garland S2: 0.15–0.2 s, 370 → 650 Hz;
        // moans 254 → 203 Hz, 0.8–1.4 s).
        2 => {
            let f = rng.range(350.0, 400.0);
            let count = 4 + rng.next_u32() % 3;
            for _ in 0..count {
                push(unit(Kind::Chirp, rng.range(0.17, 0.24), &[f, f * 1.75], Ease::In, 0.45, rng.range(0.6, 0.75)));
            }
            let m = rng.range(235.0, 260.0);
            push(unit(Kind::Moan, rng.range(0.9, 1.4), &[m, m * 0.9], Ease::Linear, 0.8, rng.range(0.8, 1.4)));
        }
        // Two grunts, then a long wail that turns up at the end (S1 22.5–26 s: 341 Hz,
        // rising to ~500 Hz in the last 0.8 s).
        3 => {
            let g = rng.range(120.0, 140.0);
            push(unit(Kind::Grunt, rng.range(0.35, 0.55), &[g, g * 0.78, g * 0.58], Ease::Out, 0.6, rng.range(0.5, 0.8)));
            push(unit(Kind::Grunt, rng.range(0.35, 0.55), &[g * 0.97, g * 0.75, g * 0.56], Ease::Out, 0.55, rng.range(0.9, 1.4)));
            let w = rng.range(320.0, 350.0);
            push(unit(Kind::Cry, rng.range(2.5, 3.4), &[w, w * 1.2], Ease::Linear, 0.85, rng.range(1.5, 2.2)));
        }
        // Low moans (S2: ~85–120 Hz fundamentals, 0.6–2 s).
        4 => {
            let f = rng.range(95.0, 120.0);
            push(unit(Kind::Moan, rng.range(1.2, 2.0), &[f, f * 0.93], Ease::Linear, 0.65, rng.range(0.9, 1.4)));
            push(unit(Kind::Moan, rng.range(1.0, 1.6), &[f * 0.95, f * 0.85], Ease::Linear, 0.6, rng.range(1.2, 2.0)));
        }
        // A held wail with a gentle rise (S1 37–46 s), then a downsweep.
        5 => {
            let f = rng.range(390.0, 440.0);
            push(unit(Kind::Cry, rng.range(1.3, 2.0), &[f, f * 1.04], Ease::Linear, 0.75, rng.range(0.5, 0.9)));
            let d = rng.range(950.0, 1150.0);
            push(unit(Kind::Cry, rng.range(0.5, 0.8), &[d, d * 0.65], Ease::Out, 0.55, rng.range(1.0, 1.6)));
        }
        // Downsweep cries (S1: 1100–1530 → 460–600 Hz in 0.45–1.1 s).
        _ => {
            let count = 2 + rng.next_u32() % 3;
            for _ in 0..count {
                let d = rng.range(950.0, 1250.0);
                push(unit(Kind::Cry, rng.range(0.5, 0.9), &[d, d * 0.75, d * 0.6], Ease::Out, 0.55, rng.range(0.6, 1.0)));
            }
        }
    }
    th
}

impl Generator for Whale {
    fn set_model(&mut self, cfg: &super::ModelConfig) {
        if let super::ModelConfig::Whale(c) = cfg {
            let sr = self.sample_rate;
            self.lp.set(c.tone_hz, sr);
            self.reverb.set(sr, c.t60_s, (c.tone_hz * 0.7).max(400.0));
            self.cfg = *c;
        }
    }

    fn render(&mut self, out: &mut [f32], c: &Controls) {
        let sr = self.sample_rate;
        let dt = 1.0 / sr;
        let cfg = self.cfg;
        self.out_gain.to(cfg.gain, out.len());
        let wet = cfg.reverb.clamp(0.0, 1.0);
        let dry = 1.0 - 0.5 * wet;
        for o in out.iter_mut() {
            if !self.singing {
                self.wait -= dt;
                // A held-back whale (activity 0) does not start a new unit; it never cuts one.
                if self.wait <= 0.0 && c.activity > 0.02 {
                    self.advance();
                }
            }
            let mut v = 0.0;
            if self.singing {
                let u = self.cur;
                let (att, rel, tilt0, tilt1, fz, wz, boost, top, breath) = voice_of(u.kind);
                if self.ctrl == 0 {
                    // Control update: contour position at the end of this interval.
                    let te = self.t + CTRL as f32 * dt;
                    let f0 = u.contour.at(te / u.dur);
                    self.f_now = f0;
                    let tilt = tilt0 + (tilt1 - tilt0) * cfg.brightness;
                    let env = envelope(te, u.dur, att, rel) * u.level;
                    let mut w = harmonic_weights(f0, tilt, fz * cfg.pitch.sqrt(), wz, boost, top, MAX_H);
                    for x in w.iter_mut() {
                        *x *= env;
                    }
                    self.osc.glide_to(f0, &w, CTRL, sr);
                    self.breath.set(2.5 * f0, 1.2, sr);
                }
                v = self.osc.tick();
                if breath > 0.0 {
                    let env = envelope(self.t, u.dur, att, rel) * u.level;
                    let (_, bp, _) = self.breath.process(self.rng.bipolar());
                    v += self.breath_lp.tick(bp) * breath * env;
                }
                self.ctrl += 1;
                if self.ctrl == CTRL {
                    self.ctrl = 0;
                    self.osc.settle();
                }
                self.t += dt;
                if self.t >= u.dur {
                    self.singing = false;
                    self.f_now = 0.0;
                    self.next_position();
                }
            }
            let d = self.lp.tick(v);
            let r = self.reverb.tick(d);
            *o = 0.3 * (dry * d + wet * 1.4 * r) * self.out_gain.next();
        }
        self.out_gain.finish();
        self.lp.flush();
        self.breath.flush();
        self.breath_lp.flush();
        self.reverb.flush();
    }

    fn pitch(&self) -> f32 {
        self.f_now
    }

    fn busy(&self) -> bool {
        self.singing
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn render(cfg: WhaleConfig, seed: u64, seconds: f32) -> Vec<f32> {
        let mut w = Whale::new(SR, seed, cfg);
        let c = Controls { activity: 1.0, ..Default::default() };
        let mut out = vec![0.0f32; (seconds * SR) as usize];
        for ch in out.chunks_mut(64) {
            w.render(ch, &c);
        }
        out
    }

    #[test]
    fn song_has_structure_and_sensible_length() {
        for seed in 0..20 {
            let w = Whale::new(SR, seed, WhaleConfig::default());
            assert!((3..=THEMES).contains(&w.themes) || w.themes >= 1);
            let s = w.song_seconds();
            assert!((30.0..400.0).contains(&s), "seed {seed}: song {s} s");
            for th in &w.song[..w.themes] {
                assert!(th.reps >= 2 && th.n >= 1);
                for u in &th.units[..th.n] {
                    assert!((0.15..3.6).contains(&u.dur), "unit {:?} {} s", u.kind, u.dur);
                    let top = u.contour.max_hz();
                    assert!((60.0..1700.0).contains(&top), "unit {:?} up to {top} Hz", u.kind);
                }
            }
        }
    }

    #[test]
    fn sings_smoothly_without_clicks() {
        let x = render(WhaleConfig { reverb: 0.0, ..Default::default() }, 3, 90.0);
        let rms = (x.iter().map(|v| (v * v) as f64).sum::<f64>() / x.len() as f64).sqrt() as f32;
        assert!(rms > 0.01 && rms < 1.0, "rms {rms}");
        // Largest step between samples against the signal's own scale: a tonal source at
        // ≤ 2.2 kHz low-passed cannot jump more than a fraction of its peak per sample.
        let peak = x.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        let step = x.windows(2).fold(0.0f32, |m, w| m.max((w[1] - w[0]).abs()));
        assert!(step < 0.35 * peak, "step {step} vs peak {peak}");
    }

    #[test]
    fn deterministic_per_seed_and_seeds_differ() {
        let a = render(WhaleConfig::default(), 11, 20.0);
        let b = render(WhaleConfig::default(), 11, 20.0);
        let c = render(WhaleConfig::default(), 12, 20.0);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }
}
