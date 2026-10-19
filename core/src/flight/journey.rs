//! The `journey` preset: the biome is a function of place, so a long flight crosses meadows,
//! forests, taiga, highlands and the occasional desert (docs/world.md, "Journey").
//!
//! * **Regions**: a jittered Voronoi diagram on an 800 m hashed grid (domain-warped so the
//!   borders curve). Each region takes one biome from a smooth large-scale **climate**:
//!   temperature and moisture fields at ~3 km and a ruggedness field at ~2 km, plus a little
//!   per-region chance, so neighbours are plausible (taiga next to snowy mountains, meadows
//!   next to the desert, never snow next to sand).
//! * **Blending**: within [`BLEND`] (difference of distances to the region sites, ≈180 m of
//!   ground across a border) the neighbouring biome fades in. Heights blend by evaluating each
//!   biome's own base relief and lerping the heights; numeric parameters lerp; discrete traits
//!   (air, beaches, snow and sand cover, bird tables) come from the dominant biome, or are
//!   dithered by a smooth noise per tile / tree / terrain sample, so ecotones look natural.
//!
//! Everything is a pure function of (seed, place). The sea coast and the big river are not
//! part of a journey: their geometry (a continent field, a band of rivers) is global.

use super::land::{biome, fbm, hash, noise2, smoothstep, unit, Biome, Stream};

/// Name of the preset in `land::BIOMES`.
pub const NAME: &str = "journey";
/// Its index in `land::BIOMES` (after the eleven fixed biomes).
pub const JOURNEY: u32 = 11;
/// Grid of the region sites, m.
pub const REGION: f64 = 800.0;
/// Difference of distances to two region sites over which the farther one fades in, m.
/// Near the line between the sites this is twice the distance from the border: the blend
/// spans ≈ 180 m of ground.
pub const BLEND: f32 = 180.0;
/// How far beyond `BLEND` a tile's centre must be from any other biome for the whole tile
/// to be uniform (the region field is at most ~1.7-Lipschitz with the warp; a tile's half
/// diagonal is 28 m: 2 · 1.7 · 28 ≈ 96 m).
pub const TILE_MARGIN: f32 = 120.0;
/// Domain warp of the region lookup: amplitude and scale, m.
const WARP: f64 = 100.0;
const WARP_SCALE: f64 = 700.0;
/// Scale of the temperature and moisture fields, m.
const CLIMATE: f64 = 3000.0;
const MAX: usize = 4;

const MIXED: u32 = 0;
const BROADLEAF: u32 = 1;
const TAIGA: u32 = 2;
const RIVER_VALLEY: u32 = 3;
const MEADOWS: u32 = 4;
const HIGHLANDS: u32 = 5;
const DESERT: u32 = 6;
const SNOWY: u32 = 7;
const LAKE_SHORE: u32 = 8;

/// The biomes at a place and their weights (sorted, heaviest first; summing to 1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Blend {
    pub n: u8,
    pub idx: [u8; MAX],
    pub w: [f32; MAX],
    /// Difference of distances to the nearest site of another biome (when asked for, else
    /// at least `BLEND`): `BLEND` or more means no other biome here.
    pub margin: f32,
}

impl Blend {
    pub fn single(index: u32) -> Self {
        Self { n: 1, idx: [index as u8, 0, 0, 0], w: [1.0, 0.0, 0.0, 0.0], margin: f32::INFINITY }
    }

    /// The biome with the largest weight.
    pub fn dominant(&self) -> u32 {
        self.idx[0] as u32
    }

    pub fn uniform(&self) -> bool {
        self.n <= 1
    }

    /// A biome drawn by weight with `u` in [0, 1) (a dither: ecotones).
    pub fn pick(&self, u: f32) -> u32 {
        let mut x = u;
        for k in 0..self.n as usize {
            if x < self.w[k] {
                return self.idx[k] as u32;
            }
            x -= self.w[k];
        }
        self.idx[self.n.max(1) as usize - 1] as u32
    }

    /// (biome, weight) pairs.
    pub fn parts(&self) -> impl Iterator<Item = (&'static Biome, f32)> + '_ {
        (0..self.n as usize).map(move |k| (biome(self.idx[k] as u32), self.w[k]))
    }

    /// A numeric trait, weighted.
    pub fn lerp(&self, f: impl Fn(&Biome) -> f32) -> f32 {
        if self.n <= 1 {
            return f(biome(self.idx[0] as u32));
        }
        self.parts().map(|(b, w)| w * f(b)).sum()
    }
}

/// Site of region cell (i, j), m (jittered within the middle 40 % of the cell, so the 3 × 3
/// cells around a point always hold every site that can matter).
fn site(seed: u64, i: i64, j: i64) -> (f64, f64) {
    let h = hash(seed, i, j, 0x5E17);
    let jx = 0.3 + 0.4 * ((h >> 40) as f64 / 16_777_216.0);
    let jy = 0.3 + 0.4 * ((h & 0xFF_FFFF) as f64 / 16_777_216.0);
    ((i as f64 + jx) * REGION, (j as f64 + jy) * REGION)
}

/// The climate at (x, y): (temperature, moisture, ruggedness), each roughly a standard score.
pub fn climate(seed: u64, x: f64, y: f64) -> (f32, f32, f32) {
    let z = |salt: u64, s: f64| (fbm(seed, salt, x / s, y / s) - 0.5) / 0.158;
    (z(0xC11, CLIMATE), z(0xC12, CLIMATE), z(0xC13, 0.7 * CLIMATE))
}

/// The climate of region cell (i, j): at its site, with a little chance per region.
fn region_climate(seed: u64, i: i64, j: i64) -> (f32, f32, f32, f32) {
    let (sx, sy) = site(seed, i, j);
    let (t, m, r) = climate(seed, sx, sy);
    let mut s = Stream::new(hash(seed, i, j, 0x5E18));
    let t = t + 0.3 * (s.uniform() - 0.5);
    let m = m + 0.5 * (s.uniform() - 0.5);
    let r = r + 0.4 * (s.uniform() - 0.5);
    (t, m, r, s.uniform())
}

/// The biome of region cell (i, j): from its climate. A journey starts somewhere gentle: the
/// regions around the origin are never desert or snowy peaks.
pub fn region_biome(seed: u64, i: i64, j: i64) -> u32 {
    let b = climate_biome(seed, i, j);
    if i.abs() <= 1 && j.abs() <= 1 {
        return match b {
            DESERT => MEADOWS,
            SNOWY => TAIGA,
            b => b,
        };
    }
    b
}

fn climate_biome(seed: u64, i: i64, j: i64) -> u32 {
    let (t, m, r, u) = region_climate(seed, i, j);
    if t < -1.25 {
        // Cold: snowy peaks where it is rugged, taiga elsewhere.
        if r > 0.3 { SNOWY } else { TAIGA }
    } else if r > 1.25 {
        HIGHLANDS
    } else if t < -0.7 {
        if r > 0.5 { HIGHLANDS } else { TAIGA }
    } else if t > 1.0 && m < -0.7 {
        // Hot and dry: the rare desert, never next to a cold region.
        // (The temperature field ~1 km around the site, all round.)
        let (sx, sy) = site(seed, i, j);
        let cold = (0..6).any(|k| {
            let a = k as f64 * core::f64::consts::FRAC_PI_3;
            let (x, y) = (sx + 1000.0 * a.cos(), sy + 1000.0 * a.sin());
            (fbm(seed, 0xC11, x / CLIMATE, y / CLIMATE) - 0.5) / 0.158 < -0.5
        });
        if cold { MEADOWS } else { DESERT }
    } else if m < -0.7 {
        MEADOWS
    } else if m > 0.9 {
        // Wet lowlands: lakes and many brooks.
        if u < 0.5 { LAKE_SHORE } else { RIVER_VALLEY }
    } else if t > 0.3 {
        // Warm and humid: broadleaf woods, a lake or a valley now and then.
        match u {
            u if u < 0.55 => BROADLEAF,
            u if u < 0.75 => MIXED,
            u if u < 0.85 => MEADOWS,
            u if u < 0.93 => RIVER_VALLEY,
            _ => LAKE_SHORE,
        }
    } else {
        match u {
            u if u < 0.5 => MIXED,
            u if u < 0.65 => BROADLEAF,
            u if u < 0.75 => MEADOWS,
            u if u < 0.88 => LAKE_SHORE,
            _ => RIVER_VALLEY,
        }
    }
}

/// The biomes at (x, y). `look` > 0 also measures `margin` out to `BLEND + look` (for the
/// uniform-tile test); 0 for a plain lookup.
pub fn blend(seed: u64, x: f64, y: f64, look: f32) -> Blend {
    let n = |salt: u64| WARP * 2.0 * (noise2(seed, salt, x / WARP_SCALE, y / WARP_SCALE) as f64 - 0.5);
    let (wx, wy) = (x + n(0x3A1), y + n(0x3A2));
    let (ci, cj) = ((wx / REGION).floor() as i64, (wy / REGION).floor() as i64);
    let mut d = [(0.0f64, 0i64, 0i64); 9];
    let mut near = 0;
    for a in 0..3 {
        for b in 0..3 {
            let (i, j) = (ci + a as i64 - 1, cj + b as i64 - 1);
            let (sx, sy) = site(seed, i, j);
            let k = a * 3 + b;
            d[k] = ((sx - wx).hypot(sy - wy), i, j);
            if d[k].0 < d[near].0 {
                near = k;
            }
        }
    }
    let b0 = region_biome(seed, d[near].1, d[near].2);
    let mut out = Blend { n: 1, idx: [b0 as u8, 0, 0, 0], w: [1.0, 0.0, 0.0, 0.0], margin: f32::INFINITY };
    for (k, &(dk, i, j)) in d.iter().enumerate() {
        let f = (dk - d[near].0) as f32;
        if k == near || f >= BLEND + look {
            continue;
        }
        let bk = region_biome(seed, i, j) as u8;
        if bk as u32 != b0 {
            out.margin = out.margin.min(f);
        }
        if f >= BLEND {
            continue;
        }
        let w = smoothstep(BLEND, 0.0, f);
        if let Some(e) = (0..out.n as usize).find(|&e| out.idx[e] == bk) {
            out.w[e] += w;
        } else if (out.n as usize) < MAX {
            out.idx[out.n as usize] = bk;
            out.w[out.n as usize] = w;
            out.n += 1;
        }
    }
    if out.n == 1 {
        out.w[0] = 1.0;
        return out;
    }
    let total: f32 = out.w[..out.n as usize].iter().sum();
    for w in out.w[..out.n as usize].iter_mut() {
        *w /= total;
    }
    // Heaviest first (stable: the nearest region wins a tie).
    for a in 1..out.n as usize {
        let mut k = a;
        while k > 0 && out.w[k] > out.w[k - 1] {
            out.w.swap(k, k - 1);
            out.idx.swap(k, k - 1);
            k -= 1;
        }
    }
    out
}

/// A smooth dither in [0, 1) for picking a discrete trait in an ecotone (patches of ~30 m).
pub fn dither(seed: u64, x: f64, y: f64) -> f32 {
    let v = noise2(seed, 0xD17E, x / 30.0, y / 30.0);
    // Value noise bunches around 0.5: spread it towards uniform.
    smoothstep(0.15, 0.85, v).min(0.9999)
}

/// Uniform [0, 1) from a hash (a per-tile dither).
pub fn tile_dither(seed: u64, i: i32, j: i32) -> f32 {
    unit(hash(seed, i as i64, j as i64, 0xD17F))
}

/// The biome of a blend: numeric traits weighted, discrete ones (name, air, beaches, snow and
/// sand cover, relief kind) of `discrete`. A single biome is returned as it is.
pub fn mix(bl: &Blend, discrete: u32) -> Biome {
    if bl.n <= 1 {
        return *biome(bl.idx[0] as u32);
    }
    let mut b = *biome(discrete);
    let l = |f: &dyn Fn(&Biome) -> f32| bl.lerp(f);
    b.forest = l(&|b| b.forest);
    b.density_ha = l(&|b| b.density_ha);
    for k in 0..b.species.len() {
        b.species[k] = l(&|b| b.species[k]);
    }
    // Rivers: the crossing rate lerps, not the distance between them.
    b.river_every = 1.0 / l(&|b| 1.0 / b.river_every);
    b.lake = l(&|b| b.lake);
    b.lake_size = [l(&|b| b.lake_size[0]), l(&|b| b.lake_size[1])];
    b.ridge = l(&|b| b.ridge);
    b.rocks = l(&|b| b.rocks);
    b.waterfall = l(&|b| b.waterfall);
    b.campfire = l(&|b| b.campfire);
    b.birds = l(&|b| b.birds);
    b.patch = l(&|b| b.patch);
    b.relief.valley = l(&|b| b.relief.valley);
    b.relief.mountains = l(&|b| b.relief.mountains);
    b.relief.hills = l(&|b| b.relief.hills);
    // No snowline / treeline is "very high" (keeps the lerp finite).
    b.snowline = l(&|b| b.snowline.min(3000.0));
    b.treeline = l(&|b| b.treeline.min(3000.0));
    b.rock_slope = l(&|b| b.rock_slope);
    b.temperature = l(&|b| b.temperature);
    b.wind = l(&|b| b.wind);
    let h = &mut b.habitat;
    h.forest = l(&|b| b.habitat.forest);
    h.open = l(&|b| b.habitat.open);
    h.water = l(&|b| b.habitat.water);
    h.coast = l(&|b| b.habitat.coast);
    h.alpine = l(&|b| b.habitat.alpine);
    h.desert = l(&|b| b.habitat.desert);
    b
}

/// The traits [`super::terrain::Params::surface_of`] reads (snow, sand and beaches of
/// `discrete`, the rock slope and the snowline weighted): cheaper than [`mix`].
pub fn ground(bl: &Blend, discrete: u32) -> Biome {
    let mut b = *biome(discrete);
    if bl.n > 1 {
        b.rock_slope = bl.lerp(|b| b.rock_slope);
        b.snowline = bl.lerp(|b| b.snowline.min(3000.0));
    }
    b
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flight::land::BIOMES;

    #[test]
    fn regions_are_varied_and_plausible() {
        let mut count = [0usize; 12];
        let mut odd = 0;
        for seed in 1..6u64 {
            for i in -20..20 {
                for j in -20..20 {
                    let b = region_biome(seed, i, j);
                    count[b as usize] += 1;
                    // Snow never borders sand.
                    if b == DESERT {
                        for (a, c) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                            if region_biome(seed, i + a, j + c) == SNOWY {
                                odd += 1;
                            }
                        }
                    }
                }
            }
        }
        let total: usize = count.iter().sum();
        for (k, c) in count.iter().enumerate().take(9) {
            eprintln!("{:>16}: {:.1} %", BIOMES[k].name, 100.0 * *c as f32 / total as f32);
        }
        assert_eq!(odd, 0);
        assert_eq!(count[9] + count[10] + count[11], 0, "no sea coast, big river or journey");
        for k in 0..9 {
            assert!(count[k] > 0, "{} never appears", BIOMES[k].name);
        }
        assert!(count[DESERT as usize] * 12 < total, "the desert is rare");
    }

    #[test]
    fn blends_are_continuous_and_normalised() {
        let mut prev = blend(3, 0.0, 0.0, 0.0);
        let mut borders = 0;
        for k in 1..40_000 {
            let x = k as f64 * 0.5;
            let b = blend(3, x, 0.4 * x, 0.0);
            let s: f32 = b.w[..b.n as usize].iter().sum();
            assert!((s - 1.0).abs() < 1e-5);
            // Weights per biome change little over 0.5 m.
            for id in 0..12u8 {
                let w = |b: &Blend| (0..b.n as usize).filter(|&e| b.idx[e] == id).map(|e| b.w[e]).sum::<f32>();
                assert!((w(&b) - w(&prev)).abs() < 0.02, "jump at {x}: {prev:?} -> {b:?}");
            }
            borders += (b.n > 1) as usize;
            prev = b;
        }
        assert!(borders > 100, "{borders}");
    }
}
