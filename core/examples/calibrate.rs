//! Source-level calibration of the mix layer (core/src/mix/calibration.rs).
//!
//!   cargo run --release --example calibrate            # measure, compare with the targets
//!
//! Renders every sound class at its reference condition with per-emitter A-weighted probes,
//! prints the measured level next to the target, and the raw (uncalibrated) generator level
//! to paste into `TABLE[..].raw_db`. Metrics: LAFmax at 1 m for bird song, LAeq at 1 m for
//! point sources, LAeq at the listener (1/r² sum over the scene's emitters, +2 dB ground)
//! for distributed ones, loudest clap at the listener for thunder.

use asmr_core::controls::id;
use asmr_core::director::Species;
use asmr_core::mix::calibration::{entry, Metric, TABLE, THUNDER};
use asmr_core::mix::SoundClass;
use asmr_core::{param, Engine};

const SR: f32 = 48_000.0;
/// +2 dB: the incoherent ground reflection the mix analysis also assumes.
const GROUND_DB: f32 = 2.04;

type Probes = Vec<(SoundClass, f32, f32, f64)>;

fn run(scene: &str, setup: &dyn Fn(&Engine), warm_s: f32, secs: f32) -> Probes {
    let e = Engine::from_json(SR, scene).unwrap_or_else(|err| panic!("{err}"));
    e.set_param(param::PLAYING, 1.0);
    // No attention / throttling during calibration: every bird sings freely.
    e.set_control(id::MIX_CLARITY, 0.0);
    setup(&e);
    let (mut l, mut r) = (vec![0.0f32; 512], vec![0.0f32; 512]);
    let blocks = |s: f32| (s * SR / 512.0) as usize;
    for _ in 0..blocks(warm_s) {
        e.render(&mut l, &mut r);
    }
    e.enable_mix_probes();
    for _ in 0..blocks(secs) {
        e.render(&mut l, &mut r);
    }
    e.mix_probes()
}

fn pow_mean(dbs: impl Iterator<Item = f32>) -> f32 {
    let v: Vec<f32> = dbs.collect();
    10.0 * (v.iter().map(|d| 10f32.powf(d / 10.0)).sum::<f32>() / v.len().max(1) as f32).log10()
}

/// Level at the listener of all probes of `class`: Σ 10^(LAeq/10) · mean(1/r²), + ground.
fn at_ear(p: &Probes, class: SoundClass) -> f32 {
    let s: f64 = p.iter().filter(|q| q.0 == class).map(|q| 10f64.powf(q.1 as f64 / 10.0) * q.3).sum();
    10.0 * s.max(1e-30).log10() as f32 + GROUND_DB
}

fn report(class: SoundClass, measured: f32) {
    let e = entry(class);
    let gain_db = 20.0 * class.gain().log10() - 20.0 * e.post.log10();
    let raw = measured - gain_db;
    println!(
        "{:<26} {:<12} target {:5.1}  measured {:5.1}  ({:+5.1} dB)   raw_db {:6.1}   [{}]",
        format!("{class:?}"),
        format!("{:?}", e.metric),
        e.target_db,
        measured,
        measured - e.target_db,
        raw,
        e.condition
    );
}

const BIRDS_SCENE: &str = r#"{
  "name": "calibration birds", "seed": 7,
  "environment": { "time_of_day": 6.0, "temperature_c": 15.0, "wind": 0.0, "water": 0.0, "birds": 1.0 },
  "director": { "flock_interval": [100000, 100001], "first_flock": 100000 },
  "entities": [
    { "kind": "tree", "id": 1, "species": "oak", "position": [6, 0, -1.7], "canopy_height": 5, "foliage": { "model": "foliage", "preset": "oak", "params": { "gain": 0.0 } } },
    BIRDS
  ]
}"#;

fn birds_scene() -> String {
    let birds: Vec<String> = Species::ALL
        .iter()
        .map(|s| s.name())
        .enumerate()
        .map(|(i, s)| format!(r#"{{ "kind": "bird", "id": {}, "species": "{s}", "behaviour": {{ "type": "resident", "home": 1, "roam": 0.01 }} }}"#, 100 + i))
        .collect();
    BIRDS_SCENE.replace("BIRDS", &birds.join(",\n    "))
}

fn single(model: &str, preset: &str, params: &str, env: &str) -> String {
    format!(
        r#"{{ "name": "calibration {model}", "seed": 11, "environment": {{ {env} }},
            "entities": [ {{ "kind": "source", "id": 1, "position": [3, 0, -1.2], "sound": {{ "model": "{model}", "preset": "{preset}", "params": {{ {params} }} }} }} ] }}"#
    )
}

fn main() {
    let default = asmr_core::scene::DEFAULT_SCENE_JSON;
    println!("Calibration (A-weighted, dB SPL). Paste raw_db into mix/calibration.rs::TABLE.\n");

    // Birds, procedural and neural voices, dawn and night.
    for neural in [1.0, 0.0] {
        let mut lafmax: Vec<(SoundClass, f32)> = Vec::new();
        for hour in [5.5f32, 23.5] {
            let p = run(
                &birds_scene(),
                &|e: &Engine| {
                    e.set_param(param::TIME_OF_DAY, hour);
                    e.set_param(param::NEURAL_BIRDS, neural);
                },
                15.0,
                150.0,
            );
            for q in p.iter().filter(|q| matches!(q.0, SoundClass::Bird(_))) {
                match lafmax.iter_mut().find(|x| x.0 == q.0) {
                    Some(x) => x.1 = x.1.max(q.2),
                    None => lafmax.push((q.0, q.2)),
                }
            }
        }
        println!("-- birds, {} voices --", if neural > 0.5 { "neural (where trained)" } else { "procedural" });
        for s in Species::ALL {
            let c = SoundClass::Bird(s);
            match lafmax.iter().find(|x| x.0 == c) {
                Some(x) => report(c, x.1),
                None => println!("{c:?}: silent"),
            }
        }
    }

    println!("-- water, wind, rain (default scene) --");
    let p = run(default, &|e: &Engine| {
        e.set_param(param::BIRDS, 0.0);
        e.set_param(param::WATER, 0.6);
        e.set_param(param::WIND, 0.45);
    }, 10.0, 300.0); // long: wind gusts are intermittent (one every ~25 s at 5 m/s)
    report(SoundClass::Water, pow_mean(p.iter().filter(|q| q.0 == SoundClass::Water).map(|q| q.1)));
    report(SoundClass::Foliage, at_ear(&p, SoundClass::Foliage));
    report(SoundClass::Wind, at_ear(&p, SoundClass::Wind));
    println!("    (water at the listener: {:.1} dB(A))", at_ear(&p, SoundClass::Water));

    let p = run(default, &|e: &Engine| {
        e.set_param(param::BIRDS, 0.0);
        e.set_param(param::RAIN, 4.0);
        e.set_param(param::STORM, 0.0);
    }, 40.0, 40.0);
    report(SoundClass::Rain, at_ear(&p, SoundClass::Rain));

    println!("-- insects, fire --");
    let p = run(&single("insects", "field_cricket", r#""count": 4"#, r#""time_of_day": 22.0, "temperature_c": 22.0"#), &|_| {}, 5.0, 30.0);
    report(SoundClass::Insects, pow_mean(p.iter().filter(|q| q.0 == SoundClass::Insects).map(|q| q.1)));
    let p = run(&single("fire", "campfire", r#""heat_kw": 20"#, r#""time_of_day": 21.0"#), &|_| {}, 5.0, 30.0);
    report(SoundClass::Fire, pow_mean(p.iter().filter(|q| q.0 == SoundClass::Fire).map(|q| q.1)));

    println!("-- thunder (storm 1, 4 minutes) --");
    let p = run(default, &|e: &Engine| {
        e.set_param(param::BIRDS, 0.0);
        e.set_param(param::RAIN, 20.0);
        e.set_param(param::STORM, 1.0);
    }, 5.0, 240.0);
    // The proxy sits at THUNDER_PROXY_M: listener level = 1 m level − 20 log10(r).
    let t = p.iter().filter(|q| q.0 == SoundClass::Thunder).map(|q| q.2 - 10.0 * (1.0 / q.3).log10() as f32).fold(-100.0f32, f32::max);
    report(SoundClass::Thunder, t + GROUND_DB);

    let _ = (TABLE.len(), THUNDER.metric == Metric::LaFmaxScene);
}
