//! Weather state: turns the few weather inputs (wind, rain rate, temperature, humidity,
//! storm) into the slowly evolving quantities the physical models need: smoothed and
//! fluctuating rain, wet leaves, canopy dripping that outlasts the rain, wet ground and
//! puddles, and lightning strikes.
//!
//! * Rain onset/stop is smoothed (seconds) and fluctuates like passing rain cells (OU, ~20 s).
//! * Canopy water store C (mm) after Rutter et al. (1971):
//!   dC/dt = (1−p)·R − D − E, free throughfall p = 0.25, capacity S ≈ 1.2 mm,
//!   drainage D = D_s·e^{b(C−S)} (D_s = 0.12 mm/h, b = 3.7 mm⁻¹) plus wind shaking
//!   ∝ C·(U/5)², evaporation E = E_pot·C/S with E_pot = 0.45·VPD·(1 + 0.5·U) mm/h.
//!   Drainage is what drips: it rises fast in rain and tails off for a long time after.
//! * Ground: a surface water store fed by rain, drained by infiltration (~4 mm/h) and
//!   evaporation, plus slower soil moisture; puddles form when the store exceeds ~1 mm.
//! * Lightning: Poisson flashes at up to 4/min with storm activity, around a storm centre
//!   that drifts in bearing and comes closer as the storm grows: storm 1 → ~0.6–1.5 km
//!   (thunder 2–4 s after the flash), storm 0.5 → ~2–4 km, a weak storm ~5–10 km (lognormal
//!   spread, the centre wandering slowly), with an occasional close strike (250–600 m) in a
//!   severe storm (storm > 0.7; the default direction stays below). About
//!   a third are intracloud flashes (diffuse light, rumble only), the rest cloud-to-ground
//!   with 1–5 return strokes. The flash brightness follows those strokes
//!   ([`Flash::brightness`]: leader glow, the flicker of the strokes, dimmer and smeared with
//!   distance); the director points the thunder emitter at the strike.
//!
//! `time_scale` speeds up only the water stores (canopy, ground), so demos can show an hour
//! of drying in a minute.

use crate::rng::Rng;
use crate::scene::{EnvironmentDesc, Season};
use crate::sources::common::{rain_unit, vapour_pressure_kpa, wind_speed_ms, Ou, Smooth};
use crate::sources::thunder::{random_strokes, Flash};
use crate::sources::Ambient;

use super::Environment;

const THROUGHFALL: f32 = 0.25;
const CAPACITY_MM: f32 = 1.2;
const DRAIN_DS: f32 = 0.12;
const DRAIN_B: f32 = 3.7;
const INFILTRATION_MM_H: f32 = 4.0;

pub struct Weather {
    rng: Rng,
    rain: Smooth,
    rain_ou: Ou,
    rain_now: f32,
    canopy_mm: f32,
    drip_mm_h: f32,
    ground_mm: f32,
    soil: f32,
    time_scale: f32,
    dryness: f32,
    leaf_cover: f32,
    temperature: f32,
    humidity: f32,
    wind_ms: f32,
    storm: f32,
    strike: u32,
    strike_distance: f32,
    strokes: u8,
    struck: bool,
    next_strike: f32,
    bearing: f32,
    bearing_ou: Ou,
    /// Slow wander of the storm centre's (log) distance.
    centre_ou: Ou,
    flash: f32,
    light: Flash,
    /// Seconds since the latest flash started.
    age: f32,
}

impl Weather {
    pub fn new(env: &EnvironmentDesc, seed: u64) -> Self {
        let mut rng = Rng::new(seed);
        let wet = env.wetness.clamp(0.0, 1.0);
        let (dryness, leaf_cover) = season_leaves(env.season);
        let rain0 = env.initial_rain_mm_h();
        let bearing = rng.range(0.0, core::f32::consts::TAU);
        Self {
            next_strike: rng.range(4.0, 15.0),
            rng,
            rain: Smooth::new(rain0),
            rain_ou: Ou::default(),
            rain_now: rain0,
            canopy_mm: wet * CAPACITY_MM,
            drip_mm_h: 0.0,
            ground_mm: wet * 1.5,
            soil: wet,
            time_scale: env.weather_time_scale.clamp(0.0, 1000.0),
            dryness,
            leaf_cover,
            temperature: env.temperature_c,
            humidity: env.humidity,
            wind_ms: 0.0,
            storm: env.storm,
            strike: 0,
            strike_distance: 0.0,
            strokes: 0,
            struck: false,
            bearing,
            bearing_ou: Ou::default(),
            centre_ou: Ou::default(),
            flash: 0.0,
            light: Flash::default(),
            age: 1e3,
        }
    }

    /// Seasonal leaf state (dryness, deciduous leaf cover).
    pub fn set_season(&mut self, season: Season) {
        (self.dryness, self.leaf_cover) = season_leaves(season);
    }

    pub fn set_time_scale(&mut self, s: f32) {
        self.time_scale = s.clamp(0.0, 1000.0);
    }

    /// Rain rate right now in the open, mm/h.
    pub fn rain_mm_h(&self) -> f32 {
        self.rain_now
    }

    pub fn leaf_wetness(&self) -> f32 {
        (self.canopy_mm / CAPACITY_MM).clamp(0.0, 1.0)
    }

    pub fn drip_mm_h(&self) -> f32 {
        self.drip_mm_h
    }

    /// Lightning flash brightness 0..1 for the UI: the flicker of the leader and the return
    /// strokes (or the diffuse surges of an intracloud flash), dimmer with distance.
    pub fn flash(&self) -> f32 {
        self.flash
    }

    /// True during the step in which lightning struck.
    pub fn struck(&self) -> bool {
        self.struck
    }

    /// Bearing of the latest strike (radians, 0 = +x forward, +π/2 = left) and distance (m).
    pub fn strike_position(&self) -> (f32, f32) {
        (self.bearing, self.strike_distance)
    }

    /// The latest flash's timing (strokes) and seconds since it started.
    pub fn last_flash(&self) -> (Flash, f32) {
        (self.light, self.age)
    }

    /// Median strike distance at storm activity `storm` (0..1), m: the storm centre.
    pub fn storm_centre_m(storm: f32) -> f32 {
        1000.0 * (2.2 * (1.0 - storm.clamp(0.0, 1.0))).exp()
    }

    pub fn step(&mut self, dt: f32, env: &Environment) {
        self.temperature = env.temperature_c;
        self.humidity = env.humidity.clamp(0.0, 1.0);
        self.wind_ms = wind_speed_ms(env.wind);
        self.storm = env.storm.clamp(0.0, 1.0);

        // Rain: smooth onset (faster) and stop (slower), with rain-cell fluctuation.
        let target = env.rain_mm_h.max(0.0);
        let tau = if target > self.rain.value { 5.0 } else { 4.0 };
        let base = self.rain.step(target, dt, tau);
        let cell = self.rain_ou.step(dt, 20.0, 0.3, &mut self.rng);
        self.rain_now = if base < 0.02 { 0.0 } else { base * (1.0 + cell).clamp(0.3, 2.0) };

        // Water stores (optionally accelerated).
        let h = dt * self.time_scale / 3600.0;
        let r = self.rain_now;
        let raining = smoothstep01(r / 0.2);
        let rh = self.humidity.max(0.97 * raining);
        let vpd = vapour_pressure_kpa(self.temperature) * (1.0 - rh);
        let e_pot = 0.45 * vpd * (1.0 + 0.5 * self.wind_ms);
        let c = self.canopy_mm;
        let wet_gate = smoothstep01(c / (0.05 * CAPACITY_MM));
        let drain = (DRAIN_DS * (DRAIN_B * (c - CAPACITY_MM)).exp() + 0.5 * c * (self.wind_ms / 5.0).powi(2)) * wet_gate;
        let evap = e_pot * (c / CAPACITY_MM).min(1.0);
        self.canopy_mm = (c + ((1.0 - THROUGHFALL) * r - drain - evap) * h).clamp(0.0, 4.0 * CAPACITY_MM);
        self.drip_mm_h = drain;

        let g = self.ground_mm;
        let infil = INFILTRATION_MM_H * (1.0 - 0.6 * self.soil) * smoothstep01(g / 0.2);
        let g_evap = 0.5 * e_pot * smoothstep01(g);
        self.ground_mm = (g + (r + 0.3 * drain - infil - g_evap) * h).clamp(0.0, 40.0);
        self.soil = (self.soil + (infil / 20.0 - self.soil * 0.03 - 0.02 * e_pot * self.soil) * h).clamp(0.0, 1.0);

        // Lightning.
        self.struck = false;
        self.age += dt;
        self.flash = self.light.brightness(self.age, self.strike_distance);
        if self.storm > 0.0 {
            let b = self.bearing_ou.step(dt, 120.0, 0.6, &mut self.rng);
            let wander = self.centre_ou.step(dt, 90.0, 0.25, &mut self.rng);
            self.next_strike -= dt * self.storm;
            if self.next_strike <= 0.0 {
                self.next_strike = 15.0 * crate::sources::common::exp1(&mut self.rng);
                let centre = Self::storm_centre_m(self.storm) * wander.exp();
                // A strike within a few hundred metres (the tearing crack) belongs to a severe
                // storm only: one the listener asked for (direction.weather), not the default
                // course of a focus soundscape (docs/comfort.md).
                let close = self.rng.uniform() < 0.07 * crate::sources::common::smoothstep(0.7, 1.0, self.storm);
                let d = if close { self.rng.range(250.0, 600.0) } else { centre * (0.4 * self.rng.gauss()).exp() };
                self.strike_distance = d.clamp(200.0, 25_000.0);
                self.bearing = (b + 0.4 * self.rng.gauss()).rem_euclid(core::f32::consts::TAU);
                self.strike = self.strike.wrapping_add(1);
                // About a third are intracloud flashes (a close strike is to the ground).
                self.strokes = if !close && self.rng.uniform() < 0.33 { 0 } else { random_strokes(&mut self.rng) };
                self.light = Flash::new(self.strike, self.strokes);
                self.age = 0.0;
                self.struck = true;
                self.flash = self.light.brightness(0.0, self.strike_distance);
            }
        }
    }

    pub fn ambient(&self, env: &Environment) -> Ambient {
        let ground = (1.0 - (-self.ground_mm / 0.3).exp()).max(self.soil);
        Ambient {
            wind_ms: self.wind_ms,
            rain_mm_h: self.rain_now,
            temperature_c: self.temperature,
            humidity: self.humidity,
            leaf_wetness: self.leaf_wetness(),
            ground_wetness: ground.clamp(0.0, 1.0),
            puddles: smoothstep01((self.ground_mm - 1.0) / 5.0) * 0.6,
            drip_mm_h: self.drip_mm_h,
            hour: env.hour,
            dryness: self.dryness,
            leaf_cover: self.leaf_cover,
            storm: self.storm,
            strike: self.strike,
            strike_distance_m: self.strike_distance,
            strike_strokes: self.strokes,
            strike_bearing: self.bearing,
        }
    }

    /// Rain on the 0..1 scale the bird agents react to.
    pub fn rain_unit(&self) -> f32 {
        rain_unit(self.rain_now)
    }
}

fn season_leaves(s: Season) -> (f32, f32) {
    match s {
        Season::Spring => (0.0, 0.8),
        Season::Summer => (0.1, 1.0),
        Season::Autumn => (0.85, 0.75),
        Season::Winter => (1.0, 0.05),
    }
}

fn smoothstep01(x: f32) -> f32 {
    let t = x.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canopy_drips_after_rain_stops_then_dries() {
        let desc = EnvironmentDesc::default();
        let mut w = Weather::new(&desc, 1);
        w.set_time_scale(60.0);
        let dt = 64.0 / 48_000.0;
        let mut env = Environment { rain_mm_h: 6.0, wind: 0.2, ..Default::default() };
        let mut t = 0.0;
        while t < 60.0 {
            w.step(dt, &env);
            t += dt;
        }
        assert!(w.leaf_wetness() > 0.9, "{}", w.leaf_wetness());
        let raining_drip = w.drip_mm_h();
        assert!(raining_drip > 1.0, "{raining_drip}");
        env.rain_mm_h = 0.0;
        // A minute after the rain (one simulated hour), it still drips, but less.
        while t < 120.0 {
            w.step(dt, &env);
            t += dt;
        }
        assert_eq!(w.rain_mm_h(), 0.0);
        let after = w.drip_mm_h();
        assert!(after > 0.005 && after < raining_drip, "{after}");
        // Much later it has stopped dripping.
        w.set_time_scale(1000.0);
        while t < 300.0 {
            w.step(dt, &env);
            t += dt;
        }
        assert!(w.drip_mm_h() < 0.2 * after, "{}", w.drip_mm_h());
    }

    #[test]
    fn storm_strikes_lightning() {
        let mut w = Weather::new(&EnvironmentDesc::default(), 2);
        let env = Environment { storm: 1.0, ..Default::default() };
        let mut strikes = 0;
        for _ in 0..(120.0 * 750.0) as usize {
            w.step(1.0 / 750.0, &env);
            if w.struck() {
                strikes += 1;
                let (_, d) = w.strike_position();
                assert!((300.0..=25_000.0).contains(&d));
            }
        }
        // 3 per minute on average.
        assert!((2..=14).contains(&strikes), "{strikes}");
    }

    /// A strong storm is close (thunder a few seconds after the flash), a moderate one a few
    /// kilometres away.
    #[test]
    fn stronger_storm_strikes_closer() {
        let median = |storm: f32| {
            let mut w = Weather::new(&EnvironmentDesc::default(), 5);
            let env = Environment { storm, ..Default::default() };
            let mut d = Vec::new();
            let dt = 1.0 / 100.0;
            while d.len() < 200 {
                w.step(dt, &env);
                if w.struck() {
                    d.push(w.strike_position().1);
                }
            }
            d.sort_by(|a, b| a.partial_cmp(b).unwrap());
            (d[100], d[20], d[180])
        };
        let (m1, lo1, hi1) = median(1.0);
        assert!((600.0..1500.0).contains(&m1) && lo1 > 250.0 && hi1 < 3000.0, "{m1} {lo1} {hi1}");
        let (m5, _, _) = median(0.5);
        assert!((2000.0..4500.0).contains(&m5), "{m5}");
    }

    /// The flash flickers with its return strokes, then goes dark well before the sound.
    #[test]
    fn flash_follows_the_strokes() {
        let mut w = Weather::new(&EnvironmentDesc::default(), 3);
        let env = Environment { storm: 1.0, ..Default::default() };
        let dt = 64.0 / 48_000.0;
        let mut seen = 0;
        for _ in 0..(200.0 / dt) as usize {
            w.step(dt, &env);
            if !w.struck() {
                continue;
            }
            let (flash, _) = w.last_flash();
            let d = w.strike_position().1;
            let mut peak = 0.0f32;
            let mut rises = 0;
            let (mut prev, mut falling) = (w.flash(), false);
            // Until the thunder arrives (≥ 200 m: ≥ 0.6 s), at least a second.
            for _ in 0..((d / 340.0).max(1.0) / dt) as usize {
                w.step(dt, &Environment { storm: 0.0, ..env });
                let v = w.flash();
                peak = peak.max(v);
                if v > prev + 1e-4 && falling {
                    rises += 1;
                }
                falling = v < prev - 1e-5;
                prev = v;
            }
            assert!(peak > 0.0 && peak <= 1.0);
            if !flash.cloud && flash.n >= 2 && d < 2000.0 {
                assert!(rises + 1 >= flash.n, "{rises} rises for {} strokes", flash.n);
                assert!(peak > 0.5, "near strike {d} m: {peak}");
            }
            // Dark again before the thunder arrives.
            assert!(w.flash() < 0.02, "{} at {d} m", w.flash());
            seen += 1;
        }
        assert!(seen > 3);
    }
}
