//! A small pod of dolphins at a polite distance: whistles and sparse echolocation clicks.
//!
//! Biology (Caldwell & Caldwell 1965; Janik & Sayigh 2013; Au 1993): bottlenose and common
//! dolphins whistle — frequency-modulated tones, fundamental ≈ 4–20 kHz, 0.2–1.5 s, with a
//! few weaker harmonics. Every animal has a *signature whistle*, its own contour (upsweep,
//! downsweep, arch, U, sine, often repeated as 2–3 loops with short gaps), plus variants.
//! Animals answer each other within a second. Echolocation clicks are broadband pulses of
//! tens of microseconds (energy to > 100 kHz; at 48 kHz they are heard as soft ticks centred
//! near 10 kHz) in trains whose inter-click interval glides smoothly (≈ 30–100 ms while
//! scanning, a few ms in a "buzz" when closing in on prey).
//!
//! Fitted to NOAA Passive Acoustics Group clips (common / striped / bottlenose dolphin) and
//! a Caribbean hydrophone recording (docs/scenes/underwater.md): whistle units 0.17–0.75 s
//! (median 0.26–0.5 s), dominant frequency median 9–12 kHz (3 kHz in the Caribbean take),
//! range 0.8–2 octaves per whistle; clicks centred at 9.5–14 kHz, intervals 30–100 ms.
//!
//! Comfort: whistles are the one sound in the scene above 3 kHz that is tonal, so they stay
//! short, sparse and soft; the default transposes them down by a fifth (≈ 3.5–9 kHz, still
//! within the species' range), a gentle low-pass takes the edge off the top and buzzes are
//! rare and quiet.

use serde::{Deserialize, Serialize};

use super::common::{Events, Ramp};
use super::marine::{envelope, Contour, Ease, Harmonic, OnePole, Reverb, CTRL, MAX_H};
use super::rain::check;
use super::{Controls, Generator};
use crate::rng::Rng;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct DolphinConfig {
    /// Linear output gain (0..64).
    pub gain: f32,
    /// Animals in the pod (1..4): each has its own signature whistle.
    pub pod: u32,
    /// Whistles per minute for the whole pod while it is vocal (0..60).
    pub whistles_per_min: f32,
    /// Transposition of every whistle (0.4..1.2; 1 = the measured 5–15 kHz).
    pub pitch: f32,
    /// Click level against the whistles (0..2; 0 = no clicks).
    pub clicks: f32,
    /// Click trains per minute (0..20).
    pub trains_per_min: f32,
    /// Share of trains that end in a buzz (0..1).
    pub buzz: f32,
    /// Low-pass of distance, Hz (2000..16000; one pole).
    pub tone_hz: f32,
    /// Reverberation (0..1) and its time, s (0.2..3).
    pub reverb: f32,
    pub t60_s: f32,
}

impl Default for DolphinConfig {
    fn default() -> Self {
        Self { gain: 1.0, pod: 3, whistles_per_min: 8.0, pitch: 0.68, clicks: 0.5, trains_per_min: 3.0, buzz: 0.1, tone_hz: 7000.0, reverb: 0.1, t60_s: 0.5 }
    }
}

impl DolphinConfig {
    pub fn validate(&self) -> Result<(), String> {
        check("gain", self.gain, 0.0, 64.0)?;
        check("pod", self.pod as f32, 1.0, 4.0)?;
        check("whistles_per_min", self.whistles_per_min, 0.0, 60.0)?;
        check("pitch", self.pitch, 0.4, 1.2)?;
        check("clicks", self.clicks, 0.0, 2.0)?;
        check("trains_per_min", self.trains_per_min, 0.0, 20.0)?;
        check("buzz", self.buzz, 0.0, 1.0)?;
        check("tone_hz", self.tone_hz, 2000.0, 16000.0)?;
        check("reverb", self.reverb, 0.0, 1.0)?;
        check("t60_s", self.t60_s, 0.2, 3.0)
    }
}

const PODMAX: usize = 4;

/// One animal's signature: contour, length, loops.
#[derive(Clone, Copy, Debug, Default)]
struct Signature {
    contour: Contour,
    dur: f32,
    loops: u32,
    loop_gap: f32,
}

#[derive(Clone, Copy, Default)]
struct Voice {
    on: bool,
    t: f32,
    dur: f32,
    contour: Contour,
    level: f32,
    /// Loops left after this one, and the gap before the next.
    loops_left: u32,
    loop_gap: f32,
    wait: f32,
    osc: Harmonic,
    ctrl: usize,
}

const CLICK_LEN: usize = 40;

/// A click train in progress.
#[derive(Clone, Copy, Default)]
struct Train {
    on: bool,
    left: u32,
    total: u32,
    ici: f32,
    /// ICI multiplier per click (smooth glide).
    ici_mul: f32,
    to_next: f32,
    level: f32,
    fc: f32,
    buzz: bool,
}

pub struct Dolphins {
    sample_rate: f32,
    rng: Rng,
    cfg: DolphinConfig,
    sigs: [Signature; PODMAX],
    voices: [Voice; PODMAX],
    ev_whistle: Events,
    ev_train: Events,
    /// Pending answer: animal, seconds.
    answer: Option<(usize, f32)>,
    train: Train,
    /// Click kernel (Gabor wavelet) rendered into a short ring of future samples.
    ring: [f32; 64],
    ring_pos: usize,
    lp: [OnePole; 2],
    reverb: Reverb,
    out_gain: Ramp,
}

impl Dolphins {
    pub const PRESETS: &'static [&'static str] = &["bottlenose_pod", "distant_pod", "whistles_only"];

    pub fn bottlenose_pod() -> DolphinConfig {
        DolphinConfig::default()
    }
    pub fn distant_pod() -> DolphinConfig {
        DolphinConfig { whistles_per_min: 6.0, tone_hz: 4500.0, reverb: 0.2, t60_s: 0.8, clicks: 0.35, ..Default::default() }
    }
    pub fn whistles_only() -> DolphinConfig {
        DolphinConfig { clicks: 0.0, trains_per_min: 0.0, ..Default::default() }
    }

    pub fn preset(name: &str) -> Option<DolphinConfig> {
        Some(match name {
            "bottlenose_pod" => Self::bottlenose_pod(),
            "distant_pod" => Self::distant_pod(),
            "whistles_only" => Self::whistles_only(),
            _ => return None,
        })
    }

    pub fn new(sample_rate: f32, seed: u64, cfg: DolphinConfig) -> Self {
        let sr = sample_rate;
        let mut rng = Rng::new(seed ^ 0xD01F_1A55);
        let mut sigs = [Signature::default(); PODMAX];
        for s in sigs.iter_mut() {
            *s = signature(&mut rng);
        }
        Self {
            sample_rate,
            sigs,
            voices: [Voice::default(); PODMAX],
            ev_whistle: Events::new(&mut rng),
            ev_train: Events::new(&mut rng),
            answer: None,
            train: Train::default(),
            ring: [0.0; 64],
            ring_pos: 0,
            lp: [OnePole::new(cfg.tone_hz, sr), OnePole::new(cfg.tone_hz * 1.4, sr)],
            reverb: Reverb::new(sr, 1.0, cfg.t60_s, 5000.0),
            out_gain: Ramp::new(cfg.gain),
            rng,
            cfg,
        }
    }

    pub fn config(&self) -> &DolphinConfig {
        &self.cfg
    }

    fn start_whistle(&mut self, animal: usize) {
        let pitch = self.cfg.pitch;
        let sig = self.sigs[animal];
        // Two thirds signature whistles, the rest variants (another contour family).
        let (contour, dur, loops, gap) = if self.rng.uniform() < 0.67 {
            (sig.contour, sig.dur, sig.loops, sig.loop_gap)
        } else {
            let v = signature(&mut self.rng);
            (v.contour, v.dur, 1, 0.0)
        };
        let v = &mut self.voices[animal];
        if v.on || v.loops_left > 0 {
            return;
        }
        v.on = true;
        v.t = 0.0;
        v.dur = dur * (1.0 + 0.03 * self.rng.bipolar());
        v.contour = contour.scaled(pitch * (1.0 + 0.01 * self.rng.bipolar()));
        v.level = 10f32.powf(self.rng.range(-4.0, 0.0) / 20.0);
        v.loops_left = loops.saturating_sub(1);
        v.loop_gap = gap;
        v.osc.reset();
        v.ctrl = 0;
        // Another animal may answer about a second after this whistle ends.
        let pod = self.cfg.pod.clamp(1, PODMAX as u32) as usize;
        if pod > 1 && self.answer.is_none() && self.rng.uniform() < 0.3 {
            let other = (animal + 1 + (self.rng.next_u32() as usize % (pod - 1))) % pod;
            let total = dur * loops as f32 + gap * loops.saturating_sub(1) as f32;
            self.answer = Some((other, total + self.rng.range(0.3, 1.2)));
        }
    }

    fn start_train(&mut self) {
        let buzz = self.rng.uniform() < self.cfg.buzz;
        let total = if buzz { 25 + self.rng.next_u32() % 25 } else { 12 + self.rng.next_u32() % 30 };
        let ici = if buzz { self.rng.range(0.006, 0.009) } else { self.rng.range(0.045, 0.09) };
        self.train = Train {
            on: true,
            left: total,
            total,
            ici,
            // The interval glides by ±25 % over the train (approach or turning away).
            ici_mul: (1.0 + self.rng.range(-0.25, 0.2)).powf(1.0 / total as f32),
            to_next: 0.0,
            level: if buzz { 0.35 } else { 1.0 } * 10f32.powf(self.rng.range(-5.0, 0.0) / 20.0),
            fc: self.rng.range(8500.0, 12500.0) * self.cfg.pitch.max(0.6).sqrt(),
            buzz,
        };
    }

    /// Adds one Gabor click (≈ 0.2 ms, centred at `fc`) into the ring of future samples.
    fn click(&mut self, amp: f32, fc: f32) {
        let sr = self.sample_rate;
        let sigma = 0.07e-3 * sr; // samples
        let w = core::f32::consts::TAU * fc.min(0.3 * sr) / sr;
        let mid = (CLICK_LEN / 2) as f32;
        let n = self.ring.len();
        for i in 0..CLICK_LEN {
            let x = i as f32 - mid;
            let g = (-0.5 * x * x / (sigma * sigma)).exp();
            self.ring[(self.ring_pos + i) % n] += amp * g * (w * x).cos();
        }
    }
}

/// A random whistle contour from the families described in the literature (Hz at pitch 1).
fn signature(rng: &mut Rng) -> Signature {
    let lo = rng.range(5000.0, 7500.0);
    let hi = lo * rng.range(1.6, 2.3);
    let (pts, ease, dur): ([f32; 5], Ease, f32) = match rng.next_u32() % 6 {
        0 => ([lo, hi, 0.0, 0.0, 0.0], Ease::In, rng.range(0.3, 0.65)),                    // upsweep
        1 => ([hi, lo, 0.0, 0.0, 0.0], Ease::Out, rng.range(0.25, 0.55)),                    // downsweep
        2 => ([lo, hi, lo * 1.15, 0.0, 0.0], Ease::Smooth, rng.range(0.35, 0.7)),           // arch
        3 => ([hi * 0.9, lo, hi, 0.0, 0.0], Ease::Smooth, rng.range(0.35, 0.7)),            // U
        4 => ([lo * 1.2, hi * 0.85, lo * 1.1, hi * 0.8, lo * 1.25], Ease::Smooth, rng.range(0.5, 0.8)), // sine
        _ => ([lo, lo * 1.15, hi, 0.0, 0.0], Ease::Smooth, rng.range(0.35, 0.65)),          // flat then rise
    };
    let n = pts.iter().take_while(|&&v| v > 0.0).count();
    let loops = 1 + rng.next_u32() % 2;
    Signature { contour: Contour::through(&pts[..n], ease), dur, loops, loop_gap: rng.range(0.12, 0.3) }
}

impl Generator for Dolphins {
    fn set_model(&mut self, cfg: &super::ModelConfig) {
        if let super::ModelConfig::Dolphins(c) = cfg {
            let sr = self.sample_rate;
            self.lp = [OnePole::new(c.tone_hz, sr), OnePole::new(c.tone_hz * 1.4, sr)];
            self.reverb.set(sr, c.t60_s, 5000.0);
            self.cfg = *c;
        }
    }

    fn render(&mut self, out: &mut [f32], c: &Controls) {
        let sr = self.sample_rate;
        let dt = 1.0 / sr;
        let cfg = self.cfg;
        let pod = cfg.pod.clamp(1, PODMAX as u32) as usize;
        let vocal = c.activity > 0.02;
        self.out_gain.to(cfg.gain, out.len());
        let wet = cfg.reverb.clamp(0.0, 1.0);
        for o in out.iter_mut() {
            if vocal {
                if self.ev_whistle.tick(cfg.whistles_per_min / 60.0 / sr, &mut self.rng) > 0 {
                    let a = self.rng.next_u32() as usize % pod;
                    self.start_whistle(a);
                }
                if let Some((a, t)) = self.answer {
                    if t <= 0.0 {
                        self.answer = None;
                        self.start_whistle(a);
                    } else {
                        self.answer = Some((a, t - dt));
                    }
                }
                if cfg.clicks > 0.0 && !self.train.on && self.ev_train.tick(cfg.trains_per_min / 60.0 / sr, &mut self.rng) > 0 {
                    self.start_train();
                }
            }
            // Whistles.
            let mut v = 0.0;
            for voice in self.voices.iter_mut() {
                if !voice.on {
                    if voice.loops_left > 0 {
                        voice.wait -= dt;
                        if voice.wait <= 0.0 {
                            voice.loops_left -= 1;
                            voice.on = true;
                            voice.t = 0.0;
                            voice.osc.reset();
                            voice.ctrl = 0;
                        }
                    }
                    continue;
                }
                if voice.ctrl == 0 {
                    let te = voice.t + CTRL as f32 * dt;
                    let f0 = voice.contour.at(te / voice.dur);
                    let env = envelope(te, voice.dur, 0.03, 0.05) * voice.level;
                    let mut w = [0.0f32; MAX_H];
                    // Fundamental and two weak harmonics (−18, −30 dB; the recordings show
                    // −10…−20 dB close up, distance takes the rest).
                    w[0] = env;
                    w[1] = 0.126 * env;
                    w[2] = 0.032 * env;
                    voice.osc.glide_to(f0, &w, CTRL, sr);
                }
                v += voice.osc.tick();
                voice.ctrl += 1;
                if voice.ctrl == CTRL {
                    voice.ctrl = 0;
                    voice.osc.settle();
                }
                voice.t += dt;
                if voice.t >= voice.dur {
                    voice.on = false;
                    voice.wait = voice.loop_gap;
                }
            }
            v *= 0.25;
            // Clicks.
            if self.train.on {
                self.train.to_next -= dt;
                if self.train.to_next <= 0.0 {
                    let tr = self.train;
                    let k = (tr.total - tr.left) as f32 / tr.total.max(1) as f32;
                    // Beam sweeping past: sin² over the train, ±1.5 dB click to click.
                    let amp = tr.level * (core::f32::consts::PI * (0.08 + 0.84 * k)).sin().powi(2) * 10f32.powf(0.075 * self.rng.bipolar());
                    // A buzz is a quieter, duller creak (the animal is closing in, beam narrow).
                    let fc = if tr.buzz { 0.8 * tr.fc } else { tr.fc };
                    self.click(0.12 * cfg.clicks * amp, fc);
                    self.train.left -= 1;
                    self.train.ici *= tr.ici_mul;
                    self.train.to_next = self.train.ici;
                    if self.train.left == 0 {
                        self.train.on = false;
                    }
                }
            }
            let n = self.ring.len();
            v += self.ring[self.ring_pos];
            self.ring[self.ring_pos] = 0.0;
            self.ring_pos = (self.ring_pos + 1) % n;
            let d0 = self.lp[0].tick(v);
            let d = self.lp[1].tick(d0);
            let r = self.reverb.tick(d);
            *o = (d + wet * r) * self.out_gain.next();
        }
        self.out_gain.finish();
        for l in self.lp.iter_mut() {
            l.flush();
        }
        self.reverb.flush();
    }

    fn busy(&self) -> bool {
        self.voices.iter().any(|v| v.on || v.loops_left > 0) || self.train.on
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn render(cfg: DolphinConfig, seed: u64, seconds: f32) -> Vec<f32> {
        let mut d = Dolphins::new(SR, seed, cfg);
        let c = Controls { activity: 1.0, ..Default::default() };
        let mut out = vec![0.0f32; (seconds * SR) as usize];
        for ch in out.chunks_mut(64) {
            d.render(ch, &c);
        }
        out
    }

    #[test]
    fn whistles_stay_in_range_and_are_short() {
        let mut rng = Rng::new(5);
        for _ in 0..200 {
            let s = signature(&mut rng);
            assert!((0.25..1.0).contains(&s.dur));
            let top = s.contour.max_hz();
            assert!((6000.0..18_000.0).contains(&top), "{top}");
        }
    }

    #[test]
    fn pod_is_audible_soft_and_deterministic() {
        let a = render(DolphinConfig { whistles_per_min: 30.0, trains_per_min: 10.0, ..Default::default() }, 2, 30.0);
        let b = render(DolphinConfig { whistles_per_min: 30.0, trains_per_min: 10.0, ..Default::default() }, 2, 30.0);
        assert_eq!(a, b);
        let rms = (a.iter().map(|v| (v * v) as f64).sum::<f64>() / a.len() as f64).sqrt();
        let peak = a.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(rms > 1e-3 && peak < 1.0, "rms {rms} peak {peak}");
    }

    #[test]
    fn silent_when_held_back() {
        let mut d = Dolphins::new(SR, 1, DolphinConfig::default());
        let c = Controls { activity: 0.0, ..Default::default() };
        let mut buf = [0.0f32; 64];
        for _ in 0..(10.0 * SR / 64.0) as usize {
            d.render(&mut buf, &c);
            assert!(buf.iter().all(|&v| v == 0.0));
        }
    }
}
