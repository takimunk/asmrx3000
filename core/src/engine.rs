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

use crate::acoustics::{Acoustics, Emitter, Environment as Acoustic, HrtfModel};
use crate::ambisonics::CHANNELS;
use crate::controls::{self, ControlBank, ControlInfo, ControlState};
use crate::director::{Director, Environment};
use crate::dsp::soft_clip;
use crate::fft::fft;
use crate::math::{Mat3, Vec3};
use crate::physics::ground::GroundType;
use crate::rng::Rng;
use crate::scene::{object_flags, ObjectBridge, ObjectInfo, SceneDesc, World};
use crate::windfield::WindField;

/// Control-rate block: source positions and gains update once per block.
pub const BLOCK: usize = 64;
pub const MAX_SOURCES: usize = 64;
/// Half-width of the scene along y (metres) used by the wind front and the UI.
pub const WIND_SPAN: f32 = 20.0;

const SPECTRUM_RING: usize = 4096;
const SPECTRUM_FFT: usize = 2048;
const WIND_FIELD_POINTS: usize = 64;
/// Live per-track peak waveforms for the UI (ASMR_SCOPE_*), one point per block.
pub const SCOPE_TRACKS: usize = 9;
const SCOPE_LEN: usize = 4096;

const WATER_GAIN: f32 = 1.0;
const WIND_GAIN: f32 = 0.6;
const BIRD_GAIN: f32 = 0.8;
const RAIN_GAIN: f32 = 1.0;
/// The thunder emitter is a direction proxy at `director::THUNDER_PROXY_M`; its generator
/// already includes the long-range attenuation, so undo the proxy's 1/r here.
const THUNDER_GAIN: f32 = crate::director::THUNDER_PROXY_M;
const FIRE_GAIN: f32 = 1.0;
const INSECT_GAIN: f32 = 0.8;
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
    /// Local time of day in hours, 0..24.
    pub const TIME_OF_DAY: u32 = 9;
    /// Rain rate in the open, mm/h (0..200). 0.5 drizzle, 4 steady, 15 heavy, 50 cloudburst.
    pub const RAIN: u32 = 16;
    /// Air temperature, °C.
    pub const TEMPERATURE: u32 = 17;
    /// Relative humidity 0..1.
    pub const HUMIDITY: u32 = 18;
    /// Thunderstorm activity 0..1 (lightning rate and closeness).
    pub const STORM: u32 = 19;
    /// 1 = trained neural voices for species that have one, 0 = procedural voices only.
    pub const NEURAL_BIRDS: u32 = 10;
    // Acoustic environment (asmr.h). TEMPERATURE and HUMIDITY above are shared with the weather.
    pub const PRESSURE: u32 = 34;
    pub const GROUND: u32 = 35;
    pub const TREE_DENSITY: u32 = 36;
    pub const FOLIAGE: u32 = 37;
    pub const REVERB: u32 = 38;
    pub const EAR_HEIGHT: u32 = 39;
    pub const HRTF: u32 = 40;
    pub const COUNT: usize = 41;
}

pub mod telemetry {
    pub const DSP_LOAD: u32 = 0;
    pub const GUST: u32 = 1;
    pub const SAMPLE_RATE: u32 = 2;
    pub const BUBBLE_RATE: u32 = 3;
    pub const TIME: u32 = 4;
    pub const OUTPUT_LEVEL: u32 = 5;
    /// Actual rain rate (smoothed, fluctuating), mm/h.
    pub const RAIN_RATE: u32 = 8;
    /// Leaf wetness 0..1 (canopy water store / capacity).
    pub const WETNESS: u32 = 9;
    /// Lightning flash 0..1 (jumps to 1 on a strike, decays in ~0.2 s; thunder follows r/c later).
    pub const LIGHTNING: u32 = 10;
    /// Canopy drip rate, mm/h.
    pub const DRIP: u32 = 11;
    pub const COUNT: usize = 12;
}

#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SourceKind {
    Water = 0,
    Wind = 1,
    Bird = 2,
    Rain = 3,
    Thunder = 4,
    Fire = 5,
    Insect = 6,
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

pub(crate) struct AtomicF32(AtomicU32);

impl AtomicF32 {
    pub(crate) fn new(v: f32) -> Self {
        Self(AtomicU32::new(v.to_bits()))
    }
    #[inline]
    pub(crate) fn load(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Relaxed))
    }
    #[inline]
    pub(crate) fn store(&self, v: f32) {
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
    objects: ObjectBridge,
    controls: ControlBank,
    /// Live waveform taps, see `scope`.
    scope: Box<[AtomicF32]>,
    scope_pos: AtomicUsize,
}

struct Dsp {
    sample_rate: f32,
    world: World,
    director: Director,
    acoustics: Acoustics,
    wind: WindField,
    rng: Rng,
    bus: [[f32; BLOCK]; CHANNELS],
    scratch: [f32; BLOCK],
    left: [f32; BLOCK],
    right: [f32; BLOCK],
    mono: [f32; BLOCK],
    fade: f32,
    volume: f32,
    time: f64,
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    load: f32,
    out_level: f32,
    /// Control values last applied to the generators.
    ctl: ControlState,
    ctl_applied: bool,
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
        Self::with_scene(sample_rate, &SceneDesc::default_scene()).expect("built-in scene is valid")
    }

    /// Engine running the scene in `json` (see docs/director.md).
    pub fn from_json(sample_rate: f32, json: &str) -> Result<Self, String> {
        Self::with_scene(sample_rate, &SceneDesc::from_json(json)?)
    }

    pub fn with_scene(sample_rate: f32, desc: &SceneDesc) -> Result<Self, String> {
        let sr = if sample_rate.is_finite() && sample_rate >= 8000.0 { sample_rate } else { 48_000.0 };
        let rng = Rng::new(0xA5A5_3000 ^ desc.seed);
        let world = World::build(desc, sr)?;
        let director = Director::new(desc, &world);

        let shared = Shared {
            params: atomics(param::COUNT),
            telemetry: atomics(telemetry::COUNT),
            spectrum: atomics(SPECTRUM_RING),
            spectrum_pos: AtomicUsize::new(0),
            sources: atomics(MAX_SOURCES * SOURCE_FIELDS),
            source_count: AtomicUsize::new(0),
            wind_field: atomics(WIND_FIELD_POINTS),
            objects: ObjectBridge::new(&world),
            controls: ControlBank::new(controls::Bases::new(desc, &world), desc),
            scope: atomics(SCOPE_TRACKS * SCOPE_LEN),
            scope_pos: AtomicUsize::new(0),
        };
        let env = &desc.environment;
        shared.params[param::VOLUME as usize].store(0.7);
        shared.params[param::WIND as usize].store(env.initial_wind().unwrap_or(0.45));
        shared.params[param::BIRDS as usize].store(env.birds.unwrap_or(0.6));
        shared.params[param::WATER as usize].store(env.water.unwrap_or(0.6));
        shared.params[param::TIME_OF_DAY as usize].store(env.time_of_day.rem_euclid(24.0));
        shared.params[param::RAIN as usize].store(env.initial_rain_mm_h());
        shared.params[param::TEMPERATURE as usize].store(env.temperature_c);
        shared.params[param::HUMIDITY as usize].store(env.humidity);
        shared.params[param::STORM as usize].store(env.storm);
        shared.params[param::NEURAL_BIRDS as usize].store(1.0);
        shared.telemetry[telemetry::SAMPLE_RATE as usize].store(sr);
        let env = Acoustic::default();
        for (id, v) in [
            (param::PRESSURE, env.pressure_kpa),
            (param::GROUND, env.ground as u32 as f32),
            (param::TREE_DENSITY, env.tree_density_per_ha),
            (param::FOLIAGE, env.foliage),
            (param::REVERB, env.reverb),
            (param::EAR_HEIGHT, env.ear_height),
            (param::HRTF, env.hrtf as u32 as f32),
        ] {
            shared.params[id as usize].store(v);
        }

        let acoustics = Acoustics::new(sr, world.emitters.len());
        let dsp = Dsp {
            sample_rate: sr,
            world,
            director,
            acoustics,
            wind: WindField::new(sr),
            rng,
            bus: [[0.0; BLOCK]; CHANNELS],
            scratch: [0.0; BLOCK],
            left: [0.0; BLOCK],
            right: [0.0; BLOCK],
            mono: [0.0; BLOCK],
            fade: 0.0,
            volume: 0.7,
            time: 0.0,
            load: 0.0,
            out_level: 0.0,
            ctl: ControlState::default(),
            ctl_applied: false,
        };
        dsp.publish(&shared);
        Ok(Self { shared, dsp: UnsafeCell::new(dsp) })
    }

    pub fn set_param(&self, id: u32, value: f32) {
        if let Some(p) = self.shared.params.get(id as usize) {
            if value.is_finite() {
                p.store(value);
                self.shared.controls.param_set(id);
            }
        }
    }

    /// Copies the control tree (macros and their physical children) into `out`, returns how
    /// many were written. Any thread.
    pub fn controls(&self, out: &mut [ControlInfo]) -> usize {
        self.shared.controls.read(&self.shared.params, out)
    }

    /// Sets a control (NaN on a child: follow the macro again). Any thread; applied by the
    /// audio thread on the next block.
    pub fn set_control(&self, id: u32, value: f32) {
        self.shared.controls.set(id, value, &self.shared.params);
    }

    /// Effective value of one control.
    pub fn control(&self, id: u32) -> f32 {
        self.shared.controls.state(&self.shared.params).get(id)
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

    /// Copies scene objects (trees, rivers, birds) into `out`, returns how many were written.
    pub fn objects(&self, out: &mut [ObjectInfo]) -> usize {
        self.shared.objects.read(out)
    }

    /// Moves a user-movable object (applied by the audio thread at the next block).
    pub fn set_object_position(&self, id: u32, x: f32, y: f32, z: f32) {
        self.shared.objects.request_move(id, Vec3::new(x, y, z));
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

    /// The newest `out.len()` peaks of a scope track, oldest first. Returns how many are real
    /// (the rest, at the start, are zero).
    pub fn scope(&self, track: u32, out: &mut [f32]) -> usize {
        let t = track as usize;
        if t >= SCOPE_TRACKS {
            out.fill(0.0);
            return 0;
        }
        let pos = self.shared.scope_pos.load(Ordering::Acquire);
        let n = out.len().min(SCOPE_LEN).min(pos);
        let pad = out.len() - n;
        out[..pad].fill(0.0);
        for k in 0..n {
            let idx = (pos - n + k) % SCOPE_LEN;
            out[pad + k] = self.shared.scope[t * SCOPE_LEN + idx].load();
        }
        n
    }
}

/// Track indices of `Engine::scope`; mirror ASMR_SCOPE_* in asmr.h.
pub mod scope {
    pub const WATER: usize = 0;
    pub const WIND: usize = 1;
    pub const BIRDS: usize = 2;
    /// Birds in flight.
    pub const FLIGHT: usize = 3;
    pub const OUTPUT: usize = 4;
    pub const RAIN: usize = 5;
    pub const INSECTS: usize = 6;
    pub const FIRE: usize = 7;
    pub const THUNDER: usize = 8;
}

fn push_scope(shared: &Shared, peaks: &[f32; SCOPE_TRACKS]) {
    let pos = shared.scope_pos.load(Ordering::Relaxed);
    let i = pos % SCOPE_LEN;
    for (t, p) in peaks.iter().enumerate() {
        shared.scope[t * SCOPE_LEN + i].store(*p);
    }
    shared.scope_pos.store(pos.wrapping_add(1), Ordering::Release);
}

impl Dsp {
    fn process_block(&mut self, shared: &Shared, out_l: &mut [f32], out_r: &mut [f32]) {
        let len = out_l.len();
        let sr = self.sample_rate;
        let dt = len as f32 / sr;
        let p = |k: u32| shared.params[k as usize].load();

        let playing = p(param::PLAYING) > 0.5;
        if !playing && self.fade < 1e-4 {
            // Paused: the scene is frozen, but user moves still land.
            shared.objects.apply_moves(&mut self.world);
            self.world.update_emitters();
            self.fade = 0.0;
            out_l.fill(0.0);
            out_r.fill(0.0);
            self.mono[..len].fill(0.0);
            self.out_level = 0.0;
            push_scope(shared, &[0.0; SCOPE_TRACKS]);
            return;
        }

        // Controls: macros and their physical children -> generator configs (only on change).
        let st = shared.controls.state(&shared.params);
        if !self.ctl_applied || st != self.ctl {
            controls::apply(&mut self.world, &st, &shared.controls.bases);
            let w = self.director.weather_mut();
            w.set_time_scale(st.get(controls::id::RAIN_DRYING));
            w.set_season(controls::season_of(st.get(controls::id::ENV_SEASON)));
            self.ctl = st;
            self.ctl_applied = true;
        }
        shared.controls.publish_params(&st, &shared.params);
        let gustiness = st.get(controls::id::WIND_GUSTINESS);

        let wind = p(param::WIND).clamp(0.0, 1.0);
        let birds = p(param::BIRDS).clamp(0.0, 1.0);
        let neural = p(param::NEURAL_BIRDS) > 0.5;
        let water = p(param::WATER).clamp(0.0, 1.0);
        let orbit = p(param::ORBIT).clamp(0.0, 1.0);
        let volume = p(param::VOLUME).clamp(0.0, 1.5);
        let head = Mat3::from_head(p(param::HEAD_YAW), p(param::HEAD_PITCH), p(param::HEAD_ROLL));

        self.wind.step(wind, dt, &mut self.rng);
        // Scene: user moves in, director moves the animals, emitters follow their owners.
        shared.objects.apply_moves(&mut self.world);
        let env = Environment {
            hour: p(param::TIME_OF_DAY).rem_euclid(24.0),
            birds,
            rain: 0.0,
            orbit,
            wind,
            rain_mm_h: p(param::RAIN).clamp(0.0, 200.0),
            temperature_c: p(param::TEMPERATURE).clamp(-40.0, 50.0),
            humidity: p(param::HUMIDITY).clamp(0.0, 1.0),
            storm: p(param::STORM).clamp(0.0, 1.0),
        };
        self.director.step(dt, &env, &mut self.world);
        self.world.update_emitters();
        self.acoustics.set_environment(&Acoustic {
            temperature_c: env.temperature_c,
            humidity_pct: env.humidity * 100.0,
            pressure_kpa: p(param::PRESSURE),
            ground: GroundType::from_index(p(param::GROUND).max(0.0) as u32),
            tree_density_per_ha: p(param::TREE_DENSITY),
            foliage: p(param::FOLIAGE),
            reverb: p(param::REVERB),
            ear_height: p(param::EAR_HEIGHT),
            hrtf: if p(param::HRTF) > 0.5 { HrtfModel::Analytic } else { HrtfModel::Measured },
        });

        for ch in self.bus.iter_mut() {
            ch[..len].fill(0.0);
        }
        let inv_len = 1.0 / len as f32;
        let mut peaks = [0.0f32; SCOPE_TRACKS];

        for (idx, em) in self.world.emitters.iter_mut().enumerate() {
            let tmp = &mut self.scratch[..len];
            let mut controls = em.controls;
            controls.water = water * em.flow;
            controls.neural = neural;
            controls.gust = (wind + (self.wind.local(em.position.y) - wind) * gustiness).max(0.0);
            em.generator.render(tmp, &controls);
            let kind_gain = match em.kind {
                SourceKind::Water => WATER_GAIN,
                SourceKind::Wind => WIND_GAIN,
                SourceKind::Bird => BIRD_GAIN,
                SourceKind::Rain => RAIN_GAIN,
                SourceKind::Thunder => THUNDER_GAIN,
                SourceKind::Fire => FIRE_GAIN,
                SourceKind::Insect => INSECT_GAIN,
            };

            let energy: f32 = tmp.iter().map(|v| v * v).sum();
            em.level = em.level * 0.8 + (energy * inv_len).sqrt() * 0.2;
            let peak = tmp.iter().fold(0.0f32, |m, v| m.max(v.abs())) * kind_gain;
            let track = match em.kind {
                SourceKind::Water => scope::WATER,
                SourceKind::Wind => scope::WIND,
                SourceKind::Bird => scope::BIRDS,
                SourceKind::Rain => scope::RAIN,
                SourceKind::Thunder => scope::THUNDER,
                SourceKind::Fire => scope::FIRE,
                SourceKind::Insect => scope::INSECTS,
            };
            peaks[track] = peaks[track].max(peak);
            if em.kind == SourceKind::Bird && self.world.entities[em.owner].flags & object_flags::MOVING != 0 {
                peaks[scope::FLIGHT] = peaks[scope::FLIGHT].max(peak);
            }

            // Propagation: delay/Doppler, spreading, air, ground, directivity, occlusion,
            // forest scattering, near field and reverb sends (propagation.rs).
            em.rel = head.apply_transposed(em.position);
            let mut emitter = Emitter::at(em.position, kind_gain);
            if em.kind == SourceKind::Water {
                emitter.ground = Some(GroundType::Water);
            }
            self.acoustics.propagate(idx, &emitter, tmp, &mut self.bus);
        }

        self.acoustics.render(&mut self.bus, &head, len, &mut self.left[..len], &mut self.right[..len]);

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
        peaks[scope::OUTPUT] = self.mono[..len].iter().fold(0.0f32, |m, v| m.max(v.abs()));
        push_scope(shared, &peaks);
        self.time += dt as f64;
    }

    fn publish(&self, shared: &Shared) {
        let count = self.world.emitters.len().min(MAX_SOURCES);
        for (i, s) in self.world.emitters.iter().take(count).enumerate() {
            let freq = s.generator.pitch();
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

        let bubble_rate: f32 = self.world.emitters.iter().map(|s| s.generator.event_rate()).sum();
        shared.objects.publish(&self.world);
        let t = &shared.telemetry;
        t[telemetry::GUST as usize].store(self.wind.gust());
        t[telemetry::BUBBLE_RATE as usize].store(bubble_rate);
        t[telemetry::TIME as usize].store((self.time % 86_400.0) as f32);
        t[telemetry::OUTPUT_LEVEL as usize].store(self.out_level);
        let w = self.director.weather();
        t[telemetry::RAIN_RATE as usize].store(w.rain_mm_h());
        t[telemetry::WETNESS as usize].store(w.leaf_wetness());
        t[telemetry::LIGHTNING as usize].store(w.flash());
        t[telemetry::DRIP as usize].store(w.drip_mm_h());
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
        assert_eq!(engine.sources(&mut src), 43);
        let mut spec = [0.0f32; 32];
        engine.spectrum(&mut spec);
        assert!(spec.iter().any(|v| *v > 0.0));
    }

    #[test]
    fn neural_birds_sing_in_the_scene() {
        let engine = Engine::new(48_000.0);
        assert_eq!(engine.param(param::NEURAL_BIRDS), 1.0);
        engine.set_param(param::PLAYING, 1.0);
        engine.set_param(param::BIRDS, 1.0);
        let (mut l, mut r) = (vec![0.0f32; 512], vec![0.0f32; 512]);
        let mut sang = false;
        for _ in 0..(48_000 * 20 / 512) {
            engine.render(&mut l, &mut r);
            assert!(l.iter().chain(&r).all(|v| v.is_finite() && v.abs() <= 1.0));
            let mut src = [SourceInfo::default(); MAX_SOURCES];
            let n = engine.sources(&mut src);
            sang |= src[..n].iter().any(|s| s.kind == SourceKind::Bird as u32 && s.freq > 500.0);
        }
        assert!(sang, "no bird sang in 20 s");
    }

    #[test]
    fn scope_follows_output() {
        let engine = Engine::new(48_000.0);
        engine.set_param(param::PLAYING, 1.0);
        let (mut l, mut r) = (vec![0.0f32; 4800], vec![0.0f32; 4800]);
        engine.render(&mut l, &mut r);
        let mut scope = [0.0f32; 64];
        assert_eq!(engine.scope(scope::OUTPUT as u32, &mut scope), 64);
        assert!(scope.iter().any(|v| *v > 0.0));
        assert_eq!(engine.scope(SCOPE_TRACKS as u32, &mut scope), 0);
    }

    #[test]
    fn objects_report_and_move() {
        use crate::scene::{object_flags, ObjectKind};
        let engine = Engine::new(48_000.0);
        assert_eq!(engine.param(param::TIME_OF_DAY), 9.0);
        let mut objs = [ObjectInfo::default(); 64];
        let n = engine.objects(&mut objs);
        assert_eq!(n, 31);
        let tree = objs[..n].iter().find(|o| o.id == 2).unwrap();
        assert_eq!(tree.kind, ObjectKind::Tree as u32);
        assert_ne!(tree.flags & object_flags::MOVABLE, 0);
        engine.set_object_position(2, 0.0, 10.0, -1.7);
        let (mut l, mut r) = ([0.0f32; 256], [0.0f32; 256]);
        engine.render(&mut l, &mut r);
        engine.objects(&mut objs);
        let tree = objs[..n].iter().find(|o| o.id == 2).unwrap();
        assert_eq!((tree.x, tree.y, tree.z), (0.0, 10.0, -1.7));
        // Birds cannot be dragged.
        let bird = *objs[..n].iter().find(|o| o.kind == ObjectKind::Bird as u32).unwrap();
        engine.set_object_position(bird.id, 0.0, 0.0, 0.0);
        engine.render(&mut l, &mut r);
        engine.objects(&mut objs);
        let after = objs[..n].iter().find(|o| o.id == bird.id).unwrap();
        assert!((after.x, after.y, after.z) != (0.0, 0.0, 0.0));
        assert!(Engine::from_json(48_000.0, "{\"name\":1}").is_err());
    }

    #[test]
    fn rain_param_reaches_the_rain_emitters() {
        let engine = Engine::new(48_000.0);
        engine.set_param(param::PLAYING, 1.0);
        engine.set_param(param::RAIN, 12.0);
        let (mut l, mut r) = (vec![0.0f32; 512], vec![0.0f32; 512]);
        for _ in 0..(48_000 * 6 / 512) {
            engine.render(&mut l, &mut r);
            assert!(l.iter().chain(&r).all(|v| v.is_finite() && v.abs() <= 1.0));
        }
        assert!(engine.telemetry(telemetry::RAIN_RATE) > 6.0);
        let mut src = [SourceInfo::default(); MAX_SOURCES];
        let n = engine.sources(&mut src);
        let rain = src[..n].iter().filter(|s| s.kind == SourceKind::Rain as u32 && s.level > 1e-4).count();
        assert!(rain >= 10, "{rain} rain emitters audible");
    }

    #[test]
    fn controls_tree_and_macros() {
        use crate::controls::{flags, id, NO_PARENT};
        let engine = Engine::new(48_000.0);
        let mut c = [ControlInfo::default(); 64];
        let n = engine.controls(&mut c);
        assert_eq!(n, crate::controls::COUNT);
        let ids: Vec<u32> = c[..n].iter().map(|x| x.id).collect();
        for x in &c[..n] {
            assert!(!x.key_str().is_empty() && x.min <= x.value && x.value <= x.max, "{}", x.key_str());
            assert_eq!(ids.iter().filter(|&&i| i == x.id).count(), 1);
            if x.parent == NO_PARENT {
                assert_ne!(x.flags & flags::MACRO, 0);
            } else {
                let p = c[..n].iter().find(|p| p.id == x.parent).unwrap();
                assert_ne!(p.flags & flags::MACRO, 0);
                assert_eq!(p.group, x.group);
            }
        }
        // Legacy params are the macros.
        engine.set_param(param::WATER, 0.3);
        assert_eq!(engine.control(id::WATER_FLOW), 0.3);
        engine.set_control(id::WIND_SPEED, 10.0);
        assert!((crate::sources::common::wind_speed_ms(engine.param(param::WIND)) - 10.0).abs() < 1e-3);
        // Children follow the macro along hydraulic geometry...
        let slow = engine.control(id::WATER_SPEED);
        engine.set_control(id::WATER_FLOW, 0.9);
        let fast = engine.control(id::WATER_SPEED);
        assert!(fast > slow && engine.control(id::WATER_DEPTH) > 0.1);
        // ...unless the user set them; NaN hands them back.
        engine.set_control(id::WATER_SPEED, 0.5);
        engine.set_control(id::WATER_FLOW, 0.2);
        assert_eq!(engine.control(id::WATER_SPEED), 0.5);
        engine.set_control(id::WATER_SPEED, f32::NAN);
        assert!(engine.control(id::WATER_SPEED) < 0.5);
        // Storm follows heavy rain unless set explicitly.
        engine.set_control(id::RAIN_RATE, 40.0);
        assert!(engine.control(id::RAIN_STORM) > 0.8);
        engine.set_param(param::STORM, 0.0);
        assert_eq!(engine.control(id::RAIN_STORM), 0.0);
        // Presets are discrete children with names.
        assert_eq!(crate::controls::option_name(id::WATER_REGIME, 0), Some("scene"));
        assert_eq!(crate::controls::option_name(id::WATER_REGIME, 6), Some("rapids"));
    }

    #[test]
    fn water_controls_change_the_sound() {
        use crate::controls::id;
        let rms = |regime: f32, flow: f32| {
            let engine = Engine::new(48_000.0);
            engine.set_param(param::PLAYING, 1.0);
            engine.set_param(param::BIRDS, 0.0);
            engine.set_param(param::WIND, 0.0);
            engine.set_control(id::WATER_REGIME, regime);
            engine.set_control(id::WATER_FLOW, flow);
            let (mut l, mut r) = (vec![0.0f32; 512], vec![0.0f32; 512]);
            let mut e = 0.0f64;
            for i in 0..(48_000 * 4 / 512) {
                engine.render(&mut l, &mut r);
                if i > 90 {
                    e += l.iter().map(|v| (*v as f64).powi(2)).sum::<f64>();
                }
            }
            e
        };
        let brook = rms(0.0, 0.6);
        assert!(rms(0.0, 1.0) > 1.5 * brook);
        assert!(rms(0.0, 0.0) < 0.2 * brook);
        assert!(rms(6.0, 0.6) > 10.0 * brook, "rapids");
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
