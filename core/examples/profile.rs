//! Where the audio thread's time goes: CPU per section of the render path (control and
//! director, each generator kind, mix, propagation, path filters, encode, reverb, rotation,
//! decode, master) and how many emitters exist and actually sound, per scenario.
//!
//!   cargo run --release --example profile -- [seconds=30] [scenario …]
//!
//! Scenarios: `default` (the app's built-in scene as it starts), `dawn` (5:30, chorus),
//! `night` (22:00, insects), `rain` (8 mm/h with a storm).
//! Without names, all of them. Renders in 480-frame callbacks like the app on AirPods
//! and skips the first 5 s (shaders aside, the director settles). Percentages are of one
//! core in real time, i.e. what Activity Monitor would show for the audio thread alone.
//! `PACED=1` sleeps out each 10 ms callback like a real audio device instead of rendering
//! flat out: the core then idles between bursts, as it does in the app.

use std::time::Instant;

use asmr_core::profile;
use asmr_core::{param, Engine, SceneDesc};

const SR: f32 = 48_000.0;
const CALLBACK: usize = 480;
const WARMUP_S: f32 = 5.0;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let seconds: f32 = args.first().and_then(|s| s.parse().ok()).unwrap_or(30.0);
    let mut names: Vec<&str> = args.iter().skip(1).map(String::as_str).collect();
    if names.is_empty() {
        names = vec!["default", "dawn", "night", "rain"];
    }
    prefer_performance_cores();
    for name in names {
        run(name, seconds);
    }
}

fn run(name: &str, seconds: f32) {
    let json = match name {
        _ => asmr_core::scene::DEFAULT_SCENE_JSON.to_string(),
    };
    let desc = SceneDesc::from_json(&json).unwrap_or_else(|e| panic!("{e}"));
    let engine = Engine::with_scene(SR, &desc).unwrap_or_else(|e| panic!("{e}"));
    engine.set_param(param::PLAYING, 1.0);
    engine.set_param(param::VOLUME, 0.7);
    match name {
        "dawn" => engine.set_param(param::TIME_OF_DAY, 5.5),
        "night" => engine.set_param(param::TIME_OF_DAY, 22.0),
        "rain" => {
            engine.set_param(param::RAIN, 8.0);
            engine.set_param(param::STORM, 0.6);
        }
        _ => {}
    }

    let (mut l, mut r) = (vec![0.0f32; CALLBACK], vec![0.0f32; CALLBACK]);
    let callbacks = |s: f32| (s * SR) as usize / CALLBACK;
    for _ in 0..callbacks(WARMUP_S) {
        engine.render(&mut l, &mut r);
    }
    profile::enable();
    let paced = std::env::var_os("PACED").is_some();
    let period = std::time::Duration::from_secs_f64(CALLBACK as f64 / SR as f64);
    let mut deadline = Instant::now();
    for _ in 0..callbacks(seconds) {
        engine.render(&mut l, &mut r);
        if paced {
            deadline += period;
            if let Some(rest) = deadline.checked_duration_since(Instant::now()) {
                std::thread::sleep(rest);
            }
        }
    }
    println!("\n{}", profile::format(&format!("{name}{} ({})", if paced { ", paced" } else { "" }, desc.name)));
}

/// Ask for performance cores, as the CoreAudio render thread gets (QOS_CLASS_USER_INTERACTIVE).
fn prefer_performance_cores() {
    #[cfg(target_os = "macos")]
    {
        extern "C" {
            fn pthread_set_qos_class_self_np(qos: u32, priority: i32) -> i32;
        }
        unsafe {
            pthread_set_qos_class_self_np(if std::env::var_os("ECORE").is_some() { 0x09 } else { 0x21 }, 0);
        }
    }
}
