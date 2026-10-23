//! Full default scene, one take per water regime (the app's mix: birds, wind, insects, river),
//! for A/B of the water against the rest of the forest.
//!
//!   [SCENE=file.json] cargo run --release --example water_ab -- [seconds] [out_dir] [regime ...]
//!
//! Writes `<out_dir>/<regime>.wav` (stereo 48 kHz) and, with the river alone,
//! `<out_dir>/<regime>_river.wav`.

use std::fs::File;
use std::io::{BufWriter, Write};

use asmr_core::controls::{id, options};
use asmr_core::scene::{EntityDesc, DEFAULT_SCENE_JSON};
use asmr_core::{param, Engine, SceneDesc};

fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let secs: f32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(30.0);
    let dir = args.get(2).cloned().unwrap_or_else(|| ".".into());
    let names: Vec<String> = if args.len() > 3 { args[3..].to_vec() } else { options(id::WATER_REGIME).iter().map(|s| s.to_string()).collect() };
    // SCENE=path.json replaces the default scene.
    let json = std::env::var("SCENE").map(|p| std::fs::read_to_string(p).unwrap()).unwrap_or_else(|_| DEFAULT_SCENE_JSON.to_string());
    let base = SceneDesc::from_json(&json).unwrap();
    for name in &names {
        let idx = options(id::WATER_REGIME).iter().position(|o| o == name).expect("regime") as f32;
        for alone in [false, true] {
            let mut d = base.clone();
            if alone {
                d.entities.retain(|e| matches!(e, EntityDesc::River(_)));
            }
            let e = Engine::with_scene(48_000.0, &d).unwrap();
            e.set_param(param::PLAYING, 1.0);
            e.set_param(param::VOLUME, 0.8);
            e.set_control(id::WATER_REGIME, idx);
            let (mut l, mut r) = (vec![0.0f32; 480], vec![0.0f32; 480]);
            for _ in 0..(5 * 100) {
                e.render(&mut l, &mut r);
            }
            let mut pcm = Vec::new();
            for _ in 0..(secs * 100.0) as usize {
                e.render(&mut l, &mut r);
                for (a, b) in l.iter().zip(&r) {
                    pcm.push(*a);
                    pcm.push(*b);
                }
            }
            let path = format!("{dir}/{name}{}.wav", if alone { "_river" } else { "" });
            write_wav(&path, &pcm)?;
            println!("{path}");
        }
    }
    Ok(())
}

fn write_wav(path: &str, pcm: &[f32]) -> std::io::Result<()> {
    let mut f = BufWriter::new(File::create(path)?);
    let n = (pcm.len() * 4) as u32;
    f.write_all(b"RIFF")?;
    f.write_all(&(36 + n).to_le_bytes())?;
    f.write_all(b"WAVEfmt ")?;
    f.write_all(&16u32.to_le_bytes())?;
    f.write_all(&3u16.to_le_bytes())?;
    f.write_all(&2u16.to_le_bytes())?;
    f.write_all(&48_000u32.to_le_bytes())?;
    f.write_all(&(48_000u32 * 8).to_le_bytes())?;
    f.write_all(&8u16.to_le_bytes())?;
    f.write_all(&32u16.to_le_bytes())?;
    f.write_all(b"data")?;
    f.write_all(&n.to_le_bytes())?;
    for s in pcm {
        f.write_all(&s.to_le_bytes())?;
    }
    f.flush()
}
