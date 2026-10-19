//! Lock-free hand-over of the streamed objects to the UI thread: a seqlock over fixed arrays
//! of atomics. The audio thread writes a whole snapshot (objects + shape polylines) between
//! two increments of `seq`; readers retry while a write is in progress.

use std::sync::atomic::{fence, AtomicU32, Ordering};

use crate::engine::AtomicF32;
use crate::math::Vec3;
use crate::scene::ObjectInfo;

/// Most objects one snapshot carries (nearest trees are dropped first).
pub const OBJECTS: usize = 256;
/// Objects with a shape (river bed, lake shore, ridge line).
pub const PATHS: usize = 16;
pub const PATH_POINTS: usize = super::land::PATH_POINTS;

pub struct FlightBridge {
    seq: AtomicU32,
    count: AtomicU32,
    ids: Box<[AtomicU32]>,
    kinds: Box<[AtomicU32]>,
    flags: Box<[AtomicU32]>,
    pos: Box<[AtomicF32]>,
    paths: AtomicU32,
    path_id: Box<[AtomicU32]>,
    path_len: Box<[AtomicU32]>,
    path_pts: Box<[AtomicF32]>,
}

fn u32s(n: usize) -> Box<[AtomicU32]> {
    (0..n).map(|_| AtomicU32::new(0)).collect()
}

fn f32s(n: usize) -> Box<[AtomicF32]> {
    (0..n).map(|_| AtomicF32::new(0.0)).collect()
}

/// Writer handle for one snapshot (audio thread).
pub struct Snapshot<'a> {
    b: &'a FlightBridge,
    n: usize,
    paths: usize,
}

impl Snapshot<'_> {
    pub fn len(&self) -> usize {
        self.n
    }

    pub fn full(&self) -> bool {
        self.n >= OBJECTS
    }

    pub fn object(&mut self, o: ObjectInfo) {
        if self.n >= OBJECTS {
            return;
        }
        let (b, i) = (self.b, self.n);
        b.ids[i].store(o.id, Ordering::Relaxed);
        b.kinds[i].store(o.kind, Ordering::Relaxed);
        b.flags[i].store(o.flags, Ordering::Relaxed);
        b.pos[3 * i].store(o.x);
        b.pos[3 * i + 1].store(o.y);
        b.pos[3 * i + 2].store(o.z);
        self.n += 1;
    }

    /// Shape of the object `id`, listener-relative points.
    pub fn path(&mut self, id: u32, pts: impl Iterator<Item = Vec3>) {
        if self.paths >= PATHS {
            return;
        }
        let (b, i) = (self.b, self.paths);
        let mut n = 0;
        for p in pts.take(PATH_POINTS) {
            let k = (i * PATH_POINTS + n) * 3;
            b.path_pts[k].store(p.x);
            b.path_pts[k + 1].store(p.y);
            b.path_pts[k + 2].store(p.z);
            n += 1;
        }
        b.path_id[i].store(id, Ordering::Relaxed);
        b.path_len[i].store(n as u32, Ordering::Relaxed);
        self.paths += 1;
    }
}

impl Drop for Snapshot<'_> {
    fn drop(&mut self) {
        let b = self.b;
        b.count.store(self.n as u32, Ordering::Relaxed);
        b.paths.store(self.paths as u32, Ordering::Relaxed);
        let s = b.seq.load(Ordering::Relaxed);
        b.seq.store(s.wrapping_add(1), Ordering::Release);
    }
}

impl Default for FlightBridge {
    fn default() -> Self {
        Self::new()
    }
}

impl FlightBridge {
    pub fn new() -> Self {
        Self {
            seq: AtomicU32::new(0),
            count: AtomicU32::new(0),
            ids: u32s(OBJECTS),
            kinds: u32s(OBJECTS),
            flags: u32s(OBJECTS),
            pos: f32s(3 * OBJECTS),
            paths: AtomicU32::new(0),
            path_id: u32s(PATHS),
            path_len: u32s(PATHS),
            path_pts: f32s(3 * PATHS * PATH_POINTS),
        }
    }

    /// Starts a snapshot; it is published when the handle drops. Single writer.
    pub fn begin(&self) -> Snapshot<'_> {
        let s = self.seq.load(Ordering::Relaxed);
        self.seq.store(s.wrapping_add(1), Ordering::Relaxed);
        fence(Ordering::Release);
        Snapshot { b: self, n: 0, paths: 0 }
    }

    /// Runs `f` until it saw a consistent snapshot (a few retries at most).
    fn consistent<T>(&self, mut f: impl FnMut() -> T) -> T {
        let mut out = f();
        for _ in 0..16 {
            let s1 = self.seq.load(Ordering::Acquire);
            if s1 % 2 == 1 {
                std::hint::spin_loop();
                continue;
            }
            out = f();
            fence(Ordering::Acquire);
            if self.seq.load(Ordering::Relaxed) == s1 {
                break;
            }
        }
        out
    }

    /// Any thread.
    pub fn read(&self, out: &mut [ObjectInfo]) -> usize {
        self.consistent(|| {
            let n = (self.count.load(Ordering::Relaxed) as usize).min(OBJECTS).min(out.len());
            for (i, o) in out.iter_mut().take(n).enumerate() {
                *o = ObjectInfo {
                    id: self.ids[i].load(Ordering::Relaxed),
                    kind: self.kinds[i].load(Ordering::Relaxed),
                    flags: self.flags[i].load(Ordering::Relaxed),
                    x: self.pos[3 * i].load(),
                    y: self.pos[3 * i + 1].load(),
                    z: self.pos[3 * i + 2].load(),
                };
            }
            n
        })
    }

    /// Shape points (x, y, z triples) of object `id` into `out`; returns the point count.
    pub fn path(&self, id: u32, out: &mut [f32]) -> usize {
        self.consistent(|| {
            let n = (self.paths.load(Ordering::Relaxed) as usize).min(PATHS);
            for i in 0..n {
                if self.path_id[i].load(Ordering::Relaxed) != id {
                    continue;
                }
                let len = (self.path_len[i].load(Ordering::Relaxed) as usize).min(PATH_POINTS).min(out.len() / 3);
                for k in 0..len * 3 {
                    out[k] = self.path_pts[i * PATH_POINTS * 3 + k].load();
                }
                return len;
            }
            0
        })
    }
}

/// Where the listener is in the landscape and what the landscape is (UI terrain). Published
/// by the audio thread every block; read by the UI with [`FrameBridge::read`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(C)]
pub struct Frame {
    /// Listener position in the landscape, m (world frame).
    pub x: f64,
    pub y: f64,
    /// Course heading, rad: the travel frame is the world rotated by −heading.
    pub heading: f32,
    /// Absolute heights (m above sea level) of the ears and of the ground under them.
    pub ear_z: f32,
    pub ground_z: f32,
    pub relief: f32,
    /// The landscape: pass to `asmr_terrain_new`.
    pub seed: u64,
    /// The preset (`flight.biome`): pass it to the terrain sampler.
    pub biome: u32,
    /// Changes whenever the landscape is regenerated.
    pub generation: u32,
    /// 0 cruising .. 1 stopped at a place.
    pub linger: f32,
    /// Precipitation falls as snow (1) or rain (0).
    pub snow: f32,
    /// How the ground looks: snow cover, sand cover, bare rock above this slope, a sea.
    pub snow_cover: f32,
    pub sand_cover: f32,
    pub rock_slope: f32,
    pub sea: f32,
    /// Mean elevation (sine) of the skyline all around: 0 open .. ~0.5 deep valley.
    pub enclosure: f32,
    /// The biome under the listener (index into `land::BIOMES`): the preset itself, or in a
    /// `journey` the dominant biome of the region it is crossing.
    pub local_biome: u32,
}

const _: () = assert!(core::mem::size_of::<Frame>() == 80);

/// Seqlock over the frame's words.
pub struct FrameBridge {
    seq: AtomicU32,
    words: [AtomicU32; FRAME_WORDS],
}

const FRAME_WORDS: usize = core::mem::size_of::<Frame>() / 4;

impl Default for FrameBridge {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameBridge {
    pub fn new() -> Self {
        Self { seq: AtomicU32::new(0), words: core::array::from_fn(|_| AtomicU32::new(0)) }
    }

    /// Single writer (audio thread).
    pub fn publish(&self, f: &Frame) {
        let s = self.seq.load(Ordering::Relaxed);
        self.seq.store(s.wrapping_add(1), Ordering::Relaxed);
        fence(Ordering::Release);
        // SAFETY: Frame is repr(C), plain old data, a multiple of 4 bytes.
        let words: [u32; FRAME_WORDS] = unsafe { core::mem::transmute_copy(f) };
        for (a, w) in self.words.iter().zip(words) {
            a.store(w, Ordering::Relaxed);
        }
        self.seq.store(s.wrapping_add(2), Ordering::Release);
    }

    /// Any thread: the latest consistent frame.
    pub fn read(&self) -> Frame {
        let mut words = [0u32; FRAME_WORDS];
        for _ in 0..64 {
            let s1 = self.seq.load(Ordering::Acquire);
            if s1 % 2 == 1 {
                std::hint::spin_loop();
                continue;
            }
            for (w, a) in words.iter_mut().zip(self.words.iter()) {
                *w = a.load(Ordering::Relaxed);
            }
            fence(Ordering::Acquire);
            if self.seq.load(Ordering::Relaxed) == s1 {
                break;
            }
        }
        // SAFETY: as above; any bit pattern is a valid Frame.
        unsafe { core::mem::transmute_copy(&words) }
    }
}
