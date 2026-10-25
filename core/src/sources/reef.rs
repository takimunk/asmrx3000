//! A coral reef heard from a few tens of metres: snapping-shrimp crackle and fish.
//!
//! Snapping shrimp (Alpheidae) close a claw so fast that a cavitation bubble forms and
//! collapses; each snap is a broadband impulse (Versluis et al. 2000; Au & Banks 1998). A reef
//! holds thousands of them, snapping independently: a Poisson process of impulses whose
//! amplitudes follow the spread of distances (with shrimp spread evenly over the bottom, the
//! number within r grows as r², the amplitude falls as 1/r, so P(A > a) ∝ a⁻²). Far snaps are
//! duller (scattering and absorption take the top off), near ones brighter. The colony's
//! activity drifts slowly (time of day, tide), a couple of dB over minutes.
//!
//! Fish call with their swim bladders, drummed by sonic muscles: each muscle twitch is a
//! damped low pulse. Damselfish make short pulse trains ("chirps", pulses ≈ 36 ms apart,
//! measured on the NOAA American Samoa hydrophone clip, carrier 400–1100 Hz, trains
//! 60–270 ms); groupers and grunts make lower, slower pulse series (≈ 90–200 Hz); others
//! single knocks and pops.
//!
//! Comfort: the crackle is shaped soft (gentle 12 dB/octave slopes, no resonance, centroid
//! near 3–4 kHz rather than the brighter spectrum a hydrophone right on the reef records) and
//! the loudest snaps are capped at ~12 dB over the median, so it reads as an enveloping
//! texture and never as a fizz; fish pulses rise over 1–4 ms, so none of them clicks.

use serde::{Deserialize, Serialize};

use super::common::{Events, Ou, Ramp};
use super::marine::{Allpass, Highpass, Lowpass, SoftPulse};
use super::rain::check;
use super::{Controls, Generator};
use crate::rng::Rng;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct ReefConfig {
    /// Linear output gain (0..64).
    pub gain: f32,
    /// Snaps per second of the shrimp colony within earshot (0..5000).
    pub snaps_per_s: f32,
    /// Crackle level (0..2).
    pub crackle: f32,
    /// Brightness of the nearest snaps, Hz (2000..12000): the colony's distance.
    pub tone_hz: f32,
    /// Fish calls per minute (0..60).
    pub fish_per_min: f32,
    /// Fish level (0..2).
    pub fish: f32,
    /// Slow wander of the colony's activity, dB (0..6).
    pub wander_db: f32,
}

impl Default for ReefConfig {
    fn default() -> Self {
        Self { gain: 1.0, snaps_per_s: 900.0, crackle: 1.0, tone_hz: 5000.0, fish_per_min: 10.0, fish: 1.0, wander_db: 2.0 }
    }
}

impl ReefConfig {
    pub fn validate(&self) -> Result<(), String> {
        check("gain", self.gain, 0.0, 64.0)?;
        check("snaps_per_s", self.snaps_per_s, 0.0, 5000.0)?;
        check("crackle", self.crackle, 0.0, 2.0)?;
        check("tone_hz", self.tone_hz, 2000.0, 12000.0)?;
        check("fish_per_min", self.fish_per_min, 0.0, 60.0)?;
        check("fish", self.fish, 0.0, 2.0)?;
        check("wander_db", self.wander_db, 0.0, 6.0)
    }
}

/// Loudest snap against the faintest counted one (amplitude): caps the crest.
const CAP: f32 = 6.0;
const PULSES: usize = 12;
const CALLS: usize = 3;

#[derive(Clone, Copy, Default)]
struct Call {
    on: bool,
    left: u32,
    count: u32,
    period: f32,
    to_next: f32,
    freq: f32,
    decay: f32,
    rise: f32,
    amp: f32,
}

pub struct Reef {
    sample_rate: f32,
    rng: Rng,
    cfg: ReefConfig,
    snaps: Events,
    wander: Ou,
    level: f32,
    /// Near / middle / far snap paths.
    lp: [Lowpass; 3],
    hp: Highpass,
    /// Multipath over the reef: every snap arrives smeared over a few milliseconds.
    smear: [Allpass; 3],
    pulses: [SoftPulse; PULSES],
    next_pulse: usize,
    calls: [Call; CALLS],
    fish_ev: Events,
    fish_lp: Lowpass,
    fish_hp: Highpass,
    out_gain: Ramp,
}

impl Reef {
    pub const PRESETS: &'static [&'static str] = &["coral_reef", "distant_reef", "quiet_reef"];

    pub fn coral_reef() -> ReefConfig {
        ReefConfig::default()
    }
    pub fn distant_reef() -> ReefConfig {
        ReefConfig { snaps_per_s: 1500.0, tone_hz: 4000.0, fish_per_min: 6.0, fish: 0.7, ..Default::default() }
    }
    pub fn quiet_reef() -> ReefConfig {
        ReefConfig { snaps_per_s: 250.0, crackle: 0.8, fish_per_min: 4.0, ..Default::default() }
    }

    pub fn preset(name: &str) -> Option<ReefConfig> {
        Some(match name {
            "coral_reef" => Self::coral_reef(),
            "distant_reef" => Self::distant_reef(),
            "quiet_reef" => Self::quiet_reef(),
            _ => return None,
        })
    }

    pub fn new(sample_rate: f32, seed: u64, cfg: ReefConfig) -> Self {
        let sr = sample_rate;
        let mut rng = Rng::new(seed ^ 0x2EEF_5A1D);
        Self {
            sample_rate,
            snaps: Events::new(&mut rng),
            wander: Ou::default(),
            level: 1.0,
            lp: Self::paths(cfg.tone_hz, sr),
            hp: Highpass::new(1000.0, sr),
            smear: [Allpass::new(1.3, 0.62, sr), Allpass::new(2.9, 0.6, sr), Allpass::new(4.7, 0.58, sr)],
            pulses: [SoftPulse::default(); PULSES],
            next_pulse: 0,
            calls: [Call::default(); CALLS],
            fish_ev: Events::new(&mut rng),
            fish_lp: Lowpass::new(1600.0, sr),
            fish_hp: Highpass::new(60.0, sr),
            out_gain: Ramp::new(cfg.gain),
            rng,
            cfg,
        }
    }

    fn paths(tone: f32, sr: f32) -> [Lowpass; 3] {
        [Lowpass::new(tone, sr), Lowpass::new(tone * 0.6, sr), Lowpass::new(tone * 0.36, sr)]
    }

    pub fn config(&self) -> &ReefConfig {
        &self.cfg
    }

    fn start_call(&mut self) {
        let Some(slot) = self.calls.iter().position(|c| !c.on) else { return };
        let r = self.rng.uniform();
        let level = 10f32.powf(self.rng.range(-10.0, 0.0) / 20.0);
        let call = if r < 0.55 {
            // Damselfish chirp.
            Call { count: 4 + self.rng.next_u32() % 7, period: self.rng.range(0.031, 0.041), freq: self.rng.range(380.0, 950.0), decay: 0.006, rise: 0.0012, amp: 0.6 * level, ..Default::default() }
        } else if r < 0.8 {
            // Low grunt: a slower pulse series.
            Call { count: 5 + self.rng.next_u32() % 8, period: self.rng.range(0.045, 0.075), freq: self.rng.range(100.0, 200.0), decay: 0.014, rise: 0.004, amp: 0.3 * level, ..Default::default() }
        } else {
            // A knock or two.
            Call { count: 1 + self.rng.next_u32() % 2, period: self.rng.range(0.08, 0.16), freq: self.rng.range(180.0, 450.0), decay: 0.009, rise: 0.002, amp: 0.7 * level, ..Default::default() }
        };
        self.calls[slot] = Call { on: true, left: call.count, to_next: 0.0, ..call };
    }
}

impl Generator for Reef {
    fn set_model(&mut self, cfg: &super::ModelConfig) {
        if let super::ModelConfig::Reef(c) = cfg {
            if c.tone_hz != self.cfg.tone_hz {
                let sr = self.sample_rate;
                for (l, k) in self.lp.iter_mut().zip([1.0, 0.6, 0.36]) {
                    l.set(c.tone_hz * k, sr);
                }
            }
            self.cfg = *c;
        }
    }

    fn render(&mut self, out: &mut [f32], c: &Controls) {
        let sr = self.sample_rate;
        let dt = out.len() as f32 / sr;
        let cfg = self.cfg;
        self.out_gain.to(cfg.gain, out.len());
        // Colony activity: OU in log level (τ = 40 s); the rate follows it.
        let sigma = cfg.wander_db / 8.686;
        let w = self.wander.step(dt, 40.0, sigma, &mut self.rng).clamp(-3.0 * sigma.max(1e-3), 3.0 * sigma.max(1e-3));
        let target = w.exp();
        self.level += (target - self.level) * (dt / 2.0).min(1.0);
        let rate = cfg.snaps_per_s * self.level * self.level / sr;
        let snap_gain = 0.05 * cfg.crackle;
        let fish_gain = 0.25 * cfg.fish;
        let fish_rate = if c.activity > 0.02 { cfg.fish_per_min / 60.0 / sr } else { 0.0 };
        let sdt = 1.0 / sr;
        for o in out.iter_mut() {
            // Snaps.
            let mut x = [0.0f32; 3];
            let n = self.snaps.tick(rate, &mut self.rng);
            for _ in 0..n {
                let a = (1.0 / self.rng.uniform().max(1.0 / (CAP * CAP))).sqrt();
                let s = if self.rng.uniform() < 0.5 { -a } else { a };
                let path = if a > 2.2 { 0 } else if a > 1.4 { 1 } else { 2 };
                x[path] += s;
            }
            let mut snap = 0.0;
            for (p, l) in self.lp.iter_mut().enumerate() {
                snap += l.tick(x[p]);
            }
            let mut snap = self.hp.tick(snap);
            for a in self.smear.iter_mut() {
                snap = a.tick(snap);
            }
            let snap = snap * snap_gain;

            // Fish.
            if self.fish_ev.tick(fish_rate, &mut self.rng) > 0 {
                self.start_call();
            }
            for k in 0..CALLS {
                let call = self.calls[k];
                if !call.on {
                    continue;
                }
                let mut cl = call;
                cl.to_next -= sdt;
                if cl.to_next <= 0.0 {
                    let i = (cl.count - cl.left) as f32;
                    // The train swells and fades over its pulses (no abrupt first pulse).
                    let shape = (core::f32::consts::PI * (i + 0.5) / cl.count as f32).sin().sqrt();
                    let f = cl.freq * (1.0 + 0.02 * self.rng.bipolar());
                    let slot = self.next_pulse;
                    self.next_pulse = (self.next_pulse + 1) % PULSES;
                    self.pulses[slot].strike(f, cl.decay, cl.rise, cl.amp * shape, sr);
                    cl.left -= 1;
                    cl.to_next = cl.period;
                    if cl.left == 0 {
                        cl.on = false;
                    }
                }
                self.calls[k] = cl;
            }
            let mut fish = 0.0;
            for p in self.pulses.iter_mut() {
                fish += p.tick();
            }
            let fish = self.fish_lp.tick(self.fish_hp.tick(fish)) * fish_gain;
            *o = (snap + fish) * self.out_gain.next();
        }
        self.out_gain.finish();
        for l in self.lp.iter_mut() {
            l.flush();
        }
        self.hp.flush();
        self.fish_lp.flush();
        self.fish_hp.flush();
        for p in self.pulses.iter_mut() {
            p.flush();
        }
    }

    fn event_rate(&self) -> f32 {
        self.cfg.snaps_per_s * self.level * self.level
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn render(cfg: ReefConfig, seed: u64, seconds: f32) -> Vec<f32> {
        let mut r = Reef::new(SR, seed, cfg);
        let c = Controls { activity: 1.0, ..Default::default() };
        let mut out = vec![0.0f32; (seconds * SR) as usize];
        for ch in out.chunks_mut(64) {
            r.render(ch, &c);
        }
        out
    }

    #[test]
    fn crackle_is_dense_soft_and_capped() {
        let x = render(ReefConfig { fish_per_min: 0.0, wander_db: 0.0, ..Default::default() }, 4, 20.0);
        let rms = (x.iter().map(|v| (v * v) as f64).sum::<f64>() / x.len() as f64).sqrt() as f32;
        let peak = x.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(rms > 1e-3, "rms {rms}");
        // A soft texture: crest factor well below that of a sparse click track.
        eprintln!("reef crest {:.1} dB", 20.0 * (peak / rms).log10());
        assert!(peak / rms < 9.0, "crest {}", peak / rms);
        // 50 ms levels vary little (an enveloping bed, not a sparse crackle).
        let w = 2400;
        let lv: Vec<f32> = x.chunks(w).map(|c| 10.0 * (c.iter().map(|v| v * v).sum::<f32>() / w as f32).log10()).collect();
        let mut s = lv.clone();
        s.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let spread = s[s.len() * 9 / 10] - s[s.len() / 10];
        eprintln!("reef 50 ms spread {spread:.1} dB");
        assert!(spread < 5.0, "50 ms level spread {spread} dB");
    }

    #[test]
    fn fish_pulses_have_soft_onsets() {
        let x = render(ReefConfig { snaps_per_s: 0.0, fish_per_min: 60.0, ..Default::default() }, 9, 30.0);
        let peak = x.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        let step = x.windows(2).fold(0.0f32, |m, w| m.max((w[1] - w[0]).abs()));
        assert!(peak > 1e-3);
        assert!(step < 0.2 * peak, "step {step} vs peak {peak}");
    }

    #[test]
    fn deterministic() {
        assert_eq!(render(ReefConfig::default(), 3, 5.0), render(ReefConfig::default(), 3, 5.0));
    }
}
