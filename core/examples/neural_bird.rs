//! Renders neural birds solo (no spatialisation) and measures CPU per voice.
//!
//!   cargo run --release --example neural_bird -- [species] [seconds] [out.wav] [voices] [seed]
//!
//! CPU is reported two ways: averaged over the whole render (birds pause between songs, as in
//! the scene) and only over blocks in which the bird was singing (the worst case).

use std::fs::File;
use std::io::{BufWriter, Write};
use std::time::Instant;

use asmr_core::neural::{self, BirdControls, NeuralBird};

fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let species = args.get(1).cloned().unwrap_or_else(|| "chaffinch".into());
    let seconds: f32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(30.0);
    let path = args.get(3).cloned().unwrap_or_else(|| format!("neural_{species}.wav"));
    let voices: usize = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(1);
    let seed: u64 = args.get(5).and_then(|s| s.parse().ok()).unwrap_or(11);

    let sr = 48_000.0f32;
    let model = neural::builtin(&species).unwrap_or_else(|| panic!("no built-in model {species}"));
    println!("{species}: {} parameters", model.params());
    let mut birds: Vec<NeuralBird> = (0..voices).map(|i| NeuralBird::new(model.clone(), sr, seed + 1000 * i as u64)).collect();
    let controls = BirdControls { activity: 0.8, ..Default::default() };

    let block = 256usize;
    let blocks = (seconds * sr) as usize / block;
    let mut buf = vec![0.0f32; block];
    let mut mix = vec![0.0f32; block];
    let mut pcm: Vec<i16> = Vec::with_capacity(blocks * block);
    let (mut total, mut singing_time, mut singing_blocks) = (0.0f64, 0.0f64, 0usize);
    let mut peak = 0.0f32;
    for _ in 0..blocks {
        mix.iter_mut().for_each(|m| *m = 0.0);
        for b in birds.iter_mut() {
            let t = Instant::now();
            b.render(&mut buf, &controls);
            let dt = t.elapsed().as_secs_f64();
            total += dt;
            if b.is_singing() {
                singing_time += dt;
                singing_blocks += 1;
            }
            for (m, s) in mix.iter_mut().zip(&buf) {
                *m += s / voices as f32;
            }
        }
        for &m in &mix {
            peak = peak.max(m.abs());
            pcm.push((m.clamp(-1.0, 1.0) * 32767.0) as i16);
        }
    }
    let block_s = block as f64 / sr as f64;
    let avg = total / (blocks * voices) as f64 / block_s * 100.0;
    let sing = if singing_blocks > 0 { singing_time / singing_blocks as f64 / block_s * 100.0 } else { 0.0 };
    println!(
        "{path}: {seconds:.0}s, {voices} voice(s), peak {:.1} dBFS\n  CPU per voice: {avg:.3} % of one core on average, {sing:.3} % while singing ({:.0}% of the time singing)",
        20.0 * peak.max(1e-9).log10(),
        100.0 * singing_blocks as f64 / (blocks * voices) as f64,
    );
    write_wav(&path, sr as u32, &pcm)
}

fn write_wav(path: &str, sample_rate: u32, pcm: &[i16]) -> std::io::Result<()> {
    let mut f = BufWriter::new(File::create(path)?);
    let data_len = (pcm.len() * 2) as u32;
    f.write_all(b"RIFF")?;
    f.write_all(&(36 + data_len).to_le_bytes())?;
    f.write_all(b"WAVEfmt ")?;
    f.write_all(&16u32.to_le_bytes())?;
    f.write_all(&1u16.to_le_bytes())?;
    f.write_all(&1u16.to_le_bytes())?;
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
