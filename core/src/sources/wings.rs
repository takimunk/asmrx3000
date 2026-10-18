//! Wingbeats: each downstroke pushes a short burst of band-limited turbulence, so a flapping
//! bird is a train of noise pulses at the wingbeat frequency. That rhythm is one of the
//! clearest spatial cues of a take-off or a close fly-by.
//!
//! Small passerines fly in bounds (bursts of flapping, then a closed-wing glide), pigeons
//! clatter on take-off (wing claps), owls are silent by design of their feathers, gulls and
//! ravens row slowly with deep whooshing beats, larks flutter.
//!
//! Two ways to drive it: `render(out, flap)` with a steady cruising `flap` lets the wings
//! improvise the bounding rhythm themselves (flight-mode birds); `render_driven` follows the
//! effort exactly, including 0 for every glide, so the director's flight styles
//! (`director::flight_motion`) decide when the wings beat and the path rises and falls with
//! them.

use core::f32::consts::TAU;

use crate::dsp::Svf;
use crate::rng::Rng;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WingStyle {
    /// ~16 Hz, bounding flight.
    Passerine,
    /// ~7 Hz, loud whistling beats, claps at take-off.
    Pigeon,
    /// Silent flight.
    Owl,
    /// ~9 Hz, soft, continuous.
    Swallow,
    /// ~3 Hz, deep whooshing beats (gulls, ravens).
    Large,
    /// ~22 Hz, soft fluttering (larks hovering).
    Lark,
}

pub struct Wings {
    sample_rate: f32,
    rng: Rng,
    style: WingStyle,
    rate: f32,
    gain: f32,
    phase: f32,
    band: Svf,
    flap: f32,
    /// Bounding flight: seconds left in the current flapping / gliding bout.
    bout: f32,
    gliding: bool,
    clap: f32,
}

impl Wings {
    pub fn new(sample_rate: f32, seed: u64, style: WingStyle) -> Self {
        let mut rng = Rng::new(seed);
        let (rate, gain, fc) = match style {
            WingStyle::Passerine => (rng.range(14.0, 18.0), 0.25, 1800.0),
            WingStyle::Pigeon => (rng.range(6.0, 8.0), 0.6, 1100.0),
            WingStyle::Owl => (4.0, 0.0, 600.0),
            WingStyle::Swallow => (rng.range(8.0, 10.0), 0.12, 2200.0),
            WingStyle::Large => (rng.range(2.8, 3.6), 0.5, 650.0),
            WingStyle::Lark => (rng.range(20.0, 24.0), 0.1, 2600.0),
        };
        Self {
            sample_rate,
            rng,
            style,
            rate,
            gain,
            phase: 0.0,
            band: Svf::new(fc, 0.8, sample_rate),
            flap: 0.0,
            bout: 0.0,
            gliding: false,
            clap: 0.0,
        }
    }

    /// `flap` 0..1: 0 perched/gliding, ~0.5 cruising, 1 take-off/landing effort. Small
    /// passerines add their own flap/glide bouts while cruising.
    pub fn render(&mut self, out: &mut [f32], flap: f32) {
        self.render_driven(out, flap, false)
    }

    /// As `render`; with `driven` the effort is followed as given (no own bouts).
    pub fn render_driven(&mut self, out: &mut [f32], flap: f32, driven: bool) {
        let dt = out.len() as f32 / self.sample_rate;
        // Faster response when driven: a glide must silence the wings within a bound.
        let (tau, floor) = if driven { (0.012, 0.02) } else { (0.05, 1e-3) };
        self.flap += (flap.clamp(0.0, 1.0) - self.flap) * (dt / tau).min(1.0);
        if self.gain == 0.0 || self.flap < floor {
            out.fill(0.0);
            return;
        }
        if driven {
            self.gliding = false;
        } else if self.style == WingStyle::Passerine {
            self.bout -= dt;
            if self.bout <= 0.0 {
                // Hard effort keeps flapping; cruising alternates flap and glide bouts.
                self.gliding = !self.gliding && self.flap < 0.8;
                self.bout = if self.gliding { self.rng.range(0.12, 0.3) } else { self.rng.range(0.25, 0.5) };
            }
        }
        let amp = if self.gliding { 0.0 } else { self.gain * self.flap.powf(0.8) };
        let inc = self.rate * (0.85 + 0.3 * self.flap) / self.sample_rate;
        let clap_decay = (-1.0 / (0.002 * self.sample_rate)).exp();
        // Large wings: a long whoosh over most of the downstroke.
        let large = self.style == WingStyle::Large;
        for o in out.iter_mut() {
            let prev = self.phase;
            self.phase += inc;
            if self.phase >= 1.0 {
                self.phase -= 1.0;
            }
            // Pigeons clap their wings together at the top of the upstroke when taking off.
            if self.style == WingStyle::Pigeon && prev < 0.5 && self.phase >= 0.5 && self.flap > 0.85 && amp > 0.0 {
                self.clap = self.rng.range(0.6, 1.0);
            }
            // Downstroke is the first half of the cycle.
            let down = if self.phase < 0.5 { (TAU * self.phase).sin().max(0.0) } else { 0.0 };
            let env = if large { 0.15 + down } else { down * down };
            let (_, bp, _) = self.band.process(self.rng.bipolar());
            let click = self.clap * self.rng.bipolar();
            self.clap *= clap_decay;
            *o = amp * (bp * env * 2.0 + click * 0.5);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flapping_is_audible_and_owls_are_silent() {
        let energy = |style| {
            let mut w = Wings::new(48_000.0, 3, style);
            let mut buf = [0.0f32; 64];
            let mut e = 0.0f32;
            for _ in 0..750 {
                w.render(&mut buf, 1.0);
                e += buf.iter().map(|v| v * v).sum::<f32>();
            }
            e
        };
        assert!(energy(WingStyle::Pigeon) > energy(WingStyle::Passerine));
        assert!(energy(WingStyle::Passerine) > 0.1);
        assert_eq!(energy(WingStyle::Owl), 0.0);
        assert!(energy(WingStyle::Large) > energy(WingStyle::Lark));
    }

    #[test]
    fn driven_glides_are_silent() {
        let mut w = Wings::new(48_000.0, 5, WingStyle::Passerine);
        let mut buf = [0.0f32; 64];
        for _ in 0..100 {
            w.render_driven(&mut buf, 0.65, true);
        }
        assert!(buf.iter().any(|v| v.abs() > 1e-4));
        for _ in 0..40 {
            w.render_driven(&mut buf, 0.0, true);
        }
        assert!(buf.iter().all(|v| *v == 0.0), "glide must be silent within ~50 ms");
    }
}
