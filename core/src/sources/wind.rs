//! Wind: the sound of moving air meeting obstacles. There is no sound of "air" on its own;
//! everything heard is flow interacting with something.
//!
//! * **Body**: turbulent vortex shedding from every twig, branch and trunk. A cylinder of
//!   diameter d sheds at f = St·U/d (St ≈ 0.2) and radiates as a dipole, intensity ∝ U⁶
//!   (Phillips 1956). Vegetation spans a wide range of diameters; with Leonardo's rule
//!   (N(d) ∝ d⁻², length ∝ d) the total shedding length per log-diameter band times d is
//!   roughly constant, so each band contributes about equally: the familiar broad roar.
//!   The model sums `BANDS` noise bands at St·U/dₖ for log-spaced dₖ in [d_min, d_max],
//!   broadened by turbulence (low Q), gated by Re > 40 and by audibility. Flexible
//!   vegetation reconfigures (leaves fold, drag ∝ U^1.3 instead of U², Vogel 1989), so the
//!   default level law is I ∝ U^4.6, not U⁶; `level_exponent` sets it.
//! * **Aeolian tones** from wires: coherent shedding locks onto the wire's transverse modes
//!   fₙ = n·f₁ ("aeolian harp"); the mode nearest St·U/d sings, and as the wind changes the
//!   tone jumps between harmonics. Long power lines (f₁ ≈ 1 Hz) give a beating hum that just
//!   follows St·U/d.
//! * **Gaps and hollows** (cavity tones): the shear layer over an opening of length L
//!   oscillates at f ≈ 0.57·(m − ¼)·U/L (Rossiter modes); when a mode meets the cavity's
//!   Helmholtz resonance the gap whistles or howls, so the howl comes and goes with gusts.
//! * **Distance**: far wind is the average of a large area (smoother gusts) and loses its
//!   highs to air absorption (only the roar remains).
//! * **Not one filter sweep.** A source is an extent of vegetation, not a point, and the
//!   wind in it is not one number: each diameter band has its own place in the source and
//!   its own turbulence. Band k hears the arriving gust after its own lag inside the source
//!   (extent ≈ distance/4, 2..40 m, crossed at the gust-front speed: frozen turbulence via
//!   [`GustTrail`]) and adds local turbulence whose time scale follows the element: twigs
//!   answer the small, fast eddies (≈0.15 s), limbs and trunks only the large slow ones
//!   (≈2 s). So the bands rise and fall at different moments, the spectral centre wanders
//!   with the local speeds, and the sum over the extent averages the gusts the way a real
//!   crown does. Far sources also breathe slowly (gust fronts crossing a forest, τ ≈ 7 s).
//!
//! Local wind speed comes from the travelling gust field (`Controls::gust`, mapped to m/s by
//! [`wind_speed_ms`]), times the source's exposure.

use core::f32::consts::TAU;

use serde::{Deserialize, Serialize};

use super::common::*;
use super::rain::check;
use super::{Controls, Generator};
use crate::rng::Rng;

const BANDS: usize = 8;
const WIRE_SLOTS: usize = 4;
/// Reference speed for levels, m/s.
const U_REF: f32 = 5.0;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct WindConfig {
    /// Wind speed at this source relative to the canopy-height mean (0..2.5): sheltered
    /// hollow 0.3, forest interior 0.7, open ridge 1.5.
    pub exposure: f32,
    /// Smallest and largest obstacle diameters, m (0.0003..1.0): needles/twigs .. trunks.
    pub d_min_m: f32,
    pub d_max_m: f32,
    /// Strouhal number (0.12..0.3).
    pub strouhal: f32,
    /// Intensity ∝ U^n (3..8): 6 rigid dipoles, ~4.6 flexible vegetation.
    pub level_exponent: f32,
    /// Amount of broadband body (0..3).
    pub body: f32,
    /// Spectral tilt across the diameter bands, dB per doubling of diameter (−9..9).
    pub tilt_db: f32,
    /// Wire diameter, m (0 = no wire; 0.0005..0.05).
    pub wire_diameter_m: f32,
    /// First transverse mode of the wire, Hz (0.3..300): power line span ~1, fence wire ~40.
    pub wire_fundamental_hz: f32,
    /// Wire tone level (0..3).
    pub wires: f32,
    /// Helmholtz resonance of a gap or hollow, Hz (0 = none; 40..4000).
    pub gap_hz: f32,
    /// Length of the opening along the flow, m (0.005..3).
    pub gap_length_m: f32,
    /// Sharpness of the gap resonance (3..80).
    pub gap_q: f32,
    /// Gap tone level (0..3).
    pub gap: f32,
    /// Distance to the bulk of the wind source, m (1..3000).
    pub distance_m: f32,
    /// How strongly the source follows gusts (0 = mean wind only, 1 = physical, up to 2).
    pub gust_response: f32,
    /// Wind-blown grains (0..3): sand or snow hopping over the surface (saltation) once the
    /// wind exceeds `sand_threshold_ms`; a hiss of countless tiny impacts.
    pub sand: f32,
    /// Centre of the grain hiss, Hz (500..12000): sand ~6 kHz, snow softer.
    pub sand_hz: f32,
    /// Wind speed at which grains start to move, m/s (2..12): dry sand ~5, loose snow ~4.
    pub sand_threshold_ms: f32,
    /// Linear output gain (0..8).
    pub gain: f32,
}

impl Default for WindConfig {
    fn default() -> Self {
        Self {
            exposure: 1.0,
            d_min_m: 0.002,
            d_max_m: 0.15,
            strouhal: 0.2,
            level_exponent: 4.6,
            body: 1.0,
            tilt_db: 0.0,
            wire_diameter_m: 0.0,
            wire_fundamental_hz: 1.0,
            wires: 0.0,
            gap_hz: 0.0,
            gap_length_m: 0.1,
            gap_q: 25.0,
            gap: 0.0,
            distance_m: 10.0,
            gust_response: 1.0,
            sand: 0.0,
            sand_hz: 6000.0,
            sand_threshold_ms: 5.0,
            gain: 1.0,
        }
    }
}

impl WindConfig {
    pub fn validate(&self) -> Result<(), String> {
        check("exposure", self.exposure, 0.0, 2.5)?;
        check("d_min_m", self.d_min_m, 0.0003, 1.0)?;
        check("d_max_m", self.d_max_m, self.d_min_m, 1.0)?;
        check("strouhal", self.strouhal, 0.12, 0.3)?;
        check("level_exponent", self.level_exponent, 3.0, 8.0)?;
        check("body", self.body, 0.0, 3.0)?;
        check("tilt_db", self.tilt_db, -9.0, 9.0)?;
        if self.wire_diameter_m != 0.0 {
            check("wire_diameter_m", self.wire_diameter_m, 0.0005, 0.05)?;
        }
        check("wire_fundamental_hz", self.wire_fundamental_hz, 0.3, 300.0)?;
        check("wires", self.wires, 0.0, 3.0)?;
        if self.gap_hz != 0.0 {
            check("gap_hz", self.gap_hz, 40.0, 4000.0)?;
        }
        check("gap_length_m", self.gap_length_m, 0.005, 3.0)?;
        check("gap_q", self.gap_q, 3.0, 80.0)?;
        check("gap", self.gap, 0.0, 3.0)?;
        check("distance_m", self.distance_m, 1.0, 3000.0)?;
        check("gust_response", self.gust_response, 0.0, 2.0)?;
        check("sand", self.sand, 0.0, 3.0)?;
        check("sand_hz", self.sand_hz, 500.0, 12_000.0)?;
        check("sand_threshold_ms", self.sand_threshold_ms, 2.0, 12.0)?;
        check("gain", self.gain, 0.0, 8.0)
    }
}

#[derive(Clone, Copy, Default)]
struct WireSlot {
    n: u32,
    phase: f32,
    amp: Ramp,
    target: f32,
}

pub struct Wind {
    sample_rate: f32,
    rng: Rng,
    cfg: WindConfig,
    gust: Smooth,
    trail: GustTrail,
    /// Per diameter band: gust smoothing, local turbulence, lag inside the source (0..1 of
    /// the extent).
    band_gust: [Smooth; BANDS],
    band_turb: [Ou; BANDS],
    lag: [f32; BANDS],
    breath: Ou,
    u: f32,
    bands: [Svf; BANDS],
    band_gain: [Ramp; BANDS],
    lp1: f32,
    lp2: f32,
    lp_coef: f32,
    turb: Ou,
    wire_ou: Ou,
    slots: [WireSlot; WIRE_SLOTS],
    gap1: Svf,
    gap2: Svf,
    gap_gain: Ramp,
    gap_freq: f32,
    gap_target: f32,
    tone_hz: f32,
    started: bool,
    sand_band: Svf,
    sand_gain: Ramp,
    sand_env: f32,
    sand_coef: f32,
}

impl Wind {
    pub fn new(sample_rate: f32, seed: u64, cfg: WindConfig) -> Self {
        let sr = sample_rate;
        let mut rng = Rng::new(seed);
        // Bands sit at scattered places in the source (stratified, shuffled).
        let mut lag = [0.0f32; BANDS];
        for (k, l) in lag.iter_mut().enumerate() {
            *l = (k as f32 + rng.uniform()) / BANDS as f32;
        }
        for k in (1..BANDS).rev() {
            let j = (rng.uniform() * (k + 1) as f32) as usize % (k + 1);
            lag.swap(k, j);
        }
        Self {
            sample_rate,
            rng,
            gust: Smooth::new(0.0),
            trail: GustTrail::default(),
            band_gust: [Smooth::new(0.0); BANDS],
            band_turb: [Ou::default(); BANDS],
            lag,
            breath: Ou::default(),
            u: 0.0,
            bands: [Svf::new(200.0, 1.5, sr); BANDS],
            band_gain: [Ramp::new(0.0); BANDS],
            lp1: 0.0,
            lp2: 0.0,
            lp_coef: 1.0,
            turb: Ou::default(),
            wire_ou: Ou::default(),
            slots: [WireSlot::default(); WIRE_SLOTS],
            gap1: Svf::new(cfg.gap_hz.max(40.0), cfg.gap_q, sr),
            gap2: Svf::new(cfg.gap_hz.max(40.0), cfg.gap_q, sr),
            gap_gain: Ramp::new(0.0),
            gap_freq: cfg.gap_hz,
            gap_target: 0.0,
            started: false,
            tone_hz: 0.0,
            sand_band: Svf::new(cfg.sand_hz, 0.7, sr),
            sand_gain: Ramp::new(0.0),
            sand_env: 0.0,
            sand_coef: one_pole_coef(25.0, sr),
            cfg,
        }
    }

    pub fn config(&self) -> &WindConfig {
        &self.cfg
    }

    /// Change parameters at run time; all levels and frequencies glide.
    pub fn set_config(&mut self, cfg: WindConfig) {
        self.cfg = cfg;
    }

    /// Wind speed at the source right now, m/s.
    pub fn speed(&self) -> f32 {
        self.u
    }

    /// Open-field breeze close by: grass, shrubs, a few branches.
    pub fn breeze() -> WindConfig {
        WindConfig { d_min_m: 0.001, d_max_m: 0.05, tilt_db: -2.0, distance_m: 8.0, ..Default::default() }
    }
    /// The roar of a whole forest some distance away.
    pub fn forest_roar() -> WindConfig {
        WindConfig { d_min_m: 0.001, d_max_m: 0.4, body: 1.6, tilt_db: 2.0, distance_m: 150.0, gust_response: 0.7, ..Default::default() }
    }
    /// Storm through an exposed edge: strong body, a hollow howling in the gusts.
    pub fn storm() -> WindConfig {
        WindConfig {
            exposure: 1.3,
            d_min_m: 0.001,
            d_max_m: 0.3,
            body: 1.5,
            gap_hz: 180.0,
            gap_length_m: 0.35,
            gap_q: 18.0,
            gap: 0.8,
            distance_m: 25.0,
            gust_response: 1.3,
            ..Default::default()
        }
    }
    /// Overhead power lines: low singing hum with slow beats.
    pub fn power_lines() -> WindConfig {
        WindConfig { body: 0.3, wire_diameter_m: 0.02, wire_fundamental_hz: 0.8, wires: 1.2, distance_m: 15.0, ..Default::default() }
    }
    /// A taut fence or guy wire: an aeolian harp jumping between harmonics.
    pub fn fence_wire() -> WindConfig {
        WindConfig { body: 0.2, wire_diameter_m: 0.003, wire_fundamental_hz: 45.0, wires: 1.0, distance_m: 4.0, ..Default::default() }
    }
    /// Wind howling through a gap between rocks or boards.
    pub fn gap_howl() -> WindConfig {
        WindConfig { body: 0.5, gap_hz: 240.0, gap_length_m: 0.25, gap_q: 22.0, gap: 1.2, distance_m: 6.0, gust_response: 1.3, ..Default::default() }
    }
    /// A hollow trunk or knot hole: low hooting resonance.
    pub fn hollow_tree() -> WindConfig {
        WindConfig { body: 0.4, gap_hz: 110.0, gap_length_m: 0.18, gap_q: 30.0, gap: 1.0, distance_m: 5.0, ..Default::default() }
    }
    /// A narrow slit: thin whistle.
    pub fn whistle() -> WindConfig {
        WindConfig { body: 0.3, gap_hz: 950.0, gap_length_m: 0.012, gap_q: 40.0, gap: 0.8, distance_m: 3.0, ..Default::default() }
    }

    /// Wind over open dunes: little to shed on but tufts of grass, and sand hissing over the
    /// crests once the wind passes ~5 m/s.
    pub fn dune_sand() -> WindConfig {
        WindConfig { exposure: 1.4, d_min_m: 0.0005, d_max_m: 0.02, body: 0.35, tilt_db: -3.0, sand: 1.2, sand_hz: 6000.0, sand_threshold_ms: 5.0, distance_m: 12.0, ..Default::default() }
    }
    /// Wind over a rocky crest: rough body from the crags and a low howl through the gaps,
    /// coming and going with the gusts.
    pub fn ridge_howl() -> WindConfig {
        WindConfig { exposure: 1.3, d_min_m: 0.005, d_max_m: 0.6, body: 0.6, tilt_db: 2.0, gap_hz: 140.0, gap_length_m: 0.6, gap_q: 14.0, gap: 1.0, distance_m: 20.0, gust_response: 1.5, ..Default::default() }
    }
    /// Snow drifting off a crest: a soft hiss of grains over the snow cover.
    pub fn blowing_snow() -> WindConfig {
        WindConfig { exposure: 1.2, d_min_m: 0.001, d_max_m: 0.05, body: 0.3, sand: 0.9, sand_hz: 4200.0, sand_threshold_ms: 4.0, distance_m: 15.0, gust_response: 1.3, ..Default::default() }
    }

    pub fn preset(name: &str) -> Option<WindConfig> {
        Some(match name {
            "breeze" => Self::breeze(),
            "forest_roar" => Self::forest_roar(),
            "storm" => Self::storm(),
            "power_lines" => Self::power_lines(),
            "fence_wire" => Self::fence_wire(),
            "gap_howl" => Self::gap_howl(),
            "hollow_tree" => Self::hollow_tree(),
            "whistle" => Self::whistle(),
            "dune_sand" => Self::dune_sand(),
            "ridge_howl" => Self::ridge_howl(),
            "blowing_snow" => Self::blowing_snow(),
            _ => return None,
        })
    }

    pub const PRESETS: &'static [&'static str] =
        &["breeze", "forest_roar", "storm", "power_lines", "fence_wire", "gap_howl", "hollow_tree", "whistle", "dune_sand", "ridge_howl", "blowing_snow"];

    /// Wire shedding frequency for wind speed `u` (m/s).
    pub fn wire_shedding_hz(&self, u: f32) -> f32 {
        strouhal_hz(self.cfg.strouhal, u, self.cfg.wire_diameter_m.max(1e-4))
    }

    fn update_wires(&mut self, u: f32, len: usize, dt: f32) {
        let c = &self.cfg;
        let d = c.wire_diameter_m;
        let mut want = [(0u32, 0.0f32); 3];
        if d > 0.0 && c.wires > 0.0 {
            let fs = strouhal_hz(c.strouhal, u, d);
            let re = reynolds(u, d);
            let coherent = smoothstep(40.0, 150.0, re) * (1.0 - smoothstep(1e5, 3e5, re));
            let audible = smoothstep(25.0, 60.0, fs) * (1.0 - smoothstep(9000.0, 14_000.0, fs));
            // Shedding coherence wanders with small-scale turbulence (singing comes and goes).
            let wander = (0.75 + 0.35 * self.wire_ou.step(dt, 1.5, 0.6, &mut self.rng)).clamp(0.2, 1.3);
            let level = c.wires * flow_level(u, U_REF, 6.0) * coherent * audible * wander * 0.05;
            let f1 = c.wire_fundamental_hz;
            let n0 = (fs / f1).round().max(1.0) as u32;
            for (k, w) in want.iter_mut().enumerate() {
                let n = (n0 + k as u32).saturating_sub(1).max(1);
                let fnn = n as f32 * f1;
                let width = 0.06 * fnn + 0.3 * f1;
                let lock = (-((fs - fnn) / width).powi(2)).exp();
                *w = (n, level * lock);
            }
            self.tone_hz = fs;
        } else {
            self.tone_hz = 0.0;
        }
        // Keep phases of slots that stay; free slots fade out before reuse.
        for s in self.slots.iter_mut() {
            s.target = 0.0;
        }
        for &(n, a) in want.iter().filter(|w| w.1 > 0.0) {
            if let Some(s) = self.slots.iter_mut().find(|s| s.n == n) {
                s.target = a;
            } else if let Some(s) = self.slots.iter_mut().find(|s| s.amp.cur < 1e-6 && s.target == 0.0) {
                s.n = n;
                s.target = a;
            }
        }
        for s in self.slots.iter_mut() {
            s.amp.to(s.target, len);
        }
    }

    fn update_gap(&mut self, u: f32, len: usize) {
        let c = &self.cfg;
        let mut level = 0.0;
        if c.gap_hz > 0.0 && c.gap > 0.0 {
            let fh = c.gap_hz;
            // Rossiter shear-layer modes; the best-matching one drives the cavity.
            let (mut exc, mut best) = (0.0f32, fh);
            for m in 1..=3 {
                let f = 0.57 * (m as f32 - 0.25) * u / c.gap_length_m;
                let e = (-((f - fh) / (0.25 * fh)).powi(2)).exp();
                if e > exc {
                    best = f;
                }
                exc += e;
            }
            level = c.gap * exc.min(1.0) * flow_level(u, U_REF, 4.0) * 0.06;
            // The jet pulls the pitch slightly towards the shear-layer frequency.
            self.gap_freq = fh * (1.0 + 0.04 * ((best - fh) / fh).tanh());
            let sr = self.sample_rate;
            self.gap1.set(self.gap_freq, c.gap_q, sr);
            self.gap2.set(self.gap_freq, c.gap_q, sr);
            if exc > 0.3 && self.tone_hz == 0.0 {
                self.tone_hz = self.gap_freq;
            }
        }
        self.gap_target = level;
        self.gap_gain.to(level, len);
    }
}

impl Generator for Wind {
    fn set_model(&mut self, cfg: &super::ModelConfig) {
        if let super::ModelConfig::Wind(c) = cfg {
            if *c != self.cfg {
                self.set_config(*c);
            }
        }
    }

    fn render(&mut self, out: &mut [f32], ctl: &Controls) {
        let len = out.len();
        let sr = self.sample_rate;
        let dt = len as f32 / sr;
        let c = self.cfg.clone();

        // Gust arriving at the source; its parts hear it at different times.
        let mean = wind_slider(ctl.env.wind_ms);
        let local = (mean + (ctl.gust - mean) * c.gust_response).max(0.0);
        self.trail.push(local, dt);
        let extent = (0.25 * c.distance_m).clamp(2.0, 40.0);
        let cross_s = extent / (3.0 + 6.0 * mean.min(1.5)); // gust-front speed as windfield.rs
        let smooth_s = 0.12 + c.distance_m / 80.0;
        if !self.started {
            self.started = true;
            self.gust = Smooth::new(local);
            for g in self.band_gust.iter_mut() {
                *g = Smooth::new(local);
            }
        }
        // Wires and gaps sit in the middle of the source.
        let g = self.gust.step(self.trail.at(0.5 * cross_s), dt, smooth_s);
        let turb = self.turb.step(dt, 0.25, 0.18, &mut self.rng);
        self.u = wind_speed_ms(g) * c.exposure * (1.0 + turb).max(0.3);
        let u = self.u;
        // Slow breathing of a distant roar.
        let far = smoothstep(30.0, 300.0, c.distance_m);
        let breath = if far > 0.0 { (0.35 * far * self.breath.step(dt, 7.0, 1.0, &mut self.rng)).exp() } else { 1.0 };

        // Body: diameter bands, each with its own local wind.
        let u_mean = (wind_speed_ms(mean) * c.exposure).max(0.5);
        let mut level_sum = 0.0f32;
        for k in 0..BANDS {
            let x = k as f32 / (BANDS - 1) as f32;
            let d = c.d_min_m * (c.d_max_m / c.d_min_m).powf(x);
            let gk = self.band_gust[k].step(self.trail.at(self.lag[k] * cross_s), dt, smooth_s);
            // Twigs follow small fast eddies, limbs only the large slow ones.
            let tau = (0.15 + 1.9 * x * x) * (5.0 / u_mean).sqrt().clamp(0.5, 2.0);
            let sigma = 0.2 - 0.14 * x;
            let tk = self.band_turb[k].step(dt, tau, sigma, &mut self.rng);
            let uk = wind_speed_ms(gk) * c.exposure * tk.exp();
            let level = c.body * flow_level(uk, U_REF, c.level_exponent) * 0.012 * breath;
            level_sum += level;
            let f = strouhal_hz(c.strouhal, uk, d).clamp(12.0, 16_000.0);
            let q = 1.3;
            self.bands[k].set(f, q, sr);
            let audible = smoothstep(14.0, 45.0, f) * (1.0 - smoothstep(12_000.0, 16_000.0, f));
            // 8 bands over the same diameter range as the former 6: same total level.
            let tilt = 10f32.powf(c.tilt_db * (d / c.d_min_m).log2() / 20.0) * (6.0 / BANDS as f32).sqrt();
            let shed = smoothstep(40.0, 200.0, reynolds(uk, d));
            self.band_gain[k].to(level * tilt * audible * shed * band_norm(f, q, sr), len);
        }
        let level = level_sum;
        // Air absorption + area averaging: far wind keeps only the roar.
        let f_lp = (14_000.0 * (20.0 / c.distance_m.max(20.0)).powf(0.6)).max(300.0);
        self.lp_coef = one_pole_coef(f_lp, sr);
        self.update_wires(u, len, dt);
        self.update_gap(u, len);
        // Saltation (Bagnold 1941; Owen 1964): grains hop once the wind passes the threshold,
        // mass flux q ∝ u³(1 − u_t²/u²). The hiss is the sum of countless impacts: power ∝ q.
        let xs = u / c.sand_threshold_ms.max(1.0);
        let flux = if c.sand > 0.0 && xs > 1.0 { xs * xs * xs * (1.0 - 1.0 / (xs * xs)) } else { 0.0 };
        let sand_level = c.sand * 0.01 * flux.sqrt();
        self.sand_band.set(c.sand_hz, 0.7, sr);
        self.sand_gain.to(sand_level * band_norm(c.sand_hz, 0.7, sr), len);
        let grains = sand_level > 0.0 || self.sand_gain.cur > 0.0;

        let silent = level < 1e-7
            && self.band_gain.iter().all(|r| r.cur < 1e-7)
            && self.slots.iter().all(|s| s.amp.cur < 1e-7 && s.target == 0.0)
            && self.gap_gain.cur < 1e-7
            && self.gap_target < 1e-7
            && !grains;
        if silent {
            out.fill(0.0);
            for r in self.band_gain.iter_mut() {
                r.settle(0.0);
            }
            self.gap_gain.settle(0.0);
            return;
        }

        let f1 = c.wire_fundamental_hz;
        let wire_jitter = 1.0 + 0.004 * turb;
        let mut incs = [0.0f32; WIRE_SLOTS];
        for (i, s) in self.slots.iter().enumerate() {
            incs[i] = s.n as f32 * f1 * wire_jitter / sr;
        }
        let gap_norm = band_norm(self.gap_freq.max(40.0), c.gap_q, sr) * 1.4;
        let out_gain = c.gain;
        for o in out.iter_mut() {
            let white = self.rng.bipolar();
            let mut body = 0.0;
            for k in 0..BANDS {
                let (_, bp, _) = self.bands[k].process(white);
                body += bp * self.band_gain[k].next();
            }
            self.lp1 += (body - self.lp1) * self.lp_coef;
            self.lp2 += (self.lp1 - self.lp2) * self.lp_coef;
            let mut v = self.lp2;
            for (i, s) in self.slots.iter_mut().enumerate() {
                let a = s.amp.next();
                s.phase += incs[i];
                if s.phase >= 1.0 {
                    s.phase -= 1.0;
                }
                if a > 0.0 {
                    let p = TAU * s.phase;
                    v += a * (p.sin() + 0.12 * (2.0 * p).sin());
                }
            }
            let (_, g1, _) = self.gap1.process(white);
            let (_, g2, _) = self.gap2.process(g1);
            v += g2 * self.gap_gain.next() * gap_norm;
            if grains {
                let w2 = self.rng.bipolar();
                self.sand_env += (4.0 * self.rng.bipolar() - self.sand_env) * self.sand_coef;
                let (_, sb, _) = self.sand_band.process(w2);
                v += sb * self.sand_gain.next() * (1.0 + 3.0 * self.sand_env).max(0.2);
            }
            *o = v * out_gain;
        }
        for k in 0..BANDS {
            let t = self.band_gain[k].cur;
            self.band_gain[k].settle(t);
            self.bands[k].flush();
        }
        for s in self.slots.iter_mut() {
            s.amp.settle(s.amp.cur.max(0.0));
            if s.target == 0.0 && s.amp.cur < 1e-6 {
                s.amp.settle(0.0);
            }
        }
        let gg = self.gap_gain.cur;
        self.gap_gain.settle(gg);
        let sg = self.sand_gain.cur;
        self.sand_gain.settle(if grains { sg.max(0.0) } else { 0.0 });
        self.sand_band.flush();
        self.sand_env = flush(self.sand_env);
        self.gap1.flush();
        self.gap2.flush();
        self.lp1 = flush(self.lp1);
        self.lp2 = flush(self.lp2);
    }

    fn pitch(&self) -> f32 {
        self.tone_hz
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn level(cfg: WindConfig, wind_ms: f32) -> f32 {
        let mut w = Wind::new(48_000.0, 3, cfg);
        let mut c = Controls::default();
        c.env.wind_ms = wind_ms;
        c.gust = wind_slider(wind_ms);
        let mut buf = [0.0f32; 512];
        let mut e = 0.0f64;
        for _ in 0..200 {
            w.render(&mut buf, &c);
            e += buf.iter().map(|v| (*v as f64).powi(2)).sum::<f64>();
        }
        10.0 * (e / (200.0 * 512.0)).log10() as f32
    }

    #[test]
    fn sand_hisses_only_in_a_wind() {
        let calm = level(Wind::dune_sand(), 2.0);
        let windy = level(Wind::dune_sand(), 9.0);
        let breeze = level(Wind::breeze(), 9.0);
        let howl = level(Wind::ridge_howl(), 9.0);
        let snow = level(Wind::blowing_snow(), 9.0);
        eprintln!("dune sand 2 m/s {calm:.1} dB, 9 m/s {windy:.1} dB; breeze {breeze:.1}; ridge howl {howl:.1}; blowing snow {snow:.1}");
        assert!(windy > calm + 15.0);
        assert!(howl.is_finite() && snow.is_finite());
    }

    #[test]
    fn aeolian_tone_follows_wind_speed() {
        let w = Wind::new(48_000.0, 1, Wind::fence_wire());
        // f = St·U/d: doubling the wind doubles the tone; 0.2·10/0.003 = 667 Hz.
        assert!((w.wire_shedding_hz(10.0) - 666.7).abs() < 1.0);
        assert!((w.wire_shedding_hz(10.0) / w.wire_shedding_hz(5.0) - 2.0).abs() < 1e-4);
    }
}
