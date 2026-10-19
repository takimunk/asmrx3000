//! Lightning and thunder: the light of a flash ([`Flash`], shared with the director's
//! weather so the picture and the sound come from the same strokes) and the sum of the
//! pressure pulses radiated by every piece of a tortuous lightning channel, each arriving at
//! its own time.
//!
//! **The flash.** A cloud-to-ground flash starts with a faint stepped leader (~15–35 ms),
//! then 1–5 return strokes down the same channel 30–120 ms apart (geometric mean ~60 ms;
//! subsequent strokes carry ~⅓–½ of the first one's current; Rakov & Uman 2003), sometimes
//! one with a continuing current that keeps the channel glowing for ~0.1–0.3 s. That is the
//! flicker. An intracloud flash shows no channel: a few diffuse surges of light inside the
//! cloud over half a second. Brightness falls with distance (a strike a few hundred metres
//! away is blinding, one 10 km away a dim sheet glow, also smeared longer by the cloud).
//!
//! **The thunder.**
//! * The channel is a 3D walk from the ground strike point up to the cloud base with two
//!   scales of tortuosity: straight-ish macro runs of ~30–300 m that change direction by tens
//!   of degrees, and ~8 m micro segments deviating ~16° from the run (Few 1969; Ribner & Roy
//!   1982). Runs seen broadside give the claps, runs seen end-on the rolls. Side branches
//!   hang off it, and a horizontal in-cloud part (kilometres long) adds the long rumble.
//! * Each segment's sudden heating radiates a weak shock, an N-wave whose initial length
//!   follows Few's relaxation radius R₀ = √(E_l/(π·p₀)) (E_l = energy per metre of channel,
//!   ~10⁵–10⁶ J/m, varying moderately along the channel and falling with height as the
//!   return-stroke current does): the dominant frequency is ≈ 0.63·c/R₀. Weak-shock
//!   propagation lengthens the N-wave: ∝ r^¼ while a run looks like a line, then only
//!   ∝ √(ln r) (spherical, weakening shock).
//! * Every return stroke re-heats the main channel: the same arrival pattern again, shifted
//!   by the stroke interval, weaker and with shorter N-waves (less energy). The crack of a
//!   close strike is that tearing, overlapping sequence of near-channel N-waves.
//! * Arrival time r/c spreads the pulses over seconds: the near part of the channel arrives
//!   first, the far part later. Spherical spreading and air absorption (ISO 9613-1 over each
//!   pulse's path, a four-pole fit) darken far thunder; within ~1 km the shock fronts
//!   regenerate what absorption takes (the crack). Upward refraction shadows the low parts
//!   of the channel first, the cloud beyond ~20 km.
//! * Turbulence scatters each pulse into a coda of weaker, stretched copies of itself (micro
//!   multipath, growing faster than range): the rumble keeps the spectrum of the N-waves instead of
//!   being a filtered noise. Slow refractive fading (a few dB at ~0.1–1 Hz, growing with
//!   range) makes the level swell.
//! * Echoes from hills, forest edges and the cloud base: a few smeared, darker delayed copies
//!   of the whole thunder (0.3–4 s near, up to ~10 s for a far strike).
//!
//! The emitter is a direction proxy (the director places it ~40 m away in the direction of
//! the strike, so the spatialiser gives the right bearing); the long-range physics is done
//! here. Sound starts r/c after the flash (3 s per km).

use serde::{Deserialize, Serialize};

use super::common::*;
use super::rain::check;
use super::{Controls, Generator};
use crate::rng::Rng;

const MAX_PULSES: usize = 6144;
/// Main-channel segments remembered for the subsequent return strokes.
const MAX_SEGS: usize = 1024;
const RING: usize = 1 << 17;
const LOOKAHEAD: f32 = 0.25;
/// Listener ear height above ground, m.
const EAR: f32 = 1.7;
/// Echo delay line (a power of two, ≥ 9.5 s at 48 kHz).
const ECHO: usize = 1 << 19;
const TAPS: usize = 7;
/// Air-absorption table: path lengths from `ABS_R0` to `ABS_R1` m, log spaced.
const ABS_N: usize = 48;
const ABS_R0: f32 = 20.0;
const ABS_R1: f32 = 60_000.0;
/// Path attenuations (dB, ISO 9613-1) at which the four one-pole low-passes of the
/// absorption filter take over (fitted: within ~2 dB of ISO up to 30–40 dB of loss).
const ABS_POLES_DB: [f32; 4] = [4.0, 14.0, 20.0, 30.0];
/// Radius of curvature of sound rays in the storm's temperature lapse, m (c / |dc/dz|,
/// ~6.5 K/km): a source at height z is heard on the ground out to √(2·R·z) + √(2·R·ear).
const RAY_RADIUS: f32 = 80_000.0;
/// Below this the channel radiates as a line (cylindrical weak shock, period ∝ r^¼),
/// beyond as a point (spherical, period ∝ √ln r), m: about one tortuosity run.
const LINE_M: f32 = 120.0;
/// Most return strokes in one flash.
pub const MAX_STROKES: usize = 5;
/// Output high-pass, Hz.
const HP_HZ: f32 = 42.0;

/// The timing of one lightning flash: when its strokes happen and how strong they are.
/// Deterministic in (strike counter, stroke count), so the director's flash and the thunder
/// generator agree without passing more than two numbers.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Flash {
    /// Intracloud flash (no ground channel, no return strokes): diffuse light and a rumble.
    pub cloud: bool,
    /// Stepped-leader time before the first return stroke, s (0 for an intracloud flash).
    pub leader: f32,
    /// Return strokes (or the surges of an intracloud flash).
    pub n: usize,
    /// Their times after the start of the flash, s.
    pub t: [f32; MAX_STROKES],
    /// Their strength relative to the first return stroke.
    pub a: [f32; MAX_STROKES],
    /// The stroke followed by a continuing current (glows longer), or `MAX_STROKES`.
    pub lingering: usize,
}

impl Flash {
    /// `strokes` 0 = intracloud flash, 1..=5 = return strokes of a cloud-to-ground flash.
    pub fn new(strike: u32, strokes: u8) -> Flash {
        let mut rng = Rng::new(0x7A11_F1A5 ^ (strike as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let mut f = Flash { cloud: strokes == 0, lingering: MAX_STROKES, ..Default::default() };
        if f.cloud {
            f.n = 3 + (rng.next_u32() % 3) as usize;
            let mut t = rng.range(0.0, 0.04);
            for k in 0..f.n {
                f.t[k] = t;
                f.a[k] = rng.range(0.45, 1.0);
                t += rng.range(0.06, 0.2);
            }
        } else {
            f.n = (strokes as usize).min(MAX_STROKES);
            f.leader = rng.range(0.015, 0.035);
            let mut t = f.leader;
            for k in 0..f.n {
                f.t[k] = t;
                f.a[k] = if k == 0 { 1.0 } else { (0.45 * (0.4 * rng.gauss()).exp()).clamp(0.2, 0.9) };
                t += (0.06 * (0.45 * rng.gauss()).exp()).clamp(0.03, 0.12);
            }
            if rng.uniform() < 0.4 {
                f.lingering = rng.next_u32() as usize % f.n;
            }
        }
        f
    }

    /// Picture brightness 0..1 at `age` s after the flash started, seen from `distance` m.
    pub fn brightness(&self, age: f32, distance: f32) -> f32 {
        if age < 0.0 || age > 2.5 || self.n == 0 {
            return 0.0;
        }
        let d = distance.max(1.0);
        // The cloud diffuses and smears the light of a far flash.
        let smear = 1.0 + d / 4000.0;
        let mut v = 0.0;
        if self.cloud {
            let near = 0.45 * (2500.0 / d).min(1.0).powf(0.9);
            for k in 0..self.n {
                let x = age - self.t[k];
                let rise = 0.03 * smear;
                if x > 0.0 {
                    v += self.a[k] * if x < rise { x / rise } else { (-(x - rise) / (0.12 * smear)).exp() };
                }
            }
            return (near * v).min(1.0).max(0.0) * (0.85 + 0.15 * (age * 31.0).sin().abs());
        }
        // Beyond full brightness a close strike saturates (blinding) rather than clips, so
        // every stroke still shows as a flicker.
        let near = (1300.0 / d).powf(1.2).clamp(0.05, 2.0);
        if age < self.leader {
            // The stepped leader: a faint, stepping glow.
            v = 0.06 * age / self.leader * (0.6 + 0.4 * (age * 1000.0 / 1.1).fract());
        }
        for k in 0..self.n {
            let x = age - self.t[k];
            if x < 0.0 {
                continue;
            }
            // Holds a frame (~12 ms) so every stroke shows at 60 fps, then dies away.
            let tau = if k == self.lingering { 0.18 } else { 0.03 } * smear;
            v += self.a[k] * if x < 0.012 { 1.0 } else { (-(x - 0.012) / tau).exp() };
        }
        if age > self.t[0] {
            // The lit rain and cloud fade more slowly.
            v += 0.12 * (-(age - self.t[0]) / 0.35).exp();
        }
        let x = near * v;
        if x < 0.7 {
            x
        } else {
            0.7 + 0.3 * (1.0 - (-(x - 0.7) / 0.3).exp())
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct ThunderConfig {
    /// Height of the channel top (cloud base), m (500..8000).
    pub cloud_base_m: f32,
    /// Mean change of direction between channel segments, degrees (3..45).
    pub tortuosity_deg: f32,
    /// Mean channel segment length, m (3..60).
    pub segment_m: f32,
    /// Energy per metre of channel, J/m (1e4..5e6): sets the N-wave length (pitch).
    pub energy_j_per_m: f32,
    /// Side branches (0..12).
    pub branches: u32,
    /// Share of in-cloud horizontal channel (0..1): longer, softer rolls. With a fixed
    /// distance, also the share of intracloud flashes.
    pub intracloud: f32,
    /// Strike distance, m (200..30000), or negative to take strikes from the environment.
    pub distance_m: f32,
    /// With a fixed distance: mean seconds between strikes (5..600).
    pub interval_s: f32,
    /// Linear output gain (0..8).
    pub gain: f32,
}

impl Default for ThunderConfig {
    fn default() -> Self {
        Self {
            cloud_base_m: 3000.0,
            tortuosity_deg: 16.0,
            segment_m: 8.0,
            energy_j_per_m: 5e4,
            branches: 5,
            intracloud: 0.3,
            distance_m: -1.0,
            interval_s: 30.0,
            gain: 1.0,
        }
    }
}

impl ThunderConfig {
    pub fn validate(&self) -> Result<(), String> {
        check("cloud_base_m", self.cloud_base_m, 500.0, 8000.0)?;
        check("tortuosity_deg", self.tortuosity_deg, 3.0, 45.0)?;
        check("segment_m", self.segment_m, 3.0, 60.0)?;
        check("energy_j_per_m", self.energy_j_per_m, 1e4, 5e6)?;
        check("branches", self.branches as f32, 0.0, 12.0)?;
        check("intracloud", self.intracloud, 0.0, 1.0)?;
        if self.distance_m >= 0.0 {
            check("distance_m", self.distance_m, 200.0, 30_000.0)?;
        }
        check("interval_s", self.interval_s, 5.0, 600.0)?;
        check("gain", self.gain, 0.0, 8.0)
    }
}

#[derive(Clone, Copy, Default)]
struct Pulse {
    /// Arrival time relative to `clock` origin, s.
    t: f32,
    amp: f32,
    /// N-wave length, s.
    period: f32,
    /// Path length over which air absorption acts (the shock part excluded), m.
    r_abs: f32,
    /// Extra darkening of the refraction shadow (scales the absorption poles, ≤ 1).
    dark: f32,
    /// Time spread by turbulent scattering, s.
    spread: f32,
}

/// Atmospheric absorption coefficient, dB/m, ISO 9613-1 (pure tones; `f` Hz, `t_c` °C,
/// `rh` relative humidity 0..1, sea-level pressure).
pub fn iso9613_db_per_m(f: f32, t_c: f32, rh: f32) -> f32 {
    let (f, tk) = (f as f64, t_c.clamp(-40.0, 50.0) as f64 + 273.15);
    let (t0, t01) = (293.15, 273.16);
    let psat = 10f64.powf(-6.8346 * (t01 / tk).powf(1.261) + 4.6151);
    let h = (rh.clamp(0.01, 1.0) as f64 * 100.0) * psat;
    let fro = 24.0 + 4.04e4 * h * (0.02 + h) / (0.391 + h);
    let frn = (tk / t0).powf(-0.5) * (9.0 + 280.0 * h * (-4.170 * ((tk / t0).powf(-1.0 / 3.0) - 1.0)).exp());
    let a = 8.686
        * f
        * f
        * (1.84e-11 * (tk / t0).sqrt()
            + (tk / t0).powf(-2.5) * (0.01275 * (-2239.1 / tk).exp() / (fro + f * f / fro) + 0.1068 * (-3352.0 / tk).exp() / (frn + f * f / frn)));
    a as f32
}

/// Pole frequencies of the absorption filter for path length `r` m: each one-pole takes
/// over where the ISO loss reaches its level in [`ABS_POLES_DB`].
fn absorption_poles(r: f32, t_c: f32, rh: f32) -> [f32; 4] {
    let mut out = [0.0; 4];
    for (k, db) in ABS_POLES_DB.iter().enumerate() {
        let (mut lo, mut hi) = (10.0f32.ln(), 40_000.0f32.ln());
        if iso9613_db_per_m(40_000.0, t_c, rh) * r < *db {
            out[k] = 40_000.0;
            continue;
        }
        for _ in 0..28 {
            let m = 0.5 * (lo + hi);
            if iso9613_db_per_m(m.exp(), t_c, rh) * r < *db {
                lo = m;
            } else {
                hi = m;
            }
        }
        out[k] = (0.5 * (lo + hi)).exp();
    }
    out
}

#[derive(Clone, Copy, Default)]
struct V {
    x: f32,
    y: f32,
    z: f32,
}

impl V {
    fn add(self, o: V, s: f32) -> V {
        V { x: self.x + o.x * s, y: self.y + o.y * s, z: self.z + o.z * s }
    }
    fn len(self) -> f32 {
        (self.x * self.x + self.y * self.y + self.z * self.z).sqrt()
    }
    fn norm(self) -> V {
        let l = self.len().max(1e-9);
        V { x: self.x / l, y: self.y / l, z: self.z / l }
    }
    fn cross_len(self, o: V) -> f32 {
        let c = V { x: self.y * o.z - self.z * o.y, y: self.z * o.x - self.x * o.z, z: self.x * o.y - self.y * o.x };
        c.len()
    }
}

/// One straight piece of channel: midpoint, direction, length, energy per metre (relative).
#[derive(Clone, Copy, Default)]
struct Seg {
    mid: V,
    dir: V,
    len: f32,
    e: f32,
}

/// One environmental echo: two close sub-taps (an extended reflector smears the echo).
#[derive(Clone, Copy, Default)]
struct Tap {
    d: [usize; 2],
    g: [f32; 2],
    a: f32,
    y1: f32,
    y2: f32,
}

/// Two-scale tortuous walk state.
struct Walk {
    dir: V,
    run: V,
    left: f32,
    /// Log-energy offset (correlated along the channel).
    le: f32,
    /// Energy factor of the current macro run.
    run_e: f32,
}

pub struct Thunder {
    sample_rate: f32,
    rng: Rng,
    cfg: ThunderConfig,
    pulses: Box<[Pulse]>,
    n: usize,
    next: usize,
    clock: f32,
    ring: Box<[f32]>,
    read: usize,
    segs: Box<[Seg]>,
    echo: Box<[f32]>,
    echo_w: usize,
    taps: [Tap; TAPS],
    /// Seconds since the last pulse was written (echo geometry changes only when quiet).
    quiet: f32,
    fade: Ou,
    flutter: Ou,
    fade_g: f32,
    /// Keeps the (mostly infrasonic) energy below ~40 Hz out, as recordings and ears do.
    hp: crate::dsp::Svf,
    last_strike: u32,
    timer: f32,
    distance: f32,
    temperature: f32,
    humidity: f32,
    /// Absorption-filter poles per path length (see `ABS_*`), and the weather they were
    /// computed for.
    absorb: [[f32; 4]; ABS_N],
    absorb_for: (f32, f32),
}

impl Thunder {
    pub fn new(sample_rate: f32, seed: u64, cfg: ThunderConfig) -> Self {
        let mut rng = Rng::new(seed);
        let timer = rng.range(0.5, 2.0);
        let mut t = Self {
            sample_rate,
            rng,
            pulses: vec![Pulse::default(); MAX_PULSES].into_boxed_slice(),
            n: 0,
            next: 0,
            clock: 0.0,
            ring: vec![0.0; RING].into_boxed_slice(),
            read: 0,
            segs: vec![Seg::default(); MAX_SEGS].into_boxed_slice(),
            echo: vec![0.0; ECHO].into_boxed_slice(),
            echo_w: 0,
            taps: [Tap::default(); TAPS],
            quiet: 1e9,
            fade: Ou::default(),
            flutter: Ou::default(),
            fade_g: 1.0,
            hp: crate::dsp::Svf::new(HP_HZ, 0.6, sample_rate),
            last_strike: 0,
            timer,
            distance: 0.0,
            temperature: 15.0,
            humidity: 0.75,
            absorb: [[0.0; 4]; ABS_N],
            absorb_for: (f32::NAN, f32::NAN),
            cfg,
        };
        t.new_echoes(2000.0);
        t.update_absorption();
        t
    }

    /// Recomputes the absorption table when temperature or humidity have moved noticeably
    /// (a few hundred ISO evaluations, only at a strike).
    fn update_absorption(&mut self) {
        let (t, h) = (self.temperature, self.humidity);
        if (t - self.absorb_for.0).abs() < 1.0 && (h - self.absorb_for.1).abs() < 0.03 {
            return;
        }
        self.absorb_for = (t, h);
        for k in 0..ABS_N {
            let r = ABS_R0 * (ABS_R1 / ABS_R0).powf(k as f32 / (ABS_N - 1) as f32);
            self.absorb[k] = absorption_poles(r, t, h);
        }
    }

    /// Absorption-filter poles for a path of `r` m (log-interpolated table).
    fn poles(&self, r: f32) -> [f32; 4] {
        if r <= ABS_R0 {
            return [40_000.0; 4];
        }
        let x = ((r / ABS_R0).ln() / (ABS_R1 / ABS_R0).ln() * (ABS_N - 1) as f32).min((ABS_N - 1) as f32 - 1e-3);
        let (i, u) = (x as usize, x.fract());
        let (a, b) = (self.absorb[i], self.absorb[i + 1]);
        core::array::from_fn(|k| a[k] * (b[k] / a[k]).powf(u))
    }

    pub fn config(&self) -> &ThunderConfig {
        &self.cfg
    }

    pub fn set_config(&mut self, cfg: ThunderConfig) {
        self.cfg = cfg;
    }

    /// Distance of the most recent strike, m.
    pub fn last_distance(&self) -> f32 {
        self.distance
    }

    /// Seconds until the first sound of the pending strike (negative once it is heard).
    pub fn time_to_first_arrival(&self) -> f32 {
        if self.next < self.n {
            self.pulses[self.next].t - self.clock
        } else {
            -1.0
        }
    }

    /// Arrival times of the pending pulses from now, s: (first, 10th, 50th, 90th percentile,
    /// last), weighted by energy for the percentiles. Zero when nothing is pending.
    pub fn arrival_spread(&self) -> [f32; 5] {
        let p = &self.pulses[self.next..self.n];
        if p.is_empty() {
            return [0.0; 5];
        }
        let total: f32 = p.iter().map(|q| q.amp * q.amp * q.period).sum();
        let mut out = [p[0].t - self.clock, 0.0, 0.0, 0.0, p[p.len() - 1].t - self.clock];
        let mut acc = 0.0;
        let mut k = 1;
        for q in p {
            acc += q.amp * q.amp * q.period;
            while k < 4 && acc >= [0.0, 0.1, 0.5, 0.9][k] * total {
                out[k] = q.t - self.clock;
                k += 1;
            }
        }
        out
    }

    /// The storm overhead follows the director's lightning.
    pub fn storm() -> ThunderConfig {
        ThunderConfig::default()
    }
    pub fn close_strike() -> ThunderConfig {
        ThunderConfig { distance_m: 700.0, interval_s: 25.0, branches: 7, intracloud: 0.15, ..Default::default() }
    }
    pub fn summer_storm() -> ThunderConfig {
        ThunderConfig { distance_m: 3000.0, interval_s: 20.0, ..Default::default() }
    }
    pub fn distant_rumble() -> ThunderConfig {
        ThunderConfig { distance_m: 12_000.0, interval_s: 18.0, intracloud: 0.5, ..Default::default() }
    }
    /// Long in-cloud rolls of a mature storm.
    pub fn rolling() -> ThunderConfig {
        ThunderConfig { distance_m: 5000.0, interval_s: 25.0, intracloud: 0.8, branches: 9, ..Default::default() }
    }

    pub fn preset(name: &str) -> Option<ThunderConfig> {
        Some(match name {
            "storm" => Self::storm(),
            "close_strike" => Self::close_strike(),
            "summer_storm" => Self::summer_storm(),
            "distant_rumble" => Self::distant_rumble(),
            "rolling" => Self::rolling(),
            _ => return None,
        })
    }

    pub const PRESETS: &'static [&'static str] = &["storm", "close_strike", "summer_storm", "distant_rumble", "rolling"];

    /// A ground strike at horizontal distance `distance` m with its own random strokes.
    pub fn strike(&mut self, distance: f32) {
        let id = self.rng.next_u32();
        let strokes = random_strokes(&mut self.rng);
        self.strike_with(distance, id, strokes);
    }

    /// Starts flash `id` (`strokes` as in [`Flash::new`]) at horizontal distance `distance` m;
    /// its sound starts r/c after the flash's return strokes. Real-time safe (fixed buffers).
    pub fn strike_with(&mut self, distance: f32, id: u32, strokes: u8) {
        let distance = distance.clamp(50.0, 40_000.0);
        self.distance = distance;
        let flash = Flash::new(id, strokes);
        // Keep the not yet rendered pulses of a previous strike, re-based to now.
        let keep = self.n - self.next;
        for i in 0..keep {
            let mut p = self.pulses[self.next + i];
            p.t -= self.clock;
            self.pulses[i] = p;
        }
        self.n = keep;
        self.next = 0;
        self.clock = 0.0;
        // The landscape's echoes depend on where the strike is; change them only when quiet.
        if self.quiet > 6.0 && keep == 0 {
            self.new_echoes(distance);
        }

        self.update_absorption();
        let c = sound_speed(self.temperature);
        let cfg = self.cfg;
        let sigma = cfg.tortuosity_deg.to_radians();
        let base = cfg.cloud_base_m * self.rng.range(0.8, 1.2);
        let lean = V { x: 0.15 * self.rng.gauss(), y: 0.15 * self.rng.gauss(), z: 1.0 };

        if flash.cloud {
            // Intracloud: a horizontal channel network 1–3 km above the cloud base; each
            // surge of light lights (and sounds) a stretch of it.
            let alt = base + self.rng.range(1000.0, 3000.0);
            let length = (2500.0 + 6000.0 * cfg.intracloud) * (0.35 * self.rng.gauss()).exp();
            let ang = self.rng.range(0.0, core::f32::consts::TAU);
            let start = V { x: distance - 0.5 * length * ang.cos(), y: -0.5 * length * ang.sin(), z: alt };
            let mut w = self.walk_start(V { x: ang.cos(), y: ang.sin(), z: 0.0 });
            let seg_len = 2.0 * cfg.segment_m;
            let total = (length / seg_len) as usize;
            let mut p = start;
            for i in 0..total.min(MAX_SEGS) {
                let k = (i * flash.n / total.max(1)).min(flash.n - 1);
                let (mid, dir, len, e) = self.walk_step(&mut w, &mut p, seg_len, sigma, 0.0, 0.25);
                let late = 0.15 * self.rng.uniform();
                self.emit(mid, dir, len, 0.45 * e * flash.a[k], flash.t[k] + late, c);
                if i % 120 == 60 {
                    self.branch(p, 0.3 * flash.a[k], flash.t[k], 0.0, sigma, c);
                }
            }
        } else {
            // Main channel, ground to cloud base.
            let ground = V { x: distance, y: 0.0, z: 0.0 };
            let mut w = self.walk_start(lean.norm());
            let mut p = ground;
            let mut nseg = 0;
            while p.z < base && nseg < MAX_SEGS {
                let len = cfg.segment_m * (0.5 * self.rng.gauss()).exp();
                let (mid, dir, len, e) = self.walk_step(&mut w, &mut p, len, sigma, 0.5, 0.0);
                // The return-stroke current dies away up the channel (MTLE, λ ≈ 2 km; Nucci
                // et al. 1990), and the energy deposited per metre with it: the lowest,
                // nearest stretch is the loudest — the crack — and the top rolls softer.
                let e = e * (-mid.z.max(0.0) / 2000.0).exp();
                self.segs[nseg] = Seg { mid, dir, len, e };
                nseg += 1;
                // The leader does not go underground.
                p.z = p.z.max(0.0);
            }
            let top = p;
            // Every return stroke re-heats the main channel.
            for k in 0..flash.n {
                for s in 0..nseg {
                    let sg = self.segs[s];
                    let jitter = (0.25 * self.rng.gauss()).exp();
                    // A subsequent stroke follows the leader up only part of the old channel.
                    if k > 0 && sg.mid.z > base * (0.6 + 0.4 * flash.a[k]) {
                        break;
                    }
                    self.emit(sg.mid, sg.dir, sg.len, sg.e * flash.a[k] * jitter, flash.t[k], c);
                }
            }
            // Side branches of the first stroke, mostly in the lower two thirds.
            for _ in 0..cfg.branches {
                let k = (self.rng.uniform() * self.rng.uniform() * nseg as f32) as usize;
                let from = self.segs[k.min(nseg.saturating_sub(1))].mid;
                let strength = self.rng.range(0.15, 0.35);
                self.branch(from, strength, flash.t[0], -0.6, sigma, c);
            }
            // The in-cloud part: horizontal, kilometres long, weaker, spreading from the top
            // of the channel in two legs.
            // (Lightning mapping arrays: the in-cloud part of a ground flash spreads 5–15 km.)
            let length = (4000.0 + 15_000.0 * cfg.intracloud) * (0.4 * self.rng.gauss()).exp();
            let ang = self.rng.range(0.0, core::f32::consts::TAU);
            let seg_len = 2.0 * cfg.segment_m;
            for leg in 0..2 {
                let a = ang + leg as f32 * self.rng.range(2.0, 4.3);
                let mut w = self.walk_start(V { x: a.cos(), y: a.sin(), z: 0.1 });
                let mut p = top;
                let share = if leg == 0 { 0.6 } else { 0.4 };
                let total = (share * length / seg_len) as usize;
                for i in 0..total.min(MAX_SEGS / 2) {
                    let (mid, dir, len, e) = self.walk_step(&mut w, &mut p, seg_len, sigma, 0.0, 0.25);
                    // In-cloud leaders creep (~10⁵ m/s): the far end lights a little later;
                    // the recoil streamers before each stroke re-light stretches of it.
                    let k = (i * 7 / 64 + leg) % flash.n;
                    let delay = flash.t[k] + i as f32 * seg_len / 1.5e5;
                    // Weaker currents than the return strokes: less energy per metre.
                    self.emit(mid, dir, len, 0.6 * e, delay, c);
                }
            }
        }
        self.pulses[..self.n].sort_unstable_by(|a, b| a.t.partial_cmp(&b.t).unwrap_or(core::cmp::Ordering::Equal));
    }

    fn walk_start(&mut self, dir: V) -> Walk {
        Walk { dir, run: dir, left: 0.0, le: 0.25 * self.rng.gauss(), run_e: 1.0 }
    }

    /// One micro segment of a two-scale walk. `up` keeps the macro runs climbing; `flat`
    /// squashes their vertical part (in-cloud channels run horizontally).
    fn walk_step(&mut self, w: &mut Walk, p: &mut V, len: f32, sigma: f32, up: f32, flat: f32) -> (V, V, f32, f32) {
        if w.left <= 0.0 {
            // In-cloud channels follow the charge layer: they keep their heading for
            // kilometres instead of wandering like a branch.
            let g = if up > 0.0 {
                0.5
            } else if flat > 0.0 {
                0.25
            } else {
                0.7
            };
            let mut r = V { x: w.run.x + g * self.rng.gauss(), y: w.run.y + g * self.rng.gauss(), z: w.run.z + g * self.rng.gauss() + up };
            if flat > 0.0 {
                r.z *= flat;
            }
            w.run = r.norm();
            w.left = 140.0 * (0.7 * self.rng.gauss()).exp();
            w.run_e = (0.3 * self.rng.gauss()).exp();
        }
        w.dir = V { x: w.run.x + sigma * self.rng.gauss(), y: w.run.y + sigma * self.rng.gauss(), z: w.run.z + sigma * self.rng.gauss() }.norm();
        w.le = 0.93 * w.le + 0.09 * self.rng.gauss();
        let mid = p.add(w.dir, 0.5 * len);
        *p = p.add(w.dir, len);
        w.left -= len;
        (mid, w.dir, len, w.le.exp() * w.run_e)
    }

    fn branch(&mut self, from: V, strength: f32, delay: f32, down: f32, sigma: f32, c: f32) {
        let ang = self.rng.range(0.0, core::f32::consts::TAU);
        let mut w = self.walk_start(V { x: ang.cos(), y: ang.sin(), z: down }.norm());
        let mut p = from;
        let segs = 15 + self.rng.next_u32() % 60;
        for _ in 0..segs {
            let len = self.cfg.segment_m * (0.5 * self.rng.gauss()).exp();
            let (mid, dir, len, e) = self.walk_step(&mut w, &mut p, len, sigma, down * 0.3, 0.0);
            self.emit(mid, dir, len, strength * e, delay, c);
            if p.z < 0.0 {
                break;
            }
        }
    }

    /// The N-wave of one channel segment heard at the listener.
    fn emit(&mut self, mid: V, dir: V, len: f32, rel_energy: f32, delay: f32, c: f32) {
        if self.n >= MAX_PULSES {
            return;
        }
        let to = V { x: -mid.x, y: -mid.y, z: EAR - mid.z };
        let r = to.len().max(1.0);
        let side = dir.cross_len(to.norm());
        let directivity = 0.12 + 0.88 * side * side;
        let e = self.cfg.energy_j_per_m * rel_energy;
        let r0 = (e / (core::f32::consts::PI * P0)).sqrt().max(0.05);
        let t0 = r0 / (0.63 * c);
        // Weak-shock lengthening: as a line (cylindrical, ∝ r^¼) while the run still looks
        // like a line, then as a point (spherical, ∝ √(ln r)), slowing as the shock weakens.
        let near = r.min(LINE_M);
        let stretch = (1.0 + near / (50.0 * r0)).powf(0.25) * (1.0 + 0.08 * (r / near).ln()).sqrt();
        let period = t0 * stretch;
        // While the overpressure is still a shock (≳ a few Pa, within ~1000·R₀ ≈ 0.3–1 km)
        // steepening regenerates the fronts faster than air absorbs them: the crack of a
        // near strike keeps its high frequencies. Absorption acts on the rest of the path.
        let rs = 2000.0 * r0;
        let r_abs = (r * r + rs * rs).sqrt() - rs;
        // Upward refraction: a source at height z reaches the ground only out to
        // √(2Rz) + √(2R·ear); beyond, diffraction and turbulent scattering leak a weak, dark
        // remnant into the shadow. Low parts of the channel go first, the cloud last (~20 km).
        let horizontal = (mid.x * mid.x + mid.y * mid.y).sqrt();
        let edge = (2.0 * RAY_RADIUS * mid.z.max(0.0)).sqrt() + (2.0 * RAY_RADIUS * EAR).sqrt();
        let depth_km = ((horizontal - edge) / 1000.0).max(0.0);
        let shadow_db = (12.0 * depth_km.powf(0.7)).min(30.0) + 20.0 * smoothstep(25_000.0, 35_000.0, horizontal);
        let shadow = 10f32.powf(-shadow_db / 20.0);
        let dark = 10f32.powf(-shadow_db / 40.0);
        // A lengthening weak shock spreads its energy over a longer wave: the amplitude falls
        // a little faster than 1/r (energy ∝ amp²·period conserved).
        let amp = directivity * (len / 15.0).sqrt() * (e / 5e5).sqrt() * shadow * 600.0 / (r * stretch.sqrt()) * (0.3 * self.rng.gauss()).exp();
        // Turbulence scatters part of each pulse into a coda that grows with range, faster
        // than linearly once the sound is scattered more than once (≳ 5 km).
        let spread = (0.012 + r / 30_000.0 * (1.0 + r / 8000.0)).min(1.0);
        self.pulses[self.n] = Pulse { t: delay + r / c, amp, period, r_abs, dark, spread };
        self.n += 1;
    }

    /// New echo geometry for a strike `distance` m away: reflectors from 0.3 s up to 4 s
    /// (near strike) or ~9 s (far strike), darker and weaker with delay.
    fn new_echoes(&mut self, distance: f32) {
        let sr = self.sample_rate;
        let far = smoothstep(1000.0, 10_000.0, distance);
        // Hills, forest edges and the cloud base anywhere around a path kilometres long:
        // the farther the strike, the larger the region that reflects back to us.
        let max = ((ECHO - 2) as f32 / sr).min(4.2 + 6.0 * far);
        // Far strikes come with more diffuse, relatively stronger echoes (the direct sound
        // is weak, the rumble long).
        let wet = 0.22 + 0.35 * far;
        for k in 0..TAPS {
            let u = (k as f32 + self.rng.uniform()) / TAPS as f32;
            let delay = (0.3 * (max / 0.3).powf(u)).min(max - 0.2);
            let smear = (0.02 + 0.03 * delay) * self.rng.range(0.6, 1.4);
            let g = wet * (0.4f32 / delay.max(0.3)).powf(0.5) * self.rng.range(0.6, 1.0);
            self.taps[k] = Tap {
                d: [(delay * sr) as usize, ((delay + smear) * sr) as usize],
                g: [g * 0.6, g * 0.5],
                a: one_pole_coef(1500.0 / (1.0 + 1.5 * delay), sr),
                y1: 0.0,
                y2: 0.0,
            };
        }
    }

    /// Writes one N-wave, low-passed by air absorption (four one-poles at `poles` Hz, scaled
    /// by `dark`), into the ring at `offset` samples.
    fn nwave(&mut self, offset: usize, amp: f32, period: f32, poles: [f32; 4], dark: f32) {
        let sr = self.sample_rate;
        let len = ((period * sr) as usize).max(2);
        if offset + 2 >= RING {
            return;
        }
        let a: [f32; 4] = core::array::from_fn(|k| one_pole_coef(poles[k] * dark, sr));
        // Long enough for the slowest pole to settle (a truncated tail is a step: a click).
        let tau = |k: usize| 1.0 / (core::f32::consts::TAU * (poles[k] * dark).min(0.49 * sr));
        let tail = ((14.0 * tau(0) + 5.0 * (tau(1) + tau(2) + tau(3))) * sr) as usize + 4;
        let total = (len + tail).min(RING - offset - 1);
        let mut y = [0.0f32; 4];
        let inv = 2.0 / len as f32;
        for i in 0..total {
            let x = if i < len { amp * (1.0 - inv * i as f32) } else { 0.0 };
            y[0] += (x - y[0]) * a[0];
            y[1] += (y[0] - y[1]) * a[1];
            y[2] += (y[1] - y[2]) * a[2];
            y[3] += (y[2] - y[3]) * a[3];
            let k = (self.read + offset + i) & (RING - 1);
            self.ring[k] += y[3];
        }
    }

    /// One pulse: the coherent N-wave plus its scattered coda of stretched, darker copies.
    fn write_pulse(&mut self, p: Pulse, offset: usize) {
        let sr = self.sample_rate;
        let coherent = (-p.spread * 4.0).exp();
        let poles = self.poles(p.r_abs);
        self.nwave(offset, p.amp * coherent, p.period, poles, p.dark);
        let copies = 3 + (12.0 * (p.spread / 0.1).min(1.0)) as usize;
        let rest = p.amp * (1.0 - coherent * coherent).sqrt() / (copies as f32).sqrt();
        let room = (RING as f32 / sr - LOOKAHEAD - 0.3).max(0.1);
        for _ in 0..copies {
            let d = (p.spread * exp1(&mut self.rng)).min(room);
            // Slightly smeared, not darker: turbulence scatters high frequencies at least as
            // well as low ones (what the coherent front loses, the coda carries).
            let stretch = self.rng.range(1.0, 1.1);
            // Mostly the same polarity: a scattered shock is still a compression first.
            let sign = if self.rng.uniform() < 0.75 { 1.0 } else { -1.0 };
            let a = rest * sign * (0.4 * self.rng.gauss()).exp() * 0.8 / stretch.sqrt();
            self.nwave(offset + (d * sr) as usize, a, p.period * stretch, poles, p.dark);
        }
    }
}

/// Return strokes of a cloud-to-ground flash: 1 (15 %), 2–5 otherwise (mean ~3).
pub fn random_strokes(rng: &mut Rng) -> u8 {
    let u = rng.uniform();
    if u < 0.15 {
        1
    } else if u < 0.4 {
        2
    } else if u < 0.65 {
        3
    } else if u < 0.85 {
        4
    } else {
        5
    }
}

impl Generator for Thunder {
    fn set_model(&mut self, cfg: &super::ModelConfig) {
        if let super::ModelConfig::Thunder(c) = cfg {
            if *c != self.cfg {
                self.set_config(*c);
            }
        }
    }

    fn render(&mut self, out: &mut [f32], c: &Controls) {
        let sr = self.sample_rate;
        let len = out.len();
        let dt = len as f32 / sr;
        self.temperature = c.env.temperature_c;
        self.humidity = c.env.humidity;
        if self.cfg.distance_m < 0.0 {
            if c.env.strike != self.last_strike {
                self.last_strike = c.env.strike;
                if c.env.strike_distance_m > 0.0 {
                    self.strike_with(c.env.strike_distance_m, c.env.strike, c.env.strike_strokes);
                }
            }
        } else {
            self.timer -= dt;
            if self.timer <= 0.0 {
                self.timer = self.cfg.interval_s * self.rng.range(0.5, 1.5);
                let d = self.cfg.distance_m * self.rng.range(0.8, 1.25);
                let id = self.rng.next_u32();
                let strokes = if self.rng.uniform() < 0.5 * self.cfg.intracloud { 0 } else { random_strokes(&mut self.rng) };
                self.strike_with(d, id, strokes);
            }
        }
        self.quiet += dt;
        while self.next < self.n && self.pulses[self.next].t <= self.clock + LOOKAHEAD {
            let p = self.pulses[self.next];
            let offset = ((p.t - self.clock).max(0.0) * sr) as usize;
            self.write_pulse(p, offset);
            self.next += 1;
            self.quiet = 0.0;
        }
        // Refractive fading: slow swells, deeper for far strikes, and a faster turbulent
        // flutter (gusts of the storm's wind along the path).
        let depth = 0.12 + 0.3 * smoothstep(500.0, 8000.0, self.distance);
        let slow = self.fade.step(dt, 1.2, 1.0, &mut self.rng);
        let fast = self.flutter.step(dt, 0.15, 1.0, &mut self.rng);
        let target = (depth * slow + 0.22 * fast).exp();
        let (g0, g1) = (self.fade_g, target);
        self.fade_g = target;
        let g = self.cfg.gain;
        let inv = 1.0 / len.max(1) as f32;
        for (i, o) in out.iter_mut().enumerate() {
            let k = self.read & (RING - 1);
            let dry = flush(self.ring[k]);
            self.ring[k] = 0.0;
            self.read = (self.read + 1) & (RING - 1);
            self.echo[self.echo_w] = dry;
            let mut wet = 0.0;
            for t in self.taps.iter_mut() {
                let x = t.g[0] * self.echo[(self.echo_w + ECHO - t.d[0]) & (ECHO - 1)] + t.g[1] * self.echo[(self.echo_w + ECHO - t.d[1]) & (ECHO - 1)];
                t.y1 += (x - t.y1) * t.a;
                t.y2 += (t.y1 - t.y2) * t.a;
                wet += t.y2;
            }
            self.echo_w = (self.echo_w + 1) & (ECHO - 1);
            let fade = g0 + (g1 - g0) * i as f32 * inv;
            let (_, _, hp) = self.hp.process(dry * fade + wet);
            *o = flush(hp) * g;
        }
        self.hp.flush();
        for t in self.taps.iter_mut() {
            t.y1 = flush(t.y1);
            t.y2 = flush(t.y2);
        }
        self.clock += dt;
        if self.next >= self.n {
            self.n = 0;
            self.next = 0;
            self.clock = 0.0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(distance: f32, seconds: f32) -> (Vec<f32>, f32) {
        let (buf, first, _) = render_spread(distance, seconds);
        (buf, first)
    }

    /// Also the energy-weighted arrival percentiles (`arrival_spread`).
    fn render_spread(distance: f32, seconds: f32) -> (Vec<f32>, f32, [f32; 5]) {
        let mut t = Thunder::new(48_000.0, 4, ThunderConfig { distance_m: -1.0, ..Default::default() });
        t.strike(distance);
        let first = t.time_to_first_arrival();
        let spread = t.arrival_spread();
        let mut buf = vec![0.0f32; (seconds * 48_000.0) as usize];
        for chunk in buf.chunks_mut(64) {
            t.render(chunk, &Controls::default());
        }
        (buf, first, spread)
    }

    #[test]
    fn thunder_arrives_after_flash_and_darkens_with_distance() {
        let (near, t_near) = render(1000.0, 16.0);
        let (far, t_far, spread) = render_spread(8000.0, 40.0);
        // 3 s per km.
        assert!((t_near - 1000.0 / 341.0).abs() < 0.3, "{t_near}");
        // A far flash's in-cloud part (kilometres long) may run towards the listener and be
        // heard first, but never later than the strike point, and the bulk of the energy
        // still comes from around the strike.
        assert!(t_far < 8000.0 / 341.0 + 0.3 && t_far > 8000.0 / 341.0 - 12.0, "{t_far}");
        assert!((spread[2] - 8000.0 / 341.0).abs() < 3.0, "{spread:?}");
        let energy = |b: &[f32]| b.iter().map(|v| (*v as f64).powi(2)).sum::<f64>();
        assert!(energy(&near) > 10.0 * energy(&far), "{} vs {}", energy(&near), energy(&far));
        // High-frequency share (first difference) drops with distance.
        let hf = |b: &[f32]| b.windows(2).map(|w| ((w[1] - w[0]) as f64).powi(2)).sum::<f64>() / energy(b).max(1e-30);
        assert!(hf(&near) > 2.0 * hf(&far), "{} vs {}", hf(&near), hf(&far));
        assert!(near.iter().chain(&far).all(|v| v.is_finite()));
    }

    /// The flash comes first; the thunder follows r/c after its first return stroke, when
    /// the light is long over.
    #[test]
    fn flash_precedes_first_sound_by_r_over_c() {
        for (id, strokes, d) in [(7u32, 3u8, 900.0f32), (8, 0, 2500.0), (9, 5, 400.0)] {
            let flash = Flash::new(id, strokes);
            let mut t = Thunder::new(48_000.0, 1, ThunderConfig::default());
            let env = crate::sources::Ambient { strike: id, strike_distance_m: d, strike_strokes: strokes, temperature_c: 15.0, ..Default::default() };
            let c = Controls { env, ..Default::default() };
            let mut buf = [0.0f32; 64];
            t.render(&mut buf, &c);
            let first = t.time_to_first_arrival() + 64.0 / 48_000.0;
            let speed = sound_speed(15.0);
            // The nearest piece of channel is the strike point (ground) or the cloud above.
            let expect = flash.t[0] + d / speed;
            if flash.cloud {
                // At least 1 km above the (lowest) cloud base.
                assert!(first > (3000.0 * 0.8 + 1000.0) / speed, "{first}");
            } else {
                assert!((first - expect).abs() < 0.08, "{first} vs {expect}");
            }
            assert!(flash.brightness(first, d) < 0.02, "light still on when the sound comes");
            assert!((0..500).map(|i| flash.brightness(i as f32 * 0.001, d)).fold(0.0f32, f32::max) > 0.1);
        }
    }

    #[test]
    fn flash_flickers_with_its_strokes_and_dims_with_distance() {
        let f = Flash::new(3, 4);
        assert_eq!(f.n, 4);
        for k in 1..f.n {
            let gap = f.t[k] - f.t[k - 1];
            assert!((0.029..=0.121).contains(&gap), "{gap}");
        }
        // Peaks: brightness rises again at every stroke.
        let mut rises = 0;
        let mut prev = 0.0;
        let mut falling = false;
        for i in 0..1000 {
            let v = f.brightness(i as f32 * 0.001, 600.0);
            if v > prev + 1e-4 && falling {
                rises += 1;
            }
            falling = v < prev;
            prev = v;
        }
        assert!(rises >= 3, "{rises}");
        let peak = |d: f32, f: &Flash| (0..1500).map(|i| f.brightness(i as f32 * 0.001, d)).fold(0.0f32, f32::max);
        assert!(peak(500.0, &f) > 0.95);
        assert!(peak(10_000.0, &f) < 0.2);
        let ic = Flash::new(3, 0);
        assert!(ic.cloud && peak(1000.0, &ic) < 0.6 * peak(1000.0, &f));
    }
}
