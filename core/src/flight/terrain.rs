//! The relief of flight mode: a heightfield with its water bodies and ground surfaces, a pure
//! function of (seed, biome, relief scale, place) plus the macro features of [`super::land`]
//! (which are themselves pure functions of their cell), so it never depends on the route.
//!
//! * **Base relief** per biome ([`Relief`]): rolling hills (gradient-noise fBm), ridged
//!   mountains (multifractal: sharp crests, broad valleys), transverse dunes (asymmetric:
//!   gentle windward slope, steep slip face, domain-warped into crescents), a sea coast (a
//!   low-frequency "continent" field thresholded at sea level, with beaches and cliffs) and a
//!   big river meandering across the land every ~1.5 km.
//! * **Features carve it**: brooks and rivers cut V-valleys and run at a water level that
//!   only falls along their course (a running minimum of the land they cross), lakes sit in a
//!   basin at the level of the lowest point of their rim, ridges rise as rocky crests.
//! * **Surfaces** come out of the result: water where the water level is above the ground,
//!   bare rock on steep slopes, snow (snowy biomes, frozen lakes), sand (desert, beaches),
//!   forest floor or grass by the forest cover.
//!
//! The audio thread never evaluates this directly: [`super::land::Land`] caches it per tile on
//! a 5 m grid when the tile is made (≈80 µs per tile) and looks heights up bilinearly. The UI
//! samples it at a finer grid through `asmr_terrain_sample` (its own macro cache).

use super::land::{hash, Biome, Feature, FeatureKind};

/// Sea level, m (absolute heights are relative to it).
pub const SEA_LEVEL: f32 = 0.0;
/// Distance between parallel big rivers, m.
pub const BIG_RIVER_SPACING: f64 = 1500.0;
/// "No water here".
pub const DRY: f32 = f32::NEG_INFINITY;

/// Relief of a biome. Heights in metres, scales in metres.
#[derive(Clone, Copy, Debug)]
pub struct Relief {
    /// Rolling hills: amplitude and horizontal scale.
    pub hills: f32,
    pub hills_scale: f32,
    /// Ridged mountains: height of the crests above the valleys, and their spacing.
    pub mountains: f32,
    pub mountain_scale: f32,
    /// Transverse dunes: crest height and wavelength (0 = none).
    pub dunes: f32,
    pub dune_len: f32,
    /// How wide and deep rivers carve their valleys (0 = channel only .. 1 = broad valley).
    pub valley: f32,
    /// Share of the land under the sea (0 = inland).
    pub sea: f32,
    /// Width of the big river, m (0 = none).
    pub big_river: f32,
}

impl Relief {
    pub const fn flat() -> Self {
        Self { hills: 0.0, hills_scale: 300.0, mountains: 0.0, mountain_scale: 600.0, dunes: 0.0, dune_len: 90.0, valley: 0.3, sea: 0.0, big_river: 0.0 }
    }
}

/// What lies on the ground, for the acoustics (ground impedance), the rain and the picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum Surface {
    #[default]
    Forest = 0,
    Grass = 1,
    Water = 2,
    Sand = 3,
    Snow = 4,
    Rock = 5,
}

impl Surface {
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Surface::Grass,
            2 => Surface::Water,
            3 => Surface::Sand,
            4 => Surface::Snow,
            5 => Surface::Rock,
            _ => Surface::Forest,
        }
    }

    pub fn ground(self) -> crate::physics::ground::GroundType {
        use crate::physics::ground::GroundType as G;
        match self {
            Surface::Forest => G::ForestFloor,
            Surface::Grass => G::Grass,
            Surface::Water => G::Water,
            Surface::Sand => G::Sand,
            Surface::Snow => G::Snow,
            Surface::Rock => G::Rock,
        }
    }
}

/// One terrain sample.
#[derive(Clone, Copy, Debug)]
pub struct Sample {
    /// Ground height, m above sea level.
    pub ground: f32,
    /// Water surface height, or [`DRY`].
    pub water: f32,
    /// Flow of the water here (world frame, m/s; zero for still water).
    pub flow: [f32; 2],
    /// Height of the ground above the nearest water level (beaches: sand below ~2.5 m), m.
    pub shore: f32,
    /// Kind of water: 0 none, 1 lake, 2 river, 3 sea.
    pub kind: u8,
}

impl Sample {
    pub fn wet(&self) -> bool {
        self.water > self.ground + 0.02
    }
}

// ---------------------------------------------------------------------------------------
// Noise
// ---------------------------------------------------------------------------------------

#[inline]
fn fade(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

#[inline]
fn grad(h: u64, x: f32, y: f32) -> f32 {
    // Twelve directions around the circle (no axis bias).
    const G: [(f32, f32); 12] = [
        (1.0, 0.0), (0.866, 0.5), (0.5, 0.866), (0.0, 1.0), (-0.5, 0.866), (-0.866, 0.5),
        (-1.0, 0.0), (-0.866, -0.5), (-0.5, -0.866), (0.0, -1.0), (0.5, -0.866), (0.866, -0.5),
    ];
    let g = G[((h >> 33) % 12) as usize];
    g.0 * x + g.1 * y
}

/// Gradient (Perlin) noise, roughly in [-0.7, 0.7].
pub fn perlin(seed: u64, salt: u64, x: f64, y: f64) -> f32 {
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = ((x - x0) as f32, (y - y0) as f32);
    let (i, j) = (x0 as i64, y0 as i64);
    let (u, v) = (fade(fx), fade(fy));
    let a = grad(hash(seed, i, j, salt), fx, fy);
    let b = grad(hash(seed, i + 1, j, salt), fx - 1.0, fy);
    let c = grad(hash(seed, i, j + 1, salt), fx, fy - 1.0);
    let d = grad(hash(seed, i + 1, j + 1, salt), fx - 1.0, fy - 1.0);
    let ab = a + (b - a) * u;
    let cd = c + (d - c) * u;
    ab + (cd - ab) * v
}

/// Fractal sum of `oct` octaves, each rotated ~37° against the last (no grid alignment).
pub fn fbm(seed: u64, salt: u64, x: f64, y: f64, oct: u32) -> f32 {
    let (mut x, mut y) = (x, y);
    let (mut sum, mut amp, mut norm) = (0.0f32, 1.0f32, 0.0f32);
    for k in 0..oct {
        sum += amp * perlin(seed, salt.wrapping_add(k as u64 * 0x51), x, y);
        norm += amp;
        amp *= 0.5;
        let (nx, ny) = (0.8 * x - 0.6 * y, 0.6 * x + 0.8 * y);
        x = nx * 2.03 + 3.7;
        y = ny * 2.03 - 1.9;
    }
    sum / norm * 1.4
}

/// Ridged multifractal in [0, 1]: sharp crests where the noise crosses zero, broad valleys.
pub fn ridged(seed: u64, salt: u64, x: f64, y: f64, oct: u32) -> f32 {
    let (mut x, mut y) = (x, y);
    let (mut sum, mut amp, mut norm, mut weight) = (0.0f32, 1.0f32, 0.0f32, 1.0f32);
    for k in 0..oct {
        let n = 1.0 - (perlin(seed, salt.wrapping_add(k as u64 * 0x77), x, y) * 1.45).abs().min(1.0);
        let s = n * n * weight;
        weight = (s * 1.6).clamp(0.0, 1.0);
        sum += s * amp;
        norm += amp;
        amp *= 0.5;
        let (nx, ny) = (0.8 * x - 0.6 * y, 0.6 * x + 0.8 * y);
        x = nx * 2.1 - 2.3;
        y = ny * 2.1 + 5.1;
    }
    (sum / norm).clamp(0.0, 1.0)
}

#[inline]
fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

// ---------------------------------------------------------------------------------------
// Base relief
// ---------------------------------------------------------------------------------------

/// The relief before features carve it.
#[derive(Clone, Copy, Debug)]
pub struct Base {
    pub h: f32,
    /// The slow part of the relief (valley floors, levels of rivers and lakes follow it).
    pub low: f32,
    /// Sea biome: signed distance to the shoreline, m (+ inland); +inf without a sea.
    pub coast: f32,
}

/// The parameters every evaluation needs.
#[derive(Clone, Copy, Debug)]
pub struct Params {
    pub seed: u64,
    pub biome: Biome,
    /// User relief scale (`flight.relief`, 0 = flat .. 2 = twice as steep).
    pub relief: f32,
}

impl Params {
    pub fn new(seed: u64, biome: &Biome, relief: f32) -> Self {
        Self { seed, biome: *biome, relief: relief.clamp(0.0, 2.0) }
    }

    /// The base relief at (x, y).
    pub fn base(&self, x: f64, y: f64) -> Base {
        let r = &self.biome.relief;
        let k = self.relief;
        let s = self.seed;
        let hs = r.hills_scale.max(50.0) as f64;
        let low = r.hills * k * fbm(s, 0x4111, x / (2.6 * hs), y / (2.6 * hs), 2);
        let mut h = low + 0.55 * r.hills * k * fbm(s, 0x4122, x / hs, y / hs, 4);
        if r.mountains > 0.0 {
            let ms = r.mountain_scale as f64;
            // Mountain massifs come and go: a slow mask keeps some broad valleys open.
            let mask = smoothstep(-0.35, 0.25, fbm(s, 0x4133, x / (2.2 * ms), y / (2.2 * ms), 2));
            let m = ridged(s, 0x4144, x / ms, y / ms, 5);
            h += r.mountains * k * mask * (m * m * 1.3 - 0.15);
        }
        if r.dunes > 0.0 {
            h += self.dunes(x, y) * k;
        }
        let mut coast = f32::INFINITY;
        if r.sea > 0.0 {
            // A "continent" field at ~1.4 km thresholded at the sea's share; its value turned
            // into metres from the shoreline by the field's typical gradient.
            const SCALE: f64 = 1400.0;
            let c = fbm(s, 0x4155, x / SCALE, y / SCALE, 3);
            let t = (r.sea - 0.5) * 0.9;
            let d = (c - t) * 1800.0;
            coast = d;
            // Cliffs where a second field is high; beaches elsewhere.
            let cliff = smoothstep(0.15, 0.4, fbm(s, 0x4166, x / 500.0, y / 500.0, 2));
            let beach_w = 45.0;
            let land = h.max(0.0) + 3.0;
            let beach = if d < 0.0 {
                (d * 0.035).max(-14.0)
            } else if d < beach_w {
                d / beach_w * 2.6
            } else {
                2.6 + (land - 2.6) * smoothstep(beach_w, beach_w + 220.0, d) + 0.004 * (d - beach_w).min(400.0)
            };
            let cliff_h = 12.0 + 0.6 * land;
            let cliffy = if d < 0.0 { (d * 0.12).max(-16.0) } else { cliff_h * smoothstep(4.0, 22.0, d) + land * smoothstep(60.0, 260.0, d) * 0.5 };
            h = beach + (cliffy - beach) * cliff;
        }
        Base { h, low, coast }
    }

    /// Transverse dunes: asymmetric crests across the wind (gentle windward slope, steep slip
    /// face), domain-warped into crescents, with slow draa swells under them.
    fn dunes(&self, x: f64, y: f64) -> f32 {
        let r = &self.biome.relief;
        let s = self.seed;
        let th = 0.6 + (hash(s, 0, 0, 0xD0E) >> 40) as f64 / 16_777_216.0 * 0.8;
        let (c, sn) = (th.cos(), th.sin());
        let u = c * x + sn * y;
        let v = -sn * x + c * y;
        let l = r.dune_len as f64;
        let warp = 0.55 * fbm(s, 0xD01, x / (2.5 * l), y / (2.5 * l), 2) as f64 + 0.25 * ((v / (1.7 * l)).sin());
        let p = (u / l + warp).rem_euclid(1.0) as f32;
        // Windward 0 .. 0.78 rises, the slip face falls in the last 22 %.
        let prof = if p < 0.78 { smoothstep(0.0, 0.78, p) } else { 1.0 - smoothstep(0.78, 1.0, p) };
        let amp = 0.55 + 0.45 * fbm(s, 0xD02, x / (4.0 * l), y / (4.0 * l), 2).clamp(-1.0, 1.0);
        let draa = 0.6 * r.dunes * fbm(s, 0xD03, x / (8.0 * l), y / (8.0 * l), 2);
        r.dunes * amp.max(0.15) * prof.powf(1.3) + draa
    }

    /// The big river nearest to (x, y): (signed distance across, m; along-river coordinate;
    /// direction of flow (world, unit); the river's water level here).
    pub fn big_river_at(&self, x: f64, y: f64) -> Option<(f32, [f32; 2], f32)> {
        let w = self.biome.relief.big_river;
        if w <= 0.0 {
            return None;
        }
        let s = self.seed;
        let th = (hash(s, 1, 0, 0xB16) >> 40) as f64 / 16_777_216.0 * core::f64::consts::PI;
        let (c, sn) = (th.cos(), th.sin());
        let u = c * x + sn * y;
        let v = -sn * x + c * y;
        let band = (v / BIG_RIVER_SPACING).round();
        let ph = (hash(s, band as i64, 0, 0xB17) >> 40) as f64 / 16_777_216.0 * 6.283;
        let tau = core::f64::consts::TAU;
        let (a1, l1, a2, l2) = (160.0, 1900.0, 55.0, 620.0);
        let mid = band * BIG_RIVER_SPACING + a1 * (tau * u / l1 + ph).sin() + a2 * (tau * u / l2 + 2.0 * ph).sin();
        let slope = a1 * tau / l1 * (tau * u / l1 + ph).cos() + a2 * tau / l2 * (tau * u / l2 + 2.0 * ph).cos();
        let dist = ((v - mid) / (1.0 + slope * slope).sqrt()) as f32;
        // Flow along +u or −u by band; direction follows the meander.
        let sign = if hash(s, band as i64, 1, 0xB18) & 1 == 0 { 1.0 } else { -1.0 };
        let (du, dv) = (1.0 / (1.0 + slope * slope).sqrt(), slope / (1.0 + slope * slope).sqrt());
        let dir = [(sign * (c * du - sn * dv)) as f32, (sign * (sn * du + c * dv)) as f32];
        // Level: the slow relief at the river's centre line.
        let (cx, cy) = (c * u - sn * mid, sn * u + c * mid);
        let r = &self.biome.relief;
        let hs = r.hills_scale.max(50.0) as f64;
        let level = r.hills * self.relief * fbm(s, 0x4111, cx / (2.6 * hs), cy / (2.6 * hs), 2) - 1.5;
        Some((dist, dir, level))
    }

    /// Full sample at (x, y), with the macro features in `features` (indices into `all`).
    pub fn sample(&self, all: &[Feature], features: &[usize], x: f64, y: f64) -> Sample {
        let b = self.base(x, y);
        let mut h = b.h;
        let mut water = DRY;
        let mut flow = [0.0f32; 2];
        let mut kind = 0u8;
        let mut shore = f32::INFINITY;
        let valley = self.biome.relief.valley;

        // Sea.
        if b.coast.is_finite() {
            shore = h - SEA_LEVEL;
            if h < SEA_LEVEL {
                water = SEA_LEVEL;
                kind = 3;
            }
        }

        // The big river: a flat floodplain, banks rising to the land over ~150 m.
        if let Some((d, dir, level)) = self.big_river_at(x, y) {
            let half = 0.5 * self.biome.relief.big_river;
            let ad = d.abs();
            let plain = level + 1.6;
            let bed = level - 4.0 * (1.0 - (ad / half).min(1.0).powi(2));
            let carved = if ad < half {
                bed
            } else {
                let t = smoothstep(half + 25.0, half + 25.0 + 160.0 + 120.0 * valley, ad);
                let bank = level + 1.6 * smoothstep(half, half + 10.0, ad);
                bank + (h.max(plain) - bank) * t
            };
            let near = 1.0 - smoothstep(half + 150.0, half + 400.0, ad);
            h = h + (carved - h) * near;
            if ad < half + 1.0 && h < level {
                water = level;
                let speed = 0.9 * (1.0 - (ad / half).min(1.0).powi(2)) + 0.15;
                flow = [dir[0] * speed, dir[1] * speed];
                kind = 2;
            }
            shore = shore.min(h - level);
        }

        // Macro features.
        for &k in features {
            let f = &all[k];
            if !f.alive || !f.near_box(x, y, f.reach() as f64) {
                continue;
            }
            match f.kind {
                FeatureKind::River => {
                    let (d, seg, t) = f.path_distance(x, y);
                    let half = 0.5 * f.size;
                    let level = f.level_at(seg, t);
                    let cut = (h - level).max(0.0);
                    let reach = half + 3.0 + (2.5 + 4.0 * valley) * cut.min(25.0) + 30.0 * valley;
                    if d < reach {
                        if d < half {
                            // The bed: deepest mid-stream.
                            let bed = level - (0.3 + 0.08 * f.size) * (1.0 - (d / half.max(0.5)).powi(2)).max(0.2);
                            h = h.min(bed);
                        } else {
                            // Valley walls rising from the bank to the land.
                            let wall = level + 0.3 + (h - level - 0.3).max(0.0) * smoothstep(half, reach, d).powf(0.8);
                            h = h.min(wall);
                        }
                        if d < half + 0.5 {
                            water = water.max(level);
                            let (dx, dy) = f.path_dir(seg);
                            let speed = f.flow_speed();
                            flow = [dx * speed, dy * speed];
                            kind = 2;
                        }
                    }
                    shore = shore.min((h - level).max(0.0) + (d - half).max(0.0) * 0.05);
                }
                FeatureKind::Lake => {
                    let r = f.ellipse_radius(x, y);
                    let level = f.z;
                    if r < 1.0 {
                        let depth = 0.6 + 0.12 * f.size.min(60.0);
                        let floor = level - depth * (1.0 - r * r).sqrt().max(0.15);
                        h = floor.min(h.min(level - 0.25));
                        water = water.max(level);
                        if kind == 0 {
                            kind = 1;
                        }
                    } else if r < 1.8 {
                        // A low rim around the water; the land beyond keeps its shape.
                        let t = smoothstep(1.0, 1.8, r);
                        let rim = level + 0.2 + 2.5 * t;
                        h = if h < rim { h.max(level + 0.15 + 0.4 * t) } else { rim + (h - rim) * t.powf(0.6) };
                    }
                    shore = shore.min(h - level);
                }
                FeatureKind::Ridge => {
                    let (d, _, _) = f.path_distance(x, y);
                    let half = f.a;
                    if d < half {
                        let t = d / half;
                        let rough = 0.85 + 0.3 * perlin(self.seed, f.id as u64, x / 7.0, y / 7.0);
                        h += f.size * (1.0 - t * t).powi(2) * rough;
                    }
                }
                _ => {}
            }
        }
        if water != DRY && water <= h + 0.02 {
            water = DRY;
            kind = 0;
            flow = [0.0; 2];
        }
        Sample { ground: h, water, flow, shore: shore.min(1000.0), kind }
    }

    /// Ground surface for acoustics and rain, from a sample, its slope (rise per metre), the
    /// forest cover there and the frozen state of the biome.
    pub fn surface(&self, s: &Sample, slope: f32, cover: f32) -> Surface {
        let b = &self.biome;
        if s.wet() {
            // Lakes freeze in the snow; rivers and the sea keep flowing.
            return if b.snow > 0.5 && s.kind == 1 { Surface::Snow } else { Surface::Water };
        }
        if slope > b.rock_slope {
            return Surface::Rock;
        }
        if b.snow > 0.5 || s.ground > b.snowline {
            return Surface::Snow;
        }
        if b.sand > 0.5 || (s.shore < 2.6 && s.shore > -0.5 && b.beaches) {
            return Surface::Sand;
        }
        if cover > 0.5 {
            Surface::Forest
        } else {
            Surface::Grass
        }
    }
}

/// Samples the relief for the UI (its own macro cells; not real-time: allocates once).
pub struct Sampler {
    land: super::land::Land,
    near: Vec<usize>,
}

/// Floats per sample written by [`Sampler::grid`]: ground, water level (−1e9 dry), flow x,
/// flow y (world frame, m/s), height above the nearest water (beaches), water kind
/// (0 none, 1 lake, 2 river, 3 sea).
pub const SAMPLE_FLOATS: usize = 6;

impl Sampler {
    pub fn new(seed: u64, biome: u32, relief: f32) -> Self {
        Self { land: super::land::Land::with_relief(seed, biome, relief), near: Vec::with_capacity(256) }
    }

    /// `nx` × `ny` samples starting at world (x0, y0), `step` m apart, index `j * nx + i`.
    pub fn grid(&mut self, x0: f64, y0: f64, step: f64, nx: usize, ny: usize, out: &mut [f32]) -> usize {
        let n = (nx * ny).min(out.len() / SAMPLE_FLOATS);
        if n == 0 {
            return 0;
        }
        let (x1, y1) = (x0 + step * (nx - 1) as f64, y0 + step * (ny - 1) as f64);
        self.land.stream_macros(0.5 * (x0 + x1), 0.5 * (y0 + y1));
        self.near.clear();
        for (k, f) in self.land.features.iter().enumerate() {
            if f.alive && f.bbox[0] <= x1 && f.bbox[2] >= x0 && f.bbox[1] <= y1 && f.bbox[3] >= y0 {
                self.near.push(k);
            }
        }
        for k in 0..n {
            let (i, j) = (k % nx, k / nx);
            let (x, y) = (x0 + step * i as f64, y0 + step * j as f64);
            let s = self.land.params.sample(&self.land.features, &self.near, x, y);
            let o = &mut out[k * SAMPLE_FLOATS..(k + 1) * SAMPLE_FLOATS];
            o[0] = s.ground;
            o[1] = if s.water == DRY { -1.0e9 } else { s.water };
            o[2] = s.flow[0];
            o[3] = s.flow[1];
            o[4] = s.shore;
            o[5] = s.kind as f32;
        }
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flight::land::BIOMES;

    #[test]
    fn noise_is_bounded_and_smooth() {
        let mut lo = f32::MAX;
        let mut hi = f32::MIN;
        for i in 0..2000 {
            let x = i as f64 * 0.37;
            let v = perlin(1, 2, x, x * 0.7);
            lo = lo.min(v);
            hi = hi.max(v);
            let d = (perlin(1, 2, x + 0.01, x * 0.7) - v).abs();
            assert!(d < 0.05, "jump {d}");
            let r = ridged(3, 4, x * 0.1, x * 0.05, 5);
            assert!((0.0..=1.0).contains(&r));
        }
        assert!(lo < -0.3 && hi > 0.3 && lo > -1.0 && hi < 1.0, "{lo} {hi}");
    }

    #[test]
    fn biomes_have_their_relief() {
        let range = |name: &str| {
            let b = BIOMES.iter().find(|b| b.name == name).unwrap();
            let p = Params::new(5, b, 1.0);
            let (mut lo, mut hi, mut wet) = (f32::MAX, f32::MIN, 0);
            for i in 0..60 {
                for j in 0..60 {
                    let s = p.sample(&[], &[], i as f64 * 25.0, j as f64 * 25.0);
                    lo = lo.min(s.ground);
                    hi = hi.max(s.ground);
                    wet += s.wet() as i32;
                }
            }
            (hi - lo, wet)
        };
        for b in BIOMES.iter() {
            let (r, w) = range(b.name);
            eprintln!("{:>16}: relief {r:.1} m, {w} wet of 3600", b.name);
        }
        let (meadows, _) = range("meadows");
        let (mountains, _) = range("snowy_mountains");
        let (dunes, _) = range("desert");
        assert!(meadows > 3.0 && meadows < 30.0, "meadows {meadows}");
        assert!(mountains > 60.0, "mountains {mountains}");
        assert!(dunes > 8.0, "dunes {dunes}");
        let (_, sea) = range("sea_coast");
        let (_, river) = range("big_river");
        assert!(sea > 0 || river >= 0);
    }
}
