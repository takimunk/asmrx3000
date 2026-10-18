//! Rain: drops from a Marshall–Palmer size distribution falling at terminal velocity onto a
//! surface, plus large drips shed by a wet canopy.
//!
//! * Drop size distribution (Marshall & Palmer 1948): N(D) = N₀·e^{−ΛD},
//!   N₀ = 8000 m⁻³mm⁻¹, Λ = 4.1·R^{−0.21} mm⁻¹ (R in mm/h).
//! * Terminal velocity (Atlas et al. 1973): v(D) = 9.65 − 10.3·e^{−0.6D} m/s (D in mm).
//! * Arrival flux per m² and per mm of diameter: N(D)·v(D). 1 mm/h is ~2000 drops/m²/s, most
//!   of them tiny and nearly silent; the few big drops make most of the sound.
//! * Each impact is a short noise burst + structural modes whose band, length and level depend
//!   on the surface, its wetness and the drop's size and speed (amplitude ∝ (D·v)^1.5).
//!   On water, drops of 0.8–1.1 mm entrain a bubble almost every time (Pumphrey & Crum's
//!   "type I" regime, ~14 kHz) and drops over 2.2 mm sometimes entrain a larger one; those
//!   ring as Minnaert bubbles.
//! * Rendering: drops larger than a split diameter (chosen so at most `resolved_per_s` drops
//!   per second are rendered one by one) are synthesised individually; the energy of all
//!   smaller drops goes into a noise bed with the same spectrum (Campbell's theorem). A
//!   small patch next to the listener is therefore mostly individual drops, a large distant
//!   one mostly a smooth bed, from the same physics.
//! * Canopy drips: drops of ~4.8 mm shed by wet leaves (drainage from the director's canopy
//!   water store), falling from `drip_height_m`; they reach v = v_t·sqrt(1 − e^{−2gh/v_t²}).

use serde::{Deserialize, Serialize};

use super::common::*;
use super::{Controls, Generator, BLOCK};
use crate::rng::Rng;

/// Smallest drop considered, mm (smaller ones are inaudible).
const D_MIN: f32 = 0.3;
/// Largest stable drop, mm (bigger ones break up).
const D_MAX: f32 = 6.0;
const N0: f32 = 8000.0;
const BINS: usize = 24;
/// Mean canopy drip diameter, mm.
const DRIP_D: f32 = 4.8;

/// Marshall–Palmer slope Λ (1/mm) for rain rate `r` mm/h.
pub fn mp_lambda(r: f32) -> f32 {
    4.1 * r.max(0.01).powf(-0.21)
}

/// Terminal fall speed of a raindrop of diameter `d` mm, m/s (Atlas et al. 1973).
pub fn terminal_velocity(d: f32) -> f32 {
    (9.65 - 10.3 * (-0.6 * d).exp()).max(0.1)
}

/// Drops per m² per second with diameter in [a, D_MAX] at rain rate `r` mm/h.
pub fn flux_above(r: f32, a: f32) -> f32 {
    if r <= 0.0 {
        return 0.0;
    }
    let l = mp_lambda(r);
    let l2 = l + 0.6;
    N0 * (9.65 * ((-l * a).exp() - (-l * D_MAX).exp()) / l - 10.3 * ((-l2 * a).exp() - (-l2 * D_MAX).exp()) / l2)
}

/// Volume of a drop of diameter `d` mm, mm³.
fn drop_volume(d: f32) -> f32 {
    core::f32::consts::PI / 6.0 * d * d * d
}

/// Drips per second per m² for a drainage rate of 1 mm/h (drops of [`DRIP_D`]).
pub fn drips_per_mm_h() -> f32 {
    1.0e6 / 3600.0 / drop_volume(DRIP_D)
}

/// Impact speed of a drop of diameter `d` mm after falling `h` m from rest.
pub fn fall_speed(d: f32, h: f32) -> f32 {
    let vt = terminal_velocity(d);
    vt * (1.0 - (-2.0 * G * h.max(0.0) / (vt * vt)).exp()).sqrt()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Surface {
    /// Broad leaves (canopy or understorey): taps with a leaf-plate mode.
    #[default]
    Leaves,
    /// Conifer needles: tiny high ticks, most drops intercepted silently.
    Needles,
    Grass,
    /// Bare soil: dull thuds, splats when wet.
    Soil,
    /// Leaf litter on the forest floor: crisp when dry, soft when soaked.
    Litter,
    /// Rock / stone: sharp clicks with a splash.
    Rock,
    /// Open water (lake, slow river): impact + entrained Minnaert bubbles.
    Water,
    /// Shallow puddles: like water, more splash.
    Puddle,
    /// Sheet metal roof: ringing plate modes ("tin roof").
    Roof,
    /// Wooden deck / log.
    Wood,
    /// Snow cover: almost silent.
    Snow,
}

impl Surface {
    /// Ground surfaces collect puddles as the ground gets wet.
    fn puddles_form(self) -> bool {
        matches!(self, Surface::Soil | Surface::Litter | Surface::Grass)
    }
    fn is_water(self) -> bool {
        matches!(self, Surface::Water | Surface::Puddle)
    }
    fn is_canopy(self) -> bool {
        matches!(self, Surface::Leaves | Surface::Needles)
    }
}

/// Impact timbre of a surface, dry and soaked. Interpolated by wetness.
#[derive(Clone, Copy)]
struct Tone {
    fc: f32,
    q: f32,
    decay: f32,
    level: f32,
    /// Structural mode (Hz, decay s, relative amplitude).
    mode: (f32, f32, f32),
    /// Splash tail: relative amplitude, band Hz, decay s.
    splash: (f32, f32, f32),
}

fn tone(s: Surface, wet: bool) -> Tone {
    let t = |fc, q, decay, level, mode, splash| Tone { fc, q, decay, level, mode, splash };
    match (s, wet) {
        (Surface::Leaves, false) => t(3200.0, 1.2, 0.0025, 1.0, (620.0, 0.006, 0.35), (0.0, 0.0, 0.0)),
        (Surface::Leaves, true) => t(2300.0, 1.1, 0.0035, 0.9, (480.0, 0.004, 0.25), (0.12, 4000.0, 0.004)),
        (Surface::Needles, false) => t(6500.0, 1.0, 0.0008, 0.35, (0.0, 0.0, 0.0), (0.0, 0.0, 0.0)),
        (Surface::Needles, true) => t(5000.0, 1.0, 0.0012, 0.3, (0.0, 0.0, 0.0), (0.1, 6000.0, 0.003)),
        (Surface::Grass, false) => t(4200.0, 0.9, 0.0015, 0.45, (0.0, 0.0, 0.0), (0.0, 0.0, 0.0)),
        (Surface::Grass, true) => t(3000.0, 0.9, 0.0022, 0.4, (0.0, 0.0, 0.0), (0.12, 5000.0, 0.004)),
        (Surface::Soil, false) => t(700.0, 0.8, 0.003, 0.5, (0.0, 0.0, 0.0), (0.0, 0.0, 0.0)),
        (Surface::Soil, true) => t(1100.0, 0.8, 0.002, 0.45, (0.0, 0.0, 0.0), (0.3, 4500.0, 0.006)),
        (Surface::Litter, false) => t(2800.0, 1.0, 0.002, 0.9, (900.0, 0.003, 0.2), (0.0, 0.0, 0.0)),
        (Surface::Litter, true) => t(1500.0, 0.9, 0.003, 0.55, (0.0, 0.0, 0.0), (0.2, 4000.0, 0.005)),
        (Surface::Rock, _) => t(5500.0, 0.7, 0.0004, 1.1, (0.0, 0.0, 0.0), (0.3, 7000.0, 0.009)),
        (Surface::Water, _) => t(2500.0, 0.8, 0.0008, 0.25, (0.0, 0.0, 0.0), (0.08, 6000.0, 0.004)),
        (Surface::Puddle, _) => t(2200.0, 0.8, 0.001, 0.35, (0.0, 0.0, 0.0), (0.18, 5500.0, 0.006)),
        (Surface::Roof, _) => t(4000.0, 0.8, 0.0003, 0.6, (980.0, 0.04, 0.5), (0.1, 6000.0, 0.004)),
        (Surface::Wood, _) => t(2500.0, 0.9, 0.0008, 0.7, (380.0, 0.012, 0.4), (0.1, 5000.0, 0.004)),
        (Surface::Snow, _) => t(900.0, 0.7, 0.001, 0.08, (0.0, 0.0, 0.0), (0.0, 0.0, 0.0)),
    }
}

fn mix(a: Tone, b: Tone, w: f32) -> Tone {
    let l = |x: f32, y: f32| lerp(x, y, w);
    Tone {
        fc: l(a.fc, b.fc),
        q: l(a.q, b.q),
        decay: l(a.decay, b.decay),
        level: l(a.level, b.level),
        mode: (l(a.mode.0, b.mode.0), l(a.mode.1, b.mode.1), l(a.mode.2, b.mode.2)),
        splash: (l(a.splash.0, b.splash.0), l(a.splash.1, b.splash.1), l(a.splash.2, b.splash.2)),
    }
}

/// Linear amplitude of an impact by drop diameter (mm) and speed (m/s).
#[inline]
fn impact_amp(d: f32, v: f32) -> f32 {
    0.1 * (d * v / 13.0).powf(1.5)
}

/// Impacts of rain and drips on one surface. Reused by `Rain` (ground patches), `Foliage`
/// (rain on the canopy) and `Water` (rain on the river).
pub struct RainLayer {
    pub surface: Surface,
    pub area: f32,
    /// Leaf size scale for leaf surfaces (0.06 m = birch).
    pub leaf_size: f32,
    pub resolved_per_s: f32,
    grains: GrainPool<40>,
    bubbles: BubbleBank<24>,
    ev_rain: Events,
    ev_drip: Events,
    bed: NoiseBand,
    bed_hi: NoiseBand,
    tone: Tone,
    split: f32,
    lambda: f32,
    rain_rate: f32,
    drip_rate: f32,
    drip_height: f32,
    puddles: f32,
    bins_d: [f32; BINS],
    bins_w: [f32; BINS],
    drops_per_s: f32,
    resolved_rate: f32,
    key: [f32; 8],
    key_surface: Option<Surface>,
    bed_target: [f32; 2],
}

impl RainLayer {
    pub fn new(surface: Surface, area: f32, sr: f32, rng: &mut Rng) -> Self {
        let mut bins_d = [0.0; BINS];
        let mut bins_w = [0.0; BINS];
        for i in 0..BINS {
            let a = D_MIN * (D_MAX / D_MIN).powf(i as f32 / BINS as f32);
            let b = D_MIN * (D_MAX / D_MIN).powf((i + 1) as f32 / BINS as f32);
            bins_d[i] = (a * b).sqrt();
            bins_w[i] = b - a;
        }
        let t = tone(surface, false);
        Self {
            surface,
            area: area.max(0.01),
            leaf_size: 0.06,
            resolved_per_s: 250.0,
            grains: GrainPool::new(),
            bubbles: BubbleBank::new(),
            ev_rain: Events::new(rng),
            ev_drip: Events::new(rng),
            bed: NoiseBand::new(t.fc, t.q, sr),
            bed_hi: NoiseBand::new(14_000.0, 2.0, sr),
            tone: t,
            split: D_MIN,
            lambda: mp_lambda(1.0),
            rain_rate: 0.0,
            drip_rate: 0.0,
            drip_height: 5.0,
            puddles: 0.0,
            bins_d,
            bins_w,
            drops_per_s: 0.0,
            resolved_rate: 0.0,
            key: [-1.0; 8],
            key_surface: None,
            bed_target: [0.0; 2],
        }
    }

    /// Drops per second hitting this layer right now (resolved + bed).
    pub fn drops_per_s(&self) -> f32 {
        self.drops_per_s
    }

    pub fn is_silent(&self) -> bool {
        self.rain_rate <= 0.0 && self.drip_rate <= 0.0 && self.bed_target == [0.0; 2] && self.grains.active() == 0 && self.bubbles.active() == 0 && self.bed.gain.cur < 1e-7 && self.bed_hi.gain.cur < 1e-7
    }

    /// Control-rate update. `rain` mm/h reaching the surface, `drips` drops/s on the whole
    /// layer, `wet` 0..1 surface wetness, `puddles` 0..1 ground covered by water.
    pub fn update(&mut self, rain: f32, drips: f32, drip_height: f32, wet: f32, puddles: f32, len: usize, sr: f32) {
        let wet = wet.clamp(0.0, 1.0);
        let rain = rain.max(0.0);
        let drips = drips.max(0.0);
        let puddles = if self.surface.puddles_form() { puddles.clamp(0.0, 0.9) } else { 0.0 };
        // The distribution maths is the expensive part; inputs move slowly (smoothed weather),
        // so recompute only when something changed by more than a couple of percent.
        let key = [rain, drips, drip_height, wet, puddles, self.area, self.leaf_size, self.resolved_per_s];
        let rel = |a: f32, b: f32, tol: f32| (a - b).abs() <= tol * a.abs().max(b.abs()) + 1e-6;
        let same = self.key_surface == Some(self.surface)
            && rel(key[0], self.key[0], 0.03)
            && rel(key[1], self.key[1], 0.05)
            && (key[3] - self.key[3]).abs() < 0.02
            && (key[4] - self.key[4]).abs() < 0.02
            && key[2] == self.key[2]
            && key[5..] == self.key[5..];
        if same {
            self.bed.gain.to(self.bed_target[0], len);
            self.bed_hi.gain.to(self.bed_target[1], len);
            return;
        }
        self.key = key;
        self.key_surface = Some(self.surface);

        self.tone = mix(tone(self.surface, false), tone(self.surface, true), wet);
        if self.surface.is_canopy() || self.surface == Surface::Litter {
            self.tone.fc *= (0.06 / self.leaf_size.clamp(0.005, 0.3)).powf(0.35);
            self.tone.mode.0 *= (0.06 / self.leaf_size.clamp(0.005, 0.3)).powf(0.5);
        }
        self.rain_rate = rain;
        self.drip_height = drip_height;
        self.puddles = puddles;
        let water = if self.surface.is_water() { 1.0 } else { self.puddles };

        // Split diameter: at most `resolved_per_s` individually rendered drops (drips get up
        // to half of the budget, the rest of them also goes into the bed).
        let cap = self.resolved_per_s.max(1.0);
        let drip_resolved = drips.min(0.5 * cap);
        self.drip_rate = drip_resolved;
        let cap = cap - drip_resolved;
        let total = if rain > 0.0 { flux_above(rain, D_MIN) * self.area } else { 0.0 };
        self.drops_per_s = total + drips;
        self.lambda = mp_lambda(rain);
        self.split = if total <= cap {
            D_MIN
        } else {
            let (mut lo, mut hi) = (D_MIN, D_MAX);
            for _ in 0..12 {
                let mid = 0.5 * (lo + hi);
                if flux_above(rain, mid) * self.area > cap {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            hi
        };
        self.resolved_rate = if rain > 0.0 { flux_above(rain, self.split) * self.area / sr } else { 0.0 };

        // Campbell bed: all drops below the split, plus the drips beyond the budget.
        let energy = |d: f32, v: f32, tone: &Tone| {
            let a = impact_amp(d, v) * tone.level;
            let mut lo = a * a * 0.5 * tone.decay * sr * (1.0 + tone.splash.0 * tone.splash.0);
            let mut hi = 0.0;
            if water > 0.0 {
                let (p, amp, r) = bubble_odds(d);
                hi = water * p * amp * amp * 0.25 * sr / bubble_damping(minnaert_hz(r));
                lo *= 1.0 - 0.5 * water;
            }
            (lo, hi)
        };
        let (mut e_lo, mut e_hi) = (0.0f32, 0.0f32);
        if rain > 0.0 && self.split > D_MIN {
            for i in 0..BINS {
                let d = self.bins_d[i];
                if d >= self.split {
                    break;
                }
                let v = terminal_velocity(d);
                let flux = N0 * (-self.lambda * d).exp() * v * self.bins_w[i] * self.area / sr;
                let (lo, hi) = energy(d, v, &self.tone);
                e_lo += flux * lo;
                e_hi += flux * hi;
            }
        }
        let extra = drips - drip_resolved;
        if extra > 0.0 {
            let (lo, hi) = energy(DRIP_D, fall_speed(DRIP_D, drip_height), &self.tone);
            e_lo += extra / sr * lo;
            e_hi += extra / sr * hi;
        }
        self.bed.svf.set(self.tone.fc, self.tone.q, sr);
        self.bed_target = [campbell_amp(1.0, e_lo), campbell_amp(1.0, e_hi)];
        self.bed.gain.to(self.bed_target[0], len);
        self.bed_hi.gain.to(self.bed_target[1], len);
    }

    /// Draw a drop diameter from the flux-weighted distribution above the split.
    fn draw_drop(&self, rng: &mut Rng) -> f32 {
        for _ in 0..8 {
            let d = self.split + exp1(rng) / self.lambda;
            if d <= D_MAX && rng.uniform() * 9.65 < terminal_velocity(d) {
                return d;
            }
        }
        self.split
    }

    fn impact(&mut self, d: f32, v: f32, at: u32, rng: &mut Rng, sr: f32) {
        let on_water = self.surface.is_water() || rng.uniform() < self.puddles;
        let base = if on_water && !self.surface.is_water() { tone(Surface::Puddle, false) } else { self.tone };
        let a = impact_amp(d, v) * base.level * rng.range(0.6, 1.3);
        let size = (2.0 / d).powf(0.25);
        let mut spec = GrainSpec {
            amp: a,
            fc: base.fc * size * rng.range(0.8, 1.25),
            q: base.q,
            attack: 0.00015,
            decay: base.decay * rng.range(0.7, 1.4),
            delay: at,
            ..Default::default()
        };
        if base.mode.2 > 0.0 {
            let f = if self.surface == Surface::Roof {
                const PLATE: [f32; 5] = [430.0, 980.0, 1650.0, 2900.0, 4100.0];
                PLATE[(rng.next_u32() % 5) as usize] * rng.range(0.97, 1.03)
            } else {
                base.mode.0 * rng.range(0.7, 1.4)
            };
            spec.modes[0] = (f, base.mode.1 * rng.range(0.7, 1.3), a * base.mode.2);
        }
        self.grains.spawn(&spec, sr);
        if base.splash.0 > 0.0 && d > 2.0 {
            let splash = GrainSpec {
                amp: a * base.splash.0,
                fc: base.splash.1 * rng.range(0.8, 1.2),
                q: 0.8,
                attack: 0.001,
                decay: base.splash.2 * rng.range(0.7, 1.3),
                delay: at + (rng.range(0.0005, 0.002) * sr) as u32,
                ..Default::default()
            };
            self.grains.spawn(&splash, sr);
        }
        // Dry leaf litter crackles: a second sub-click a millisecond later.
        if self.surface == Surface::Litter && self.tone.mode.2 > 0.1 && rng.uniform() < 0.5 {
            let mut c = spec;
            c.amp *= 0.5;
            c.fc *= 1.3;
            c.delay = at + (rng.range(0.0005, 0.0015) * sr) as u32;
            c.modes = [(0.0, 0.0, 0.0); 2];
            self.grains.spawn(&c, sr);
        }
        if on_water {
            let (p, amp, r) = bubble_odds(d);
            if rng.uniform() < p {
                let r = r * rng.range(0.85, 1.2);
                let delay = at + (rng.range(0.003, 0.012) * sr) as u32;
                self.bubbles.spawn(r, amp * rng.range(0.6, 1.2), rng.range(0.05, 0.3), delay, sr);
            }
        }
    }

    /// Adds this block of impacts and bed to `out`; `white` is the block's noise.
    pub fn render_add(&mut self, out: &mut [f32], white: &[f32], rng: &mut Rng, sr: f32) {
        if self.resolved_rate > 0.0 || self.drip_rate > 0.0 {
            let drip = self.drip_rate / sr;
            for i in 0..out.len() as u32 {
                for _ in 0..self.ev_rain.tick(self.resolved_rate, rng) {
                    let d = self.draw_drop(rng);
                    self.impact(d, terminal_velocity(d), i, rng, sr);
                }
                for _ in 0..self.ev_drip.tick(drip, rng) {
                    let d = (DRIP_D + 0.6 * rng.gauss()).clamp(3.5, 6.0);
                    let v = fall_speed(d, self.drip_height);
                    self.impact(d, v, i, rng, sr);
                }
            }
        }
        self.grains.render_add(out);
        self.bubbles.render_add(out, sr);
        self.bed.render_add(white, out);
        self.bed_hi.render_add(white, out);
    }

    pub fn end_block(&mut self) {}
}

/// Bubble entrainment by a drop of diameter `d` mm on water: (probability, amplitude,
/// radius m). Type I regular entrainment for 0.8–1.1 mm drops (~0.22 mm bubbles, ~14 kHz,
/// loud for their size); irregular entrainment of larger bubbles for drops over 2.2 mm.
fn bubble_odds(d: f32) -> (f32, f32, f32) {
    if (0.8..1.1).contains(&d) {
        (0.9, 0.025, 0.00022)
    } else if d > 2.2 {
        (0.4, 0.05 * (d / 4.0), 0.0004 + 0.0004 * (d - 2.2))
    } else {
        (0.0, 0.0, 0.001)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct RainConfig {
    /// What the drops hit.
    pub surface: Surface,
    /// Patch area represented by this emitter, m² (0.05..2000). Near patches small, the
    /// diffuse bed of a whole clearing large.
    pub area_m2: f32,
    /// Share of open-sky rain reaching the patch (1 open; ~0.25 free throughfall under a
    /// closed canopy; 0 under a roof).
    pub exposure: f32,
    /// Share of the canopy's drip falling on this patch (1 under a tree, 0 in the open).
    pub canopy_drip: f32,
    /// Height the canopy drips fall from, m (0.2..40).
    pub drip_height_m: f32,
    /// Leaf size for leaf surfaces and litter, m (0.005..0.3); smaller = higher pitched.
    pub leaf_size_m: f32,
    /// Fixed rain rate in mm/h, or negative to follow the environment (the normal case).
    pub rate_mm_h: f32,
    /// Fixed surface wetness 0..1, or negative to follow the environment.
    pub wetness: f32,
    /// Budget of individually synthesised drops per second (20..2000); the rest is bed.
    pub resolved_per_s: f32,
    /// Linear output gain (0..8).
    pub gain: f32,
}

impl Default for RainConfig {
    fn default() -> Self {
        Self {
            surface: Surface::Leaves,
            area_m2: 20.0,
            exposure: 1.0,
            canopy_drip: 0.0,
            drip_height_m: 5.0,
            leaf_size_m: 0.06,
            rate_mm_h: -1.0,
            wetness: -1.0,
            resolved_per_s: 150.0,
            gain: 1.0,
        }
    }
}

impl RainConfig {
    pub fn validate(&self) -> Result<(), String> {
        check("area_m2", self.area_m2, 0.05, 2000.0)?;
        check("exposure", self.exposure, 0.0, 1.0)?;
        check("canopy_drip", self.canopy_drip, 0.0, 1.0)?;
        check("drip_height_m", self.drip_height_m, 0.2, 40.0)?;
        check("leaf_size_m", self.leaf_size_m, 0.005, 0.3)?;
        check("rate_mm_h", self.rate_mm_h, -1.0, 200.0)?;
        check("wetness", self.wetness, -1.0, 1.0)?;
        check("resolved_per_s", self.resolved_per_s, 20.0, 2000.0)?;
        check("gain", self.gain, 0.0, 8.0)
    }
}

pub(crate) fn check(name: &str, v: f32, lo: f32, hi: f32) -> Result<(), String> {
    if v.is_finite() && v >= lo && v <= hi {
        Ok(())
    } else {
        Err(format!("{name} = {v} is outside {lo}..{hi}"))
    }
}

/// A patch of ground (or roof, rock, water) under the rain.
pub struct Rain {
    sample_rate: f32,
    rng: Rng,
    cfg: RainConfig,
    layer: RainLayer,
    out: Ramp,
}

impl Rain {
    pub fn new(sample_rate: f32, seed: u64, cfg: RainConfig) -> Self {
        let mut rng = Rng::new(seed);
        let mut layer = RainLayer::new(cfg.surface, cfg.area_m2, sample_rate, &mut rng);
        layer.leaf_size = cfg.leaf_size_m;
        layer.resolved_per_s = cfg.resolved_per_s;
        Self { sample_rate, rng, out: Ramp::new(cfg.gain), cfg, layer }
    }

    pub fn config(&self) -> &RainConfig {
        &self.cfg
    }

    /// Change parameters at run time; levels ramp, so there are no clicks.
    pub fn set_config(&mut self, cfg: RainConfig) {
        self.layer.surface = cfg.surface;
        self.layer.area = cfg.area_m2.max(0.01);
        self.layer.leaf_size = cfg.leaf_size_m;
        self.layer.resolved_per_s = cfg.resolved_per_s;
        self.cfg = cfg;
    }

    pub fn light_drizzle() -> RainConfig {
        RainConfig { surface: Surface::Leaves, area_m2: 40.0, rate_mm_h: 0.5, wetness: 0.6, ..Default::default() }
    }
    pub fn steady_rain() -> RainConfig {
        RainConfig { surface: Surface::Litter, area_m2: 30.0, rate_mm_h: 4.0, wetness: 0.8, ..Default::default() }
    }
    pub fn downpour() -> RainConfig {
        RainConfig { surface: Surface::Soil, area_m2: 60.0, rate_mm_h: 30.0, wetness: 1.0, ..Default::default() }
    }
    /// Close leaves around the listener (follows the environment).
    pub fn on_leaves() -> RainConfig {
        RainConfig { surface: Surface::Leaves, area_m2: 6.0, resolved_per_s: 250.0, ..Default::default() }
    }
    pub fn on_soil() -> RainConfig {
        RainConfig { surface: Surface::Soil, area_m2: 20.0, ..Default::default() }
    }
    pub fn on_rock() -> RainConfig {
        RainConfig { surface: Surface::Rock, area_m2: 3.0, ..Default::default() }
    }
    pub fn on_water() -> RainConfig {
        RainConfig { surface: Surface::Water, area_m2: 30.0, ..Default::default() }
    }
    pub fn puddles() -> RainConfig {
        RainConfig { surface: Surface::Puddle, area_m2: 4.0, ..Default::default() }
    }
    pub fn tin_roof() -> RainConfig {
        RainConfig { surface: Surface::Roof, area_m2: 12.0, ..Default::default() }
    }
    pub fn wood_deck() -> RainConfig {
        RainConfig { surface: Surface::Wood, area_m2: 8.0, ..Default::default() }
    }
    pub fn meadow() -> RainConfig {
        RainConfig { surface: Surface::Grass, area_m2: 80.0, resolved_per_s: 120.0, ..Default::default() }
    }
    /// Forest floor under a tree: throughfall plus canopy drips on the litter.
    pub fn under_canopy() -> RainConfig {
        RainConfig { surface: Surface::Litter, area_m2: 12.0, exposure: 0.25, canopy_drip: 1.0, resolved_per_s: 150.0, ..Default::default() }
    }
    /// Needle litter under a conifer: softer, fewer but longer-lasting drips.
    pub fn under_conifer() -> RainConfig {
        RainConfig { surface: Surface::Soil, area_m2: 12.0, exposure: 0.15, canopy_drip: 0.8, drip_height_m: 7.0, resolved_per_s: 150.0, ..Default::default() }
    }

    pub fn preset(name: &str) -> Option<RainConfig> {
        Some(match name {
            "light_drizzle" => Self::light_drizzle(),
            "steady_rain" => Self::steady_rain(),
            "downpour" => Self::downpour(),
            "on_leaves" => Self::on_leaves(),
            "on_soil" => Self::on_soil(),
            "on_rock" => Self::on_rock(),
            "on_water" => Self::on_water(),
            "puddles" => Self::puddles(),
            "tin_roof" => Self::tin_roof(),
            "wood_deck" => Self::wood_deck(),
            "meadow" => Self::meadow(),
            "under_canopy" => Self::under_canopy(),
            "under_conifer" => Self::under_conifer(),
            _ => return None,
        })
    }

    pub const PRESETS: &'static [&'static str] = &[
        "light_drizzle",
        "steady_rain",
        "downpour",
        "on_leaves",
        "on_soil",
        "on_rock",
        "on_water",
        "puddles",
        "tin_roof",
        "wood_deck",
        "meadow",
        "under_canopy",
        "under_conifer",
    ];

    /// Drops per second on this patch (telemetry).
    pub fn drops_per_s(&self) -> f32 {
        self.layer.drops_per_s()
    }
}

impl Generator for Rain {
    fn set_model(&mut self, cfg: &super::ModelConfig) {
        if let super::ModelConfig::Rain(c) = cfg {
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
}

impl Rain {
    fn render_block(&mut self, out: &mut [f32], c: &Controls) {
        let sr = self.sample_rate;
        let e = &c.env;
        let cfg = &self.cfg;
        let rain = if cfg.rate_mm_h >= 0.0 { cfg.rate_mm_h } else { e.rain_mm_h * cfg.exposure };
        let drips = if cfg.rate_mm_h >= 0.0 { 0.0 } else { e.drip_mm_h * cfg.canopy_drip * cfg.area_m2 * drips_per_mm_h() };
        let wet = if cfg.wetness >= 0.0 {
            cfg.wetness
        } else if cfg.surface.is_canopy() {
            e.leaf_wetness
        } else {
            e.ground_wetness
        };
        let puddles = if cfg.rate_mm_h >= 0.0 { 0.0 } else { e.puddles };
        self.layer.update(rain, drips, cfg.drip_height_m, wet, puddles, out.len(), sr);
        if self.layer.is_silent() {
            out.fill(0.0);
            return;
        }
        let len = out.len();
        let mut white = [0.0f32; BLOCK];
        for w in white[..len].iter_mut() {
            *w = self.rng.bipolar();
        }
        out.fill(0.0);
        self.layer.render_add(out, &white[..len], &mut self.rng, sr);
        self.out.to(cfg.gain, len);
        for o in out.iter_mut() {
            *o *= self.out.next();
        }
        self.out.finish();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marshall_palmer_flux_and_rain_rate() {
        // Integrate the volume flux back to a rain rate: should be the R we put in (MP is
        // a fit, so within ~35 %).
        for r in [1.0f32, 5.0, 25.0] {
            let l = mp_lambda(r);
            let mut vol = 0.0f64;
            let n = 2000;
            for i in 0..n {
                let d = D_MIN + (D_MAX - D_MIN) * (i as f32 + 0.5) / n as f32;
                let dd = (D_MAX - D_MIN) / n as f32;
                vol += (N0 * (-l * d).exp() * terminal_velocity(d) * drop_volume(d) * dd) as f64; // mm³/m²/s
            }
            let mm_h = vol * 3600.0 / 1e6;
            assert!((mm_h / r as f64 - 1.0).abs() < 0.35, "R {r}: {mm_h}");
        }
        // Heavier rain has more and bigger drops.
        assert!(flux_above(10.0, 3.0) > 5.0 * flux_above(1.0, 3.0));
        assert!((terminal_velocity(2.0) - 6.5).abs() < 0.3);
        assert!(fall_speed(4.8, 0.5) < fall_speed(4.8, 8.0));
    }
}
