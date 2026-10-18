//! Bird voices: the trained neural voice where the species has one (docs/neural-sound.md),
//! the procedural voice otherwise. Short flight / contact / alarm calls stay procedural: the
//! neural models are trained on full song only.
//!
//! Neural song: chaffinch, robin, song thrush, nightingale (in the shared base) and
//! blackbird (adapter). Procedural: tawny owl, cuckoo, wood pigeon, swallow, skylark, gull,
//! raven (no training data yet, see docs/neural-sound.md §7.9).

use super::Species;
use crate::neural::{self, BirdControls, NeuralBird};
use crate::sources::{Bird, Controls, Generator};

/// Name of the built-in neural model for a species, if one is trained.
pub fn neural_model_name(species: Species) -> Option<&'static str> {
    match species {
        Species::Chaffinch => Some("chaffinch"),
        Species::Robin => Some("robin"),
        Species::SongThrush => Some("song_thrush"),
        Species::Nightingale => Some("nightingale"),
        Species::Blackbird => Some("blackbird"),
        _ => None,
    }
}

pub struct BirdVoice {
    procedural: Bird,
    neural: Option<NeuralBird>,
    using_neural: bool,
    /// Linear level match neural -> procedural (mix/calibration.rs).
    neural_gain: f32,
}

/// Builds the voice for one bird. Parses the neural model on first use, so call it while
/// building the scene, not from the audio thread.
pub fn bird_voice(species: Species, procedural: Bird, sample_rate: f32, seed: u64) -> Box<dyn Generator> {
    let neural = neural_model_name(species).and_then(neural::builtin).map(|m| NeuralBird::new(m, sample_rate, seed));
    let neural_gain = 10f32.powf(crate::mix::calibration::neural_trim_db(species) / 20.0);
    Box::new(BirdVoice { procedural, neural, using_neural: false, neural_gain })
}

impl Generator for BirdVoice {
    fn render(&mut self, out: &mut [f32], c: &Controls) {
        self.using_neural = c.neural && !c.calling && self.neural.is_some();
        match &mut self.neural {
            Some(n) if self.using_neural => {
                if c.reply {
                    n.hurry(1.4);
                }
                n.render(out, &BirdControls { activity: c.activity, wait: c.wait, ..Default::default() });
                if !c.legacy_levels {
                    out.iter_mut().for_each(|v| *v *= self.neural_gain);
                }
            }
            _ => self.procedural.render(out, c),
        }
    }

    fn busy(&self) -> bool {
        match &self.neural {
            Some(n) if self.using_neural => n.is_singing(),
            _ => self.procedural.busy(),
        }
    }

    fn pitch(&self) -> f32 {
        match &self.neural {
            Some(n) if self.using_neural => {
                if n.is_singing() {
                    n.freq()
                } else {
                    0.0
                }
            }
            _ => self.procedural.pitch(),
        }
    }
}
