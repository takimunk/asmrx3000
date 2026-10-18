//! The landscape of flight mode: a deterministic function of (seed, biome), streamed in
//! chunks along the flight path and retired behind it.
//!
//! Everything is generated in the *path frame* (s = arc length along the flight path,
//! u = lateral offset, + to the left) and mapped once, at generation, into a flat world frame
//! (x, y) through the path's centre line. Objects are rigid in that world frame; the listener
//! moves along the path, so the propagation sees true relative motion (Doppler, parallax).
//!
//! Content is a pure function of `(seed, biome, chunk / bin index)`: the order and timing of
//! generation never change what exists. Storage is fixed-capacity rings sized at build time,
//! so streaming never allocates (see [`Land::stream`]).

use crate::director::Species;
use crate::scene::TreeSpecies;

/// Length of one streamed chunk along the path, m.
pub const CHUNK: f64 = 32.0;
/// Content exists this far ahead of the listener (the propagators carry ~235 m of delay).
pub const AHEAD: f64 = 210.0;
/// ... and is retired this far behind.
pub const BEHIND: f64 = 70.0;
/// Individual trees are generated within |u| < TREE_U; beyond it the forest exists only as
/// aggregate cells.
pub const TREE_U: f32 = 60.0;
/// Lateral bands of the aggregate cells (edges, m).
pub const BAND_EDGES: [f32; 8] = [-140.0, -100.0, -60.0, -20.0, 20.0, 60.0, 100.0, 140.0];
pub const BANDS: usize = 7;
/// Density levels at which the area of an inner cell is tabulated (user density 0.25..2).
pub const LEVELS: usize = 8;
/// Half-width of the tree-free corridor along the flight path, m.
pub const CORRIDOR: f32 = 3.5;

pub const MAX_TREES: usize = 6144;
pub const MAX_CHUNKS: usize = 16;
pub const MAX_FEATURES: usize = 64;
pub const MAX_SITES: usize = 256;
pub const MAX_BIRDS: usize = 256;
pub const PATH_POINTS: usize = 24;

/// Tree grid spacing, m (jittered grid: at most one tree per cell).
const GRID: f64 = 2.5;
/// Path table spacing and capacity (ring).
const PATH_STEP: f64 = 2.0;
const PATH_CAP: usize = 512;
/// Features are generated this far beyond the chunk horizon (their extent across the path).
const FEATURE_AHEAD: f64 = 170.0;
/// Turning radius at curviness 1, m.
const MIN_RADIUS: f64 = 250.0;

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
// Flight path
// ---------------------------------------------------------------------------------------

#[derive(Clone, Copy, Default, Debug)]
pub struct PathPt {
    pub x: f64,
    pub y: f64,
    pub heading: f64,
}

/// The centre line of the flight, sampled every `PATH_STEP` m into a ring. Curvature is a
/// smooth function of s scaled by the curviness at the time the path is laid down (ahead of
/// the listener), so changing curviness never kinks the path.
pub struct Path {
    pts: Box<[PathPt]>,
    /// Absolute index of the newest sample (s = index * PATH_STEP).
    last: u64,
    phases: [f64; 3],
}

impl Path {
    pub fn new(seed: u64) -> Self {
        let mut pts = vec![PathPt::default(); PATH_CAP].into_boxed_slice();
        pts[0] = PathPt::default();
        let mut r = Stream::new(hash(seed, 0, 0, 0x9A7));
        let phases = [r.range(0.0, 6.283) as f64, r.range(0.0, 6.283) as f64, r.range(0.0, 6.283) as f64];
        Self { pts, last: 0, phases }
    }

    /// Curvature (1/m) at s for curviness 1.
    fn kappa(&self, s: f64) -> f64 {
        let p = &self.phases;
        let g = 0.55 * (core::f64::consts::TAU * s / 530.0 + p[0]).sin()
            + 0.30 * (core::f64::consts::TAU * s / 310.0 + p[1]).sin()
            + 0.15 * (core::f64::consts::TAU * s / 170.0 + p[2]).sin();
        g / MIN_RADIUS
    }

    /// Lays the path down to at least `s` with the given curviness (0..1).
    pub fn extend(&mut self, s: f64, curviness: f32) {
        let curv = curviness.clamp(0.0, 1.0) as f64;
        while (self.last as f64) * PATH_STEP < s {
            let a = self.pts[(self.last as usize) % PATH_CAP];
            let s0 = self.last as f64 * PATH_STEP;
            let dth = curv * self.kappa(s0 + 0.5 * PATH_STEP) * PATH_STEP;
            let mid = a.heading + 0.5 * dth;
            let b = PathPt { x: a.x + PATH_STEP * mid.cos(), y: a.y + PATH_STEP * mid.sin(), heading: a.heading + dth };
            self.last += 1;
            self.pts[(self.last as usize) % PATH_CAP] = b;
        }
    }

    /// Centre-line point at s (clamped to the stored window).
    pub fn at(&self, s: f64) -> PathPt {
        let first = self.last.saturating_sub(PATH_CAP as u64 - 1);
        let f = (s / PATH_STEP).clamp(first as f64, self.last as f64);
        let i = (f.floor() as u64).min(self.last.saturating_sub(1)).max(first);
        let t = (f - i as f64).clamp(0.0, 1.0);
        let a = self.pts[(i as usize) % PATH_CAP];
        if i == self.last {
            return a;
        }
        let b = self.pts[((i + 1) as usize) % PATH_CAP];
        let mut p = PathPt { x: a.x + (b.x - a.x) * t, y: a.y + (b.y - a.y) * t, heading: a.heading + (b.heading - a.heading) * t };
        // Beyond the newest sample: continue straight.
        let over = s - self.last as f64 * PATH_STEP;
        if over > 0.0 {
            p.x += over * p.heading.cos();
            p.y += over * p.heading.sin();
        }
        p
    }

    /// World (x, y) of the path-frame point (s, u).
    pub fn world(&self, s: f64, u: f32) -> [f64; 2] {
        let p = self.at(s);
        [p.x - u as f64 * p.heading.sin(), p.y + u as f64 * p.heading.cos()]
    }

    /// Curvature of the laid-down path at s (1/m).
    pub fn curvature(&self, s: f64) -> f64 {
        let a = self.at(s - 1.0);
        let b = self.at(s + 1.0);
        (b.heading - a.heading) / 2.0
    }
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
    pub band: u8,
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

/// Aggregate of one lateral band of one chunk: the forest beyond the individually voiced trees.
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
pub struct Chunk {
    pub k: i64,
    /// Absolute tree indices [start, end).
    pub trees: (u64, u64),
    pub cells: [Cell; BANDS],
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

/// Path-frame description used to carve the forest and to place sites.
#[derive(Clone, Copy, Debug, Default)]
pub struct Shape {
    /// Anchor s / u.
    pub s: f64,
    pub u: f32,
    /// River: tan of the crossing angle, meander amplitude / wavelength / phase, half width.
    /// Lake: semi-axes a (along s), b (along u). Ridge: half length (a), thickness (b).
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub e: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct Feature {
    pub id: u32,
    pub kind: FeatureKind,
    pub alive: bool,
    /// Representative world point (river: crossing of the path; lake: centre; ridge: middle).
    pub x: f64,
    pub y: f64,
    /// Height of its top (ridge, rock) or of the sound above the ground, m.
    pub z: f32,
    pub s_min: f64,
    pub s_max: f64,
    pub shape: Shape,
    /// Polyline (river, ridge) or outline (lake) in world (x, y).
    pub path: [[f64; 2]; PATH_POINTS],
    pub path_len: u8,
    /// Water preset index (`sources::Water::PRESETS`) / insect preset for meadows.
    pub preset: u8,
    /// River width / rock radius, m.
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
            s_min: 0.0,
            s_max: 0.0,
            shape: Shape::default(),
            path: [[0.0; 2]; PATH_POINTS],
            path_len: 0,
            preset: 0,
            size: 0.0,
        }
    }
}

impl Feature {
    /// How much this feature clears trees at (s, u): 1 = no tree here.
    fn carves(&self, s: f64, u: f32, margin: f32) -> bool {
        let sh = &self.shape;
        match self.kind {
            FeatureKind::River => {
                let centre = sh.s + (u * sh.a) as f64 + (sh.b * (core::f32::consts::TAU * u / sh.c + sh.d).sin()) as f64;
                let cos = 1.0 / (1.0 + sh.a * sh.a).sqrt();
                ((s - centre) as f32).abs() * cos < sh.e + margin
            }
            FeatureKind::Lake => {
                let ds = (s - sh.s) as f32 / (sh.a + margin);
                let du = (u - sh.u) / (sh.b + margin);
                ds * ds + du * du < 1.0
            }
            FeatureKind::Ridge => ((s - sh.s) as f32).abs() < sh.a && (u - sh.u).abs() < sh.b + margin,
            FeatureKind::Rock => {
                let ds = (s - sh.s) as f32;
                let du = u - sh.u;
                ds * ds + du * du < (sh.a + margin) * (sh.a + margin)
            }
            FeatureKind::Campfire => {
                let ds = (s - sh.s) as f32;
                let du = u - sh.u;
                ds * ds + du * du < 36.0
            }
            FeatureKind::Waterfall | FeatureKind::Meadow => false,
        }
    }

    /// Lies on the ground at (s, u): water or bare rock (affects the ground type).
    pub fn covers(&self, s: f64, u: f32) -> bool {
        matches!(self.kind, FeatureKind::River | FeatureKind::Lake) && self.carves(s, u, 0.0)
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
    pub feature: u32,
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
    /// Chunk it lives in (retired with it).
    pub k: i64,
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

/// The streamed world: path, chunks of trees with aggregate cells, features, sites, birds.
pub struct Land {
    pub seed: u64,
    pub biome: u32,
    pub path: Path,
    pub trees: Box<[Tree]>,
    pub tree_head: u64,
    pub tree_tail: u64,
    pub chunks: Box<[Chunk]>,
    /// Chunks [chunk_head, chunk_tail) exist.
    pub chunk_head: i64,
    pub chunk_tail: i64,
    pub features: Box<[Feature]>,
    pub sites: Box<[Site]>,
    pub birds: Box<[Bird]>,
    /// Next bin per feature family (river, lake, ridge, campfire).
    next_bin: [i64; 4],
    next_id: u32,
}

/// Bin lengths of the feature families.
const LAKE_BIN: f64 = 420.0;
const RIDGE_BIN: f64 = 380.0;
const CAMP_BIN: f64 = 600.0;

/// Object ids of the streamed world start here (static scene ids are small).
pub const ID_BASE: u32 = 0x4000_0000;

impl Land {
    /// Allocates all storage. Not real-time safe.
    pub fn new(seed: u64, biome: u32) -> Self {
        Self {
            seed,
            biome,
            path: Path::new(seed),
            trees: vec![Tree::default(); MAX_TREES].into_boxed_slice(),
            tree_head: 0,
            tree_tail: 0,
            chunks: vec![Chunk::default(); MAX_CHUNKS].into_boxed_slice(),
            chunk_head: 0,
            chunk_tail: 0,
            features: vec![Feature::default(); MAX_FEATURES].into_boxed_slice(),
            sites: vec![Site::default(); MAX_SITES].into_boxed_slice(),
            birds: vec![Bird::default(); MAX_BIRDS].into_boxed_slice(),
            next_bin: [i64::MIN; 4],
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

    /// Clears the content (path and ids are kept) and switches seed / biome. Real-time safe.
    pub fn reset(&mut self, seed: u64, biome: u32, s: f64) {
        self.seed = seed;
        self.biome = biome;
        self.tree_head = self.tree_tail;
        let k = (((s - BEHIND) / CHUNK).floor()) as i64;
        self.chunk_head = k;
        self.chunk_tail = k;
        for f in self.features.iter_mut() {
            f.alive = false;
        }
        for x in self.sites.iter_mut() {
            x.alive = false;
        }
        for b in self.birds.iter_mut() {
            b.alive = false;
        }
        self.next_bin = [i64::MIN; 4];
    }

    pub fn tree(&self, abs: u64) -> &Tree {
        &self.trees[(abs as usize) % MAX_TREES]
    }

    pub fn chunk(&self, k: i64) -> &Chunk {
        &self.chunks[k.rem_euclid(MAX_CHUNKS as i64) as usize]
    }

    /// Forest cover in [0, 1] at a path-frame point, before carving.
    pub fn base_cover(&self, s: f64, u: f32) -> f32 {
        let b = self.biome_def();
        let n = fbm(self.seed, 0xC0FE, s / b.patch as f64, u as f64 / b.patch as f64);
        let t = 0.5 + (0.5 - b.forest) * 0.45;
        smoothstep(t - 0.08, t + 0.08, n)
    }

    /// Forest cover with water, rock and camps carved out.
    pub fn cover(&self, s: f64, u: f32) -> f32 {
        if self.carved(s, u, 1.5) {
            return 0.0;
        }
        self.base_cover(s, u)
    }

    fn carved(&self, s: f64, u: f32, margin: f32) -> bool {
        self.features.iter().any(|f| f.alive && s >= f.s_min - 10.0 && s <= f.s_max + 10.0 && f.carves(s, u, margin))
    }

    /// What lies on the ground at (s, u): 0 forest floor / 1 grass / 2 water.
    pub fn ground_at(&self, s: f64, u: f32) -> u32 {
        if self.features.iter().any(|f| f.alive && s >= f.s_min && s <= f.s_max && f.covers(s, u)) {
            2
        } else if self.base_cover(s, u) > 0.5 {
            0
        } else {
            1
        }
    }

    /// Streams content so that everything within [s - BEHIND, s + AHEAD] exists. Generates
    /// at most `budget` chunks per call (amortised on the audio thread); never allocates.
    pub fn stream(&mut self, s: f64, curviness: f32, budget: usize) {
        self.path.extend(s + AHEAD + FEATURE_AHEAD + 160.0, curviness);
        self.retire(s);
        self.features_until(s + AHEAD + FEATURE_AHEAD);
        let mut n = 0;
        while (self.chunk_tail as f64) * CHUNK < s + AHEAD && n < budget {
            if self.chunk_tail < self.chunk_head {
                self.chunk_tail = self.chunk_head;
            }
            self.generate_chunk(self.chunk_tail);
            self.chunk_tail += 1;
            n += 1;
        }
    }

    fn retire(&mut self, s: f64) {
        while self.chunk_head < self.chunk_tail && ((self.chunk_head + 1) as f64) * CHUNK < s - BEHIND {
            let c = *self.chunk(self.chunk_head);
            self.tree_head = self.tree_head.max(c.trees.1);
            let k = self.chunk_head;
            for b in self.birds.iter_mut() {
                if b.alive && b.k <= k {
                    b.alive = false;
                }
            }
            self.chunk_head += 1;
        }
        if self.chunk_head == self.chunk_tail {
            // Nothing generated yet (or everything retired): start at the window.
            let k = ((s - BEHIND) / CHUNK).floor() as i64;
            if k > self.chunk_head {
                self.chunk_head = k;
                self.chunk_tail = k;
            }
        }
        let mut gone = false;
        for f in self.features.iter_mut() {
            if f.alive && f.s_max < s - BEHIND - 20.0 {
                f.alive = false;
                gone = true;
            }
        }
        if gone {
            for x in self.sites.iter_mut() {
                if x.alive && !self.features.iter().any(|f| f.alive && f.id == x.feature) {
                    x.alive = false;
                }
            }
        }
    }

    fn add_feature(&mut self, mut f: Feature) -> Option<u32> {
        let slot = self.features.iter().position(|x| !x.alive)?;
        f.id = self.id();
        f.alive = true;
        self.features[slot] = f;
        Some(f.id)
    }

    fn add_site(&mut self, mut x: Site) {
        if let Some(slot) = self.sites.iter().position(|q| !q.alive) {
            x.id = self.id();
            x.alive = true;
            self.sites[slot] = x;
        }
    }

    fn map_polyline(&self, f: &mut Feature, pts: &[(f64, f32)]) {
        let n = pts.len().min(PATH_POINTS);
        for (i, &(s, u)) in pts.iter().take(n).enumerate() {
            f.path[i] = self.path.world(s, u);
        }
        f.path_len = n as u8;
    }

    /// Generates feature bins whose content can reach up to `until`.
    fn features_until(&mut self, until: f64) {
        let b = *self.biome_def();
        let lo = |bin_len: f64| ((until - 2.0 * AHEAD - FEATURE_AHEAD) / bin_len).floor() as i64;
        let families: [(f64, usize); 4] = [(b.river_every as f64, 0), (LAKE_BIN, 1), (RIDGE_BIN, 2), (CAMP_BIN, 3)];
        for (len, fam) in families {
            if self.next_bin[fam] == i64::MIN {
                self.next_bin[fam] = lo(len).max(-1);
            }
            while (self.next_bin[fam] as f64) * len < until {
                let bin = self.next_bin[fam];
                match fam {
                    0 => self.river(bin, len),
                    1 => self.lake(bin),
                    2 => self.ridge(bin),
                    _ => self.campfire(bin),
                }
                self.next_bin[fam] += 1;
            }
        }
    }

    fn river(&mut self, bin: i64, len: f64) {
        let mut r = Stream::new(hash(self.seed, bin, self.biome as i64, 0x7171));
        if bin < 0 || r.uniform() > 0.85 {
            return;
        }
        let s = (bin as f64 + 0.2 + 0.6 * r.uniform() as f64) * len;
        // Kind: brook over pebbles, rocky brook, slow river.
        let (preset, width) = match pick(&[0.5, 0.25, 0.25], r.uniform()) {
            0 => (3u8, r.range(1.2, 2.8)),
            1 => (4u8, r.range(2.0, 4.0)),
            _ => (2u8, r.range(7.0, 14.0)),
        };
        let tan = r.range(-0.75, 0.75);
        let amp = r.range(3.0, 12.0);
        let wavelength = r.range(60.0, 130.0);
        let phase = r.range(0.0, 6.283);
        let shape = Shape { s, u: 0.0, a: tan, b: amp, c: wavelength, d: phase, e: 0.5 * width };
        let centre = |u: f32| s + (u * tan) as f64 + (amp * (core::f32::consts::TAU * u / wavelength + phase).sin()) as f64;
        let mut pts = [(0.0f64, 0.0f32); PATH_POINTS];
        for (i, p) in pts.iter_mut().enumerate() {
            let u = -150.0 + 300.0 * i as f32 / (PATH_POINTS - 1) as f32;
            *p = (centre(u), u);
        }
        let ext = (150.0 * tan.abs() + amp) as f64 + 10.0;
        let mut f = Feature { kind: FeatureKind::River, z: 0.1, s_min: s - ext, s_max: s + ext, shape, preset, size: width, ..Default::default() };
        let w = self.path.world(centre(0.0), 0.0);
        (f.x, f.y) = (w[0], w[1]);
        self.map_polyline(&mut f, &pts);
        let Some(id) = self.add_feature(f) else { return };
        // Sound sites every 10 m along the bed within reach of the path.
        let step = 10.0f32;
        let mut u = -100.0f32;
        let weight = if preset == 2 { 1.6 } else { 1.0 };
        let mut k = 0;
        while u <= 100.0 {
            let w = self.path.world(centre(u), u);
            let sv = (core::f32::consts::TAU * u / 45.0 + phase).sin() + 0.4 * (r.uniform() - 0.5);
            let variation = [(0.2 * sv).exp(), (-0.15 * sv).exp(), -0.05 * sv];
            self.add_site(Site { kind: SiteKind::Water, feature: id, x: w[0], y: w[1], z: 0.1, preset, weight, length: step, variation, ..Default::default() });
            u += step;
            k += 1;
        }
        let _ = k;
    }

    fn lake(&mut self, bin: i64) {
        let mut r = Stream::new(hash(self.seed, bin, self.biome as i64, 0x1A4E));
        if bin < 0 || r.uniform() > self.biome_def().lake {
            return;
        }
        let a = r.range(20.0, 45.0);
        let b = r.range(15.0, 35.0);
        let s = (bin as f64 + 0.25 + 0.5 * r.uniform() as f64) * LAKE_BIN;
        // Some lakes lie across the path (you fly over the water), most beside it.
        let u = if r.uniform() < 0.35 { r.range(-0.4, 0.4) * b } else { r.sign() * (b + r.range(6.0, 35.0)) };
        let shape = Shape { s, u, a, b, ..Default::default() };
        let mut pts = [(0.0f64, 0.0f32); PATH_POINTS];
        for (i, p) in pts.iter_mut().enumerate() {
            let t = core::f32::consts::TAU * i as f32 / PATH_POINTS as f32;
            *p = (s + (a * t.cos()) as f64, u + b * t.sin());
        }
        let mut f = Feature { kind: FeatureKind::Lake, z: 0.05, s_min: s - a as f64, s_max: s + a as f64, shape, preset: 1, size: a.max(b), ..Default::default() };
        let w = self.path.world(s, u);
        (f.x, f.y) = (w[0], w[1]);
        self.map_polyline(&mut f, &pts);
        let Some(id) = self.add_feature(f) else { return };
        // Lapping along the shore (every ~16 m), a little inside the water.
        let n = ((core::f32::consts::TAU * 0.5 * (a + b)) / 16.0).ceil().clamp(6.0, 16.0) as usize;
        for i in 0..n {
            let t = core::f32::consts::TAU * (i as f32 + 0.5) / n as f32;
            let w = self.path.world(s + ((a - 1.5) * t.cos()) as f64, u + (b - 1.5) * t.sin());
            self.add_site(Site { kind: SiteKind::Water, feature: id, x: w[0], y: w[1], z: 0.05, preset: 1, weight: 0.7, length: 16.0, variation: [1.0, 1.0, 0.0], ..Default::default() });
        }
    }

    fn ridge(&mut self, bin: i64) {
        let mut r = Stream::new(hash(self.seed, bin, self.biome as i64, 0x21D6));
        let b = *self.biome_def();
        if bin < 0 || r.uniform() > b.ridge {
            return;
        }
        let half = r.range(30.0, 80.0);
        let s = (bin as f64 + 0.2 + 0.6 * r.uniform() as f64) * RIDGE_BIN;
        let side = r.sign();
        let u = side * r.range(16.0, 45.0);
        let height = r.range(4.0, 12.0);
        let shape = Shape { s, u, a: half, b: 2.5, ..Default::default() };
        let pts = [(s - half as f64, u + side * r.range(-6.0, 6.0)), (s, u), (s + half as f64, u + side * r.range(-6.0, 6.0))];
        let mut f = Feature { kind: FeatureKind::Ridge, z: height, s_min: s - half as f64, s_max: s + half as f64, shape, size: height, ..Default::default() };
        let w = self.path.world(s, u);
        (f.x, f.y) = (w[0], w[1]);
        self.map_polyline(&mut f, &pts);
        let Some(_) = self.add_feature(f) else { return };
        // A waterfall down the ridge face, falling towards the path.
        if r.uniform() < b.waterfall {
            let ws = s + r.range(-0.5, 0.5) as f64 * half as f64;
            let wu = u - side * 3.5;
            let w = self.path.world(ws, wu);
            let fall = Feature {
                kind: FeatureKind::Waterfall,
                x: w[0],
                y: w[1],
                z: 0.6 * height,
                s_min: ws - 5.0,
                s_max: ws + 5.0,
                shape: Shape { s: ws, u: wu, a: 3.0, ..Default::default() },
                preset: 6,
                size: height,
                ..Default::default()
            };
            if let Some(id) = self.add_feature(fall) {
                self.add_site(Site { kind: SiteKind::Water, feature: id, x: w[0], y: w[1], z: 0.5 * height, preset: 6, weight: 2.5, length: 4.0, variation: [1.0, 1.0, 0.0], ..Default::default() });
                let p = self.path.world(ws, wu - side * 3.0);
                self.add_site(Site { kind: SiteKind::Water, feature: id, x: p[0], y: p[1], z: 0.2, preset: 4, weight: 1.2, length: 6.0, variation: [1.0, 1.0, 0.0], ..Default::default() });
            }
        }
    }

    fn campfire(&mut self, bin: i64) {
        let mut r = Stream::new(hash(self.seed, bin, self.biome as i64, 0xCA4F));
        if bin < 0 || r.uniform() > self.biome_def().campfire {
            return;
        }
        // In the most open spot among a few candidates near the path.
        let s0 = (bin as f64 + 0.2 + 0.6 * r.uniform() as f64) * CAMP_BIN;
        let mut best = (s0, 10.0f32, 2.0f32);
        for _ in 0..8 {
            let s = s0 + r.range(-40.0, 40.0) as f64;
            let u = r.sign() * r.range(8.0, 30.0);
            let c = self.cover(s, u);
            if c < best.2 {
                best = (s, u, c);
            }
        }
        let (s, u, _) = best;
        let w = self.path.world(s, u);
        let f = Feature { kind: FeatureKind::Campfire, x: w[0], y: w[1], z: 0.3, s_min: s - 6.0, s_max: s + 6.0, shape: Shape { s, u, ..Default::default() }, ..Default::default() };
        if let Some(id) = self.add_feature(f) {
            self.add_site(Site { kind: SiteKind::Fire, feature: id, x: w[0], y: w[1], z: 0.3, preset: 0, weight: 1.0, length: 1.0, variation: [1.0, 1.0, 0.0], ..Default::default() });
        }
    }

    fn generate_chunk(&mut self, k: i64) {
        let b = *self.biome_def();
        let s0 = k as f64 * CHUNK;
        let mut chunk = Chunk { k, trees: (self.tree_tail, self.tree_tail), cells: [Cell::default(); BANDS] };
        let mut r = Stream::new(hash(self.seed, k, self.biome as i64, 0xC4C4));

        // Boulders in rocky country.
        if r.uniform() < b.rocks * 0.6 {
            let n = 1 + (r.uniform() * 3.0) as usize;
            for _ in 0..n {
                let s = s0 + r.range(0.0, CHUNK as f32) as f64;
                let u = r.sign() * r.range(4.0, 40.0);
                if self.carved(s, u, 1.0) {
                    continue;
                }
                let radius = r.range(0.5, 1.8);
                let w = self.path.world(s, u);
                let f = Feature {
                    kind: FeatureKind::Rock,
                    x: w[0],
                    y: w[1],
                    z: radius * 1.4,
                    s_min: s - radius as f64,
                    s_max: s + radius as f64,
                    shape: Shape { s, u, a: radius, ..Default::default() },
                    size: radius,
                    ..Default::default()
                };
                let _ = self.add_feature(f);
            }
        }

        // Trees on a jittered grid, thinned by cover; generated at twice the biome density,
        // the user's density keeps `rank < density / 2`.
        let gen_density = 2.0 * b.density_ha;
        let p_cell = (gen_density * (GRID * GRID) as f32 / 10_000.0).min(1.0);
        let rows = (CHUNK / GRID).round() as i64;
        let half = (TREE_U as f64 / GRID).round() as i64;
        let mut band_sum = [(0.0f64, 0.0f64, 0.0f32, 0u32, [0.0f32; 7]); BANDS];
        let mut levels = [[0.0f32; LEVELS]; BANDS];
        for gs in 0..rows {
            for gu in -half..half {
                let h = hash(self.seed, k * 64 + gs, gu, 0x7EE5 ^ self.biome as u64);
                let mut t = Stream::new(h);
                let s = s0 + (gs as f64 + t.uniform() as f64) * GRID;
                let u = ((gu as f64 + t.uniform() as f64) * GRID) as f32;
                if u.abs() < CORRIDOR {
                    continue;
                }
                let accept = t.uniform();
                let c = self.base_cover(s, u);
                if accept >= c * p_cell {
                    continue;
                }
                if self.carved(s, u, 1.5) {
                    continue;
                }
                // Species: biome weights tilted by local groves; willow and alder by water.
                let mut w = b.species;
                for (i, x) in w.iter_mut().enumerate() {
                    *x *= 0.3 + 1.4 * noise2(self.seed, 0x5EC0 + i as u64, s / 90.0, u as f64 / 90.0);
                }
                if self.carved(s, u, 14.0) {
                    w[3] += 0.6;
                    w[4] += 0.6;
                }
                let sp = pick(&w, t.uniform());
                let (crown_z, crown_r, trunk_r) = tree_shape(SPECIES[sp], &mut t);
                let rank = t.uniform();
                if self.tree_tail - self.tree_head >= MAX_TREES as u64 {
                    continue;
                }
                let pos = self.path.world(s, u);
                let band = BAND_EDGES.iter().rposition(|e| u >= *e).unwrap_or(0).min(BANDS - 1);
                let id = self.id();
                let tree = Tree { id, x: pos[0], y: pos[1], species: sp as u8, crown_z, crown_r, trunk_r, rank, band: band as u8 };
                self.trees[(self.tree_tail as usize) % MAX_TREES] = tree;
                self.tree_tail += 1;
                let area = tree.crown_area();
                let bs = &mut band_sum[band];
                bs.0 += pos[0];
                bs.1 += pos[1];
                bs.2 += crown_z;
                bs.3 += 1;
                bs.4[sp] += area;
                for (l, a) in levels[band].iter_mut().enumerate() {
                    if rank < (l as f32 + 1.0) * 0.25 * 0.5 {
                        *a += area;
                    }
                }

                // Birds live in the trees.
                if t.uniform() < b.birds * 2.0 {
                    let edge = c < 0.6;
                    let bw = bird_weights(SPECIES[sp], edge);
                    let species = pick(&bw, t.uniform()) as u8;
                    let a = t.range(0.0, 6.283);
                    let rr = t.range(0.2, 0.8) * crown_r;
                    let z = crown_z + t.range(-0.4, 0.9) * crown_r;
                    let pos3 = [pos[0] + (rr * a.cos()) as f64, pos[1] + (rr * a.sin()) as f64, z as f64];
                    let brank = t.uniform();
                    if let Some(slot) = self.birds.iter().position(|x| !x.alive) {
                        let id = self.id();
                        self.birds[slot] = Bird { id, alive: true, species, rank: brank, k, pos: pos3, ..Default::default() };
                    }
                }
            }
        }
        chunk.trees.1 = self.tree_tail;

        // Aggregate cells: inner bands from the trees, outer bands from the cover field.
        for (band, cell) in chunk.cells.iter_mut().enumerate() {
            let (lo, hi) = (BAND_EDGES[band], BAND_EDGES[band + 1]);
            let mid_u = 0.5 * (lo + hi);
            let centre = self.path.world(s0 + 0.5 * CHUNK, mid_u);
            let bs = band_sum[band];
            if bs.3 > 0 {
                cell.x = bs.0 / bs.3 as f64;
                cell.y = bs.1 / bs.3 as f64;
                cell.canopy_z = bs.2 / bs.3 as f32;
                cell.area = levels[band];
                cell.species = bs.4.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).map_or(0, |x| x.0) as u8;
            } else if lo >= TREE_U || hi <= -TREE_U {
                // Outer band: integrate the cover field on a 4 x 4 grid.
                let mut cov = 0.0;
                for i in 0..4 {
                    for j in 0..4 {
                        let s = s0 + (i as f64 + 0.5) * CHUNK / 4.0;
                        let u = lo + (j as f32 + 0.5) * (hi - lo) / 4.0;
                        cov += self.cover(s, u);
                    }
                }
                let area1 = cov / 16.0 * CHUNK as f32 * (hi - lo) * 0.6;
                for (l, a) in cell.area.iter_mut().enumerate() {
                    *a = area1 * (l as f32 + 1.0) * 0.25;
                }
                (cell.x, cell.y) = (centre[0], centre[1]);
                cell.canopy_z = 8.0;
                let mut w = b.species;
                for (i, x) in w.iter_mut().enumerate() {
                    *x *= 0.3 + 1.4 * noise2(self.seed, 0x5EC0 + i as u64, (s0 + 16.0) / 90.0, mid_u as f64 / 90.0);
                }
                cell.species = w.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).map_or(0, |x| x.0) as u8;
            } else {
                (cell.x, cell.y) = (centre[0], centre[1]);
                cell.canopy_z = 8.0;
            }
        }

        // Cricket meadows in the open (one or two patches per chunk at most).
        let mut placed = 0;
        for _ in 0..6 {
            let s = s0 + r.range(0.0, CHUNK as f32) as f64;
            let u = r.sign() * r.range(5.0, 55.0);
            let preset = if r.uniform() < 0.6 { 0u8 } else { 3u8 };
            if placed >= 2 || self.cover(s, u) > 0.12 {
                continue;
            }
            let w = self.path.world(s, u);
            let f = Feature {
                kind: FeatureKind::Meadow,
                x: w[0],
                y: w[1],
                z: 0.1,
                s_min: s - 8.0,
                s_max: s + 8.0,
                shape: Shape { s, u, a: 8.0, ..Default::default() },
                preset,
                size: 8.0,
                ..Default::default()
            };
            if let Some(id) = self.add_feature(f) {
                self.add_site(Site { kind: SiteKind::Insects, feature: id, x: w[0], y: w[1], z: 0.1, preset, weight: 1.0, length: 8.0, variation: [1.0, 1.0, 0.0], ..Default::default() });
                placed += 1;
            }
        }

        let slot = k.rem_euclid(MAX_CHUNKS as i64) as usize;
        self.chunks[slot] = chunk;
    }
}
