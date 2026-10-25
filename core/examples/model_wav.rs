//! Renders one physical sound model, mono and unspatialised, to a 48 kHz 32-bit float WAV: input
//! for `ml/asmr_ml/refcompare.py` (comparison with field recordings).
//!
//!   cargo run --release --manifest-path core/Cargo.toml --example model_wav -- \
//!       <model> <preset|-> <seconds> <out.wav> [key=value ...]
//!
//! model: wind foliage water rain thunder fire insects whale dolphins reef sea bowl, or `wings:<style|species>`
//! (bowl: struck once at the start).
//! preset `-` = the model's defaults.
//!
//! key=value:
//!   environment  wind_ms rain_mm_h temperature hour humidity leaf_wetness ground_wetness
//!                dryness leaf_cover storm
//!   controls     gust (wind-slider units; default = the slider of wind_ms) water activity flap
//!                gust_amp (0..1, slow random gusts around the mean, default 0) gust_period (s, default 6)
//!   run          seed gain (output gain, default 1)
//!   thunder      distance (m) and interval (s) -> distance_m / interval_s of the config
//!   wings        species=<name> flight=<seconds> pause=<seconds> (repeated take-off + flight,
//!                with the director's flight_motion wingbeat pattern of the species)
//!   anything else is a model parameter (SoundDesc params): `flow_ms=1.2 gain=2` ...
//!
//! `wings:wood_pigeon` (or `wings:pigeon species=wood_pigeon`) renders take-offs of that
//! species. Species: chaffinch robin song_thrush nightingale tawny_owl cuckoo wood_pigeon
//! swallow blackbird skylark gull raven; wing styles: passerine pigeon owl swallow large lark.

use std::fs::File;
use std::io::{BufWriter, Write};

use asmr_core::director::{flight_motion, FlightStyle, Species};
use asmr_core::sources::{Ambient, Controls, Generator, ModelKind, SoundDesc, WingStyle, Wings};

const SR: f32 = 48_000.0;
const BLOCK: usize = 64;

fn species(name: &str) -> Option<Species> {
    use Species::*;
    Some(match name {
        "chaffinch" => Chaffinch,
        "robin" => Robin,
        "song_thrush" => SongThrush,
        "nightingale" => Nightingale,
        "tawny_owl" => TawnyOwl,
        "cuckoo" => Cuckoo,
        "wood_pigeon" => WoodPigeon,
        "swallow" => Swallow,
        "blackbird" => Blackbird,
        "skylark" => Skylark,
        "gull" => Gull,
        "raven" => Raven,
        _ => return None,
    })
}

fn style_default(name: &str) -> Option<(WingStyle, FlightStyle)> {
    Some(match name {
        "passerine" => (WingStyle::Passerine, FlightStyle::Bounding),
        "pigeon" => (WingStyle::Pigeon, FlightStyle::Clatter),
        "owl" => (WingStyle::Owl, FlightStyle::Silent),
        "swallow" => (WingStyle::Swallow, FlightStyle::Swooping),
        "large" => (WingStyle::Large, FlightStyle::Soaring),
        "lark" => (WingStyle::Lark, FlightStyle::Hovering),
        _ => return None,
    })
}

fn value(v: &str) -> serde_json::Value {
    if let Ok(x) = v.parse::<f64>() {
        serde_json::json!(x)
    } else if let Ok(b) = v.parse::<bool>() {
        serde_json::json!(b)
    } else {
        serde_json::json!(v)
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() < 5 {
        eprintln!("usage: model_wav <model|wings:style> <preset|-> <seconds> <out.wav> [key=value ...]\nmodels: wind foliage water rain thunder fire insects");
        std::process::exit(2);
    }
    let (model, preset) = (a[1].as_str(), a[2].as_str());
    let secs: f32 = a[3].parse().expect("seconds");
    let out = &a[4];
    let mut env = Ambient::default();
    let mut c = Controls::default();
    let (mut seed, mut gain) = (1u64, 1.0f32);
    let mut params = serde_json::Map::new();
    let (mut gust_amp, mut gust_period, mut explicit_gust) = (0.0f32, 6.0f32, false);
    let mut sp_name = String::new();
    let (mut flight_s, mut pause_s) = (7.0f32, 3.0f32);
    for kv in &a[5..] {
        let (k, v) = kv.split_once('=').unwrap_or_else(|| panic!("expected key=value, got `{kv}`"));
        let f = || v.parse::<f32>().unwrap_or_else(|_| panic!("`{k}` needs a number"));
        match k {
            "wind_ms" => env.wind_ms = f(),
            "rain_mm_h" => env.rain_mm_h = f(),
            "temperature" | "temperature_c" => env.temperature_c = f(),
            "hour" => env.hour = f(),
            "humidity" => env.humidity = f(),
            "leaf_wetness" => env.leaf_wetness = f(),
            "ground_wetness" => env.ground_wetness = f(),
            "dryness" => env.dryness = f(),
            "leaf_cover" => env.leaf_cover = f(),
            "storm" => env.storm = f(),
            "gust" => {
                c.gust = f();
                explicit_gust = true;
            }
            "gust_amp" => gust_amp = f(),
            "gust_period" => gust_period = f(),
            "water" => c.water = f(),
            "activity" => c.activity = f(),
            "flap" => c.flap = f(),
            "seed" => seed = v.parse().expect("seed"),
            "gain" if model.starts_with("wings") => gain = f(),
            "species" => sp_name = v.to_string(),
            "flight" => flight_s = f(),
            "pause" => pause_s = f(),
            "distance" => {
                params.insert("distance_m".into(), value(v));
            }
            "interval" => {
                params.insert("interval_s".into(), value(v));
            }
            _ => {
                params.insert(k.to_string(), value(v));
            }
        }
    }
    c.env = env;
    if !explicit_gust {
        c.gust = asmr_core::sources::common::wind_slider(env.wind_ms);
    }
    let gust_mean = c.gust;
    let n = (secs * SR) as usize;
    let mut pcm = vec![0.0f32; n];

    if let Some(style) = model.strip_prefix("wings:") {
        let sp = species(&sp_name).or_else(|| species(style));
        let (ws, fs) = match sp {
            Some(s) => (s.def().wings, s.def().flight),
            None => style_default(style).unwrap_or_else(|| panic!("unknown wing style/species `{style}`")),
        };
        let mut wings = Wings::new(SR, seed, ws);
        let side = [1.0, 0.0, 0.0];
        let cycle = flight_s + pause_s;
        for (b, chunk) in pcm.chunks_mut(BLOCK).enumerate() {
            let t = (b * BLOCK) as f32 / SR;
            let tf = t % cycle;
            let flap = if tf < flight_s { flight_motion(fs, tf, flight_s, side, 0.3).1 } else { 0.0 };
            wings.render_driven(chunk, flap, true);
        }
        for s in pcm.iter_mut() {
            *s *= gain;
        }
    } else {
        let kind = ModelKind::ALL.iter().copied().find(|k| k.name() == model).unwrap_or_else(|| panic!("unknown model `{model}`"));
        let mut d = SoundDesc { model: kind, preset: if preset == "-" { None } else { Some(preset.to_string()) }, params };
        if kind == ModelKind::Thunder && !d.params.contains_key("distance_m") {
            d.params.insert("distance_m".into(), serde_json::json!(1500.0));
        }
        let mut g: Box<dyn Generator> = d.build(SR, seed).unwrap_or_else(|e| panic!("{e}"));
        if kind == ModelKind::Insects {
            c.activity = c.activity.max(1.0);
        }
        if kind == ModelKind::Bowl {
            // One strike at the start (a no-op for the rubbed presets).
            c.activity = c.activity.max(1.0);
            c.cue = 1;
        }
        let (mut rng, mut lp) = (0x9E3779B97F4A7C15u64 ^ seed, 0.0f32);
        let a = (asmr_core::BLOCK as f32 / SR / (gust_period / std::f32::consts::TAU)).min(1.0);
        for chunk in pcm.chunks_mut(asmr_core::BLOCK) {
            if gust_amp > 0.0 {
                // Slow gaussian-ish noise (sum of uniforms), one-pole low-passed, unit variance.
                let mut u = 0.0f32;
                for _ in 0..4 {
                    rng ^= rng << 13;
                    rng ^= rng >> 7;
                    rng ^= rng << 17;
                    u += (rng >> 40) as f32 / (1u64 << 24) as f32 - 0.5;
                }
                lp += a * (u * 1.7 - lp);
                c.gust = (gust_mean * (1.0 + gust_amp * lp * (2.0 / a).sqrt() * 0.7)).max(0.0);
            }
            g.render(chunk, &c);
        }
    }
    let peak = pcm.iter().fold(0.0f32, |m, x| m.max(x.abs()));
    let rms = (pcm.iter().map(|x| (*x as f64).powi(2)).sum::<f64>() / n as f64).sqrt();
    eprintln!("{model} {preset}: {secs} s, rms {:.1} dBFS, peak {:.3}", 20.0 * rms.max(1e-9).log10(), peak);
    let mut f = BufWriter::new(File::create(out).expect("create wav"));
    let bytes = (n * 4) as u32;
    f.write_all(b"RIFF").unwrap();
    f.write_all(&(36 + bytes).to_le_bytes()).unwrap();
    f.write_all(b"WAVEfmt ").unwrap();
    for x in [16u32, 0x0001_0003, SR as u32, SR as u32 * 4, 0x0020_0004] {
        f.write_all(&x.to_le_bytes()).unwrap();
    }
    f.write_all(b"data").unwrap();
    f.write_all(&bytes.to_le_bytes()).unwrap();
    for s in &pcm {
        f.write_all(&s.to_le_bytes()).unwrap();
    }
}
