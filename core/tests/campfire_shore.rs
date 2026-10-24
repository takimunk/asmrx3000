//! The campfire-by-the-sea scene holds together: it parses, every lever drives a group the scene
//! has, every direction segment names only the scene's levers within their range, the episode
//! links resolve, and every source sits above the ground plane (a source under it is lost to the
//! ground effect). When the sound files are present (they are not in git), the engine builds it
//! and it sounds.

use std::collections::HashSet;
use std::path::Path;

use asmr_core::{param, Engine, SceneDesc};
use serde_json::Value;

const JSON: &str = include_str!("../scenes/campfire_shore.json");

#[test]
fn campfire_shore_is_consistent() {
    SceneDesc::from_json(JSON).expect("scene parses");
    let v: Value = serde_json::from_str(JSON).unwrap();
    let ground = v["environment"]["ground_z"].as_f64().unwrap();
    let mut groups = HashSet::new();
    for e in v["entities"].as_array().unwrap() {
        groups.insert(e["group"].as_str().unwrap().to_string());
        let m = &e["motion"];
        let pts: Vec<&Value> = match m["type"].as_str().unwrap() {
            "fixed" => vec![&m["position"]],
            "pass" => vec![&m["from"], &m["to"]],
            t => panic!("unexpected motion {t}"),
        };
        for p in pts {
            let z = p[2].as_f64().unwrap();
            assert!(z > ground, "clip {} at z {z} is under the ground ({ground})", e["id"]);
        }
    }
    let mut levers = std::collections::HashMap::new();
    for l in v["levers"].as_array().unwrap() {
        for d in l["drives"].as_array().unwrap() {
            assert!(groups.contains(d["group"].as_str().unwrap()), "lever {} drives a missing group", l["key"]);
        }
        levers.insert(l["key"].as_str().unwrap().to_string(), (l["min"].as_f64().unwrap(), l["max"].as_f64().unwrap()));
    }
    assert_eq!(levers.len(), 4);
    let dir = &v["direction"];
    assert_eq!(dir["climate"], Value::Bool(false));
    let names: HashSet<&str> = dir["episodes"].as_array().unwrap().iter().map(|e| e["name"].as_str().unwrap()).collect();
    assert!(names.contains(dir["start"].as_str().unwrap()));
    for ep in dir["episodes"].as_array().unwrap() {
        for a in ep["after"].as_array().into_iter().flatten() {
            assert!(names.contains(a.as_str().unwrap()), "{}: after {a}", ep["name"]);
        }
        if let Some(t) = ep["then"].as_str() {
            assert!(names.contains(t), "{}: then {t}", ep["name"]);
        }
        for s in ep["segments"].as_array().unwrap() {
            for (k, x) in s["levers"].as_object().unwrap() {
                let (lo, hi) = levers.get(k).unwrap_or_else(|| panic!("{}: unknown lever {k}", ep["name"]));
                let x = x.as_f64().unwrap();
                assert!((*lo..=*hi).contains(&x), "{}: {k} = {x}", ep["name"]);
            }
        }
    }
}

#[test]
fn campfire_shore_sounds_when_its_files_are_there() {
    let sounds = Path::new(env!("CARGO_MANIFEST_DIR")).join("../assets/sounds/generated/campfire_shore");
    if !sounds.join("campfire_crackle_ccby.wav").exists() {
        eprintln!("campfire_shore sounds not fetched; skipping the render check");
        return;
    }
    let e = Engine::from_json(48_000.0, JSON).expect("scene builds");
    assert_eq!(e.scene_lever_count(), 4);
    e.set_param(param::PLAYING, 1.0);
    let (mut l, mut r) = (vec![0.0f32; 512], vec![0.0f32; 512]);
    let mut sum = 0.0f64;
    let mut peak = 0.0f32;
    for b in 0..(20.0 * 48_000.0 / 512.0) as usize {
        e.render(&mut l, &mut r);
        if b > 100 {
            for x in l.iter().chain(r.iter()) {
                sum += (*x as f64).powi(2);
                peak = peak.max(x.abs());
            }
        }
    }
    assert!(sum > 0.0 && peak < 1.0, "silent or clipping: energy {sum}, peak {peak}");
}
