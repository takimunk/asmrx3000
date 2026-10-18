//! asmr-core: real-time acoustic environment simulation.
//!
//! The C ABI below (mirrored in `include/asmr.h`) is the only surface the platform shells use.

pub mod acoustics;
mod ambisonics;
mod bed;
mod binaural;
pub mod controls;
pub mod director;
pub mod neural;
mod dsp;
mod engine;
mod fft;
mod delay;
mod fftconv;
pub mod flight;
pub mod headpose;
mod hrtf;
pub mod math;
pub mod mix;
mod nearfield;
mod num;
mod physics;
mod prompt;
mod propagation;
mod reverb;
mod rng;
mod rotation;
pub mod scene;
mod shaping;
mod simd;
pub mod sources;
mod windfield;

pub use ambisonics::CHANNELS;
pub use controls::ControlInfo;
pub use engine::{param, telemetry, Engine, SourceInfo, SourceKind, BLOCK, MAX_SOURCES};
pub use scene::{object_flags, ObjectInfo, ObjectKind, SceneDesc, MAX_OBJECTS};

use std::ffi::{c_char, CStr};
use std::slice;

/// Creates an engine. Free it with `asmr_engine_free`.
#[no_mangle]
pub extern "C" fn asmr_engine_new(sample_rate: f32) -> *mut Engine {
    Box::into_raw(Box::new(Engine::new(sample_rate)))
}

/// Creates an engine running the scene described by the NUL-terminated JSON `scene_json`
/// (format: docs/director.md). Returns NULL if the scene does not parse or validate.
///
/// # Safety
/// `scene_json` must be NULL or a valid NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn asmr_engine_new_with_scene(sample_rate: f32, scene_json: *const c_char) -> *mut Engine {
    if scene_json.is_null() {
        return std::ptr::null_mut();
    }
    match CStr::from_ptr(scene_json).to_str().map_err(|e| e.to_string()).and_then(|j| Engine::from_json(sample_rate, j)) {
        Ok(e) => Box::into_raw(Box::new(e)),
        Err(_) => std::ptr::null_mut(),
    }
}

/// # Safety
/// `engine` must come from `asmr_engine_new` and must not be used afterwards.
/// No other thread may be inside `asmr_render` for this engine.
#[no_mangle]
pub unsafe extern "C" fn asmr_engine_free(engine: *mut Engine) {
    if !engine.is_null() {
        drop(Box::from_raw(engine));
    }
}

/// Renders `frames` stereo samples (non-interleaved). Call from one audio thread only.
///
/// # Safety
/// `left` and `right` must each point to `frames` writable floats.
#[no_mangle]
pub unsafe extern "C" fn asmr_render(engine: *const Engine, left: *mut f32, right: *mut f32, frames: u32) {
    if engine.is_null() || left.is_null() || right.is_null() {
        return;
    }
    let l = slice::from_raw_parts_mut(left, frames as usize);
    let r = slice::from_raw_parts_mut(right, frames as usize);
    (*engine).render(l, r);
}

/// # Safety
/// `engine` must be a live engine pointer.
#[no_mangle]
pub unsafe extern "C" fn asmr_set_param(engine: *const Engine, param: u32, value: f32) {
    if let Some(e) = engine.as_ref() {
        e.set_param(param, value);
    }
}

/// # Safety
/// `engine` must be a live engine pointer.
#[no_mangle]
pub unsafe extern "C" fn asmr_get_param(engine: *const Engine, param: u32) -> f32 {
    engine.as_ref().map_or(0.0, |e| e.param(param))
}

/// # Safety
/// `engine` must be a live engine pointer.
#[no_mangle]
pub unsafe extern "C" fn asmr_get_telemetry(engine: *const Engine, which: u32) -> f32 {
    engine.as_ref().map_or(0.0, |e| e.telemetry(which))
}

/// # Safety
/// `out` must point to `max` writable `AsmrSourceInfo`.
#[no_mangle]
pub unsafe extern "C" fn asmr_get_sources(engine: *const Engine, out: *mut SourceInfo, max: u32) -> u32 {
    match (engine.as_ref(), out.is_null()) {
        (Some(e), false) => e.sources(slice::from_raw_parts_mut(out, max as usize)) as u32,
        _ => 0,
    }
}

/// # Safety
/// `out` must point to `bands` writable floats.
#[no_mangle]
pub unsafe extern "C" fn asmr_get_spectrum(engine: *const Engine, out: *mut f32, bands: u32) {
    if let (Some(e), false) = (engine.as_ref(), out.is_null()) {
        e.spectrum(slice::from_raw_parts_mut(out, bands as usize));
    }
}

/// # Safety
/// `out` must point to `points` writable floats.
#[no_mangle]
pub unsafe extern "C" fn asmr_get_wind_field(engine: *const Engine, out: *mut f32, points: u32) {
    if let (Some(e), false) = (engine.as_ref(), out.is_null()) {
        e.wind_field(slice::from_raw_parts_mut(out, points as usize));
    }
}

/// Allocates `len` bytes in the module's linear memory (wasm shells copy buffers through this).
#[no_mangle]
pub extern "C" fn asmr_alloc(len: u32) -> *mut u8 {
    let mut v = Vec::<u8>::with_capacity(len as usize);
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    p
}

/// # Safety
/// `ptr`/`len` must come from `asmr_alloc`.
#[no_mangle]
pub unsafe extern "C" fn asmr_dealloc(ptr: *mut u8, len: u32) {
    if !ptr.is_null() {
        drop(Vec::from_raw_parts(ptr, 0, len as usize));
    }
}

/// # Safety
/// `out` must point to `max` writable `AsmrObjectInfo`.
#[no_mangle]
pub unsafe extern "C" fn asmr_get_objects(engine: *const Engine, out: *mut ObjectInfo, max: u32) -> u32 {
    match (engine.as_ref(), out.is_null()) {
        (Some(e), false) => e.objects(slice::from_raw_parts_mut(out, max as usize)) as u32,
        _ => 0,
    }
}

/// # Safety
/// `engine` must be a live engine pointer.
#[no_mangle]
pub unsafe extern "C" fn asmr_set_object_position(engine: *const Engine, id: u32, x: f32, y: f32, z: f32) {
    if let Some(e) = engine.as_ref() {
        e.set_object_position(id, x, y, z);
    }
}

/// Live peak waveform of a scope track (ASMR_SCOPE_*), oldest first; returns how many points
/// are real.
///
/// # Safety
/// `out` must point to `points` writable floats.
#[no_mangle]
pub unsafe extern "C" fn asmr_get_scope(engine: *const Engine, track: u32, out: *mut f32, points: u32) -> u32 {
    match (engine.as_ref(), out.is_null()) {
        (Some(e), false) => e.scope(track, slice::from_raw_parts_mut(out, points as usize)) as u32,
        _ => 0,
    }
}

/// Copies the control tree into `out` (see asmr.h `AsmrControlInfo`), returns the count.
///
/// # Safety
/// `out` must point to `max` writable `AsmrControlInfo`.
#[no_mangle]
pub unsafe extern "C" fn asmr_get_controls(engine: *const Engine, out: *mut ControlInfo, max: u32) -> u32 {
    match (engine.as_ref(), out.is_null()) {
        (Some(e), false) => e.controls(slice::from_raw_parts_mut(out, max as usize)) as u32,
        _ => 0,
    }
}

/// Sets a control. NaN on a child control hands it back to its macro.
///
/// # Safety
/// `engine` must be a live engine pointer.
#[no_mangle]
pub unsafe extern "C" fn asmr_set_control(engine: *const Engine, id: u32, value: f32) {
    if let Some(e) = engine.as_ref() {
        e.set_control(id, value);
    }
}

/// Writes the NUL-terminated name of option `index` of a discrete control (presets; 0 =
/// "scene") into `out` and returns its length, or 0 if there is no such option.
///
/// # Safety
/// `out` must point to `max` writable bytes.
#[no_mangle]
pub unsafe extern "C" fn asmr_get_control_option(_engine: *const Engine, id: u32, index: u32, out: *mut c_char, max: u32) -> u32 {
    let Some(name) = controls::option_name(id, index) else { return 0 };
    if out.is_null() || max == 0 {
        return 0;
    }
    let n = name.len().min(max as usize - 1);
    let dst = slice::from_raw_parts_mut(out as *mut u8, n + 1);
    dst[..n].copy_from_slice(&name.as_bytes()[..n]);
    dst[n] = 0;
    n as u32
}

/// Shape of a streamed (flight mode) object: river bed, lake shore or ridge line, as
/// listener-relative (x, y, z) triples written to `out`; returns the number of points
/// (0 for objects without a shape and outside flight mode).
///
/// # Safety
/// `out` must point to `3 * max_points` writable floats.
#[no_mangle]
pub unsafe extern "C" fn asmr_get_object_path(engine: *const Engine, id: u32, out: *mut f32, max_points: u32) -> u32 {
    match (engine.as_ref(), out.is_null()) {
        (Some(e), false) => e.object_path(id, slice::from_raw_parts_mut(out, 3 * max_points as usize)) as u32,
        _ => 0,
    }
}

/// Head orientation from a motion sensor (AirPods), any thread. Quaternion (w, x, y, z) rotating
/// head-frame vectors into the world frame, in the engine frame (+x forward, +y left, +z up) and
/// already recentred; angular velocity (rad/s) in the head frame; `age` = seconds since the
/// sensor measured it. From the first call on this replaces ASMR_PARAM_HEAD_YAW/PITCH/ROLL; the
/// engine predicts the pose to the time each block is heard (ASMR_PARAM_OUTPUT_LATENCY).
///
/// # Safety
/// `engine` must be a live engine pointer.
#[no_mangle]
pub unsafe extern "C" fn asmr_set_head_pose(engine: *const Engine, qw: f32, qx: f32, qy: f32, qz: f32, wx: f32, wy: f32, wz: f32, age: f32) {
    if let Some(e) = engine.as_ref() {
        e.set_head_pose([qw, qx, qy, qz], [wx, wy, wz], age);
    }
}

/// Instead of `asmr_render`: renders a 7.1.4 loudspeaker bed for an external binaural renderer
/// (AUSpatialMixer with the listener's personalised HRTF). `channels` points to `count` channel
/// pointers of `frames` floats, order L R C LFE Ls Rs Rls Rrs Vhl Vhr Ltr Rtr
/// (kAudioChannelLayoutTag_Atmos_7_1_4); extra channels are zeroed. `world_locked` = 1 skips the
/// engine's head rotation (the renderer tracks the head itself). Audio thread only.
///
/// # Safety
/// `channels` must point to `count` valid pointers, each to `frames` writable floats.
#[no_mangle]
pub unsafe extern "C" fn asmr_render_bed(engine: *const Engine, channels: *const *mut f32, count: u32, frames: u32, world_locked: u32) {
    if engine.is_null() || channels.is_null() || count == 0 {
        return;
    }
    let ptrs = slice::from_raw_parts(channels, count as usize);
    if ptrs.iter().any(|p| p.is_null()) || count as usize > 32 {
        return;
    }
    let mut outs: [&mut [f32]; 32] = Default::default();
    for (o, p) in outs.iter_mut().zip(ptrs) {
        *o = slice::from_raw_parts_mut(*p, frames as usize);
    }
    (*engine).render_bed(&mut outs[..count as usize], world_locked != 0);
}

/// Current state as JSON: every control (key, value, unit, range, preset names, overridden),
/// environment readings and scene objects (docs/prompt-control.md). Writes it NUL-terminated
/// into `out` if `max` is larger than its length; returns the length in bytes either way (call
/// with `out` NULL to size the buffer).
///
/// # Safety
/// `out` must be NULL or point to `max` writable bytes.
#[no_mangle]
pub unsafe extern "C" fn asmr_get_state_json(engine: *const Engine, out: *mut c_char, max: u32) -> u32 {
    match engine.as_ref() {
        Some(e) => prompt::write_c(&e.state_json(), out, max),
        None => 0,
    }
}

/// Applies a JSON patch `{"glide_s", "changes": [{"key", "value"}], "moves": [{"object",
/// "position"}]}`: values are clamped, presets may be names, "reset" hands a child back to its
/// macro, unknown keys are reported. Continuous controls glide. The result JSON (`ok`,
/// `applied`, `moved`, `errors`, `undo`) is written into `out` if it fits; returns its length.
///
/// # Safety
/// `patch` must be a NUL-terminated string; `out` NULL or `max` writable bytes.
#[no_mangle]
pub unsafe extern "C" fn asmr_apply_patch_json(engine: *const Engine, patch: *const c_char, out: *mut c_char, max: u32) -> u32 {
    let (Some(e), false) = (engine.as_ref(), patch.is_null()) else { return 0 };
    let result = match CStr::from_ptr(patch).to_str() {
        Ok(p) => e.apply_patch_json(p),
        Err(_) => r#"{"ok":false,"applied":[],"moved":[],"errors":["patch is not UTF-8"],"undo":{"changes":[]}}"#.to_string(),
    };
    prompt::write_c(&result, out, max)
}

/// `{"system", "context", "tool": {"name", "description", "input_schema"}}` for a language
/// model: static system prompt, the current values as text, the tool schema generated from
/// the control tree. Same buffer convention as `asmr_get_state_json`.
///
/// # Safety
/// `out` must be NULL or point to `max` writable bytes.
#[no_mangle]
pub unsafe extern "C" fn asmr_get_prompt_json(engine: *const Engine, out: *mut c_char, max: u32) -> u32 {
    match engine.as_ref() {
        Some(e) => prompt::write_c(&e.prompt_json(), out, max),
        None => 0,
    }
}
