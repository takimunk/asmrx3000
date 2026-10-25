//! The water itself, heard with the head under the surface: the ambient bed of open water,
//! a slow stream of bubbles (a diver breathing out, or a vent), and rain on the surface above.
//!
//! * **Bed.** Ocean ambient noise in calm weather (Wenz 1962; Knudsen curves) is broadband
//!   with its maximum at a few hundred Hz, falling ≈ 5–6 dB per octave above 1 kHz; below
//!   ~100 Hz the "current" is heard as pseudo-sound, the turbulent flow over the ears.
//!   The swell moves the water to and fro with periods of 8–15 s, so the flow noise breathes
//!   slowly. Here: low flow noise (< 160 Hz), a broad "wash" band around 450 Hz and a faint
//!   tail above 1 kHz, their levels swelling with a slow, smooth swell cycle (amplitude only:
//!   no pitch is ever modulated).
//! * **Bubbles.** An air bubble rings at its Minnaert frequency f = 3.26 / r (r in m) and
//!   decays in a few milliseconds (van den Doel 2005 damping). A diver breathes out every
//!   ~5 s: a 1–2 s burst of bubbles of 1.5–6 mm radius (0.55–2.2 kHz) with a few large
//!   ones; a vent releases a steady trickle. Each bubble starts with a 0.4 ms rise (a SoftPulse),
//!   so even a dense burst never clicks.
//! * **Rain above.** Drops hitting the surface make an impact pulse and, for the larger
//!   drops, a bubble (Medwin & Nystuen; Prosperetti & Oguz 1993). Small drops ring at
//!   13–25 kHz — a bright hiss that is accurate but tiring; this model keeps the patter of
//!   impacts in a soft 1–4.5 kHz band and the occasional larger-drop bubble, and leaves the
//!   small-drop hump out (documented as a comfort choice in docs/scenes/underwater.md).

use serde::{Deserialize, Serialize};

use super::common::{bubble_damping, minnaert_hz, Events, Ou, Ramp};
use super::marine::{Highpass, Lowpass, OnePole, SoftPulse};
use super::rain::check;
use super::{Controls, Generator};
use crate::dsp::Svf;
use crate::rng::Rng;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct SeaConfig {
    /// Linear output gain (0..64).
    pub gain: f32,
    /// Low flow noise of the current (0..2).
    pub flow: f32,
    /// Broad mid-band wash of the open water (0..2).
    pub wash: f32,
    /// Swell period, s (4..20).
    pub swell_s: f32,
    /// How much the bed swells with it, dB peak to peak (0..8).
    pub surge_db: f32,
    /// Bubble level (0..2).
    pub bubbles: f32,
    /// A diver's exhalations per minute (0..20; 0 = no diver).
    pub breaths_per_min: f32,
    /// Steady bubbles per second from a vent (0..40).
    pub vent_per_s: f32,
    /// Low-pass of the bubbles and the rain, Hz (500..8000): their distance.
    pub tone_hz: f32,
    /// Rain rate on the surface above, mm/h (0..60).
    pub rain_mm_h: f32,
    /// Rain level (0..2).
    pub rain: f32,
}

impl Default for SeaConfig {
    fn default() -> Self {
        Self { gain: 1.0, flow: 1.0, wash: 1.0, swell_s: 10.0, surge_db: 4.0, bubbles: 0.0, breaths_per_min: 0.0, vent_per_s: 0.0, tone_hz: 3000.0, rain_mm_h: 0.0, rain: 0.0 }
    }
}

impl SeaConfig {
    pub fn validate(&self) -> Result<(), String> {
        check("gain", self.gain, 0.0, 64.0)?;
        check("flow", self.flow, 0.0, 2.0)?;
        check("wash", self.wash, 0.0, 2.0)?;
        check("swell_s", self.swell_s, 4.0, 20.0)?;
        check("surge_db", self.surge_db, 0.0, 8.0)?;
        check("bubbles", self.bubbles, 0.0, 2.0)?;
        check("breaths_per_min", self.breaths_per_min, 0.0, 20.0)?;
        check("vent_per_s", self.vent_per_s, 0.0, 40.0)?;
        check("tone_hz", self.tone_hz, 500.0, 8000.0)?;
        check("rain_mm_h", self.rain_mm_h, 0.0, 60.0)?;
        check("rain", self.rain, 0.0, 2.0)
    }
}

const BUBBLES: usize = 32;

pub struct Sea {
    sample_rate: f32,
    rng: Rng,
    cfg: SeaConfig,
    // Bed.
    flow_lp: Lowpass,
    flow_hp: Highpass,
    wash: Svf,
    tail_lp: Lowpass,
    tail_hp: Highpass,
    tail_tilt: OnePole,
    swell_phase: f32,
    swell_period: f32,
    drift: Ou,
    flow_g: Ramp,
    wash_g: Ramp,
    // Bubbles.
    pool: [SoftPulse; BUBBLES],
    next: usize,
    vent: Events,
    /// Seconds to the next exhalation, seconds into the current one and its length.
    breath_wait: f32,
    breath_t: f32,
    breath_len: f32,
    breath_rate: f32,
    breath_ev: Events,
    bubble_lp: Lowpass,
    // Rain.
    drops: Events,
    big_drops: Events,
    rain_hp: Highpass,
    rain_lp: Lowpass,
    rain_g: Ramp,
    out_gain: Ramp,
}

impl Sea {
    pub const PRESETS: &'static [&'static str] = &["open_water", "deep_water", "diver_bubbles", "vent_bubbles", "rain_above"];

    pub fn open_water() -> SeaConfig {
        SeaConfig::default()
    }
    /// Farther from the surface: less wash, a slower, deeper swell.
    pub fn deep_water() -> SeaConfig {
        SeaConfig { flow: 0.8, wash: 0.45, swell_s: 13.0, surge_db: 3.0, ..Default::default() }
    }
    pub fn diver_bubbles() -> SeaConfig {
        SeaConfig { flow: 0.0, wash: 0.0, surge_db: 0.0, bubbles: 1.0, breaths_per_min: 11.0, ..Default::default() }
    }
    pub fn vent_bubbles() -> SeaConfig {
        SeaConfig { flow: 0.0, wash: 0.0, surge_db: 0.0, bubbles: 1.0, vent_per_s: 6.0, tone_hz: 2400.0, ..Default::default() }
    }
    pub fn rain_above() -> SeaConfig {
        SeaConfig { flow: 0.0, wash: 0.0, surge_db: 0.0, rain_mm_h: 8.0, rain: 1.0, ..Default::default() }
    }

    pub fn preset(name: &str) -> Option<SeaConfig> {
        Some(match name {
            "open_water" => Self::open_water(),
            "deep_water" => Self::deep_water(),
            "diver_bubbles" => Self::diver_bubbles(),
            "vent_bubbles" => Self::vent_bubbles(),
            "rain_above" => Self::rain_above(),
            _ => return None,
        })
    }

    pub fn new(sample_rate: f32, seed: u64, cfg: SeaConfig) -> Self {
        let sr = sample_rate;
        let mut rng = Rng::new(seed ^ 0x05EA_BED5);
        let swell_phase = rng.uniform();
        let swell_period = cfg.swell_s * rng.range(0.85, 1.15);
        let breath_wait = rng.range(0.5, 4.0);
        Self {
            sample_rate,
            flow_lp: Lowpass::new(160.0, sr),
            flow_hp: Highpass::new(22.0, sr),
            wash: Svf::new(450.0, 0.55, sr),
            tail_lp: Lowpass::new(4000.0, sr),
            tail_hp: Highpass::new(900.0, sr),
            tail_tilt: OnePole::new(1500.0, sr),
            swell_phase,
            swell_period,
            drift: Ou::default(),
            flow_g: Ramp::new(0.0),
            wash_g: Ramp::new(0.0),
            pool: [SoftPulse::default(); BUBBLES],
            next: 0,
            vent: Events::new(&mut rng),
            breath_wait,
            breath_t: 0.0,
            breath_len: 0.0,
            breath_rate: 0.0,
            breath_ev: Events::new(&mut rng),
            bubble_lp: Lowpass::new(cfg.tone_hz, sr),
            drops: Events::new(&mut rng),
            big_drops: Events::new(&mut rng),
            rain_hp: Highpass::new(1000.0, sr),
            rain_lp: Lowpass::new(cfg.tone_hz.min(4500.0), sr),
            rain_g: Ramp::new(0.0),
            out_gain: Ramp::new(cfg.gain),
            rng,
            cfg,
        }
    }

    pub fn config(&self) -> &SeaConfig {
        &self.cfg
    }

    fn bubble(&mut self, r: f32, amp: f32) {
        let sr = self.sample_rate;
        let f = minnaert_hz(r);
        if f > 0.4 * sr {
            return;
        }
        let decay = 1.0 / bubble_damping(f);
        // The quietest slot (with 32 slots and millisecond decays, normally a silent one).
        let mut slot = self.next;
        let mut e = f32::MAX;
        for (i, p) in self.pool.iter().enumerate() {
            let pe = p.energy();
            if pe < e {
                e = pe;
                slot = i;
            }
        }
        self.next = (slot + 1) % BUBBLES;
        self.pool[slot] = SoftPulse::default();
        self.pool[slot].strike(f, decay, 0.0004, amp, sr);
    }
}

impl Generator for Sea {
    fn set_model(&mut self, cfg: &super::ModelConfig) {
        if let super::ModelConfig::Sea(c) = cfg {
            let sr = self.sample_rate;
            self.bubble_lp.set(c.tone_hz, sr);
            self.rain_lp.set(c.tone_hz.min(4500.0), sr);
            self.cfg = *c;
        }
    }

    fn render(&mut self, out: &mut [f32], _c: &Controls) {
        let sr = self.sample_rate;
        let len = out.len();
        let dt = len as f32 / sr;
        let cfg = self.cfg;

        // Swell: one smooth cycle per period; the period itself changes only at the top of a
        // cycle (it is a level, never a pitch, so this cannot be heard as wobble).
        self.swell_phase += dt / self.swell_period.max(1.0);
        if self.swell_phase >= 1.0 {
            self.swell_phase -= 1.0;
            self.swell_period = cfg.swell_s * self.rng.range(0.85, 1.15);
        }
        let s = (core::f32::consts::TAU * self.swell_phase).sin();
        let s_lag = (core::f32::consts::TAU * (self.swell_phase - 0.2)).sin();
        let drift = self.drift.step(dt, 30.0, 0.1, &mut self.rng).clamp(-0.3, 0.3).exp();
        let half = 0.5 * cfg.surge_db / 20.0;
        let flow_amp = 0.35 * cfg.flow * 10f32.powf(half * s) * drift;
        let wash_amp = 0.12 * cfg.wash * 10f32.powf(half * s_lag) * drift;
        self.flow_g.to(flow_amp, len);
        self.wash_g.to(wash_amp, len);

        // Diver: exhalations at `breaths_per_min`, each a 1.1–1.8 s burst whose bubble rate
        // rises and falls (sin²), peaking near 45 bubbles/s.
        let mut breath_rate = 0.0;
        if cfg.breaths_per_min > 0.0 && cfg.bubbles > 0.0 {
            if self.breath_len > 0.0 {
                self.breath_t += dt;
                let u = self.breath_t / self.breath_len;
                if u >= 1.0 {
                    self.breath_len = 0.0;
                    let period = 60.0 / cfg.breaths_per_min;
                    self.breath_wait = period * self.rng.range(0.8, 1.2) - self.breath_t;
                } else {
                    breath_rate = self.breath_rate * (core::f32::consts::PI * u).sin().powi(2);
                }
            } else {
                self.breath_wait -= dt;
                if self.breath_wait <= 0.0 {
                    self.breath_t = 0.0;
                    self.breath_len = self.rng.range(1.1, 1.8);
                    self.breath_rate = self.rng.range(35.0, 55.0);
                }
            }
        }

        // Rain: impacts at ~150 drops/s per mm/h within earshot (capped), larger drops with a
        // bubble at ~1 per s per mm/h.
        let mm = cfg.rain_mm_h.max(0.0);
        let rain_on = cfg.rain > 0.0 && mm > 0.0;
        let drop_rate = if rain_on { (150.0 * mm).min(6000.0) / sr } else { 0.0 };
        let big_rate = if rain_on { (1.0 * mm).min(40.0) / sr } else { 0.0 };
        self.rain_g.to(if rain_on { 0.02 * cfg.rain } else { 0.0 }, len);
        let vent_rate = if cfg.bubbles > 0.0 { cfg.vent_per_s / sr } else { 0.0 };
        let b_rate = breath_rate / sr;

        self.out_gain.to(cfg.gain, len);
        for o in out.iter_mut() {
            // Bed.
            let n1 = self.rng.bipolar();
            let n2 = self.rng.bipolar();
            let flow = self.flow_hp.tick(self.flow_lp.tick(n1)) * self.flow_g.next();
            let (_, bp, _) = self.wash.process(n2);
            let tail = self.tail_tilt.tick(self.tail_hp.tick(self.tail_lp.tick(n2)));
            let wash = (bp + 0.35 * tail) * self.wash_g.next();

            // Bubbles.
            if self.vent.tick(vent_rate, &mut self.rng) > 0 {
                let r = self.rng.range(1.5e-3, 3.0e-3);
                let a = 0.5 * self.rng.range(0.4, 1.0);
                self.bubble(r, a);
            }
            if b_rate > 0.0 && self.breath_ev.tick(b_rate, &mut self.rng) > 0 {
                // Log-uniform 1.5–6 mm, a few large ones; bigger bubbles are louder.
                let big = self.rng.uniform() < 0.08;
                let r = if big { self.rng.range(6e-3, 9e-3) } else { (self.rng.range(1.5e-3f32.ln(), 6e-3f32.ln())).exp() };
                let a = 0.6 * (r / 3e-3).sqrt() * self.rng.range(0.3, 1.0);
                self.bubble(r, a);
            }
            // Rain impacts.
            let mut imp = 0.0;
            let n = self.drops.tick(drop_rate, &mut self.rng);
            for _ in 0..n {
                let a = (1.0 / self.rng.uniform().max(1.0 / 16.0)).sqrt();
                imp += if self.rng.uniform() < 0.5 { -a } else { a };
            }
            if self.big_drops.tick(big_rate, &mut self.rng) > 0 {
                let r = self.rng.range(0.8e-3, 2.0e-3);
                let a = 0.12 * cfg.rain / cfg.bubbles.max(0.25) * self.rng.range(0.3, 1.0);
                self.bubble(r, a);
            }
            let rain = self.rain_lp.tick(self.rain_hp.tick(imp)) * self.rain_g.next();

            let mut b = 0.0;
            for p in self.pool.iter_mut() {
                b += p.tick();
            }
            let bubbles = self.bubble_lp.tick(b) * 0.3 * cfg.bubbles.max(0.25);

            *o = (flow + wash + bubbles + rain) * self.out_gain.next();
        }
        self.flow_g.finish();
        self.wash_g.finish();
        self.rain_g.finish();
        self.out_gain.finish();
        self.flow_lp.flush();
        self.flow_hp.flush();
        self.wash.flush();
        self.tail_lp.flush();
        self.tail_hp.flush();
        self.tail_tilt.flush();
        self.bubble_lp.flush();
        self.rain_hp.flush();
        self.rain_lp.flush();
        for p in self.pool.iter_mut() {
            p.flush();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn render(cfg: SeaConfig, seed: u64, seconds: f32) -> Vec<f32> {
        let mut s = Sea::new(SR, seed, cfg);
        let c = Controls { activity: 1.0, ..Default::default() };
        let mut out = vec![0.0f32; (seconds * SR) as usize];
        for ch in out.chunks_mut(64) {
            s.render(ch, &c);
        }
        out
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| (v * v) as f64).sum::<f64>() / x.len().max(1) as f64).sqrt() as f32
    }

    #[test]
    fn bed_swells_gently() {
        let x = render(Sea::open_water(), 1, 60.0);
        let lv: Vec<f32> = x.chunks(48_000).map(|c| 20.0 * rms(c).log10()).collect();
        let (lo, hi) = lv.iter().fold((f32::MAX, f32::MIN), |(a, b), &v| (a.min(v), b.max(v)));
        assert!(rms(&x) > 0.01);
        assert!(hi - lo < 7.0, "1 s levels span {} dB", hi - lo);
        assert!(hi - lo > 1.0, "no swell: {} dB", hi - lo);
    }

    #[test]
    fn presets_are_audible_and_soft() {
        for p in Sea::PRESETS {
            let x = render(Sea::preset(p).unwrap(), 2, 20.0);
            let r = rms(&x);
            let peak = x.iter().fold(0.0f32, |m, v| m.max(v.abs()));
            assert!(r > 1e-3 && peak < 1.0, "{p}: rms {r} peak {peak}");
            let step = x.windows(2).fold(0.0f32, |m, w| m.max((w[1] - w[0]).abs()));
            assert!(step < 0.5 * peak.max(1e-6), "{p}: step {step} vs peak {peak}");
        }
    }

    #[test]
    fn deterministic() {
        assert_eq!(render(Sea::diver_bubbles(), 7, 8.0), render(Sea::diver_bubbles(), 7, 8.0));
    }
}
