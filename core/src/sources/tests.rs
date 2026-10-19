//! Cross-model tests: every preset of every model under several weathers.

use super::common::{wind_slider, wind_speed_ms};
use super::*;

const SR: f32 = 48_000.0;

fn weathers() -> [(&'static str, Controls); 5] {
    let base = |gust: f32, env: Ambient| Controls { gust, water: 0.6, env, ..Default::default() };
    [
        ("calm", base(0.0, Ambient { hour: 23.0, temperature_c: 20.0, ..Default::default() })),
        ("breeze", base(0.45, Ambient { wind_ms: wind_speed_ms(0.45), hour: 13.0, temperature_c: 25.0, ..Default::default() })),
        (
            "rain",
            base(
                0.6,
                Ambient { wind_ms: wind_speed_ms(0.6), rain_mm_h: 8.0, leaf_wetness: 1.0, ground_wetness: 1.0, puddles: 0.4, drip_mm_h: 3.0, hour: 21.0, ..Default::default() },
            ),
        ),
        ("storm", base(1.6, Ambient { wind_ms: wind_speed_ms(1.2), rain_mm_h: 60.0, leaf_wetness: 1.0, ground_wetness: 1.0, puddles: 0.6, drip_mm_h: 10.0, storm: 1.0, strike: 1, strike_distance_m: 900.0, strike_strokes: 3, ..Default::default() })),
        ("after_rain", base(0.3, Ambient { wind_ms: wind_speed_ms(0.3), leaf_wetness: 0.8, ground_wetness: 0.9, puddles: 0.2, drip_mm_h: 0.6, dryness: 0.9, hour: 6.0, ..Default::default() })),
    ]
}

fn all_presets() -> Vec<SoundDesc> {
    ModelKind::ALL.iter().flat_map(|&m| m.presets().iter().map(move |p| SoundDesc::new(m, p))).collect()
}

/// Renders `seconds`, returns (rms, peak) and checks every sample.
fn run(g: &mut dyn Generator, c: &Controls, seconds: f32) -> (f32, f32) {
    let mut buf = [0.0f32; 64];
    let blocks = (seconds * SR / 64.0) as usize;
    let (mut e, mut peak) = (0.0f64, 0.0f32);
    for _ in 0..blocks {
        g.render(&mut buf, c);
        for &v in &buf {
            assert!(v.is_finite(), "non-finite sample");
            assert!(v == 0.0 || v.abs() >= f32::MIN_POSITIVE, "subnormal sample {v:e}");
            e += (v as f64).powi(2);
            peak = peak.max(v.abs());
        }
    }
    (((e / (blocks * 64) as f64).sqrt()) as f32, peak)
}

#[test]
fn every_preset_is_finite_bounded_and_deterministic() {
    for d in all_presets() {
        for (name, c) in weathers() {
            let mut a = d.build(SR, 42).unwrap();
            let (rms, peak) = run(a.as_mut(), &c, 1.5);
            assert!(peak < 8.0, "{:?}/{:?} in {name}: peak {peak}", d.model, d.preset);
            assert!(rms < 2.0, "{:?}/{:?} in {name}: rms {rms}", d.model, d.preset);
            // Same seed, same output.
            let mut b = d.build(SR, 42).unwrap();
            let (rms2, peak2) = run(b.as_mut(), &c, 1.5);
            assert_eq!((rms, peak), (rms2, peak2), "{:?}/{:?} not deterministic", d.model, d.preset);
        }
    }
}

#[test]
fn quiet_weather_is_quiet_and_rain_is_heard() {
    let calm = weathers()[0].1;
    let rain = weathers()[2].1;
    for p in ["on_leaves", "on_soil", "on_water", "under_canopy", "tin_roof"] {
        let d = SoundDesc::new(ModelKind::Rain, p);
        let (dry, _) = run(d.build(SR, 1).unwrap().as_mut(), &calm, 1.0);
        let (wet, _) = run(d.build(SR, 1).unwrap().as_mut(), &rain, 1.0);
        assert_eq!(dry, 0.0, "{p} makes sound without rain");
        assert!(wet > 1e-3, "{p}: rain rms {wet}");
    }
    // Wind models are silent in still air.
    for p in Wind::PRESETS {
        let (rms, _) = run(SoundDesc::new(ModelKind::Wind, p).build(SR, 1).unwrap().as_mut(), &calm, 1.0);
        assert!(rms < 1e-6, "{p}: {rms}");
    }
}

#[test]
fn rain_gets_louder_with_rate_and_drips_continue_after() {
    let level = |mm_h: f32, drip: f32| {
        let c = Controls { env: Ambient { rain_mm_h: mm_h, drip_mm_h: drip, leaf_wetness: 1.0, ground_wetness: 1.0, ..Default::default() }, ..Default::default() };
        run(Rain::new(SR, 3, Rain::under_canopy()).as_mut_gen(), &c, 3.0).0
    };
    let (light, heavy) = (level(1.0, 0.0), level(20.0, 0.0));
    assert!(heavy > 2.0 * light, "{light} -> {heavy}");
    // Rain stopped, canopy still draining: drips alone are audible.
    assert!(level(0.0, 0.5) > 1e-4);
}

#[test]
fn aeolian_wire_tone_matches_strouhal() {
    // Fence wire (d = 3 mm, f₁ = 45 Hz) at 6 m/s: shedding at 0.2·6/0.003 = 400 Hz; the
    // singing mode is the harmonic of 45 Hz nearest to it (405 Hz).
    let gust = wind_slider(6.0);
    let c = Controls { gust, env: Ambient { wind_ms: 6.0, ..Default::default() }, ..Default::default() };
    let mut w = Wind::new(SR, 7, WindConfig { body: 0.0, gust_response: 0.0, ..Wind::fence_wire() });
    let mut buf = vec![0.0f32; 48_000];
    let mut warm = [0.0f32; 64];
    for _ in 0..1500 {
        w.render(&mut warm, &c);
    }
    let mut speed = 0.0;
    for chunk in buf.chunks_mut(64) {
        w.render(chunk, &c);
        speed += w.speed() / 750.0;
    }
    let fs = 0.2 * speed / 0.003;
    let mag = |f: f32| {
        let (mut re, mut im) = (0.0f64, 0.0f64);
        for (i, v) in buf.iter().enumerate() {
            let p = core::f64::consts::TAU * f as f64 * i as f64 / SR as f64;
            re += *v as f64 * p.cos();
            im += *v as f64 * p.sin();
        }
        re * re + im * im
    };
    let mut best = (0.0, 0.0);
    let mut f = 100.0;
    while f < 1500.0 {
        let m = mag(f);
        if m > best.1 {
            best = (f, m);
        }
        f += 5.0;
    }
    assert!((best.0 - fs).abs() < 0.12 * fs + 25.0, "peak {} Hz, Strouhal {fs} Hz (U = {speed})", best.0);
}

#[test]
fn parameter_changes_glide_without_clicks() {
    // Abrupt preset switches: the largest sample-to-sample jump right after the switch is
    // no larger than in the steady state of the new setting.
    fn jumps(g: &mut dyn Generator, c: &Controls, blocks: usize) -> f32 {
        let mut buf = [0.0f32; 64];
        let mut prev = 0.0f32;
        let mut m = 0.0f32;
        for _ in 0..blocks {
            g.render(&mut buf, c);
            for &v in &buf {
                m = m.max((v - prev).abs());
                prev = v;
            }
        }
        m
    }
    let c = weathers()[2].1;
    let mut w = Wind::new(SR, 1, Wind::breeze());
    jumps(&mut w, &c, 750);
    w.set_config(Wind::storm());
    let switch = jumps(&mut w, &c, 15);
    let steady = jumps(&mut w, &c, 1500);
    assert!(switch <= steady * 1.5 + 1e-4, "wind: switch {switch} vs steady {steady}");

    let mut f = Foliage::new(SR, 1, Foliage::birch());
    jumps(&mut f, &c, 750);
    f.set_config(Foliage::autumn_oak());
    let switch = jumps(&mut f, &c, 15);
    let steady = jumps(&mut f, &c, 1500);
    assert!(switch <= steady * 1.5 + 1e-4, "foliage: switch {switch} vs steady {steady}");

    let mut r = Water::new(SR, 1, Water::brook_pebbles());
    jumps(&mut r, &c, 750);
    r.set_config(Water::rapids());
    let switch = jumps(&mut r, &c, 15);
    let steady = jumps(&mut r, &c, 1500);
    assert!(switch <= steady * 1.5 + 1e-4, "water: switch {switch} vs steady {steady}");
}

#[test]
fn registry_validates_presets_and_params() {
    let ok = SoundDesc::new(ModelKind::Water, "rapids").with("flow_ms", 3.0);
    match ok.resolve().unwrap() {
        ModelConfig::Water(c) => assert_eq!(c.flow_ms, 3.0),
        _ => panic!(),
    }
    assert!(SoundDesc::new(ModelKind::Water, "jacuzzi").resolve().is_err());
    assert!(SoundDesc::new(ModelKind::Water, "rapids").with("flow_ms", 60.0).resolve().is_err());
    assert!(SoundDesc::new(ModelKind::Rain, "on_soil").with("colour", "red").resolve().is_err());
    assert!(SoundDesc::new(ModelKind::Insects, "field_cricket").with("count", 40).resolve().is_err());
    let json = r#"{"model":"foliage","preset":"aspen","params":{"exposure":1.4}}"#;
    let d: SoundDesc = serde_json::from_str(json).unwrap();
    assert!(d.build(SR, 1).is_ok());
    // Every preset of every model resolves and validates.
    for d in all_presets() {
        d.resolve().unwrap_or_else(|e| panic!("{e}"));
    }
}

trait AsGen {
    fn as_mut_gen(&mut self) -> &mut dyn Generator;
}
impl<T: Generator> AsGen for T {
    fn as_mut_gen(&mut self) -> &mut dyn Generator {
        self
    }
}

#[test]
fn bed_and_individual_events_carry_the_same_energy() {
    // Campbell's theorem: the level must not depend on how many events are rendered one by
    // one versus folded into the noise bed.
    let rain = Controls { env: Ambient { rain_mm_h: 6.0, ..Default::default() }, ..Default::default() };
    let rms_rain = |cap: f32| {
        let cfg = RainConfig { resolved_per_s: cap, area_m2: 2.0, ..Rain::on_soil() };
        run(&mut Rain::new(SR, 9, cfg), &rain, 6.0).0
    };
    let (few, many) = (rms_rain(20.0), rms_rain(2000.0));
    assert!((20.0 * (few / many).log10()).abs() < 2.0, "rain: {few} vs {many}");

    let brook = Controls { water: 0.6, ..Default::default() };
    let rms_water = |cap: f32| {
        let mut w = Water::new(SR, 9, Water::brook_pebbles());
        w.set_resolved_per_s(cap);
        run(&mut w, &brook, 6.0).0
    };
    let (few, many) = (rms_water(60.0), rms_water(1000.0));
    assert!((20.0 * (few / many).log10()).abs() < 2.0, "water: {few} vs {many}");
}
