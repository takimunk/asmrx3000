//! Running water from first principles: the sound of a brook is mostly the sum of
//! many small air bubbles ringing at their Minnaert resonance as they form.
//!
//! Each bubble is a damped sinusoid with a slight upward chirp (van den Doel, 2005,
//! "Physically based models for liquid sounds"):
//!   f0 = 3.26 / r            (Hz, r in metres)
//!   d  = 0.043 f0 + 0.0014 f0^1.5   (damping, 1/s)
//!   f(t) = f0 (1 + sigma t), sigma = 0.1 d
//! A river is several of these streams spread along a line, which gives it width.

use core::f32::consts::TAU;

use crate::dsp::Svf;
use crate::rng::Rng;

const MAX_BUBBLES: usize = 64;
const R_MIN: f32 = 0.000_6; // 0.6 mm -> ~5.4 kHz
const R_MAX: f32 = 0.008; // 8 mm   -> ~400 Hz
const UPDATE: usize = 16; // samples between chirp/frequency updates

struct Bubble {
    re: f32,
    im: f32,
    cr: f32,
    ci: f32,
    freq: f32,
    dfreq: f32,
    decay: f32,
    amp: f32,
}

impl Bubble {
    #[inline]
    fn update_rotation(&mut self, sample_rate: f32) {
        let (s, c) = (TAU * self.freq / sample_rate).sin_cos();
        self.cr = c * self.decay;
        self.ci = s * self.decay;
    }
}

pub struct BubbleStream {
    sample_rate: f32,
    rng: Rng,
    bubbles: Vec<Bubble>,
    bed: Svf,
    base_rate: f32,
    rate: f32,
    lfo: f32,
    lfo_rate: f32,
    counter: usize,
}

impl BubbleStream {
    pub fn new(sample_rate: f32, seed: u64, base_rate: f32) -> Self {
        let mut rng = Rng::new(seed);
        let lfo_rate = rng.range(0.05, 0.15);
        let lfo = rng.uniform();
        Self {
            sample_rate,
            rng,
            bubbles: Vec::with_capacity(MAX_BUBBLES),
            bed: Svf::new(700.0, 0.5, sample_rate),
            base_rate,
            rate: 0.0,
            lfo,
            lfo_rate,
            counter: 0,
        }
    }

    /// Current bubble rate in bubbles per second.
    pub fn rate(&self) -> f32 {
        self.rate
    }

    fn spawn(&mut self) {
        // Power-law radius distribution: many small bubbles, few large ones.
        let u = self.rng.uniform();
        let r = R_MIN * (R_MAX / R_MIN).powf(u.powf(1.6));
        let f = 3.26 / r;
        if f > 0.45 * self.sample_rate {
            return;
        }
        let damping = 0.043 * f + 0.0014 * f.powf(1.5);
        let sigma = 0.1 * damping;
        let mut b = Bubble {
            re: 1.0,
            im: 0.0,
            cr: 0.0,
            ci: 0.0,
            freq: f,
            dfreq: f * sigma * UPDATE as f32 / self.sample_rate,
            decay: (-damping / self.sample_rate).exp(),
            amp: (r / R_MAX).powf(0.6) * self.rng.range(0.3, 1.0),
        };
        b.update_rotation(self.sample_rate);
        self.bubbles.push(b);
    }

    /// `water` is the flow intensity in 0..1.
    pub fn render(&mut self, out: &mut [f32], water: f32) {
        let w = water.clamp(0.0, 1.0);
        self.lfo = (self.lfo + self.lfo_rate * out.len() as f32 / self.sample_rate) % 1.0;
        self.rate = self.base_rate * (0.1 + 1.6 * w) * (1.0 + 0.35 * (TAU * self.lfo).sin());
        if w < 1e-3 && self.bubbles.is_empty() {
            self.rate = 0.0;
            out.fill(0.0);
            return;
        }
        let p = self.rate / self.sample_rate;
        let bed_gain = 0.04 * w;

        for o in out.iter_mut() {
            if self.counter % UPDATE == 0 {
                self.bubbles.retain(|b| b.re * b.re + b.im * b.im > 1e-6);
                let sr = self.sample_rate;
                for b in self.bubbles.iter_mut() {
                    b.freq += b.dfreq;
                    b.update_rotation(sr);
                }
            }
            self.counter = self.counter.wrapping_add(1);

            if w > 1e-3 && self.bubbles.len() < MAX_BUBBLES && self.rng.uniform() < p {
                self.spawn();
            }

            let mut s = 0.0f32;
            for b in self.bubbles.iter_mut() {
                let re = b.re * b.cr - b.im * b.ci;
                let im = b.re * b.ci + b.im * b.cr;
                b.re = re;
                b.im = im;
                s += im * b.amp;
            }
            let (_, bed, _) = self.bed.process(self.rng.bipolar());
            *o = s * 0.5 + bed * bed_gain;
        }
    }
}
