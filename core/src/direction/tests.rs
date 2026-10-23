use super::*;
use crate::scene::SceneDesc;

fn forest() -> SceneDesc {
    SceneDesc::default_scene()
}

fn with_seed(seed: u64) -> SceneDesc {
    let mut d = forest();
    d.seed = seed;
    d
}

/// Runs a direction on its own for `hours` (the controls follow it, nobody touches them);
/// `each` sees every planning step: (seconds, direction, output).
fn run(desc: &SceneDesc, steer: Steer, hours: f32, mut each: impl FnMut(f32, &Direction, &Output)) -> Direction {
    let mut d = Direction::new(desc).unwrap().unwrap();
    let mut current = start_values(desc);
    for k in 1..=(hours * 3600.0 / TICK).round() as u64 {
        let out = *d.step(TICK, &Input::new(steer, current));
        for c in 0..CHANNELS {
            if out.write[c] {
                current[c] = out.value[c];
            }
        }
        each(k as f32 * TICK, &d, &out);
    }
    d
}

fn start_values(desc: &SceneDesc) -> [f32; CHANNELS] {
    let e = &desc.environment;
    let wind = wind_speed_ms(e.initial_wind().unwrap_or(0.45));
    [e.time_of_day, wind, 270.0, gust_of(wind), e.initial_rain_mm_h(), e.storm, e.temperature_c, e.humidity, e.birds.unwrap_or(0.6), e.water.unwrap_or(0.6)]
}

fn steer() -> Steer {
    Steer::from_desc(&DirectionDesc::default())
}

#[test]
fn builtin_catalogue_is_valid_and_the_forest_is_directed() {
    let eps = DirectionDesc::default().catalogue().unwrap();
    let names: Vec<&str> = eps.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, BUILTIN_NAMES);
    assert!(forest().direction.is_some(), "the default forest runs its own course");
    assert_eq!(crate::controls::CUES.len(), BUILTIN_NAMES.len() + 2);
}

#[test]
fn bad_designs_are_rejected_with_a_reason() {
    let bad = |json: &str| {
        let d: DirectionDesc = serde_json::from_str(json).unwrap();
        d.catalogue().unwrap_err()
    };
    assert!(bad(r#"{"episodes": [{"name": "x", "segments": []}]}"#).contains("no segments"));
    assert!(bad(r#"{"episodes": [{"name": "x", "after": ["nope"], "segments": [{}]}]}"#).contains("nope"));
    assert!(bad(r#"{"episodes": [{"name": "x", "segments": [{"rain": 500}]}]}"#).contains("rain"));
    assert!(bad(r#"{"script": [{"episode": "monsoon"}]}"#).contains("monsoon"));
    // A single number is a range of one.
    let d: DirectionDesc = serde_json::from_str(r#"{"only": true, "episodes": [{"name": "fog", "segments": [{"minutes": 30, "wind": 0}]}]}"#).unwrap();
    let eps = d.catalogue().unwrap();
    assert_eq!(eps[0].segments[0].minutes, Some(Span([30.0, 30.0])));
}

/// A day of the default forest: the clock runs, spells change and follow each other the way
/// weather can, values stay physical, every rain is followed by the time after it.
#[test]
fn a_day_runs_on_its_own() {
    let desc = forest();
    let mut seq: Vec<usize> = Vec::new();
    let (mut lo, mut hi) = ([f32::INFINITY; CHANNELS], [f32::NEG_INFINITY; CHANNELS]);
    let d = run(&desc, steer(), 24.0, |_, d, out| {
        let i = d.info(&steer());
        if seq.last() != Some(&i.episode) {
            seq.push(i.episode);
        }
        for c in 0..CHANNELS {
            lo[c] = lo[c].min(out.value[c]);
            hi[c] = hi[c].max(out.value[c]);
        }
    });
    let eps = d.episodes();
    let name = |i: usize| eps[i].name.as_str();
    let names: Vec<&str> = seq.iter().map(|&i| name(i)).collect();
    let distinct = { let mut v = seq.clone(); v.sort(); v.dedup(); v.len() };
    assert!(seq.len() >= 12 && distinct >= 5, "{names:?}");
    for w in seq.windows(2) {
        assert!(d.may_follow(w[0], w[1]) || eps[w[0]].then.as_deref() == Some(name(w[1])), "{} → {}", name(w[0]), name(w[1]));
        if ["shower", "steady_rain", "thunderstorm"].contains(&name(w[0])) {
            assert_eq!(name(w[1]), "after_rain");
        }
    }
    assert!(lo[RAIN] >= 0.0 && hi[RAIN] <= 100.0 && lo[STORM] >= 0.0 && hi[STORM] <= 1.0);
    assert!(lo[HUMIDITY] >= 0.0 && hi[HUMIDITY] <= 1.0 && lo[BIRDS] >= 0.0 && hi[BIRDS] <= 1.0);
    assert!(lo[WIND] >= 0.0 && hi[WIND] < 20.0, "{} {}", lo[WIND], hi[WIND]);
    // The day's cycle: a cool night, a warm afternoon.
    assert!(hi[TEMP] - lo[TEMP] > 6.0, "{} {}", lo[TEMP], hi[TEMP]);
    // A full turn of the clock.
    assert!((d.hour() - desc.environment.time_of_day).abs() < 0.05, "{}", d.hour());
}

/// Rain-hours and storm-minutes over many days: the weather steer moves them a lot.
#[test]
fn the_weather_steer_makes_it_wetter_or_fairer() {
    let wet_share = |w: f32| {
        let mut wet = 0.0;
        let mut total = 0.0;
        for seed in 1..=4 {
            run(&with_seed(seed), Steer { weather: w, ..steer() }, 24.0, |_, _, out| {
                total += 1.0;
                if out.value[RAIN] > 0.2 {
                    wet += 1.0;
                }
            });
        }
        wet / total
    };
    let (fair, mid, wet) = (wet_share(-1.0), wet_share(0.0), wet_share(1.0));
    assert!(fair < mid && mid < wet, "{fair} {mid} {wet}");
    assert!(fair < 0.05 && wet > 0.25, "{fair} {wet}");
}

#[test]
fn the_energy_steer_moves_wind_and_birds_at_once() {
    let mean = |e: f32| {
        let (mut w, mut b, mut n) = (0.0, 0.0, 0.0);
        run(&forest(), Steer { energy: e, ..steer() }, 3.0, |t, _, out| {
            if t > 600.0 {
                w += out.value[WIND];
                b += out.value[BIRDS];
                n += 1.0;
            }
        });
        (w / n, b / n)
    };
    let (still, lively) = (mean(-1.0), mean(1.0));
    assert!(lively.0 > 1.5 * still.0, "{still:?} {lively:?}");
    assert!(lively.1 > still.1 + 0.1, "{still:?} {lively:?}");
}

/// A steady direction changes spells rarely, an eventful one often.
#[test]
fn the_change_steer_sets_the_pace_of_spells() {
    let count = |c: f32| {
        let mut n = 0;
        let mut last = usize::MAX;
        for seed in 1..=3 {
            run(&with_seed(seed), Steer { change: c, ..steer() }, 12.0, |_, d, _| {
                let e = d.info(&steer()).episode;
                if e != last {
                    n += 1;
                    last = e;
                }
            });
        }
        n
    };
    let (steady, eventful) = (count(0.0), count(1.0));
    assert!(eventful as f32 > 1.4 * steady as f32, "{steady} {eventful}");
}

/// A value set by hand is left alone for the hold, then taken back gently; reset hands it
/// back at once.
#[test]
fn the_users_hand_is_held_then_taken_back() {
    let desc = forest();
    let mut d = Direction::new(&desc).unwrap().unwrap();
    let s = Steer { hold_s: 120.0, ..steer() };
    let mut cur = start_values(&desc);
    let step = |d: &mut Direction, cur: &mut [f32; CHANNELS], touch: bool, release: bool| {
        let mut inp = Input::new(s, *cur);
        inp.touched[WIND] = touch;
        inp.released[WIND] = release;
        let out = *d.step(TICK, &inp);
        for c in 0..CHANNELS {
            if out.write[c] {
                cur[c] = out.value[c];
            }
        }
        out
    };
    for _ in 0..600 {
        step(&mut d, &mut cur, false, false);
    }
    // The user sets 12 m/s.
    cur[WIND] = 12.0;
    let out = step(&mut d, &mut cur, true, false);
    assert!(!out.write[WIND]);
    assert!(d.info(&s).held & (1 << WIND) != 0);
    for _ in 0..1150 {
        let out = step(&mut d, &mut cur, false, false);
        assert!(!out.write[WIND]);
    }
    assert_eq!(cur[WIND], 12.0);
    // After the hold it glides back; halfway it is between, at the end it is the direction's.
    for _ in 0..60 {
        step(&mut d, &mut cur, false, false);
    }
    let first = cur[WIND];
    assert!(first < 12.0 && first > 11.0, "{first}");
    for _ in 0..(RETURN_S / TICK) as usize {
        step(&mut d, &mut cur, false, false);
    }
    assert!(cur[WIND] < 7.0, "{}", cur[WIND]);
    assert_eq!(d.info(&s).held, 0);
    // Turning the wind down again and again is remembered as a stiller direction.
    let before = d.info(&s).energy;
    cur[WIND] = 0.2;
    step(&mut d, &mut cur, true, false);
    assert!(d.info(&s).energy < before);
    // Reset: back at once (gliding from the user's value).
    let out = step(&mut d, &mut cur, false, true);
    assert!(out.write[WIND] && (out.value[WIND] - 0.2).abs() < 0.1);
}

/// The clock adopts a time set by hand and goes on from there.
#[test]
fn the_clock_goes_on_from_a_time_set_by_hand() {
    let desc = forest();
    let mut d = Direction::new(&desc).unwrap().unwrap();
    let s = Steer { clock: 60.0, ..steer() };
    let mut cur = start_values(&desc);
    cur[HOUR] = 21.5;
    let mut inp = Input::new(s, cur);
    inp.touched[HOUR] = true;
    d.step(TICK, &inp);
    for _ in 0..600 {
        d.step(TICK, &Input::new(s, cur));
    }
    // One minute at 60x: one more scene hour.
    assert!((d.hour() - 22.5).abs() < 0.02, "{}", d.hour());
    // A standing clock stands.
    let s0 = Steer { clock: 0.0, ..s };
    let h = d.hour();
    for _ in 0..600 {
        d.step(TICK, &Input::new(s0, cur));
    }
    assert_eq!(d.hour(), h);
}

/// Cues `name` on a clear morning and runs two hours; (spells in order, segment labels, the
/// first time thunder / rain over 5 mm/h was heard, the strongest wind and when).
fn cue_from_clear(name: &str) -> (Vec<String>, Vec<String>, Option<f32>, Option<f32>, (f32, f32)) {
    let mut desc = forest();
    desc.direction.as_mut().unwrap().start = Some("clear".into());
    let mut d = Direction::new(&desc).unwrap().unwrap();
    let thunder = d.index_of(name).unwrap();
    let s = steer();
    let mut cur = start_values(&desc);
    let mut inp = Input::new(s, cur);
    inp.cue = Some(Cue::Episode(thunder));
    d.step(TICK, &inp);
    assert!(d.info(&s).cued);
    let mut seen: Vec<(String, String)> = Vec::new();
    let mut first_storm = None;
    let mut first_rain = None;
    let mut gust_peak: (f32, f32) = (0.0, 0.0);
    let mut t = 0.0;
    while t < 2.0 * 3600.0 {
        let out = *d.step(TICK, &Input::new(s, cur));
        for c in 0..CHANNELS {
            if out.write[c] {
                cur[c] = out.value[c];
            }
        }
        t += TICK;
        let e = d.episodes()[d.info(&s).episode].name.clone();
        let l = d.segment_label().to_string();
        if seen.last() != Some(&(e.clone(), l.clone())) {
            seen.push((e.clone(), l.clone()));
        }
        if e == name {
            if cur[STORM] > 0.1 && first_storm.is_none() {
                first_storm = Some(t);
            }
            if cur[RAIN] > 5.0 && first_rain.is_none() {
                first_rain = Some(t);
            }
            if cur[WIND] > gust_peak.0 {
                gust_peak = (cur[WIND], t);
            }
        }
    }
    let mut eps: Vec<String> = seen.iter().map(|(e, _)| e.clone()).collect();
    eps.dedup();
    let labels = seen.iter().filter(|(e, _)| e == name).map(|(_, l)| l.clone()).collect();
    (eps, labels, first_storm, first_rain, gust_peak)
}

/// Asking for a thunderstorm on a clear summer morning (convection needs no overcast): the
/// clear spell wraps up and the storm comes in its order (thunder first, the calm, the gust
/// front, the downpour), then the time after the rain.
#[test]
fn a_cued_storm_comes_the_natural_way() {
    let (eps, labels, storm, rain, gust_peak) = cue_from_clear("thunderstorm");
    assert_eq!(&eps[..3], ["clear", "thunderstorm", "after_rain"], "{eps:?}");
    assert_eq!(labels, ["distant thunder", "calm before the storm", "gust front", "the cell overhead", "trailing rain", "storm moving away"]);
    let (storm, rain) = (storm.unwrap(), rain.unwrap());
    // Within ~10 min of asking, thunder is heard; the rain comes after it and after the gust.
    assert!(storm < 900.0, "{storm}");
    assert!(rain > storm + 300.0 && gust_peak.1 < rain + 60.0 && gust_peak.0 > 7.0, "{storm} {rain} {gust_peak:?}");
}

/// A long rain cannot set in under a clear sky: a short overcast spell comes between.
#[test]
fn a_cue_that_cannot_follow_gets_a_bridge() {
    let (eps, _, _, _, _) = cue_from_clear("steady_rain");
    assert_eq!(&eps[..4], ["clear", "overcast", "steady_rain", "after_rain"], "{eps:?}");
}

/// The brook swells after a downpour, later than the rain, and recedes slowly.
#[test]
fn the_brook_swells_after_heavy_rain() {
    let desc = forest();
    let mut d = Direction::new(&desc).unwrap().unwrap();
    let s = steer();
    let mut cur = start_values(&desc);
    let base = cur[WATER];
    let mut peak = (0.0f32, 0.0f32);
    for k in 0..(5 * 36000) {
        let t = k as f32 * TICK;
        // 30 min of 20 mm/h rain set by hand.
        cur[RAIN] = if t < 1800.0 { 20.0 } else { 0.0 };
        let out = *d.step(TICK, &Input::new(s, cur));
        if out.write[WATER] {
            cur[WATER] = out.value[WATER];
        }
        if cur[WATER] > peak.0 {
            peak = (cur[WATER], t);
        }
    }
    assert!(peak.0 > base + 0.15, "{peak:?}");
    assert!(peak.1 > 2400.0 && peak.1 < 3.0 * 3600.0, "{peak:?}");
    assert!(cur[WATER] > base + 0.02 && cur[WATER] < peak.0, "{} {peak:?}", cur[WATER]);
}

/// A scripted plan plays in order, at the length asked for, then runs free.
#[test]
fn a_script_plays_then_runs_free() {
    let mut desc = forest();
    desc.direction = Some(serde_json::from_str(r#"{"script": [{"episode": "mist", "minutes": 10}, {"episode": "breeze", "minutes": 5}, {"episode": "shower"}]}"#).unwrap());
    let mut log: Vec<(String, f32)> = Vec::new();
    run(&desc, steer(), 3.0, |t, d, _| {
        let e = &d.episodes()[d.info(&steer()).episode].name;
        if log.last().map(|(n, _)| n) != Some(e) {
            log.push((e.clone(), t));
        }
    });
    let names: Vec<&str> = log.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(&names[..4], ["mist", "breeze", "shower", "after_rain"], "{log:?}");
    // The first spell starts part-way through its ten minutes; the breeze lasts five.
    let breeze = log[2].1 - log[1].1;
    assert!(log[1].1 <= 600.0 + 1.0 && (breeze - 300.0).abs() < 90.0, "{log:?}");
    assert!(names.len() > 4);
}

/// `amount` 0 hands everything back; 0.5 halves the deviations.
#[test]
fn amount_scales_and_switches_off() {
    let desc = forest();
    let mut d = Direction::new(&desc).unwrap().unwrap();
    let cur = start_values(&desc);
    let out = *d.step(TICK, &Input::new(steer(), cur));
    assert!(out.write.iter().all(|w| *w));
    let off = *d.step(TICK, &Input::new(Steer { amount: 0.0, ..steer() }, cur));
    assert!(off.write.iter().all(|w| !w));
    assert!(off.release[TEMP] && off.release[STORM] && !off.release[WIND]);
    let again = *d.step(TICK, &Input::new(Steer { amount: 0.0, ..steer() }, cur));
    assert!(!again.release.iter().any(|r| *r));
}

/// Through the engine: the direction moves the controls the sliders show, a slider set by
/// hand is held, a cue reads back "none" and shows as the forecast.
#[test]
fn engine_runs_the_course_and_respects_the_hand() {
    use crate::controls::id::*;
    let e = crate::Engine::new(48_000.0);
    let render = |seconds: f32| {
        let mut l = vec![0.0; 512];
        let mut r = vec![0.0; 512];
        for _ in 0..(seconds * 48_000.0 / 512.0) as usize {
            e.render(&mut l, &mut r);
        }
    };
    let value = |id: u32| {
        let mut c = vec![crate::ControlInfo::default(); crate::controls::COUNT];
        let n = e.controls(&mut c);
        c[..n].iter().find(|c| c.id == id).unwrap().value
    };
    e.set_param(crate::param::PLAYING, 1.0);
    e.set_control(DIRECTION_CLOCK, 60.0);
    let h0 = value(ENV_TIME);
    render(3.0);
    let info = e.direction_info();
    assert_eq!((info.available, info.active), (1, 1));
    assert!(e.direction_episode_name(info.episode as usize).is_some());
    // 3 s at 60x = 3 scene minutes.
    let h1 = value(ENV_TIME);
    assert!((h1 - h0 - 0.05).abs() < 0.01, "{h0} {h1}");
    // A slider set by hand stays.
    e.set_control(WIND_SPEED, 9.0);
    render(2.0);
    assert!((value(WIND_SPEED) - 9.0).abs() < 1e-3);
    assert!(e.direction_info().held & (1 << WIND) != 0);
    // A cue.
    let storm = crate::controls::CUES.iter().position(|c| *c == "thunderstorm").unwrap();
    e.set_control(DIRECTION_CUE, storm as f32);
    render(0.5);
    assert_eq!(value(DIRECTION_CUE), 0.0);
    let info = e.direction_info();
    assert_eq!(info.cued, 1);
    let next = e.direction_episode_name(info.next as usize).unwrap();
    assert!(next == "thunderstorm" || next == "overcast", "{next}");
    // Off: the clock stands where it is.
    e.set_control(DIRECTION_AMOUNT, 0.0);
    render(0.5);
    let h2 = value(ENV_TIME);
    render(2.0);
    assert_eq!(value(ENV_TIME), h2);
    assert_eq!(e.direction_info().active, 0);
}
