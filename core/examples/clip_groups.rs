//! Checks that every clip-group parameter changes the rendered output of a clip scene.
//!
//!   cargo run --release --example clip_groups -- scene.json [seconds]
//!
//! For each group (all others pushed to -30 dB) and each parameter it offers, renders the
//! scene at the parameter's minimum, default and maximum with the same seed and prints the
//! output RMS (dB) and zero-crossing rate (pitch indicator).

use asmr_core::{param, Engine, SceneDesc};

fn render(desc: &SceneDesc, seconds: f32, set: &[(usize, u32, f32)]) -> (f32, f32) {
    let sr = 48_000.0;
    let e = Engine::with_scene(sr, desc).unwrap();
    e.set_param(param::PLAYING, 1.0);
    e.set_param(param::VOLUME, 0.8);
    for &(g, p, v) in set {
        e.clip_groups().set(g, p, v);
    }
    let (mut l, mut r) = (vec![0.0f32; 512], vec![0.0f32; 512]);
    let (mut sum, mut n, mut zc, mut prev) = (0.0f64, 0usize, 0usize, 0.0f32);
    for b in 0..(seconds * sr) as usize / 512 {
        e.render(&mut l, &mut r);
        if b < 50 {
            continue;
        }
        for &x in &l {
            sum += (x as f64).powi(2);
            n += 1;
            if (x >= 0.0) != (prev >= 0.0) {
                zc += 1;
            }
            prev = x;
        }
    }
    (10.0 * (sum / n.max(1) as f64).max(1e-14).log10() as f32, zc as f32 / (n as f32 / sr))
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let desc = SceneDesc::from_json(&std::fs::read_to_string(&a[1]).unwrap()).unwrap();
    let seconds: f32 = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(25.0);
    let probe = Engine::with_scene(48_000.0, &desc).unwrap();
    let g = probe.clip_groups();
    println!("{}: {} groups", desc.name, g.len());
    for i in 0..g.len() {
        let others: Vec<(usize, u32, f32)> = (0..g.len()).filter(|&j| j != i).map(|j| (j, 0, -30.0)).collect();
        println!("group {} ({}), mask {:07b}", i, g.id(i).unwrap(), g.mask(i));
        for p in 0..7u32 {
            if g.mask(i) >> p & 1 == 0 {
                continue;
            }
            let (lo, hi, def) = asmr_core::clips::group_param_range(p, g.model(i));
            let mut line = format!("  param {p}:");
            for v in [lo, def, hi] {
                let mut s = others.clone();
                s.push((i, p, v));
                let (db, z) = render(&desc, seconds, &s);
                line += &format!("  [{v:6.2}] {db:6.1} dB {z:7.0} zc/s");
            }
            println!("{line}");
        }
    }
}
