//! CI comfort check (docs/comfort.md): a couple of simulated minutes of the built-in forest
//! must have no clicks, no loudness surprise and no peak over −1 dBTP. The full audit (an hour
//! across times of day and weather) is `cargo run --release --example comfort`.
//!
//! COMFORT_TEST_SECONDS overrides the length (default 150 s).

use asmr_core::comfort::{Analyzer, EventKind, Thresholds, Verdict};
use asmr_core::mix::calibration::group;
use asmr_core::{param, Engine, SceneDesc};

#[test]
fn forest_has_no_clicks_or_loudness_surprises() {
    let secs: f32 = std::env::var("COMFORT_TEST_SECONDS").ok().and_then(|v| v.parse().ok()).unwrap_or(150.0);
    let sr = 48_000.0;
    let desc = SceneDesc::from_json(asmr_core::scene::DEFAULT_SCENE_JSON).unwrap();
    let engine = Engine::with_scene(sr, &desc).unwrap();
    engine.set_param(param::PLAYING, 1.0);
    engine.set_param(param::VOLUME, 0.8);
    let th = Thresholds::default();
    let mut a = Analyzer::with_thresholds(sr, &th);
    a.set_groups(&group::NAMES);
    let chunk = 2400;
    let (mut l, mut r) = (vec![0.0f32; chunk], vec![0.0f32; chunk]);
    for _ in 0..(secs * sr) as usize / chunk {
        engine.render(&mut l, &mut r);
        assert!(l.iter().chain(&r).all(|v| v.is_finite()), "non-finite output");
        a.push(&l, &r);
        a.context(&engine.mix_info().group_lufs[..group::NAMES.len()]);
    }
    let rep = a.finish(&th);
    eprintln!("{}", rep.text(4));
    assert_eq!(rep.count(EventKind::Click), 0, "clicks in the forest");
    for name in ["loudness surprise", "sudden onset", "true peak", "max momentary"] {
        let c = rep.check(name).unwrap();
        assert!(c.verdict < Verdict::Fail, "{name}: {:.1} {} (fail at {:.1})", c.value, c.unit, c.limits.1);
    }
}
