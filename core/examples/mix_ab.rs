//! A/B renders of the sound director's mix layer (docs/sound-director.md).
//!
//!   cargo run --release --example mix_ab -- [seconds] [out_dir] [scenes]
//!
//! Scenes (comma-separated, default all): `dawn` (dense default scene: dawn chorus with every
//! bird, breeze, brook, light rain), `default` (forest_river.json as it starts), `storm` (downpour, thunder, strong wind, a few birds),
//! The default set renders them all. Each is rendered twice from
//! the same seed: **before** (legacy per-kind gains, no attention, ducking or dynamics) and
//! **after** (calibrated levels + the sound director). The same analysis runs in both, so the
//! numbers compare: BS.1770 loudness of the output, per-group level (LUFS share) and masking
//! (fraction of the group's loudness hidden by everything else, Zwicker partial loudness),
//! the masking of the most salient sources (soloists), overlapping bird phrases, and the CPU
//! time of the mix layer. "Is it white noise?" metrics of the output (docs/sound-director.md,
//! section 9.6): spectral flatness (Wiener entropy), loudness-pattern flatness, critical-band
//! envelope fluctuation, frames with audible gaps, the texture loudness (sone) and how many
//! groups are heard as themselves. Writes `<out_dir>/<scene>_{before,after}.wav` and
//! `<out_dir>/summary.md`.
//!
//! Experiments: `MIX_AB_PARAMS="6=3.14,16=40"` sets engine params (id=value) and
//! `MIX_AB_PATCH='{"glide_s":0,"changes":[{"key":"mix.texture_budget","value":0}]}'` applies a
//! control patch, both takes; `MIX_TRACE=1` prints a timeline.

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
    group_sone: [f64; group::COUNT],
    /// Sum over samples of the number of groups heard as themselves (masked < 80 %).
    streams: f64,
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
    /// Output spectrum metrics (see `spectral`).
    out: Spectral,
    /// Mono output for the spectral metrics.
    mono: Vec<f32>,
}

/// "Is it white noise?" metrics of the rendered output (mono sum, after the dynamics).
#[derive(Default, Clone, Copy)]
struct Spectral {
    /// Wiener entropy (geometric / arithmetic mean of the power spectrum, 100 Hz–12 kHz),
    /// mean over 85 ms frames: 1 = white noise, → 0 = tonal / sparse.
    sfm: f64,
    /// Flatness of the Zwicker specific-loudness pattern across the critical bands
    /// (bands 1..22): 1 = every band equally loud (perceptual "grey wall").
    loud_flat: f64,
    /// Envelope fluctuation: std dev over 43 ms frames of each critical band's level, dB,
    /// within 1 s windows, mean over bands. Stationary summed noise → small; a legible
    /// scene with events and gaps → large.
    fluct_db: f64,
    /// Temporal sparsity of the output: share of 43 ms frames 6 dB or more below the
    /// 1 s mean level in at least one third of the bands (gaps the ear can hear into).
    gaps: f64,
}

fn fft(re: &mut [f64], im: &mut [f64]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = -2.0 * std::f64::consts::PI / len as f64;
        for i in (0..n).step_by(len) {
            for k in 0..len / 2 {
                let (c, s) = ((ang * k as f64).cos(), (ang * k as f64).sin());
                let (a, b) = (i + k, i + k + len / 2);
                let (xr, xi) = (re[b] * c - im[b] * s, re[b] * s + im[b] * c);
                re[b] = re[a] - xr;
                im[b] = im[a] - xi;
                re[a] += xr;
                im[a] += xi;
            }
        }
        len <<= 1;
    }
}

fn spectral(x: &[f32]) -> Spectral {
    use asmr_core::mix::loudness::{BandModel, BANDS, EDGES};
    const N: usize = 4096;
    let model = BandModel::new(SR);
    let bin = SR as f64 / N as f64;
    let win: Vec<f64> = (0..N).map(|i| 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / N as f64).cos()).collect();
    let w2: f64 = win.iter().map(|w| w * w).sum();
    // Output digital -> Pa at the ears at the nominal playback mapping (0 dBFS = 78 dB SPL).
    let pa = 10f64.powf((78.0 - 93.979) / 20.0);
    let (mut sfm, mut lf, mut nf) = (0.0, 0.0, 0.0);
    let mut levels: Vec<[f64; BANDS]> = Vec::new();
    let (lo, hi) = ((100.0 / bin) as usize, (12_000.0 / bin) as usize);
    let mut pos = 0;
    while pos + N <= x.len() {
        let mut re: Vec<f64> = (0..N).map(|i| x[pos + i] as f64 * win[i] * pa).collect();
        let mut im = vec![0.0; N];
        fft(&mut re, &mut im);
        let pw: Vec<f64> = (0..N / 2).map(|k| (re[k] * re[k] + im[k] * im[k]) * 2.0 / w2).collect();
        let (mut lg, mut ar) = (0.0, 0.0);
        for p in &pw[lo..hi] {
            lg += (p + 1e-30).ln();
            ar += p;
        }
        let m = (hi - lo) as f64;
        if ar > 0.0 {
            sfm += (lg / m).exp() / (ar / m);
        }
        // Critical-band powers of the two halves (43 ms each) for fluctuation, of the whole
        // frame for the loudness pattern.
        let mut bands = [0.0f32; BANDS];
        let mut lv = [0.0f64; BANDS];
        for z in 0..BANDS {
            let (k0, k1) = (((EDGES[z] as f64 / bin).ceil() as usize).max(1), ((EDGES[z + 1] as f64 / bin).ceil() as usize).min(N / 2));
            let e: f64 = pw[k0..k1.max(k0)].iter().sum();
            bands[z] = e as f32;
            lv[z] = 10.0 * (e + 1e-20).log10();
        }
        levels.push(lv);
        let mut exc = [0.0f32; BANDS];
        model.excitation(&bands, &mut exc);
        let nz: Vec<f64> = (1..23).map(|z| model.specific(z, exc[z], 0.0) as f64 + 1e-4).collect();
        let g = (nz.iter().map(|v| v.ln()).sum::<f64>() / nz.len() as f64).exp();
        let a = nz.iter().sum::<f64>() / nz.len() as f64;
        lf += g / a;
        nf += 1.0;
        pos += N / 2;
    }
    // Fluctuation and gaps in 1 s windows (~23 frames of 43 ms hop).
    let w = 23;
    let (mut fl, mut fln, mut gaps, mut gn) = (0.0, 0.0, 0.0, 0.0);
    for chunk in levels.chunks(w).filter(|c| c.len() == w) {
        let mut deep = vec![0usize; w];
        for z in 1..23 {
            let mean = chunk.iter().map(|l| l[z]).sum::<f64>() / w as f64;
            let var = chunk.iter().map(|l| (l[z] - mean).powi(2)).sum::<f64>() / w as f64;
            fl += var.sqrt();
            fln += 1.0;
            // Gap relative to the power mean of the window.
            let pm = 10.0 * (chunk.iter().map(|l| 10f64.powf(l[z] / 10.0)).sum::<f64>() / w as f64).log10();
            for (i, l) in chunk.iter().enumerate() {
                if l[z] < pm - 6.0 {
                    deep[i] += 1;
                }
            }
        }
        gaps += deep.iter().filter(|d| **d >= 7).count() as f64;
        gn += w as f64;
    }
    let d = |a: f64, b: f64| if b > 0.0 { a / b } else { 0.0 };
    Spectral { sfm: d(sfm, nf), loud_flat: d(lf, nf), fluct_db: d(fl, fln), gaps: d(gaps, gn) }
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
        // The default scene (forest_river.json) as it starts.
        "default" => {}
        _ => panic!("unknown scene {scene}"),
    }
}

fn render(scene: &str, after: bool, secs: f32, path: &str) -> Stats {
    let e = Engine::new(SR);
    e.set_mix_bypass(!after);
    setup(&e, scene);
    // Experiments: MIX_AB_PARAMS="38=0,1=0.7" (asmr param id = value), MIX_AB_PATCH = a
    // control patch (docs/prompt-control.md), applied to both takes.
    if let Ok(list) = std::env::var("MIX_AB_PARAMS") {
        for kv in list.split(',').filter(|s| !s.is_empty()) {
            let (k, v) = kv.split_once('=').expect("id=value");
            e.set_param(k.trim().parse().expect("param id"), v.trim().parse().expect("value"));
        }
    }
    if let Ok(patch) = std::env::var("MIX_AB_PATCH") {
        let r = e.apply_patch_json(&patch);
        assert!(r.contains("\"ok\":true"), "{r}");
    }
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
            st.mono.push(0.5 * (a + c));
        }
        if b % 10 == 0 {
            let m = e.mix_info();
            st.short.push(m.loudness_short);
            st.max_momentary = st.max_momentary.max(m.loudness_momentary);
            st.streams += (0..group::COUNT).filter(|&g| m.group_sone[g] > 0.1 && m.group_masking[g] < 0.8).count() as f64;
            for g in 0..group::COUNT {
                if m.group_lufs[g] > -69.0 {
                    st.group_pow[g] += 10f64.powf(m.group_lufs[g] as f64 / 10.0);
                }
                let w = m.group_sone[g] as f64;
                st.group_mask[g] += m.group_masking[g] as f64 * w;
                st.group_duck[g] += m.group_duck_db[g] as f64;
                st.group_w[g] += w;
                st.group_sone[g] += w;
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
            if trace && (b % 100 == 0 || m.loudness_momentary > st.max_momentary - 0.5) {
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
                println!(
                    "{:5.1}s mom {:5.1} st {:5.1} gain {:5.1} phr {}/{} tex {:.1}/{:.1} sone cut {:.1} {}/{} still {:.2} anchors {:?} | {} | {}",
                    (b - warm) as f32 / 100.0,
                    m.loudness_momentary,
                    m.loudness_short,
                    m.gain_db,
                    m.phrases,
                    m.max_phrases,
                    m.texture_sone,
                    m.texture_protected_sone,
                    m.texture_cut_db,
                    m.textures_protected,
                    m.textures,
                    m.stillness,
                    m.anchor_source.map(|a| a as i64 - if a == u32::MAX { a as i64 + 1 } else { 0 }),
                    g.join(" "),
                    solo.join("; ")
                );
            }
        }
    }
    st.wall = t0.elapsed().as_secs_f64();
    st.seconds = secs;
    st.integrated = e.mix_info().loudness_integrated;
    st.mix_cpu = e.mix_cpu_s();
    st.out = spectral(&st.mono);
    st.mono = Vec::new();
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
    let scenes: Vec<String> = args.get(3).map_or(vec!["default".into(), "dawn".into(), "storm".into()], |s| s.split(',').map(String::from).collect());
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
        row(&mut md, "**spectral flatness (Wiener entropy, 1 = white)**", format!("{:.3}", before.out.sfm), format!("{:.3}", after.out.sfm));
        row(&mut md, "loudness-pattern flatness (1 = every band equally loud)", format!("{:.3}", before.out.loud_flat), format!("{:.3}", after.out.loud_flat));
        row(&mut md, "band envelope fluctuation, dB", format!("{:.2}", before.out.fluct_db), format!("{:.2}", after.out.fluct_db));
        row(&mut md, "frames with audible gaps (≥7 bands −6 dB)", format!("{:.1} %", 100.0 * before.out.gaps), format!("{:.1} %", 100.0 * after.out.gaps));
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
        let tex = |s: &Stats| {
            let v: f64 = [group::WATER, group::WIND, group::RAIN, group::INSECTS].iter().map(|g| s.group_sone[*g]).sum();
            format!("{:.1}", v / s.samples)
        };
        row(&mut md, "texture loudness (water+wind+rain+insects), sone", tex(&before), tex(&after));
        row(&mut md, "groups heard as themselves (masked < 80 %), mean", format!("{:.2}", before.streams / before.samples), format!("{:.2}", after.streams / after.samples));
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
