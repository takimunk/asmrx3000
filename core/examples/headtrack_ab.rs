//! Offline A/B renders for head tracking and externalisation.
//!
//!   cargo run --release --example headtrack_ab -- [out_dir=../build/ab] [seconds=20]
//!
//! Head tracking. A listener looks around naturally (two sways plus a nod). AirPods report the
//! pose at 25 Hz, 30 ms old, and sound reaches the ear 160 ms after rendering. The engine is
//! fed the *tracking error*: the rotation between the pose it renders with (the core's own
//! predictor, `headpose.rs`) and the true head pose at the moment the block is heard. That is
//! what a moving listener perceives: with a perfect tracker the forest stands still, and every
//! swing in these files is error.
//!   head_lagged.wav     no prediction (last measured pose, smoothed)
//!   head_predicted.wav  prediction over the measured latency (default)
//! Externalisation (static head):
//!   ext_anechoic.wav    direct sound + ground only (no trunks: no early reflections, no reverb)
//!   ext_full.wav        full forest acoustics
//! Each run prints the rms / max tracking error.

use std::fs::File;
use std::io::{BufWriter, Write};

use asmr_core::headpose::{HeadPoseInput, HeadTracker, PoseSettings, Quat};
use asmr_core::math::Vec3;
use asmr_core::{param, Engine};

const SR: f32 = 48_000.0;
const BLOCK: usize = 64;
const LATENCY: f32 = 0.16;
const SENSOR_AGE: f32 = 0.03;

fn true_pose(t: f32) -> Quat {
    use std::f32::consts::PI;
    let yaw = 50f32.to_radians() * (2.0 * PI * 0.23 * t).sin() + 15f32.to_radians() * (2.0 * PI * 0.71 * t + 1.0).sin();
    let pitch = 10f32.to_radians() * (2.0 * PI * 0.37 * t).sin();
    Quat::from_yaw_pitch_roll(yaw, pitch, 0.0)
}

fn conj(q: Quat) -> Quat {
    Quat { w: q.w, x: -q.x, y: -q.y, z: -q.z }
}

fn render_tracking(path: &str, seconds: f32, prediction: f32) -> std::io::Result<(f32, f32)> {
    let engine = Engine::new(SR);
    engine.set_param(param::PLAYING, 1.0);
    engine.set_param(param::VOLUME, 0.8);
    // The engine renders exactly the error pose we hand it.
    engine.set_param(param::HEAD_PREDICTION, 0.0);
    engine.set_param(param::HEAD_PARALLAX, 0.0);
    let input = HeadPoseInput::new();
    let mut tracker = HeadTracker::new();
    let settings = PoseSettings { output_latency: LATENCY, prediction, parallax: false };
    let blocks = (seconds * SR) as usize / BLOCK;
    let mut pcm = Vec::with_capacity(blocks * BLOCK * 2);
    let (mut l, mut r) = ([0.0f32; BLOCK], [0.0f32; BLOCK]);
    let (mut next, mut sum, mut max, mut n) = (0.0f64, 0.0f32, 0.0f32, 0usize);
    for b in 0..blocks {
        let t = (b * BLOCK) as f64 / SR as f64;
        while t >= next {
            let tm = next as f32 - SENSOR_AGE;
            let (q0, q1) = (true_pose(tm - 0.005), true_pose(tm + 0.005));
            let d = conj(q0).mul(q1);
            let d = if d.w < 0.0 { Quat { w: -d.w, x: -d.x, y: -d.y, z: -d.z } } else { d };
            let w = Vec3::new(d.x, d.y, d.z) * (2.0 / 0.01);
            input.set(true_pose(tm), w, SENSOR_AGE, next);
            next += 0.04;
        }
        let pose = tracker.step(&input, (0.0, 0.0, 0.0), t, BLOCK as f32 / SR, &settings);
        let truth = true_pose(t as f32 + LATENCY);
        // Error rotation in the head frame: what the listener's (true) head sees.
        let e = conj(truth).mul(pose.quat).normalized();
        engine.set_head_pose([e.w, e.x, e.y, e.z], [0.0; 3], 0.0);
        engine.render(&mut l, &mut r);
        if t > 2.0 {
            let a = pose.quat.angle_to(truth).to_degrees();
            sum += a * a;
            max = max.max(a);
            n += 1;
        }
        for (a, c) in l.iter().zip(&r) {
            pcm.push((a * 32767.0) as i16);
            pcm.push((c * 32767.0) as i16);
        }
    }
    write_wav(path, &pcm)?;
    Ok(((sum / n.max(1) as f32).sqrt(), max))
}

fn render_static(path: &str, seconds: f32, anechoic: bool) -> std::io::Result<()> {
    let engine = Engine::new(SR);
    engine.set_param(param::PLAYING, 1.0);
    engine.set_param(param::VOLUME, 0.8);
    if anechoic {
        engine.set_param(param::TREE_DENSITY, 0.0);
        engine.set_param(param::REVERB, 0.0);
        engine.set_param(param::HEAD_PARALLAX, 0.0);
    }
    let blocks = (seconds * SR) as usize / BLOCK;
    let mut pcm = Vec::with_capacity(blocks * BLOCK * 2);
    let (mut l, mut r) = ([0.0f32; BLOCK], [0.0f32; BLOCK]);
    for _ in 0..blocks {
        engine.render(&mut l, &mut r);
        for (a, c) in l.iter().zip(&r) {
            pcm.push((a * 32767.0) as i16);
            pcm.push((c * 32767.0) as i16);
        }
    }
    write_wav(path, &pcm)
}

fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let dir = args.get(1).cloned().unwrap_or_else(|| "../build/ab".into());
    let seconds: f32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(20.0);
    std::fs::create_dir_all(&dir)?;
    for (name, prediction) in [("head_lagged", 0.0), ("head_predicted", 1.0)] {
        let path = format!("{dir}/{name}.wav");
        let (rms, max) = render_tracking(&path, seconds, prediction)?;
        println!("{path}: tracking error rms {rms:.1} deg, max {max:.1} deg (25 Hz pose, {:.0} ms latency)", LATENCY * 1000.0);
    }
    for (name, anechoic) in [("ext_anechoic", true), ("ext_full", false)] {
        let path = format!("{dir}/{name}.wav");
        render_static(&path, seconds, anechoic)?;
        println!("{path}");
    }
    Ok(())
}

fn write_wav(path: &str, pcm: &[i16]) -> std::io::Result<()> {
    let mut f = BufWriter::new(File::create(path)?);
    let data_len = (pcm.len() * 2) as u32;
    f.write_all(b"RIFF")?;
    f.write_all(&(36 + data_len).to_le_bytes())?;
    f.write_all(b"WAVEfmt ")?;
    f.write_all(&16u32.to_le_bytes())?;
    f.write_all(&1u16.to_le_bytes())?;
    f.write_all(&2u16.to_le_bytes())?;
    f.write_all(&(SR as u32).to_le_bytes())?;
    f.write_all(&(SR as u32 * 4).to_le_bytes())?;
    f.write_all(&4u16.to_le_bytes())?;
    f.write_all(&16u16.to_le_bytes())?;
    f.write_all(b"data")?;
    f.write_all(&data_len.to_le_bytes())?;
    for s in pcm {
        f.write_all(&s.to_le_bytes())?;
    }
    f.flush()
}
