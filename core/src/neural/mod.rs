//! Neural sound sources: tiny per-species models trained in `ml/` (see docs/neural-sound.md).
//!
//! Inference is our own: a handful of layers (Linear, GRU) over plain `f32` slices, no
//! dependencies, no allocation after construction, identical on native and WASM.

pub mod bird;
pub mod layers;
pub mod song;
pub mod voice;
pub mod weights;

use std::sync::{Arc, OnceLock};

pub use bird::{BirdControls, NeuralBird};
use song::SongModel;
use voice::VoiceModel;
use weights::{Error, Weights};

/// Everything one species needs: DDSP voice, song syntax, contour bank and a few constants.
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
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, Error> {
        let w = Weights::parse(bytes)?;
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

/// Species shipped inside the binary (`core/assets/neural/*.asnw`, exported by
/// `ml/asmr_ml/export.py`).
pub const BUILTIN: &[(&str, &[u8])] = &[
    ("chaffinch", include_bytes!("../../assets/neural/chaffinch.asnw")),
    ("robin", include_bytes!("../../assets/neural/robin.asnw")),
    ("song_thrush", include_bytes!("../../assets/neural/song_thrush.asnw")),
    ("nightingale", include_bytes!("../../assets/neural/nightingale.asnw")),
];

/// Parsed built-in model, shared by all voices of that species. Parses on first use, so call
/// it when building the scene, not from the audio thread.
pub fn builtin(species: &str) -> Option<Arc<NeuralModel>> {
    static CACHE: OnceLock<Vec<(&'static str, Arc<NeuralModel>)>> = OnceLock::new();
    let all = CACHE.get_or_init(|| {
        BUILTIN
            .iter()
            .filter_map(|(name, bytes)| match NeuralModel::from_bytes(bytes) {
                Ok(m) => Some((*name, Arc::new(m))),
                Err(e) => {
                    eprintln!("asmr-core: built-in model {name}: {e}");
                    None
                }
            })
            .collect()
    });
    all.iter().find(|(n, _)| *n == species).map(|(_, m)| m.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_models_load() {
        for (name, _) in BUILTIN {
            let m = builtin(name).unwrap_or_else(|| panic!("{name} failed to load"));
            assert!(m.params() > 10_000);
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
