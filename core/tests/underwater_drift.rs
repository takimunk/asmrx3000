//! The underwater scene is fully procedural and comfortable: it parses, references no sound
//! files, uses only the underwater models, keeps every moving source slow enough that its
//! Doppler shift stays far below audibility, drives no group's distance from a lever (a
//! distance glide is a Doppler glide), and the engine renders it cleanly: finite, no clicks,
//! no loud surprises.

use std::collections::HashSet;

use asmr_core::{param, Engine, SceneDesc};
use serde_json::Value;

const JSON: &str = include_str!("../scenes/underwater_drift.json");
/// The engine propagates at the speed of sound in air (343 m/s), so a source moving at v
/// shifts by up to v/343: 0.5 m/s is 0.15 %, 2.5 cents.
const MAX_SPEED_MS: f64 = 0.5;

#[test]
fn underwater_scene_is_procedural_and_slow() {
    SceneDesc::from_json(JSON).expect("scene parses");
    let v: Value = serde_json::from_str(JSON).unwrap();
    let ground = v["environment"]["ground_z"].as_f64().unwrap();
    let mut groups = HashSet::new();
    for e in v["entities"].as_array().unwrap() {
        let id = &e["id"];
        assert!(e.get("files").is_none(), "clip {id} references sound files");
        assert!(e.get("echo").is_none(), "clip {id}: echo feedback smears pitch");
        assert!(e.get("rate").is_none(), "clip {id}: rate jitter changes pitch");
        let model = e["sound"]["model"].as_str().unwrap_or_else(|| panic!("clip {id} has no procedural sound"));
        assert!(["sea", "reef", "whale", "dolphins"].contains(&model), "clip {id}: model {model}");
        groups.insert(e["group"].as_str().unwrap().to_string());
        let m = &e["motion"];
        match m["type"].as_str().unwrap() {
            "fixed" => assert!(m["position"][2].as_f64().unwrap() > ground),
            "drift" => {
                let speed = m["speed"].as_f64().unwrap() - m["own_speed"].as_f64().unwrap_or(0.0);
                assert!(speed.abs() <= MAX_SPEED_MS, "clip {id} drifts at {speed} m/s");
                assert!(m["height"][0].as_f64().unwrap() > ground);
            }
            t => panic!("clip {id}: motion {t} (escort / orbit / pass move too fast or wander)"),
        }
    }
    for g in ["current", "bubbles", "reef", "whales", "dolphins", "rain_above"] {
        assert!(groups.contains(g), "group {g} missing");
    }
    for l in v["levers"].as_array().unwrap() {
        for d in l["drives"].as_array().unwrap() {
            assert!(groups.contains(d["group"].as_str().unwrap()));
            let p = d["param"].as_str().unwrap();
            assert!(p != "distance" && p != "pitch", "lever {} drives {p}", l["key"]);
        }
    }
    for c in v["controls"].as_array().unwrap() {
        for p in c["params"].as_array().into_iter().flatten() {
            assert!(!["distance", "pitch"].contains(&p.as_str().unwrap()), "{}: panel offers {p}", c["group"]);
        }
    }
}

#[test]
fn underwater_scene_renders_cleanly() {
    let e = Engine::from_json(48_000.0, JSON).expect("scene builds without any file");
    e.set_param(param::PLAYING, 1.0);
    let n = 512;
    let (mut l, mut r) = (vec![0.0f32; n], vec![0.0f32; n]);
    let seconds = 40.0;
    let blocks = (seconds * 48_000.0 / n as f32) as usize;
    let mut energy = Vec::with_capacity(blocks);
    let mut peak = 0.0f32;
    let mut worst_jump = 0.0f32;
    let mut prev = (0.0f32, 0.0f32);
    for b in 0..blocks {
        e.render(&mut l, &mut r);
        let mut s = 0.0f64;
        let mut d2 = 0.0f64;
        for i in 0..n {
            assert!(l[i].is_finite() && r[i].is_finite());
            s += (l[i] as f64).powi(2) + (r[i] as f64).powi(2);
            let (dl, dr) = (l[i] - prev.0, r[i] - prev.1);
            d2 += (dl as f64).powi(2) + (dr as f64).powi(2);
            prev = (l[i], r[i]);
            if b > 200 {
                peak = peak.max(l[i].abs()).max(r[i].abs());
            }
        }
        // Clicks: the largest sample step against the block's typical step.
        let typical = (d2 / (2 * n) as f64).sqrt() as f32 + 1e-6;
        if b > 200 {
            for i in 1..n {
                worst_jump = worst_jump.max((l[i] - l[i - 1]).abs() / typical).max((r[i] - r[i - 1]).abs() / typical);
            }
        }
        energy.push(s / (2 * n) as f64);
    }
    let settled = &energy[400..];
    let mean = settled.iter().sum::<f64>() / settled.len() as f64;
    assert!(mean > 1e-8, "scene is silent");
    assert!(peak < 0.89, "peak {peak} (≥ −1 dBFS)");
    // Short-term (≈ 1 s) level never far above the take's average.
    let w = 94;
    let mut worst = f64::MIN;
    for c in settled.windows(w).step_by(10) {
        let m = c.iter().sum::<f64>() / w as f64;
        worst = worst.max(10.0 * (m / mean).log10());
    }
    assert!(worst < 8.0, "1 s level {worst:.1} dB over the average");
    assert!(worst_jump < 12.0, "sample step {worst_jump:.1}× the local typical step");
}
