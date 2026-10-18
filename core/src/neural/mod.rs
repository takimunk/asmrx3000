//! Neural sound sources (see docs/neural-sound.md).
//!
//! Birds: one shared multi-species base model (`birds_base.asnw`: DDSP voice, song syntax,
//! contour decoder) plus a small adapter per species (`<species>.asna`: embeddings, input
//! normalisation, optional LoRA, a few constants). `compose` folds an adapter into the base at
//! load time, so every voice runs a plain small network.
//!
//! Inference is our own: a handful of layers (Linear, GRU) over plain `f32` slices, no
//! dependencies, no allocation after construction, identical on native and WASM.

pub mod bird;
pub mod compose;
pub mod layers;
pub mod song;
pub mod voice;
pub mod weights;

use std::sync::{Arc, Mutex, OnceLock};

pub use bird::{BirdControls, NeuralBird};
use song::SongModel;
use voice::VoiceModel;
use weights::{Error, Weights};

/// One species, ready to sing: base + adapter composed.
pub struct NeuralModel {
    pub voice: VoiceModel,
    pub song: SongModel,
    /// Loudness the voice was trained to treat as silence (between syllables).
    pub silence_db: f32,
    pub silence_effort: f32,
    pub out_gain: f32,
    /// Typical pause between songs in the training data, before `activity` scaling.
    pub song_interval_s: f32,
}

impl NeuralModel {
    /// Composes a species from a parsed base and the bytes of its adapter file.
    pub fn compose(base: &Weights, adapter: &[u8]) -> Result<Self, Error> {
        let w = compose::compose(base, &Weights::parse(adapter)?)?;
        Ok(Self {
            voice: VoiceModel::load(&w)?,
            song: SongModel::load(&w)?,
            silence_db: w.scalar("cfg.silence_db")?,
            silence_effort: w.scalar("cfg.silence_effort")?,
            out_gain: w.scalar("cfg.out_gain")?,
            song_interval_s: w.scalar("cfg.song_interval_s")?,
        })
    }

    pub fn params(&self) -> usize {
        self.voice.params() + self.song.params()
    }
}

/// Shared bird base model shipped in the binary.
pub const BUILTIN_BASE: &[u8] = include_bytes!("../../assets/neural/birds_base.asnw");

/// Species adapters shipped in the binary (`ml/asmr_ml/export_multi.py`).
pub const BUILTIN: &[(&str, &[u8])] = &[
    ("chaffinch", include_bytes!("../../assets/neural/chaffinch.asna")),
    ("robin", include_bytes!("../../assets/neural/robin.asna")),
    ("song_thrush", include_bytes!("../../assets/neural/song_thrush.asna")),
    ("nightingale", include_bytes!("../../assets/neural/nightingale.asna")),
    ("blackbird", include_bytes!("../../assets/neural/blackbird.asna")),
];

/// The parsed built-in base (parsed once).
pub fn base() -> Result<&'static Weights, Error> {
    static BASE: OnceLock<Result<Weights, String>> = OnceLock::new();
    BASE.get_or_init(|| Weights::parse(BUILTIN_BASE).map_err(|e| e.to_string())).as_ref().map_err(|e| Error(e.clone()))
}

/// Built-in species, composed on first use and shared by all voices of that species. Call it
/// when building the scene, not from the audio thread.
pub fn builtin(species: &str) -> Option<Arc<NeuralModel>> {
    type Cache = Mutex<Vec<(&'static str, Arc<NeuralModel>)>>;
    static CACHE: OnceLock<Cache> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(Vec::new()));
    let mut cache = cache.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, m)) = cache.iter().find(|(n, _)| *n == species) {
        return Some(m.clone());
    }
    let (name, bytes) = BUILTIN.iter().find(|(n, _)| *n == species)?;
    match base().and_then(|b| NeuralModel::compose(b, bytes)) {
        Ok(m) => {
            let m = Arc::new(m);
            cache.push((name, m.clone()));
            Some(m)
        }
        Err(e) => {
            eprintln!("asmr-core: built-in species {name}: {e}");
            None
        }
    }
}

/// A species adapter that is not built in (e.g. downloaded later), on the built-in base.
pub fn load_adapter(bytes: &[u8]) -> Result<Arc<NeuralModel>, Error> {
    Ok(Arc::new(NeuralModel::compose(base()?, bytes)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_models_load() {
        for (name, _) in BUILTIN {
            let m = builtin(name).unwrap_or_else(|| panic!("{name} failed to load"));
            assert!(m.params() > 10_000);
            assert!(Arc::ptr_eq(&m, &builtin(name).unwrap()), "composed once, then shared");
        }
    }

    #[test]
    fn bird_sings_finite_audio_without_allocating_state_changes() {
        let m = builtin("chaffinch").unwrap();
        let mut b = NeuralBird::new(m, 48_000.0, 7);
        let c = BirdControls { activity: 1.0, ..Default::default() };
        let mut buf = vec![0.0f32; 512];
        let (mut energy, mut peak, mut sang) = (0.0f64, 0.0f32, false);
        for _ in 0..(48_000 * 8 / 512) {
            b.render(&mut buf, &c);
            sang |= b.is_singing();
            for &s in &buf {
                assert!(s.is_finite());
                peak = peak.max(s.abs());
                energy += (s as f64).powi(2);
            }
        }
        assert!(sang, "never started a song");
        assert!(energy > 1.0, "silent: energy {energy}");
        assert!(peak < 4.0, "too loud: peak {peak}");
    }

    #[test]
    fn works_at_44k1() {
        let m = builtin("chaffinch").unwrap();
        let mut b = NeuralBird::new(m, 44_100.0, 3);
        let c = BirdControls { activity: 1.0, ..Default::default() };
        let mut buf = vec![0.0f32; 441];
        for _ in 0..400 {
            b.render(&mut buf, &c);
            assert!(buf.iter().all(|s| s.is_finite()));
        }
    }
}
