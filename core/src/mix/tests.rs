//! Sound director tests: calibration, limits (phrases, soloists, duck depth, peaks), intent.

use super::analysis::{duck_targets, Analysis, DuckParams};
use super::calibration::{entry, group, SoundClass};
use super::loudness::{from_db_spl, BANDS};
use super::*;
use crate::controls::id;
use crate::director::Species;
use crate::engine::{param, Engine};

const SR: f32 = 48_000.0;

fn run(e: &Engine, secs: f32, mut each: impl FnMut(&Engine)) {
    let (mut l, mut r) = (vec![0.0f32; 480], vec![0.0f32; 480]);
    for _ in 0..(secs * 100.0) as usize {
        e.render(&mut l, &mut r);
        assert!(l.iter().chain(&r).all(|v| v.is_finite() && v.abs() <= 1.0));
        each(e);
    }
}

fn birds_scene(species: &[&str], roam: f32) -> String {
    let birds: Vec<String> = species
        .iter()
        .enumerate()
        .map(|(i, s)| format!(r#"{{ "kind": "bird", "id": {}, "species": "{s}", "behaviour": {{ "type": "resident", "home": 1, "roam": {roam} }} }}"#, 100 + i))
        .collect();
    format!(
        r#"{{ "name": "mix test", "seed": 5,
             "environment": {{ "time_of_day": 5.5, "wind": 0.0, "water": 0.0, "birds": 1.0 }},
             "director": {{ "flock_interval": [100000, 100001], "first_flock": 100000 }},
             "entities": [ {{ "kind": "tree", "id": 1, "species": "oak", "position": [6, 0, -1.7], "canopy_height": 5 }}, {} ] }}"#,
        birds.join(",")
    )
}

#[test]
fn calibrated_source_levels_match_the_table() {
    // Bird song: LAFmax at 1 m, procedural and neural voices level-matched.
    for neural in [0.0, 1.0] {
        let e = Engine::from_json(SR, &birds_scene(&["chaffinch"], 0.01)).unwrap();
        e.set_param(param::PLAYING, 1.0);
        e.set_param(param::NEURAL_BIRDS, neural);
        e.set_control(id::MIX_CLARITY, 0.0);
        run(&e, 12.0, |_| {});
        e.enable_mix_probes();
        run(&e, 40.0, |_| {});
        let p = e.mix_probes();
        let finch = p.iter().find(|q| q.0 == SoundClass::Bird(Species::Chaffinch)).unwrap();
        let target = entry(SoundClass::Bird(Species::Chaffinch)).target_db;
        assert!((finch.2 - target).abs() < 2.5, "chaffinch (neural {neural}): LAFmax {:.1} vs {target}", finch.2);
    }
    // A brook segment: LAeq at 1 m.
    let e = Engine::new(SR);
    e.set_param(param::PLAYING, 1.0);
    e.set_param(param::BIRDS, 0.0);
    e.set_control(id::MIX_CLARITY, 0.0);
    run(&e, 3.0, |_| {});
    e.enable_mix_probes();
    run(&e, 15.0, |_| {});
    let p = e.mix_probes();
    let water: Vec<f32> = p.iter().filter(|q| q.0 == SoundClass::Water).map(|q| q.1).collect();
    let mean = 10.0 * (water.iter().map(|d| 10f32.powf(d / 10.0)).sum::<f32>() / water.len() as f32).log10();
    let target = entry(SoundClass::Water).target_db;
    assert!((mean - target).abs() < 1.5, "brook segment LAeq {mean:.1} vs {target}");
    // Every class has a finite gain; thunder undoes its proxy distance.
    for c in [SoundClass::Water, SoundClass::Foliage, SoundClass::Wind, SoundClass::Rain, SoundClass::Thunder, SoundClass::Fire, SoundClass::Insects] {
        assert!(c.gain().is_finite() && c.gain() > 0.0, "{c:?}");
    }
    assert!(SoundClass::Thunder.gain() > crate::director::THUNDER_PROXY_M * 0.01);
}

#[test]
fn calibration_puts_birds_above_a_breeze() {
    // Before the director the breeze in the crowns sat ~19 dB above the birds (the "mush");
    // calibrated, a dawn chorus 4–13 m away is well above a 5 m/s rustle.
    let level = |bypass: bool, birds: f32, wind: f32| {
        let e = Engine::new(SR);
        e.set_mix_bypass(bypass);
        e.set_param(param::PLAYING, 1.0);
        e.set_param(param::TIME_OF_DAY, 5.5);
        e.set_param(param::BIRDS, birds);
        e.set_param(param::WIND, wind);
        e.set_param(param::WATER, 0.0);
        e.set_control(id::MIX_DYNAMICS, 0.0);
        run(&e, 4.0, |_| {});
        e.reset_loudness();
        run(&e, 20.0, |_| {});
        e.mix_info().loudness_integrated
    };
    let (birds_old, wind_old) = (level(true, 1.0, 0.0), level(true, 0.0, 0.5));
    let (birds_new, wind_new) = (level(false, 1.0, 0.0), level(false, 0.0, 0.5));
    assert!(wind_old > birds_old + 10.0, "legacy: wind {wind_old:.1} birds {birds_old:.1}");
    assert!(birds_new > wind_new + 6.0, "calibrated: birds {birds_new:.1} wind {wind_new:.1}");
}

#[test]
fn bird_phrases_respect_the_density_limit() {
    let all = ["chaffinch", "robin", "song_thrush", "nightingale", "chaffinch", "robin", "song_thrush", "wood_pigeon"];
    let overlap = |density: f32, bypass: bool| {
        let e = Engine::from_json(SR, &birds_scene(&all, 0.01)).unwrap();
        e.set_mix_bypass(bypass);
        e.set_param(param::PLAYING, 1.0);
        e.set_control(id::MIX_CLARITY, 1.0);
        e.set_control(id::MIX_DENSITY, density);
        run(&e, 12.0, |_| {});
        let limit = Intent { density, ..Intent::default() }.max_phrases().unwrap_or(usize::MAX) as u32;
        let (mut max, mut sum, mut n) = (0u32, 0.0, 0.0);
        run(&e, 40.0, |e| {
            let m = e.mix_info();
            max = max.max(m.phrases);
            sum += m.phrases as f32;
            n += 1.0;
        });
        (max, sum / n, limit)
    };
    let (free_max, free_mean, _) = overlap(0.5, true);
    let (max1, mean1, lim1) = overlap(0.0, false);
    let (max3, _, lim3) = overlap(0.34, false);
    assert_eq!(lim1, 1);
    assert!(max1 <= lim1, "one at a time: {max1}");
    assert!(max3 <= lim3, "limit {lim3}: {max3}");
    assert!(free_max > 2 && free_mean > 2.0 * mean1, "unthrottled {free_max}/{free_mean:.2} vs {mean1:.2}");
    assert!(mean1 > 0.3, "birds still sing: {mean1:.2}");
}

#[test]
fn soloists_stay_within_the_limit() {
    for n in [1.0f32, 2.0] {
        let e = Engine::new(SR);
        e.set_param(param::PLAYING, 1.0);
        e.set_param(param::TIME_OF_DAY, 5.5);
        e.set_param(param::BIRDS, 1.0);
        e.set_control(id::MIX_SOLOISTS, n);
        let mut seen = 0;
        run(&e, 40.0, |e| {
            let m = e.mix_info();
            assert!(m.soloists as f32 <= n, "{} soloists > {n}", m.soloists);
            seen = seen.max(m.soloists);
            for k in 0..MAX_SOLOISTS {
                let s = m.soloist_source[k];
                assert!(s == u32::MAX || (s as usize) < crate::engine::MAX_SOURCES);
            }
        });
        assert!(seen >= 1, "a dawn chorus has a foreground");
    }
}

#[test]
fn ducking_is_narrow_and_bounded() {
    let mut a = Analysis::new(SR, 4);
    // Track 0: broadband bed at 55 dB per band; track 2: a "bird" in the 3.7–4.4 kHz band,
    // 6 dB under the bed there.
    for z in 0..BANDS {
        a.fast[0][z] = from_db_spl(55.0);
    }
    a.fast[2][17] = from_db_spl(49.0);
    for t in 0..a.tracks {
        let p = a.fast[t];
        a.model.excitation(&p, &mut a.exc[t]);
    }
    let p = DuckParams { target_smr_db: 8.0, depth_db: 5.0 };
    let mut out = vec![[0.0f32; BANDS]; a.tracks];
    duck_targets(&a, &[0.0, 0.0, 1.0, 0.0], &[true, false, false, false], &p, &mut out);
    let d = out[0];
    assert!(d[17] <= -4.9 && d[17] >= -5.0 - 1e-4, "full cut where the bird is: {}", d[17]);
    assert!(d.iter().all(|v| *v <= 0.0 && *v >= -5.0 - 1e-4));
    let wide = d.iter().filter(|v| **v < -0.5).count();
    assert!(wide <= 3, "narrowband: {wide} bands cut: {d:?}");
    assert_eq!(d[5], 0.0);
    assert_eq!(d[23], 0.0);
    // The foreground itself is never ducked.
    assert!(out[2].iter().all(|v| *v == 0.0));
}

#[test]
fn storm_stays_comfortable() {
    let e = Engine::new(SR);
    e.set_param(param::PLAYING, 1.0);
    e.set_param(param::VOLUME, 1.0);
    e.set_param(param::RAIN, 40.0);
    e.set_param(param::STORM, 1.0);
    e.set_param(param::WIND, 0.8);
    let target = e.control(id::MIX_LOUDNESS);
    run(&e, 8.0, |_| {});
    e.reset_loudness();
    let (mut peak, mut st_max) = (0.0f32, -70.0f32);
    let (mut l, mut r) = (vec![0.0f32; 480], vec![0.0f32; 480]);
    for _ in 0..4000 {
        e.render(&mut l, &mut r);
        peak = l.iter().chain(&r).fold(peak, |m, v| m.max(v.abs()));
        st_max = st_max.max(e.telemetry(crate::engine::telemetry::LOUDNESS_MOMENTARY));
    }
    assert!(peak <= 0.9, "peak {peak}");
    // Thunder claps get through as events, but the comfort compressor keeps even the
    // loudest 400 ms within ~12 LU of the target.
    assert!(st_max < target + 12.0, "momentary {st_max:.1} vs target {target}");
    let i = e.mix_info().loudness_integrated;
    assert!((i - target).abs() < 3.0, "integrated {i:.1} vs target {target}");
    assert!(e.telemetry(crate::engine::telemetry::MIX_GAIN_DB) < 0.0);
}

#[test]
fn intent_from_the_prompt_moves_the_mix() {
    let water = |focus: f32| {
        let e = Engine::new(SR);
        e.set_param(param::PLAYING, 1.0);
        e.set_param(param::TIME_OF_DAY, 5.5);
        let r = e.apply_patch_json(&format!(r#"{{"glide_s":0,"changes":[{{"key":"mix.focus.water","value":{focus}}},{{"key":"mix.clarity","value":0.8}}]}}"#));
        assert!(r.contains("\"ok\":true"), "{r}");
        let st = e.shared_state();
        let i = Intent::from_controls(&st);
        assert_eq!(i.focus[group::WATER], focus);
        assert!((i.clarity - 0.8).abs() < 1e-6);
        let mut lufs = 0.0;
        run(&e, 15.0, |_| {});
        run(&e, 10.0, |e| lufs += 10f32.powf(e.mix_info().group_lufs[group::WATER] / 10.0));
        10.0 * (lufs / 1000.0).log10()
    };
    let front = water(1.0);
    let back = water(-1.0);
    // ~6 dB (the exact spread depends on how busy the dawn chorus is: the leveller follows
    // the whole scene).
    assert!(front > back + 5.0, "water in front {front:.1} vs behind {back:.1}");
}

#[test]
fn mix_telemetry_through_the_c_abi() {
    let e = Engine::new(SR);
    e.set_param(param::PLAYING, 1.0);
    run(&e, 5.0, |_| {});
    let mut info = MixInfo::default();
    // SAFETY: live engine, writable struct.
    unsafe { crate::asmr_get_mix(&e, &mut info) };
    assert!(info.loudness_short > -60.0 && info.loudness_short < 0.0, "{}", info.loudness_short);
    assert!(info.gain_db.is_finite());
    for g in 0..group::COUNT {
        assert!(info.group_lufs[g].is_finite() && (0.0..=1.0).contains(&info.group_masking[g]) && info.group_duck_db[g] <= 0.0);
    }
    assert!(info.group_lufs[group::WATER] > -60.0, "the brook is heard");
    assert_eq!(core::mem::size_of::<MixInfo>(), 4 * (4 + 32 + 1 + 4 + 4 + 4 + 2 + 4));
}
