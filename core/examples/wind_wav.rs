//! Wind and foliage models driven by the travelling gust field, for listening and for
//! comparison with field recordings (ml/asmr_ml/refcompare.py).
//!
//!   cargo run --release --example wind_wav -- wind forest_roar 6 60 out.wav
//!   cargo run --release --example wind_wav -- foliage oak 6 60 out.wav
//!   cargo run --release --example wind_wav -- mix breeze+oak 6 60 out.wav
//!   cargo run --release --example wind_wav -- wind forest_roar 5 60 out.wav distance_m=60
//!
//! Mono, 48 kHz, 16-bit, normalised to −26 dBFS RMS. The gust field steps per 64-sample
//! block exactly as in the engine (gustiness 1, the source at the listener); `mix` sums a
//! wind and a foliage preset as a crown is heard in the scene.

use std::fs::File;
use std::io::{BufWriter, Write};

use asmr_core::rng::Rng;
use asmr_core::sources::common::{wind_slider, wind_speed_ms};
use asmr_core::sources::{Ambient, Controls, ModelKind, SoundDesc};
use asmr_core::windfield::WindField;

const SR: f32 = 48_000.0;
const BLOCK: usize = 64;

fn main() -> std::io::Result<()> {
    let a: Vec<String> = std::env::args().collect();
    if a.len() < 6 {
        eprintln!("usage: wind_wav <wind|foliage|mix> <preset> <m/s> <seconds> out.wav");
        std::process::exit(2);
    }
    let ms: f32 = a[3].parse().expect("m/s");
    let secs: f32 = a[4].parse().expect("seconds");
    // Extra `key=value` parameters apply to the first model.
    let extra: Vec<(String, f64)> = a[6..].iter().filter_map(|kv| kv.split_once('=').map(|(k, v)| (k.to_string(), v.parse().expect("number")))).collect();
    let build = |m: ModelKind, p: &str, seed: u64| {
        let mut d = SoundDesc::new(m, p);
        if seed == 5 {
            for (k, v) in &extra {
                d = d.with(k, *v);
            }
        }
        d.build(SR, seed).unwrap_or_else(|e| panic!("{e}"))
    };
    let mut gens = Vec::new();
    match a[1].as_str() {
        "wind" => gens.push(build(ModelKind::Wind, &a[2], 5)),
        "foliage" => gens.push(build(ModelKind::Foliage, &a[2], 5)),
        "mix" => {
            let (w, f) = a[2].split_once('+').expect("mix wants wind_preset+foliage_preset");
            gens.push(build(ModelKind::Wind, w, 5));
            gens.push(build(ModelKind::Foliage, f, 6));
        }
        m => panic!("unknown model {m}"),
    }
    let slider = wind_slider(ms);
    let mut field = WindField::new(SR);
    let mut rng = Rng::new(1);
    let dt = BLOCK as f32 / SR;
    let mut buf = [0.0f32; BLOCK];
    let blocks = (secs * SR) as usize / BLOCK;
    let warm = (5.0 * SR) as usize / BLOCK;
    let mut take = Vec::with_capacity(blocks * BLOCK);
    for b in 0..warm + blocks {
        field.step(slider, dt, &mut rng);
        let c = Controls { gust: field.local(0.0), env: Ambient { wind_ms: wind_speed_ms(slider), ..Default::default() }, ..Default::default() };
        let mut sum = [0.0f32; BLOCK];
        for g in gens.iter_mut() {
            g.render(&mut buf, &c);
            for (s, v) in sum.iter_mut().zip(buf.iter()) {
                *s += v;
            }
        }
        if b >= warm {
            take.extend_from_slice(&sum);
        }
    }
    let n = take.len();
    let rms = (take.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / n as f64).sqrt() as f32;
    let peak = take.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    let norm = (0.05 / rms.max(1e-9)).min(0.95 / peak.max(1e-9));
    let mut f = BufWriter::new(File::create(&a[5])?);
    let data_len = (n * 2) as u32;
    f.write_all(b"RIFF")?;
    f.write_all(&(36 + data_len).to_le_bytes())?;
    f.write_all(b"WAVEfmt ")?;
    f.write_all(&16u32.to_le_bytes())?;
    f.write_all(&1u16.to_le_bytes())?;
    f.write_all(&1u16.to_le_bytes())?;
    f.write_all(&(SR as u32).to_le_bytes())?;
    f.write_all(&(SR as u32 * 2).to_le_bytes())?;
    f.write_all(&2u16.to_le_bytes())?;
    f.write_all(&16u16.to_le_bytes())?;
    f.write_all(b"data")?;
    f.write_all(&data_len.to_le_bytes())?;
    for v in take {
        f.write_all(&(((v * norm).clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes())?;
    }
    println!("{}: {secs} s at {ms} m/s, physical rms {:.1} dBFS, file gain {:+.2} dB", a[5], 20.0 * rms.max(1e-9).log10(), 20.0 * norm.log10());
    Ok(())
}
