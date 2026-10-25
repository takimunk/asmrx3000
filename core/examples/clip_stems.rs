//! Renders a clip scene's groups as separate stems (for balancing and comfort checks): the
//! whole scene, then each group alone (every other group at its lowest level), with the mix's
//! leveller and compressor off so the stems add up. Head still, 48 kHz stereo 16-bit.
//!
//!   cargo run --release --manifest-path core/Cargo.toml --example clip_stems -- \
//!       <seconds> <scene.json> <out_dir>
//!
//! Prints each stem's RMS and peak; writes `<out_dir>/<group>.wav` and `<out_dir>/all.wav`.

use std::fs::File;
use std::io::{BufWriter, Write};

use asmr_core::clips::group_param;
use asmr_core::controls::id;
use asmr_core::{param, Engine};

fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let seconds: f32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(60.0);
    let scene = std::fs::read_to_string(args.get(2).expect("scene.json"))?;
    let out_dir = args.get(3).cloned().unwrap_or_else(|| "stems".into());
    std::fs::create_dir_all(&out_dir)?;
    let sr = 48_000.0f32;
    let probe = Engine::from_json(sr, &scene).unwrap_or_else(|e| panic!("{e}"));
    let groups: Vec<String> = (0..probe.clip_groups().len()).filter_map(|i| probe.clip_groups().id(i).map(str::to_string)).collect();
    drop(probe);
    let mut takes: Vec<Option<usize>> = vec![None];
    takes.extend((0..groups.len()).map(Some));
    for solo in takes {
        let e = Engine::from_json(sr, &scene).unwrap_or_else(|e| panic!("{e}"));
        e.set_param(param::PLAYING, 1.0);
        e.set_control(id::MIX_DYNAMICS, 0.0);
        if let Some(s) = solo {
            for i in 0..groups.len() {
                if i != s {
                    e.clip_groups().set(i, group_param::LEVEL, -80.0);
                }
            }
        }
        let n = 512;
        let (mut l, mut r) = (vec![0.0f32; n], vec![0.0f32; n]);
        let blocks = (seconds * sr / n as f32) as usize;
        let mut pcm = Vec::with_capacity(blocks * n * 2);
        let (mut e2, mut peak) = (0.0f64, 0.0f32);
        for _ in 0..blocks {
            e.render(&mut l, &mut r);
            for i in 0..n {
                e2 += (l[i] as f64).powi(2) + (r[i] as f64).powi(2);
                peak = peak.max(l[i].abs()).max(r[i].abs());
                pcm.push((l[i].clamp(-1.0, 1.0) * 32767.0) as i16);
                pcm.push((r[i].clamp(-1.0, 1.0) * 32767.0) as i16);
            }
        }
        let name = solo.map_or("all".to_string(), |s| groups[s].clone());
        let rms = (e2 / (2 * blocks * n) as f64).sqrt();
        println!("{name:>12}: rms {:6.1} dBFS, peak {:6.1} dBFS", 20.0 * rms.max(1e-12).log10(), 20.0 * peak.max(1e-9).log10());
        write_wav(&format!("{out_dir}/{name}.wav"), sr as u32, &pcm)?;
    }
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
    f.write_all(&2u16.to_le_bytes())?;
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
