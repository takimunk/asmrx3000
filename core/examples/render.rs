//! Offline renderer: the main tool for listening to engine changes without the app.
//!
//!   cargo run --release --example render -- [seconds] [out.wav] [turn]
//!
//! `turn` slowly sweeps the head yaw +-90 deg so you can check that the scene stays put.
//! `neural` switches the birds to the neural voices (any extra argument may be either word).

use std::fs::File;
use std::io::{BufWriter, Write};
use std::time::Instant;

use asmr_core::{param, telemetry, Engine};

fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let seconds: f32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(20.0);
    let path = args.get(2).cloned().unwrap_or_else(|| "render.wav".into());
    let turn = args.iter().skip(3).any(|s| s == "turn");
    let neural = args.iter().skip(3).any(|s| s == "neural");

    let sr = 48_000.0f32;
    let engine = Engine::new(sr);
    engine.set_param(param::PLAYING, 1.0);
    engine.set_param(param::VOLUME, 0.8);
    engine.set_param(param::NEURAL_BIRDS, if neural { 1.0 } else { 0.0 });

    let block = 512usize;
    let blocks = (seconds * sr) as usize / block;
    let mut l = vec![0.0f32; block];
    let mut r = vec![0.0f32; block];
    let mut pcm: Vec<i16> = Vec::with_capacity(blocks * block * 2);
    let (mut peak, mut energy) = (0.0f32, 0.0f64);

    let started = Instant::now();
    for b in 0..blocks {
        if turn {
            let t = (b * block) as f32 / sr;
            engine.set_param(param::HEAD_YAW, (t * 0.4).sin() * std::f32::consts::FRAC_PI_2);
        }
        engine.render(&mut l, &mut r);
        for (a, c) in l.iter().zip(&r) {
            assert!(a.is_finite() && c.is_finite(), "non-finite sample");
            peak = peak.max(a.abs()).max(c.abs());
            energy += (*a as f64).powi(2) + (*c as f64).powi(2);
            pcm.push((a * 32767.0) as i16);
            pcm.push((c * 32767.0) as i16);
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
