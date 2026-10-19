//! Singing insects: mechanical stridulation and tymbal buckling, with rates that follow
//! temperature (insects are ectotherms: their muscles, and so their song, run faster when warm).
//!
//! * **Crickets** (Gryllidae) close their wings to drag a scraper across a file of teeth; an
//!   escapement makes the tooth-strike rate equal the resonance of the wing's harp, so each
//!   closing stroke (a syllable) is an almost pure tone (field cricket ~4.5 kHz) whose pitch
//!   sags slightly as the stroke slows. Syllables come in chirps.
//! * **Snowy tree cricket** (Oecanthus fultoni) is Dolbear's thermometer (1897): chirps per
//!   minute N = 4·T_F − 160, i.e. N = 7.2·T_C − 32; neighbours synchronise.
//! * **Bush crickets** (Tettigoniidae) sing long trains of broadband, high (~10 kHz) syllables
//!   at night; **grasshoppers** rub leg pegs on a wing vein: buzzy noise echemes in the sun.
//! * **Cicadas** buckle ribbed tymbals; each buckling is a click that rings the abdominal
//!   resonator (~5 kHz), at a few hundred pulses per second, in long crescendos, in the heat.
//!
//! Rate laws are linear in temperature (Q₁₀ ≈ 2 is a 4–5 %/°C slope near 20 °C). Each species
//! has a temperature floor, active hours, and stops in heavy rain or strong wind.
//!
//! The same model voices the other small rhythmic animals of the evening and the wood, which
//! follow the same pattern (a pulse rate set by a muscle, temperature-dependent, grouped into
//! notes and calls) but excite a resonator instead of a file of teeth:
//! * **Frogs** push air from the lungs over the vocal cords into the vocal sac; each pulse of
//!   the cords rings the sac's resonance (formants), pulses come at tens per second within a
//!   note and notes are grouped into calls. Marsh frog (Pelophylax ridibundus): the "laughing"
//!   call, 6–12 notes at ~7/s, dominant 1–2 kHz, loud dusk-and-night choruses of lakes and
//!   rivers in late spring. European tree frog (Hyla arborea): rapid "krak-krak" notes of a
//!   few pulses, ~7 notes/s, dominant ~2.2 kHz, very loud at night. Common frog (Rana
//!   temporaria): a soft, low purring croak, ~25 pulses/s around 500 Hz. Call and pulse rates
//!   rise with temperature like the insects' (Gerhardt & Huber 2002; Schneider 2005).
//! * **Woodpecker drumming** (great spotted woodpecker, Dendrocopos major): 10–16 bill strikes
//!   in ~0.6 s at ~20/s, slowing and fading at the end, each strike ringing the resonance of
//!   the dead branch (a few hundred Hz to ~1.5 kHz) under a dry knock; one drum every 20–60 s,
//!   mostly in the morning.

use core::f32::consts::TAU;

use serde::{Deserialize, Serialize};

use super::common::*;
use super::rain::check;
use super::{Controls, Generator};
use crate::rng::Rng;

const MAX_INDIVIDUALS: usize = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum InsectKind {
    #[default]
    FieldCricket,
    TreeCricket,
    BushCricket,
    Grasshopper,
    Cicada,
    MarshFrog,
    TreeFrog,
    CommonFrog,
    Woodpecker,
}

/// Species song parameters at 20 °C and their temperature slopes.
#[derive(Clone, Copy, Debug)]
pub struct SongLaw {
    /// Carrier (resonance) frequency, Hz.
    pub carrier_hz: f32,
    /// Syllables (pulses) per second at 20 °C, and relative change per °C.
    pub pulse_hz: f32,
    pub pulse_slope: f32,
    /// Syllables per chirp / echeme (0 = continuous trains).
    pub pulses_per_chirp: u32,
    /// Resonator Q (tonal crickets high, scraping grasshoppers low).
    pub q: f32,
}

pub fn song_law(kind: InsectKind) -> SongLaw {
    match kind {
        InsectKind::FieldCricket => SongLaw { carrier_hz: 4500.0, pulse_hz: 30.0, pulse_slope: 0.045, pulses_per_chirp: 4, q: 25.0 },
        InsectKind::TreeCricket => SongLaw { carrier_hz: 2900.0, pulse_hz: 50.0, pulse_slope: 0.05, pulses_per_chirp: 6, q: 30.0 },
        InsectKind::BushCricket => SongLaw { carrier_hz: 10_000.0, pulse_hz: 30.0, pulse_slope: 0.04, pulses_per_chirp: 0, q: 2.5 },
        InsectKind::Grasshopper => SongLaw { carrier_hz: 8000.0, pulse_hz: 20.0, pulse_slope: 0.04, pulses_per_chirp: 12, q: 1.5 },
        InsectKind::Cicada => SongLaw { carrier_hz: 5200.0, pulse_hz: 220.0, pulse_slope: 0.03, pulses_per_chirp: 0, q: 8.0 },
        // Frogs: carrier = dominant (sac) frequency, pulse_hz = pulses within a note,
        // pulses_per_chirp = notes per call.
        InsectKind::MarshFrog => SongLaw { carrier_hz: 1500.0, pulse_hz: 60.0, pulse_slope: 0.03, pulses_per_chirp: 9, q: 5.0 },
        InsectKind::TreeFrog => SongLaw { carrier_hz: 2200.0, pulse_hz: 110.0, pulse_slope: 0.03, pulses_per_chirp: 18, q: 7.0 },
        InsectKind::CommonFrog => SongLaw { carrier_hz: 500.0, pulse_hz: 25.0, pulse_slope: 0.03, pulses_per_chirp: 3, q: 4.0 },
        // Woodpecker: carrier = branch resonance, pulse_hz = strikes per second, strikes per drum.
        InsectKind::Woodpecker => SongLaw { carrier_hz: 800.0, pulse_hz: 20.0, pulse_slope: 0.0, pulses_per_chirp: 13, q: 18.0 },
    }
}

/// Notes per second within a frog's call, and seconds of pulsing per note (temperature-free
/// values at 20 °C; scaled with the pulse rate).
fn note_law(kind: InsectKind) -> (f32, f32) {
    match kind {
        InsectKind::MarshFrog => (7.0, 0.075),
        InsectKind::TreeFrog => (7.0, 0.03),
        InsectKind::CommonFrog => (2.5, 0.3),
        _ => (1.0, 0.1),
    }
}

fn is_frog(kind: InsectKind) -> bool {
    matches!(kind, InsectKind::MarshFrog | InsectKind::TreeFrog | InsectKind::CommonFrog)
}

/// Chirps (or echemes) per minute at `t_c` °C; 0 below the activity threshold.
/// Tree cricket: Dolbear's law N = 7.2·T − 32.
pub fn chirps_per_min(kind: InsectKind, t_c: f32) -> f32 {
    match kind {
        InsectKind::TreeCricket => (7.2 * t_c - 32.0).max(0.0),
        // Gryllus: ~2.6 chirps/s at 20 °C, linear in T above ~8 °C.
        InsectKind::FieldCricket => (13.0 * (t_c - 8.0)).max(0.0),
        // Chorthippus: one echeme every few seconds, faster when warm.
        InsectKind::Grasshopper => (1.0 * (t_c - 10.0)).max(0.0),
        InsectKind::BushCricket | InsectKind::Cicada => 0.0,
        // Calls per minute of a frog in a chorus (each call several notes).
        InsectKind::MarshFrog => (1.0 * (t_c - 8.0)).max(0.0),
        InsectKind::TreeFrog => (0.8 * (t_c - 8.0)).max(0.0),
        InsectKind::CommonFrog => (1.5 * (t_c - 3.0)).max(0.0),
        // Drums per minute: a bird, not an ectotherm.
        InsectKind::Woodpecker => 1.6,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct InsectConfig {
    pub kind: InsectKind,
    /// Individuals in this emitter (1..12).
    pub count: u32,
    /// Carrier override, Hz (0 = species default; 1000..16000).
    pub carrier_hz: f32,
    /// Silent below this temperature, °C (−5..35).
    pub min_temp_c: f32,
    /// Active hours (local time); from == to means all day.
    pub active_from_h: f32,
    pub active_to_h: f32,
    /// Microclimate offset added to the air temperature, °C (−10..10): sunny meadow +3.
    pub temperature_offset_c: f32,
    /// Phase coupling between individuals (0..1): tree crickets chirp in unison.
    pub synchrony: f32,
    /// Individual variation of rate and pitch (0..0.3).
    pub spread: f32,
    /// Linear output gain (0..8).
    pub gain: f32,
}

impl Default for InsectConfig {
    fn default() -> Self {
        Self {
            kind: InsectKind::FieldCricket,
            count: 3,
            carrier_hz: 0.0,
            min_temp_c: 12.0,
            active_from_h: 19.0,
            active_to_h: 4.0,
            temperature_offset_c: 0.0,
            synchrony: 0.0,
            spread: 0.08,
            gain: 1.0,
        }
    }
}

impl InsectConfig {
    pub fn validate(&self) -> Result<(), String> {
        check("count", self.count as f32, 1.0, MAX_INDIVIDUALS as f32)?;
        if self.carrier_hz != 0.0 {
            check("carrier_hz", self.carrier_hz, 1000.0, 16_000.0)?;
        }
        check("min_temp_c", self.min_temp_c, -5.0, 35.0)?;
        check("active_from_h", self.active_from_h, 0.0, 24.0)?;
        check("active_to_h", self.active_to_h, 0.0, 24.0)?;
        check("temperature_offset_c", self.temperature_offset_c, -10.0, 10.0)?;
        check("synchrony", self.synchrony, 0.0, 1.0)?;
        check("spread", self.spread, 0.0, 0.3)?;
        check("gain", self.gain, 0.0, 8.0)
    }
}

#[derive(Clone, Copy, Default)]
struct Individual {
    /// Chirp clock, 0..1 per chirp period.
    chirp: f32,
    /// Index of the current syllable within the chirp, or u32::MAX between chirps.
    syl: u32,
    syl_phase: f32,
    carrier_phase: f32,
    rate_jit: f32,
    pitch_jit: f32,
    singing: bool,
    bout: f32,
    level: Smooth,
    level_ramp: Ramp,
    band: Svf,
    res: Mode,
    tymbal: bool,
    swell: f32,
    /// Frogs and drumming: a second resonance (the sac's second formant, the knock) and
    /// per-call variation (notes in this call, strength of this note).
    res2: Mode,
    notes: u32,
    strength: f32,
}

pub struct Insects {
    sample_rate: f32,
    rng: Rng,
    cfg: InsectConfig,
    ind: [Individual; MAX_INDIVIDUALS],
    law: SongLaw,
    temp: f32,
    activity: f32,
    chirps: u32,
}

impl Insects {
    pub fn new(sample_rate: f32, seed: u64, cfg: InsectConfig) -> Self {
        let mut rng = Rng::new(seed);
        let law = song_law(cfg.kind);
        let mut ind = [Individual::default(); MAX_INDIVIDUALS];
        for i in ind.iter_mut() {
            i.chirp = rng.uniform();
            i.syl = u32::MAX;
            i.rate_jit = 1.0 + cfg.spread * rng.bipolar();
            i.pitch_jit = 1.0 + 0.5 * cfg.spread * rng.bipolar();
            i.bout = rng.range(0.5, 6.0);
            i.swell = rng.uniform();
        }
        Self { sample_rate, rng, law, cfg, ind, temp: 20.0, activity: 0.0, chirps: 0 }
    }

    pub fn config(&self) -> &InsectConfig {
        &self.cfg
    }

    pub fn set_config(&mut self, cfg: InsectConfig) {
        self.law = song_law(cfg.kind);
        self.cfg = cfg;
    }

    /// Chirps started so far (all individuals), for tests.
    pub fn chirps(&self) -> u32 {
        self.chirps
    }

    pub fn field_cricket() -> InsectConfig {
        InsectConfig::default()
    }
    /// Dolbear's thermometer: chirps in unison, rate = 7.2·T − 32 per minute.
    pub fn tree_cricket() -> InsectConfig {
        InsectConfig { kind: InsectKind::TreeCricket, count: 5, min_temp_c: 10.0, active_from_h: 20.0, active_to_h: 4.0, synchrony: 0.8, ..Default::default() }
    }
    pub fn bush_cricket() -> InsectConfig {
        InsectConfig { kind: InsectKind::BushCricket, count: 2, min_temp_c: 14.0, active_from_h: 16.0, active_to_h: 2.0, ..Default::default() }
    }
    pub fn grasshopper() -> InsectConfig {
        InsectConfig { kind: InsectKind::Grasshopper, count: 4, min_temp_c: 17.0, active_from_h: 9.0, active_to_h: 18.5, temperature_offset_c: 3.0, ..Default::default() }
    }
    pub fn cicada() -> InsectConfig {
        InsectConfig { kind: InsectKind::Cicada, count: 3, min_temp_c: 22.0, active_from_h: 9.0, active_to_h: 19.0, temperature_offset_c: 2.0, ..Default::default() }
    }
    /// A chorus of marsh frogs at a lake or a slow river, dusk into the night.
    pub fn marsh_frogs() -> InsectConfig {
        InsectConfig { kind: InsectKind::MarshFrog, count: 6, min_temp_c: 12.0, active_from_h: 19.5, active_to_h: 2.0, spread: 0.12, gain: 1.6, ..Default::default() }
    }
    /// Tree frogs in the reeds and bushes by the water: loud, rapid, at night.
    pub fn tree_frogs() -> InsectConfig {
        InsectConfig { kind: InsectKind::TreeFrog, count: 4, min_temp_c: 12.0, active_from_h: 21.0, active_to_h: 3.0, spread: 0.1, synchrony: 0.3, gain: 1.4, ..Default::default() }
    }
    /// Common frogs purring softly in a pond (cool nights are enough for them).
    pub fn common_frogs() -> InsectConfig {
        InsectConfig { kind: InsectKind::CommonFrog, count: 5, min_temp_c: 5.0, active_from_h: 18.0, active_to_h: 5.0, spread: 0.12, gain: 1.0, ..Default::default() }
    }
    /// A great spotted woodpecker drumming on a dead branch, in the morning.
    pub fn woodpecker() -> InsectConfig {
        InsectConfig { kind: InsectKind::Woodpecker, count: 1, min_temp_c: -5.0, active_from_h: 6.0, active_to_h: 12.0, spread: 0.1, gain: 1.0, ..Default::default() }
    }

    pub fn preset(name: &str) -> Option<InsectConfig> {
        Some(match name {
            "field_cricket" => Self::field_cricket(),
            "tree_cricket" => Self::tree_cricket(),
            "bush_cricket" => Self::bush_cricket(),
            "grasshopper" => Self::grasshopper(),
            "cicada" => Self::cicada(),
            "marsh_frogs" => Self::marsh_frogs(),
            "tree_frogs" => Self::tree_frogs(),
            "common_frogs" => Self::common_frogs(),
            "woodpecker" => Self::woodpecker(),
            _ => return None,
        })
    }

    pub const PRESETS: &'static [&'static str] = &["field_cricket", "tree_cricket", "bush_cricket", "grasshopper", "cicada", "marsh_frogs", "tree_frogs", "common_frogs", "woodpecker"];

    fn update(&mut self, c: &Controls, len: usize) {
        let sr = self.sample_rate;
        let dt = len as f32 / sr;
        let e = &c.env;
        let cfg = &self.cfg;
        self.temp = e.temperature_c + cfg.temperature_offset_c;
        let warm = smoothstep(cfg.min_temp_c, cfg.min_temp_c + 2.5, self.temp);
        let hours = if (cfg.active_from_h - cfg.active_to_h).abs() < 1e-3 { 1.0 } else { hour_window(e.hour, cfg.active_from_h, cfg.active_to_h, 1.5) };
        let rain = (1.0 - 1.6 * rain_unit(e.rain_mm_h)).clamp(0.0, 1.0);
        let wind = 1.0 - smoothstep(6.0, 12.0, wind_speed_ms(c.gust).max(e.wind_ms));
        self.activity = warm * hours * rain * wind;
        let law = self.law;
        let carrier = if cfg.carrier_hz > 0.0 { cfg.carrier_hz } else { law.carrier_hz };
        let n = cfg.count as usize;

        // Mean chirp phase for synchrony (Kuramoto coupling).
        let (mut sx, mut sy) = (0.0f32, 0.0f32);
        for i in self.ind.iter().take(n).filter(|i| i.singing) {
            sx += (TAU * i.chirp).cos();
            sy += (TAU * i.chirp).sin();
        }
        let mean = sy.atan2(sx) / TAU;
        for (k, i) in self.ind.iter_mut().enumerate() {
            let active = k < n;
            // Bouts of song and rest; more song when conditions are good.
            i.bout -= dt;
            if i.bout <= 0.0 {
                let p = self.activity;
                i.singing = active && self.rng.uniform() < 0.25 + 0.75 * p && p > 0.02;
                i.bout = if i.singing { self.rng.range(4.0, 25.0) } else { self.rng.range(1.0, 8.0) * (1.5 - p) };
            }
            let target = if i.singing && active { self.activity } else { 0.0 };
            i.level.step(target, dt, 0.15);
            i.level_ramp.to(i.level.value, len);
            if cfg.synchrony > 0.0 && i.singing {
                let d = (mean - i.chirp + 0.5).rem_euclid(1.0) - 0.5;
                i.chirp = (i.chirp + cfg.synchrony * 1.5 * d * dt).rem_euclid(1.0);
            }
            let f = carrier * i.pitch_jit * (1.0 + 0.004 * (self.temp - 20.0));
            match cfg.kind {
                InsectKind::BushCricket | InsectKind::Grasshopper => i.band.set(f, law.q, sr),
                InsectKind::Cicada => i.res.tune(f, core::f32::consts::PI * f / law.q, sr),
                k if is_frog(k) => {
                    // Vocal sac: dominant formant and a weaker second one.
                    i.res.tune(f, core::f32::consts::PI * f / law.q, sr);
                    i.res2.tune(f * 2.1, core::f32::consts::PI * f * 2.1 / (0.7 * law.q), sr);
                }
                InsectKind::Woodpecker => {
                    // The branch: one strong mode and a higher, shorter one.
                    i.res.tune(f, core::f32::consts::PI * f / law.q, sr);
                    i.res2.tune(f * 2.7, core::f32::consts::PI * f * 2.7 / (0.5 * law.q), sr);
                    i.band.set(2500.0, 0.8, sr);
                }
                _ => {}
            }
        }
    }
}

impl Insects {
    /// One sample of a frog's call or a woodpecker's drum: calls (chirp clock) of `notes`
    /// notes (syl), each note a train of pulses (carrier_phase) ringing the resonators.
    #[allow(clippy::too_many_arguments)]
    fn call_sample(i: &mut Individual, kind: InsectKind, law: &SongLaw, calls_hz: f32, rate: f32, sr: f32, rng: &mut Rng, count: &mut u32) -> f32 {
        let drum = kind == InsectKind::Woodpecker;
        i.chirp += calls_hz * i.rate_jit / sr * if drum { 1.0 } else { rng.range(0.6, 1.4) };
        if i.chirp >= 1.0 {
            i.chirp -= 1.0;
            if i.singing && i.syl == u32::MAX {
                i.syl = 0;
                i.syl_phase = 0.0;
                i.carrier_phase = 1.0;
                // Calls vary: a short laugh or a long one, a drum of 10 or 16 strikes.
                let n = law.pulses_per_chirp as f32;
                i.notes = (n * rng.range(0.6, 1.35)).round().max(1.0) as u32;
                i.strength = rng.range(0.6, 1.0);
                *count += 1;
            }
        }
        if i.syl == u32::MAX {
            return i.res.tick() + i.res2.tick() + if drum { i.band.process(0.0).1 } else { 0.0 };
        }
        let (note_hz, note_len) = note_law(kind);
        let (period, active) = if drum {
            // Strikes slow down towards the end of the drum.
            let u = i.syl as f32 / i.notes.max(1) as f32;
            (1.0 / (law.pulse_hz * i.rate_jit * (1.0 - 0.25 * u * u)), 0.0)
        } else {
            (1.0 / (note_hz * rate * i.rate_jit), note_len / rate)
        };
        i.syl_phase += 1.0 / sr;
        let mut x = 0.0;
        if drum {
            if i.carrier_phase >= 1.0 {
                i.carrier_phase = 0.0;
                let u = i.syl as f32 / i.notes.max(1) as f32;
                let a = i.strength * (1.0 - 0.5 * u * u) * rng.range(0.8, 1.0);
                i.res.strike(a);
                i.res2.strike(0.4 * a * rng.range(0.5, 1.0));
                i.swell = a;
            }
            i.carrier_phase += 1.0 / (period * sr);
            // The dry knock of the bill: a few ms of bright noise.
            let knock = i.swell * rng.bipolar();
            i.swell *= 0.985;
            x += 0.6 * i.band.process(knock).1;
        } else if i.syl_phase < active {
            // Pulses of the vocal cords within the note, louder in the middle of it.
            i.carrier_phase += law.pulse_hz * rate * i.rate_jit / sr;
            if i.carrier_phase >= 1.0 {
                i.carrier_phase -= 1.0;
                let u = i.syl_phase / active;
                let env = (core::f32::consts::PI * u).sin().max(0.15);
                let a = i.strength * env * rng.range(0.7, 1.0);
                i.res.strike(a);
                i.res2.strike(0.45 * a);
            }
        }
        if i.syl_phase >= period {
            i.syl_phase -= period;
            i.syl += 1;
            if !drum {
                i.carrier_phase = 1.0;
                i.strength *= rng.range(0.85, 1.12);
            }
            if i.syl >= i.notes {
                i.syl = u32::MAX;
            }
        }
        x + i.res.tick() + i.res2.tick()
    }
}

impl Generator for Insects {
    fn set_model(&mut self, cfg: &super::ModelConfig) {
        if let super::ModelConfig::Insects(c) = cfg {
            if *c != self.cfg {
                self.set_config(*c);
            }
        }
    }

    fn render(&mut self, out: &mut [f32], c: &Controls) {
        let sr = self.sample_rate;
        let len = out.len();
        self.update(c, len);
        let n = self.cfg.count as usize;
        if self.ind.iter().take(n).all(|i| i.level.value < 1e-5 && i.level_ramp.cur < 1e-5) && self.activity < 1e-4 {
            out.fill(0.0);
            for i in self.ind.iter_mut() {
                i.level_ramp.settle(0.0);
            }
            return;
        }
        out.fill(0.0);
        let law = self.law;
        let kind = self.cfg.kind;
        let t = self.temp;
        let rate_factor = (1.0 + law.pulse_slope * (t - 20.0)).max(0.2);
        let pulse_hz = law.pulse_hz * rate_factor;
        let chirp_hz = chirps_per_min(kind, t) / 60.0;
        let carrier = if self.cfg.carrier_hz > 0.0 { self.cfg.carrier_hz } else { law.carrier_hz };
        let carrier = carrier * (1.0 + 0.004 * (t - 20.0));
        let g = self.cfg.gain * 0.6;
        for i in self.ind.iter_mut().take(n) {
            if i.level.value < 1e-5 && i.level_ramp.cur < 1e-5 {
                i.level_ramp.settle(0.0);
                continue;
            }
            let p_inc = pulse_hz * i.rate_jit / sr;
            let c_inc = chirp_hz * i.rate_jit / sr;
            let fc = carrier * i.pitch_jit;
            for o in out.iter_mut() {
                let lvl = i.level_ramp.next();
                let mut v = 0.0;
                match kind {
                    InsectKind::FieldCricket | InsectKind::TreeCricket | InsectKind::Grasshopper => {
                        i.chirp += c_inc;
                        if i.chirp >= 1.0 {
                            i.chirp -= 1.0;
                            if i.singing {
                                i.syl = 0;
                                i.syl_phase = 0.0;
                                self.chirps += 1;
                            }
                        }
                        if i.syl != u32::MAX {
                            i.syl_phase += p_inc;
                            if i.syl_phase >= 1.0 {
                                i.syl_phase -= 1.0;
                                i.syl += 1;
                                if i.syl >= law.pulses_per_chirp {
                                    i.syl = u32::MAX;
                                }
                            }
                        }
                        if i.syl != u32::MAX && i.syl_phase < 0.6 {
                            // Closing stroke: sin² envelope over 60 % of the syllable period.
                            let x = i.syl_phase / 0.6;
                            let env = (core::f32::consts::PI * x).sin().powi(2) * (0.8 + 0.2 * i.syl as f32 / law.pulses_per_chirp as f32);
                            if kind == InsectKind::Grasshopper {
                                let (_, bp, _) = i.band.process(self.rng.bipolar());
                                v = bp * env * 2.5;
                            } else {
                                // Escapement: pure tone, pitch sagging ~4 % over the stroke.
                                i.carrier_phase += fc * (1.0 - 0.04 * x) / sr;
                                if i.carrier_phase >= 1.0 {
                                    i.carrier_phase -= 1.0;
                                }
                                let ph = TAU * i.carrier_phase;
                                v = env * (ph.sin() + 0.08 * (2.0 * ph).sin());
                            }
                        }
                    }
                    InsectKind::BushCricket => {
                        // Continuous trains of double syllables, broadband scraping.
                        i.syl_phase += p_inc;
                        if i.syl_phase >= 1.0 {
                            i.syl_phase -= 1.0;
                        }
                        let x = i.syl_phase;
                        let env = if x < 0.35 { (core::f32::consts::PI * x / 0.35).sin() } else if (0.45..0.7).contains(&x) { 0.6 * (core::f32::consts::PI * (x - 0.45) / 0.25).sin() } else { 0.0 };
                        let (_, bp, _) = i.band.process(self.rng.bipolar());
                        v = bp * env * 2.0;
                    }
                    InsectKind::MarshFrog | InsectKind::TreeFrog | InsectKind::CommonFrog | InsectKind::Woodpecker => {
                        v = Self::call_sample(i, kind, &law, chirp_hz, rate_factor, sr, &mut self.rng, &mut self.chirps);
                    }
                    InsectKind::Cicada => {
                        // Tymbal clicks ring the abdominal resonator; the two tymbals alternate.
                        i.syl_phase += p_inc * 0.5;
                        if i.syl_phase >= 0.5 {
                            i.syl_phase -= 0.5;
                            i.tymbal = !i.tymbal;
                            i.res.strike(if i.tymbal { 1.0 } else { 0.8 } * self.rng.range(0.85, 1.0));
                        }
                        i.swell = (i.swell + 0.25 / sr) % 1.0;
                        v = i.res.tick() * 1.0 * (0.75 + 0.25 * (TAU * i.swell).sin());
                    }
                }
                *o += v * lvl * g;
            }
            let l = i.level_ramp.cur;
            i.level_ramp.settle(l);
            i.band.flush();
        }
    }

    fn pitch(&self) -> f32 {
        if self.activity > 0.05 {
            if self.cfg.carrier_hz > 0.0 {
                self.cfg.carrier_hz
            } else {
                self.law.carrier_hz
            }
        } else {
            0.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::Ambient;

    fn chirps_in(cfg: InsectConfig, t_c: f32, seconds: f32) -> f32 {
        let mut ins = Insects::new(48_000.0, 5, InsectConfig { count: 1, spread: 0.0, ..cfg });
        // Force the individual to sing throughout.
        ins.ind[0].singing = true;
        ins.ind[0].bout = 1e9;
        let c = Controls { env: Ambient { temperature_c: t_c, hour: 23.0, ..Default::default() }, ..Default::default() };
        let mut buf = [0.0f32; 64];
        let blocks = (seconds * 48_000.0 / 64.0) as usize;
        for _ in 0..blocks {
            ins.render(&mut buf, &c);
            assert!(buf.iter().all(|v| v.is_finite()));
        }
        ins.chirps() as f32 * 60.0 / seconds
    }

    #[test]
    fn dolbear_law() {
        for t in [14.0f32, 20.0, 26.0] {
            let n = chirps_in(Insects::tree_cricket(), t, 60.0);
            let want = 7.2 * t - 32.0;
            assert!((n - want).abs() <= want * 0.05 + 1.0, "T {t}: {n}/min, Dolbear {want}");
        }
        // Crickets slow down as it cools, and fall silent in the cold.
        assert!(chirps_in(Insects::field_cricket(), 15.0, 30.0) < chirps_in(Insects::field_cricket(), 25.0, 30.0));
        let mut ins = Insects::new(48_000.0, 1, Insects::field_cricket());
        let cold = Controls { env: Ambient { temperature_c: 6.0, hour: 23.0, ..Default::default() }, ..Default::default() };
        let mut buf = [0.0f32; 64];
        let mut e = 0.0;
        for _ in 0..3000 {
            ins.render(&mut buf, &cold);
            e += buf.iter().map(|v| v * v).sum::<f32>();
        }
        assert_eq!(e, 0.0);
    }

    fn energy(cfg: InsectConfig, hour: f32, t_c: f32, seconds: f32) -> (f32, u32) {
        let mut ins = Insects::new(48_000.0, 9, cfg);
        let c = Controls { env: Ambient { temperature_c: t_c, hour, ..Default::default() }, ..Default::default() };
        let mut buf = [0.0f32; 64];
        let mut e = 0.0f32;
        for _ in 0..(seconds * 48_000.0 / 64.0) as usize {
            ins.render(&mut buf, &c);
            assert!(buf.iter().all(|v| v.is_finite()));
            e += buf.iter().map(|v| v * v).sum::<f32>();
        }
        (e, ins.chirps())
    }

    #[test]
    fn frogs_call_at_night_and_woodpeckers_drum_in_the_morning() {
        for cfg in [Insects::marsh_frogs(), Insects::tree_frogs(), Insects::common_frogs()] {
            let (night, calls) = energy(cfg, 23.0, 18.0, 60.0);
            let (noon, _) = energy(cfg, 13.0, 18.0, 20.0);
            assert!(night > 1.0 && calls > 3, "{:?}: {night} {calls}", cfg.kind);
            assert_eq!(noon, 0.0, "{:?} silent at noon", cfg.kind);
        }
        let (morning, drums) = energy(Insects::woodpecker(), 8.0, 10.0, 120.0);
        assert!(morning > 0.1 && (1..=8).contains(&drums), "woodpecker {morning} {drums}");
    }
}
