//! Wind in the canopy: a broadband "body" (turbulent noise whose spectrum shifts up with
//! gust speed) plus leaf rustle (sparse short noise grains whose density grows with the
//! square of the local gust). Driven by the shared `WindField`.

use crate::dsp::Svf;
use crate::rng::Rng;

pub struct WindSource {
    sample_rate: f32,
    rng: Rng,
    body: Svf,
    leaves: Svf,
    leaf_env: f32,
    leaf_amp: f32,
    leaf_decay: f32,
    gust: f32,
    timbre: f32,
}

impl WindSource {
    /// `timbre` (~0.7..1.3) shifts the spectrum so neighbouring trees sound different.
    pub fn new(sample_rate: f32, seed: u64, timbre: f32) -> Self {
        Self {
            sample_rate,
            rng: Rng::new(seed),
            body: Svf::new(200.0, 0.7, sample_rate),
            leaves: Svf::new(4200.0 * timbre, 0.9, sample_rate),
            leaf_env: 0.0,
            leaf_amp: 0.0,
            leaf_decay: (-1.0 / (0.0035 * sample_rate)).exp(),
            gust: 0.0,
            timbre,
        }
    }

    /// `local_gust` comes from the wind field at this source's position.
    pub fn render(&mut self, out: &mut [f32], local_gust: f32) {
        self.gust += (local_gust - self.gust) * 0.05;
        let g = self.gust;
        if g < 1e-4 && self.leaf_env < 1e-4 {
            out.fill(0.0);
            return;
        }
        self.body.set((90.0 + 520.0 * g * self.timbre).min(2500.0), 0.7, self.sample_rate);
        let body_amp = 0.5 * g.powf(1.5);
        let p_leaf = 1600.0 * (g * g).min(3.0) / self.sample_rate;
        let leaf_gain = 0.35 * g.min(1.5);

        for o in out.iter_mut() {
            let (_, body, _) = self.body.process(self.rng.bipolar());
            if self.rng.uniform() < p_leaf {
                self.leaf_env = 1.0;
                self.leaf_amp = self.rng.range(0.15, 1.0);
            }
            let (_, rustle, _) = self.leaves.process(self.rng.bipolar());
            let leaf = rustle * self.leaf_env * self.leaf_amp * leaf_gain;
            self.leaf_env *= self.leaf_decay;
            *o = body * body_amp + leaf;
        }
    }
}
