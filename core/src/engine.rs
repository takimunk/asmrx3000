//! The real-time engine: scene sources -> propagation -> 3rd-order Ambisonic bus ->
//! head rotation -> binaural.
//!
//! Threading contract: `render` is called from exactly one (audio) thread. Everything else
//! (params, telemetry, spectrum) may be called from any thread; it only touches atomics.

use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, AtomicU64, AtomicUsize, Ordering};
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

use core::f32::consts::TAU;

use crate::bed;
use crate::acoustics::{Acoustics, Emitter, Environment as Acoustic, HrtfModel, Trunk};
use crate::ambisonics::CHANNELS;
use crate::controls::{self, ControlBank, ControlInfo, ControlState};
use crate::director::{Director, Environment};
use crate::dsp::soft_clip;
use crate::fft::fft;
use crate::flight::{self, bridge::FlightBridge, Flight};
use crate::headpose::{HeadPoseInput, HeadTracker, PoseSettings, Quat};
use crate::math::Vec3;
use crate::mix::{self, EmitterView, Intent, Mix, MixInfo, MixShared};
use crate::physics::ground::GroundType;
use crate::prompt::{self, Glides};
use crate::rng::Rng;
use crate::scene::{object_flags, Geometry, ObjectBridge, ObjectInfo, SceneDesc, World, MAX_OBJECTS};
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

// Source levels come from the calibration table (mix/calibration.rs); the mix layer
// (mix/mod.rs) adds intent, attention, masking-aware ducking and output dynamics.

/// Scope track of a source kind (shared with flight mode).
pub(crate) fn scope_track(kind: SourceKind) -> usize {
    match kind {
        SourceKind::Water => scope::WATER,
        SourceKind::Wind => scope::WIND,
        SourceKind::Bird => scope::BIRDS,
        SourceKind::Rain => scope::RAIN,
        SourceKind::Thunder => scope::THUNDER,
        SourceKind::Fire => scope::FIRE,
        SourceKind::Insect => scope::INSECTS,
    }
}

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
    // Head tracking (headpose.rs).
    /// Render-to-ear delay the head pose is predicted over, seconds (host-measured).
    pub const OUTPUT_LATENCY: u32 = 41;
    /// 0 = render the last measured pose, 1 = full latency compensation.
    pub const HEAD_PREDICTION: u32 = 42;
    /// 1 = the head turns about the neck (ears translate: motion parallax), 0 = pure rotation.
    pub const HEAD_PARALLAX: u32 = 43;
    /// Flight mode on (1) / off (0): cross-fades the scene into an endless streamed landscape
    /// the listener travels through (flight/, docs/flight-mode.md).
    pub const FLIGHT: u32 = 44;
    pub const COUNT: usize = 45;
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
    /// Head-pose prediction horizon used in the last block, ms.
    pub const HEAD_HORIZON_MS: u32 = 12;
    /// Head-pose samples per second arriving from the host.
    pub const HEAD_RATE: u32 = 13;
    /// Flight mode cross-fade 0 (scene) .. 1 (flying).
    pub const FLIGHT: u32 = 14;
    /// Current flight speed, m/s (ramps to the flight.speed control).
    pub const FLIGHT_SPEED: u32 = 15;
    /// Ears above the ground, m.
    pub const FLIGHT_ALTITUDE: u32 = 16;
    /// Distance travelled along the path, m (wraps at 10 km).
    pub const FLIGHT_ODOMETER: u32 = 17;
    /// Turn rate of the course, rad/s (+ = turning left).
    pub const FLIGHT_TURN: u32 = 18;
    /// Heading of the course in the landscape, rad (0..2π, counter-clockwise).
    pub const FLIGHT_HEADING: u32 = 19;
    /// Position in the landscape, m (wrapping at 4096 m: for scrolling ground textures).
    pub const FLIGHT_X: u32 = 20;
    pub const FLIGHT_Y: u32 = 21;
    /// Mix loudness after the mix dynamics, before VOLUME: momentary (400 ms), short-term
    /// (3 s) and gated integrated (BS.1770), LUFS.
    pub const LOUDNESS_MOMENTARY: u32 = 22;
    pub const LOUDNESS_SHORT: u32 = 23;
    pub const LOUDNESS_INTEGRATED: u32 = 24;
    /// Mix dynamics gain (leveller + comfort compressor + limiter), dB.
    pub const MIX_GAIN_DB: u32 = 25;
    /// Foreground soloists right now.
    pub const SOLOISTS: u32 = 26;
    /// Share of the soloists' loudness lost to masking by the bed, 0..1 (after ducking).
    pub const MASKING: u32 = 27;
    pub const COUNT: usize = 28;
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
    /// Prompt control: in-flight glides and object labels (core/src/prompt.rs).
    glides: Glides,
    labels: Vec<String>,
    /// Live waveform taps, see `scope`.
    scope: Box<[AtomicF32]>,
    scope_pos: AtomicUsize,
    /// Flight mode, built off the audio thread on first use and handed over here.
    flight_inbox: AtomicPtr<Flight>,
    flight_requested: AtomicU32,
    /// 1 while the streamed objects (not the scene's) are reported.
    flight_shown: AtomicU32,
    flight_objects: FlightBridge,
    /// Where the listener is in the landscape (UI terrain), and whether it was published.
    flight_frame: crate::flight::bridge::FrameBridge,
    flight_framed: AtomicU32,
    /// Bird telemetry (`asmr_get_birds`).
    birds: crate::director::BirdBridge,
    seed: u64,
    head: HeadPoseInput,
    /// Seconds of audio rendered (f64 bits): the clock head-pose arrivals are stamped with.
    clock: AtomicU64,
    /// Mix-layer telemetry (`asmr_get_mix`).
    mix: MixShared,
}

struct Dsp {
    sample_rate: f32,
    world: World,
    director: Director,
    acoustics: Acoustics,
    scene_trees: Vec<Trunk>,
    scene_geometry_initialized: bool,
    head: HeadTracker,
    /// 0 = binaural (our decoder), 1 = 7.1.4 bed rotated by our head pose, 2 = world-locked bed.
    bed_mode: u8,
    bed: [[f32; BLOCK]; bed::CHANNELS],
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
    /// The sound director's mix layer.
    mix: Mix,
    gains: [f32; BLOCK],
    flight: Option<Box<Flight>>,
    /// Cross-fade scene -> flight, 0..1.
    flight_mix: f32,
}

pub struct Engine {
    shared: Shared,
    rendering: AtomicBool,
    dsp: UnsafeCell<Dsp>,
}

// SAFETY: render/render_bed acquire `rendering` before accessing `dsp`. Overlapping calls
// return silence without waiting. Everything reachable from other threads is atomic.
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
            glides: Glides::new(),
            labels: prompt::labels(&world),
            scope: atomics(SCOPE_TRACKS * SCOPE_LEN),
            scope_pos: AtomicUsize::new(0),
            flight_inbox: AtomicPtr::new(std::ptr::null_mut()),
            flight_requested: AtomicU32::new(0),
            flight_shown: AtomicU32::new(0),
            flight_objects: FlightBridge::new(),
            flight_frame: crate::flight::bridge::FrameBridge::new(),
            flight_framed: AtomicU32::new(0),
            birds: crate::director::BirdBridge::new(),
            seed: desc.seed,
            head: HeadPoseInput::new(),
            clock: AtomicU64::new(0),
            mix: MixShared::new(),
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
            (param::EAR_HEIGHT, -world.ground_z),
            (param::HRTF, env.hrtf as u32 as f32),
            (param::OUTPUT_LATENCY, 0.15),
            (param::HEAD_PREDICTION, 1.0),
            (param::HEAD_PARALLAX, 1.0),
        ] {
            shared.params[id as usize].store(v);
        }

        let mut acoustics = Acoustics::new(sr, world.emitters.len());
        acoustics.set_environment(&Acoustic {
            temperature_c: desc.environment.temperature_c,
            humidity_pct: desc.environment.humidity * 100.0,
            ear_height: -world.ground_z,
            ..Acoustic::default()
        });
        let mut dsp = Dsp {
            sample_rate: sr,
            world,
            director,
            acoustics,
            scene_trees: Vec::with_capacity(MAX_OBJECTS),
            scene_geometry_initialized: false,
            head: HeadTracker::new(),
            bed_mode: 0,
            bed: [[0.0; BLOCK]; bed::CHANNELS],
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
            mix: Mix::new(sr),
            gains: [1.0; BLOCK],
            flight: None,
            flight_mix: 0.0,
        };
        dsp.sync_scene_trees();
        dsp.publish(&shared);
        let mut engine = Self { shared, rendering: AtomicBool::new(false), dsp: UnsafeCell::new(dsp) };
        if let Some(f) = &desc.flight {
            if f.apply(&engine)? {
                // Starts in the air, at cruising speed: no cross-fade from the scene.
                let ptr = engine.shared.flight_inbox.swap(std::ptr::null_mut(), Ordering::AcqRel);
                let dsp = engine.dsp.get_mut();
                if !ptr.is_null() {
                    // SAFETY: produced by Box::into_raw in `prepare_flight`, taken once.
                    let mut pack = unsafe { Box::from_raw(ptr) };
                    pack.cruise(engine.shared.controls.state(&engine.shared.params).get(controls::id::FLIGHT_SPEED));
                    dsp.flight = Some(pack);
                }
                dsp.flight_mix = 1.0;
            }
        }
        Ok(engine)
    }

    /// Builds flight mode (once) on the calling thread and hands it to the audio thread.
    /// Allocates; called from `set_param(FLIGHT, 1)`, i.e. the UI thread.
    pub fn prepare_flight(&self) {
        if self.shared.flight_requested.swap(1, Ordering::AcqRel) != 0 {
            return;
        }
        let sr = self.shared.telemetry[telemetry::SAMPLE_RATE as usize].load();
        let pack = Box::new(Flight::new(sr, self.shared.seed ^ 0xF1_6000));
        self.shared.flight_inbox.store(Box::into_raw(pack), Ordering::Release);
    }

    /// Where the listener is in the flight landscape and what it is (None before flight mode
    /// was first built). Any thread.
    pub fn flight_frame(&self) -> Option<crate::flight::bridge::Frame> {
        (self.shared.flight_framed.load(Ordering::Acquire) != 0).then(|| self.shared.flight_frame.read())
    }

    /// Shape of a streamed object (river bed, lake shore, ridge line) as listener-relative
    /// (x, y, z) triples; returns the number of points. Any thread.
    pub fn object_path(&self, id: u32, out: &mut [f32]) -> usize {
        if self.shared.flight_shown.load(Ordering::Acquire) == 0 {
            return 0;
        }
        self.shared.flight_objects.path(id, out)
    }

    /// Head orientation from a motion sensor. Any thread. `q` (w, x, y, z): head -> world rotation in
    /// the engine frame (+x forward, +y left, +z up), already recentred; `w`: angular velocity in
    /// the head frame, rad/s; `age`: how long ago (s) the sensor measured it. Overrides the
    /// HEAD_YAW/PITCH/ROLL params from the first call on.
    pub fn set_head_pose(&self, q: [f32; 4], w: [f32; 3], age: f32) {
        if q.iter().chain(&w).all(|v| v.is_finite()) && age.is_finite() {
            let now = f64::from_bits(self.shared.clock.load(Ordering::Relaxed));
            let q = Quat { w: q[0], x: q[1], y: q[2], z: q[3] };
            self.shared.head.set(q, Vec3::new(w[0], w[1], w[2]), age.clamp(0.0, 1.0), now);
        }
    }

    pub fn set_param(&self, id: u32, value: f32) {
        let guard = self.shared.glides.edit();
        if let Some(p) = self.shared.params.get(id as usize) {
            if value.is_finite() {
                p.store(value);
                self.shared.controls.param_set(id);
                for i in controls::indices_of_param(id) {
                    self.shared.glides.cancel(i);
                }
            }
        }
        drop(guard);
        if id == param::FLIGHT && value.is_finite() && value > 0.5 {
            self.prepare_flight();
        }
    }

    /// Copies the control tree (macros and their physical children) into `out`, returns how
    /// many were written. Any thread.
    pub fn controls(&self, out: &mut [ControlInfo]) -> usize {
        self.shared.glides.tick(&self.shared.controls, &self.shared.params);
        self.shared.controls.read(&self.shared.params, out)
    }

    /// Sets a control (NaN on a child: follow the macro again). Any thread; applied by the
    /// audio thread on the next block.
    pub fn set_control(&self, id: u32, value: f32) {
        let _guard = self.shared.glides.edit();
        if let Some(i) = controls::index_of_id(id) {
            self.shared.glides.cancel(i);
        }
        self.shared.controls.set(id, value, &self.shared.params);
    }

    fn prompt_view(&self) -> prompt::View<'_> {
        let s = &self.shared;
        let mut sources = vec![SourceInfo::default(); MAX_SOURCES];
        let n = self.sources(&mut sources);
        sources.truncate(n);
        let mut objects = vec![ObjectInfo::default(); MAX_OBJECTS];
        let n = self.objects(&mut objects);
        objects.truncate(n);
        let live = prompt::Live { sources, objects, mix: self.mix_info() };
        prompt::View { bank: &s.controls, params: &s.params, telemetry: &s.telemetry, glides: &s.glides, objects: &s.objects, labels: &s.labels, live }
    }

    /// Controls, environment readings and scene objects as JSON (docs/prompt-control.md).
    pub fn state_json(&self) -> String {
        prompt::state_json(&self.prompt_view())
    }

    /// Applies a JSON patch of control targets (gliding) and object moves; returns the
    /// result JSON with errors and an undo patch (docs/prompt-control.md).
    pub fn apply_patch_json(&self, patch: &str) -> String {
        let (result, params) = prompt::apply_patch(&self.prompt_view(), patch);
        // Switches (flight on/off) are engine parameters, set after the control transaction.
        for (id, value) in params {
            self.set_param(id, value);
        }
        result
    }

    /// System prompt, current-state context and tool schema for a language model.
    pub fn prompt_json(&self) -> String {
        prompt::prompt_json(&self.prompt_view())
    }

    /// Effective values of every control (tests).
    #[cfg(test)]
    pub(crate) fn shared_state(&self) -> ControlState {
        self.shared.controls.state(&self.shared.params)
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
        let Some(_guard) = self.begin_render() else {
            left.fill(0.0);
            right.fill(0.0);
            return;
        };
        #[cfg(not(target_arch = "wasm32"))]
        let started = Instant::now();
        // SAFETY: the render guard grants exclusive access to the DSP state.
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

    /// Audio thread only, instead of `render`: renders a 7.1.4 loudspeaker bed (channel order
    /// `bed::LAYOUT`, L R C LFE Ls Rs Rls Rrs Vhl Vhr Ltr Rtr) for an external binaural renderer.
    /// `world_locked`: skip our head rotation (the renderer applies AirPods tracking itself).
    pub fn render_bed(&self, out: &mut [&mut [f32]], world_locked: bool) {
        let Some(_guard) = self.begin_render() else {
            for channel in out { channel.fill(0.0); }
            return;
        };
        // SAFETY: the render guard grants exclusive access to the DSP state.
        let dsp = unsafe { &mut *self.dsp.get() };
        dsp.bed_mode = if world_locked { 2 } else { 1 };
        let n = out.iter().map(|c| c.len()).min().unwrap_or(0);
        let mut l = [0.0f32; BLOCK];
        let mut r = [0.0f32; BLOCK];
        let mut off = 0;
        while off < n {
            let len = (n - off).min(BLOCK);
            dsp.process_block(&self.shared, &mut l[..len], &mut r[..len]);
            for (ch, o) in out.iter_mut().enumerate() {
                match dsp.bed.get(ch) {
                    Some(b) => o[off..off + len].copy_from_slice(&b[..len]),
                    None => o[off..off + len].fill(0.0),
                }
            }
            off += len;
        }
        dsp.bed_mode = 0;
        dsp.publish(&self.shared);
    }

    fn begin_render(&self) -> Option<RenderGuard<'_>> {
        self.rendering.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .ok().map(|_| RenderGuard(&self.rendering))
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

    /// Mix-layer telemetry: loudness, per-group levels and masking, soloists. Any thread.
    pub fn mix_info(&self) -> MixInfo {
        self.shared.mix.load()
    }

    /// Role of emitter `idx` in the mix right now (tests, tools). Not while rendering.
    pub fn mix_role(&self, idx: usize) -> mix::Role {
        // SAFETY: tools call this between renders on the render thread.
        unsafe { &*self.dsp.get() }.mix.role(idx)
    }

    /// A/B tool: `true` = the mix before the sound director (legacy kind gains, no attention,
    /// ducking or dynamics; analysis still runs). Call between renders, on the render thread.
    pub fn set_mix_bypass(&self, on: bool) {
        // SAFETY: as above.
        unsafe { &mut *self.dsp.get() }.mix.set_bypass(on);
    }

    /// Calibration tool: per-emitter A-weighted sound level probes (allocates; between renders).
    pub fn enable_mix_probes(&self) {
        // SAFETY: as above.
        unsafe { &mut *self.dsp.get() }.mix.enable_probes();
    }

    /// Probe results: (class, LAeq at 1 m, LAFmax at 1 m, mean 1/r²) per emitter.
    pub fn mix_probes(&self) -> Vec<(mix::SoundClass, f32, f32, f64)> {
        // SAFETY: as above.
        let m = &unsafe { &*self.dsp.get() }.mix;
        m.probes().iter().map(|p| (p.class, p.meter.laeq(), p.meter.lafmax(), p.inv_r2 / p.blocks.max(1) as f64)).collect()
    }

    /// Starts timing the mix layer; `mix_cpu_s` reads the total. Between renders.
    pub fn enable_mix_timing(&self) {
        // SAFETY: as above.
        unsafe { &mut *self.dsp.get() }.mix.enable_timing();
    }

    pub fn mix_cpu_s(&self) -> f64 {
        // SAFETY: as above.
        unsafe { &*self.dsp.get() }.mix.cpu_s
    }

    /// Restarts the integrated-loudness measurement.
    pub fn reset_loudness(&self) {
        // SAFETY: as above.
        unsafe { &mut *self.dsp.get() }.mix.reset_integrated();
    }

    /// Copies scene objects (trees, rivers, birds) into `out`, returns how many were written.
    pub fn objects(&self, out: &mut [ObjectInfo]) -> usize {
        if self.shared.flight_shown.load(Ordering::Acquire) != 0 {
            return self.shared.flight_objects.read(out);
        }
        self.shared.objects.read(out)
    }

    /// Per-bird behaviour telemetry of the scene (species, state, flight style, wingbeat,
    /// velocity); 0 while flight mode is shown. Any thread.
    pub fn birds(&self, out: &mut [crate::director::BirdInfo]) -> usize {
        if self.shared.flight_shown.load(Ordering::Acquire) != 0 {
            return 0;
        }
        self.shared.birds.read(out)
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
        let mut re = [0.0f32; SPECTRUM_FFT];
        let mut im = [0.0f32; SPECTRUM_FFT];
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

impl Drop for Engine {
    fn drop(&mut self) {
        let p = self.shared.flight_inbox.swap(std::ptr::null_mut(), Ordering::AcqRel);
        if !p.is_null() {
            // SAFETY: produced by Box::into_raw in `prepare_flight` and never taken.
            drop(unsafe { Box::from_raw(p) });
        }
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

struct RenderGuard<'a>(&'a AtomicBool);

impl Drop for RenderGuard<'_> {
    fn drop(&mut self) { self.0.store(false, Ordering::Release); }
}

impl Dsp {
    /// Reuse storage reserved at construction; moving a tree invalidates the path fits.
    fn sync_scene_trees(&mut self) {
        let mut i = 0;
        let mut changed = false;
        for entity in &self.world.entities {
            if let Geometry::Tree { canopy_height, canopy_radius } = entity.geometry {
                // Scene trees store crown dimensions; estimate trunk radius from the crown.
                let trunk = Trunk { x: entity.position.x, y: entity.position.y,
                    radius: (canopy_radius * 0.08).clamp(0.05, 0.6),
                    height: canopy_height + entity.position.z - self.world.ground_z };
                if self.scene_trees.get(i) != Some(&trunk) {
                    changed = true;
                    if i < self.scene_trees.len() { self.scene_trees[i] = trunk; }
                    else { self.scene_trees.push(trunk); }
                }
                i += 1;
            }
        }
        changed |= self.scene_trees.len() != i;
        self.scene_trees.truncate(i);
        if changed || !self.scene_geometry_initialized {
            self.acoustics.update_geometry(&self.scene_trees, &[], true);
            self.scene_geometry_initialized = true;
        }
    }

    fn process_block(&mut self, shared: &Shared, out_l: &mut [f32], out_r: &mut [f32]) {
        let len = out_l.len();
        let sr = self.sample_rate;
        let dt = len as f32 / sr;
        let p = |k: u32| shared.params[k as usize].load();
        shared.glides.advance(dt);

        let playing = p(param::PLAYING) > 0.5;
        if !playing && self.fade < 1e-4 {
            // Paused: the scene is frozen, but user moves still land.
            shared.objects.apply_moves(&mut self.world);
            self.world.update_emitters();
            self.sync_scene_trees();
            self.fade = 0.0;
            out_l.fill(0.0);
            out_r.fill(0.0);
            self.mono[..len].fill(0.0);
            for channel in &mut self.bed { channel[..len].fill(0.0); }
            self.out_level = 0.0;
            push_scope(shared, &[0.0; SCOPE_TRACKS]);
            return;
        }

        // Flight mode: take the pack once it is built; cross-fade over 1.5 s.
        if self.flight.is_none() {
            let ptr = shared.flight_inbox.swap(std::ptr::null_mut(), Ordering::AcqRel);
            if !ptr.is_null() {
                // SAFETY: produced by Box::into_raw in `Engine::prepare_flight`, taken once.
                self.flight = Some(unsafe { Box::from_raw(ptr) });
            }
        }
        let flying = p(param::FLIGHT) > 0.5 && self.flight.is_some();
        let step = if flying { dt / 1.5 } else { -dt / 1.5 };
        self.flight_mix = (self.flight_mix + step).clamp(0.0, 1.0);
        // Fully in flight: the scene's emitters are not rendered at all.
        let scene_emitters = if self.flight_mix >= 1.0 { 0 } else { self.world.emitters.len() };

        // Controls: macros and their physical children -> generator configs (only on change).
        let st = if let Some(_guard) = shared.glides.try_edit() {
            shared.glides.tick_locked(&shared.controls, &shared.params);
            let st = shared.controls.state(&shared.params);
            shared.controls.publish_params(&st, &shared.params);
            st
        } else { self.ctl };
        if !self.ctl_applied || st != self.ctl {
            controls::apply(&mut self.world, &st, &shared.controls.bases);
            let w = self.director.weather_mut();
            w.set_time_scale(st.get(controls::id::RAIN_DRYING));
            w.set_season(controls::season_of(st.get(controls::id::ENV_SEASON)));
            self.ctl = st;
            self.ctl_applied = true;
        }
        self.mix.begin(Intent::from_controls(&st), dt);
        shared.controls.publish_params(&st, &shared.params);
        let gustiness = st.get(controls::id::WIND_GUSTINESS);

        let wind = p(param::WIND).clamp(0.0, 1.0);
        let birds = p(param::BIRDS).clamp(0.0, 1.0);
        let neural = p(param::NEURAL_BIRDS) > 0.5;
        let water = p(param::WATER).clamp(0.0, 1.0);
        let orbit = p(param::ORBIT).clamp(0.0, 1.0);
        let volume = p(param::VOLUME).clamp(0.0, 1.5);
        // Head pose: predicted to the time this block is heard, smoothed, plus neck parallax.
        let settings = PoseSettings {
            output_latency: p(param::OUTPUT_LATENCY).clamp(0.0, 0.5),
            prediction: p(param::HEAD_PREDICTION).clamp(0.0, 1.5),
            parallax: p(param::HEAD_PARALLAX) > 0.5,
        };
        let fallback = (p(param::HEAD_YAW), p(param::HEAD_PITCH), p(param::HEAD_ROLL));
        let pose = self.head.step(&shared.head, fallback, self.time, dt, &settings);
        let head = pose.rotation;
        self.acoustics.set_listener_offset(pose.offset);

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
        self.sync_scene_trees();
        self.acoustics.update_environment(&Acoustic {
            temperature_c: env.temperature_c,
            humidity_pct: env.humidity * 100.0,
            pressure_kpa: p(param::PRESSURE),
            ground: GroundType::from_index(p(param::GROUND).max(0.0) as u32),
            tree_density_per_ha: p(param::TREE_DENSITY),
            foliage: p(param::FOLIAGE),
            reverb: p(param::REVERB),
            ear_height: p(param::EAR_HEIGHT),
            hrtf: if p(param::HRTF) > 0.5 { HrtfModel::Analytic } else { HrtfModel::Measured },
        }, dt);

        for ch in self.bus.iter_mut() {
            ch[..len].fill(0.0);
        }
        let inv_len = 1.0 / len as f32;
        let mut peaks = [0.0f32; SCOPE_TRACKS];

        let listener = pose.offset;
        // Scene focus: where the head points, and whether the listener travels (flight speed;
        // the scene's listener stands still).
        let travel = match self.flight.as_deref() {
            Some(f) if self.flight_mix > 0.5 => f.pose().speed,
            _ => 0.0,
        };
        self.mix.set_listener(&head, travel);
        for (idx, em) in self.world.emitters[..scene_emitters].iter_mut().enumerate() {
            let tmp = &mut self.scratch[..len];
            let mut controls = em.controls;
            controls.water = water * em.flow;
            controls.neural = neural;
            controls.legacy_levels = self.mix.bypass;
            controls.gust = (wind + (self.wind.local(em.position.y) - wind) * gustiness).max(0.0);
            let flying = em.kind == SourceKind::Bird && self.world.entities[em.owner].flags & object_flags::MOVING != 0;
            let view = EmitterView {
                key: idx as u64,
                class: em.class,
                kind: em.kind,
                rel: em.position - listener,
                wants: controls.activity > 0.02,
                busy: em.generator.busy(),
                flying,
            };
            let voice = self.mix.voice(idx, &view);
            let (kind_gain, send, eq_db) = (voice.gain, voice.send, *voice.eq_db);
            if !voice.permit {
                // Density throttle: hold the next phrase back until a slot frees.
                controls.activity = 0.0;
            }
            em.generator.render(tmp, &controls);
            self.mix.feed(idx, tmp, kind_gain, em.generator.busy());

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
            if flying {
                peaks[scope::FLIGHT] = peaks[scope::FLIGHT].max(peak);
            }

            // Propagation: delay/Doppler, spreading, air, ground, directivity, occlusion,
            // forest scattering, near field and reverb sends (propagation.rs).
            em.rel = head.apply_transposed(em.position);
            let mut emitter = Emitter::at(em.position, kind_gain);
            emitter.eq_db = eq_db;
            emitter.send = send;
            if em.kind == SourceKind::Water {
                emitter.ground = Some(GroundType::Water);
            }
            self.acoustics.propagate(idx, &emitter, tmp, &mut self.bus);
        }

        if scene_emitters == 0 {
            self.left[..len].fill(0.0);
            self.right[..len].fill(0.0);
            for ch in self.bed.iter_mut() {
                ch[..len].fill(0.0);
            }
        } else if self.bed_mode == 0 {
            self.acoustics.render(&mut self.bus, &head, len, &mut self.left[..len], &mut self.right[..len]);
        } else {
            let h = if self.bed_mode == 1 { Some(&head) } else { None };
            self.acoustics.render_bed(&mut self.bus, h, len, &mut self.bed);
            // Front pair stands in for the stereo taps (spectrum, level meters).
            self.left[..len].copy_from_slice(&self.bed[0][..len]);
            self.right[..len].copy_from_slice(&self.bed[1][..len]);
        }
        if let (Some(f), true) = (self.flight.as_deref_mut(), self.flight_mix > 0.0) {
            for v in peaks.iter_mut() {
                *v *= (self.flight_mix * core::f32::consts::FRAC_PI_2).cos();
            }
            let inputs = flight::Inputs {
                st: &st,
                bases: &shared.controls.bases,
                hour: env.hour,
                birds,
                rain: self.director.weather().rain_unit(),
                ambient: self.director.weather().ambient(&env),
                head,
                // The predicted head pose, relative to the direction of travel (+ = left / up).
                head_yaw: pose.quat.to_yaw_pitch_roll().0,
                head_pitch: pose.quat.to_yaw_pitch_roll().1,
                listener_offset: pose.offset,
                head_tracked: shared.head.get().is_some_and(|s| self.time - s.arrived < 1.0),
                acoustic: Acoustic {
                    temperature_c: env.temperature_c,
                    humidity_pct: env.humidity * 100.0,
                    pressure_kpa: p(param::PRESSURE),
                    ground: GroundType::ForestFloor,
                    tree_density_per_ha: p(param::TREE_DENSITY),
                    foliage: p(param::FOLIAGE),
                    reverb: p(param::REVERB),
                    ear_height: p(param::EAR_HEIGHT),
                    hrtf: if p(param::HRTF) > 0.5 { HrtfModel::Analytic } else { HrtfModel::Measured },
                },
                wind: &self.wind,
                wind_slider: wind,
                gustiness,
                water,
                neural,
            };
            let bed = match self.bed_mode {
                0 => None,
                1 => Some((&mut self.bed, Some(&head))),
                _ => Some((&mut self.bed, None)),
            };
            f.process(&inputs, self.flight_mix, &mut self.left[..len], &mut self.right[..len], bed, &mut peaks, &mut self.mix);
        }
        self.mix.finish(len);

        // Output dynamics (pascals at the ears -> digital): leveller, comfort compressor, limiter.
        self.mix.master(&self.left[..len], &self.right[..len], &mut self.gains[..len]);
        let target_fade = if playing { 1.0 } else { 0.0 };
        let fade_coef = 1.0 - (-1.0 / (0.06 * sr)).exp();
        let vol_coef = 1.0 - (-1.0 / (0.05 * sr)).exp();
        let mut energy = 0.0f32;
        for i in 0..len {
            self.fade += (target_fade - self.fade) * fade_coef;
            self.volume += (volume - self.volume) * vol_coef;
            let g = self.fade * self.volume * self.gains[i];
            if self.bed_mode != 0 {
                for ch in self.bed.iter_mut() {
                    ch[i] = soft_clip(ch[i] * g);
                }
            }
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
        shared.clock.store(self.time.to_bits(), Ordering::Relaxed);
    }

    fn publish(&self, shared: &Shared) {
        let flight = self.flight.as_deref().filter(|_| self.flight_mix > 0.5);
        let mut scene_iter;
        let mut flight_iter;
        let emitters: &mut dyn Iterator<Item = &crate::scene::Emitter> = match flight {
            Some(f) => {
                flight_iter = f.sounding();
                &mut flight_iter
            }
            None => {
                scene_iter = self.world.emitters.iter();
                &mut scene_iter
            }
        };
        let mut count = 0;
        for (i, s) in emitters.take(MAX_SOURCES).enumerate() {
            count = i + 1;
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
        shared.birds.publish(&self.director, &self.world);
        let t = &shared.telemetry;
        match flight {
            Some(f) => {
                f.publish(&shared.flight_objects);
                shared.flight_shown.store(1, Ordering::Release);
            }
            None => shared.flight_shown.store(0, Ordering::Release),
        }
        if let Some(f) = self.flight.as_deref() {
            f.publish_frame(&shared.flight_frame);
            shared.flight_framed.store(1, Ordering::Release);
        }
        if let Some(f) = self.flight.as_deref() {
            let pose = f.pose();
            t[telemetry::FLIGHT_SPEED as usize].store(pose.speed);
            t[telemetry::FLIGHT_ALTITUDE as usize].store(pose.height);
            t[telemetry::FLIGHT_ODOMETER as usize].store((pose.s % 10_000.0) as f32);
            t[telemetry::FLIGHT_TURN as usize].store(pose.turn);
            t[telemetry::FLIGHT_HEADING as usize].store(pose.heading as f32);
            t[telemetry::FLIGHT_X as usize].store(pose.x.rem_euclid(4096.0) as f32);
            t[telemetry::FLIGHT_Y as usize].store(pose.y.rem_euclid(4096.0) as f32);
        }
        t[telemetry::FLIGHT as usize].store(self.flight_mix);
        t[telemetry::GUST as usize].store(self.wind.gust());
        t[telemetry::BUBBLE_RATE as usize].store(bubble_rate);
        t[telemetry::TIME as usize].store((self.time % 86_400.0) as f32);
        t[telemetry::OUTPUT_LEVEL as usize].store(self.out_level);
        let w = self.director.weather();
        t[telemetry::RAIN_RATE as usize].store(w.rain_mm_h());
        t[telemetry::WETNESS as usize].store(w.leaf_wetness());
        t[telemetry::LIGHTNING as usize].store(w.flash());
        t[telemetry::DRIP as usize].store(w.drip_mm_h());
        t[telemetry::HEAD_HORIZON_MS as usize].store(self.head.horizon * 1000.0);
        t[telemetry::HEAD_RATE as usize].store(self.head.rate);
        let m = self.mix.publish(&shared.mix);
        t[telemetry::LOUDNESS_MOMENTARY as usize].store(m.loudness_momentary);
        t[telemetry::LOUDNESS_SHORT as usize].store(m.loudness_short);
        t[telemetry::LOUDNESS_INTEGRATED as usize].store(m.loudness_integrated);
        t[telemetry::MIX_GAIN_DB as usize].store(m.gain_db);
        t[telemetry::SOLOISTS as usize].store(m.soloists as f32);
        t[telemetry::MASKING as usize].store(m.masking_after);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlapping_render_calls_return_silence_without_touching_dsp() {
        let engine = Engine::new(48_000.0);
        let guard = engine.begin_render().unwrap();
        let mut l = [1.0; BLOCK];
        let mut r = [1.0; BLOCK];
        engine.render(&mut l, &mut r);
        assert_eq!(l, [0.0; BLOCK]);
        assert_eq!(r, [0.0; BLOCK]);
        let mut channel = [1.0; BLOCK];
        engine.render_bed(&mut [&mut channel], false);
        assert_eq!(channel, [0.0; BLOCK]);
        drop(guard);
        assert!(engine.begin_render().is_some());
    }

    #[test]
    fn a_busy_control_transaction_does_not_block_audio() {
        let engine = Engine::new(48_000.0);
        engine.set_param(param::PLAYING, 1.0);
        let mut l = [0.0; BLOCK];
        let mut r = [0.0; BLOCK];
        engine.render(&mut l, &mut r);
        let _editing = engine.shared.glides.edit();
        engine.render(&mut l, &mut r);
        assert!(l.iter().chain(&r).all(|v| v.is_finite()));
    }

    #[test]
    fn scene_geometry_tracks_tree_moves_and_ground_height() {
        let engine = Engine::new(48_000.0);
        assert!((engine.param(param::EAR_HEIGHT) - 1.7).abs() < 1e-5);
        let mut objects = [ObjectInfo::default(); MAX_OBJECTS];
        let n = engine.objects(&mut objects);
        let tree = objects[..n].iter().find(|o| o.kind == crate::ObjectKind::Tree as u32).unwrap();
        engine.set_object_position(tree.id, 1.0, 0.0, -1.7);
        engine.render(&mut [0.0; BLOCK], &mut [0.0; BLOCK]);
        let _guard = engine.begin_render().unwrap();
        // SAFETY: the render guard excludes all other DSP access.
        let dsp = unsafe { &*engine.dsp.get() };
        assert!(dsp.acoustics.context().obstacles.iter().any(|obstacle| {
            matches!(obstacle, crate::physics::diffraction::Obstacle::Cylinder { x, y, .. }
                if *x == 1.0 && *y == 0.0)
        }));
    }

    #[test]
    fn bed_is_exactly_silent_after_pause() {
        let engine = Engine::new(48_000.0);
        let mut channels = [[0.0; BLOCK]; bed::CHANNELS];
        engine.set_param(param::PLAYING, 1.0);
        for _ in 0..1000 {
            let mut outputs: Vec<_> = channels.iter_mut().map(|c| &mut c[..]).collect();
            engine.render_bed(&mut outputs, false);
        }
        engine.set_param(param::PLAYING, 0.0);
        for _ in 0..700 {
            let mut outputs: Vec<_> = channels.iter_mut().map(|c| &mut c[..]).collect();
            engine.render_bed(&mut outputs, false);
        }
        assert!(channels.iter().flatten().all(|v| *v == 0.0));
    }

    #[test]
    fn a_manual_control_survives_concurrent_glide_readers() {
        use std::sync::Arc;
        let engine = Arc::new(Engine::new(48_000.0));
        engine.apply_patch_json(r#"{"glide_s":120,"changes":[{"key":"master.volume","value":0.1}]}"#);
        let reader = Arc::clone(&engine);
        let thread = std::thread::spawn(move || {
            let mut controls = [ControlInfo::default(); controls::COUNT];
            for _ in 0..1000 { reader.controls(&mut controls); }
        });
        for _ in 0..1000 { engine.set_control(controls::id::MASTER_VOLUME, 0.73); }
        thread.join().unwrap();
        engine.render(&mut [0.0; BLOCK], &mut [0.0; BLOCK]);
        assert_eq!(engine.control(controls::id::MASTER_VOLUME), 0.73);
    }

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
        assert_eq!(engine.sources(&mut src), 46);
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
        assert_eq!(n, 34);
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
        let mut c = [ControlInfo::default(); 96];
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
            // Absolute level comparison of the model itself: no calibration headroom, leveller
            // or ducking in the way.
            engine.set_mix_bypass(true);
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
