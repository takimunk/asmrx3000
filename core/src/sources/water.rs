//! Water from first principles: almost everything we hear from water is air bubbles ringing
//! at their Minnaert resonance as they are entrained, plus impacts and, for big flows, the
//! collective oscillation of bubble clouds.
//!
//! Each bubble is a damped sinusoid with a slight upward chirp (van den Doel 2005):
//!   f₀ = 3.26/r (Hz, r in m),  d = 0.043·f₀ + 0.0014·f₀^1.5 (1/s),  f(t) = f₀(1 + ξ·d·t).
//!
//! Regimes come out of a few physical parameters rather than per-regime code paths:
//! * **Entrainment** happens where turbulent velocity fluctuations u′ break the free surface.
//!   u′ grows with flow speed, bed roughness relative to depth (rocks near the surface) and
//!   extra turbulence (steps, constrictions); below u′ ≈ 0.05 m/s the surface stays smooth
//!   (a slow deep river only gurgles at the odd snag).
//! * **Bubble sizes** follow the broken power law measured under breaking water (Deane &
//!   Stokes 2002): N(r) ∝ r^−3/2 below the Hinze scale a_H and ∝ r^−10/3 above it, with
//!   a_H = 0.5·(We_c·σ/ρ)^{3/5}·ε^{−2/5} from the turbulent dissipation ε ≈ u′³/L. Gentle
//!   brooks keep large bubbles (deep "bloops"); violent rapids shred them (bright fizz). The
//!   largest bubbles scale with the rocks.
//! * **Plunging flow** (falls, steps) adds impact noise with power ∝ ½ρ·Q·v², v = √(2gH+U²),
//!   and bubble-cloud oscillations far below single-bubble frequencies (a cloud of radius R
//!   and void fraction β rings near (1/2πR)·√(3γp₀/ρβ), tens of Hz): the waterfall's thrum.
//! * **Lapping** on a lake shore: wind waves at the JONSWAP peak f_p = 3.5·(g/U)·(gF/U²)^−0.33
//!   with height H ∝ U²/g·√(gF/U²) arrive and slosh against the stones (soft 250–700 Hz
//!   slaps of ~0.1 s), entrain a burst of soft small bubbles, fizz back through the gravel
//!   and now and then gulp an air pocket.
//! * **Drips** into a pool: an impact plus one large chirping bubble (the "plink").
//!   **Trickles**: a thin jet breaking into drops at the Rayleigh–Plateau rate U/(4.5·d).
//! * Rain on the water surface (via [`RainLayer`]).
//!
//! Event counts that would be too many to render one by one go into a noise bed of the same
//! energy and spectrum (Campbell's theorem); rapids are mostly bed, a brook mostly bubbles.
//!
//! What makes running water sound like water rather than like rain on a pond is that the
//! bubbles are not a steady Poisson rain: each stone, lip and eddy entrains air in bursts as
//! it sheds vortices (Strouhal rate 0.2·U/d), each burst lasting a fraction of an eddy
//! turnover, and over a second the entrained flux is fixed by the hydraulics (`Babble`). So a
//! rocky brook chatters at 4–16 Hz while it has almost no slow swell. Bubbles big enough to
//! sound like a "bloop" are pinched off right at the surface, where they wobble out of shape
//! and break up: they ring several times shorter than a free spherical bubble
//! (`BubbleBank::spawn_damped`), and a gulp of air behind a snag is a short cluster of bubbles
//! with a slosh, not one clean tone.
//!
//! Beds that are always there, even where nothing is entrained in bulk: the **surface-flow
//! bed** (wavelets breaking at banks, snags and weed, ∝ u³ per area) is what a slow river
//! sounds like, and the **shore wash** (the laps along the rest of the shore) keeps a lake
//! from falling silent between the laps near the listener.
//!
//! Tuned against field recordings (`ml/asmr_ml/refcompare.py`, docs/references.md): each
//! change below cites the feature it fixed (reference median vs ours before).

use serde::{Deserialize, Serialize};

use super::common::*;
use super::rain::{check, fall_speed, RainLayer, Surface};
use super::{Controls, Generator, BLOCK};
use crate::rng::Rng;

const R_MIN: f32 = 0.0003;
/// Individually rendered bubbles per second (the rest goes into the bed).
const RESOLVED: f32 = 500.0;
/// Overall level: a pebble brook segment at 1 m is LAeq ≈ 57 dB (mix calibration table), at 4 m
/// ~10 dB under the birds (all regimes keep
/// their physical ratios).
const WATER_LEVEL: f32 = 0.048;
/// Discrete events (laps, drips, trickle, snag gulps) keep their own calibration.
const EVENT_LEVEL: f32 = 0.2 / WATER_LEVEL;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct WaterConfig {
    /// Mean surface flow speed, m/s (0..6). In a scene the WATER macro moves it along the
    /// hydraulic geometry curve (see `controls`).
    pub flow_ms: f32,
    /// Water depth, m (0.01..5).
    pub depth_m: f32,
    /// Width of the stream at this emitter, m (0.05..50).
    pub width_m: f32,
    /// Length of stream (or shore) this emitter represents, m (0.1..100).
    pub length_m: f32,
    /// Median bed / shore rock size, m (0.002..2).
    pub rock_d50_m: f32,
    /// Extra turbulence from steps and constrictions (0..1).
    pub turbulence: f32,
    /// How readily the flow entrains air (0..1.5): clear water 0.3, white water 1.
    pub aeration: f32,
    /// Fall height of a plunging flow (step, cascade, waterfall), m (0..30); also the fall
    /// height of drips.
    pub drop_height_m: f32,
    /// Wave action on a shore (0..2): 0 none, 1 typical lake; waves also grow with the wind.
    pub waves: f32,
    /// Wind fetch over the lake, m (10..20000).
    pub fetch_m: f32,
    /// Discrete drips per second into a pool (0..20).
    pub drips_hz: f32,
    /// Thin stream (trickle) into a pool (0..1).
    pub trickle: f32,
    /// Level of rain falling on the water surface (0..3).
    pub rain_on_surface: f32,
    /// Sea surf: height of the breakers, m (0 = none; 0..4). Each breaker plunges (a
    /// broadband crash), washes up the beach (hiss) and drains back (through shingle: a
    /// rattle of pebbles).
    pub surf: f32,
    /// Swell period, s (4..18): time between breakers.
    pub swell_s: f32,
    /// Shingle on the beach (0 sand .. 1 pebbles): the rattle of the backwash.
    pub shingle: f32,
    /// Linear output gain (0..8).
    pub gain: f32,
}

impl Default for WaterConfig {
    fn default() -> Self {
        Self {
            flow_ms: 0.8,
            depth_m: 0.12,
            width_m: 1.5,
            length_m: 4.0,
            rock_d50_m: 0.08,
            turbulence: 0.25,
            aeration: 0.5,
            drop_height_m: 0.0,
            waves: 0.0,
            fetch_m: 300.0,
            drips_hz: 0.0,
            trickle: 0.0,
            rain_on_surface: 1.0,
            surf: 0.0,
            swell_s: 9.0,
            shingle: 0.0,
            gain: 1.0,
        }
    }
}

impl WaterConfig {
    pub fn validate(&self) -> Result<(), String> {
        check("flow_ms", self.flow_ms, 0.0, 6.0)?;
        check("depth_m", self.depth_m, 0.01, 5.0)?;
        check("width_m", self.width_m, 0.05, 50.0)?;
        check("length_m", self.length_m, 0.1, 100.0)?;
        check("rock_d50_m", self.rock_d50_m, 0.002, 2.0)?;
        check("turbulence", self.turbulence, 0.0, 1.0)?;
        check("aeration", self.aeration, 0.0, 1.5)?;
        check("drop_height_m", self.drop_height_m, 0.0, 30.0)?;
        check("waves", self.waves, 0.0, 2.0)?;
        check("fetch_m", self.fetch_m, 10.0, 20_000.0)?;
        check("drips_hz", self.drips_hz, 0.0, 20.0)?;
        check("trickle", self.trickle, 0.0, 1.0)?;
        check("rain_on_surface", self.rain_on_surface, 0.0, 3.0)?;
        check("surf", self.surf, 0.0, 4.0)?;
        check("swell_s", self.swell_s, 4.0, 18.0)?;
        check("shingle", self.shingle, 0.0, 1.0)?;
        check("gain", self.gain, 0.0, 8.0)
    }
}

/// Derived hydraulics for the current flow (exposed for tests and telemetry).
#[derive(Clone, Copy, Debug, Default)]
pub struct Hydraulics {
    pub froude: f32,
    /// Turbulent velocity fluctuation u′, m/s.
    pub u_turb: f32,
    /// Hinze scale, m.
    pub hinze_m: f32,
    pub r_max_m: f32,
    /// Bubbles entrained per second on this emitter's surface.
    pub bubbles_per_s: f32,
    /// Plunge impact power, W.
    pub plunge_w: f32,
}

pub fn hydraulics(c: &WaterConfig, u: f32) -> Hydraulics {
    let h = c.depth_m.max(0.01);
    let k = (c.rock_d50_m / h).min(2.0);
    let froude = u / (G * h).sqrt();
    let mut u_t = u * (0.06 + 0.1 * k + 0.3 * c.turbulence);
    let v_imp = (2.0 * G * c.drop_height_m + u * u).sqrt();
    let q = u * h * c.width_m;
    let plunge_w = if c.drop_height_m > 0.02 { 0.5 * 1000.0 * q * v_imp * v_imp } else { 0.0 };
    if plunge_w > 0.0 {
        u_t = u_t.max(0.25 * v_imp);
    }
    let l = h.min(2.0 * c.rock_d50_m).max(0.02);
    let eps = u_t.powi(3) / l;
    let hinze = 4.15e-3 * eps.max(1e-6).powf(-0.4);
    // (The size law clamps a_H into [r_min, r_max]; the raw value is reported.)
    let r_max = (0.06 * c.rock_d50_m).clamp(0.0015, 0.01).max(if plunge_w > 0.0 { 0.006 } else { 0.0 });
    let area = c.width_m * c.length_m;
    let s = smoothstep(0.05, 0.4, u_t) * (u_t / 0.3).powf(1.5);
    let mut rate = 1500.0 * (0.3 + c.aeration) * s * area;
    rate += plunge_w.sqrt() * 40.0 * (0.3 + c.aeration);
    Hydraulics { froude, u_turb: u_t, hinze_m: hinze, r_max_m: r_max, bubbles_per_s: rate, plunge_w }
}

/// Broken power-law bubble radius sampler (Deane & Stokes).
#[derive(Clone, Copy, Default)]
struct RadiusLaw {
    r0: f32,
    a: f32,
    r1: f32,
    share_small: f32,
}

const P_SMALL: f32 = 1.5;
const P_LARGE: f32 = 10.0 / 3.0;

fn power_mass(r0: f32, r1: f32, p: f32) -> f32 {
    (r1.powf(1.0 - p) - r0.powf(1.0 - p)) / (1.0 - p)
}

fn power_draw(r0: f32, r1: f32, p: f32, u: f32) -> f32 {
    let (a, b) = (r0.powf(1.0 - p), r1.powf(1.0 - p));
    (a + u * (b - a)).powf(1.0 / (1.0 - p))
}

impl RadiusLaw {
    fn new(r0: f32, a: f32, r1: f32) -> Self {
        let a = a.clamp(r0 * 1.01, r1);
        let m1 = power_mass(r0, a, P_SMALL);
        let m2 = if r1 > a * 1.001 { a.powf(P_LARGE - P_SMALL) * power_mass(a, r1, P_LARGE) } else { 0.0 };
        Self { r0, a, r1, share_small: m1 / (m1 + m2).max(1e-12) }
    }

    fn draw(&self, rng: &mut Rng) -> f32 {
        if rng.uniform() < self.share_small || self.r1 <= self.a * 1.001 {
            power_draw(self.r0, self.a, P_SMALL, rng.uniform())
        } else {
            power_draw(self.a, self.r1, P_LARGE, rng.uniform())
        }
    }
}

#[inline]
fn bubble_amp(r: f32) -> f32 {
    0.32 * (r / 0.003).powf(0.9)
}

fn bubble_energy(r: f32, sr: f32) -> f32 {
    let a = bubble_amp(r);
    a * a * 0.25 * sr / (bubble_damping(minnaert_hz(r)) * surface_damping(r))
}

/// One breaking wave: plunge, swash up the beach, backwash.
#[derive(Clone, Copy, Default)]
struct Breaker {
    on: bool,
    t: f32,
    dur: f32,
    amp: f32,
}

/// Sea surf (see `WaterConfig::surf`). A plunging breaker of height H dissipates the wave's
/// energy flux E·c_g ∝ H²·T over a few seconds: the crash is broadband noise (a curtain of
/// bubbles entrained at once, far too many to resolve: Campbell bed) whose level grows with
/// H^1.5·√T; the swash hisses as the bore runs up the beach and the backwash drains through
/// the sand or rattles the shingle (pebble collisions at a rate ∝ backwash × shingle). Wave
/// groups (sets of larger waves every ~6 periods) and a continuous roar of the breakers
/// further along the shore make the rest. Up to three breakers overlap.
struct Surf {
    breakers: [Breaker; 3],
    next: f32,
    group: f32,
    crash: [NoiseBand; 3],
    hiss: NoiseBand,
    roar: NoiseBand,
    ev_rattle: Events,
    rattle_rate: f32,
}

impl Surf {
    fn new(sr: f32, rng: &mut Rng) -> Self {
        Self {
            breakers: [Breaker::default(); 3],
            next: rng.range(0.5, 4.0),
            group: rng.uniform(),
            crash: [NoiseBand::new(110.0, 0.5, sr), NoiseBand::new(650.0, 0.6, sr), NoiseBand::new(2800.0, 0.7, sr)],
            hiss: NoiseBand::new(6000.0, 0.8, sr),
            roar: NoiseBand::new(180.0, 0.45, sr),
            ev_rattle: Events::new(rng),
            rattle_rate: 0.0,
        }
    }

    fn active(&self) -> bool {
        self.breakers.iter().any(|b| b.on) || self.roar.gain.cur > 0.0
    }

    /// Advances the breakers by one block and sets the band levels.
    fn update(&mut self, cfg: &WaterConfig, dt: f32, len: usize, sr: f32, rng: &mut Rng) {
        let period = cfg.swell_s.clamp(4.0, 18.0);
        if cfg.surf > 0.0 {
            self.next -= dt;
            self.group = (self.group + dt / (6.5 * period)) % 1.0;
            if self.next <= 0.0 {
                self.next = period * rng.range(0.8, 1.2);
                if let Some(b) = self.breakers.iter_mut().find(|b| !b.on) {
                    let set = 0.65 + 0.35 * (core::f32::consts::TAU * self.group).sin();
                    *b = Breaker { on: true, t: 0.0, dur: period * rng.range(0.65, 0.85), amp: cfg.surf * set * (0.3 * rng.gauss()).exp() };
                }
            }
        }
        let (mut crash, mut wash, mut back) = (0.0f32, 0.0f32, 0.0f32);
        for b in self.breakers.iter_mut().filter(|b| b.on) {
            b.t += dt;
            let t = b.t;
            let c = if t < 0.35 { (t / 0.35) * (t / 0.35) } else { (-(t - 0.35) / 0.9).exp() };
            crash += b.amp.powf(1.5) * c;
            wash += b.amp * smoothstep(0.3, 1.2, t) * (1.0 - smoothstep(0.45 * b.dur, 0.7 * b.dur, t));
            back += b.amp * smoothstep(0.5 * b.dur, 0.65 * b.dur, t) * (1.0 - smoothstep(0.85 * b.dur, b.dur, t));
            if t >= b.dur {
                b.on = false;
            }
        }
        let k = 2.0 * EVENT_LEVEL * period.sqrt() / 3.0;
        for (band, (w, fc, q)) in self.crash.iter_mut().zip(CRASH_BANDS) {
            band.gain.to(k * w * (crash + 0.15 * wash) * band_norm(fc, q, sr), len);
        }
        self.hiss.gain.to(k * (0.45 * wash + 0.3 * back * (1.0 - cfg.shingle)) * band_norm(6000.0, 0.8, sr), len);
        let shore = (cfg.length_m / 40.0).clamp(0.05, 3.0).sqrt();
        self.roar.gain.to(k * 0.22 * cfg.surf.powf(1.5) * shore * band_norm(180.0, 0.45, sr), len);
        self.rattle_rate = 500.0 * cfg.shingle * back / sr;
    }
}

/// Plunge impact noise bands: (weight, centre Hz, Q). The lowest band grows with the fall
/// height (see `update`).
const PLUNGE_BANDS: [(f32, f32, f32); 4] = [(16.0, 110.0, 0.5), (0.6, 300.0, 0.7), (1.0, 1200.0, 0.7), (0.5, 4000.0, 0.8)];

/// Surface-flow bed: (weight, centre Hz, Q), shaped after the slow-river references (flat
/// 0.5–2 kHz third octaves, about −3 dB/octave above, falling off under 400 Hz).
const RIPPLE_BANDS: [(f32, f32, f32); 3] = [(1.0, 750.0, 0.55), (0.4, 2200.0, 0.6), (0.05, 7000.0, 0.7)];
/// Shore wash bed: (weight, centre Hz, Q).
const WASH_BANDS: [(f32, f32, f32); 2] = [(1.0, 400.0, 0.5), (0.03, 3500.0, 0.5)];

/// Surf crash bands: (weight, centre Hz, Q).
const CRASH_BANDS: [(f32, f32, f32); 3] = [(1.0, 110.0, 0.5), (0.7, 650.0, 0.6), (0.35, 2800.0, 0.7)];

pub struct Water {
    sample_rate: f32,
    rng: Rng,
    cfg: WaterConfig,
    flow: Smooth,
    hyd: Hydraulics,
    law: RadiusLaw,
    bubbles: BubbleBank<192>,
    ev_bubble: Events,
    resolved: f32,
    bed_lo: NoiseBand,
    bed_hi: NoiseBand,
    splash: NoiseBand,
    plunge: [NoiseBand; 4],
    cloud: [Mode; 2],
    ev_cloud: Events,
    cloud_rate: f32,
    cloud_amp: f32,
    murmur: NoiseBand,
    grains: GrainPool<24>,
    // Lapping.
    wave_clock: f32,
    wave_period: f32,
    wave_h: f32,
    group: f32,
    burst: f32,
    // Drips and trickles.
    drip_clock: f32,
    trickle_rate: f32,
    ev_trickle: Events,
    rain: RainLayer,
    out: Ramp,
    rate_telemetry: f32,
    dirty: bool,
    key_u: f32,
    key_wind: f32,
    /// Bubbles per second rendered one by one (the rest is bed).
    resolved_per_s: f32,
    wander_ou: Ou,
    /// Own generator, so the slow wandering does not depend on how many events were drawn.
    wander_rng: Rng,
    wander: f32,
    snag_rate: f32,
    ev_snag: Events,
    bed_mod: Ramp,
    surf: Surf,
    babble: Babble,
    /// Continuous surface-flow bed (see `update`).
    ripple: [NoiseBand; 3],
    /// Wash of the waves lapping along the rest of the shore.
    wash: [NoiseBand; 2],
    wash_mod: Ramp,
    ripple_lp: f32,
    /// Fast flicker of the unresolved bed (see `render_block`): low-passed noise, its depth,
    /// and its own generator (the other random streams stay as they were).
    flicker: f32,
    flicker_depth: f32,
    flicker_rng: Rng,
}

/// Babbling spots (clustered entrainment). Air is not entrained at a steady rate: a stone
/// sheds a vortex, the standing wave behind it tumbles, a packet of the falling sheet hits
/// the pool, and each such breaking event entrains a cluster of bubbles within a few tens of
/// ms. Shedding behind a stone of size d in a flow U runs at the Strouhal rate St·U/d
/// (St ≈ 0.2); the few stones nearest the listener (`FEATURES`) dominate a segment, and each
/// burst lasts about a third of an eddy turnover d/U. Shedding is quasi-periodic (intervals
/// jittered ±60 %), and over a second the entrained air flux is fixed by the hydraulics: the
/// bursts redistribute it in time rather than add to it. So the modulator is the burst train
/// minus its own running mean (τ ≈ 0.3 s): `gate = 1 + depth·(g − ⟨g⟩)`, mean 1.
///
/// (The first version let three spots surge at 0.6–4 Hz with slow, 0.1–0.6 s glides: that put
/// the brook's modulation at 0.1–1 Hz, −12 dB against −23 dB in the references, and left the
/// rocky brook with no fast chatter at all: 4–16 Hz 12 dB, 16–64 Hz 9 dB short. The
/// references rise with rate: −23, −20, −15, −12 dB at 0.1–1, 1–4, 4–16, 16–64 Hz.)
struct Babble {
    next: f32,
    rng: Rng,
    fast: f32,
    slow: f32,
    avg: f32,
    /// Bursts per second, decay time (s) and modulation depth (0..1).
    rate: f32,
    tau: f32,
    depth: f32,
    /// Entrainment factor (mean 1) and bubble size factor of this block: a strong burst
    /// tears off slightly bigger bubbles (a deeper gurgle).
    gate: f32,
    grow: f32,
}

/// Breaking features (stones, packets of the falling sheet) heard individually.
const FEATURES: f32 = 3.0;
/// Rise time of one entrainment burst, s.
const BURST_RISE: f32 = 0.008;

impl Babble {
    fn new(rng: &mut Rng) -> Self {
        let mut rng = Rng::new(rng.next_u32() as u64 ^ 0xB0BB_1E55);
        Self { next: rng.uniform(), rng, fast: 0.0, slow: 0.0, avg: 1.0, rate: 1.0, tau: 0.08, depth: 0.0, gate: 1.0, grow: 1.0 }
    }

    /// Burst statistics for breaking features of size `d` (m) in a flow `u` (m/s); `depth`
    /// 0..1 is how much the flow chatters (calm water not at all, a brook a lot).
    fn tune(&mut self, u: f32, d: f32, depth: f32) {
        let d = d.max(0.01);
        self.rate = (FEATURES * 0.2 * u / d).clamp(0.5, 15.0);
        self.tau = (0.3 * d / u.max(0.05)).clamp(0.025, 0.12);
        self.depth = depth.clamp(0.0, 1.0);
    }

    /// Advances one block.
    fn step(&mut self, dt: f32) {
        if self.depth <= 0.0 {
            self.gate = 1.0;
            self.grow = 1.0;
            return;
        }
        self.next -= dt;
        if self.next <= 0.0 {
            self.next += self.rng.range(0.4, 1.6) / self.rate;
            let a = (0.3 * self.rng.gauss()).exp();
            self.slow += a;
            self.fast += a;
        }
        self.slow = flush(self.slow * (-dt / self.tau).exp());
        self.fast = flush(self.fast * (-dt / BURST_RISE).exp());
        // Mean of the burst train = rate·E[a]·(τ − τ_rise), E[a] = e^{σ²/2}.
        let mean = self.rate * 1.046 * (self.tau - BURST_RISE).max(1e-3);
        let g = (self.slow - self.fast).max(0.0) / mean;
        self.avg += (g - self.avg) * (dt / 0.3).min(1.0);
        let x = g - self.avg;
        self.gate = (1.0 + self.depth * x).max(0.05);
        self.grow = (1.0 + 0.12 * self.depth * x).clamp(0.8, 1.3);
    }
}

/// Damping factor of a bubble of radius `r` pinched off at the surface: big ones wobble and
/// break up (×1 at 2 mm, ×3 at 6 mm, ×4.5 at 1 cm).
fn surface_damping(r: f32) -> f32 {
    1.0 + ((r - 0.002) / 0.002).clamp(0.0, 3.5)
}

impl Water {
    pub fn new(sample_rate: f32, seed: u64, cfg: WaterConfig) -> Self {
        let sr = sample_rate;
        let mut rng = Rng::new(seed);
        let rain = RainLayer::new(Surface::Water, cfg.width_m * cfg.length_m, sr, &mut rng);
        Self {
            sample_rate,
            flow: Smooth::new(0.0),
            hyd: Hydraulics::default(),
            law: RadiusLaw::new(R_MIN, 0.002, 0.006),
            bubbles: BubbleBank::new(),
            ev_bubble: Events::new(&mut rng),
            resolved: 0.0,
            bed_lo: NoiseBand::new(800.0, 1.0, sr),
            bed_hi: NoiseBand::new(3000.0, 1.0, sr),
            splash: NoiseBand::new(4500.0, 0.7, sr),
            plunge: PLUNGE_BANDS.map(|(_, f, q)| NoiseBand::new(f, q, sr)),
            cloud: [Mode::default(); 2],
            ev_cloud: Events::new(&mut rng),
            cloud_rate: 0.0,
            cloud_amp: 0.0,
            murmur: NoiseBand::new(250.0, 0.6, sr),
            grains: GrainPool::new(),
            wave_clock: rng.uniform() * 2.0,
            wave_period: 2.0,
            wave_h: 0.0,
            group: rng.uniform(),
            burst: 0.0,
            drip_clock: rng.uniform(),
            trickle_rate: 0.0,
            ev_trickle: Events::new(&mut rng),
            rain,
            out: Ramp::new(cfg.gain),
            rate_telemetry: 0.0,
            dirty: true,
            key_u: -1.0,
            key_wind: -1.0,
            resolved_per_s: RESOLVED,
            wander_ou: Ou::default(),
            wander_rng: Rng::new(seed ^ 0x57A7_E0F1),
            wander: 1.0,
            snag_rate: 0.0,
            ev_snag: Events::new(&mut rng),
            bed_mod: Ramp::new(1.0),
            surf: Surf::new(sr, &mut rng),
            babble: Babble::new(&mut rng),
            ripple: RIPPLE_BANDS.map(|(_, f, q)| NoiseBand::new(f, q, sr)),
            wash: WASH_BANDS.map(|(_, f, q)| NoiseBand::new(f, q, sr)),
            wash_mod: Ramp::new(1.0),
            ripple_lp: 0.0,
            flicker: 0.0,
            flicker_depth: 0.0,
            flicker_rng: Rng::new(seed ^ 0xF11C_4E55),
            rng,
            cfg,
        }
    }

    pub fn config(&self) -> &WaterConfig {
        &self.cfg
    }

    /// Change parameters at run time (flow and levels glide, no clicks).
    pub fn set_config(&mut self, cfg: WaterConfig) {
        self.rain.area = cfg.width_m * cfg.length_m;
        self.cfg = cfg;
        self.dirty = true;
    }

    pub fn hydraulics(&self) -> Hydraulics {
        self.hyd
    }

    /// Budget of individually rendered bubbles per second (tests / quality settings).
    pub fn set_resolved_per_s(&mut self, n: f32) {
        self.resolved_per_s = n.clamp(1.0, 5000.0);
        self.dirty = true;
    }

    pub fn still_lake() -> WaterConfig {
        WaterConfig { flow_ms: 0.0, depth_m: 2.0, width_m: 3.0, length_m: 6.0, rock_d50_m: 0.04, waves: 0.5, fetch_m: 300.0, aeration: 0.3, ..Default::default() }
    }
    pub fn lake_lapping() -> WaterConfig {
        WaterConfig { waves: 1.2, fetch_m: 800.0, rock_d50_m: 0.12, ..Self::still_lake() }
    }
    pub fn slow_river() -> WaterConfig {
        WaterConfig { flow_ms: 0.4, depth_m: 1.2, width_m: 8.0, length_m: 6.0, rock_d50_m: 0.05, turbulence: 0.1, aeration: 0.3, ..Default::default() }
    }
    pub fn brook_pebbles() -> WaterConfig {
        WaterConfig::default()
    }
    /// Faster brook over cobbles and stones, broken by the stones rather than by steps.
    /// (Was 25 cm boulders with 15 cm steps: their 1 cm bubbles and plunge put 125–500 Hz
    /// 4–8 dB over the brook references, the few big stones shed bursts too regularly —
    /// a lumpy 1–4 Hz pulse, `ioi_cv` 0.60 against 0.84 — and it "boomed"; distance 0.75 → 0.50.)
    pub fn brook_rocks() -> WaterConfig {
        WaterConfig { flow_ms: 1.4, depth_m: 0.2, width_m: 2.5, rock_d50_m: 0.10, turbulence: 0.5, aeration: 0.5, ..Default::default() }
    }
    pub fn rapids() -> WaterConfig {
        WaterConfig { flow_ms: 2.5, depth_m: 0.4, width_m: 6.0, length_m: 5.0, rock_d50_m: 0.4, turbulence: 0.5, aeration: 0.6, ..Default::default() }
    }
    pub fn small_waterfall() -> WaterConfig {
        WaterConfig { flow_ms: 1.2, depth_m: 0.15, width_m: 1.5, length_m: 2.0, rock_d50_m: 0.3, turbulence: 0.7, aeration: 1.0, drop_height_m: 1.5, ..Default::default() }
    }
    pub fn drips() -> WaterConfig {
        WaterConfig { flow_ms: 0.0, depth_m: 0.3, width_m: 0.5, length_m: 0.5, drips_hz: 1.3, drop_height_m: 1.2, waves: 0.0, ..Default::default() }
    }
    pub fn trickle() -> WaterConfig {
        WaterConfig { flow_ms: 0.0, depth_m: 0.1, width_m: 0.4, length_m: 0.4, trickle: 0.6, drop_height_m: 0.3, ..Default::default() }
    }
    /// Sea surf on a sandy beach: plunging breakers, swash and a soft backwash.
    pub fn sea_surf() -> WaterConfig {
        WaterConfig { flow_ms: 0.0, depth_m: 1.5, width_m: 10.0, length_m: 40.0, rock_d50_m: 0.01, aeration: 1.0, surf: 1.2, swell_s: 9.0, shingle: 0.05, ..Default::default() }
    }
    /// Surf on a rocky, shingly shore under cliffs: harder crash, pebbles rattling back.
    pub fn rock_surf() -> WaterConfig {
        WaterConfig { surf: 1.5, swell_s: 10.0, shingle: 0.8, rock_d50_m: 0.3, ..Self::sea_surf() }
    }
    /// Mid-stream of a big lowland river: deep, wide, smooth; boils and eddies gulp now and then.
    pub fn big_river() -> WaterConfig {
        WaterConfig { flow_ms: 0.55, depth_m: 3.0, width_m: 12.0, length_m: 8.0, rock_d50_m: 0.04, turbulence: 0.04, aeration: 0.15, waves: 0.25, fetch_m: 2000.0, ..Default::default() }
    }
    /// The bank of a big river: slow water along the shore, small waves lapping.
    pub fn river_bank() -> WaterConfig {
        WaterConfig { flow_ms: 0.25, depth_m: 0.6, width_m: 4.0, length_m: 20.0, rock_d50_m: 0.06, turbulence: 0.1, aeration: 0.3, waves: 0.8, fetch_m: 3000.0, ..Default::default() }
    }

    pub fn preset(name: &str) -> Option<WaterConfig> {
        Some(match name {
            "still_lake" => Self::still_lake(),
            "lake_lapping" => Self::lake_lapping(),
            "slow_river" => Self::slow_river(),
            "brook_pebbles" => Self::brook_pebbles(),
            "brook_rocks" => Self::brook_rocks(),
            "rapids" => Self::rapids(),
            "small_waterfall" => Self::small_waterfall(),
            "drips" => Self::drips(),
            "trickle" => Self::trickle(),
            "sea_surf" => Self::sea_surf(),
            "rock_surf" => Self::rock_surf(),
            "big_river" => Self::big_river(),
            "river_bank" => Self::river_bank(),
            _ => return None,
        })
    }

    pub const PRESETS: &'static [&'static str] =
        &["still_lake", "lake_lapping", "slow_river", "brook_pebbles", "brook_rocks", "rapids", "small_waterfall", "drips", "trickle", "sea_surf", "rock_surf", "big_river", "river_bank"];

    fn update(&mut self, c: &Controls, len: usize) {
        let sr = self.sample_rate;
        let dt = len as f32 / sr;
        let cfg = self.cfg.clone();
        // Slow, independent wandering of the local entrainment (eddies come and go), so the
        // river is not one uniform wall.
        // Kept shallow (σ 0.12, was 0.35): real brooks have almost no swell at 0.1–1 Hz; their
        // movement is the faster babble (`Babble`).
        self.wander = self.wander_ou.step(dt, 6.0, 0.12, &mut self.wander_rng).clamp(-0.5, 0.5).exp();
        if self.key_u < 0.0 {
            self.flow = Smooth::new(cfg.flow_ms);
        }
        let u = self.flow.step(cfg.flow_ms, dt, 0.6);
        let wind = c.env.wind_ms;
        let unchanged = !self.dirty && (u - self.key_u).abs() <= 0.01 * u.max(self.key_u) + 1e-5 && (wind - self.key_wind).abs() < 0.05;
        if unchanged {
            for b in [&mut self.bed_lo, &mut self.bed_hi, &mut self.splash, &mut self.murmur] {
                let t = b.gain.cur;
                b.gain.to(t, len);
            }
            for b in self.plunge.iter_mut().chain(self.ripple.iter_mut()).chain(self.wash.iter_mut()) {
                let t = b.gain.cur;
                b.gain.to(t, len);
            }
            self.rain.update(c.env.rain_mm_h, 0.0, 1.0, 1.0, 0.0, len, sr);
            return;
        }
        self.dirty = false;
        self.key_u = u;
        self.key_wind = wind;
        let mut hyd = hydraulics(&cfg, u);
        hyd.hinze_m = hyd.hinze_m.clamp(0.0008, hyd.r_max_m);
        if cfg.drop_height_m > 0.02 && cfg.flow_ms == 0.0 {
            hyd.plunge_w = 0.0;
        }
        self.law = RadiusLaw::new(R_MIN, hyd.hinze_m, hyd.r_max_m);

        // Slow, deep water: eddies and snags gulp now and then.
        // (Far rarer than one would think: a smooth river is mostly silent, with a soft
        // glug now and then.)
        self.snag_rate = (cfg.width_m * cfg.length_m * 0.025 * u * (1.0 - smoothstep(0.05, 0.2, hyd.u_turb))).min(0.7);
        let total = hyd.bubbles_per_s;
        // The budget follows the flow: about three quarters of the bubbles are rendered one by
        // one at any flow (up to 6× the base), the rest is bed. With a fixed 500/s budget,
        // WATER 0.8–1.0 turned 80–90 % of the bubbles into noise: a steady hiss with crest
        // 2.5 dB against 5–6 in the recordings, 0.2 onsets/s against ~3, every modulation band
        // 2–6 dB short; rendering all of them makes a sparse brook too tonal and clicky
        // (flatness −16 against −7 dB). ~72 % resolved matches the references best (brook
        // distance 0.4–0.6 from WATER 0.4 to 1.0).
        let base = self.resolved_per_s;
        let budget = (0.72 * total).min(12.0 * base).max(base.min(total) * 0.72);
        self.resolved = total.min(budget) / sr;
        let bed_rate = (total - budget).max(0.0) / sr;
        // Energy and spectral centre of the bubble population (deterministic quantiles).
        let (mut e_sum, mut lf_sum) = (0.0f32, 0.0f32);
        for i in 0..12 {
            let q = (i as f32 + 0.5) / 12.0;
            let r = if q < self.law.share_small {
                power_draw(self.law.r0, self.law.a, P_SMALL, q / self.law.share_small)
            } else {
                power_draw(self.law.a, self.law.r1, P_LARGE, (q - self.law.share_small) / (1.0 - self.law.share_small))
            };
            // With the same log-normal size jitter the rendered bubbles get (3-point
            // Gauss–Hermite in log r): with surface damping the energy is far from linear in r.
            for (w, z) in [(1.0 / 6.0, -1.732f32), (2.0 / 3.0, 0.0), (1.0 / 6.0, 1.732)] {
                let rj = r * (0.2 * z).exp();
                let e = bubble_energy(rj, sr) * w;
                e_sum += e;
                lf_sum += e * minnaert_hz(rj).ln();
            }
        }
        // Rendered bubbles carry amp × 0.7 × e^{0.3·N(0,1)}: E[(0.7·e^{0.3g})²] = 0.49·e^{0.18}.
        let e_mean = e_sum / 12.0 * 0.49 * 0.18f32.exp();
        let fc = (lf_sum / e_sum.max(1e-12)).exp().clamp(300.0, 8000.0);
        // A bubble's band-limited energy, rendered as noise: unit-noise energy = amp²·τ/2.
        // Split over two bands around the population's spectral centre.
        let bed = campbell_amp(bed_rate, e_mean) * core::f32::consts::FRAC_1_SQRT_2;
        self.bed_lo.svf.set(fc / 1.7, 1.4, sr);
        self.bed_hi.svf.set(fc * 1.7, 1.4, sr);
        self.bed_lo.gain.to(bed * band_norm(fc / 1.7, 1.4, sr), len);
        self.bed_hi.gain.to(bed * band_norm(fc * 1.7, 1.4, sr), len);
        // Spray falling back: hiss once the surface is really broken.
        let spray = 0.0004 * (total * smoothstep(0.25, 0.8, hyd.u_turb)).sqrt();
        self.splash.gain.to(spray * band_norm(4500.0, 0.7, sr), len);
        // Turbulent murmur of moving water (very quiet unless fast).
        let murmur = 0.00015 * u.powi(2) * (cfg.width_m * cfg.length_m).sqrt();
        self.murmur.gain.to(murmur * band_norm(250.0, 0.6, sr), len);
        // Surface-flow bed. Even a "smooth" river surface is never acoustically still: the
        // flow is sheared at the banks, at snags, weed and emergent stones, where capillary
        // wavelets break and entrain myriads of sub-millimetre to millimetre bubbles (2–10 kHz)
        // and the surface slaps softly (0.5–1 kHz). Too many and too small to resolve: a
        // Campbell bed whose power follows the turbulent dissipation reaching the surface,
        // ∝ u³ per unit area. In a brook it sits far under the bubbles; on a slow river it is
        // what one hears (the references are a continuous hiss whose 3–8 kHz were 20–25 dB
        // above our former near-silence between glugs).
        let ripple = 0.0044 * u.powf(1.5) * (cfg.width_m * cfg.length_m).sqrt();
        for (b, (w, f, q)) in self.ripple.iter_mut().zip(RIPPLE_BANDS) {
            b.gain.to(ripple * w * band_norm(f, q, sr), len);
        }

        // Plunge: impact noise and bubble-cloud thrum. A tall fall carries big air cavities and
        // the slug of water hitting the pool: the small-waterfall references have about as
        // much energy per Hz at 100 Hz as at 1 kHz (our spectrum used to start at 300 Hz and
        // was 10–15 dB short at 50–250 Hz), so a low band grows in with the fall height.
        let p = hyd.plunge_w;
        let amp = 0.0006 * p.sqrt();
        let low = smoothstep(0.3, 2.0, cfg.drop_height_m);
        for (k, (b, (w, f, q))) in self.plunge.iter_mut().zip(PLUNGE_BANDS).enumerate() {
            let w = if k == 0 { w * low } else { w };
            b.gain.to(amp * w * band_norm(f, q, sr) * 0.3, len);
        }
        // Breaking events: vortices shed by the stones, or packets of the falling sheet (whose
        // speed is the impact speed). Calm water does not chatter; rapids, mostly bed, a bit
        // less than a brook.
        let v_imp = if p > 0.0 { (2.0 * G * cfg.drop_height_m + u * u).sqrt() } else { u };
        let chatter = (0.3 * smoothstep(0.03, 0.15, hyd.u_turb) + 0.7 * smoothstep(0.1, 0.5, hyd.u_turb)) * (1.0 - 0.3 * smoothstep(0.6, 1.4, hyd.u_turb));
        // The more bubbles, the more their sum averages out: a swollen gentle brook keeps its
        // audible surging only if its bursts are deeper (×1 at ~700 bubbles/s, up to ×1.8, but
        // no deeper than 0.35). Rocky brooks and rapids already chatter that much (deeper made
        // them too jerky: distance 0.7 → 1.3–1.5).
        let dense = (hyd.bubbles_per_s / 700.0).max(1.0).powf(0.35).min(1.8);
        let depth = 0.6 * chatter;
        self.babble.tune(v_imp, cfg.rock_d50_m, depth.max((depth * dense).min(0.35)));
        // (A plunge's bed is a dense curtain: only a trace of it, or the small-waterfall
        // references' 1–16 Hz modulation is overshot by 2–4 dB.)
        self.flicker_depth = 0.8 * chatter * if p > 0.0 { 0.3 } else { 1.0 };
        let q = u * cfg.depth_m * cfg.width_m;
        let r_cloud = (0.12 + 0.25 * q.sqrt()).clamp(0.08, 1.5);
        let beta = 0.05;
        let f_cloud = (3.0 * 1.4 * P0 / (1000.0 * beta)).sqrt() / (core::f32::consts::TAU * r_cloud);
        self.cloud[0].tune(f_cloud, f_cloud / 3.0, sr);
        self.cloud[1].tune(f_cloud * 1.9, f_cloud * 1.9 / 3.0, sr);
        self.cloud_rate = if p > 0.0 { (4.0 + 0.5 * p.sqrt()).min(20.0) } else { 0.0 };
        self.cloud_amp = amp * 1.5 * (1.0 + 2.0 * low);

        // Wind waves on a lake shore.
        if cfg.waves > 0.0 {
            let uw = c.env.wind_ms.max(0.7);
            let x = G * cfg.fetch_m / (uw * uw);
            let fp = (3.5 * G / uw * x.powf(-0.33)).clamp(0.2, 2.5);
            let hs = 0.0016 * uw * uw / G * x.sqrt();
            self.wave_period = 1.0 / fp;
            self.wave_h = cfg.waves * (0.008 + hs).min(0.5);
        } else {
            self.wave_h = 0.0;
        }
        // The laps along the rest of the shore, each too far or too small to stand out: an
        // incoherent wash under the near laps (lake recordings never fall silent between laps:
        // p95−p10 of the level is 14 dB, ours was 32 dB). Power ∝ wave height × shore length.
        let wash = 0.0035 * EVENT_LEVEL * (self.wave_h / 0.03).sqrt() * (cfg.length_m / 6.0).sqrt();
        for (b, (w, f, q)) in self.wash.iter_mut().zip(WASH_BANDS) {
            b.gain.to(wash * w * band_norm(f, q, sr), len);
        }

        // Trickle: jet breaks into drops, a share entrains bubbles.
        self.trickle_rate = if cfg.trickle > 0.0 {
            let uj = 0.4 + 1.6 * cfg.trickle;
            let dj = 0.002 + 0.004 * cfg.trickle;
            uj / (4.5 * dj) * 0.4
        } else {
            0.0
        };

        let area = cfg.width_m * cfg.length_m;
        self.rain.area = area;
        self.rain.update(c.env.rain_mm_h, 0.0, 1.0, 1.0, 0.0, len, sr);
        self.hyd = hyd;
        self.rate_telemetry = total + self.trickle_rate;
    }

    fn lap(&mut self, at: u32) {
        let sr = self.sample_rate;
        let rng = &mut self.rng;
        let h = self.wave_h * (0.6 + 0.8 * (core::f32::consts::TAU * self.group).sin().abs()) * (0.4 * rng.gauss()).exp();
        let a = (h / 0.03).sqrt().min(3.0);
        // The wave slops against the stones in a few small slaps along the shore, not one
        // thud: two to four soft bursts spread over a fraction of a second.
        let slaps = 2 + (rng.next_u32() % 3) as u32;
        let mut t = 0.0f32;
        for k in 0..slaps {
            let w = if k == 0 { 1.0 } else { rng.range(0.25, 0.7) };
            self.grains.spawn(
                &GrainSpec {
                    amp: 0.024 * EVENT_LEVEL * a * w,
                    fc: rng.range(250.0, 700.0),
                    q: rng.range(0.5, 0.9),
                    attack: rng.range(0.008, 0.04),
                    decay: rng.range(0.06, 0.22),
                    delay: at + (t * sr) as u32,
                    ..Default::default()
                },
                sr,
            );
            t += rng.range(0.04, 0.16);
        }
        // Backwash through the gravel: a fizz of bursting foam and trickling water.
        self.grains.spawn(
            &GrainSpec { amp: 0.005 * EVENT_LEVEL * a, fc: rng.range(3000.0, 6000.0), q: 0.5, attack: 0.06, decay: 0.25, delay: at + (rng.range(0.1, 0.3) * sr) as u32, ..Default::default() },
            sr,
        );
        // Small bubbles entrained by the fold: many and soft, spread over the slosh (a louder
        // burst made the laps clicky: 20 ms events, 1.2–1.6 kHz 6 dB over the references).
        self.burst += 120.0 * a;
        // Now and then a pocket of air trapped under a stone: a short wobbling gulp.
        if rng.uniform() < (2.0 * h).min(0.15) {
            let r = rng.range(0.004, 0.01);
            let delay = at + (rng.range(0.02, 0.15) * sr) as u32;
            self.bubbles.spawn_damped(r, 0.025 * EVENT_LEVEL * a, rng.range(0.3, 0.6), delay, sr, surface_damping(r));
        }
    }

    fn drip(&mut self, at: u32) {
        let sr = self.sample_rate;
        let rng = &mut self.rng;
        let v = fall_speed(4.6, self.cfg.drop_height_m.max(0.1));
        let a = (v / 4.0).powf(1.5);
        self.grains.spawn(&GrainSpec { amp: 0.1 * EVENT_LEVEL * a, fc: rng.range(1200.0, 2200.0), q: 0.8, attack: 0.0001, decay: 0.001, delay: at, ..Default::default() }, sr);
        if rng.uniform() < 0.8 {
            let r = rng.range(0.0015, 0.0035);
            self.bubbles.spawn(r, 0.5 * EVENT_LEVEL * a * rng.range(0.7, 1.1), rng.range(0.08, 0.2), at + (0.002 * sr) as u32, sr);
        }
    }
}

impl Generator for Water {
    fn set_model(&mut self, cfg: &super::ModelConfig) {
        if let super::ModelConfig::Water(c) = cfg {
            if *c != self.cfg {
                self.set_config(*c);
            }
        }
    }

    fn render(&mut self, out: &mut [f32], c: &Controls) {
        for chunk in out.chunks_mut(BLOCK) {
            self.render_block(chunk, c);
        }
    }

    fn event_rate(&self) -> f32 {
        self.rate_telemetry
    }
}

impl Water {
    fn render_block(&mut self, out: &mut [f32], c: &Controls) {
        let sr = self.sample_rate;
        let len = out.len();
        let dt = len as f32 / sr;
        self.update(c, len);
        // Chatter: brooks over stones a lot, rapids (mostly bed) and calm water little.
        self.babble.step(dt);
        let gate = self.babble.gate;
        let grow = self.babble.grow;
        let law = self.law;
        let drip_inc = if self.cfg.drips_hz > 0.0 { self.cfg.drips_hz / sr } else { 0.0 };
        let wave_inc = if self.wave_h > 0.0 { 1.0 / (self.wave_period * sr) } else { 0.0 };
        self.group = (self.group + dt / (8.0 * self.wave_period)) % 1.0;
        let burst_decay = (-1.0 / (0.12 * sr)).exp();
        let mut white = [0.0f32; BLOCK];
        for w in white[..len].iter_mut() {
            *w = self.rng.bipolar();
        }
        let white = &white[..len];
        out.fill(0.0);
        let surfing = self.cfg.surf > 0.0 || self.surf.active();
        if surfing {
            let cfg = self.cfg;
            self.surf.update(&cfg, dt, len, sr, &mut self.rng);
        }

        // Events at their sample offsets.
        let clouds = self.cloud_rate > 0.0 || self.cloud.iter().any(|m| m.energy() > 1e-14);
        for i in 0..len {
            let at = i as u32;
            let n = self.ev_bubble.tick(self.resolved * self.wander * gate + self.burst / sr, &mut self.rng);
            for _ in 0..n {
                let small_burst = self.burst > 1.0 && self.rng.uniform() < 0.5;
                // Log-normal jitter softens the hard edges of the size law (no spectral ridge).
                let r = if small_burst { self.rng.range(0.0005, 0.0025) } else { (law.draw(&mut self.rng) * grow * (0.2 * self.rng.gauss()).exp()).min(law.r1 * 1.3) };
                // (Amplitude spread σ 0.3, was 0.5, same mean energy: fewer loud outliers. It
                // lengthened the brook's events, 90 → 110 ms, but barely moved its onset rate.)
                let amp = bubble_amp(r) * (0.3 * self.rng.gauss()).exp() * 0.7 * if small_burst { 0.12 * EVENT_LEVEL } else { 1.0 };
                // Turbulent flow drags bubbles to the surface faster: stronger chirp.
                let xi = self.rng.range(0.0, 0.1 + 0.25 * (self.hyd.u_turb / 0.5).min(1.0));
                self.bubbles.spawn_damped(r, amp, xi, at, sr, surface_damping(r));
            }
            self.burst *= burst_decay;
            if self.snag_rate > 0.0 && self.ev_snag.tick(self.snag_rate / sr, &mut self.rng) > 0 {
                // A pocket of air gulped behind a snag or stone: it breaks into a few bubbles
                // of different sizes within a few tens of ms, with a soft slosh of the water
                // closing over it. A short, low "glug", never a clean tone.
                let rng = &mut self.rng;
                let strength = rng.range(0.4, 1.0);
                let r = rng.range(0.003, 0.007);
                self.bubbles.spawn_damped(r, bubble_amp(r) * 0.06 * EVENT_LEVEL * strength, rng.range(0.1, 0.3), at, sr, surface_damping(r));
                for _ in 0..2 + rng.next_u32() % 3 {
                    let rs = rng.range(0.0012, 0.003);
                    let d = at + (rng.range(0.005, 0.07) * sr) as u32;
                    self.bubbles.spawn_damped(rs, bubble_amp(rs) * 0.08 * EVENT_LEVEL * strength * rng.range(0.4, 1.0), rng.range(0.1, 0.4), d, sr, surface_damping(rs));
                }
                let spec = GrainSpec { amp: 0.012 * EVENT_LEVEL * strength, fc: rng.range(250.0, 600.0), q: 0.7, attack: 0.02, decay: rng.range(0.1, 0.3), delay: at, ..Default::default() };
                self.grains.spawn(&spec, sr);
            }
            if wave_inc > 0.0 {
                self.wave_clock += wave_inc;
                if self.wave_clock >= 1.0 {
                    self.wave_clock -= 1.0 + 0.25 * self.rng.gauss().clamp(-2.0, 2.0) * 0.5;
                    self.lap(at);
                }
            }
            if drip_inc > 0.0 {
                self.drip_clock += drip_inc;
                if self.drip_clock >= 1.0 {
                    self.drip_clock -= 1.0 + 0.25 * self.rng.bipolar();
                    self.drip(at);
                }
            }
            if surfing && self.surf.rattle_rate > 0.0 && self.surf.ev_rattle.tick(self.surf.rattle_rate, &mut self.rng) > 0 {
                // A pebble knocked over by the backwash.
                let rng = &mut self.rng;
                let spec = GrainSpec { amp: 0.05 * EVENT_LEVEL * rng.range(0.3, 1.2), fc: rng.range(2500.0, 7000.0), q: 0.9, attack: 0.0002, decay: rng.range(0.002, 0.007), delay: at, ..Default::default() };
                self.grains.spawn(&spec, sr);
            }
            if self.trickle_rate > 0.0 && self.ev_trickle.tick(self.trickle_rate / sr, &mut self.rng) > 0 {
                let r = self.rng.range(0.0006, 0.002);
                self.bubbles.spawn(r, bubble_amp(r) * 1.5 * EVENT_LEVEL, self.rng.range(0.05, 0.2), at, sr);
            }
            if clouds {
                if self.ev_cloud.tick(self.cloud_rate * gate / sr, &mut self.rng) > 0 {
                    let a = self.cloud_amp * self.rng.range(0.4, 1.0);
                    self.cloud[0].strike(a);
                    self.cloud[1].strike(a * 0.4);
                }
                out[i] += self.cloud[0].tick() + self.cloud[1].tick();
            }
        }
        self.bubbles.render_add(out, sr);
        self.grains.render_add(out);
        let mut bed = [0.0f32; BLOCK];
        let bed = &mut bed[..len];
        for b in [&mut self.bed_lo, &mut self.bed_hi, &mut self.splash, &mut self.murmur] {
            b.render_add(white, bed);
        }
        // The unresolved bubbles and the plunge follow the entrainment bursts (power ∝ rate).
        for b in self.plunge.iter_mut() {
            b.render_add(white, bed);
        }
        // The unresolved bubbles are no steady hiss either: hundreds of them come and go in
        // every few tens of ms, so the bed flickers at 16–64 Hz. A stationary bed left every
        // brook 4–5 dB short there (`am_16-64` −17 dB against −12 in the references). Log-normal
        // power flicker, ~30 Hz low-passed, mean 1.
        let a = 1.0 - (-core::f32::consts::TAU * 30.0 * dt).exp();
        self.flicker += a * (self.flicker_rng.gauss() - self.flicker);
        let sd = (a / (2.0 - a)).sqrt();
        let k = self.flicker_depth;
        let fl = (k * self.flicker / sd - 0.5 * k * k).exp();
        self.bed_mod.to((self.wander * gate * fl).sqrt(), len);
        for (o, b) in out.iter_mut().zip(bed.iter()) {
            *o += b * self.bed_mod.next();
        }
        self.bed_mod.finish();
        // The ripple bed through a gentle 3 kHz low-pass: the smallest bubbles are damped by
        // the wavelets that make them (the references fall ~−3 dB/octave above 2 kHz, the
        // band skirts alone left 4–12 kHz 3–4 dB too strong).
        let mut rip = [0.0f32; BLOCK];
        let rip = &mut rip[..len];
        for b in self.ripple.iter_mut() {
            b.render_add(white, rip);
        }
        let a = one_pole_coef(3000.0, sr);
        for (o, r) in out.iter_mut().zip(rip.iter()) {
            self.ripple_lp += a * (r - self.ripple_lp);
            *o += self.ripple_lp;
        }
        self.ripple_lp = flush(self.ripple_lp);
        // The wash swells and fades with the wave groups (sets of bigger waves), like the laps.
        let mut wash = [0.0f32; BLOCK];
        let wash = &mut wash[..len];
        for b in self.wash.iter_mut() {
            b.render_add(white, wash);
        }
        self.wash_mod.to(0.35 + 1.1 * (core::f32::consts::TAU * self.group).sin().abs(), len);
        for (o, w) in out.iter_mut().zip(wash.iter()) {
            *o += w * self.wash_mod.next();
        }
        self.wash_mod.finish();
        if surfing {
            for b in self.surf.crash.iter_mut() {
                b.render_add(white, out);
            }
            self.surf.hiss.render_add(white, out);
            self.surf.roar.render_add(white, out);
        }
        let mut rain = [0.0f32; BLOCK];
        let rain = &mut rain[..len];
        self.rain.render_add(rain, white, &mut self.rng, sr);
        let rain_level = self.cfg.rain_on_surface;
        self.out.to(self.cfg.gain, len);
        for (o, r) in out.iter_mut().zip(rain.iter()) {
            *o = (*o * WATER_LEVEL + r * rain_level) * self.out.next();
        }
        self.out.finish();
        for m in self.cloud.iter_mut() {
            if m.energy() < 1e-14 {
                m.clear();
            }
        }
        self.burst = flush(self.burst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RMS (dB) of a preset over `seconds`, with the scene's controls.
    fn level(cfg: WaterConfig, seconds: f32) -> (f32, f32) {
        let sr = 48_000.0;
        let mut w = Water::new(sr, 7, cfg);
        let c = Controls { water: 1.0, ..Default::default() };
        let mut buf = [0.0f32; 512];
        let (mut e, mut n, mut peak) = (0.0f64, 0usize, 0.0f32);
        let mut env = Vec::new();
        for _ in 0..(seconds * sr / 512.0) as usize {
            w.render(&mut buf, &c);
            let be: f64 = buf.iter().map(|v| (*v as f64).powi(2)).sum();
            env.push(be);
            e += be;
            n += 512;
            peak = peak.max(buf.iter().fold(0.0, |m, v| m.max(v.abs())));
        }
        // Short-term (0.5 s) level swing: breakers come and go.
        let k = 47;
        let st: Vec<f64> = env.chunks(k).map(|c| c.iter().sum::<f64>() / (k * 512) as f64).collect();
        let (lo, hi) = st.iter().fold((f64::MAX, 0.0f64), |a, v| (a.0.min(*v), a.1.max(*v)));
        (10.0 * (e / n as f64).log10() as f32, 10.0 * (hi / lo.max(1e-30)).log10() as f32)
    }

    #[test]
    fn surf_breaks_and_rivers_flow() {
        let (brook, _) = level(Water::brook_pebbles(), 10.0);
        let (surf, swing) = level(Water::sea_surf(), 40.0);
        let (rock, _) = level(Water::rock_surf(), 40.0);
        let (big, _) = level(Water::big_river(), 10.0);
        let (bank, _) = level(Water::river_bank(), 10.0);
        eprintln!("brook {brook:.1} dB, sea surf {surf:.1} dB (swing {swing:.1} dB), rock surf {rock:.1} dB, big river {big:.1} dB, bank {bank:.1} dB");
        // Surf along 40 m of beach is far louder than one brook segment, and breathes with the
        // breakers.
        assert!(surf > brook + 15.0, "surf {surf} vs brook {brook}");
        assert!(swing > 6.0, "surf swing {swing} dB");
        assert!(big > brook - 25.0 && big < brook + 10.0, "big river {big}");
        assert!(bank.is_finite() && rock.is_finite());
    }

    #[test]
    fn regimes_follow_hydraulics() {
        let rate = |c: WaterConfig| hydraulics(&c, c.flow_ms).bubbles_per_s;
        // A slow deep river barely entrains air; rapids entrain orders of magnitude more.
        assert!(rate(Water::slow_river()) < 1.0);
        assert!(rate(Water::brook_pebbles()) > 100.0);
        assert!(rate(Water::rapids()) > 50.0 * rate(Water::brook_pebbles()));
        // Violent turbulence shreds bubbles: smaller Hinze scale in rapids.
        let h = |c: WaterConfig| hydraulics(&c, c.flow_ms).hinze_m;
        assert!(h(Water::rapids()) < h(Water::brook_pebbles()));
        // Falls carry impact power.
        assert!(hydraulics(&Water::small_waterfall(), 1.2).plunge_w > 1000.0);
    }

    #[test]
    fn radius_law_respects_bounds() {
        let law = RadiusLaw::new(R_MIN, 0.0015, 0.006);
        let mut rng = Rng::new(9);
        let mut small = 0;
        for _ in 0..10_000 {
            let r = law.draw(&mut rng);
            assert!((R_MIN * 0.999..=0.006 * 1.001).contains(&r));
            if r < 0.0015 {
                small += 1;
            }
        }
        // r^-1.5 below the Hinze scale puts most bubbles there.
        assert!(small > 7000, "{small}");
    }
}
