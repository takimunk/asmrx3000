//! asmr-core: real-time acoustic environment simulation.
//!
//! The C ABI below (mirrored in `include/asmr.h`) is the only surface the platform shells use.

mod ambisonics;
mod binaural;
mod dsp;
mod engine;
mod fft;
mod math;
mod rng;
mod sources;
mod windfield;

pub use engine::{param, telemetry, Engine, SourceInfo, SourceKind, MAX_SOURCES};

use std::slice;

/// Creates an engine. Free it with `asmr_engine_free`.
#[no_mangle]
pub extern "C" fn asmr_engine_new(sample_rate: f32) -> *mut Engine {
    Box::into_raw(Box::new(Engine::new(sample_rate)))
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
