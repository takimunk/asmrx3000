//! One thunder strike to a WAV (for comparing with recordings).
//!
//!   cargo run --release --example thunder_wav -- out.wav 1000 [seconds] [seed] [strokes, 0 = intracloud]

use std::io::Write;

use asmr_core::sources::{Controls, Generator, Thunder, ThunderConfig};

fn main() -> std::io::Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let path = a.get(1).cloned().unwrap_or("thunder.wav".into());
    let dist: f32 = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(1000.0);
    let secs: f32 = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(25.0);
    let seed: u64 = a.get(4).and_then(|s| s.parse().ok()).unwrap_or(4);
    let sr = 48_000.0;
    let mut t = Thunder::new(sr, seed, ThunderConfig { distance_m: -1.0, ..Default::default() });
    let strokes: u8 = a.get(5).and_then(|s| s.parse().ok()).unwrap_or(3);
    let t0 = std::time::Instant::now();
    t.strike_with(dist, seed as u32, strokes);
    let strike_us = t0.elapsed().as_secs_f64() * 1e6;
    let first = t.time_to_first_arrival();
    println!("arrivals (first, p10, p50, p90, last): {:?}", t.arrival_spread());
    let mut buf = vec![0.0f32; (secs * sr) as usize];
    let (mut worst, mut total) = (0.0f64, 0.0f64);
    for c in buf.chunks_mut(64) {
        let t0 = std::time::Instant::now();
        t.render(c, &Controls::default());
        let us = t0.elapsed().as_secs_f64() * 1e6;
        worst = worst.max(us);
        total += us;
    }
    println!("strike setup {strike_us:.0} µs, worst 64-sample block {worst:.0} µs (budget 1333), mean {:.2} % of a core", total / (secs as f64 * 1e6) * 100.0);
    let peak = buf.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    let rms = (buf.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / buf.len() as f64).sqrt();
    println!("first arrival {first:.2} s, peak {:.1} dB, rms {:.1} dB", 20.0 * peak.log10(), 20.0 * rms.log10());
    let g = 0.89 / peak.max(1e-9);
    let pcm: Vec<i16> = buf.iter().map(|v| (v * g * 32767.0) as i16).collect();
    let mut f = std::io::BufWriter::new(std::fs::File::create(path)?);
    let n = (pcm.len() * 2) as u32;
    f.write_all(b"RIFF")?;
    f.write_all(&(36 + n).to_le_bytes())?;
    f.write_all(b"WAVEfmt ")?;
    f.write_all(&16u32.to_le_bytes())?;
    f.write_all(&1u16.to_le_bytes())?;
    f.write_all(&1u16.to_le_bytes())?;
    f.write_all(&48_000u32.to_le_bytes())?;
    f.write_all(&96_000u32.to_le_bytes())?;
    f.write_all(&2u16.to_le_bytes())?;
    f.write_all(&16u16.to_le_bytes())?;
    f.write_all(b"data")?;
    f.write_all(&n.to_le_bytes())?;
    for s in pcm {
        f.write_all(&s.to_le_bytes())?;
    }
    Ok(())
}
