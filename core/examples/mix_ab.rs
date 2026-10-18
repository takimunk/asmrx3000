//! A/B renders of the sound director's mix layer (docs/sound-director.md).
//!
//!   cargo run --release --example mix_ab -- [seconds] [out_dir] [scenes]
//!
//! Scenes (comma-separated, default all): `dawn` (dense default scene: dawn chorus with every
//! bird, breeze, brook, light rain), `storm` (downpour, thunder, strong wind, a few birds),
//! `flight` (flight mode over the streamed landscape at dawn). Each is rendered twice from
//! the same seed: **before** (legacy per-kind gains, no attention, ducking or dynamics) and
//! **after** (calibrated levels + the sound director). The same analysis runs in both, so the
//! numbers compare: BS.1770 loudness of the output, per-group level (LUFS share) and masking
//! (fraction of the group's loudness hidden by everything else, Zwicker partial loudness),
//! the masking of the most salient sources (soloists), overlapping bird phrases, and the CPU
//! time of the mix layer. Writes `<out_dir>/<scene>_{before,after}.wav` and
//! `<out_dir>/summary.md`.

use std::fmt::Write as _;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::time::Instant;

use asmr_core::mix::calibration::group;
use asmr_core::{param, Engine};

const SR: f32 = 48_000.0;

#[derive(Default)]
struct Stats {
    seconds: f32,
    integrated: f32,
    short: Vec<f32>,
    max_momentary: f32,
    peak: f32,
    group_pow: [f64; group::COUNT],
    group_mask: [f64; group::COUNT],
    group_w: [f64; group::COUNT],
    group_duck: [f64; group::COUNT],
    solo_before: f64,
    solo_after: f64,
    solo_n: f64,
    smr_before: f64,
    smr_after: f64,
    soloists: f64,
    phrases: f64,
    max_phrases: u32,
    overlap3: f64,
    samples: f64,
    mix_cpu: f64,
    wall: f64,
}

fn setup(e: &Engine, scene: &str) {
    e.set_param(param::PLAYING, 1.0);
    e.set_param(param::VOLUME, 1.0);
    match scene {
        "dawn" => {
            e.set_param(param::TIME_OF_DAY, 5.5);
            e.set_param(param::BIRDS, 1.0);
            e.set_param(param::WIND, 0.5);
            e.set_param(param::WATER, 0.65);
            e.set_param(param::RAIN, 1.0);
        }
        "storm" => {
            e.set_param(param::TIME_OF_DAY, 15.0);
            e.set_param(param::BIRDS, 0.35);
            e.set_param(param::WIND, 0.72);
            e.set_param(param::RAIN, 30.0);
            e.set_param(param::STORM, 0.9);
        }
        // Solo checks of the analysis against the real output.
        "wind_only" | "birds_only" | "water_only" => {
            e.set_param(param::TIME_OF_DAY, 5.5);
            e.set_param(param::BIRDS, if scene == "birds_only" { 1.0 } else { 0.0 });
            e.set_param(param::WIND, if scene == "wind_only" { 0.5 } else { 0.0 });
            e.set_param(param::WATER, if scene == "water_only" { 0.65 } else { 0.0 });
            e.set_param(param::RAIN, 0.0);
        }
        "flight" => {
            e.set_param(param::TIME_OF_DAY, 6.5);
            e.set_param(param::BIRDS, 1.0);
            e.set_param(param::FLIGHT, 1.0);
        }
        _ => panic!("unknown scene {scene}"),
    }
}

fn render(scene: &str, after: bool, secs: f32, path: &str) -> Stats {
    let e = Engine::new(SR);
    e.set_mix_bypass(!after);
    setup(&e, scene);
    e.enable_mix_timing();
    let block = 480usize; // 10 ms
    let blocks = (secs * SR) as usize / block;
    let (mut l, mut r) = (vec![0.0f32; block], vec![0.0f32; block]);
    let mut pcm: Vec<i16> = Vec::with_capacity(blocks * block * 2);
    let mut st = Stats { max_momentary: -70.0, ..Default::default() };
    let trace = std::env::var("MIX_TRACE").is_ok();
    let warm = 500; // 5 s settle (weather, flight cross-fade) before measuring
    let t0 = Instant::now();
    for b in 0..blocks + warm {
        e.render(&mut l, &mut r);
        if b < warm {
            if b + 1 == warm {
                e.reset_loudness();
            }
            continue;
        }
        for (a, c) in l.iter().zip(&r) {
            st.peak = st.peak.max(a.abs()).max(c.abs());
            pcm.push((a.clamp(-1.0, 1.0) * 32767.0) as i16);
            pcm.push((c.clamp(-1.0, 1.0) * 32767.0) as i16);
        }
        if b % 10 == 0 {
            let m = e.mix_info();
            st.short.push(m.loudness_short);
            st.max_momentary = st.max_momentary.max(m.loudness_momentary);
            for g in 0..group::COUNT {
                if m.group_lufs[g] > -69.0 {
                    st.group_pow[g] += 10f64.powf(m.group_lufs[g] as f64 / 10.0);
                }
                let w = m.group_sone[g] as f64;
                st.group_mask[g] += m.group_masking[g] as f64 * w;
                st.group_duck[g] += m.group_duck_db[g] as f64;
                st.group_w[g] += w;
            }
            // Foreground masking, sampled while soloists are on stage.
            if m.soloists > 0 {
                st.solo_before += m.masking_before as f64;
                st.solo_after += m.masking_after as f64;
                st.smr_before += m.smr_before_db as f64;
                st.smr_after += m.smr_after_db as f64;
                st.solo_n += 1.0;
            }
            st.soloists += m.soloists as f64;
            st.phrases += m.phrases as f64;
            st.max_phrases = st.max_phrases.max(m.phrases);
            if m.phrases >= 3 {
                st.overlap3 += 1.0;
            }
            st.samples += 1.0;
            if trace && b % 100 == 0 {
                let mut src = [asmr_core::SourceInfo::default(); asmr_core::MAX_SOURCES];
                let n = e.sources(&mut src);
                let solo: Vec<String> = (0..4)
                    .filter(|&k| (m.soloist_source[k] as usize) < n)
                    .map(|k| {
                        let s = src[m.soloist_source[k] as usize];
                        let d = (s.rel_x * s.rel_x + s.rel_y * s.rel_y + s.rel_z * s.rel_z).sqrt();
                        format!("#{} k{} {:.0}m sal {:.2} mask {:.0}%", m.soloist_source[k], s.kind, d, m.soloist_salience[k], 100.0 * m.soloist_masking[k])
                    })
                    .collect();
                let g: Vec<String> = (0..group::COUNT).filter(|&g| m.group_lufs[g] > -69.0).map(|g| format!("{} {:.0}/{:.0}%/{:.1}", &group::NAMES[g][..3], m.group_lufs[g], 100.0 * m.group_masking[g], m.group_duck_db[g])).collect();
                println!("{:5.1}s st {:5.1} gain {:5.1} phr {}/{} | {} | {}", (b - warm) as f32 / 100.0, m.loudness_short, m.gain_db, m.phrases, m.max_phrases, g.join(" "), solo.join("; "));
            }
        }
    }
    st.wall = t0.elapsed().as_secs_f64();
    st.seconds = secs;
    st.integrated = e.mix_info().loudness_integrated;
    st.mix_cpu = e.mix_cpu_s();
    write_wav(path, &pcm).expect("write wav");
    st
}

fn write_wav(path: &str, pcm: &[i16]) -> std::io::Result<()> {
    let mut w = BufWriter::new(File::create(path)?);
    let data = (pcm.len() * 2) as u32;
    w.write_all(b"RIFF")?;
    w.write_all(&(36 + data).to_le_bytes())?;
    w.write_all(b"WAVEfmt ")?;
    w.write_all(&16u32.to_le_bytes())?;
    w.write_all(&1u16.to_le_bytes())?;
    w.write_all(&2u16.to_le_bytes())?;
    w.write_all(&(SR as u32).to_le_bytes())?;
    w.write_all(&(SR as u32 * 4).to_le_bytes())?;
    w.write_all(&4u16.to_le_bytes())?;
    w.write_all(&16u16.to_le_bytes())?;
    w.write_all(b"data")?;
    w.write_all(&data.to_le_bytes())?;
    for s in pcm {
        w.write_all(&s.to_le_bytes())?;
    }
    Ok(())
}

/// Loudness range: 95th − 10th percentile of short-term loudness (EBU Tech 3342, ungated).
fn lra(short: &[f32]) -> f32 {
    let mut v: Vec<f32> = short.iter().copied().filter(|x| *x > -60.0).collect();
    if v.len() < 10 {
        return 0.0;
    }
    v.sort_by(|a, b| a.total_cmp(b));
    let q = |p: f32| v[((v.len() - 1) as f32 * p) as usize];
    q(0.95) - q(0.10)
}

fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let secs: f32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(60.0);
    let dir = args.get(2).cloned().unwrap_or_else(|| "renders/mix_ab".into());
    let scenes: Vec<String> = args.get(3).map_or(vec!["dawn".into(), "storm".into(), "flight".into()], |s| s.split(',').map(String::from).collect());
    std::fs::create_dir_all(&dir)?;
    let mut md = String::new();
    let mut all = format!("# Sound director A/B ({secs:.0} s per take, 48 kHz)\n\n");
    for scene in &scenes {
        let before = render(scene, false, secs, &format!("{dir}/{scene}_before.wav"));
        let after = render(scene, true, secs, &format!("{dir}/{scene}_after.wav"));
        let _ = writeln!(md, "## {scene}\n");
        let _ = writeln!(md, "| | before | after |\n|---|---|---|");
        let row = |md: &mut String, name: &str, a: String, b: String| {
            let _ = writeln!(md, "| {name} | {a} | {b} |");
        };
        row(&mut md, "integrated loudness, LUFS", format!("{:.1}", before.integrated), format!("{:.1}", after.integrated));
        row(&mut md, "loudness range (short-term p95−p10), LU", format!("{:.1}", lra(&before.short)), format!("{:.1}", lra(&after.short)));
        row(&mut md, "max momentary, LUFS", format!("{:.1}", before.max_momentary), format!("{:.1}", after.max_momentary));
        row(&mut md, "sample peak, dBFS", format!("{:.1}", 20.0 * before.peak.max(1e-6).log10()), format!("{:.1}", 20.0 * after.peak.max(1e-6).log10()));
        for g in 0..group::COUNT {
            let lv = |s: &Stats| {
                if s.group_pow[g] > 0.0 {
                    format!("{:.1}", 10.0 * (s.group_pow[g] / s.samples).log10())
                } else {
                    "—".into()
                }
            };
            let mk = |s: &Stats| if s.group_w[g] > 0.0 { format!("{:.0} %", 100.0 * s.group_mask[g] / s.group_w[g]) } else { "—".into() };
            if before.group_pow[g] > 0.0 || after.group_pow[g] > 0.0 {
                row(&mut md, &format!("{} level LUFS / masked", group::NAMES[g]), format!("{} / {}", lv(&before), mk(&before)), format!("{} / {}", lv(&after), mk(&after)));
            }
        }
        let duck = |s: &Stats| {
            (0..group::COUNT).filter(|g| s.group_duck[*g] < -0.01 * s.samples).map(|g| format!("{} {:.1}", group::NAMES[g], s.group_duck[g] / s.samples)).collect::<Vec<_>>().join(", ")
        };
        row(&mut md, "mean duck on the bed, dB", "—".into(), duck(&after));
        let fm = |s: &Stats, v: f64| if s.solo_n > 0.0 { format!("{:.0} %", 100.0 * v / s.solo_n) } else { "—".into() };
        row(&mut md, "most salient sources (soloists) masked", fm(&before, before.solo_after), fm(&after, after.solo_after));
        row(&mut md, "  … same sources with no ducking", fm(&before, before.solo_before), fm(&after, after.solo_before));
        let fs = |s: &Stats, v: f64| if s.solo_n > 0.0 { format!("{:+.1}", v / s.solo_n) } else { "—".into() };
        row(&mut md, "foreground signal-to-mask ratio, dB", fs(&before, before.smr_after), fs(&after, after.smr_after));
        row(&mut md, "  … same sources with no ducking", fs(&before, before.smr_before), fs(&after, after.smr_before));
        row(&mut md, "soloists on stage (mean)", format!("{:.2}", before.soloists / before.samples), format!("{:.2}", after.soloists / after.samples));
        row(&mut md, "bird phrases at once: mean / max / time with ≥3", format!("{:.2} / {} / {:.0} %", before.phrases / before.samples, before.max_phrases, 100.0 * before.overlap3 / before.samples), format!("{:.2} / {} / {:.0} %", after.phrases / after.samples, after.max_phrases, 100.0 * after.overlap3 / after.samples));
        row(&mut md, "mix layer CPU, % of one core", format!("{:.2}", 100.0 * before.mix_cpu / secs as f64), format!("{:.2}", 100.0 * after.mix_cpu / secs as f64));
        row(&mut md, "whole engine, % of one core (offline)", format!("{:.1}", 100.0 * before.wall / (secs as f64 + 5.0)), format!("{:.1}", 100.0 * after.wall / (secs as f64 + 5.0)));
        let _ = writeln!(md);
        print!("{md}");
        std::io::stdout().flush()?;
        all.push_str(&md);
        md.clear();
    }
    std::fs::write(format!("{dir}/summary.md"), all)?;
    Ok(())
}
