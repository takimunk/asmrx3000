//! CPU benchmark of the spatial engine alone (propagation + reverb + rotation + binaural),
//! with N emitters fed by pre-generated noise so source generators do not count.
//!
//!   cargo run --release --example bench_spatial -- [emitters=64] [seconds=10]
//!
//! "moving": half the emitters circle the listener at 2..40 m and 3..15 m/s (Doppler,
//! refits, occlusion changes every few blocks); "static": all emitters fixed. Reports
//! thread CPU time / audio time (fraction of one core), best of three runs, so other
//! work on a busy machine does not count against the engine.

use std::time::Instant;

#[repr(C)]
struct Timespec {
    sec: i64,
    nsec: i64,
}

extern "C" {
    fn clock_gettime(clock: i32, tp: *mut Timespec) -> i32;
    #[cfg(target_os = "macos")]
    fn pthread_set_qos_class_self_np(qos: u32, priority: i32) -> i32;
}

/// Ask for performance cores, as the CoreAudio render thread gets (QOS_CLASS_USER_INTERACTIVE).
fn prefer_performance_cores() {
    #[cfg(target_os = "macos")]
    unsafe {
        pthread_set_qos_class_self_np(0x21, 0);
    }
}

/// CPU time of the calling thread (excludes time the OS ran other work on this core).
fn thread_cpu_seconds() -> Option<f64> {
    #[cfg(target_os = "macos")]
    const CLOCK_THREAD_CPUTIME_ID: i32 = 16;
    #[cfg(not(target_os = "macos"))]
    const CLOCK_THREAD_CPUTIME_ID: i32 = 3;
    let mut t = Timespec { sec: 0, nsec: 0 };
    (unsafe { clock_gettime(CLOCK_THREAD_CPUTIME_ID, &mut t) } == 0).then(|| t.sec as f64 + t.nsec as f64 * 1e-9)
}

use asmr_core::acoustics::{Acoustics, Emitter, Features};
use asmr_core::math::{Mat3, Vec3};
use asmr_core::{BLOCK, CHANNELS};

fn position(e: usize, t: f32, moving: bool) -> Vec3 {
    let k = e as f32;
    let radius = 2.0 + 38.0 * ((k * 0.618) % 1.0);
    if moving && e % 2 == 0 {
        let w = (3.0 + 12.0 * ((k * 0.37) % 1.0)) / radius;
        let a = w * t + k;
        Vec3::new(radius * a.cos(), radius * a.sin(), 1.0 + 4.0 * ((k * 0.21) % 1.0))
    } else {
        let a = k * 2.399;
        Vec3::new(radius * a.cos(), radius * a.sin(), 0.5 + 6.0 * ((k * 0.13) % 1.0))
    }
}

fn run_once(emitters: usize, seconds: f32, features: Features, moving: bool, noise: &[f32]) -> f64 {
    let sr = 48_000.0f32;
    let mut ac = Acoustics::new(sr, emitters);
    ac.set_features(features);
    let mut bus = [[0.0f32; BLOCK]; CHANNELS];
    let mut l = [0.0f32; BLOCK];
    let mut r = [0.0f32; BLOCK];
    let blocks = (seconds * sr) as usize / BLOCK;
    let mut sink = 0.0f32;
    let started = Instant::now();
    let cpu0 = thread_cpu_seconds();
    for b in 0..blocks {
        let t = (b * BLOCK) as f32 / sr;
        for ch in bus.iter_mut() {
            ch.fill(0.0);
        }
        for e in 0..emitters {
            let o = (b * 977 + e * 4099) % (noise.len() - BLOCK);
            ac.propagate(e, &Emitter::at(position(e, t, moving), 1.0), &noise[o..o + BLOCK], &mut bus);
        }
        let head = Mat3::from_head((t * 0.5).sin(), 0.1 * (t * 0.3).sin(), 0.0);
        ac.render(&mut bus, &head, BLOCK, &mut l, &mut r);
        sink += l[0] + r[BLOCK - 1];
    }
    let busy = match (cpu0, thread_cpu_seconds()) {
        (Some(a), Some(b)) => b - a,
        _ => started.elapsed().as_secs_f64(),
    };
    let load = busy / seconds as f64;
    assert!(sink.is_finite());
    load
}

fn run(emitters: usize, seconds: f32, features: Features, moving: bool, label: &str, noise: &[f32]) {
    let best = (0..3).map(|_| run_once(emitters, seconds, features, moving, noise)).fold(f64::MAX, f64::min);
    println!("{label:<40} {:>6.2} % of one core", best * 100.0);
}

fn main() {
    prefer_performance_cores();
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(64);
    let secs: f32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(10.0);
    let mut seed = 0x1234_5678u32;
    let noise: Vec<f32> = (0..1 << 16)
        .map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            (seed as f32 / u32::MAX as f32 - 0.5) * 0.1
        })
        .collect();
    let all = Features::default();
    let none = Features { ground: false, reverb: false, early_reflections: false, occlusion: false, forest: false, near_field: false };
    if let Some(mode) = args.get(3) {
        // Single long run for profiling: "static" or "moving".
        let t = run_once(n, secs, all, mode == "moving", &noise);
        println!("{mode}: {:.2} %", t * 100.0);
        return;
    }
    println!("{n} emitters, 48 kHz, block {BLOCK}:");
    run(n, secs, all, true, "full pipeline, half moving", &noise);
    run(n, secs, all, false, "full pipeline, all static", &noise);
    run(n, secs, Features { reverb: false, early_reflections: false, ..all }, true, "  moving, no reverb / early reflections", &noise);
    run(n, secs, Features { ground: false, ..all }, true, "  moving, no ground path", &noise);
    run(n, secs, none, true, "  moving, direct path only", &noise);
    run(0, secs, all, false, "fixed cost (reverb, rotation, decoder)", &noise);
}
