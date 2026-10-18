//! The real-time engine: scene sources -> propagation -> 3rd-order Ambisonic bus ->
//! head rotation -> binaural.
//!
//! Threading contract: `render` is called from exactly one (audio) thread. Everything else
//! (params, telemetry, spectrum) may be called from any thread; it only touches atomics.

use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

use core::f32::consts::TAU;

use crate::ambisonics::{self, CHANNELS};
use crate::binaural::BinauralDecoder;
use crate::dsp::soft_clip;
use crate::fft::fft;
use crate::math::{Mat3, Vec3};
use crate::rng::Rng;
use crate::sources::{Bird, BubbleStream, WindSource};
use crate::windfield::WindField;

/// Control-rate block: source positions and gains update once per block.
pub const BLOCK: usize = 64;
pub const MAX_SOURCES: usize = 32;
/// Half-width of the scene along y (metres) used by the wind front and the UI.
pub const WIND_SPAN: f32 = 20.0;

const SPECTRUM_RING: usize = 4096;
const SPECTRUM_FFT: usize = 2048;
const WIND_FIELD_POINTS: usize = 64;

const WATER_GAIN: f32 = 1.0;
const WIND_GAIN: f32 = 0.6;
const BIRD_GAIN: f32 = 0.8;
const MASTER_GAIN: f32 = 1.0;

pub mod param {
    pub const VOLUME: u32 = 0;
    pub const WIND: u32 = 1;
    pub const BIRDS: u32 = 2;
    pub const WATER: u32 = 3;
    pub const ORBIT: u32 = 4;
    pub const PLAYING: u32 = 5;
    pub const HEAD_YAW: u32 = 6;
    pub const HEAD_PITCH: u32 = 7;
    pub const HEAD_ROLL: u32 = 8;
    pub const COUNT: usize = 9;
}

pub mod telemetry {
    pub const DSP_LOAD: u32 = 0;
    pub const GUST: u32 = 1;
    pub const SAMPLE_RATE: u32 = 2;
    pub const BUBBLE_RATE: u32 = 3;
    pub const TIME: u32 = 4;
    pub const OUTPUT_LEVEL: u32 = 5;
    pub const COUNT: usize = 6;
}

#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SourceKind {
    Water = 0,
    Wind = 1,
    Bird = 2,
}

/// Mirrors `AsmrSourceInfo` in asmr.h.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct SourceInfo {
    pub kind: u32,
    pub world_x: f32,
    pub world_y: f32,
    pub world_z: f32,
    pub rel_x: f32,
    pub rel_y: f32,
    pub rel_z: f32,
    pub level: f32,
    pub freq: f32,
}

const SOURCE_FIELDS: usize = 9;

struct AtomicF32(AtomicU32);

impl AtomicF32 {
    fn new(v: f32) -> Self {
        Self(AtomicU32::new(v.to_bits()))
    }
    #[inline]
    fn load(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Relaxed))
    }
    #[inline]
    fn store(&self, v: f32) {
        self.0.store(v.to_bits(), Ordering::Relaxed)
    }
}

fn atomics(n: usize) -> Box<[AtomicF32]> {
    (0..n).map(|_| AtomicF32::new(0.0)).collect()
}

struct Shared {
    params: Box<[AtomicF32]>,
    telemetry: Box<[AtomicF32]>,
    spectrum: Box<[AtomicF32]>,
    spectrum_pos: AtomicUsize,
    sources: Box<[AtomicF32]>,
    source_count: AtomicUsize,
    wind_field: Box<[AtomicF32]>,
}

enum Generator {
    Water(BubbleStream),
    Wind(WindSource),
    Bird(Bird),
}

struct Slot {
    kind: SourceKind,
    home: Vec3,
    position: Vec3,
    generator: Generator,
    sh: [f32; CHANNELS],
    level: f32,
    rel: Vec3,
}

impl Slot {
    fn new(kind: SourceKind, position: Vec3, generator: Generator) -> Self {
        Self { kind, home: position, position, generator, sh: [0.0; CHANNELS], level: 0.0, rel: position }
    }
}

struct Dsp {
    sample_rate: f32,
    slots: Vec<Slot>,
    orbit_slot: usize,
    decoder: BinauralDecoder,
    wind: WindField,
    rng: Rng,
    bus: [[f32; BLOCK]; CHANNELS],
    scratch: [f32; BLOCK],
    left: [f32; BLOCK],
    right: [f32; BLOCK],
    mono: [f32; BLOCK],
    fade: f32,
    volume: f32,
    orbit_phase: f32,
    time: f64,
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    load: f32,
    out_level: f32,
}

pub struct Engine {
    shared: Shared,
    dsp: UnsafeCell<Dsp>,
}

// SAFETY: `dsp` is only touched by `render`, which the API contract restricts to a single
// thread. Everything reachable from other threads is atomic.
unsafe impl Sync for Engine {}
unsafe impl Send for Engine {}

impl Engine {
    pub fn new(sample_rate: f32) -> Self {
        let sr = if sample_rate.is_finite() && sample_rate >= 8000.0 { sample_rate } else { 48_000.0 };
        let mut rng = Rng::new(0xA5A5_3000);

        let mut slots = Vec::new();
        // River: a line ~4 m in front, below ear level, split into decorrelated segments.
        for i in 0..5 {
            let y = -8.0 + 4.0 * i as f32;
            let x = 4.0 + 0.6 * (i as f32 * 1.7).sin();
            slots.push(Slot::new(
                SourceKind::Water,
                Vec3::new(x, y, -1.2),
                Generator::Water(BubbleStream::new(sr, rng.next_u64(), 350.0)),
            ));
        }
        // Wind in the canopy all around.
        for i in 0..6 {
            let a = i as f32 * TAU / 6.0 + 0.3;
            let z = 3.0 + 1.5 * (i % 2) as f32;
            slots.push(Slot::new(
                SourceKind::Wind,
                Vec3::new(7.0 * a.cos(), 7.0 * a.sin(), z),
                Generator::Wind(WindSource::new(sr, rng.next_u64(), 0.75 + 0.1 * i as f32)),
            ));
        }
        // Two birds: one perched front-left (it can also orbit the listener), one front-right.
        let orbit_slot = slots.len();
        slots.push(Slot::new(
            SourceKind::Bird,
            Vec3::new(2.5, 3.0, 3.5),
            Generator::Bird(Bird::new(sr, rng.next_u64(), 4200.0, 1.0)),
        ));
        slots.push(Slot::new(
            SourceKind::Bird,
            Vec3::new(5.0, -6.0, 5.0),
            Generator::Bird(Bird::new(sr, rng.next_u64(), 3300.0, 0.85)),
        ));

        let shared = Shared {
            params: atomics(param::COUNT),
            telemetry: atomics(telemetry::COUNT),
            spectrum: atomics(SPECTRUM_RING),
            spectrum_pos: AtomicUsize::new(0),
            sources: atomics(MAX_SOURCES * SOURCE_FIELDS),
            source_count: AtomicUsize::new(0),
            wind_field: atomics(WIND_FIELD_POINTS),
        };
        shared.params[param::VOLUME as usize].store(0.7);
        shared.params[param::WIND as usize].store(0.45);
        shared.params[param::BIRDS as usize].store(0.6);
        shared.params[param::WATER as usize].store(0.6);
        shared.telemetry[telemetry::SAMPLE_RATE as usize].store(sr);

        let dsp = Dsp {
            sample_rate: sr,
            slots,
            orbit_slot,
            decoder: BinauralDecoder::new(sr),
            wind: WindField::new(sr),
            rng,
            bus: [[0.0; BLOCK]; CHANNELS],
            scratch: [0.0; BLOCK],
            left: [0.0; BLOCK],
            right: [0.0; BLOCK],
            mono: [0.0; BLOCK],
            fade: 0.0,
            volume: 0.7,
            orbit_phase: 0.0,
            time: 0.0,
            load: 0.0,
            out_level: 0.0,
        };
        dsp.publish(&shared);
        Self { shared, dsp: UnsafeCell::new(dsp) }
    }

    pub fn set_param(&self, id: u32, value: f32) {
        if let Some(p) = self.shared.params.get(id as usize) {
            if value.is_finite() {
                p.store(value);
            }
        }
    }

    pub fn param(&self, id: u32) -> f32 {
        self.shared.params.get(id as usize).map_or(0.0, |p| p.load())
    }

    pub fn telemetry(&self, id: u32) -> f32 {
        self.shared.telemetry.get(id as usize).map_or(0.0, |p| p.load())
    }

    /// Audio thread only.
    pub fn render(&self, left: &mut [f32], right: &mut [f32]) {
        #[cfg(not(target_arch = "wasm32"))]
        let started = Instant::now();
        // SAFETY: single audio thread, see the type-level comment.
        let dsp = unsafe { &mut *self.dsp.get() };
        let n = left.len().min(right.len());
        let mut off = 0;
        while off < n {
            let len = (n - off).min(BLOCK);
            dsp.process_block(&self.shared, &mut left[off..off + len], &mut right[off..off + len]);
            let mut pos = self.shared.spectrum_pos.load(Ordering::Relaxed);
            for &v in &dsp.mono[..len] {
                self.shared.spectrum[pos % SPECTRUM_RING].store(v);
                pos = pos.wrapping_add(1);
            }
            self.shared.spectrum_pos.store(pos, Ordering::Release);
            off += len;
        }
        dsp.publish(&self.shared);

        // Instant::now() traps on wasm32-unknown-unknown; there the shell reports its own load.
        #[cfg(not(target_arch = "wasm32"))]
        let budget = n as f32 / dsp.sample_rate;
        #[cfg(not(target_arch = "wasm32"))]
        if budget > 0.0 {
            let load = started.elapsed().as_secs_f32() / budget;
            dsp.load = dsp.load * 0.95 + load * 0.05;
            self.shared.telemetry[telemetry::DSP_LOAD as usize].store(dsp.load);
        }
    }

    /// Copies source telemetry into `out`, returns how many were written.
    pub fn sources(&self, out: &mut [SourceInfo]) -> usize {
        let count = self.shared.source_count.load(Ordering::Acquire).min(out.len());
        for (i, o) in out.iter_mut().take(count).enumerate() {
            let f = |k: usize| self.shared.sources[i * SOURCE_FIELDS + k].load();
            *o = SourceInfo {
                kind: f(0) as u32,
                world_x: f(1),
                world_y: f(2),
                world_z: f(3),
                rel_x: f(4),
                rel_y: f(5),
                rel_z: f(6),
                level: f(7),
                freq: f(8),
            };
        }
        count
    }

    /// Log-frequency magnitude spectrum of the output (40 Hz .. 16 kHz), each band in 0..1.
    pub fn spectrum(&self, out: &mut [f32]) {
        if out.is_empty() {
            return;
        }
        let sr = self.telemetry(telemetry::SAMPLE_RATE);
        let pos = self.shared.spectrum_pos.load(Ordering::Acquire);
        let mut re = vec![0.0f32; SPECTRUM_FFT];
        let mut im = vec![0.0f32; SPECTRUM_FFT];
        for (i, v) in re.iter_mut().enumerate() {
            let idx = (pos.wrapping_sub(SPECTRUM_FFT).wrapping_add(i)) % SPECTRUM_RING;
            let w = 0.5 - 0.5 * (TAU * i as f32 / SPECTRUM_FFT as f32).cos();
            *v = self.shared.spectrum[idx].load() * w;
        }
        fft(&mut re, &mut im);

        // Full-scale sine through a Hann window peaks at N/4.
        let norm = 4.0 / SPECTRUM_FFT as f32;
        let f_lo = 40.0f32;
        let f_hi = 16_000.0f32.min(0.5 * sr);
        let bands = out.len();
        let bin_hz = sr / SPECTRUM_FFT as f32;
        for (b, o) in out.iter_mut().enumerate() {
            let e0 = f_lo * (f_hi / f_lo).powf(b as f32 / bands as f32);
            let e1 = f_lo * (f_hi / f_lo).powf((b + 1) as f32 / bands as f32);
            let k0 = ((e0 / bin_hz) as usize).max(1);
            let k1 = ((e1 / bin_hz).ceil() as usize).max(k0 + 1).min(SPECTRUM_FFT / 2);
            let mut p = 0.0f32;
            for k in k0..k1 {
                let m = (re[k] * re[k] + im[k] * im[k]).sqrt() * norm;
                p += m * m;
            }
            p /= (k1 - k0).max(1) as f32;
            let db = 10.0 * (p + 1e-12).log10();
            *o = ((db + 90.0) / 70.0).clamp(0.0, 1.0);
        }
    }

    /// Gust strength sampled across the scene from left (+y) to right (-y).
    pub fn wind_field(&self, out: &mut [f32]) {
        let n = out.len();
        for (i, o) in out.iter_mut().enumerate() {
            let x = if n > 1 { i as f32 / (n - 1) as f32 } else { 0.0 } * (WIND_FIELD_POINTS - 1) as f32;
            let i0 = x.floor() as usize;
            let i1 = (i0 + 1).min(WIND_FIELD_POINTS - 1);
            let f = x - i0 as f32;
            *o = self.shared.wind_field[i0].load() * (1.0 - f) + self.shared.wind_field[i1].load() * f;
        }
    }
}

impl Dsp {
    fn process_block(&mut self, shared: &Shared, out_l: &mut [f32], out_r: &mut [f32]) {
        let len = out_l.len();
        let sr = self.sample_rate;
        let dt = len as f32 / sr;
        let p = |k: u32| shared.params[k as usize].load();

        let playing = p(param::PLAYING) > 0.5;
        if !playing && self.fade < 1e-4 {
            self.fade = 0.0;
            out_l.fill(0.0);
            out_r.fill(0.0);
            self.mono[..len].fill(0.0);
            self.out_level = 0.0;
            return;
        }

        let wind = p(param::WIND).clamp(0.0, 1.0);
        let birds = p(param::BIRDS).clamp(0.0, 1.0);
        let water = p(param::WATER).clamp(0.0, 1.0);
        let orbit = p(param::ORBIT).clamp(0.0, 1.0);
        let volume = p(param::VOLUME).clamp(0.0, 1.5);
        let head = Mat3::from_head(p(param::HEAD_YAW), p(param::HEAD_PITCH), p(param::HEAD_ROLL));

        self.wind.step(wind, dt, &mut self.rng);
        if orbit > 1e-3 {
            self.orbit_phase = (self.orbit_phase + TAU * orbit * 0.3 * dt) % TAU;
        }

        for ch in self.bus.iter_mut() {
            ch[..len].fill(0.0);
        }
        let inv_len = 1.0 / len as f32;

        for (idx, slot) in self.slots.iter_mut().enumerate() {
            if idx == self.orbit_slot {
                slot.position = if orbit > 1e-3 {
                    Vec3::new(3.0 * self.orbit_phase.cos(), 3.0 * self.orbit_phase.sin(), 1.8)
                } else {
                    slot.home
                };
            }

            let tmp = &mut self.scratch[..len];
            let kind_gain = match &mut slot.generator {
                Generator::Water(s) => {
                    s.render(tmp, water);
                    WATER_GAIN
                }
                Generator::Wind(w) => {
                    w.render(tmp, self.wind.local(slot.position.y));
                    WIND_GAIN
                }
                Generator::Bird(b) => {
                    b.render(tmp, birds);
                    BIRD_GAIN
                }
            };

            let energy: f32 = tmp.iter().map(|v| v * v).sum();
            slot.level = slot.level * 0.8 + (energy * inv_len).sqrt() * 0.2;

            // Propagation (M0): direction + spherical spreading. M1 adds delay/Doppler,
            // air absorption and ground effect here.
            let rel = head.apply_transposed(slot.position);
            let dist = rel.length().max(0.3);
            let dir = rel * (1.0 / dist);
            let gain = kind_gain / dist.max(0.7);
            slot.rel = rel;

            let mut target = [0.0f32; CHANNELS];
            ambisonics::encode(dir, &mut target);
            if energy == 0.0 {
                for ch in 0..CHANNELS {
                    slot.sh[ch] = target[ch] * gain;
                }
                continue;
            }
            for ch in 0..CHANNELS {
                let a = slot.sh[ch];
                let b = target[ch] * gain;
                let step = (b - a) * inv_len;
                let mut g = a;
                let out = &mut self.bus[ch];
                for i in 0..len {
                    g += step;
                    out[i] += tmp[i] * g;
                }
                slot.sh[ch] = b;
            }
        }

        self.decoder.process(&self.bus, len, &mut self.left[..len], &mut self.right[..len]);

        let target_fade = if playing { 1.0 } else { 0.0 };
        let fade_coef = 1.0 - (-1.0 / (0.06 * sr)).exp();
        let vol_coef = 1.0 - (-1.0 / (0.05 * sr)).exp();
        let mut energy = 0.0f32;
        for i in 0..len {
            self.fade += (target_fade - self.fade) * fade_coef;
            self.volume += (volume - self.volume) * vol_coef;
            let g = self.fade * self.volume * MASTER_GAIN;
            let l = soft_clip(self.left[i] * g);
            let r = soft_clip(self.right[i] * g);
            out_l[i] = l;
            out_r[i] = r;
            let m = 0.5 * (l + r);
            self.mono[i] = m;
            energy += m * m;
        }
        self.out_level = self.out_level * 0.9 + (energy * inv_len).sqrt() * 0.1;
        self.time += dt as f64;
    }

    fn publish(&self, shared: &Shared) {
        let count = self.slots.len().min(MAX_SOURCES);
        for (i, s) in self.slots.iter().take(count).enumerate() {
            let freq = match &s.generator {
                Generator::Bird(b) => b.freq(),
                _ => 0.0,
            };
            let fields = [
                s.kind as u32 as f32,
                s.position.x,
                s.position.y,
                s.position.z,
                s.rel.x,
                s.rel.y,
                s.rel.z,
                s.level,
                freq,
            ];
            for (k, v) in fields.iter().enumerate() {
                shared.sources[i * SOURCE_FIELDS + k].store(*v);
            }
        }
        shared.source_count.store(count, Ordering::Release);

        for i in 0..WIND_FIELD_POINTS {
            let y = WIND_SPAN - 2.0 * WIND_SPAN * i as f32 / (WIND_FIELD_POINTS - 1) as f32;
            shared.wind_field[i].store(self.wind.local(y));
        }

        let bubble_rate: f32 = self
            .slots
            .iter()
            .map(|s| match &s.generator {
                Generator::Water(w) => w.rate(),
                _ => 0.0,
            })
            .sum();
        let t = &shared.telemetry;
        t[telemetry::GUST as usize].store(self.wind.gust());
        t[telemetry::BUBBLE_RATE as usize].store(bubble_rate);
        t[telemetry::TIME as usize].store((self.time % 86_400.0) as f32);
        t[telemetry::OUTPUT_LEVEL as usize].store(self.out_level);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_finite_audible_output() {
        let engine = Engine::new(48_000.0);
        engine.set_param(param::PLAYING, 1.0);
        engine.set_param(param::BIRDS, 1.0);
        let mut l = vec![0.0f32; 512];
        let mut r = vec![0.0f32; 512];
        let mut energy = 0.0f64;
        let blocks = 48_000 * 3 / 512;
        for _ in 0..blocks {
            engine.render(&mut l, &mut r);
            for (a, b) in l.iter().zip(&r) {
                assert!(a.is_finite() && b.is_finite());
                assert!(a.abs() <= 1.0 && b.abs() <= 1.0);
                energy += (*a as f64).powi(2) + (*b as f64).powi(2);
            }
        }
        let rms = (energy / (2.0 * (blocks * 512) as f64)).sqrt();
        assert!(rms > 1e-3, "rms = {rms}");
        let mut src = [SourceInfo::default(); MAX_SOURCES];
        assert_eq!(engine.sources(&mut src), 13);
        let mut spec = [0.0f32; 32];
        engine.spectrum(&mut spec);
        assert!(spec.iter().any(|v| *v > 0.0));
    }

    #[test]
    fn silent_when_paused() {
        let engine = Engine::new(48_000.0);
        let mut l = vec![1.0f32; 256];
        let mut r = vec![1.0f32; 256];
        engine.render(&mut l, &mut r);
        assert!(l.iter().chain(&r).all(|v| *v == 0.0));
    }
}
