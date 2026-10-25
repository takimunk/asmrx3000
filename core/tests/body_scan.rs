//! The body scan holds together without a single sound file: it parses, every clip is a
//! procedural `bowl` (no `files`), the guide's path layers cover the script, the levers drive
//! the scene's groups, and the engine renders it -- the opening strike sounds, nothing clips,
//! and the guide is steady (no block of the first seconds is far above the rest).

use std::collections::HashSet;

use asmr_core::{param, Engine, SceneDesc};
use serde_json::Value;

const JSON: &str = include_str!("../scenes/body_scan.json");

#[test]
fn body_scan_is_procedural_and_consistent() {
    SceneDesc::from_json(JSON).expect("scene parses");
    let v: Value = serde_json::from_str(JSON).unwrap();
    let mut groups = HashSet::new();
    let mut strikes = 0;
    for e in v["entities"].as_array().unwrap() {
        let id = &e["id"];
        assert_eq!(e["kind"], "clip", "clip {id}");
        assert!(e.get("files").is_none(), "clip {id} references sound files");
        assert_eq!(e["sound"]["model"], "bowl", "clip {id} is not a bowl");
        groups.insert(e["group"].as_str().unwrap().to_string());
        match e["motion"]["type"].as_str().unwrap() {
            "path" => {
                // The guide: keyframes inside the 301 s script, gains within the clip envelope.
                let pts = e["motion"]["points"].as_array().unwrap();
                let last = pts.last().unwrap()[0].as_f64().unwrap();
                assert!(last <= 301.0, "clip {id} runs to {last} s");
                for p in pts {
                    let g = p[4].as_f64().unwrap();
                    assert!((0.0..=2.0).contains(&g), "clip {id}: gain {g}");
                }
            }
            "fixed" => {
                assert!(e["exact"].as_bool().unwrap(), "strike {id} must be timed exactly");
                assert!(e["first"].as_f64().unwrap() < 301.0);
                strikes += 1;
            }
            t => panic!("clip {id}: unexpected motion {t}"),
        }
    }
    assert_eq!(groups, HashSet::from(["guide".to_string(), "bowls".to_string()]));
    assert_eq!(strikes, 5, "three bowls and two tingsha");
    for l in v["levers"].as_array().unwrap() {
        for d in l["drives"].as_array().unwrap() {
            assert!(groups.contains(d["group"].as_str().unwrap()), "lever {} drives a missing group", l["key"]);
        }
    }
    // No file anywhere in the scene (nested keys included).
    assert!(!JSON.contains("\"files\"") && !JSON.contains(".wav"));
}

#[test]
fn body_scan_renders_softly() {
    let e = Engine::from_json(48_000.0, JSON).expect("scene builds without files");
    assert_eq!(e.scene_lever_count(), 2);
    e.set_param(param::PLAYING, 1.0);
    let (mut l, mut r) = (vec![0.0f32; 480], vec![0.0f32; 480]);
    let mut blocks = Vec::new();
    let mut peak = 0.0f32;
    for _ in 0..(12.0 * 48_000.0 / 480.0) as usize {
        e.render(&mut l, &mut r);
        let mut s = 0.0f64;
        for x in l.iter().chain(r.iter()) {
            assert!(x.is_finite());
            s += (*x as f64).powi(2);
            peak = peak.max(x.abs());
        }
        blocks.push((s / 960.0).sqrt());
    }
    assert!(peak < 0.891, "peak {peak} over -1 dBFS");
    // The opening strike (at 1 s) sounds.
    let strike: f64 = blocks[110..200].iter().copied().fold(0.0, f64::max);
    assert!(strike > 1e-4, "silent: {strike}");
    // From 2 s on, no 10 ms block is more than 12 dB over the median (no burst, no click).
    let mut sorted: Vec<f64> = blocks[200..].to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median = sorted[sorted.len() / 2];
    let worst = blocks[200..].iter().copied().fold(0.0, f64::max);
    assert!(worst < 4.0 * median, "a block at {:.1} dB over the median", 20.0 * (worst / median).log10());
}
