//! Comfort check: renders a scene offline and measures what reaches the ears for anything
//! unpleasant (clicks, loudness surprises, harshness, roughness, sustained tones, floating
//! tones, rumble, infrasound, DC, left/right imbalance). Metrics and thresholds:
//! docs/comfort.md, analysis: src/comfort.rs.
//!
//!   cargo run --release --manifest-path core/Cargo.toml --example comfort -- MINUTES [scene.json] [options]
//!   cargo run --release --manifest-path core/Cargo.toml --example comfort -- wav FILE.wav [...]
//!   cargo run --release --manifest-path core/Cargo.toml --example comfort -- selftest [MINUTES]
//!
//! Options (scene renders):
//!   turn            slow head turns (yaw ±60° over 47 s, a little pitch)
//!   hour=6.5        start time of day          seed=7       scene seed
//!   weather=0.5 energy=-0.3 change=0.8 clock=4   direction steer (clock = scene h per real h)
//!   cue=10:thunderstorm   at minute 10 ask the direction for a spell (or `next`)
//!   rain=8 storm=0.6 wind=6   fixed weather (controls; the direction leaves them after that)
//!   mute=birds,water   group trims at −40 dB (locating a problem)
//!   clarity=0       the mix's clarity (0 = no masking-aware ducking; locating a problem)
//!   wav=out.wav     keep the render (wav_from=MIN: from that minute on)
//!   csv=out.csv     per-second timeline (default build/comfort/<scene>-<minutes>m.csv)
//!   worst=8         offenders listed per kind
//!   jobs=6          render the minutes as 6 consecutive stretches of the day in parallel
//!                   (each its own engine, cues go to the first; reports merged)
//!
//! Exit code 1 when the verdict is FAIL.

use std::fs::File;
use std::io::{BufWriter, Read, Write};
use std::time::Instant;

use asmr_core::comfort::{clock, Analyzer, EventKind, Report, Thresholds, Verdict};
use asmr_core::controls::id;
use asmr_core::mix::calibration::group;
use asmr_core::{param, Engine, SceneDesc};

const SR: f32 = 48_000.0;
const CHUNK: usize = 2400;

#[derive(Clone)]
struct Opts {
    minutes: f32,
    scene: Option<String>,
    turn: bool,
    hour: Option<f32>,
    seed: Option<u64>,
    steer: Vec<(u32, f32)>,
    fixed: Vec<(u32, f32)>,
    cues: Vec<(f32, String)>,
    wav: Option<String>,
    /// Keep the WAV from this minute on (a long take, a short listening excerpt).
    wav_from: f32,
    csv: Option<String>,
    worst: usize,
    /// Takes rendered in parallel (consecutive stretches of the day, each its own engine).
    jobs: usize,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        Some("wav") => analyse_files(&args[1..]),
        Some("lowfreq") => lowfreq(&args[1]),
        Some("peek") => peek(&args[1], args[2].parse().expect("seconds")),
        Some("spikes") => {
            let (sr, l, r) = read_wav(&args[1]).expect("wav");
            let thr: f32 = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(0.1);
            for k in 2..l.len() {
                let d = (l[k] - 2.0 * l[k - 1] + l[k - 2]).abs().max((r[k] - 2.0 * r[k - 1] + r[k - 2]).abs());
                if d > thr {
                    println!("{}  {d:.3}", clock(k as f64 / sr as f64));
                }
            }
            0
        }
        Some("selftest") => selftest(args.get(1).and_then(|m| m.parse().ok()).unwrap_or(3.0)),
        _ => {
            let o = parse(&args);
            let rep = render(&o, true);
            finish(&o, &rep)
        }
    };
    std::process::exit(code);
}

fn parse(args: &[String]) -> Opts {
    let mut o = Opts { minutes: 10.0, scene: None, turn: false, hour: None, seed: None, steer: Vec::new(), fixed: Vec::new(), cues: Vec::new(), wav: None, wav_from: 0.0, csv: None, worst: 6, jobs: 1 };
    for a in args {
        if let Ok(m) = a.parse::<f32>() {
            o.minutes = m;
        } else if a == "turn" {
            o.turn = true;
        } else if let Some((k, v)) = a.split_once('=') {
            let num = || v.parse::<f32>().unwrap_or_else(|_| panic!("{k}= needs a number"));
            match k {
                "hour" => o.hour = Some(num()),
                "seed" => o.seed = Some(v.parse().expect("seed=N")),
                "weather" => o.steer.push((id::DIRECTION_WEATHER, num())),
                "energy" => o.steer.push((id::DIRECTION_ENERGY, num())),
                "change" => o.steer.push((id::DIRECTION_CHANGE, num())),
                "clock" => o.steer.push((id::DIRECTION_CLOCK, num())),
                "amount" => o.steer.push((id::DIRECTION_AMOUNT, num())),
                "rain" => o.fixed.push((id::RAIN_RATE, num())),
                "storm" => o.fixed.push((id::RAIN_STORM, num())),
                "wind" => o.fixed.push((id::WIND_SPEED, num())),
                "birds" => o.fixed.push((id::BIRDS_ACTIVITY, num())),
                "clarity" => o.fixed.push((id::MIX_CLARITY, num())),
                "mute" => {
                    for g in v.split(',') {
                        let lv = match g {
                            "water" => id::WATER_LEVEL,
                            "wind" => id::WIND_LEVEL,
                            "foliage" => id::FOLIAGE_LEVEL,
                            "rain" => id::RAIN_LEVEL,
                            "birds" => id::BIRDS_LEVEL,
                            "insects" => id::INSECTS_LEVEL,
                            "fire" => id::FIRE_LEVEL,
                            _ => panic!("mute=: water wind foliage rain birds insects fire"),
                        };
                        o.fixed.push((lv, -40.0));
                    }
                }
                "cue" => {
                    let (m, name) = v.split_once(':').expect("cue=MIN:episode");
                    o.cues.push((m.parse().expect("cue minute"), name.to_string()));
                }
                "wav" => o.wav = Some(v.to_string()),
                "wav_from" => o.wav_from = num(),
                "csv" => o.csv = Some(v.to_string()),
                "worst" => o.worst = v.parse().expect("worst=N"),
                "jobs" => o.jobs = v.parse::<usize>().expect("jobs=N").max(1),
                _ => panic!("unknown option {k}="),
            }
        } else {
            o.scene = Some(a.clone());
        }
    }
    o
}

/// Renders the scene, analysing it on the fly; writes the WAV if asked.
fn render(o: &Opts, verbose: bool) -> Report {
    if o.jobs > 1 {
        return render_parallel(o);
    }
    let (l, r, cond) = render_pcm(o, verbose, o.wav.is_some());
    if let Some(path) = &o.wav {
        write_wav(path, &l, &r).expect("write wav");
    }
    print_conditions(&cond.log, verbose);
    cond.report.expect("report")
}

/// `jobs` engines in parallel, each a consecutive stretch of the scene's day (the first one
/// takes the cues); one merged report.
fn render_parallel(o: &Opts) -> Report {
    let n = o.jobs;
    let seg = o.minutes / n as f32;
    let desc_hour = |o: &Opts| -> f32 {
        o.hour.unwrap_or_else(|| {
            let json = o.scene.as_ref().map(|f| std::fs::read_to_string(f).expect("scene")).unwrap_or_else(|| asmr_core::scene::DEFAULT_SCENE_JSON.to_string());
            SceneDesc::from_json(&json).map(|d| d.environment.time_of_day).unwrap_or(9.0)
        })
    };
    let h0 = desc_hour(o);
    println!("{} takes of {seg:.1} min in parallel from {}", n, clock_h(h0));
    let started = Instant::now();
    let parts: Vec<(Report, Vec<String>)> = std::thread::scope(|sc| {
        let handles: Vec<_> = (0..n)
            .map(|k| {
                let mut ok = o.clone();
                ok.minutes = seg;
                ok.hour = Some(h0 + k as f32 * seg / 60.0);
                ok.wav = None;
                if k > 0 {
                    ok.cues.clear();
                }
                sc.spawn(move || {
                    let (_, _, c) = render_pcm(&ok, false, false);
                    (c.report.expect("report"), c.log)
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().expect("take")).collect()
    });
    for (k, (_, log)) in parts.iter().enumerate() {
        println!("\ntake {} ({}):", k + 1, clock((k as f32 * seg * 60.0) as f64));
        print_conditions(log, true);
    }
    println!("rendered and analysed {:.1} min in {:.1} s", o.minutes, started.elapsed().as_secs_f32());
    Report::merge(parts.into_iter().map(|(r, _)| r).collect())
}

struct Conditions {
    log: Vec<String>,
    report: Option<Report>,
}

/// Renders `o.minutes` of the scene; analyses as it goes (attribution needs the engine's
/// per-group levels at each moment). Keeps the PCM when `keep`.
fn render_pcm(o: &Opts, verbose: bool, keep: bool) -> (Vec<f32>, Vec<f32>, Conditions) {
    let json = match &o.scene {
        Some(f) => std::fs::read_to_string(f).unwrap_or_else(|e| panic!("{f}: {e}")),
        None => asmr_core::scene::DEFAULT_SCENE_JSON.to_string(),
    };
    let mut desc = SceneDesc::from_json(&json).unwrap_or_else(|e| panic!("{e}"));
    if let Some(s) = o.seed {
        desc.seed = s;
    }
    if let Some(h) = o.hour {
        desc.environment.time_of_day = h.rem_euclid(24.0);
    }
    let engine = Engine::with_scene(SR, &desc).unwrap_or_else(|e| panic!("{e}"));
    engine.set_param(param::PLAYING, 1.0);
    engine.set_param(param::VOLUME, 0.8);
    for (k, v) in &o.steer {
        engine.set_control(*k, *v);
    }
    for (k, v) in &o.fixed {
        engine.set_control(*k, *v);
    }
    let names: Vec<String> = (0..engine.direction_episode_count()).filter_map(|i| engine.direction_episode_name(i).map(str::to_string)).collect();
    let mut cues: Vec<(f32, u32)> = o
        .cues
        .iter()
        .map(|(m, n)| {
            let v = if n == "next" { 1 } else { 2 + names.iter().position(|x| x == n).unwrap_or_else(|| panic!("unknown episode {n}; known: {names:?}")) as u32 };
            (m * 60.0, v)
        })
        .collect();
    if verbose {
        println!("scene: {} · {} min from {} · {}", desc.name, o.minutes, clock_h(engine.param(param::TIME_OF_DAY)), if o.turn { "slow head turns" } else { "head still" });
    }
    let mut a = Analyzer::new(SR);
    a.set_groups(&group::NAMES);
    let total = (o.minutes * 60.0 * SR) as usize;
    let (mut l, mut r) = (vec![0.0f32; CHUNK], vec![0.0f32; CHUNK]);
    let (mut kl, mut kr) = (Vec::new(), Vec::new());
    let mut log = Vec::new();
    let mut last_seg = (u32::MAX, u32::MAX);
    let started = Instant::now();
    let mut done = 0usize;
    let mut next_note = 0.0f32;
    while done < total {
        let t = done as f32 / SR;
        if o.turn {
            let yaw = (std::f32::consts::TAU * t / 47.0).sin() * 60f32.to_radians();
            let pitch = (std::f32::consts::TAU * t / 71.0).sin() * 10f32.to_radians();
            engine.set_param(param::HEAD_YAW, yaw);
            engine.set_param(param::HEAD_PITCH, pitch);
        }
        while let Some(i) = cues.iter().position(|(at, _)| *at <= t) {
            let (_, v) = cues.remove(i);
            engine.set_control(id::DIRECTION_CUE, v as f32);
        }
        let n = CHUNK.min(total - done);
        engine.render(&mut l[..n], &mut r[..n]);
        a.push(&l[..n], &r[..n]);
        let m = engine.mix_info();
        a.context(&m.group_lufs[..group::NAMES.len()]);
        if keep && t >= o.wav_from * 60.0 {
            kl.extend_from_slice(&l[..n]);
            kr.extend_from_slice(&r[..n]);
        }
        done += n;
        // Conditions: every direction stretch, and the weather every 5 min.
        let d = engine.direction_info();
        let seg = (d.episode, d.segment);
        if d.active != 0 && seg != last_seg || t >= next_note {
            let ep = engine.direction_episode_name(d.episode as usize).unwrap_or("-");
            let label = engine.direction_segment_label(d.episode as usize, d.segment as usize).unwrap_or("-");
            log.push(format!(
                "{}  {}  {:<13} {:<22} wind {:4.1} m/s  rain {:5.1} mm/h  storm {:.2}  birds {:.2}  {:5.1} LUFS",
                clock(t as f64),
                clock_h(engine.param(param::TIME_OF_DAY)),
                ep,
                label,
                engine.control(id::WIND_SPEED),
                engine.control(id::RAIN_RATE),
                engine.control(id::RAIN_STORM),
                engine.control(id::BIRDS_ACTIVITY),
                m.loudness_short
            ));
            last_seg = seg;
            if t >= next_note {
                next_note += 300.0;
            }
        }
    }
    let el = started.elapsed().as_secs_f32();
    let rep = a.finish(&Thresholds::default());
    if verbose {
        println!("rendered and analysed {:.1} min in {:.1} s ({:.0}× real time)", o.minutes, el, o.minutes * 60.0 / el.max(1e-3));
    }
    (kl, kr, Conditions { log, report: Some(rep) })
}

fn print_conditions(log: &[String], verbose: bool) {
    if verbose && !log.is_empty() {
        println!("\nconditions (direction stretches, weather every 5 min):");
        for line in log {
            println!("  {line}");
        }
    }
}

fn clock_h(h: f32) -> String {
    let m = (h.rem_euclid(24.0) * 60.0).round() as u32 % 1440;
    format!("{:02}:{:02}", m / 60, m % 60)
}

fn finish(o: &Opts, rep: &Report) -> i32 {
    println!("\n{}", rep.text(o.worst));
    let csv = o.csv.clone().unwrap_or_else(|| {
        let stem = o.scene.as_deref().and_then(|s| std::path::Path::new(s).file_stem()).and_then(|s| s.to_str()).unwrap_or("forest_river").to_string();
        let _ = std::fs::create_dir_all("build/comfort");
        format!("build/comfort/{stem}-{}m{}.csv", o.minutes, if o.turn { "-turn" } else { "" })
    });
    match std::fs::write(&csv, rep.csv()) {
        Ok(()) => println!("timeline: {csv}"),
        Err(e) => eprintln!("timeline not written ({csv}): {e}"),
    }
    if let Some(w) = &o.wav {
        println!("render: {w}");
    }
    i32::from(rep.verdict() == Verdict::Fail)
}

// ---------------------------------------------------------------------------------------
// Existing recordings
// ---------------------------------------------------------------------------------------

fn analyse_files(files: &[String]) -> i32 {
    let mut worst = Verdict::Pass;
    for f in files {
        let (sr, l, r) = match read_wav(f) {
            Ok(x) => x,
            Err(e) => {
                eprintln!("{f}: {e}");
                continue;
            }
        };
        let mut a = Analyzer::new(sr);
        a.push(&l, &r);
        let rep = a.finish(&Thresholds::default());
        println!("== {f}");
        println!("{}", rep.text(4));
        let stem = std::path::Path::new(f).file_stem().and_then(|s| s.to_str()).unwrap_or("file");
        let _ = std::fs::create_dir_all("build/comfort");
        let _ = std::fs::write(format!("build/comfort/{stem}.csv"), rep.csv());
        worst = worst.max(rep.verdict());
    }
    i32::from(worst == Verdict::Fail)
}

/// Level per low band (0.1–80 Hz, 100 Hz decimated, 20 s DFT frames) re the 100–1000 Hz
/// band: where infrasound and DC come from.
fn lowfreq(path: &str) -> i32 {
    let (sr, l, r) = read_wav(path).expect("wav");
    let dec = (sr / 200.0) as usize;
    let m: Vec<f64> = l.chunks(dec).zip(r.chunks(dec)).map(|(a, b)| (a.iter().chain(b).map(|v| *v as f64).sum::<f64>()) / (2 * a.len()) as f64).collect();
    let fe = sr as f64 / dec as f64;
    let n = (20.0 * fe) as usize;
    let mean_all = m.iter().sum::<f64>() / m.len() as f64;
    println!("mean of the whole file {:.1} dBFS", 20.0 * mean_all.abs().max(1e-12).log10());
    let bands = [(0.1, 0.5), (0.5, 1.0), (1.0, 2.0), (2.0, 4.0), (4.0, 8.0), (8.0, 16.0), (16.0, 31.5), (31.5, 63.0), (63.0, 90.0)];
    let mut acc = vec![0.0f64; bands.len()];
    let frames = m.len() / n;
    for f in 0..frames {
        let x = &m[f * n..(f + 1) * n];
        let mu = x.iter().sum::<f64>() / n as f64;
        for (b, (lo, hi)) in bands.iter().enumerate() {
            let (k0, k1) = (((lo * n as f64) / fe).ceil() as usize, ((hi * n as f64) / fe) as usize);
            for k in k0.max(1)..=k1 {
                let (mut re, mut im) = (0.0, 0.0);
                for (j, v) in x.iter().enumerate() {
                    let w = 0.5 - 0.5 * (std::f64::consts::TAU * j as f64 / n as f64).cos();
                    let ph = std::f64::consts::TAU * (k * j) as f64 / n as f64;
                    re += (v - mu) * w * ph.cos();
                    im -= (v - mu) * w * ph.sin();
                }
                acc[b] += (re * re + im * im) * 2.0 / (n as f64 * n as f64 * 0.375);
            }
        }
    }
    let full: f64 = l.iter().zip(&r).map(|(a, b)| ((*a as f64 + *b as f64) / 2.0).powi(2)).sum::<f64>() / l.len() as f64;
    for (b, (lo, hi)) in bands.iter().enumerate() {
        println!("{lo:5.1}–{hi:5.1} Hz  {:6.1} dB re full band", 10.0 * (acc[b] / frames.max(1) as f64 / full).max(1e-20).log10());
    }
    0
}

/// Prints the samples around the largest second difference (either channel) within ±30 ms
/// of `t`.
fn peek(path: &str, t: f64) -> i32 {
    let (sr, l, r) = read_wav(path).expect("wav");
    let i = (t * sr as f64) as usize;
    let (a, z) = (i.saturating_sub((0.1 * sr) as usize).max(2), (i + (0.1 * sr) as usize).min(l.len()));
    let d2 = |x: &[f32], k: usize| (x[k] - 2.0 * x[k - 1] + x[k - 2]).abs();
    let k = (a..z).max_by(|&x, &y| d2(&l, x).max(d2(&r, x)).total_cmp(&d2(&l, y).max(d2(&r, y)))).unwrap_or(i);
    println!("largest second difference at {}: L {:.4} R {:.4}", clock(k as f64 / sr as f64), d2(&l, k), d2(&r, k));
    for j in k.saturating_sub(12)..(k + 12).min(l.len()) {
        println!("{:+4}  {:+.5}  {:+.5}", j as i64 - k as i64, l[j], r[j]);
    }
    0
}

fn read_wav(path: &str) -> std::io::Result<(f32, Vec<f32>, Vec<f32>)> {
    let mut b = Vec::new();
    File::open(path)?.read_to_end(&mut b)?;
    let bad = |m: &str| std::io::Error::new(std::io::ErrorKind::InvalidData, m.to_string());
    if b.len() < 12 || &b[0..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        return Err(bad("not a RIFF/WAVE file"));
    }
    let u16_at = |i: usize| u16::from_le_bytes([b[i], b[i + 1]]);
    let u32_at = |i: usize| u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
    let (mut fmt, mut ch, mut sr, mut bits) = (0u16, 0u16, 0u32, 0u16);
    let mut i = 12;
    while i + 8 <= b.len() {
        let id = &b[i..i + 4];
        let len = u32_at(i + 4) as usize;
        let body = i + 8;
        if id == b"fmt " {
            fmt = u16_at(body);
            ch = u16_at(body + 2);
            sr = u32_at(body + 4);
            bits = u16_at(body + 14);
            if fmt == 0xFFFE {
                fmt = u16_at(body + 24);
            }
        } else if id == b"data" {
            let end = (body + len).min(b.len());
            let data = &b[body..end];
            let bps = (bits / 8) as usize;
            let frame = bps * ch as usize;
            let n = data.len() / frame.max(1);
            let sample = |k: usize| -> f32 {
                let s = &data[k..k + bps];
                match (fmt, bits) {
                    (1, 16) => i16::from_le_bytes([s[0], s[1]]) as f32 / 32768.0,
                    (1, 24) => ((i32::from_le_bytes([0, s[0], s[1], s[2]])) >> 8) as f32 / 8_388_608.0,
                    (1, 32) => i32::from_le_bytes([s[0], s[1], s[2], s[3]]) as f32 / 2_147_483_648.0,
                    (3, 32) => f32::from_le_bytes([s[0], s[1], s[2], s[3]]),
                    _ => 0.0,
                }
            };
            if !matches!((fmt, bits), (1, 16) | (1, 24) | (1, 32) | (3, 32)) {
                return Err(bad("unsupported sample format"));
            }
            let mut l = Vec::with_capacity(n);
            let mut r = Vec::with_capacity(n);
            for f in 0..n {
                let a = sample(f * frame);
                let c = if ch > 1 { sample(f * frame + bps) } else { a };
                l.push(a);
                r.push(c);
            }
            return Ok((sr as f32, l, r));
        }
        i = body + len + (len & 1);
    }
    Err(bad("no data chunk"))
}

fn write_wav(path: &str, l: &[f32], r: &[f32]) -> std::io::Result<()> {
    if let Some(dir) = std::path::Path::new(path).parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let mut f = BufWriter::new(File::create(path)?);
    let data_len = (l.len() * 4) as u32;
    f.write_all(b"RIFF")?;
    f.write_all(&(36 + data_len).to_le_bytes())?;
    f.write_all(b"WAVEfmt ")?;
    f.write_all(&16u32.to_le_bytes())?;
    f.write_all(&1u16.to_le_bytes())?;
    f.write_all(&2u16.to_le_bytes())?;
    f.write_all(&(SR as u32).to_le_bytes())?;
    f.write_all(&(SR as u32 * 4).to_le_bytes())?;
    f.write_all(&4u16.to_le_bytes())?;
    f.write_all(&16u16.to_le_bytes())?;
    f.write_all(b"data")?;
    f.write_all(&data_len.to_le_bytes())?;
    for (a, b) in l.iter().zip(r) {
        f.write_all(&((a.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes())?;
        f.write_all(&((b.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes())?;
    }
    f.flush()
}

// ---------------------------------------------------------------------------------------
// Self-test: deliberately bad inputs must be caught, the clean render must not be.
// ---------------------------------------------------------------------------------------

fn selftest(minutes: f32) -> i32 {
    let o = Opts { minutes, scene: None, turn: false, hour: Some(9.0), seed: None, steer: Vec::new(), fixed: Vec::new(), cues: Vec::new(), wav: None, wav_from: 0.0, csv: None, worst: 3, jobs: 1 };
    println!("self-test: {minutes} min of the forest at 09:00, clean, then with each defect injected\n");
    let (l, r, cond) = render_pcm(&o, false, true);
    let clean = cond.report.expect("report");
    let _ = write_wav("build/listen/selftest/1-clean.wav", &l, &r);
    let secs = l.len() as f32 / SR;
    let analyse = |l: &[f32], r: &[f32]| {
        let mut a = Analyzer::new(SR);
        a.push(l, r);
        a.finish(&Thresholds::default())
    };
    let near = |rep: &Report, kind: EventKind, t: f64, tol: f64| rep.events.iter().any(|e| e.kind == kind && e.t <= t + tol && e.t + e.dur as f64 >= t - tol);
    let mut ok = true;
    let mut row = |name: &str, pass: bool, detail: String| {
        println!("  {:<44} {}  {}", name, if pass { "caught " } else { "MISSED " }, detail);
        ok &= pass;
    };
    println!("  clean render: {} clicks, {} floating tones, {} sustained tones, surprise {:.1} LU, verdict {}", clean.count(EventKind::Click), clean.count(EventKind::Wobble), clean.count(EventKind::Tone), clean.check("loudness surprise").map_or(0.0, |c| c.value), clean.verdict().name());
    let rms = (l.iter().map(|v| v * v).sum::<f32>() / l.len() as f32).sqrt();
    let at = |frac: f32| ((secs * frac) * SR) as usize;

    // 1. A click: one sample, 6 dB above the bed's RMS level... in amplitude terms a small tick.
    {
        let (mut a, mut b) = (l.clone(), r.clone());
        let i = at(0.31);
        let amp = (rms * 10.0).min(0.2);
        a[i] += amp;
        b[i] += amp;
        let _ = write_wav("build/listen/selftest/2-click.wav", &a, &b);
        let rep = analyse(&a, &b);
        let t = i as f64 / SR as f64;
        row("click (one sample)", near(&rep, EventKind::Click, t, 0.01) && !near(&clean, EventKind::Click, t, 0.01), format!("{:.1} dBFS at {}", 20.0 * amp.log10(), clock(t)));
    }
    // 2. A step: the signal jumps by a DC offset for 200 ms (a gain or buffer glitch).
    {
        let (mut a, mut b) = (l.clone(), r.clone());
        let i = at(0.37);
        let amp = (rms * 3.0).min(0.1);
        for k in i..i + (0.2 * SR) as usize {
            a[k] += amp;
            b[k] += amp;
        }
        let _ = write_wav("build/listen/selftest/3-step.wav", &a, &b);
        let rep = analyse(&a, &b);
        let t = i as f64 / SR as f64;
        row("discontinuity (step for 200 ms)", near(&rep, EventKind::Click, t, 0.01), format!("{:.1} dBFS at {}", 20.0 * amp.log10(), clock(t)));
    }
    // 3. A floating tone: 2 kHz whistle, ±0.7 % at 2.5 Hz (≈ 12 cents RMS) for 4 s.
    {
        let (mut a, mut b) = (l.clone(), r.clone());
        let i = at(0.5);
        let amp = rms * 0.7;
        let mut ph = 0.0f32;
        for k in 0..(4.0 * SR) as usize {
            let t = k as f32 / SR;
            let f = 2000.0 * (1.0 + 0.007 * (std::f32::consts::TAU * 2.5 * t).sin());
            ph += std::f32::consts::TAU * f / SR;
            let env = (t / 0.2).min(1.0).min((4.0 - t) / 0.2);
            a[i + k] += amp * env * ph.sin();
            b[i + k] += amp * env * ph.sin();
        }
        let _ = write_wav("build/listen/selftest/4-wobble.wav", &a, &b);
        let rep = analyse(&a, &b);
        let t = i as f64 / SR as f64;
        row("wobbling tone (2 kHz ±0.7 % at 2.5 Hz, 4 s)", near(&rep, EventKind::Wobble, t + 2.0, 2.0) && !near(&clean, EventKind::Wobble, t + 2.0, 2.0), format!("{:.1} dBFS at {}", 20.0 * (amp / std::f32::consts::SQRT_2).log10(), clock(t)));
    }
    // 4. A harsh whistle: steady 4 kHz for 30 s, as loud as the bed.
    {
        let (mut a, mut b) = (l.clone(), r.clone());
        let i = at(0.1);
        let amp = rms * std::f32::consts::SQRT_2;
        let n = (30.0 * SR) as usize;
        for k in 0..n.min(a.len() - i) {
            let t = k as f32 / SR;
            let env = (t / 0.5).min(1.0);
            let v = amp * env * (std::f32::consts::TAU * 4000.0 * t).sin();
            a[i + k] += v;
            b[i + k] += v;
        }
        let _ = write_wav("build/listen/selftest/5-whistle.wav", &a, &b);
        let rep = analyse(&a, &b);
        let tone = rep.check("longest prominent tone").map_or(0.0, |c| c.value);
        let margin = rep.events.iter().filter(|e| e.kind == EventKind::Tone && (e.freq - 4000.0).abs() < 100.0).map(|e| e.value).fold(0.0f32, f32::max);
        row("harsh whistle (steady 4 kHz, 30 s)", tone >= 20.0 && rep.check("longest prominent tone").unwrap().verdict >= Verdict::Warn, format!("prominent for {tone:.0} s, TNR {margin:.1} dB over the ECMA-74 criterion"));
    }
    // 5. A burst: the whole mix +15 dB for 1.5 s.
    {
        let (mut a, mut b) = (l.clone(), r.clone());
        let i = at(0.7);
        let g = 10f32.powf(15.0 / 20.0);
        for k in i..i + (1.5 * SR) as usize {
            a[k] *= g;
            b[k] *= g;
        }
        let _ = write_wav("build/listen/selftest/6-burst.wav", &a, &b);
        let rep = analyse(&a, &b);
        let s = rep.check("loudness surprise").unwrap();
        let t = i as f64 / SR as f64;
        row("+15 dB burst for 1.5 s", s.verdict == Verdict::Fail && (s.at.unwrap() - t).abs() < 2.0, format!("surprise {:.1} LU at {} (clean {:.1})", s.value, clock(s.at.unwrap()), clean.check("loudness surprise").unwrap().value));
    }
    // 6. Hiss: white noise 6–20 kHz at the bed's level for 20 s.
    {
        let (mut a, mut b) = (l.clone(), r.clone());
        let i = at(0.2);
        let mut seed = 12345u32;
        let mut hp = [0.0f32; 2];
        for k in 0..(20.0 * SR) as usize {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let w = (seed >> 8) as f32 / 8_388_608.0 - 1.0;
            // Two one-pole high passes at ~6 kHz.
            let c = 0.45;
            let y1 = w - hp[0];
            hp[0] += c * y1;
            let y2 = y1 - hp[1];
            hp[1] += c * y2;
            a[i + k] += rms * 1.5 * y2;
            b[i + k] += rms * 1.5 * y2;
        }
        let _ = write_wav("build/listen/selftest/7-hiss.wav", &a, &b);
        let rep = analyse(&a, &b);
        let h = rep.check("hiss").unwrap();
        row("hiss (6–20 kHz noise, 20 s)", h.verdict >= Verdict::Warn, format!("hiss excess {:.1} dB (clean {:.1})", h.value, clean.check("hiss").unwrap().value));
    }
    println!("\nself-test {}", if ok { "passed" } else { "FAILED" });
    i32::from(!ok)
}
