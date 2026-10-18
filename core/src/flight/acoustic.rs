//! Where the landscape and the sound meet (docs/world.md):
//!
//! * [`terrain_path`]: the terrain profile between an emitter and the listener, sampled at a
//!   few points from the tile caches. A hill or ridge that rises into the line of sight (or
//!   close to it, within the first Fresnel zone) diffracts the sound over its crest: the
//!   main edge and one secondary edge (Deygout's method) each attenuate by the knife-edge
//!   curve of ITU-R P.526, J(v) = 6.9 + 20·log10(√((v−0.1)² + 1) + v − 0.1) with the
//!   Fresnel–Kirchhoff parameter v = h·√(2(d₁+d₂)/(λd₁d₂)), capped like ISO 9613-2's barrier
//!   term (25 dB for a double edge). Low frequencies bend over a hill, high ones do not: a
//!   brook behind a rise is heard as a muffled murmur. The same profile gives the local
//!   ground under the specular reflection point (its height and surface), so the ground
//!   effect sees snow, sand, rock or water where the sound actually bounces.
//! * [`Echoes`]: slopes and cliffs that face the listener return the sound field as discrete
//!   echoes. Sixteen directions are scanned out to ~200 m for visible, steep terrain; the
//!   four strongest become image-source taps on a shared delay line (2r/c), with a gain from
//!   the face's steepness, height and surface (rock reflects, snow and forest scatter and
//!   absorb), 1/(2r) spreading and a roughness low-pass. They are encoded in the direction
//!   of the face, so in a valley the walls answer from both sides.
//! * The **enclosure** (mean elevation of the visible skyline) feeds the late reverb: a
//!   valley or a cirque rings longer, an open plain or a dune field is dry.

use crate::ambisonics::{self, CHANNELS};
use crate::delay::DelayLine;
use crate::engine::BLOCK;
use crate::math::Vec3;
use crate::shaping::NF;

use super::land::Land;
use super::terrain::Surface;

/// Profile samples between an emitter and the listener.
const PROFILE: usize = 10;
/// Most attenuation the terrain gives a path, dB (ISO 9613-2: 20 dB single, 25 dB double edge).
const MAX_LOSS_DB: f32 = 25.0;

/// Knife-edge diffraction loss (ITU-R P.526), dB, for Fresnel parameter v.
#[inline]
pub fn knife_edge_db(v: f32) -> f32 {
    if v <= -0.78 {
        0.0
    } else {
        6.9 + 20.0 * (((v - 0.1) * (v - 0.1) + 1.0).sqrt() + v - 0.1).log10()
    }
}

/// What the terrain does to one path.
#[derive(Clone, Copy, Debug)]
pub struct PathTerrain {
    /// Geometric Fresnel factor of the main and secondary edges (v = g·√(f/c)); NaN = none.
    pub main: f32,
    pub second: f32,
    /// Weight of the edges by how much they stand out of the mean ground (0..1): flat ground
    /// is the ground effect's business, not a barrier's.
    pub weight: f32,
    /// Ground height under the specular reflection point, m.
    pub ground: f32,
    pub surface: Surface,
}

impl PathTerrain {
    pub const NONE: Self = Self { main: f32::NAN, second: f32::NAN, weight: 0.0, ground: 0.0, surface: Surface::Grass };

    /// Spectral loss on the propagation's fit grid, dB (negative).
    pub fn loss_db(&self, freqs: &[f64; NF], c: f32, out: &mut [f32; NF]) {
        if !(self.main.is_finite()) || self.weight <= 0.0 {
            out.fill(0.0);
            return;
        }
        for (o, f) in out.iter_mut().zip(freqs.iter()) {
            let s = (*f as f32 / c).sqrt();
            let mut a = knife_edge_db(self.main * s);
            if self.second.is_finite() {
                a += knife_edge_db(self.second * s);
            }
            *o = -(a.min(MAX_LOSS_DB) * self.weight);
        }
    }
}

/// The terrain between an emitter at `s` and the listener at `l` (world x, y and absolute
/// height z).
pub fn terrain_path(land: &Land, s: [f64; 3], l: [f64; 3]) -> PathTerrain {
    let (dx, dy) = (l[0] - s[0], l[1] - s[1]);
    let dh = (dx * dx + dy * dy).sqrt();
    let gs = land.ground(s[0], s[1]);
    let gl = land.ground(l[0], l[1]);
    // Specular point on the local ground: where the image path meets it.
    let hs = (s[2] as f32 - gs).max(0.05);
    let hr = (l[2] as f32 - gl).max(0.05);
    let tr = (hs / (hs + hr)) as f64;
    let (rx, ry) = (s[0] + dx * tr, s[1] + dy * tr);
    let (_, _, surface) = land.nearest(rx, ry);
    let ground = land.ground(rx, ry);
    let mut out = PathTerrain { ground, surface, ..PathTerrain::NONE };
    if dh < 8.0 {
        return out;
    }
    let dz = l[2] - s[2];
    let d3 = (dh * dh + dz * dz).sqrt();
    let mut h = [0.0f32; PROFILE];
    let mut t = [0.0f64; PROFILE];
    for k in 0..PROFILE {
        t[k] = (k as f64 + 1.0) / (PROFILE as f64 + 1.0);
        h[k] = land.top(s[0] + dx * t[k], s[1] + dy * t[k]);
    }
    // Main edge: the largest Fresnel factor along the line of sight.
    let edge = |a: usize, b: usize, za: f64, zb: f64, ta: f64, tb: f64| -> Option<(usize, f32, f32)> {
        let mut best: Option<(usize, f32, f32)> = None;
        for k in a..b {
            let u = (t[k] - ta) / (tb - ta);
            if !(0.02..0.98).contains(&u) {
                continue;
            }
            let los = za + (zb - za) * u;
            let e = h[k] as f64 - los;
            let (d1, d2) = (u * (tb - ta) * d3, (1.0 - u) * (tb - ta) * d3);
            let g = e * (2.0 * (d1 + d2) / (d1 * d2)).sqrt();
            let plane = gs as f64 + (gl - gs) as f64 * t[k];
            let prominence = (h[k] as f64 - plane) as f32;
            if best.map_or(true, |x| g as f32 > x.1) {
                best = Some((k, g as f32, prominence));
            }
        }
        best
    };
    let Some((k, g, prom)) = edge(0, PROFILE, s[2], l[2], 0.0, 1.0) else { return out };
    // Edges more than ~0.8 below the line of sight at the highest grid frequency do nothing.
    if g < -0.78 * (343.0f32 / 16_000.0).sqrt() {
        return out;
    }
    out.main = g;
    out.weight = smoothstep(0.5, 3.0, prom);
    // Secondary edge on the longer side, against the line to the main edge's top.
    let top = h[k] as f64;
    let second = if t[k] > 0.5 { edge(0, k, s[2], top, 0.0, t[k]) } else { edge(k + 1, PROFILE, top, l[2], t[k], 1.0) };
    if let Some((_, g2, p2)) = second {
        if g2 > -0.5 && p2 > 1.0 {
            out.second = g2;
        }
    }
    out
}

#[inline]
fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Reflection of a slope's surface (amplitude), and its roughness low-pass, Hz.
fn reflectance(s: Surface) -> (f32, f32) {
    match s {
        Surface::Rock => (0.9, 5000.0),
        Surface::Water => (0.9, 6000.0),
        Surface::Sand => (0.55, 3500.0),
        Surface::Grass => (0.55, 3000.0),
        // Trees scatter most of it away; snow absorbs (σ ≈ 20 kPa·s/m²).
        Surface::Forest => (0.3, 1800.0),
        Surface::Snow => (0.3, 1500.0),
    }
}

const AZIMUTHS: usize = 16;
const RANGES: [f32; 10] = [10.0, 16.0, 24.0, 34.0, 48.0, 66.0, 90.0, 120.0, 155.0, 195.0];
pub const REFLECTORS: usize = 4;
/// Echo gain reference distance: a source this far from the listener is about as loud at
/// the listener as at the reflector.
const REF_M: f32 = 25.0;

#[derive(Clone, Copy, Debug, Default)]
pub struct Reflector {
    /// Direction from the listener in the travel frame (unit).
    pub dir: Vec3,
    pub dist: f32,
    pub gain: f32,
    pub lowpass_hz: f32,
}

#[derive(Clone, Copy)]
struct Tap {
    want: Reflector,
    delay: f64,
    gain: f32,
    lp: f32,
    lp_coef: f32,
    sh: [f32; CHANNELS],
}

/// Slope echoes (see the module docs). Audio thread: fixed buffers, no allocation.
pub struct Echoes {
    sample_rate: f32,
    line: DelayLine,
    input: [f32; BLOCK],
    tmp: [f32; BLOCK],
    taps: [Tap; REFLECTORS],
    /// Mean sine of the skyline elevation all around, 0 (open plain) .. ~0.5 (deep valley).
    pub enclosure: f32,
    /// Reflectors found by the last scan (for tests and telemetry).
    pub found: [Reflector; REFLECTORS],
}

impl Echoes {
    pub fn new(sample_rate: f32) -> Self {
        let max = (2.0 * 230.0 / 330.0 * sample_rate) as usize;
        Self {
            sample_rate,
            line: DelayLine::new(max),
            input: [0.0; BLOCK],
            tmp: [0.0; BLOCK],
            taps: [Tap { want: Reflector::default(), delay: 0.0, gain: 0.0, lp: 0.0, lp_coef: 1.0, sh: [0.0; CHANNELS] }; REFLECTORS],
            enclosure: 0.0,
            found: [Reflector::default(); REFLECTORS],
        }
    }

    /// Looks around the listener at (x, y, ear height z) facing `heading` for slopes that
    /// answer, and measures the enclosure. Control rate (every few blocks).
    pub fn scan(&mut self, land: &Land, x: f64, y: f64, ear: f32, heading: f64, scale: f32) {
        let mut best = [(0.0f32, Reflector::default()); AZIMUTHS];
        let mut skyline = 0.0f32;
        for (k, slot) in best.iter_mut().enumerate() {
            let a = core::f64::consts::TAU * k as f64 / AZIMUTHS as f64;
            let (s, c) = (heading + a).sin_cos();
            let mut max_el = -1.0f32;
            let mut prev = (0.0f32, land.ground(x, y));
            for &r in RANGES.iter() {
                let (px, py) = (x + c * r as f64, y + s * r as f64);
                let h = land.top(px, py);
                let el = (h - ear) / r;
                let visible = el >= max_el - 0.02;
                let rise = h - prev.1;
                let slope = rise / (r - prev.0);
                if visible && slope > 0.25 && rise > 1.0 {
                    let face = slope / (1.0 + slope * slope).sqrt();
                    let patch = (rise / 10.0).min(1.0);
                    let (refl, lp) = reflectance(land.surface_at(px, py));
                    let g = refl * face * patch * REF_M / (2.0 * r + REF_M) * scale;
                    if g > slot.0 {
                        let (ca, sa) = (a.cos() as f32, a.sin() as f32);
                        let e = el.atan();
                        let dir = Vec3::new(ca * e.cos(), sa * e.cos(), e.sin());
                        *slot = (g, Reflector { dir, dist: r - 0.5 * (r - prev.0), gain: g, lowpass_hz: lp });
                    }
                }
                max_el = max_el.max(el);
                prev = (r, h);
            }
            skyline += max_el.max(0.0).atan().sin();
        }
        self.enclosure = skyline / AZIMUTHS as f32;
        // The strongest few, not two from neighbouring directions.
        let mut used = [false; AZIMUTHS];
        for n in 0..REFLECTORS {
            let mut pick: Option<usize> = None;
            for k in 0..AZIMUTHS {
                if used[k] || best[k].0 <= 0.003 {
                    continue;
                }
                if pick.map_or(true, |p| best[k].0 > best[p].0) {
                    pick = Some(k);
                }
            }
            match pick {
                Some(k) => {
                    used[k] = true;
                    used[(k + 1) % AZIMUTHS] = true;
                    used[(k + AZIMUTHS - 1) % AZIMUTHS] = true;
                    self.found[n] = best[k].1;
                }
                None => self.found[n] = Reflector::default(),
            }
        }
        // Keep each tap on the reflector nearest its current one (fewer jumps).
        let mut taken = [false; REFLECTORS];
        let mut order = [usize::MAX; REFLECTORS];
        for (i, tap) in self.taps.iter().enumerate() {
            let mut bestj: Option<(usize, f32)> = None;
            for (j, r) in self.found.iter().enumerate() {
                if taken[j] || r.gain <= 0.0 {
                    continue;
                }
                let d = (r.dist - tap.want.dist).abs() + 30.0 * (1.0 - r.dir.dot(tap.want.dir)).max(0.0);
                if bestj.map_or(true, |b| d < b.1) {
                    bestj = Some((j, d));
                }
            }
            if let Some((j, _)) = bestj {
                taken[j] = true;
                order[i] = j;
            }
        }
        for (i, tap) in self.taps.iter_mut().enumerate() {
            tap.want = if order[i] == usize::MAX { Reflector { gain: 0.0, ..tap.want } } else { self.found[order[i]] };
        }
    }

    /// Adds an emitter's block to the echo input: its contribution to the field at the
    /// listener (`gain` includes 1/distance).
    #[inline]
    pub fn add(&mut self, x: &[f32], gain: f32) {
        if gain <= 0.0 {
            return;
        }
        for (o, v) in self.input.iter_mut().zip(x.iter()) {
            *o += v * gain;
        }
    }

    /// Renders the echoes of this block's input into the bus (travel frame).
    pub fn render(&mut self, bus: &mut [[f32; BLOCK]; CHANNELS], len: usize, c: f32) {
        let start = self.line.write(&self.input[..len]);
        self.input[..len].fill(0.0);
        let sr = self.sample_rate;
        let fade = len as f32 / (0.08 * sr);
        for tap in self.taps.iter_mut() {
            let want_delay = (2.0 * tap.want.dist / c * sr) as f64;
            let (d0, mut g1) = (tap.delay, tap.want.gain);
            // A different reflector: fade out, jump, fade in (no chirp); otherwise glide.
            let jump = (want_delay - d0).abs() > 0.02 * sr as f64;
            let d1 = if jump && tap.gain > 1e-4 {
                g1 = 0.0;
                d0
            } else {
                want_delay
            };
            let g0 = tap.gain;
            let g1 = if g1 > g0 { (g0 + fade * g1.max(0.01)).min(g1) } else { (g0 - fade * g0.max(0.01)).max(g1) };
            tap.delay = d1;
            tap.gain = g1;
            if g0 <= 0.0 && g1 <= 0.0 {
                continue;
            }
            tap.lp_coef = crate::sources::common::one_pole_coef(tap.want.lowpass_hz.max(200.0), sr);
            ambisonics::encode(tap.want.dir, &mut tap.sh);
            let x = &mut self.tmp[..len];
            x.fill(0.0);
            let d0c = d0.clamp(2.0, self.line.max_delay() - 2.0);
            let d1c = d1.clamp(2.0, self.line.max_delay() - 2.0);
            self.line.read_linear_add(start, d0c, d1c, g0, g1, x);
            let mut lp = tap.lp;
            for v in x.iter_mut() {
                lp += (*v - lp) * tap.lp_coef;
                *v = lp;
            }
            tap.lp = crate::sources::common::flush(lp);
            for (ch, row) in bus.iter_mut().enumerate() {
                let s = tap.sh[ch];
                for (o, v) in row[..len].iter_mut().zip(x.iter()) {
                    *o += v * s;
                }
            }
        }
    }
}
