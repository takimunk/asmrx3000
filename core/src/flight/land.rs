//! The landscape of flight mode: a deterministic function of (seed, biome) over the plane,
//! streamed in square tiles around the listener, whichever way it travels, and retired when
//! it is left behind.
//!
//! * **Tiles** (40 m) exist within `KEEP` (240 m, the reach of the propagators' delay lines):
//!   each carries an aggregate cell (crown area of its forest, from the cover field), boulders
//!   and a cricket meadow. Within `TREE_R` (90 m) a tile also holds its individual trees and
//!   the birds living in them.
//! * **Macro cells** (360 m) hold the big features: brooks and rivers, lakes, ridges with
//!   waterfalls, campfires, and their sound sites.
//!
//! Content is a pure function of `(seed, biome, tile / macro index)`: the trajectory, the
//! order and the timing of generation never change what exists where. Storage is toroidal
//! (indexed by tile / macro coordinates modulo the ring size) and sized at build time, so
//! streaming never allocates and the amount of content stays bounded however the listener
//! turns (see [`Land::stream`]).

use crate::director::Species;
use crate::scene::TreeSpecies;

/// Side of a streamed tile, m.
pub const TILE: f64 = 40.0;
/// Tiles exist within this distance (centre) of the listener; retired beyond `KEEP + 40`.
pub const KEEP: f64 = 240.0;
/// Individual trees and birds exist in tiles within this distance; dropped beyond `TREE_R + 30`.
pub const TREE_R: f64 = 90.0;
/// Side of a macro cell (big features), m.
pub const MACRO: f64 = 360.0;
/// Macro cells exist within this distance (centre): a feature reaches ~480 m from its cell's
/// centre, so every feature that can touch a live tile exists before the tile is made.
const MACRO_R: f64 = 800.0;
/// Tile ring side (tiles): covers 2 · (KEEP + 40) with room to spare.
pub const W: usize = 16;
/// Macro ring side.
const MW: usize = 8;
/// Density levels at which the area of a cell is tabulated (user density 0.25..2).
pub const LEVELS: usize = 8;

pub const TREES_PER_TILE: usize = 320;
pub const BIRDS_PER_TILE: usize = 16;
const FEATURES_PER_MACRO: usize = 8;
const SITES_PER_MACRO: usize = 96;
const FEATURES_PER_TILE: usize = 4;
const SITES_PER_TILE: usize = 2;
pub const TILES: usize = W * W;
const MACROS: usize = MW * MW;
pub const MAX_FEATURES: usize = MACROS * FEATURES_PER_MACRO + TILES * FEATURES_PER_TILE;
pub const MAX_SITES: usize = MACROS * SITES_PER_MACRO + TILES * SITES_PER_TILE;
pub const PATH_POINTS: usize = 24;

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

/// A landscape preset: how much forest, which trees, how often water and rock appear.
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
    /// Chance of a lake per 420 m.
    pub lake: f32,
    /// Chance of a rocky ridge per 380 m.
    pub ridge: f32,
    /// Chance of boulders per chunk.
    pub rocks: f32,
    /// Chance that a ridge carries a waterfall.
    pub waterfall: f32,
    /// Chance of a campfire per 600 m.
    pub campfire: f32,
    /// Birds per tree.
    pub birds: f32,
    /// Size of stands and clearings, m.
    pub patch: f32,
}

#[rustfmt::skip]
pub const BIOMES: [Biome; 6] = [
    Biome { name: "mixed",        forest: 0.62, density_ha: 480.0, species: [0.20, 0.15, 0.20, 0.08, 0.10, 0.12, 0.15], river_every: 340.0, lake: 0.35, ridge: 0.25, rocks: 0.30, waterfall: 0.25, campfire: 0.15, birds: 0.030, patch: 170.0 },
    Biome { name: "broadleaf",    forest: 0.68, density_ha: 420.0, species: [0.25, 0.35, 0.00, 0.10, 0.10, 0.20, 0.00], river_every: 400.0, lake: 0.25, ridge: 0.15, rocks: 0.20, waterfall: 0.15, campfire: 0.12, birds: 0.036, patch: 190.0 },
    Biome { name: "taiga",        forest: 0.80, density_ha: 640.0, species: [0.15, 0.00, 0.50, 0.00, 0.00, 0.05, 0.35], river_every: 480.0, lake: 0.45, ridge: 0.30, rocks: 0.50, waterfall: 0.30, campfire: 0.10, birds: 0.022, patch: 220.0 },
    Biome { name: "river_valley", forest: 0.50, density_ha: 380.0, species: [0.20, 0.15, 0.00, 0.30, 0.30, 0.05, 0.00], river_every: 170.0, lake: 0.50, ridge: 0.10, rocks: 0.15, waterfall: 0.10, campfire: 0.15, birds: 0.040, patch: 150.0 },
    Biome { name: "meadows",      forest: 0.28, density_ha: 300.0, species: [0.30, 0.40, 0.00, 0.10, 0.00, 0.20, 0.00], river_every: 450.0, lake: 0.30, ridge: 0.10, rocks: 0.20, waterfall: 0.10, campfire: 0.25, birds: 0.045, patch: 140.0 },
    Biome { name: "highlands",    forest: 0.55, density_ha: 450.0, species: [0.30, 0.00, 0.30, 0.00, 0.00, 0.00, 0.40], river_every: 300.0, lake: 0.30, ridge: 0.70, rocks: 0.80, waterfall: 0.60, campfire: 0.10, birds: 0.025, patch: 160.0 },
];

pub fn biome(index: u32) -> &'static Biome {
    &BIOMES[(index as usize).min(BIOMES.len() - 1)]
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

/// Which birds live in which trees (weights over `BIRD_SPECIES`).
pub const BIRD_SPECIES: [Species; 8] = [
    Species::Chaffinch,
    Species::Robin,
    Species::SongThrush,
    Species::Nightingale,
    Species::WoodPigeon,
    Species::Cuckoo,
    Species::TawnyOwl,
    Species::Swallow,
];

fn bird_weights(sp: TreeSpecies, edge: bool) -> [f32; 8] {
    let mut w = match sp {
        TreeSpecies::Spruce | TreeSpecies::Pine => [0.45, 0.30, 0.10, 0.0, 0.07, 0.0, 0.08, 0.0],
        TreeSpecies::Oak => [0.30, 0.10, 0.20, 0.0, 0.30, 0.10, 0.0, 0.0],
        TreeSpecies::Willow | TreeSpecies::Alder => [0.20, 0.20, 0.20, 0.40, 0.0, 0.0, 0.0, 0.0],
        TreeSpecies::Birch | TreeSpecies::Aspen => [0.35, 0.25, 0.20, 0.10, 0.0, 0.10, 0.0, 0.0],
    };
    if edge {
        w[5] += 0.15;
    }
    w
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
    /// Bounding box in world (x0, y0, x1, y1).
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
    /// River width / rock radius / ridge height, m.
    pub size: f32,
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
        }
    }
}

/// Distance from (x, y) to a polyline.
fn polyline_distance(pts: &[[f64; 2]], x: f64, y: f64) -> f64 {
    let mut best = f64::INFINITY;
    for w in pts.windows(2) {
        let (ax, ay, bx, by) = (w[0][0], w[0][1], w[1][0], w[1][1]);
        let (dx, dy) = (bx - ax, by - ay);
        let l2 = (dx * dx + dy * dy).max(1e-9);
        let t = (((x - ax) * dx + (y - ay) * dy) / l2).clamp(0.0, 1.0);
        let (px, py) = (ax + t * dx - x, ay + t * dy - y);
        best = best.min((px * px + py * py).sqrt());
    }
    best
}

impl Feature {
    fn near_bbox(&self, x: f64, y: f64, margin: f64) -> bool {
        x >= self.bbox[0] - margin && x <= self.bbox[2] + margin && y >= self.bbox[1] - margin && y <= self.bbox[3] + margin
    }

    /// No tree at (x, y) (with `margin`): water, rock, a camp.
    pub fn carves(&self, x: f64, y: f64, margin: f32) -> bool {
        if !self.alive || !self.near_bbox(x, y, margin as f64 + 1.0) {
            return false;
        }
        let m = margin as f64;
        match self.kind {
            FeatureKind::River | FeatureKind::Ridge => polyline_distance(&self.path[..self.path_len as usize], x, y) < self.a as f64 + m,
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
}

/// A point that can sound: a river segment, a lake shore, a waterfall, crickets, a campfire.
#[derive(Clone, Copy, Debug, Default)]
pub struct Site {
    pub id: u32,
    pub alive: bool,
    pub kind: SiteKind,
    pub x: f64,
    pub y: f64,
    pub z: f32,
    /// Preset index in the model's PRESETS list.
    pub preset: u8,
    /// Loudness weight for the level-of-detail choice.
    pub weight: f32,
    /// Segment length (water) / patch size, m.
    pub length: f32,
    pub variation: [f32; 3],
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

/// The streamed world: tiles (aggregate cells, trees, birds, boulders, meadows) and macro
/// cells (rivers, lakes, ridges, falls, camps), each with their sound sites.
pub struct Land {
    pub seed: u64,
    pub biome: u32,
    pub tiles: Box<[Tile]>,
    pub trees: Box<[Tree]>,
    pub birds: Box<[Bird]>,
    macros: Box<[Macro]>,
    pub features: Box<[Feature]>,
    pub sites: Box<[Site]>,
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

impl Land {
    /// Allocates all storage. Not real-time safe.
    pub fn new(seed: u64, biome: u32) -> Self {
        Self {
            seed,
            biome,
            tiles: vec![Tile::default(); TILES].into_boxed_slice(),
            trees: vec![Tree::default(); TILES * TREES_PER_TILE].into_boxed_slice(),
            birds: vec![Bird::default(); TILES * BIRDS_PER_TILE].into_boxed_slice(),
            macros: vec![Macro::default(); MACROS].into_boxed_slice(),
            features: vec![Feature::default(); MAX_FEATURES].into_boxed_slice(),
            sites: vec![Site::default(); MAX_SITES].into_boxed_slice(),
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

    /// Clears the content (ids keep counting) and switches seed / biome. Real-time safe.
    pub fn reset(&mut self, seed: u64, biome: u32) {
        self.seed = seed;
        self.biome = biome;
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

    /// Forest cover in [0, 1] at a world point, before carving.
    pub fn base_cover(&self, x: f64, y: f64) -> f32 {
        let b = self.biome_def();
        let n = fbm(self.seed, 0xC0FE, x / b.patch as f64, y / b.patch as f64);
        let t = 0.5 + (0.5 - b.forest) * 0.45;
        smoothstep(t - 0.08, t + 0.08, n)
    }

    /// Forest cover with water, rock and camps carved out.
    pub fn cover(&self, x: f64, y: f64) -> f32 {
        if self.features.iter().any(|f| f.carves(x, y, 1.5)) {
            return 0.0;
        }
        self.base_cover(x, y)
    }

    /// `cover` and `ground_at` using only the features in `near` (from `local_features`).
    pub fn cover_near(&self, x: f64, y: f64, near: &[usize]) -> f32 {
        if near.iter().any(|&k| self.features[k].carves(x, y, 1.5)) {
            return 0.0;
        }
        self.base_cover(x, y)
    }

    pub fn ground_near(&self, x: f64, y: f64, near: &[usize]) -> u32 {
        if near.iter().any(|&k| self.features[k].covers(x, y)) {
            2
        } else if self.base_cover(x, y) > 0.5 {
            0
        } else {
            1
        }
    }

    /// What lies on the ground at (x, y): 0 forest floor / 1 grass / 2 water.
    pub fn ground_at(&self, x: f64, y: f64) -> u32 {
        if self.features.iter().any(|f| f.alive && f.covers(x, y)) {
            2
        } else if self.base_cover(x, y) > 0.5 {
            0
        } else {
            1
        }
    }

    /// Streams content around (x, y): macro cells, tiles within KEEP, trees within TREE_R;
    /// retires what is too far. Generates at most `budget` tiles and `budget` tree sets per
    /// call (amortised on the audio thread); never allocates.
    pub fn stream(&mut self, x: f64, y: f64, budget: usize) {
        // Macro cells first: tiles carve their forest around the features.
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
        for a in -k..=k {
            for b in -k..=k {
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
        let b = *self.biome_def();
        let mut r = Stream::new(hash(self.seed, i as i64, j as i64, 0x3AC0 ^ self.biome as u64));
        let (x0, y0) = (i as f64 * MACRO, j as f64 * MACRO);
        let f0 = m * FEATURES_PER_MACRO;
        let s0 = m * SITES_PER_MACRO;
        let at = |r: &mut Stream| (x0 + r.uniform() as f64 * MACRO, y0 + r.uniform() as f64 * MACRO);

        // Brooks and rivers: on average one crossing per `river_every` m of travel.
        let q = 0.9 * MACRO as f32 / b.river_every;
        let n_rivers = q.floor() as usize + usize::from(r.uniform() < q.fract());
        for _ in 0..n_rivers.min(2) {
            let (cx, cy) = at(&mut r);
            let (preset, width) = match pick(&[0.5, 0.25, 0.25], r.uniform()) {
                0 => (3u8, r.range(1.2, 2.8)),
                1 => (4u8, r.range(2.0, 4.0)),
                _ => (2u8, r.range(7.0, 14.0)),
            };
            let th = r.range(0.0, core::f32::consts::TAU) as f64;
            let (dx, dy) = (th.cos(), th.sin());
            let (amp, wl, ph) = (r.range(3.0, 12.0) as f64, r.range(60.0, 130.0) as f64, r.range(0.0, core::f32::consts::TAU) as f64);
            let half = 215.0;
            let mut f = Feature { kind: FeatureKind::River, x: cx, y: cy, z: 0.1, a: 0.5 * width, preset, size: width, ..Default::default() };
            for k in 0..PATH_POINTS {
                let t = -half + 2.0 * half * k as f64 / (PATH_POINTS - 1) as f64;
                let off = amp * (core::f64::consts::TAU * t / wl + ph).sin();
                f.path[k] = [cx + t * dx - off * dy, cy + t * dy + off * dx];
            }
            f.path_len = PATH_POINTS as u8;
            if self.put_feature(f0, FEATURES_PER_MACRO, f, 4.0).is_none() {
                break;
            }
            // Sound sites every 15 m along the bed, riffles and pools alternating.
            let weight = if preset == 2 { 1.6 } else { 1.0 };
            let mut t = -half + 7.5;
            while t < half {
                let off = amp * (core::f64::consts::TAU * t / wl + ph).sin();
                let sv = (core::f64::consts::TAU * t / 45.0 + ph).sin() as f32 + 0.4 * (r.uniform() - 0.5);
                let variation = [(0.2 * sv).exp(), (-0.15 * sv).exp(), -0.05 * sv];
                let site = Site { kind: SiteKind::Water, x: cx + t * dx - off * dy, y: cy + t * dy + off * dx, z: 0.1, preset, weight, length: 15.0, variation, ..Default::default() };
                self.put_site(s0, SITES_PER_MACRO, site);
                t += 15.0;
            }
        }

        // A lake.
        if r.uniform() < 0.8 * b.lake {
            let (cx, cy) = at(&mut r);
            let (a, bb, rot) = (r.range(20.0, 45.0), r.range(15.0, 35.0), r.range(0.0, 3.1416));
            let mut f = Feature { kind: FeatureKind::Lake, x: cx, y: cy, z: 0.05, a, b: bb, rot, preset: 1, size: a.max(bb), ..Default::default() };
            let (s, c) = (rot as f64).sin_cos();
            let ell = |t: f64, shrink: f64| {
                let (u, v) = ((a as f64 - shrink) * t.cos(), (bb as f64 - shrink) * t.sin());
                [cx + c * u - s * v, cy + s * u + c * v]
            };
            for k in 0..PATH_POINTS {
                f.path[k] = ell(core::f64::consts::TAU * k as f64 / PATH_POINTS as f64, 0.0);
            }
            f.path_len = PATH_POINTS as u8;
            if self.put_feature(f0, FEATURES_PER_MACRO, f, 2.0).is_some() {
                let n = ((core::f32::consts::TAU * 0.5 * (a + bb)) / 16.0).ceil().clamp(6.0, 16.0) as usize;
                for k in 0..n {
                    let p = ell(core::f64::consts::TAU * (k as f64 + 0.5) / n as f64, 1.5);
                    let site = Site { kind: SiteKind::Water, x: p[0], y: p[1], z: 0.05, preset: 1, weight: 0.7, length: 16.0, variation: [1.0, 1.0, 0.0], ..Default::default() };
                    self.put_site(s0, SITES_PER_MACRO, site);
                }
            }
        }

        // A rocky ridge, maybe with a waterfall down its face.
        if r.uniform() < 0.8 * b.ridge {
            let (cx, cy) = at(&mut r);
            let half = r.range(30.0, 80.0) as f64;
            let th = r.range(0.0, 3.1416) as f64;
            let (dx, dy) = (th.cos(), th.sin());
            let bend = r.range(-6.0, 6.0) as f64;
            let height = r.range(4.0, 12.0);
            let mut f = Feature { kind: FeatureKind::Ridge, x: cx, y: cy, z: height, a: 2.5, size: height, ..Default::default() };
            f.path[0] = [cx - half * dx - bend * dy, cy - half * dy + bend * dx];
            f.path[1] = [cx, cy];
            f.path[2] = [cx + half * dx - bend * dy, cy + half * dy + bend * dx];
            f.path_len = 3;
            if self.put_feature(f0, FEATURES_PER_MACRO, f, 4.0).is_some() && r.uniform() < b.waterfall {
                let t = r.range(-0.5, 0.5) as f64 * half;
                let side = r.sign() as f64;
                let (wx, wy) = (cx + t * dx - side * 3.5 * dy, cy + t * dy + side * 3.5 * dx);
                let fall = Feature { kind: FeatureKind::Waterfall, x: wx, y: wy, z: 0.6 * height, preset: 6, size: height, ..Default::default() };
                if self.put_feature(f0, FEATURES_PER_MACRO, fall, 3.0).is_some() {
                    let site = Site { kind: SiteKind::Water, x: wx, y: wy, z: 0.5 * height, preset: 6, weight: 2.5, length: 4.0, variation: [1.0, 1.0, 0.0], ..Default::default() };
                    self.put_site(s0, SITES_PER_MACRO, site);
                    let (px, py) = (wx - side * 3.0 * dy, wy + side * 3.0 * dx);
                    let pool = Site { kind: SiteKind::Water, x: px, y: py, z: 0.2, preset: 4, weight: 1.2, length: 6.0, variation: [1.0, 1.0, 0.0], ..Default::default() };
                    self.put_site(s0, SITES_PER_MACRO, pool);
                }
            }
        }

        // A campfire in the most open of a few spots.
        if r.uniform() < 0.6 * b.campfire {
            let mut best = (0.0, 0.0, 2.0f32);
            for _ in 0..8 {
                let (px, py) = at(&mut r);
                let c = self.cover(px, py);
                if c < best.2 {
                    best = (px, py, c);
                }
            }
            let f = Feature { kind: FeatureKind::Campfire, x: best.0, y: best.1, z: 0.3, ..Default::default() };
            if self.put_feature(f0, FEATURES_PER_MACRO, f, 6.0).is_some() {
                let site = Site { kind: SiteKind::Fire, x: best.0, y: best.1, z: 0.3, preset: 0, weight: 1.0, length: 1.0, variation: [1.0, 1.0, 0.0], ..Default::default() };
                self.put_site(s0, SITES_PER_MACRO, site);
            }
        }
    }

    // -- tiles ----------------------------------------------------------------------------

    /// Indices of live features whose box touches the square around (cx, cy).
    pub fn local_features(&self, cx: f64, cy: f64, half: f64, out: &mut [usize; 32]) -> usize {
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
        let b = *self.biome_def();
        let (x0, y0) = (i as f64 * TILE, j as f64 * TILE);
        let mut r = Stream::new(hash(self.seed, i as i64, j as i64, 0x711E ^ self.biome as u64));
        self.tiles[t] = Tile { i, j, alive: true, cell: Cell::default(), trees: false, n_trees: 0 };
        let f0 = MACROS * FEATURES_PER_MACRO + t * FEATURES_PER_TILE;
        let s0 = MACROS * SITES_PER_MACRO + t * SITES_PER_TILE;
        let mut local = [0usize; 32];
        let nl = self.local_features(x0 + 0.5 * TILE, y0 + 0.5 * TILE, 0.5 * TILE + 20.0, &mut local);
        let carved = |land: &Land, x: f64, y: f64, m: f32| local[..nl].iter().any(|&k| land.features[k].carves(x, y, m));

        // Boulders in rocky country.
        if r.uniform() < b.rocks * 0.3 {
            let n = 1 + (r.uniform() * 3.0) as usize;
            for _ in 0..n {
                let (x, y) = (x0 + r.uniform() as f64 * TILE, y0 + r.uniform() as f64 * TILE);
                let radius = r.range(0.5, 1.8);
                if carved(self, x, y, 1.0) {
                    continue;
                }
                let f = Feature { kind: FeatureKind::Rock, x, y, z: radius * 1.4, a: radius, size: radius, ..Default::default() };
                let _ = self.put_feature(f0, FEATURES_PER_TILE - 1, f, radius as f64 + 1.0);
            }
        }

        // A cricket or grasshopper meadow in the open.
        for _ in 0..4 {
            let (x, y) = (x0 + r.uniform() as f64 * TILE, y0 + r.uniform() as f64 * TILE);
            let preset = if r.uniform() < 0.6 { 0u8 } else { 3u8 };
            if self.base_cover(x, y) > 0.12 || carved(self, x, y, 2.0) {
                continue;
            }
            let f = Feature { kind: FeatureKind::Meadow, x, y, z: 0.1, a: 8.0, preset, size: 8.0, ..Default::default() };
            if self.put_feature(f0 + FEATURES_PER_TILE - 1, 1, f, 8.0).is_some() {
                let site = Site { kind: SiteKind::Insects, x, y, z: 0.1, preset, weight: 1.0, length: 8.0, variation: [1.0, 1.0, 0.0], ..Default::default() };
                self.put_site(s0, SITES_PER_TILE, site);
            }
            break;
        }

        // The aggregate cell: crown area from the cover field on a 4 x 4 grid.
        let mut cov = 0.0;
        for a in 0..4 {
            for c in 0..4 {
                let (x, y) = (x0 + (a as f64 + 0.5) * TILE / 4.0, y0 + (c as f64 + 0.5) * TILE / 4.0);
                cov += if carved(self, x, y, 1.5) { 0.0 } else { self.base_cover(x, y) };
            }
        }
        let area1 = cov / 16.0 * (TILE * TILE) as f32 * 0.65;
        let mut cell = Cell { x: x0 + 0.5 * TILE, y: y0 + 0.5 * TILE, canopy_z: 8.0, ..Default::default() };
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
        let b = *self.biome_def();
        let (x0, y0) = (tile.i as f64 * TILE, tile.j as f64 * TILE);
        let mut local = [0usize; 32];
        let nl = self.local_features(x0 + 0.5 * TILE, y0 + 0.5 * TILE, 0.5 * TILE + 20.0, &mut local);
        let carved = |land: &Land, x: f64, y: f64, m: f32| local[..nl].iter().any(|&k| land.features[k].carves(x, y, m));
        let p_cell = (2.0 * b.density_ha * (GRID * GRID) as f32 / 10_000.0).min(1.0);
        let n_grid = (TILE / GRID).round() as i64;
        let (mut n, mut nb) = (0usize, 0usize);
        for gi in 0..n_grid {
            for gj in 0..n_grid {
                let (ai, aj) = (tile.i as i64 * n_grid + gi, tile.j as i64 * n_grid + gj);
                let mut r = Stream::new(hash(self.seed, ai, aj, 0x7EE5 ^ self.biome as u64));
                let x = (ai as f64 + r.uniform() as f64) * GRID;
                let y = (aj as f64 + r.uniform() as f64) * GRID;
                let accept = r.uniform();
                let c = self.base_cover(x, y);
                if accept >= c * p_cell || n >= TREES_PER_TILE || carved(self, x, y, 1.5) {
                    continue;
                }
                // Species: biome weights tilted by local groves; willow and alder by water.
                let mut w = b.species;
                for (k, v) in w.iter_mut().enumerate() {
                    *v *= 0.3 + 1.4 * noise2(self.seed, 0x5EC0 + k as u64, x / 90.0, y / 90.0);
                }
                if carved(self, x, y, 14.0) {
                    w[3] += 0.6;
                    w[4] += 0.6;
                }
                let sp = pick(&w, r.uniform());
                let (crown_z, crown_r, trunk_r) = tree_shape(SPECIES[sp], &mut r);
                let rank = r.uniform();
                let id = self.id();
                self.trees[t * TREES_PER_TILE + n] = Tree { id, x, y, species: sp as u8, crown_z, crown_r, trunk_r, rank };
                n += 1;
                // Birds live in the trees.
                if r.uniform() < b.birds * 2.0 && nb < BIRDS_PER_TILE {
                    let bw = bird_weights(SPECIES[sp], c < 0.6);
                    let species = pick(&bw, r.uniform()) as u8;
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
        self.tiles[t].trees = true;
        self.tiles[t].n_trees = n as u16;
    }
}
