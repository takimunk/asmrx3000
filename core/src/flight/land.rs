//! The landscape of flight mode: a deterministic function of (seed, biome, relief) over the
//! plane, streamed in square tiles around the listener, whichever way it travels, and
//! retired when it is left behind.
//!
//! * **Macro cells** (360 m) hold the big features: brooks and rivers, lakes, rocky ridges
//!   with waterfalls, campfires, the big river's banks, and their sound sites. They exist
//!   within 800 m, before any tile they can touch.
//! * **Tiles** (40 m) exist within `KEEP` (240 m, the reach of the propagators' delay lines):
//!   each carries the **terrain** on a 5 m grid (ground height, water level, surface: see
//!   terrain.rs), an aggregate cell (crown area of its forest), boulders, a cricket meadow, a
//!   stretch of surf where the sea meets the land, and wind over an exposed crest. Within
//!   `TREE_R` (90 m) a tile also holds its individual trees and the birds living in them.
//!
//! Content is a pure function of `(seed, biome, relief, tile / macro index)`: the trajectory,
//! the order and the timing of generation never change what exists where. Storage is
//! toroidal (indexed by tile / macro coordinates modulo the ring size) and sized at build
//! time, so streaming never allocates and the amount of content stays bounded however the
//! listener turns (see [`Land::stream`]).

use crate::director::Species;
use crate::scene::TreeSpecies;

use super::journey::{self, Blend};
use super::terrain::{Params, Relief, Sample, Surface, DRY};

/// Side of a streamed tile, m.
pub const TILE: f64 = 40.0;
/// Tiles exist within this distance (centre) of the listener; retired beyond `KEEP + 40`.
pub const KEEP: f64 = 240.0;
/// Individual trees and birds exist in tiles within this distance; dropped beyond `TREE_R + 30`.
pub const TREE_R: f64 = 90.0;
/// Side of a macro cell (big features), m.
pub const MACRO: f64 = 360.0;
/// Macro cells exist within this distance (centre): a feature reaches ~650 m from its cell's
/// centre (a river and its valley), so every feature that can touch a live tile exists
/// before the tile is made.
const MACRO_R: f64 = 900.0;
/// Tile ring side (tiles): covers 2 · (KEEP + 40) with room to spare.
pub const W: usize = 16;
/// Macro ring side.
const MW: usize = 8;
/// Density levels at which the area of a cell is tabulated (user density 0.25..2).
pub const LEVELS: usize = 8;
/// Terrain samples per tile side (5 m apart, both edges included).
pub const GRID_N: usize = 9;
pub const GRID_STEP: f64 = TILE / (GRID_N - 1) as f64;
const GRID_LEN: usize = GRID_N * GRID_N;

pub const TREES_PER_TILE: usize = 320;
pub const BIRDS_PER_TILE: usize = 16;
const FEATURES_PER_MACRO: usize = 8;
const SITES_PER_MACRO: usize = 128;
const FEATURES_PER_TILE: usize = 4;
const SITES_PER_TILE: usize = 3;
pub const TILES: usize = W * W;
const MACROS: usize = MW * MW;
pub const MAX_FEATURES: usize = MACROS * FEATURES_PER_MACRO + TILES * FEATURES_PER_TILE;
pub const MAX_SITES: usize = MACROS * SITES_PER_MACRO + TILES * SITES_PER_TILE;
pub const PATH_POINTS: usize = 24;
/// Features a tile or a point looks at (those whose box touches it).
pub const LOCAL: usize = 48;

/// Tree grid spacing, m (jittered grid: at most one tree per cell).
const GRID: f64 = 2.5;

pub const SPECIES: [TreeSpecies; 7] =
    [TreeSpecies::Birch, TreeSpecies::Oak, TreeSpecies::Spruce, TreeSpecies::Willow, TreeSpecies::Alder, TreeSpecies::Aspen, TreeSpecies::Pine];

// ---------------------------------------------------------------------------------------
// Hashing and noise
// ---------------------------------------------------------------------------------------

#[inline]
fn mix64(mut x: u64) -> u64 {
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

/// Hash of a lattice point / index pair.
#[inline]
pub fn hash(seed: u64, a: i64, b: i64, salt: u64) -> u64 {
    mix64(seed ^ mix64((a as u64) ^ mix64((b as u64) ^ salt.wrapping_mul(0x9E37_79B9_7F4A_7C15))))
}

/// Uniform [0, 1) from a hash.
#[inline]
pub fn unit(h: u64) -> f32 {
    (h >> 40) as f32 * (1.0 / 16_777_216.0)
}

/// A tiny deterministic stream (splitmix) for one generation unit.
pub struct Stream(u64);

impl Stream {
    pub fn new(h: u64) -> Self {
        Self(h)
    }
    #[inline]
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        mix64(self.0)
    }
    #[inline]
    pub fn uniform(&mut self) -> f32 {
        unit(self.next())
    }
    #[inline]
    pub fn range(&mut self, a: f32, b: f32) -> f32 {
        a + (b - a) * self.uniform()
    }
    pub fn sign(&mut self) -> f32 {
        if self.uniform() < 0.5 {
            -1.0
        } else {
            1.0
        }
    }
}

fn smooth(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

/// Smooth value noise in [0, 1] on the unit lattice.
pub fn noise2(seed: u64, salt: u64, x: f64, y: f64) -> f32 {
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (smooth((x - x0) as f32), smooth((y - y0) as f32));
    let (i, j) = (x0 as i64, y0 as i64);
    let v = |a: i64, b: i64| unit(hash(seed, a, b, salt));
    let a = v(i, j) + (v(i + 1, j) - v(i, j)) * fx;
    let b = v(i, j + 1) + (v(i + 1, j + 1) - v(i, j + 1)) * fx;
    a + (b - a) * fy
}

/// Two octaves of value noise, roughly centred on 0.5.
pub fn fbm(seed: u64, salt: u64, x: f64, y: f64) -> f32 {
    0.65 * noise2(seed, salt, x, y) + 0.35 * noise2(seed, salt ^ 0x55, x * 2.03 + 17.1, y * 2.03 - 5.3)
}

pub fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    smooth(((x - e0) / (e1 - e0)).clamp(0.0, 1.0))
}

// ---------------------------------------------------------------------------------------
// Biomes
// ---------------------------------------------------------------------------------------

/// Where birds of a biome live (shares, roughly 0..1). A hook for the bird director: see
/// [`species_weight`] and [`Land::habitat_at`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Habitat {
    pub forest: f32,
    pub open: f32,
    pub water: f32,
    /// Sea coast (gulls, waders).
    pub coast: f32,
    /// Above the treeline, snow and rock.
    pub alpine: f32,
    pub desert: f32,
}

/// Relative weight of a bird species in a habitat, multiplied into the per-tree weights when
/// the birds of a tile are drawn. The bird director can extend this (e.g. a gull with
/// `0.1 + 3.0 * h.coast`, a dipper with `h.water`); species not listed weigh 1.
pub fn species_weight(sp: Species, h: &Habitat) -> f32 {
    #[allow(unreachable_patterns)]
    let w = match sp {
        Species::Swallow => 0.3 + h.open + h.water,
        Species::Nightingale => 0.3 + 0.6 * h.forest + h.water,
        Species::Cuckoo => 0.5 + h.open,
        Species::TawnyOwl => 0.3 + h.forest,
        Species::WoodPigeon => 0.4 + h.forest,
        Species::Chaffinch | Species::Robin | Species::SongThrush => 0.6 + 0.6 * h.forest,
        _ => 1.0,
    };
    w * (1.0 - 0.7 * h.desert) * (1.0 - 0.5 * h.alpine)
}

/// The sound of the air over exposed ground in a biome.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Air {
    None,
    /// A breeze in the grass on open crests.
    Breeze,
    /// Wind-blown sand hissing over dune crests (saltation).
    Sand,
    /// Wind howling over rocky ridges, snow drifting off them.
    Ridge,
}

/// A landscape preset: relief, ground, how much forest, which trees, how often water and rock
/// appear.
#[derive(Clone, Copy, Debug)]
pub struct Biome {
    pub name: &'static str,
    /// Share of the ground under forest.
    pub forest: f32,
    /// Trunks per hectare inside a stand (at density 1).
    pub density_ha: f32,
    /// Relative weights in `SPECIES` order: birch, oak, spruce, willow, alder, aspen, pine.
    pub species: [f32; 7],
    /// Mean distance between brook/river crossings, m.
    pub river_every: f32,
    /// Chance of a lake per macro cell.
    pub lake: f32,
    /// Semi-axes of lakes, m (min, max).
    pub lake_size: [f32; 2],
    /// Chance of a rocky ridge per macro cell.
    pub ridge: f32,
    /// Chance of boulders per tile.
    pub rocks: f32,
    /// Chance that a ridge carries a waterfall.
    pub waterfall: f32,
    /// Chance of a campfire per macro cell.
    pub campfire: f32,
    /// Birds per tree.
    pub birds: f32,
    /// Size of stands and clearings, m.
    pub patch: f32,
    /// The relief (terrain.rs).
    pub relief: Relief,
    /// Snow cover: 1 = everything not steep is snow, lakes freeze, precipitation is snow.
    pub snow: f32,
    /// Sand cover (desert).
    pub sand: f32,
    /// Sandy beaches along the sea, lakes and the big river.
    pub beaches: bool,
    /// Snow above this height, m.
    pub snowline: f32,
    /// No trees above this height, m.
    pub treeline: f32,
    /// Bare rock on slopes steeper than this (rise per metre).
    pub rock_slope: f32,
    /// Air temperature offset, °C.
    pub temperature: f32,
    /// Wind over the open land relative to the scene's.
    pub wind: f32,
    pub air: Air,
    pub habitat: Habitat,
}

const R0: Relief = Relief::flat();
const H_FOREST: Habitat = Habitat { forest: 1.0, open: 0.3, water: 0.2, coast: 0.0, alpine: 0.0, desert: 0.0 };
const NO_SNOW: f32 = f32::INFINITY;

#[rustfmt::skip]
pub const BIOMES: [Biome; 12] = [
    Biome { name: "mixed",        forest: 0.62, density_ha: 480.0, species: [0.20, 0.15, 0.20, 0.08, 0.10, 0.12, 0.15], river_every: 340.0, lake: 0.28, lake_size: [20.0, 45.0], ridge: 0.20, rocks: 0.30, waterfall: 0.25, campfire: 0.12, birds: 0.030, patch: 170.0,
            relief: Relief { hills: 12.0, hills_scale: 380.0, valley: 0.4, ..R0 }, snow: 0.0, sand: 0.0, beaches: false, snowline: NO_SNOW, treeline: NO_SNOW, rock_slope: 0.9, temperature: 0.0, wind: 1.0, air: Air::Breeze, habitat: H_FOREST },
    Biome { name: "broadleaf",    forest: 0.68, density_ha: 420.0, species: [0.25, 0.35, 0.00, 0.10, 0.10, 0.20, 0.00], river_every: 400.0, lake: 0.20, lake_size: [20.0, 45.0], ridge: 0.12, rocks: 0.20, waterfall: 0.15, campfire: 0.10, birds: 0.036, patch: 190.0,
            relief: Relief { hills: 9.0, hills_scale: 340.0, valley: 0.4, ..R0 }, snow: 0.0, sand: 0.0, beaches: false, snowline: NO_SNOW, treeline: NO_SNOW, rock_slope: 0.9, temperature: 1.0, wind: 0.9, air: Air::None, habitat: H_FOREST },
    Biome { name: "taiga",        forest: 0.80, density_ha: 640.0, species: [0.15, 0.00, 0.50, 0.00, 0.00, 0.05, 0.35], river_every: 480.0, lake: 0.36, lake_size: [25.0, 60.0], ridge: 0.24, rocks: 0.50, waterfall: 0.30, campfire: 0.08, birds: 0.022, patch: 220.0,
            relief: Relief { hills: 16.0, hills_scale: 450.0, valley: 0.5, ..R0 }, snow: 0.0, sand: 0.0, beaches: false, snowline: NO_SNOW, treeline: NO_SNOW, rock_slope: 0.8, temperature: -4.0, wind: 1.0, air: Air::None, habitat: H_FOREST },
    Biome { name: "river_valley", forest: 0.50, density_ha: 380.0, species: [0.20, 0.15, 0.00, 0.30, 0.30, 0.05, 0.00], river_every: 170.0, lake: 0.40, lake_size: [20.0, 50.0], ridge: 0.08, rocks: 0.15, waterfall: 0.10, campfire: 0.12, birds: 0.040, patch: 150.0,
            relief: Relief { hills: 22.0, hills_scale: 420.0, valley: 1.0, ..R0 }, snow: 0.0, sand: 0.0, beaches: false, snowline: NO_SNOW, treeline: NO_SNOW, rock_slope: 0.9, temperature: 1.0, wind: 0.9, air: Air::Breeze, habitat: Habitat { forest: 0.7, open: 0.5, water: 1.0, ..H_FOREST } },
    Biome { name: "meadows",      forest: 0.28, density_ha: 300.0, species: [0.30, 0.40, 0.00, 0.10, 0.00, 0.20, 0.00], river_every: 450.0, lake: 0.24, lake_size: [20.0, 45.0], ridge: 0.08, rocks: 0.20, waterfall: 0.10, campfire: 0.20, birds: 0.045, patch: 140.0,
            relief: Relief { hills: 7.0, hills_scale: 300.0, valley: 0.3, ..R0 }, snow: 0.0, sand: 0.0, beaches: false, snowline: NO_SNOW, treeline: NO_SNOW, rock_slope: 0.9, temperature: 2.0, wind: 1.1, air: Air::Breeze, habitat: Habitat { forest: 0.4, open: 1.0, ..H_FOREST } },
    Biome { name: "highlands",    forest: 0.55, density_ha: 450.0, species: [0.30, 0.00, 0.30, 0.00, 0.00, 0.00, 0.40], river_every: 300.0, lake: 0.24, lake_size: [20.0, 55.0], ridge: 0.56, rocks: 0.80, waterfall: 0.60, campfire: 0.08, birds: 0.025, patch: 160.0,
            relief: Relief { hills: 16.0, hills_scale: 420.0, mountains: 40.0, mountain_scale: 520.0, valley: 0.6, ..R0 }, snow: 0.0, sand: 0.0, beaches: false, snowline: 75.0, treeline: 55.0, rock_slope: 0.75, temperature: -2.0, wind: 1.2, air: Air::Ridge, habitat: Habitat { forest: 0.6, open: 0.6, alpine: 0.4, ..H_FOREST } },
    Biome { name: "desert",       forest: 0.0, density_ha: 120.0, species: [0.00, 0.00, 0.00, 0.50, 0.00, 0.00, 0.50], river_every: 6000.0, lake: 0.10, lake_size: [18.0, 35.0], ridge: 0.15, rocks: 0.25, waterfall: 0.0, campfire: 0.05, birds: 0.010, patch: 200.0,
            relief: Relief { hills: 6.0, hills_scale: 600.0, dunes: 11.0, dune_len: 80.0, valley: 0.2, ..R0 }, snow: 0.0, sand: 1.0, beaches: true, snowline: NO_SNOW, treeline: NO_SNOW, rock_slope: 0.85, temperature: 9.0, wind: 1.35, air: Air::Sand, habitat: Habitat { forest: 0.0, open: 0.4, water: 0.1, desert: 1.0, ..H_FOREST } },
    Biome { name: "snowy_mountains", forest: 0.35, density_ha: 420.0, species: [0.10, 0.00, 0.55, 0.00, 0.00, 0.00, 0.35], river_every: 520.0, lake: 0.28, lake_size: [25.0, 70.0], ridge: 0.45, rocks: 0.60, waterfall: 0.55, campfire: 0.04, birds: 0.010, patch: 180.0,
            relief: Relief { hills: 14.0, hills_scale: 500.0, mountains: 170.0, mountain_scale: 760.0, valley: 0.6, ..R0 }, snow: 1.0, sand: 0.0, beaches: false, snowline: -1000.0, treeline: 45.0, rock_slope: 0.85, temperature: -14.0, wind: 1.4, air: Air::Ridge, habitat: Habitat { forest: 0.4, open: 0.3, alpine: 1.0, ..H_FOREST } },
    Biome { name: "lake_shore",   forest: 0.52, density_ha: 420.0, species: [0.35, 0.05, 0.15, 0.10, 0.10, 0.05, 0.20], river_every: 520.0, lake: 1.0, lake_size: [70.0, 170.0], ridge: 0.10, rocks: 0.35, waterfall: 0.15, campfire: 0.18, birds: 0.032, patch: 170.0,
            relief: Relief { hills: 10.0, hills_scale: 360.0, valley: 0.4, ..R0 }, snow: 0.0, sand: 0.0, beaches: true, snowline: NO_SNOW, treeline: NO_SNOW, rock_slope: 0.9, temperature: 0.0, wind: 1.1, air: Air::Breeze, habitat: Habitat { forest: 0.7, open: 0.5, water: 1.0, ..H_FOREST } },
    Biome { name: "sea_coast",    forest: 0.30, density_ha: 320.0, species: [0.20, 0.10, 0.00, 0.00, 0.10, 0.10, 0.50], river_every: 700.0, lake: 0.10, lake_size: [20.0, 40.0], ridge: 0.12, rocks: 0.40, waterfall: 0.10, campfire: 0.15, birds: 0.030, patch: 160.0,
            relief: Relief { hills: 12.0, hills_scale: 420.0, sea: 0.45, valley: 0.4, ..R0 }, snow: 0.0, sand: 0.0, beaches: true, snowline: NO_SNOW, treeline: NO_SNOW, rock_slope: 0.8, temperature: 1.0, wind: 1.3, air: Air::Breeze, habitat: Habitat { forest: 0.4, open: 0.7, water: 0.3, coast: 1.0, ..H_FOREST } },
    Biome { name: "big_river",    forest: 0.48, density_ha: 400.0, species: [0.20, 0.15, 0.05, 0.25, 0.25, 0.10, 0.00], river_every: 600.0, lake: 0.20, lake_size: [20.0, 50.0], ridge: 0.08, rocks: 0.15, waterfall: 0.05, campfire: 0.15, birds: 0.038, patch: 160.0,
            relief: Relief { hills: 14.0, hills_scale: 420.0, valley: 0.7, big_river: 90.0, ..R0 }, snow: 0.0, sand: 0.0, beaches: true, snowline: NO_SNOW, treeline: NO_SNOW, rock_slope: 0.9, temperature: 1.0, wind: 1.0, air: Air::Breeze, habitat: Habitat { forest: 0.6, open: 0.5, water: 1.0, ..H_FOREST } },
    // The biome is a function of place (journey.rs): these values only stand in where a
    // single preset is asked for (descriptions, tests); the land itself uses the regions.
    Biome { name: "journey",      forest: 0.55, density_ha: 440.0, species: [0.20, 0.15, 0.20, 0.08, 0.10, 0.12, 0.15], river_every: 380.0, lake: 0.30, lake_size: [20.0, 60.0], ridge: 0.20, rocks: 0.30, waterfall: 0.25, campfire: 0.12, birds: 0.030, patch: 170.0,
            relief: Relief { hills: 12.0, hills_scale: 380.0, valley: 0.5, ..R0 }, snow: 0.0, sand: 0.0, beaches: false, snowline: NO_SNOW, treeline: NO_SNOW, rock_slope: 0.9, temperature: 0.0, wind: 1.0, air: Air::Breeze, habitat: H_FOREST },
];

pub fn biome(index: u32) -> &'static Biome {
    &BIOMES[(index as usize).min(BIOMES.len() - 1)]
}

/// Index of a biome by name.
pub fn biome_index(name: &str) -> Option<usize> {
    BIOMES.iter().position(|b| b.name == name)
}

/// Crown centre height (m above ground), crown radius, trunk radius ranges per species.
fn tree_shape(sp: TreeSpecies, r: &mut Stream) -> (f32, f32, f32) {
    match sp {
        TreeSpecies::Birch => (r.range(6.0, 9.0), r.range(1.6, 2.6), r.range(0.10, 0.18)),
        TreeSpecies::Oak => (r.range(6.5, 9.5), r.range(2.8, 4.5), r.range(0.22, 0.40)),
        TreeSpecies::Spruce => (r.range(8.0, 12.0), r.range(1.6, 2.4), r.range(0.15, 0.28)),
        TreeSpecies::Willow => (r.range(4.0, 6.0), r.range(2.2, 3.5), r.range(0.18, 0.30)),
        TreeSpecies::Alder => (r.range(5.0, 7.5), r.range(1.8, 2.8), r.range(0.12, 0.22)),
        TreeSpecies::Aspen => (r.range(7.0, 10.0), r.range(1.8, 2.8), r.range(0.12, 0.22)),
        TreeSpecies::Pine => (r.range(9.0, 13.0), r.range(1.8, 2.8), r.range(0.15, 0.28)),
    }
}

/// Species a streamed bird can be (index = `Bird::species`). The first eight live in the
/// trees of the forest biomes (weights per tree species below); the new biomes draw from the
/// bird director's biome tables (`director::species_weights_for_biome`).
pub const BIRD_SPECIES: [Species; 12] = [
    Species::Chaffinch,
    Species::Robin,
    Species::SongThrush,
    Species::Nightingale,
    Species::WoodPigeon,
    Species::Cuckoo,
    Species::TawnyOwl,
    Species::Swallow,
    Species::Blackbird,
    Species::Skylark,
    Species::Gull,
    Species::Raven,
];

fn bird_weights(sp: TreeSpecies, edge: bool) -> [f32; 12] {
    let mut w = match sp {
        TreeSpecies::Spruce | TreeSpecies::Pine => [0.45, 0.30, 0.10, 0.0, 0.07, 0.0, 0.08, 0.0, 0.05, 0.0, 0.0, 0.0],
        TreeSpecies::Oak => [0.30, 0.10, 0.20, 0.0, 0.30, 0.10, 0.0, 0.0, 0.12, 0.0, 0.0, 0.0],
        TreeSpecies::Willow | TreeSpecies::Alder => [0.20, 0.20, 0.20, 0.40, 0.0, 0.0, 0.0, 0.0, 0.10, 0.0, 0.0, 0.0],
        TreeSpecies::Birch | TreeSpecies::Aspen => [0.35, 0.25, 0.20, 0.10, 0.0, 0.10, 0.0, 0.0, 0.10, 0.0, 0.0, 0.0],
    };
    if edge {
        w[5] += 0.15;
    }
    w
}

/// The bird director's biome for one of ours, when its birds come from the director's
/// tables (the five open biomes); None for the forest biomes (birds by tree species).
pub fn director_biome(b: &Biome) -> Option<&'static str> {
    match b.name {
        "desert" | "snowy_mountains" | "lake_shore" | "sea_coast" | "big_river" => Some(b.name),
        _ => None,
    }
}

/// Index of a species in `BIRD_SPECIES` (unknown species fall back to the chaffinch).
pub fn bird_index(sp: Species) -> u8 {
    BIRD_SPECIES.iter().position(|s| *s == sp).unwrap_or(0) as u8
}

/// Birds of the open: they sing or call in the air (larks hover, gulls and ravens soar).
pub fn aerial(sp: Species) -> bool {
    matches!(sp, Species::Skylark | Species::Gull | Species::Raven)
}

fn pick(weights: &[f32], u: f32) -> usize {
    let total: f32 = weights.iter().sum();
    let mut x = u * total;
    for (i, w) in weights.iter().enumerate() {
        if x < *w {
            return i;
        }
        x -= w;
    }
    weights.len() - 1
}

// ---------------------------------------------------------------------------------------
// Streamed content
// ---------------------------------------------------------------------------------------

#[derive(Clone, Copy, Default, Debug)]
pub struct Tree {
    pub id: u32,
    pub x: f64,
    pub y: f64,
    /// Ground height at the trunk, m.
    pub ground: f32,
    pub species: u8,
    /// Crown centre height above the ground, m.
    pub crown_z: f32,
    pub crown_r: f32,
    pub trunk_r: f32,
    /// Exists while `rank < density / 2` (user density thins the generated maximum).
    pub rank: f32,
}

impl Tree {
    pub fn species(&self) -> TreeSpecies {
        SPECIES[self.species as usize]
    }
    pub fn crown_area(&self) -> f32 {
        core::f32::consts::PI * self.crown_r * self.crown_r
    }
    pub fn top(&self) -> f32 {
        self.crown_z + 1.1 * self.crown_r
    }
}

/// Aggregate of one tile: its forest seen from afar.
#[derive(Clone, Copy, Default, Debug)]
pub struct Cell {
    pub x: f64,
    pub y: f64,
    /// Mean ground height under the forest, m.
    pub ground: f32,
    /// Crown area at user density (k + 1) / 4, k = 0..LEVELS.
    pub area: [f32; LEVELS],
    pub canopy_z: f32,
    /// Dominant species (index into `SPECIES`).
    pub species: u8,
}

impl Cell {
    pub fn area_at(&self, density: f32) -> f32 {
        let x = (density * 4.0 - 1.0).clamp(-1.0, (LEVELS - 1) as f32);
        if x < 0.0 {
            return self.area[0] * (1.0 + x);
        }
        let i = (x as usize).min(LEVELS - 2);
        let t = x - i as f32;
        self.area[i] * (1.0 - t) + self.area[i + 1] * t
    }
}

#[derive(Clone, Copy, Default, Debug)]
pub struct Tile {
    pub i: i32,
    pub j: i32,
    pub alive: bool,
    pub cell: Cell,
    /// Individual trees (and their birds) are generated.
    pub trees: bool,
    pub n_trees: u16,
    /// Journey: the biome at the tile's centre (dominant), and whether it is the only one
    /// anywhere on the tile (then lookups on the tile skip the region field).
    pub region: u8,
    pub uniform: bool,
}

#[derive(Clone, Copy, Default, Debug)]
struct Macro {
    i: i32,
    j: i32,
    alive: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FeatureKind {
    #[default]
    River,
    Lake,
    Ridge,
    Rock,
    Waterfall,
    Campfire,
    Meadow,
}

#[derive(Clone, Copy, Debug)]
pub struct Feature {
    pub id: u32,
    pub kind: FeatureKind,
    pub alive: bool,
    /// Representative world point (river: middle; lake: centre; ridge: middle).
    pub x: f64,
    pub y: f64,
    /// Height of its top (ridge, rock) or of the sound above the ground, m.
    pub z: f32,
    /// Bounding box of everything it shapes (valley, basin, crest) in world (x0, y0, x1, y1).
    pub bbox: [f64; 4],
    /// Polyline (river, ridge) or outline (lake) in world (x, y).
    pub path: [[f64; 2]; PATH_POINTS],
    pub path_len: u8,
    /// Half width (river, ridge), radius (rock), semi-axes a, b and rotation (lake).
    pub a: f32,
    pub b: f32,
    pub rot: f32,
    /// Water preset index (`sources::Water::PRESETS`) / insect preset for meadows.
    pub preset: u8,
    /// River width / rock radius / ridge height / lake's larger semi-axis, m.
    pub size: f32,
    /// Lake: water level, m.
    pub level: f32,
    /// River: water level at each path point, m (falling along the path: it flows that way).
    pub levels: [f32; PATH_POINTS],
}

impl Default for Feature {
    fn default() -> Self {
        Self {
            id: 0,
            kind: FeatureKind::River,
            alive: false,
            x: 0.0,
            y: 0.0,
            z: 0.0,
            bbox: [0.0; 4],
            path: [[0.0; 2]; PATH_POINTS],
            path_len: 0,
            a: 0.0,
            b: 0.0,
            rot: 0.0,
            preset: 0,
            size: 0.0,
            level: 0.0,
            levels: [0.0; PATH_POINTS],
        }
    }
}

/// Distance from (x, y) to a polyline: (distance, segment, position along it 0..1).
fn polyline_distance(pts: &[[f64; 2]], x: f64, y: f64) -> (f64, usize, f64) {
    let mut best = (f64::INFINITY, 0, 0.0);
    for (k, w) in pts.windows(2).enumerate() {
        let (ax, ay, bx, by) = (w[0][0], w[0][1], w[1][0], w[1][1]);
        let (dx, dy) = (bx - ax, by - ay);
        let l2 = (dx * dx + dy * dy).max(1e-9);
        let t = (((x - ax) * dx + (y - ay) * dy) / l2).clamp(0.0, 1.0);
        let (px, py) = (ax + t * dx - x, ay + t * dy - y);
        let d = (px * px + py * py).sqrt();
        if d < best.0 {
            best = (d, k, t);
        }
    }
    best
}

impl Feature {
    /// Within the feature's box (grown by `margin`).
    pub fn near_box(&self, x: f64, y: f64, margin: f64) -> bool {
        x >= self.bbox[0] - margin && x <= self.bbox[2] + margin && y >= self.bbox[1] - margin && y <= self.bbox[3] + margin
    }

    /// Extra margin the terrain function looks around the box (the box already includes the
    /// valley, basin and crest).
    pub fn reach(&self) -> f32 {
        0.0
    }

    /// Distance to the path: (m, segment, t).
    pub fn path_distance(&self, x: f64, y: f64) -> (f32, usize, f32) {
        let (d, k, t) = polyline_distance(&self.path[..self.path_len.max(1) as usize], x, y);
        (d as f32, k, t as f32)
    }

    /// River water level at segment `k`, position `t`.
    pub fn level_at(&self, k: usize, t: f32) -> f32 {
        let n = (self.path_len as usize).max(1);
        let a = self.levels[k.min(n - 1)];
        let b = self.levels[(k + 1).min(n - 1)];
        a + (b - a) * t
    }

    /// Unit direction of segment `k` (the way a river flows).
    pub fn path_dir(&self, k: usize) -> (f32, f32) {
        let n = self.path_len as usize;
        if n < 2 {
            return (1.0, 0.0);
        }
        let k = k.min(n - 2);
        let (dx, dy) = (self.path[k + 1][0] - self.path[k][0], self.path[k + 1][1] - self.path[k][1]);
        let l = (dx * dx + dy * dy).sqrt().max(1e-6);
        ((dx / l) as f32, (dy / l) as f32)
    }

    /// Surface flow speed of a river by its preset, m/s.
    pub fn flow_speed(&self) -> f32 {
        match self.preset {
            2 => 0.45,
            4 => 1.3,
            _ => 0.8,
        }
    }

    /// Elliptical radius of (x, y) for a lake (1 = on the shore).
    pub fn ellipse_radius(&self, x: f64, y: f64) -> f32 {
        let (s, c) = (self.rot as f64).sin_cos();
        let (dx, dy) = (x - self.x, y - self.y);
        let u = (c * dx + s * dy) / self.a.max(1.0) as f64;
        let v = (-s * dx + c * dy) / self.b.max(1.0) as f64;
        (u * u + v * v).sqrt() as f32
    }

    /// No tree at (x, y) (with `margin`): water, rock, a camp.
    pub fn carves(&self, x: f64, y: f64, margin: f32) -> bool {
        if !self.alive || !self.near_box(x, y, margin as f64 + 1.0) {
            return false;
        }
        let m = margin as f64;
        match self.kind {
            FeatureKind::River | FeatureKind::Ridge => polyline_distance(&self.path[..self.path_len as usize], x, y).0 < self.a as f64 * if self.kind == FeatureKind::Ridge { 0.75 } else { 1.0 } + m,
            FeatureKind::Lake => {
                let (s, c) = (self.rot as f64).sin_cos();
                let (dx, dy) = (x - self.x, y - self.y);
                let u = (c * dx + s * dy) / (self.a as f64 + m);
                let v = (-s * dx + c * dy) / (self.b as f64 + m);
                u * u + v * v < 1.0
            }
            FeatureKind::Rock => ((x - self.x).powi(2) + (y - self.y).powi(2)).sqrt() < self.a as f64 + m,
            FeatureKind::Campfire => ((x - self.x).powi(2) + (y - self.y).powi(2)).sqrt() < 6.0 + m,
            FeatureKind::Waterfall | FeatureKind::Meadow => false,
        }
    }

    /// Water on the ground at (x, y).
    pub fn covers(&self, x: f64, y: f64) -> bool {
        matches!(self.kind, FeatureKind::River | FeatureKind::Lake) && self.carves(x, y, 0.0)
    }

    fn finish(&mut self, pad: f64) {
        let mut bb = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
        let pts: &[[f64; 2]] = if self.path_len > 0 { &self.path[..self.path_len as usize] } else { &[] };
        for p in pts.iter().chain(core::iter::once(&[self.x, self.y])) {
            bb = [bb[0].min(p[0]), bb[1].min(p[1]), bb[2].max(p[0]), bb[3].max(p[1])];
        }
        self.bbox = [bb[0] - pad, bb[1] - pad, bb[2] + pad, bb[3] + pad];
    }
}

/// Which slot pool voices a site.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SiteKind {
    #[default]
    Water,
    Insects,
    Fire,
    /// Wind over exposed ground (preset index into `sources::Wind::PRESETS`).
    Air,
}

/// A point that can sound: a river segment, a lake shore, surf, a waterfall, crickets, a
/// campfire, wind over a crest.
#[derive(Clone, Copy, Debug, Default)]
pub struct Site {
    pub id: u32,
    pub alive: bool,
    pub kind: SiteKind,
    pub x: f64,
    pub y: f64,
    /// Height above the ground, or above the water where there is water, m.
    pub z: f32,
    /// Preset index in the model's PRESETS list.
    pub preset: u8,
    /// Loudness weight for the level-of-detail choice.
    pub weight: f32,
    /// Segment length (water) / patch size, m.
    pub length: f32,
    pub variation: [f32; 3],
    /// A place worth stopping at (lake shore, waterfall, surf, the big river, a viewpoint).
    pub scenic: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum BirdState {
    #[default]
    Perched,
    /// Taking off / in the air along a Bézier from `from` to `to`.
    Flying,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Bird {
    pub id: u32,
    pub alive: bool,
    pub species: u8,
    pub rank: f32,
    /// World position (x, y, height above ground).
    pub pos: [f64; 3],
    pub vel: [f32; 3],
    pub state: BirdState,
    pub from: [f64; 3],
    pub c1: [f64; 3],
    pub c2: [f64; 3],
    pub to: [f64; 3],
    pub t: f32,
    pub dur: f32,
    /// The listener came close: it decided whether to flush (once).
    pub checked: bool,
    /// Seconds of alarm after a flush (sings less).
    pub alarm: f32,
}

impl Bird {
    pub fn species(&self) -> Species {
        BIRD_SPECIES[self.species as usize]
    }
}

/// Object ids of the streamed world start here (static scene ids are small).
pub const ID_BASE: u32 = 0x4000_0000;

/// Insect-model presets of the frogs and the woodpecker (indices into `sources::Insects::PRESETS`).
const FROG_MARSH: u8 = 5;
const FROG_TREE: u8 = 6;
const FROG_COMMON: u8 = 7;
const WOODPECKER: u8 = 8;
const _: () = assert!(crate::sources::Insects::PRESETS.len() == 9);

/// Wind presets used for air sites (indices into `sources::Wind::PRESETS`).
pub fn air_preset(name: &str) -> u8 {
    crate::sources::Wind::PRESETS.iter().position(|p| *p == name).unwrap_or(0) as u8
}

/// Water presets used for sites (indices into `sources::Water::PRESETS`).
pub fn water_preset(name: &str) -> u8 {
    crate::sources::Water::PRESETS.iter().position(|p| *p == name).unwrap_or(3) as u8
}

/// The streamed world: tiles (terrain, aggregate cells, trees, birds, boulders, meadows,
/// surf, wind) and macro cells (rivers, lakes, ridges, falls, camps), each with their sound
/// sites.
pub struct Land {
    pub seed: u64,
    pub biome: u32,
    /// User relief scale (`flight.relief`).
    pub relief: f32,
    pub params: Params,
    pub tiles: Box<[Tile]>,
    pub trees: Box<[Tree]>,
    pub birds: Box<[Bird]>,
    macros: Box<[Macro]>,
    pub features: Box<[Feature]>,
    pub sites: Box<[Site]>,
    /// Per tile, `GRID_N`² terrain samples: ground height, water level (or `DRY`), surface.
    ground: Box<[f32]>,
    water: Box<[f32]>,
    surface: Box<[u8]>,
    next_id: u32,
}

fn tile_slot(i: i32, j: i32) -> usize {
    i.rem_euclid(W as i32) as usize * W + j.rem_euclid(W as i32) as usize
}

fn macro_slot(i: i32, j: i32) -> usize {
    i.rem_euclid(MW as i32) as usize * MW + j.rem_euclid(MW as i32) as usize
}

/// Centre of tile (i, j).
pub fn tile_centre(i: i32, j: i32) -> (f64, f64) {
    ((i as f64 + 0.5) * TILE, (j as f64 + 0.5) * TILE)
}

/// Margin around a river's path that its valley shapes, m.
fn river_pad(width: f32, valley: f32) -> f64 {
    (0.5 * width + 3.0 + (2.5 + 4.0 * valley) * 20.0 + 30.0 * valley + 4.0) as f64
}

impl Land {
    /// Allocates all storage. Not real-time safe.
    pub fn new(seed: u64, biome: u32) -> Self {
        Self::with_relief(seed, biome, 1.0)
    }

    pub fn with_relief(seed: u64, biome: u32, relief: f32) -> Self {
        Self {
            seed,
            biome,
            relief,
            params: Params::new(seed, self::biome(biome), relief),
            tiles: vec![Tile::default(); TILES].into_boxed_slice(),
            trees: vec![Tree::default(); TILES * TREES_PER_TILE].into_boxed_slice(),
            birds: vec![Bird::default(); TILES * BIRDS_PER_TILE].into_boxed_slice(),
            macros: vec![Macro::default(); MACROS].into_boxed_slice(),
            features: vec![Feature::default(); MAX_FEATURES].into_boxed_slice(),
            sites: vec![Site::default(); MAX_SITES].into_boxed_slice(),
            ground: vec![0.0; TILES * GRID_LEN].into_boxed_slice(),
            water: vec![DRY; TILES * GRID_LEN].into_boxed_slice(),
            surface: vec![0; TILES * GRID_LEN].into_boxed_slice(),
            next_id: ID_BASE,
        }
    }

    fn id(&mut self) -> u32 {
        self.next_id = self.next_id.wrapping_add(1).max(ID_BASE);
        self.next_id
    }

    pub fn biome_def(&self) -> &'static Biome {
        biome(self.biome)
    }

    /// The `journey` preset: the biome is a function of place.
    pub fn journey(&self) -> bool {
        self.params.journey
    }

    /// Journey: the biomes at (x, y) (from the tile when the whole tile is one biome).
    pub fn blend_at(&self, x: f64, y: f64) -> Blend {
        if let Some(t) = self.tile((x / TILE).floor() as i32, (y / TILE).floor() as i32) {
            if self.tiles[t].uniform {
                return Blend::single(self.tiles[t].region as u32);
            }
        }
        self.params.blend(x, y, 0.0)
    }

    /// The biome at (x, y): the preset, or in a journey the blend there, its discrete traits
    /// dithered (an ecotone: per place) or of the dominant biome.
    pub fn biome_at(&self, x: f64, y: f64, dither: bool) -> Biome {
        if !self.params.journey {
            return *self.biome_def();
        }
        let bl = self.blend_at(x, y);
        let pick = if dither { bl.pick(journey::dither(self.seed, x, y)) } else { bl.dominant() };
        journey::mix(&bl, pick)
    }

    /// The biome around the listener at (x, y) (discrete traits of the dominant one) and the
    /// index of the dominant biome.
    pub fn local(&self, x: f64, y: f64) -> (Biome, u32) {
        if !self.params.journey {
            return (*self.biome_def(), self.biome);
        }
        let bl = self.blend_at(x, y);
        (journey::mix(&bl, bl.dominant()), bl.dominant())
    }

    /// A numeric trait at (x, y), weighted across a journey's borders (snow and sand cover
    /// for the picture).
    pub fn share(&self, x: f64, y: f64, f: impl Fn(&Biome) -> f32) -> f32 {
        if !self.params.journey {
            return f(self.biome_def());
        }
        self.blend_at(x, y).lerp(f)
    }

    /// The biome a tile is made with (tile-wide choices: boulders, meadows, air, birds of the
    /// open, the aggregate's species).
    fn tile_biome(&self, tile: &Tile) -> Biome {
        if !self.params.journey {
            return *self.biome_def();
        }
        if tile.uniform {
            return *biome(tile.region as u32);
        }
        let (cx, cy) = tile_centre(tile.i, tile.j);
        let bl = self.params.blend(cx, cy, 0.0);
        journey::mix(&bl, bl.pick(journey::tile_dither(self.seed, tile.i, tile.j)))
    }

    /// Clears the content (ids keep counting) and switches seed / biome / relief. Real-time
    /// safe.
    pub fn reset(&mut self, seed: u64, biome: u32, relief: f32) {
        self.seed = seed;
        self.biome = biome;
        self.relief = relief;
        self.params = Params::new(seed, self::biome(biome), relief);
        for t in 0..TILES {
            self.drop_tile(t);
        }
        for m in 0..MACROS {
            self.drop_macro(m);
        }
    }

    /// Trees of tile slot `t`.
    pub fn tile_trees(&self, t: usize) -> &[Tree] {
        let tile = &self.tiles[t];
        if !tile.alive || !tile.trees {
            return &[];
        }
        &self.trees[t * TREES_PER_TILE..t * TREES_PER_TILE + tile.n_trees as usize]
    }

    /// Slot of the live tile (i, j), if any.
    pub fn tile(&self, i: i32, j: i32) -> Option<usize> {
        let s = tile_slot(i, j);
        let t = &self.tiles[s];
        (t.alive && t.i == i && t.j == j).then_some(s)
    }

    /// Tile slots around (x, y) within `r` metres (centre distance ≤ r + half diagonal).
    pub fn tiles_near(&self, x: f64, y: f64, r: f64) -> impl Iterator<Item = usize> + '_ {
        let k = (r / TILE).ceil() as i32 + 1;
        let (ci, cj) = ((x / TILE).floor() as i32, (y / TILE).floor() as i32);
        let reach = r + TILE * 0.71;
        (-k..=k).flat_map(move |a| (-k..=k).map(move |b| (ci + a, cj + b))).filter_map(move |(i, j)| {
            let (cx, cy) = tile_centre(i, j);
            if (cx - x).hypot(cy - y) > reach {
                return None;
            }
            self.tile(i, j)
        })
    }

    // -- terrain lookups (audio thread: cached, no evaluation) ------------------------------

    /// Tile slot and fractional grid position of (x, y) in its cached terrain.
    #[inline]
    fn cell_of(&self, x: f64, y: f64) -> Option<(usize, f32, f32)> {
        let (i, j) = ((x / TILE).floor() as i32, (y / TILE).floor() as i32);
        let t = self.tile(i, j)?;
        let fx = ((x - i as f64 * TILE) / GRID_STEP) as f32;
        let fy = ((y - j as f64 * TILE) / GRID_STEP) as f32;
        Some((t, fx.clamp(0.0, (GRID_N - 1) as f32), fy.clamp(0.0, (GRID_N - 1) as f32)))
    }

    /// Ground height at (x, y), m: bilinear in the tile caches; the base relief (without
    /// features) where no tile is live.
    pub fn ground(&self, x: f64, y: f64) -> f32 {
        match self.cell_of(x, y) {
            Some((t, fx, fy)) => {
                let (i0, j0) = ((fx as usize).min(GRID_N - 2), (fy as usize).min(GRID_N - 2));
                let (u, v) = (fx - i0 as f32, fy - j0 as f32);
                let g = &self.ground[t * GRID_LEN..(t + 1) * GRID_LEN];
                let at = |i: usize, j: usize| g[i * GRID_N + j];
                let a = at(i0, j0) + (at(i0 + 1, j0) - at(i0, j0)) * u;
                let b = at(i0, j0 + 1) + (at(i0 + 1, j0 + 1) - at(i0, j0 + 1)) * u;
                a + (b - a) * v
            }
            None => self.params.base(x, y).h,
        }
    }

    /// The top of whatever is there: the water surface over water, else the ground, m.
    pub fn top(&self, x: f64, y: f64) -> f32 {
        let g = self.ground(x, y);
        let (_, w, _) = self.nearest(x, y);
        if w > g {
            w
        } else {
            g
        }
    }

    /// Nearest cached sample: (ground, water level or DRY, surface).
    pub fn nearest(&self, x: f64, y: f64) -> (f32, f32, Surface) {
        match self.cell_of(x, y) {
            Some((t, fx, fy)) => {
                let k = t * GRID_LEN + (fx + 0.5) as usize * GRID_N + (fy + 0.5) as usize;
                (self.ground[k], self.water[k], Surface::from_u8(self.surface[k]))
            }
            None => {
                let b = self.params.base(x, y);
                let wet = b.h < 0.0 && b.coast.is_finite();
                (b.h, if wet { 0.0 } else { DRY }, if wet { Surface::Water } else { Surface::Grass })
            }
        }
    }

    /// What lies on the ground at (x, y).
    pub fn surface_at(&self, x: f64, y: f64) -> Surface {
        self.nearest(x, y).2
    }

    /// Habitat around (x, y) for the birds: the biome's, with water and coast nearby.
    pub fn habitat_at(&self, x: f64, y: f64) -> Habitat {
        let mut h = self.biome_at(x, y, false).habitat;
        let (g, w, s) = self.nearest(x, y);
        if w != DRY || s == Surface::Water {
            h.water = h.water.max(1.0);
        }
        if self.params.biome.relief.sea > 0.0 {
            h.coast *= 1.0 - smoothstep(4.0, 30.0, g);
        }
        h
    }

    /// Full terrain sample (pure, slow: evaluates the relief and scans the features). For the
    /// UI's sampler and the tile caches.
    pub fn sample(&self, x: f64, y: f64) -> Sample {
        let mut local = [0usize; LOCAL];
        let n = self.local_features(x, y, 1.0, &mut local);
        self.params.sample(&self.features, &local[..n], x, y)
    }

    /// Forest cover in [0, 1] at a world point, before carving.
    pub fn base_cover(&self, x: f64, y: f64) -> f32 {
        if self.params.journey {
            return self.cover_in(&self.blend_at(x, y), x, y);
        }
        self.cover_of(self.biome_def(), x, y)
    }

    /// Journey: the forest cover of each biome there, weighted.
    fn cover_in(&self, bl: &Blend, x: f64, y: f64) -> f32 {
        if bl.n <= 1 {
            return self.cover_of(biome(bl.dominant()), x, y);
        }
        bl.parts().map(|(b, w)| w * self.cover_of(b, x, y)).sum()
    }

    fn cover_of(&self, b: &Biome, x: f64, y: f64) -> f32 {
        if b.forest <= 0.0 {
            return 0.0;
        }
        let n = fbm(self.seed, 0xC0FE, x / b.patch as f64, y / b.patch as f64);
        let t = 0.5 + (0.5 - b.forest) * 0.45;
        smoothstep(t - 0.08, t + 0.08, n)
    }

    /// How much the terrain allows trees: none in water, on rock, on sand away from water,
    /// above the treeline.
    fn terrain_cover(b: &Biome, ground: f32, water: f32, surface: Surface, shore: f32) -> f32 {
        if water > ground {
            return 0.0;
        }
        let line = 1.0 - smoothstep(b.treeline - 15.0, b.treeline + 5.0, ground);
        match surface {
            Surface::Water | Surface::Rock => 0.0,
            // Oases and beaches: a few trees near the water.
            Surface::Sand => if shore < 4.0 && b.sand > 0.5 { 0.6 } else { 0.0 },
            _ => line,
        }
    }

    /// Forest cover with water, rock and camps carved out.
    pub fn cover(&self, x: f64, y: f64) -> f32 {
        if self.features.iter().any(|f| f.carves(x, y, 1.5)) {
            return 0.0;
        }
        let (g, w, s) = self.nearest(x, y);
        self.base_cover(x, y) * Self::terrain_cover(&self.biome_at(x, y, true), g, w, s, 10.0)
    }

    /// `cover` using only the features in `near` (from `local_features`).
    pub fn cover_near(&self, x: f64, y: f64, near: &[usize]) -> f32 {
        if near.iter().any(|&k| self.features[k].carves(x, y, 1.5)) {
            return 0.0;
        }
        let (g, w, s) = self.nearest(x, y);
        self.base_cover(x, y) * Self::terrain_cover(&self.biome_at(x, y, true), g, w, s, 10.0)
    }

    /// Streams content around (x, y): macro cells, tiles within KEEP, trees within TREE_R;
    /// retires what is too far. Generates at most `budget` tiles and `budget` tree sets per
    /// call (amortised on the audio thread), nearest first; never allocates.
    pub fn stream(&mut self, x: f64, y: f64, budget: usize) {
        // Macro cells first: tiles carve their terrain and forest around the features.
        let (mi, mj) = ((x / MACRO).floor() as i32, (y / MACRO).floor() as i32);
        for m in 0..MACROS {
            let c = self.macros[m];
            if c.alive && ((c.i as f64 + 0.5) * MACRO - x).hypot((c.j as f64 + 0.5) * MACRO - y) > MACRO_R + MACRO {
                self.drop_macro(m);
            }
        }
        for a in -3..=3 {
            for b in -3..=3 {
                let (i, j) = (mi + a, mj + b);
                if ((i as f64 + 0.5) * MACRO - x).hypot((j as f64 + 0.5) * MACRO - y) > MACRO_R {
                    continue;
                }
                let s = macro_slot(i, j);
                let c = self.macros[s];
                if !(c.alive && c.i == i && c.j == j) {
                    self.drop_macro(s);
                    self.make_macro(i, j);
                }
            }
        }
        // Tiles.
        for t in 0..TILES {
            let tile = self.tiles[t];
            if !tile.alive {
                continue;
            }
            let (cx, cy) = tile_centre(tile.i, tile.j);
            let d = (cx - x).hypot(cy - y);
            if d > KEEP + 40.0 {
                self.drop_tile(t);
            } else if tile.trees && d > TREE_R + 30.0 {
                self.drop_trees(t);
            }
        }
        let k = (KEEP / TILE).ceil() as i32;
        let (ci, cj) = ((x / TILE).floor() as i32, (y / TILE).floor() as i32);
        let (mut made, mut grown) = (0, 0);
        // Rings outwards, so the ground under the listener comes first.
        for ring in 0..=k {
            for a in -ring..=ring {
                for b in -ring..=ring {
                    if a.abs() != ring && b.abs() != ring {
                        continue;
                    }
                    let (i, j) = (ci + a, cj + b);
                    let (cx, cy) = tile_centre(i, j);
                    let d = (cx - x).hypot(cy - y);
                    if d > KEEP {
                        continue;
                    }
                    let s = tile_slot(i, j);
                    let t = self.tiles[s];
                    if !(t.alive && t.i == i && t.j == j) {
                        if made >= budget {
                            continue;
                        }
                        self.drop_tile(s);
                        self.make_tile(i, j);
                        made += 1;
                    }
                    if d <= TREE_R && !self.tiles[s].trees && grown < budget {
                        self.make_trees(s);
                        grown += 1;
                    }
                }
            }
        }
    }

    /// Streams only the macro cells around (x, y) (the UI's terrain sampler).
    pub fn stream_macros(&mut self, x: f64, y: f64) {
        let (mi, mj) = ((x / MACRO).floor() as i32, (y / MACRO).floor() as i32);
        for a in -3..=3 {
            for b in -3..=3 {
                let (i, j) = (mi + a, mj + b);
                if ((i as f64 + 0.5) * MACRO - x).hypot((j as f64 + 0.5) * MACRO - y) > MACRO_R {
                    continue;
                }
                let s = macro_slot(i, j);
                let c = self.macros[s];
                if !(c.alive && c.i == i && c.j == j) {
                    self.drop_macro(s);
                    self.make_macro(i, j);
                }
            }
        }
    }

    fn drop_tile(&mut self, t: usize) {
        self.drop_trees(t);
        self.tiles[t].alive = false;
        let f0 = MACROS * FEATURES_PER_MACRO + t * FEATURES_PER_TILE;
        for f in &mut self.features[f0..f0 + FEATURES_PER_TILE] {
            f.alive = false;
        }
        let s0 = MACROS * SITES_PER_MACRO + t * SITES_PER_TILE;
        for x in &mut self.sites[s0..s0 + SITES_PER_TILE] {
            x.alive = false;
        }
    }

    fn drop_trees(&mut self, t: usize) {
        self.tiles[t].trees = false;
        self.tiles[t].n_trees = 0;
        for b in &mut self.birds[t * BIRDS_PER_TILE..(t + 1) * BIRDS_PER_TILE] {
            b.alive = false;
        }
    }

    fn drop_macro(&mut self, m: usize) {
        self.macros[m].alive = false;
        for f in &mut self.features[m * FEATURES_PER_MACRO..(m + 1) * FEATURES_PER_MACRO] {
            f.alive = false;
        }
        for x in &mut self.sites[m * SITES_PER_MACRO..(m + 1) * SITES_PER_MACRO] {
            x.alive = false;
        }
    }

    /// Adds a feature in the range starting at `f0` (len `n`); returns its index.
    fn put_feature(&mut self, f0: usize, n: usize, mut f: Feature, pad: f64) -> Option<usize> {
        let k = (f0..f0 + n).find(|&k| !self.features[k].alive)?;
        f.id = self.id();
        f.alive = true;
        f.finish(pad);
        self.features[k] = f;
        Some(k)
    }

    fn put_site(&mut self, s0: usize, n: usize, mut x: Site) {
        if let Some(k) = (s0..s0 + n).find(|&k| !self.sites[k].alive) {
            x.id = self.id();
            x.alive = true;
            self.sites[k] = x;
        }
    }

    // -- macro cells ----------------------------------------------------------------------

    fn make_macro(&mut self, i: i32, j: i32) {
        let m = macro_slot(i, j);
        self.macros[m] = Macro { i, j, alive: true };
        // Journey: the macro cell's features follow the biome at its centre.
        let b = if self.params.journey { self.local((i as f64 + 0.5) * MACRO, (j as f64 + 0.5) * MACRO).0 } else { *self.biome_def() };
        // (A journey's valleys widen and narrow across borders: pad for the widest.)
        let pad_valley = if self.params.journey { 1.0 } else { b.relief.valley };
        let p = self.params;
        let mut r = Stream::new(hash(self.seed, i as i64, j as i64, 0x3AC0 ^ self.biome as u64));
        let (x0, y0) = (i as f64 * MACRO, j as f64 * MACRO);
        let f0 = m * FEATURES_PER_MACRO;
        let s0 = m * SITES_PER_MACRO;
        let at = |r: &mut Stream| (x0 + r.uniform() as f64 * MACRO, y0 + r.uniform() as f64 * MACRO);
        // (Without a sea the coast is infinitely far: no need to evaluate the relief.)
        let in_sea = |x: f64, y: f64| p.biome.relief.sea > 0.0 && p.base(x, y).coast < 15.0;

        // Brooks and rivers: on average one crossing per `river_every` m of travel.
        let q = 0.9 * MACRO as f32 / b.river_every;
        let n_rivers = q.floor() as usize + usize::from(r.uniform() < q.fract());
        for _ in 0..n_rivers.min(2) {
            let (cx, cy) = at(&mut r);
            let rocky = b.relief.mountains > 0.0 || b.snow > 0.5;
            let (preset, width) = match pick(&[0.5, if rocky { 0.45 } else { 0.25 }, if rocky { 0.05 } else { 0.25 }], r.uniform()) {
                0 => (3u8, r.range(1.2, 2.8)),
                1 => (4u8, r.range(2.0, 4.0)),
                _ => (2u8, r.range(7.0, 14.0)),
            };
            let th = r.range(0.0, core::f32::consts::TAU) as f64;
            let (dx, dy) = (th.cos(), th.sin());
            let (amp, wl, ph) = (r.range(3.0, 12.0) as f64, r.range(60.0, 130.0) as f64, r.range(0.0, core::f32::consts::TAU) as f64);
            let half = 215.0;
            if in_sea(cx, cy) {
                continue;
            }
            let mut f = Feature { kind: FeatureKind::River, x: cx, y: cy, z: 0.1, a: 0.5 * width, preset, size: width, ..Default::default() };
            for k in 0..PATH_POINTS {
                let t = -half + 2.0 * half * k as f64 / (PATH_POINTS - 1) as f64;
                let off = amp * (core::f64::consts::TAU * t / wl + ph).sin();
                f.path[k] = [cx + t * dx - off * dy, cy + t * dy + off * dx];
            }
            f.path_len = PATH_POINTS as u8;
            // Water level: the land along the bed, smoothed, and falling along the course
            // (a running minimum): the river cuts through rises and flows one way.
            let raw: [f32; PATH_POINTS] = core::array::from_fn(|k| p.base(f.path[k][0], f.path[k][1]).h);
            let mut lv = [0.0f32; PATH_POINTS];
            for k in 0..PATH_POINTS {
                let (a, c) = (raw[k.saturating_sub(1)], raw[(k + 1).min(PATH_POINTS - 1)]);
                lv[k] = ((a + 2.0 * raw[k] + c) * 0.25).min(raw[k]) - 0.4;
                if k > 0 {
                    lv[k] = lv[k].min(lv[k - 1] - 0.03);
                }
            }
            f.levels = lv;
            if self.put_feature(f0, FEATURES_PER_MACRO, f, river_pad(width, pad_valley)).is_none() {
                break;
            }
            // Sound sites every 15 m along the bed, riffles and pools alternating.
            let weight = if preset == 2 { 1.6 } else { 1.0 };
            let mut t = -half + 7.5;
            while t < half {
                let off = amp * (core::f64::consts::TAU * t / wl + ph).sin();
                let sv = (core::f64::consts::TAU * t / 45.0 + ph).sin() as f32 + 0.4 * (r.uniform() - 0.5);
                let variation = [(0.2 * sv).exp(), (-0.15 * sv).exp(), -0.05 * sv];
                let (sx, sy) = (cx + t * dx - off * dy, cy + t * dy + off * dx);
                if !in_sea(sx, sy) {
                    let site = Site { kind: SiteKind::Water, x: sx, y: sy, z: 0.1, preset, weight, length: 15.0, variation, ..Default::default() };
                    self.put_site(s0, SITES_PER_MACRO, site);
                    // Marsh frogs along a slow river, here and there.
                    if preset == 2 && unit(hash(self.seed, sx as i64, sy as i64, 0xF407)) < 0.2 {
                        let site = Site { kind: SiteKind::Insects, x: sx - 0.5 * width as f64 * dy, y: sy + 0.5 * width as f64 * dx, z: 0.1, preset: FROG_MARSH, weight: 1.0, length: 10.0, variation: [1.0, 1.0, 0.0], ..Default::default() };
                        self.put_site(s0, SITES_PER_MACRO, site);
                    }
                }
                t += 15.0;
            }
        }

        // A lake in a basin, at the level of the lowest point of its rim.
        if r.uniform() < 0.8 * b.lake {
            let (cx, cy) = at(&mut r);
            let (a, bb, rot) = (r.range(b.lake_size[0], b.lake_size[1]), r.range(0.75 * b.lake_size[0], 0.8 * b.lake_size[1]), r.range(0.0, core::f32::consts::PI));
            let (s, c) = (rot as f64).sin_cos();
            let ell = |t: f64, shrink: f64| {
                let (u, v) = ((a as f64 - shrink) * t.cos(), (bb as f64 - shrink) * t.sin());
                [cx + c * u - s * v, cy + s * u + c * v]
            };
            let mut level = f32::INFINITY;
            for k in 0..16 {
                let q = ell(core::f64::consts::TAU * k as f64 / 16.0, 0.0);
                level = level.min(p.base(q[0], q[1]).h);
            }
            let level = level - 0.3;
            let wet = b.relief.sea > 0.0 && level < 1.0;
            if !wet {
                let mut f = Feature { kind: FeatureKind::Lake, x: cx, y: cy, z: 0.05, a, b: bb, rot, preset: 1, size: a.max(bb), level, ..Default::default() };
                for k in 0..PATH_POINTS {
                    f.path[k] = ell(core::f64::consts::TAU * k as f64 / PATH_POINTS as f64, 0.0);
                }
                f.path_len = PATH_POINTS as u8;
                let open_water = if self.params.journey { self.biome_at(cx, cy, false).snow < 0.5 } else { b.snow < 0.5 };
                if self.put_feature(f0, FEATURES_PER_MACRO, f, 0.8 * a.max(bb) as f64 + 4.0).is_some() && open_water {
                    // Waves lap the shore every ~16 m (none on a frozen lake).
                    let n = ((core::f32::consts::TAU * 0.5 * (a + bb)) / 16.0).ceil().clamp(6.0, 40.0) as usize;
                    for k in 0..n {
                        let q = ell(core::f64::consts::TAU * (k as f64 + 0.5) / n as f64, 1.5);
                        let site = Site { kind: SiteKind::Water, x: q[0], y: q[1], z: 0.05, preset: 1, weight: 0.7, length: 16.0, variation: [1.0, 1.0, 0.0], scenic: true, ..Default::default() };
                        self.put_site(s0, SITES_PER_MACRO, site);
                    }
                    // Frogs in the shallows: a few choruses round the shore (own hash, so the
                    // rest of the cell is unchanged).
                    let hf = hash(self.seed, cx as i64, cy as i64, 0xF406);
                    let kind = unit(hf);
                    let preset = if kind < 0.5 { FROG_MARSH } else if kind < 0.8 { FROG_TREE } else { FROG_COMMON };
                    let m = 2 + (hf >> 50) as usize % 2;
                    for k in 0..m {
                        let q = ell(core::f64::consts::TAU * (k as f64 + unit(hf >> 8) as f64) / m as f64, -2.0);
                        let site = Site { kind: SiteKind::Insects, x: q[0], y: q[1], z: 0.1, preset, weight: 1.0, length: 10.0, variation: [1.0, 1.0, 0.0], ..Default::default() };
                        self.put_site(s0, SITES_PER_MACRO, site);
                    }
                }
            }
        }

        // A rocky ridge (a crest in the relief), maybe with a waterfall down its face.
        if r.uniform() < 0.8 * b.ridge {
            let (cx, cy) = at(&mut r);
            let half = r.range(40.0, 90.0) as f64;
            let th = r.range(0.0, core::f32::consts::PI) as f64;
            let (dx, dy) = (th.cos(), th.sin());
            let bend = r.range(-10.0, 10.0) as f64;
            let height = r.range(6.0, 16.0) * (1.0 + 0.6 * (b.relief.mountains > 0.0) as u8 as f32);
            let width = r.range(10.0, 18.0);
            if !in_sea(cx, cy) {
                let mut f = Feature { kind: FeatureKind::Ridge, x: cx, y: cy, z: height, a: width, size: height, ..Default::default() };
                f.path[0] = [cx - half * dx - bend * dy, cy - half * dy + bend * dx];
                f.path[1] = [cx, cy];
                f.path[2] = [cx + half * dx - bend * dy, cy + half * dy + bend * dx];
                f.path_len = 3;
                if self.put_feature(f0, FEATURES_PER_MACRO, f, width as f64 + 2.0).is_some() && r.uniform() < b.waterfall {
                    let t = r.range(-0.4, 0.4) as f64 * half;
                    let side = r.sign() as f64;
                    let o = 0.55 * width as f64;
                    let (wx, wy) = (cx + t * dx - side * o * dy, cy + t * dy + side * o * dx);
                    let fall = Feature { kind: FeatureKind::Waterfall, x: wx, y: wy, z: 0.5 * height, preset: 6, size: height, ..Default::default() };
                    if self.put_feature(f0, FEATURES_PER_MACRO, fall, 3.0).is_some() {
                        let site = Site { kind: SiteKind::Water, x: wx, y: wy, z: 0.25 * height, preset: 6, weight: 2.5, length: 4.0, variation: [1.0, 1.0, 0.0], scenic: true, ..Default::default() };
                        self.put_site(s0, SITES_PER_MACRO, site);
                        let o2 = width as f64 + 4.0;
                        let (px, py) = (cx + t * dx - side * o2 * dy, cy + t * dy + side * o2 * dx);
                        let pool = Site { kind: SiteKind::Water, x: px, y: py, z: 0.2, preset: 4, weight: 1.2, length: 6.0, variation: [1.0, 1.0, 0.0], ..Default::default() };
                        self.put_site(s0, SITES_PER_MACRO, pool);
                    }
                }
            }
        }

        // A campfire in the most open of a few spots.
        if r.uniform() < 0.6 * b.campfire {
            let mut best = (0.0, 0.0, 2.0f32);
            for _ in 0..8 {
                let (px, py) = at(&mut r);
                let c = self.base_cover(px, py) + if in_sea(px, py) { 3.0 } else { 0.0 };
                if c < best.2 {
                    best = (px, py, c);
                }
            }
            if best.2 < 2.0 {
                let f = Feature { kind: FeatureKind::Campfire, x: best.0, y: best.1, z: 0.3, ..Default::default() };
                if self.put_feature(f0, FEATURES_PER_MACRO, f, 6.0).is_some() {
                    let site = Site { kind: SiteKind::Fire, x: best.0, y: best.1, z: 0.3, preset: 0, weight: 1.0, length: 1.0, variation: [1.0, 1.0, 0.0], scenic: true, ..Default::default() };
                    self.put_site(s0, SITES_PER_MACRO, site);
                }
            }
        }

        // The big river: its current mid-stream and lapping along both banks.
        if b.relief.big_river > 0.0 {
            let half = 0.5 * b.relief.big_river;
            let (mid, bank) = (water_preset("big_river"), water_preset("river_bank"));
            const STEP: f64 = 30.0;
            let n = (MACRO / STEP) as i64;
            for a in 0..n {
                for c in 0..n {
                    let (x, y) = (x0 + (a as f64 + 0.5) * STEP, y0 + (c as f64 + 0.5) * STEP);
                    let Some((d, _, _)) = p.big_river_at(x, y) else { continue };
                    let ad = d.abs();
                    if ad < 0.5 * STEP as f32 {
                        let site = Site { kind: SiteKind::Water, x, y, z: 0.1, preset: mid, weight: 1.8, length: 30.0, variation: [1.0, 1.0, 0.0], ..Default::default() };
                        self.put_site(s0, SITES_PER_MACRO, site);
                    } else if (ad - half + 2.0).abs() < 0.5 * STEP as f32 {
                        let site = Site { kind: SiteKind::Water, x, y, z: 0.05, preset: bank, weight: 1.0, length: 30.0, variation: [1.0, 1.0, 0.0], scenic: (hash(self.seed, x as i64, y as i64, 0x5CE) & 7) == 0, ..Default::default() };
                        self.put_site(s0, SITES_PER_MACRO, site);
                    }
                }
            }
        }
    }

    // -- tiles ----------------------------------------------------------------------------

    /// Indices of live features whose box touches the square around (cx, cy).
    pub fn local_features(&self, cx: f64, cy: f64, half: f64, out: &mut [usize; LOCAL]) -> usize {
        let mut n = 0;
        for (k, f) in self.features.iter().enumerate() {
            if n < out.len() && f.alive && f.bbox[0] <= cx + half && f.bbox[2] >= cx - half && f.bbox[1] <= cy + half && f.bbox[3] >= cy - half {
                out[n] = k;
                n += 1;
            }
        }
        n
    }

    fn make_tile(&mut self, i: i32, j: i32) {
        let t = tile_slot(i, j);
        let (x0, y0) = (i as f64 * TILE, j as f64 * TILE);
        let mut r = Stream::new(hash(self.seed, i as i64, j as i64, 0x711E ^ self.biome as u64));
        let journey = self.params.journey;
        // Journey: one biome over the whole tile, or a border tile (per-sample lookups).
        let (region, uniform) = if journey {
            let bl = self.params.blend(x0 + 0.5 * TILE, y0 + 0.5 * TILE, journey::TILE_MARGIN);
            (bl.dominant() as u8, bl.margin >= journey::BLEND + journey::TILE_MARGIN)
        } else {
            (self.biome as u8, true)
        };
        self.tiles[t] = Tile { i, j, alive: true, cell: Cell::default(), trees: false, n_trees: 0, region, uniform };
        let b = self.tile_biome(&self.tiles[t]);
        let mut blends = [Blend::single(region as u32); GRID_LEN];
        let f0 = MACROS * FEATURES_PER_MACRO + t * FEATURES_PER_TILE;
        let s0 = MACROS * SITES_PER_MACRO + t * SITES_PER_TILE;
        let mut local = [0usize; LOCAL];
        let nl = self.local_features(x0 + 0.5 * TILE, y0 + 0.5 * TILE, 0.5 * TILE + 2.0, &mut local);

        // Terrain on the 5 m grid: ground, water, surface (slope from the neighbours).
        let mut shore = [0.0f32; GRID_LEN];
        for a in 0..GRID_N {
            for c in 0..GRID_N {
                let (x, y) = (x0 + a as f64 * GRID_STEP, y0 + c as f64 * GRID_STEP);
                let s = if journey {
                    if !uniform {
                        blends[a * GRID_N + c] = self.params.blend(x, y, 0.0);
                    }
                    self.params.sample_in(&blends[a * GRID_N + c], &self.features, &local[..nl], x, y)
                } else {
                    self.params.sample(&self.features, &local[..nl], x, y)
                };
                let k = t * GRID_LEN + a * GRID_N + c;
                self.ground[k] = s.ground;
                self.water[k] = s.water;
                shore[a * GRID_N + c] = s.shore;
            }
        }
        for a in 0..GRID_N {
            for c in 0..GRID_N {
                let k = t * GRID_LEN + a * GRID_N + c;
                let g = |a: usize, c: usize| self.ground[t * GRID_LEN + a * GRID_N + c];
                let (a0, a1) = (a.saturating_sub(1), (a + 1).min(GRID_N - 1));
                let (c0, c1) = (c.saturating_sub(1), (c + 1).min(GRID_N - 1));
                let gx = (g(a1, c) - g(a0, c)) / ((a1 - a0) as f32 * GRID_STEP as f32);
                let gy = (g(a, c1) - g(a, c0)) / ((c1 - c0) as f32 * GRID_STEP as f32);
                let slope = (gx * gx + gy * gy).sqrt();
                let (x, y) = (x0 + a as f64 * GRID_STEP, y0 + c as f64 * GRID_STEP);
                let s = Sample { ground: self.ground[k], water: self.water[k], flow: [0.0; 2], shore: shore[a * GRID_N + c], kind: if self.water[k] > self.ground[k] { 1 } else { 0 } };
                self.surface[k] = if journey {
                    // The ground of the biome here, an ecotone dithered across borders.
                    let bl = &blends[a * GRID_N + c];
                    let lb = journey::ground(bl, bl.pick(journey::dither(self.seed, x, y)));
                    Params::surface_of(&lb, &s, slope, self.cover_in(bl, x, y)) as u8
                } else {
                    let cover = self.base_cover(x, y);
                    self.params.surface(&s, slope, cover) as u8
                };
            }
        }
        let carved = |land: &Land, x: f64, y: f64, m: f32| local[..nl].iter().any(|&k| land.features[k].carves(x, y, m));

        // Boulders in rocky country (not in water).
        if r.uniform() < b.rocks * 0.3 {
            let n = 1 + (r.uniform() * 3.0) as usize;
            for _ in 0..n {
                let (x, y) = (x0 + r.uniform() as f64 * TILE, y0 + r.uniform() as f64 * TILE);
                let radius = r.range(0.5, 1.8);
                let (g, w, _) = self.nearest(x, y);
                if carved(self, x, y, 1.0) || w > g - 0.3 {
                    continue;
                }
                let f = Feature { kind: FeatureKind::Rock, x, y, z: radius * 1.4, a: radius, size: radius, ..Default::default() };
                let _ = self.put_feature(f0, FEATURES_PER_TILE - 1, f, radius as f64 + 1.0);
            }
        }

        // A cricket or grasshopper meadow in the open grass.
        for _ in 0..4 {
            let (x, y) = (x0 + r.uniform() as f64 * TILE, y0 + r.uniform() as f64 * TILE);
            let preset = if r.uniform() < 0.6 { 0u8 } else { 3u8 };
            if self.base_cover(x, y) > 0.12 || carved(self, x, y, 2.0) || self.surface_at(x, y) != Surface::Grass {
                continue;
            }
            let f = Feature { kind: FeatureKind::Meadow, x, y, z: 0.1, a: 8.0, preset, size: 8.0, ..Default::default() };
            if self.put_feature(f0 + FEATURES_PER_TILE - 1, 1, f, 8.0).is_some() {
                let site = Site { kind: SiteKind::Insects, x, y, z: 0.1, preset, weight: 1.0, length: 8.0, variation: [1.0, 1.0, 0.0], ..Default::default() };
                self.put_site(s0, SITES_PER_TILE, site);
            }
            break;
        }

        // A woodpecker drumming on a dead branch somewhere in the forest (own hash).
        let hw = hash(self.seed, i as i64, j as i64, 0xD0CC);
        if unit(hw) < 0.1 {
            let (x, y) = (x0 + unit(hw >> 8) as f64 * TILE, y0 + unit(hw >> 32) as f64 * TILE);
            if self.base_cover(x, y) > 0.5 && !carved(self, x, y, 2.0) {
                let site = Site { kind: SiteKind::Insects, x, y, z: 5.0 + 6.0 * unit(hw >> 16), preset: WOODPECKER, weight: 1.0, length: 1.0, variation: [1.0, 1.0, 0.0], ..Default::default() };
                self.put_site(s0, SITES_PER_TILE, site);
            }
        }

        // Surf where the sea meets the land: the shore sample nearest the tile's centre.
        if b.relief.sea > 0.0 {
            let mut best: Option<(f32, usize, usize)> = None;
            for a in 0..GRID_N {
                for c in 0..GRID_N {
                    let k = t * GRID_LEN + a * GRID_N + c;
                    let sea = self.water[k] == 0.0 && self.ground[k] < 0.0;
                    if !sea {
                        continue;
                    }
                    let land = [(1i32, 0i32), (-1, 0), (0, 1), (0, -1)].iter().any(|&(da, dc)| {
                        let (aa, cc) = (a as i32 + da, c as i32 + dc);
                        (0..GRID_N as i32).contains(&aa) && (0..GRID_N as i32).contains(&cc) && self.ground[t * GRID_LEN + aa as usize * GRID_N + cc as usize] >= 0.0
                    });
                    let d = (a as f32 - 4.0).hypot(c as f32 - 4.0);
                    if land && best.map_or(true, |x| d < x.0) {
                        best = Some((d, a, c));
                    }
                }
            }
            if let Some((_, a, c)) = best {
                let (x, y) = (x0 + a as f64 * GRID_STEP, y0 + c as f64 * GRID_STEP);
                // Steep shores (cliffs) throw the surf on rocks and shingle.
                let k = t * GRID_LEN + a * GRID_N + c;
                let steep = (0..GRID_N).any(|cc| self.ground[t * GRID_LEN + a * GRID_N + cc] > 8.0) || self.ground[k] < -6.0;
                let preset = water_preset(if steep { "rock_surf" } else { "sea_surf" });
                let site = Site { kind: SiteKind::Water, x, y, z: 0.4, preset, weight: 3.0, length: 40.0, variation: [1.0, 1.0, 0.0], scenic: r.uniform() < 0.25, ..Default::default() };
                self.put_site(s0, SITES_PER_TILE, site);
            }
        }

        // Wind over the most exposed point: dune crests, rocky ridges, open hilltops.
        if b.air != Air::None {
            let mut sum = 0.0;
            let mut best = (f32::MIN, 0usize, 0usize);
            for a in 0..GRID_N {
                for c in 0..GRID_N {
                    let g = self.ground[t * GRID_LEN + a * GRID_N + c];
                    sum += g;
                    if g > best.0 {
                        best = (g, a, c);
                    }
                }
            }
            let mean = sum / GRID_LEN as f32;
            let k = t * GRID_LEN + best.1 * GRID_N + best.2;
            let rise = best.0 - mean;
            let (x, y) = (x0 + best.1 as f64 * GRID_STEP, y0 + best.2 as f64 * GRID_STEP);
            let surf = Surface::from_u8(self.surface[k]);
            let open = self.base_cover(x, y) < 0.3;
            let (ok, preset, weight) = match b.air {
                Air::Sand => (rise > 1.5 && surf == Surface::Sand, air_preset("dune_sand"), 0.6 + 0.15 * rise.min(8.0)),
                Air::Ridge => (rise > 3.0 && matches!(surf, Surface::Rock | Surface::Snow) || rise > 6.0, air_preset(if b.snow > 0.5 && r.uniform() < 0.5 { "blowing_snow" } else { "ridge_howl" }), 0.5 + 0.06 * rise.min(20.0)),
                Air::Breeze => (rise > 2.5 && open && surf == Surface::Grass && r.uniform() < 0.35, air_preset("breeze"), 0.5),
                Air::None => (false, 0, 0.0),
            };
            if ok && self.water[k] == DRY {
                let site = Site { kind: SiteKind::Air, x, y, z: 1.2, preset, weight, length: 20.0, variation: [1.0, 1.0, 0.0], scenic: rise > 10.0 && r.uniform() < 0.3, ..Default::default() };
                self.put_site(s0, SITES_PER_TILE, site);
            }
        }

        // The aggregate cell: crown area from the cover field on a 4 x 4 grid.
        let mut cov = 0.0;
        let mut gsum = 0.0;
        for a in 0..4 {
            for c in 0..4 {
                let (x, y) = (x0 + (a as f64 + 0.5) * TILE / 4.0, y0 + (c as f64 + 0.5) * TILE / 4.0);
                let (g, w, s) = self.nearest(x, y);
                gsum += g;
                cov += if carved(self, x, y, 1.5) { 0.0 } else { self.base_cover(x, y) * Self::terrain_cover(&self.biome_at(x, y, true), g, w, s, 10.0) };
            }
        }
        let area1 = cov / 16.0 * (TILE * TILE) as f32 * 0.65;
        let mut cell = Cell { x: x0 + 0.5 * TILE, y: y0 + 0.5 * TILE, ground: gsum / 16.0, canopy_z: 8.0, ..Default::default() };
        for (l, a) in cell.area.iter_mut().enumerate() {
            *a = area1 * (l as f32 + 1.0) * 0.25;
        }
        let mut w = b.species;
        for (k, x) in w.iter_mut().enumerate() {
            *x *= 0.3 + 1.4 * noise2(self.seed, 0x5EC0 + k as u64, cell.x / 90.0, cell.y / 90.0);
        }
        cell.species = w.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).map_or(0, |x| x.0) as u8;
        self.tiles[t].cell = cell;
    }

    /// Individual trees (generated at twice the biome density; the user's density keeps
    /// `rank < density / 2`) and the birds living in them.
    fn make_trees(&mut self, t: usize) {
        let tile = self.tiles[t];
        let b = self.tile_biome(&tile);
        // A journey's border tile: each tree takes the biome where it stands (dithered).
        let border = self.params.journey && !tile.uniform;
        let (x0, y0) = (tile.i as f64 * TILE, tile.j as f64 * TILE);
        let mut local = [0usize; LOCAL];
        let nl = self.local_features(x0 + 0.5 * TILE, y0 + 0.5 * TILE, 0.5 * TILE + 20.0, &mut local);
        let carved = |land: &Land, x: f64, y: f64, m: f32| local[..nl].iter().any(|&k| land.features[k].carves(x, y, m));
        let p_cell = (2.0 * b.density_ha * (GRID * GRID) as f32 / 10_000.0).min(1.0);
        let n_grid = (TILE / GRID).round() as i64;
        let habitat = self.habitat_at(x0 + 0.5 * TILE, y0 + 0.5 * TILE);
        // A border tile looks its biomes up on a 10 m grid (the blend there, numeric traits
        // mixed), each tree then dithers the discrete ones where it stands.
        const SUB: usize = 4;
        let mut subs = [(Blend::single(tile.region as u32), b); SUB * SUB];
        if border {
            for (k, sub) in subs.iter_mut().enumerate() {
                let (x, y) = (x0 + ((k / SUB) as f64 + 0.5) * TILE / SUB as f64, y0 + ((k % SUB) as f64 + 0.5) * TILE / SUB as f64);
                let bl = self.params.blend(x, y, 0.0);
                *sub = (bl, journey::mix(&bl, bl.dominant()));
            }
        }
        let (mut n, mut nb) = (0usize, 0usize);
        for gi in 0..n_grid {
            for gj in 0..n_grid {
                let (ai, aj) = (tile.i as i64 * n_grid + gi, tile.j as i64 * n_grid + gj);
                let mut r = Stream::new(hash(self.seed, ai, aj, 0x7EE5 ^ self.biome as u64));
                let x = (ai as f64 + r.uniform() as f64) * GRID;
                let y = (aj as f64 + r.uniform() as f64) * GRID;
                let accept = r.uniform();
                let (g, w, s) = self.nearest(x, y);
                let (bt, p_cell, base) = if border {
                    let sub = |v: f64| ((v / (TILE / SUB as f64)).floor() as usize).min(SUB - 1);
                    let (bl, mixed) = &subs[sub(x - x0) * SUB + sub(y - y0)];
                    let mut bt = *mixed;
                    let d = biome(bl.pick(journey::dither(self.seed, x, y)));
                    (bt.name, bt.sand) = (d.name, d.sand);
                    let p = (2.0 * bt.density_ha * (GRID * GRID) as f32 / 10_000.0).min(1.0);
                    (bt, p, self.cover_in(bl, x, y))
                } else {
                    (b, p_cell, -1.0)
                };
                // In the desert, trees only ring the oases.
                let c = if bt.sand > 0.5 {
                    if w > g || s == Surface::Rock { 0.0 } else { 0.5 * smoothstep(9.0, 2.0, self.oasis(x, y)) }
                } else {
                    let cover = if border { base } else { self.base_cover(x, y) };
                    cover * Self::terrain_cover(&bt, g, w, s, 10.0)
                };
                if accept >= c * p_cell || n >= TREES_PER_TILE || carved(self, x, y, 1.5) {
                    continue;
                }
                // Species: biome weights tilted by local groves; willow and alder by water.
                let mut wt = bt.species;
                for (k, v) in wt.iter_mut().enumerate() {
                    *v *= 0.3 + 1.4 * noise2(self.seed, 0x5EC0 + k as u64, x / 90.0, y / 90.0);
                }
                if carved(self, x, y, 14.0) {
                    wt[3] += 0.6;
                    wt[4] += 0.6;
                }
                let sp = pick(&wt, r.uniform());
                let (crown_z, crown_r, trunk_r) = tree_shape(SPECIES[sp], &mut r);
                let rank = r.uniform();
                let id = self.id();
                let ground = self.ground(x, y);
                self.trees[t * TREES_PER_TILE + n] = Tree { id, x, y, ground, species: sp as u8, crown_z, crown_r, trunk_r, rank };
                n += 1;
                // Birds live in the trees.
                if r.uniform() < bt.birds * 2.0 && nb < BIRDS_PER_TILE {
                    let species = match director_biome(&bt) {
                        // Tree birds of the open biomes from the director's table (gulls and
                        // ravens get their own, in the air, below).
                        Some(name) => {
                            let s = crate::director::pick_species_for_biome(name, r.uniform());
                            bird_index(if aerial(s) { Species::Chaffinch } else { s })
                        }
                        None => {
                            let mut bw = bird_weights(SPECIES[sp], c < 0.6);
                            for (k, v) in bw.iter_mut().enumerate() {
                                *v *= species_weight(BIRD_SPECIES[k], &habitat);
                            }
                            pick(&bw, r.uniform()) as u8
                        }
                    };
                    let a = r.range(0.0, core::f32::consts::TAU);
                    let rr = r.range(0.2, 0.8) * crown_r;
                    let z = crown_z + r.range(-0.4, 0.9) * crown_r;
                    let pos = [x + (rr * a.cos()) as f64, y + (rr * a.sin()) as f64, z as f64];
                    let brank = r.uniform();
                    let id = self.id();
                    self.birds[t * BIRDS_PER_TILE + nb] = Bird { id, alive: true, species, rank: brank, pos, ..Default::default() };
                    nb += 1;
                }
            }
        }
        // Birds of the open (the director's biome tables): larks hovering in song flight,
        // gulls and ravens on the wind, the rest low on the ground, dunes, rocks and shores.
        if let Some(name) = director_biome(&b) {
            let mut r = Stream::new(hash(self.seed, tile.i as i64, tile.j as i64, 0xB1D5 ^ self.biome as u64));
            let expect = 1.6 * crate::director::bird_density_for_biome(name);
            let k = expect.floor() as usize + usize::from(r.uniform() < expect.fract());
            for _ in 0..k {
                if nb >= BIRDS_PER_TILE {
                    break;
                }
                let (x, y) = (x0 + r.uniform() as f64 * TILE, y0 + r.uniform() as f64 * TILE);
                let sp = crate::director::pick_species_for_biome(name, r.uniform());
                let (g, w, _) = self.nearest(x, y);
                let z = if aerial(sp) {
                    r.range(12.0, 35.0)
                } else if w > g {
                    continue;
                } else {
                    r.range(0.3, 1.5)
                };
                let brank = r.uniform();
                let id = self.id();
                self.birds[t * BIRDS_PER_TILE + nb] = Bird { id, alive: true, species: bird_index(sp), rank: brank, pos: [x, y, z as f64], ..Default::default() };
                nb += 1;
            }
        }
        self.tiles[t].trees = true;
        self.tiles[t].n_trees = n as u16;
    }

    /// Desert: how far above the nearest water (oases) a point is, m (big when dry).
    fn oasis(&self, x: f64, y: f64) -> f32 {
        let mut best = 50.0f32;
        for f in self.features.iter().filter(|f| f.alive && f.kind == FeatureKind::Lake) {
            let r = f.ellipse_radius(x, y);
            if r < 2.2 {
                best = best.min((r - 1.0).max(0.0) * f.a.min(f.b) * 0.2);
            }
        }
        best
    }
}
