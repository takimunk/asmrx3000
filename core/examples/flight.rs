//! Offline fly-throughs of flight mode (docs/flight-mode.md).
//!
//!   cargo run --release --example flight -- [seconds] [out_dir] [speeds] [biome] [seed]
//!
//! Renders one binaural WAV per speed (default 60 s at 1.5, 6, 12 and 25 m/s, biome
//! "mixed") plus a "tour" that accelerates from a walk to fast flight and slows down again,
//! and prints CPU (render time / audio time), RMS and peak, and every 10 s what is around:
//! distance flown, altitude, sounding emitters, streamed objects by kind. A "steer" take
//! flies at 10 m/s steered by a simulated head (AirPods): left, right, then turning around.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::time::Instant;

use asmr_core::controls::id;
use asmr_core::flight::FlightDesc;
use asmr_core::{param, telemetry, Engine, ObjectInfo, SceneDesc, SourceInfo, MAX_SOURCES};

fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let seconds: f32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(60.0);
    let dir = args.get(2).cloned().unwrap_or_else(|| "build/flight".into());
    let speeds: Vec<f32> = args.get(3).map(|s| s.split(',').filter_map(|x| x.parse().ok()).collect()).unwrap_or_else(|| vec![1.5, 6.0, 12.0, 25.0]);
    let biome = args.get(4).cloned().unwrap_or_else(|| "mixed".into());
    let seed: u32 = args.get(5).and_then(|s| s.parse().ok()).unwrap_or(0);
    std::fs::create_dir_all(&dir)?;

    println!("{:>8} {:>7} {:>7} {:>8} {:>8}  file", "speed", "cpu %", "x rt", "rms dBFS", "peak");
    let mut takes: Vec<(String, Option<f32>)> = speeds.iter().map(|v| (format!("{dir}/flight_{biome}_{v}ms.wav"), Some(*v))).collect();
    takes.push((format!("{dir}/flight_{biome}_tour.wav"), None));
    takes.push((format!("{dir}/flight_{biome}_steer.wav"), Some(-1.0)));
    for (path, speed) in takes {
        let mut desc = SceneDesc::default_scene();
        desc.flight = Some(FlightDesc {
            start: true,
            speed: Some(match speed {
                Some(v) if v > 0.0 => v,
                Some(_) => 10.0,
                None => 1.5,
            }),
            biome: Some(biome.clone()),
            seed: Some(seed),
            ..Default::default()
        });
        let engine = Engine::with_scene(48_000.0, &desc).unwrap_or_else(|e| panic!("{e}"));
        engine.set_param(param::PLAYING, 1.0);
        engine.set_param(param::VOLUME, 0.8);
        engine.set_param(param::BIRDS, 0.8);

        let block = 512usize;
        let blocks = (seconds * 48_000.0) as usize / block;
        let (mut l, mut r) = (vec![0.0f32; block], vec![0.0f32; block]);
        let mut pcm: Vec<i16> = Vec::with_capacity(blocks * block * 2);
        let (mut peak, mut energy) = (0.0f32, 0.0f64);
        let mut busy = 0.0f64;
        let mut objs = [ObjectInfo::default(); 512];
        let mut src = [SourceInfo::default(); MAX_SOURCES];
        let mut log = String::new();
        for b in 0..blocks {
            let t = b as f32 * block as f32 / 48_000.0;
            if speed.is_none() {
                // Tour: walk, take off, fast flight, glide down to a stroll.
                let u = t / seconds;
                let v = if u < 0.15 { 1.5 } else if u < 0.45 { 1.5 + 23.5 * ((u - 0.15) / 0.3) } else if u < 0.75 { 25.0 } else { 25.0 - 19.0 * ((u - 0.75) / 0.25) };
                engine.set_control(id::FLIGHT_SPEED, v);
            }
            if speed == Some(-1.0) {
                // Steering by head (AirPods): look 40° left for 12 s, ahead, 40° right for
                // 12 s, ahead, then a long look over the left shoulder (turning around).
                let deg: f32 = match t % 60.0 {
                    x if x < 6.0 => 0.0,
                    x if x < 18.0 => 40.0,
                    x if x < 24.0 => 0.0,
                    x if x < 36.0 => -40.0,
                    x if x < 42.0 => 0.0,
                    x if x < 50.0 => 75.0,
                    _ => 0.0,
                };
                let y = deg.to_radians() / 2.0;
                engine.set_head_pose([y.cos(), 0.0, 0.0, y.sin()], [0.0; 3], 0.01);
            }
            let started = Instant::now();
            engine.render(&mut l, &mut r);
            busy += started.elapsed().as_secs_f64();
            for (a, c) in l.iter().zip(&r) {
                peak = peak.max(a.abs()).max(c.abs());
                energy += (*a as f64).powi(2) + (*c as f64).powi(2);
                pcm.push((a.clamp(-1.0, 1.0) * 32767.0) as i16);
                pcm.push((c.clamp(-1.0, 1.0) * 32767.0) as i16);
            }
            if b % (48_000 * 10 / block) == 0 {
                let n = engine.objects(&mut objs);
                let mut kinds = [0usize; 6];
                for o in &objs[..n] {
                    kinds[(o.kind as usize).min(5)] += 1;
                }
                let sounding = engine.sources(&mut src);
                log += &format!(
                    "    t {:>4.0} s  flown {:>6.0} m  heading {:>4.0}°  v {:>4.1} m/s  h {:>4.1} m  emitters {:>2}  objects {:>3} (trees {}, rivers {}, birds {}, sources {}, lakes {}, rocks {})\n",
                    t,
                    engine.telemetry(telemetry::FLIGHT_ODOMETER),
                    engine.telemetry(telemetry::FLIGHT_HEADING).to_degrees(),
                    engine.telemetry(telemetry::FLIGHT_SPEED),
                    engine.telemetry(telemetry::FLIGHT_ALTITUDE),
                    sounding,
                    n,
                    kinds[0],
                    kinds[1],
                    kinds[2],
                    kinds[3],
                    kinds[4],
                    kinds[5]
                );
            }
        }
        let audio = blocks as f64 * block as f64 / 48_000.0;
        let rms = (energy / (2.0 * pcm.len() as f64 / 2.0)).sqrt();
        println!(
            "{:>8} {:>7.2} {:>7.0} {:>8.1} {:>8.3}  {path}",
            match speed {
                None => "tour".to_string(),
                Some(v) if v < 0.0 => "steer".to_string(),
                Some(v) => format!("{v} m/s"),
            },
            100.0 * busy / audio,
            audio / busy,
            20.0 * rms.log10(),
            peak
        );
        print!("{log}");
        write_wav(&path, 48_000, &pcm)?;
    }
    Ok(())
}

fn write_wav(path: &str, sample_rate: u32, pcm: &[i16]) -> std::io::Result<()> {
    let mut w = BufWriter::new(File::create(path)?);
    let data = (pcm.len() * 2) as u32;
    w.write_all(b"RIFF")?;
    w.write_all(&(36 + data).to_le_bytes())?;
    w.write_all(b"WAVEfmt ")?;
    w.write_all(&16u32.to_le_bytes())?;
    w.write_all(&1u16.to_le_bytes())?;
    w.write_all(&2u16.to_le_bytes())?;
    w.write_all(&sample_rate.to_le_bytes())?;
    w.write_all(&(sample_rate * 4).to_le_bytes())?;
    w.write_all(&4u16.to_le_bytes())?;
    w.write_all(&16u16.to_le_bytes())?;
    w.write_all(b"data")?;
    w.write_all(&data.to_le_bytes())?;
    for s in pcm {
        w.write_all(&s.to_le_bytes())?;
    }
    Ok(())
}
