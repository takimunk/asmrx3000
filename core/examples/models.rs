//! Physical sound models on their own (no scene, no spatialisation).
//!
//!   cargo run --release --example models -- bench            CPU, RMS and peak per preset
//!   cargo run --release --example models -- presets out.wav  every preset in turn (mono WAV,
//!                                                            loudness-normalised per preset)
//!   cargo run --release --example models -- scene            generator CPU of the default scene
//!   cargo run --release --example models -- presets out.wav rain   only one model
//!
//! Each preset is rendered in the weather that shows it off (rain models in rain, insects at
//! their singing hour, thunder with its own strikes), at 48 kHz in 64-sample blocks like the
//! engine. CPU is wall time per second of audio on one core.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::time::Instant;

use asmr_core::sources::common::{wind_slider, wind_speed_ms};
use asmr_core::sources::{Ambient, Controls, Generator, ModelKind, SoundDesc, Thunder, ThunderConfig};

const SR: f32 = 48_000.0;
const BLOCK: usize = 64;

/// Weather a preset is shown in.
fn showcase(model: ModelKind, preset: &str) -> Controls {
    let wind = |slider: f32, env: Ambient| Controls { gust: slider, water: 0.6, env: Ambient { wind_ms: wind_speed_ms(slider), ..env }, ..Default::default() };
    let rain = Ambient { rain_mm_h: 6.0, leaf_wetness: 1.0, ground_wetness: 1.0, puddles: 0.3, drip_mm_h: 2.0, ..Default::default() };
    match model {
        ModelKind::Wind => wind(if preset == "storm" { 1.0 } else { 0.55 }, Ambient::default()),
        ModelKind::Foliage => match preset {
            "aspen" => wind(0.25, Ambient::default()),
            "old_pine" => wind(0.9, Ambient::default()),
            "autumn_oak" | "autumn_birch" => wind(0.5, Ambient { dryness: 1.0, ..Default::default() }),
            _ => wind(0.5, Ambient::default()),
        },
        ModelKind::Water => match preset {
            "lake_lapping" | "still_lake" => wind(0.4, Ambient::default()),
            _ => wind(0.2, Ambient::default()),
        },
        ModelKind::Rain => wind(0.3, rain),
        ModelKind::Thunder => wind(0.3, Ambient { strike: 1, strike_distance_m: 2500.0, storm: 1.0, ..Default::default() }),
        ModelKind::Fire => wind(0.2, Ambient::default()),
        ModelKind::Insects => {
            let hour = if matches!(preset, "grasshopper" | "cicada") { 14.0 } else { 23.0 };
            wind(0.1, Ambient { hour, temperature_c: if preset == "cicada" { 28.0 } else { 22.0 }, ..Default::default() })
        }
    }
}

fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("presets") => presets(args.get(2).map(String::as_str).unwrap_or("presets.wav"), args.get(3).map(String::as_str)),
        Some("scene") => {
            scene_cost();
            Ok(())
        }
        _ => {
            bench();
            Ok(())
        }
    }
}

fn bench() {
    println!("{:<9} {:<16} {:>9} {:>9} {:>9}", "model", "preset", "rms dBFS", "peak dB", "cpu %");
    let mut total = 0.0;
    for m in ModelKind::ALL {
        for p in m.presets() {
            let c = showcase(m, p);
            let mut g = SoundDesc::new(m, p).build(SR, 11).unwrap();
            let secs = if m == ModelKind::Thunder { 12.0 } else { 6.0 };
            let (rms, peak, cpu) = measure(g.as_mut(), &c, secs);
            total += cpu;
            println!("{:<9} {:<16} {:>9.1} {:>9.1} {:>9.3}", m.name(), p, db(rms), db(peak), cpu * 100.0);
        }
    }
    println!("sum of all presets: {:.2} % of one core", total * 100.0);
    let _ = wind_slider(1.0);
}

/// Generator cost of the whole default scene (director + every emitter's generator, no
/// propagation / binaural), dry and in heavy rain with a storm, per model kind.
fn scene_cost() {
    use asmr_core::director::{Director, Environment};
    use asmr_core::scene::{SceneDesc, World};
    for (name, env) in [
        ("dry, 4.8 m/s, 09:00", Environment { hour: 9.0, ..Default::default() }),
        ("dusk, crickets", Environment { hour: 21.0, temperature_c: 22.0, wind: 0.2, ..Default::default() }),
        ("rain 15 mm/h, storm", Environment { hour: 9.0, rain_mm_h: 15.0, storm: 1.0, wind: 0.6, ..Default::default() }),
    ] {
        let desc = SceneDesc::default_scene();
        let mut world = World::build(&desc, SR).unwrap();
        let mut dir = Director::new(&desc, &world);
        dir.weather_mut().set_time_scale(30.0);
        let dt = BLOCK as f32 / SR;
        // Let the weather settle (leaves wet, dripping) before measuring.
        for _ in 0..(40.0 / dt) as usize {
            dir.step(dt, &env, &mut world);
        }
        let mut buf = [0.0f32; BLOCK];
        let mut per_kind = [0.0f64; 8];
        let secs = 10.0;
        let blocks = (secs / dt) as usize;
        let t0 = Instant::now();
        for _ in 0..blocks {
            dir.step(dt, &env, &mut world);
            world.update_emitters();
            for em in world.emitters.iter_mut() {
                let mut c = em.controls;
                c.water = 0.6 * em.flow;
                c.gust = env.wind;
                let t = Instant::now();
                em.generator.render(&mut buf, &c);
                per_kind[em.kind as usize] += t.elapsed().as_secs_f64();
            }
        }
        let total = t0.elapsed().as_secs_f64() / secs as f64;
        let names = ["water", "wind+foliage", "birds", "rain", "thunder", "fire", "insects", "-"];
        let parts: Vec<String> = (0..7).map(|k| format!("{} {:.2}%", names[k], per_kind[k] / secs as f64 * 100.0)).collect();
        println!("{name:<22} total {:.2}% of one core ({} emitters): {}", total * 100.0, world.emitters.len(), parts.join(", "));
    }
}

fn measure(g: &mut dyn Generator, c: &Controls, secs: f32) -> (f32, f32, f32) {
    let blocks = (secs * SR) as usize / BLOCK;
    let mut buf = [0.0f32; BLOCK];
    let (mut e, mut peak) = (0.0f64, 0.0f32);
    let t = Instant::now();
    for _ in 0..blocks {
        g.render(&mut buf, c);
        for &v in &buf {
            e += (v as f64).powi(2);
            peak = peak.max(v.abs());
        }
    }
    let cpu = t.elapsed().as_secs_f32() / secs;
    (((e / (blocks * BLOCK) as f64).sqrt()) as f32, peak, cpu)
}

fn db(x: f32) -> f32 {
    20.0 * x.max(1e-9).log10()
}

fn presets(path: &str, only: Option<&str>) -> std::io::Result<()> {
    let each = 5.0f32;
    let mut pcm: Vec<i16> = Vec::new();
    let mut t = 0.0;
    for m in ModelKind::ALL {
        if only.is_some_and(|o| o != m.name()) {
            continue;
        }
        for p in m.presets() {
            let c = showcase(m, p);
            let mut g = SoundDesc::new(m, p).build(SR, 5).unwrap();
            let secs = if m == ModelKind::Thunder { 12.0 } else { each };
            if m == ModelKind::Thunder {
                // Skip the r/c of silence between flash and first sound.
                let cfg = match SoundDesc::new(m, p).resolve().unwrap() {
                    asmr_core::sources::ModelConfig::Thunder(t) => t,
                    _ => unreachable!(),
                };
                let mut t = Thunder::new(SR, 5, ThunderConfig { interval_s: 600.0, ..cfg.clone() });
                t.strike(if cfg.distance_m > 0.0 { cfg.distance_m } else { 2500.0 });
                let mut skip = [0.0f32; BLOCK];
                let quiet = Controls::default();
                while t.time_to_first_arrival() > 0.3 {
                    t.render(&mut skip, &quiet);
                }
                g = Box::new(t);
            }
            let blocks = (secs * SR) as usize / BLOCK;
            let mut buf = [0.0f32; BLOCK];
            let mut take = Vec::with_capacity(blocks * BLOCK);
            for _ in 0..blocks {
                g.render(&mut buf, &c);
                take.extend_from_slice(&buf);
            }
            // Physical level as rendered (1 m from the source), then loudness-normalised for
            // listening: -28 dBFS RMS, peaks kept under -1 dBFS.
            let n = take.len();
            let rms = (take.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / n as f64).sqrt() as f32;
            let peak = take.iter().fold(0.0f32, |m, v| m.max(v.abs()));
            let norm = (0.04 / rms.max(1e-9)).min(0.89 / peak.max(1e-9));
            let fade = (0.05 * SR) as usize;
            for (i, v) in take.iter().enumerate() {
                let f = (i.min(n - i) as f32 / fade as f32).min(1.0);
                pcm.push(((v * norm * f).clamp(-1.0, 1.0) * 32767.0) as i16);
            }
            println!("{t:6.1}s  {:<8} {:<16} rms {:6.1} dBFS  peak {:6.1} dBFS  (×{:.1} dB in the WAV)", m.name(), p, db(rms), db(peak), db(norm));
            t += secs;
        }
    }
    write_wav(path, SR as u32, &pcm)?;
    println!("{path}: {t:.0} s");
    Ok(())
}

fn write_wav(path: &str, sample_rate: u32, pcm: &[i16]) -> std::io::Result<()> {
    let mut f = BufWriter::new(File::create(path)?);
    let data_len = (pcm.len() * 2) as u32;
    f.write_all(b"RIFF")?;
    f.write_all(&(36 + data_len).to_le_bytes())?;
    f.write_all(b"WAVEfmt ")?;
    f.write_all(&16u32.to_le_bytes())?;
    f.write_all(&1u16.to_le_bytes())?;
    f.write_all(&1u16.to_le_bytes())?; // mono
    f.write_all(&sample_rate.to_le_bytes())?;
    f.write_all(&(sample_rate * 2).to_le_bytes())?;
    f.write_all(&2u16.to_le_bytes())?;
    f.write_all(&16u16.to_le_bytes())?;
    f.write_all(b"data")?;
    f.write_all(&data_len.to_le_bytes())?;
    for s in pcm {
        f.write_all(&s.to_le_bytes())?;
    }
    f.flush()
}
