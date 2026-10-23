//! Plans a scene's direction ahead, without audio, in a second: the chronicle of spells and
//! their stretches, a table of what the controls do, and a summary. For designing episodes
//! and scripts (docs/direction.md) and for hearing the effect of a steer before living it.
//!
//! cargo run --release --example direction -- [hours] [scene.json] [key=value ...]
//!   hours            how far ahead (8)
//!   scene.json       the scene (built-in forest)
//!   hour=6.5 seed=7  start hour, seed
//!   weather=0.6 energy=-0.3 change=0.8 clock=1 hold=20 amount=1   starting steer
//!   at=90:weather=1  from minute 90 on: a steer (any of the above)
//!   cue=45:thunderstorm  at minute 45: ask for a spell (or `next`)
//!   every=10         table row every N minutes (hours * 2.5 by default)
//!   csv=out.csv      every minute as CSV

use std::fmt::Write as _;

use asmr_core::direction::{Cue, Direction, Input, Output, Steer, CHANNELS, CHANNEL_NAMES, TICK};
use asmr_core::direction::{BIRDS, GUST, HOUR, HUMIDITY, RAIN, STORM, TEMP, WATER, WIND, WIND_DIR};
use asmr_core::scene::SceneDesc;
use asmr_core::sources::common::wind_speed_ms;

fn clock(h: f32) -> String {
    let m = (h.rem_euclid(24.0) * 60.0).round() as u32 % 1440;
    format!("{:02}:{:02}", m / 60, m % 60)
}

fn set(s: &mut Steer, key: &str, v: f32) -> bool {
    match key {
        "weather" => s.weather = v.clamp(-1.0, 1.0),
        "energy" => s.energy = v.clamp(-1.0, 1.0),
        "change" => s.change = v.clamp(0.0, 1.0),
        "clock" => s.clock = v.clamp(0.0, 120.0),
        "hold" => s.hold_s = v.clamp(0.5, 240.0) * 60.0,
        "amount" => s.amount = v.clamp(0.0, 1.0),
        _ => return false,
    }
    true
}

fn bar(v: f32, full: f32) -> String {
    let n = ((v / full).clamp(0.0, 1.0) * 8.0).round() as usize;
    format!("{:<8}", "▪".repeat(n))
}

fn main() {
    let mut hours = 8.0f32;
    let mut desc = SceneDesc::default_scene();
    let mut opts: Vec<(String, String)> = Vec::new();
    for a in std::env::args().skip(1) {
        if let Some((k, v)) = a.split_once('=') {
            opts.push((k.into(), v.into()));
        } else if let Ok(h) = a.parse::<f32>() {
            hours = h;
        } else {
            let json = std::fs::read_to_string(&a).unwrap_or_else(|e| panic!("{a}: {e}"));
            desc = SceneDesc::from_json(&json).unwrap_or_else(|e| panic!("{a}: {e}"));
        }
    }
    let mut steer = desc.direction.as_ref().map(Steer::from_desc).unwrap_or_else(|| {
        eprintln!("the scene has no direction: running the built-in one");
        desc.direction = Some(Default::default());
        Steer::from_desc(&Default::default())
    });
    let mut later: Vec<(f32, Steer)> = Vec::new();
    let mut cues: Vec<(f32, String)> = Vec::new();
    let (mut every, mut csv) = (None, None);
    for (k, v) in &opts {
        let num = v.parse::<f32>();
        match k.as_str() {
            "hour" => desc.environment.time_of_day = num.expect("hour=H").rem_euclid(24.0),
            "seed" => desc.seed = v.parse().expect("seed=N"),
            "every" => every = Some(num.expect("every=MIN")),
            "csv" => csv = Some(v.clone()),
            "at" => {
                let (m, kv) = v.split_once(':').expect("at=MIN:key=value");
                let (key, val) = kv.split_once('=').expect("at=MIN:key=value");
                later.push((m.parse::<f32>().expect("at minute") * 60.0, {
                    let mut s = Steer { amount: f32::NAN, ..steer };
                    assert!(set(&mut s, key, val.parse().expect("at value")), "unknown steer `{key}`");
                    s
                }));
            }
            "cue" => {
                let (m, name) = v.split_once(':').expect("cue=MIN:episode");
                cues.push((m.parse::<f32>().expect("cue minute") * 60.0, name.into()));
            }
            key => assert!(set(&mut steer, key, num.expect("a number")), "unknown option `{key}`"),
        }
    }
    let mut d = Direction::new(&desc).unwrap_or_else(|e| panic!("{e}")).expect("direction");
    for (_, n) in &cues {
        assert!(n == "next" || d.index_of(n).is_some(), "unknown episode `{n}`");
    }
    let every = every.unwrap_or((hours * 2.5).clamp(5.0, 60.0)) * 60.0;

    let e = &desc.environment;
    let wind = wind_speed_ms(e.initial_wind().unwrap_or(0.45));
    let mut cur = [e.time_of_day, wind, 270.0, 1.15, e.initial_rain_mm_h(), e.storm, e.temperature_c, e.humidity, e.birds.unwrap_or(0.6), e.water.unwrap_or(0.6)];
    println!(
        "{} · {} h from {} · weather {:+.2} energy {:+.2} change {:.2} clock {}x",
        desc.name,
        hours,
        clock(e.time_of_day),
        steer.weather,
        steer.energy,
        steer.change,
        steer.clock
    );
    println!();
    println!("  real   scene  spell / stretch");
    let mut table = String::new();
    let _ = writeln!(table, "  real   scene  wind m/s          dir  gust  rain mm/h       storm  temp  hum   birds water");
    let mut out_csv = String::from("minute,episode,segment");
    for n in CHANNEL_NAMES {
        let _ = write!(out_csv, ",{n}");
    }
    out_csv.push('\n');

    let steps = (hours * 3600.0 / TICK).round() as u64;
    let mut last = (usize::MAX, usize::MAX);
    let mut share = vec![0.0f32; d.episodes().len()];
    let (mut wet, mut thunder, mut max_wind, mut max_rain) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    let mut out = Output::default();
    for k in 0..steps {
        let t = k as f32 * TICK;
        let mut inp = Input::new(steer, cur);
        while let Some(i) = later.iter().position(|(at, _)| *at <= t) {
            let (_, s) = later.remove(i);
            // Only the field the option named differs from the steer it was parsed against.
            steer = Steer {
                weather: if s.weather != steer.weather { s.weather } else { steer.weather },
                energy: if s.energy != steer.energy { s.energy } else { steer.energy },
                change: if s.change != steer.change { s.change } else { steer.change },
                clock: if s.clock != steer.clock { s.clock } else { steer.clock },
                hold_s: if s.hold_s != steer.hold_s { s.hold_s } else { steer.hold_s },
                amount: if s.amount.is_nan() { steer.amount } else { s.amount },
            };
            println!("{:>5.0}m         ↳ steer: weather {:+.2} energy {:+.2} change {:.2} clock {}x", t / 60.0, steer.weather, steer.energy, steer.change, steer.clock);
            inp.steer = steer;
        }
        if let Some(i) = cues.iter().position(|(at, _)| *at <= t) {
            let (_, name) = cues.remove(i);
            inp.cue = Some(if name == "next" { Cue::Next } else { Cue::Episode(d.index_of(&name).unwrap()) });
            println!("{:>5.0}m         ↳ cue: {name}", t / 60.0);
        }
        out = *d.step(TICK, &inp);
        for c in 0..CHANNELS {
            if out.write[c] {
                cur[c] = out.value[c];
            }
        }
        let info = d.info(&steer);
        share[info.episode] += TICK;
        wet += if cur[RAIN] > 0.2 { TICK } else { 0.0 };
        thunder += if cur[STORM] > 0.1 { TICK } else { 0.0 };
        max_wind = max_wind.max(cur[WIND]);
        max_rain = max_rain.max(cur[RAIN]);
        if (info.episode, info.segment) != last {
            let ep = &d.episodes()[info.episode].name;
            let next = &d.episodes()[info.next].name;
            let head = if info.segment == 0 || last.0 != info.episode { format!("{ep:<13}") } else { " ".repeat(13) };
            println!(
                "{:>5.0}m  {}  {head} {:<22} {:>4.0} min{}",
                t / 60.0,
                clock(cur[HOUR]),
                d.segment_label(),
                info.segment_left_s / 60.0,
                if info.segment + 1 == info.segments { format!("   → {next}{}", if info.cued { " (asked)" } else { "" }) } else { String::new() }
            );
            last = (info.episode, info.segment);
        }
        let step = (every / TICK).round() as u64;
        if k % step == 0 {
            let _ = writeln!(
                table,
                "{:>5.0}m  {}  {:>4.1} {} {:>4.0}°  {:.2}  {:>5.1} {} {:.2}  {:>4.1}  {:.2}  {:.2}  {:.2}",
                t / 60.0,
                clock(cur[HOUR]),
                cur[WIND],
                bar(cur[WIND], 12.0),
                cur[WIND_DIR],
                cur[GUST],
                cur[RAIN],
                bar(cur[RAIN].sqrt(), 6.0),
                cur[STORM],
                cur[TEMP],
                cur[HUMIDITY],
                cur[BIRDS],
                cur[WATER]
            );
        }
        if csv.is_some() && k % (60.0 / TICK).round() as u64 == 0 {
            let _ = write!(out_csv, "{:.0},{},{}", t / 60.0, d.episodes()[info.episode].name, d.segment_label());
            for v in cur {
                let _ = write!(out_csv, ",{v:.3}");
            }
            out_csv.push('\n');
        }
    }
    println!();
    print!("{table}");
    println!();
    let total = hours * 3600.0;
    let mut by: Vec<(usize, f32)> = share.iter().copied().enumerate().filter(|(_, s)| *s > 0.0).collect();
    by.sort_by(|a, b| b.1.total_cmp(&a.1));
    let line: Vec<String> = by.iter().map(|(i, s)| format!("{} {:.0}%", d.episodes()[*i].name, 100.0 * s / total)).collect();
    println!("time by spell: {}", line.join(", "));
    println!(
        "rain {:.0}% of the time (max {:.1} mm/h), thunder {:.0}%, strongest wind {:.1} m/s, liveliness now {:.2}",
        100.0 * wet / total,
        max_rain,
        100.0 * thunder / total,
        max_wind,
        out.liveliness
    );
    if let Some(path) = csv {
        std::fs::write(&path, out_csv).unwrap_or_else(|e| panic!("{path}: {e}"));
        println!("csv: {path}");
    }
}
