//! Sound generators. Each produces a mono signal; the engine places it in space.
//!
//! Every generator sits behind the same small interface, [`Generator::render`], so the
//! procedural models here can be swapped for neural ones (core/src/neural/) per emitter
//! without touching the director or the engine.

mod bird;
mod water;
mod wind;
mod wings;

pub use bird::{Bird, VoiceStyle};
pub use water::BubbleStream;
pub use wind::WindSource;
pub use wings::{WingStyle, Wings};

/// Control values for one block. The engine fills the environment fields, the director the
/// behavioural ones; each generator reads only what it needs.
#[derive(Clone, Copy, Debug, Default)]
pub struct Controls {
    /// River flow at this emitter, 0..1.
    pub water: f32,
    /// Local gust strength from the wind field.
    pub gust: f32,
    /// How eager a voice is to sing, 0..1 (0 = silent).
    pub activity: f32,
    /// Voice mode: short flight / contact calls instead of full song.
    pub calling: bool,
    /// Wingbeat effort, 0 = perched / gliding, 1 = take-off clatter.
    pub flap: f32,
}

/// A mono sound generator. Called on the audio thread once per control block:
/// no allocation, no locks.
pub trait Generator: Send {
    fn render(&mut self, out: &mut [f32], controls: &Controls);
    /// Dominant pitch right now if the generator has one (voices), else 0.
    fn pitch(&self) -> f32 {
        0.0
    }
    /// Event rate for telemetry (bubbles per second for water), else 0.
    fn event_rate(&self) -> f32 {
        0.0
    }
}

impl Generator for BubbleStream {
    fn render(&mut self, out: &mut [f32], c: &Controls) {
        BubbleStream::render(self, out, c.water)
    }
    fn event_rate(&self) -> f32 {
        self.rate()
    }
}

impl Generator for WindSource {
    fn render(&mut self, out: &mut [f32], c: &Controls) {
        WindSource::render(self, out, c.gust)
    }
}

/// What a flying animal sounds like: a voice (procedural or neural) plus its wings, both at
/// the same point in space.
pub struct Flyer {
    pub voice: Box<dyn Generator>,
    pub wings: Wings,
    /// Linear output gain (how loud this individual is at 1 m relative to the species).
    pub gain: f32,
    scratch: [f32; crate::engine::BLOCK],
}

impl Flyer {
    pub fn new(voice: Box<dyn Generator>, wings: Wings, gain: f32) -> Self {
        Self { voice, wings, gain, scratch: [0.0; crate::engine::BLOCK] }
    }
}

impl Generator for Flyer {
    fn render(&mut self, out: &mut [f32], c: &Controls) {
        self.voice.render(out, c);
        let mut off = 0;
        while off < out.len() {
            let len = (out.len() - off).min(self.scratch.len());
            let tmp = &mut self.scratch[..len];
            self.wings.render(tmp, c.flap);
            for (o, w) in out[off..off + len].iter_mut().zip(tmp.iter()) {
                *o = (*o + *w) * self.gain;
            }
            off += len;
        }
    }
    fn pitch(&self) -> f32 {
        self.voice.pitch()
    }
}
