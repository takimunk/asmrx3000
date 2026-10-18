//! Offline renderer: the main tool for listening to engine changes without the app.
//!
//!   cargo run --release --example render -- [seconds] [out.wav] [mode] [scene.json]
//!
//! Modes:
//! * `turn`  slowly sweeps the head yaw +-90 deg so you can check that the scene stays put.
//! * `flyby` runs the built-in spatial demo scene (fly-bys, a flock changing trees, an
//!   approach/recede, circling) and prints the director's timeline plus a per-second
//!   left/right level trace.
//! * `levels` prints the weather and an approximate at-ear level per source kind every 5 s.
//! * `weather` runs the default scene at dusk through a weather change: calm with crickets →
//!   wind rising → thunderstorm and heavy rain while it cools → rain stops, the canopy keeps
//!   dripping, a gust shakes down a shower → crickets resume, slower. Weather stores run 20×
//!   faster so the drying fits in the take. Prints the weather and levels every 5 s.
//!
//! A scene file given as the 4th argument replaces the default (or demo) scene.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::time::Instant;

use asmr_core::scene::DEMO_SCENE_JSON;
use asmr_core::{object_flags, param, telemetry, Engine, ObjectInfo, ObjectKind, SceneDesc, SourceInfo, MAX_OBJECTS, MAX_SOURCES};

fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let seconds: f32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(20.0);
    let path = args.get(2).cloned().unwrap_or_else(|| "render.wav".into());
    let mode = args.get(3).map(String::as_str).unwrap_or("");
    let turn = mode == "turn";
    let flyby = mode == "flyby";
    let weather = mode == "weather";
    let levels = weather || mode == "levels";

    let sr = 48_000.0f32;
    let scene = match args.get(4) {
        Some(file) => std::fs::read_to_string(file)?,
        None if flyby => DEMO_SCENE_JSON.to_string(),
        None => asmr_core::scene::DEFAULT_SCENE_JSON.to_string(),
    };
    let mut desc = SceneDesc::from_json(&scene).unwrap_or_else(|e| panic!("{e}"));
    if weather {
        desc.environment.time_of_day = 20.5;
        desc.environment.temperature_c = 22.0;
        desc.environment.weather_time_scale = 20.0;
        desc.environment.wind = Some(0.08);
    }
    let engine = Engine::with_scene(sr, &desc).unwrap_or_else(|e| panic!("{e}"));
    engine.set_param(param::PLAYING, 1.0);
    engine.set_param(param::VOLUME, 0.8);
    println!("scene: {} ({} entities, {:.1} h)", desc.name, desc.entities.len(), engine.param(param::TIME_OF_DAY));

    let block = 512usize;
    let blocks = (seconds * sr) as usize / block;
    let mut l = vec![0.0f32; block];
    let mut r = vec![0.0f32; block];
    let mut pcm: Vec<i16> = Vec::with_capacity(blocks * block * 2);
    let (mut peak, mut energy) = (0.0f32, 0.0f64);

    let species: Vec<String> = desc
        .entities
        .iter()
        .filter_map(|e| match e {
            asmr_core::scene::EntityDesc::Bird(b) => Some(format!("{:?}", b.species).to_lowercase()),
            _ => None,
        })
        .collect();
    let mut objs = [ObjectInfo::default(); MAX_OBJECTS];
    let n = engine.objects(&mut objs);
    let mut prev = objs;
    let mut closest = [f32::MAX; MAX_OBJECTS];
    let (mut win_l, mut win_r, mut win_n) = (0.0f64, 0.0f64, 0usize);

    let mut last_flash = 0.0f32;
    let started = Instant::now();
    for b in 0..blocks {
        let t = (b * block) as f32 / sr;
        if turn {
            engine.set_param(param::HEAD_YAW, (t * 0.4).sin() * std::f32::consts::FRAC_PI_2);
        }
        if weather {
            weather_script(&engine, t, seconds);
        }
        engine.render(&mut l, &mut r);
        for (a, c) in l.iter().zip(&r) {
            assert!(a.is_finite() && c.is_finite(), "non-finite sample");
            peak = peak.max(a.abs()).max(c.abs());
            energy += (*a as f64).powi(2) + (*c as f64).powi(2);
            win_l += (*a as f64).powi(2);
            win_r += (*c as f64).powi(2);
            pcm.push((a * 32767.0) as i16);
            pcm.push((c * 32767.0) as i16);
        }
        win_n += block;

        if levels && win_n as f32 >= 5.0 * sr {
            let db = |e: f64| 10.0 * (e / win_n as f64).max(1e-12).log10();
            let tl = |k| engine.telemetry(k);
            println!(
                "{:6.1}s  wind {:4.1} m/s  rain {:5.1} mm/h  leaves wet {:3.0}%  drip {:5.2} mm/h  {:4.1} °C   L {:6.1} dB  R {:6.1} dB",
                t + block as f32 / sr,
                asmr_core::sources::common::wind_speed_ms(engine.param(param::WIND)),
                tl(telemetry::RAIN_RATE),
                tl(telemetry::WETNESS) * 100.0,
                tl(telemetry::DRIP),
                engine.param(param::TEMPERATURE),
                db(win_l),
                db(win_r),
            );
            // Rough at-ear level per source kind (emitter RMS × kind gain / distance).
            let mut src = [SourceInfo::default(); MAX_SOURCES];
            let ns = engine.sources(&mut src);
            let mut kinds = [0.0f64; 7];
            for s in &src[..ns] {
                let d = (s.rel_x * s.rel_x + s.rel_y * s.rel_y + s.rel_z * s.rel_z).sqrt().max(0.7);
                let g = [1.0, 0.6, 0.8, 1.0, 40.0, 1.0, 0.8][s.kind as usize % 7];
                kinds[s.kind as usize % 7] += ((s.level * g / d) as f64).powi(2);
            }
            let names = ["water", "wind", "birds", "rain", "thunder", "fire", "insects"];
            let parts: Vec<String> = (0..7).filter(|&k| k != 5).map(|k| format!("{} {:4.0}", names[k], 10.0 * kinds[k].max(1e-12).log10())).collect();
            println!("          dsp {:4.1}%   {}", engine.telemetry(telemetry::DSP_LOAD) * 100.0, parts.join("  "));
            (win_l, win_r, win_n) = (0.0, 0.0, 0);
        }
        let flash = engine.telemetry(telemetry::LIGHTNING);
        if weather && flash > last_flash + 0.5 {
            println!("{t:6.1}s  ⚡ lightning");
        }
        last_flash = flash;
        if flyby {
            engine.objects(&mut objs);
            let mut bird_no = 0;
            for i in 0..n {
                let (o, p) = (objs[i], prev[i]);
                if o.kind != ObjectKind::Bird as u32 {
                    continue;
                }
                let name = &species[bird_no];
                bird_no += 1;
                let d = (o.x * o.x + o.y * o.y + o.z * o.z).sqrt();
                let az = o.y.atan2(o.x).to_degrees();
                let moving = o.flags & object_flags::MOVING != 0;
                let was_moving = p.flags & object_flags::MOVING != 0;
                let absent = o.flags & object_flags::ABSENT != 0;
                if moving {
                    closest[i] = closest[i].min(d);
                }
                if moving && !was_moving {
                    println!("{t:6.1}s  {name:<12} #{:<4} takes off   at {d:5.1} m, az {az:+5.0}°", o.id);
                } else if !moving && was_moving {
                    let what = if absent { "leaves" } else { "lands" };
                    println!(
                        "{t:6.1}s  {name:<12} #{:<4} {what:<11} at {d:5.1} m, az {az:+5.0}°  (closest {:.1} m)",
                        o.id, closest[i]
                    );
                    closest[i] = f32::MAX;
                } else if !absent && p.flags & object_flags::ABSENT != 0 && !moving {
                    println!("{t:6.1}s  {name:<12} #{:<4} appears     at {d:5.1} m, az {az:+5.0}°", o.id);
                }
            }
            prev = objs;
            if win_n as f32 >= 0.5 * sr {
                let db = |e: f64| 10.0 * (e / win_n as f64).max(1e-12).log10();
                let (dl, dr) = (db(win_l), db(win_r));
                let bar = ((dl - dr) * 2.0).clamp(-20.0, 20.0) as i32;
                let mut s = [b' '; 41];
                s[20] = b'|';
                s[(20 - bar) as usize] = b'*';
                println!("        [{t:5.1}s] L {dl:6.1} dB  R {dr:6.1} dB  {}", String::from_utf8_lossy(&s));
                (win_l, win_r, win_n) = (0.0, 0.0, 0);
            }
        }
    }
    let elapsed = started.elapsed().as_secs_f32();
    let rms = (energy / pcm.len().max(1) as f64).sqrt();

    write_wav(&path, sr as u32, &pcm)?;
    println!(
        "{path}: {seconds:.1}s in {elapsed:.2}s ({:.0}x real time), peak {:.1} dBFS, rms {:.1} dBFS, dsp load {:.1}%",
        seconds / elapsed.max(1e-6),
        20.0 * peak.max(1e-9).log10(),
        20.0 * rms.max(1e-12).log10(),
        engine.telemetry(telemetry::DSP_LOAD) * 100.0,
    );
    Ok(())
}

/// calm → wind rising → storm and rain, cooling → rain stops, dripping, a gust → calm.
/// Times scale with the take length (the script is written for 150 s).
fn weather_script(engine: &Engine, t: f32, seconds: f32) {
    let u = t * 150.0 / seconds;
    let ramp = |a: f32, b: f32, t0: f32, t1: f32| a + (b - a) * ((u - t0) / (t1 - t0)).clamp(0.0, 1.0);
    let wind = if u < 20.0 {
        0.08
    } else if u < 45.0 {
        ramp(0.08, 0.85, 20.0, 45.0)
    } else if u < 100.0 {
        ramp(0.85, 0.6, 45.0, 60.0)
    } else if (118.0..124.0).contains(&u) {
        0.55
    } else {
        ramp(0.6, 0.12, 100.0, 110.0)
    };
    engine.set_param(param::WIND, wind);
    let rain = if u < 48.0 { 0.0 } else if u < 100.0 { ramp(0.0, 14.0, 48.0, 58.0) } else { 0.0 };
    engine.set_param(param::RAIN, rain);
    engine.set_param(param::STORM, if (45.0..98.0).contains(&u) { 1.0 } else { 0.0 });
    engine.set_param(param::TEMPERATURE, ramp(22.0, 15.0, 45.0, 100.0));
    engine.set_param(param::HUMIDITY, if u < 45.0 { 0.65 } else { 0.95 });
}

fn write_wav(path: &str, sample_rate: u32, pcm: &[i16]) -> std::io::Result<()> {
    let mut f = BufWriter::new(File::create(path)?);
    let data_len = (pcm.len() * 2) as u32;
    f.write_all(b"RIFF")?;
    f.write_all(&(36 + data_len).to_le_bytes())?;
    f.write_all(b"WAVEfmt ")?;
    f.write_all(&16u32.to_le_bytes())?;
    f.write_all(&1u16.to_le_bytes())?; // PCM
    f.write_all(&2u16.to_le_bytes())?; // channels
    f.write_all(&sample_rate.to_le_bytes())?;
    f.write_all(&(sample_rate * 4).to_le_bytes())?;
    f.write_all(&4u16.to_le_bytes())?;
    f.write_all(&16u16.to_le_bytes())?;
    f.write_all(b"data")?;
    f.write_all(&data_len.to_le_bytes())?;
    for s in pcm {
        f.write_all(&s.to_le_bytes())?;
    }
    f.flush()
}
