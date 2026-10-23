//! Wingbeats: each downstroke pushes a short burst of band-limited turbulence, so a flapping
//! bird is a train of noise pulses at the wingbeat frequency. That rhythm is one of the
//! clearest spatial cues of a take-off or a close fly-by.
//!
//! Real wingbeats are never a machine-gun "trrr": every beat is a separate gesture. The period
//! jitters by a few per cent, the force of the stroke (and with it the level and the
//! brightness of the turbulence) varies beat to beat, the first beats of a bout push hardest,
//! the downstroke is not always the same share of the cycle, and the feathers add their own
//! fluttering hiss on top of the air pushed by the wing. So each beat here draws its own
//! period, force, spectral centre, resonance and stroke shape when it starts, and a bird that
//! flaps on its own does so in bouts of a random number of beats (one flick, two, five)
//! between glides.
//!
//! Small passerines fly in bounds (bursts of flapping, then a closed-wing glide), pigeons
//! clatter on take-off (wing claps), owls are silent by design of their feathers, gulls and
//! ravens row slowly with deep whooshing beats, larks flutter.
//!
//! Wing noise is a near-field sound: a passerine's wings are a soft flutter at a metre or two
//! and gone at twenty, while a pigeon's clatter carries. The levels below are at 1 m and the
//! propagation (spreading, air absorption of the highs) does the rest.
//!
//! Two ways to drive it: `render(out, flap)` with a steady cruising `flap` lets the wings
//! improvise the bouts themselves; `render_driven` follows the effort exactly, including 0 for
//! every glide, so the director's flight styles (`director::flight_motion`) decide when the
//! wings beat and the path rises and falls with them (each beat still varies).

use core::f32::consts::PI;

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

/// How a style beats: nominal rate, level at 1 m, spectral centre of the pushed air, the
/// low "thump" of the downstroke (centre and share), period jitter (σ of log period), force
/// jitter (σ of log amplitude), feather hiss share, and the mean number of beats per
/// self-driven bout (0 = flaps continuously).
///
/// Recordings of close wingbeats have their energy at 200..1000 Hz (centroid 0.5..1 kHz): the
/// downstroke is a pressure pulse of air, a thump of tens to a few hundred Hz, with a broad
/// body around the wing's chord frequency; the feathers' rustle above 2 kHz is 15..20 dB
/// weaker. Only a pigeon's clap is sharp.
struct Style {
    rate: (f32, f32),
    gain: f32,
    fc: f32,
    thump_fc: f32,
    thump: f32,
    period_jitter: f32,
    force_jitter: f32,
    feathers: f32,
    bout: f32,
}

fn style(s: WingStyle) -> Style {
    match s {
        WingStyle::Passerine => Style { rate: (14.0, 18.0), gain: 0.15, fc: 1250.0, thump_fc: 170.0, thump: 1.0, period_jitter: 0.07, force_jitter: 0.35, feathers: 0.3, bout: 4.0 },
        WingStyle::Pigeon => Style { rate: (6.0, 8.0), gain: 0.55, fc: 950.0, thump_fc: 130.0, thump: 1.5, period_jitter: 0.09, force_jitter: 0.3, feathers: 0.3, bout: 0.0 },
        WingStyle::Owl => Style { rate: (4.0, 4.0), gain: 0.0, fc: 400.0, thump_fc: 100.0, thump: 0.0, period_jitter: 0.1, force_jitter: 0.3, feathers: 0.0, bout: 0.0 },
        WingStyle::Swallow => Style { rate: (8.0, 10.0), gain: 0.07, fc: 1350.0, thump_fc: 180.0, thump: 0.8, period_jitter: 0.1, force_jitter: 0.4, feathers: 0.3, bout: 5.0 },
        WingStyle::Large => Style { rate: (2.8, 3.6), gain: 0.38, fc: 750.0, thump_fc: 100.0, thump: 1.8, period_jitter: 0.06, force_jitter: 0.25, feathers: 0.2, bout: 0.0 },
        WingStyle::Lark => Style { rate: (20.0, 24.0), gain: 0.06, fc: 1450.0, thump_fc: 190.0, thump: 0.7, period_jitter: 0.08, force_jitter: 0.35, feathers: 0.3, bout: 0.0 },
    }
}

pub struct Wings {
    sample_rate: f32,
    rng: Rng,
    style: WingStyle,
    rate: f32,
    gain: f32,
    fc: f32,
    thump_fc: f32,
    thump: f32,
    period_jitter: f32,
    force_jitter: f32,
    feathers: f32,
    bout_mean: f32,
    /// The thump of the downstroke, the air pushed by the wing, and the feathers' hiss.
    thumper: Svf,
    body: Svf,
    feather: Svf,
    flap: f32,
    // The beat being played.
    phase: f32,
    inc: f32,
    force: f32,
    bn: f32,
    tn: f32,
    hn: f32,
    down: f32,
    beating: bool,
    /// Feather flutter: low-passed noise modulating the hiss.
    flutter: f32,
    flutter_coef: f32,
    /// Self-driven bouts: beats left in this bout, seconds of glide left.
    beats_left: u32,
    glide: f32,
    /// Index of the beat within its bout (the first strokes push hardest).
    nth: u32,
    clap: f32,
}

impl Wings {
    pub fn new(sample_rate: f32, seed: u64, style_kind: WingStyle) -> Self {
        let mut rng = Rng::new(seed);
        let s = style(style_kind);
        let rate = rng.range(s.rate.0, s.rate.1);
        let fc = s.fc * rng.range(0.85, 1.15);
        Self {
            sample_rate,
            style: style_kind,
            rate,
            gain: s.gain,
            fc,
            thump_fc: s.thump_fc * rng.range(0.85, 1.2),
            thump: s.thump,
            period_jitter: s.period_jitter,
            force_jitter: s.force_jitter,
            feathers: s.feathers,
            bout_mean: s.bout,
            body: Svf::new(fc, 0.8, sample_rate),
            feather: Svf::new((fc * 4.0).min(0.4 * sample_rate), 1.2, sample_rate),
            thumper: Svf::new(s.thump_fc, 0.7, sample_rate),
            flap: 0.0,
            phase: 0.0,
            inc: 0.0,
            force: 0.0,
            bn: 1.0,
            tn: 1.0,
            hn: 1.0,
            down: 0.5,
            beating: false,
            flutter: 0.0,
            flutter_coef: 1.0 - (-2.0 * PI * 180.0 / sample_rate).exp(),
            beats_left: 0,
            glide: 0.0,
            nth: 0,
            clap: 0.0,
            rng,
        }
    }

    /// `flap` 0..1: 0 perched/gliding, ~0.5 cruising, 1 take-off/landing effort. Small
    /// passerines add their own flap/glide bouts while cruising.
    pub fn render(&mut self, out: &mut [f32], flap: f32) {
        self.render_driven(out, flap, false)
    }

    /// Draws the next beat: its period, force, colour and stroke shape.
    fn next_beat(&mut self, driven: bool) {
        let rng = &mut self.rng;
        // Self-driven bouts: a random number of beats, then a glide on closed wings.
        if !driven && self.bout_mean > 0.0 && self.flap < 0.8 {
            if self.beats_left == 0 {
                if self.beating || self.nth > 0 {
                    self.glide = rng.range(0.08, 0.18) + rng.range(0.0, 0.3) * (1.0 - self.flap);
                    self.beating = false;
                    self.nth = 0;
                }
                // Geometric number of beats around the style's mean: runs of a few beats, now and then ten.
                let p = 1.0 / self.bout_mean;
                let mut n = 1;
                while n < 14 && rng.uniform() > p {
                    n += 1;
                }
                self.beats_left = n;
                if self.glide > 0.0 {
                    return;
                }
            }
            self.beats_left -= 1;
        }
        let effort = 0.85 + 0.3 * self.flap;
        let period = (1.0 / (self.rate * effort)) * (self.period_jitter * rng.gauss()).clamp(-2.5, 2.5).exp();
        self.inc = 1.0 / (period * self.sample_rate);
        // The first strokes of a bout push hardest; the rest settle.
        let push = 1.0 + 0.35 * (-(self.nth as f32) / 1.5).exp();
        self.force = push * (self.force_jitter * rng.gauss()).clamp(-2.5, 2.5).exp();
        // A harder stroke is also brighter (faster tip speed sheds smaller eddies).
        let fc = self.fc * self.force.powf(0.35) * (0.15 * rng.gauss()).exp();
        let nyq = 0.42 * self.sample_rate;
        self.body.set(fc.clamp(150.0, nyq), rng.range(0.6, 1.1), self.sample_rate);
        self.feather.set((fc * rng.range(3.0, 5.0)).clamp(1200.0, nyq), rng.range(0.9, 1.6), self.sample_rate);
        let tfc = self.thump_fc * self.force.powf(0.2) * (0.12 * rng.gauss()).exp();
        self.thumper.set(tfc, rng.range(0.6, 0.9), self.sample_rate);
        // Equal turbulence power per Hz whatever the centre: the filters' bandwidth grows with
        // their centre, so normalise to a 1 kHz reference.
        self.bn = (1000.0 / fc.clamp(150.0, nyq)).sqrt();
        self.tn = (1000.0 / tfc).sqrt();
        self.hn = (1000.0 / (fc * 3.5).clamp(1200.0, nyq)).sqrt();
        self.down = rng.range(0.42, 0.6);
        self.beating = true;
        self.nth += 1;
    }

    /// As `render`; with `driven` the effort is followed as given (no own bouts).
    pub fn render_driven(&mut self, out: &mut [f32], flap: f32, driven: bool) {
        let sr = self.sample_rate;
        let dt = out.len() as f32 / sr;
        // Faster response when driven: a glide must silence the wings within a bound.
        let (tau, floor) = if driven { (0.012, 0.02) } else { (0.05, 1e-3) };
        let was = self.flap;
        self.flap += (flap.clamp(0.0, 1.0) - self.flap) * (dt / tau).min(1.0);
        if self.gain == 0.0 || self.flap < floor {
            out.fill(0.0);
            // The next flight starts a fresh bout with a fresh stroke.
            self.phase = 0.0;
            self.beating = false;
            self.beats_left = 0;
            self.glide = 0.0;
            self.nth = 0;
            return;
        }
        if was < floor || !self.beating && self.glide <= 0.0 && self.inc == 0.0 {
            self.next_beat(driven);
        }
        let amp = self.gain * self.flap.powf(0.8);
        let clap_decay = (-1.0 / (0.002 * sr)).exp();
        let large = self.style == WingStyle::Large;
        let pigeon = self.style == WingStyle::Pigeon;
        for o in out.iter_mut() {
            if self.glide > 0.0 {
                self.glide -= 1.0 / sr;
                if self.glide <= 0.0 {
                    self.glide = 0.0;
                    self.next_beat(driven);
                }
                *o = 0.0;
                continue;
            }
            let prev = self.phase;
            self.phase += self.inc;
            if self.phase >= 1.0 {
                self.phase -= 1.0;
                self.next_beat(driven);
                if self.glide > 0.0 {
                    self.phase = 0.0;
                    *o = 0.0;
                    continue;
                }
            }
            let p = self.phase;
            let d = self.down;
            // Pigeons clap their wings together at the top of the upstroke when taking off
            // (not on every beat).
            if pigeon && prev < d && p >= d && self.flap > 0.85 && self.rng.uniform() < 0.8 {
                self.clap = self.rng.range(0.5, 1.0) * self.force.min(1.5);
            }
            // The downstroke pushes the air; the upstroke only whispers (large wings whoosh
            // through both).
            let env = if p < d {
                let s = (PI * p / d).sin();
                if large {
                    0.15 + s
                } else {
                    s
                }
            } else {
                let s = (PI * (p - d) / (1.0 - d)).sin();
                if large {
                    0.15 + 0.35 * s
                } else {
                    0.4 * s
                }
            };
            let (bp, _, _) = self.body.process(self.rng.bipolar());
            let (thump, _, _) = self.thumper.process(self.rng.bipolar());
            self.flutter += (self.rng.bipolar() - self.flutter) * self.flutter_coef;
            let (_, fb, _) = self.feather.process(self.rng.bipolar());
            let hiss = fb * self.feathers * self.hn * (1.0 + 2.5 * self.flutter).max(0.0);
            let click = self.clap * self.rng.bipolar();
            self.clap *= clap_decay;
            // The thump is the pressure pulse itself (smooth, one lobe); the body and the hiss
            // are turbulence on top of it.
            // (`env` can round to a hair below zero at the end of a stroke: no NaN from it.)
            let push = self.thump * self.tn * thump * env.max(0.0).sqrt();
            *o = amp * (self.force * env * (bp * 2.0 * self.bn + hiss) + self.force * push + click * 0.5);
        }
        self.body.flush();
        self.feather.flush();
        self.thumper.flush();
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
        assert!(energy(WingStyle::Passerine) > 0.03);
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

    /// Beat onsets: the times at which the 1 ms envelope rises through a threshold.
    fn beats(style: WingStyle, flap: f32, secs: f32) -> (Vec<f32>, Vec<f32>) {
        let sr = 48_000.0;
        let mut w = Wings::new(sr, 11, style);
        let mut buf = [0.0f32; 48];
        let (mut on, mut peaks) = (Vec::new(), Vec::new());
        let (mut above, mut peak) = (false, 0.0f32);
        for k in 0..(secs * 1000.0) as usize {
            w.render(&mut buf, flap);
            let e = (buf.iter().map(|v| v * v).sum::<f32>() / 48.0).sqrt();
            let t = k as f32 / 1000.0;
            if !above && e > 0.006 {
                above = true;
                on.push(t);
                peak = e;
            } else if above {
                peak = peak.max(e);
                if e < 0.002 {
                    above = false;
                    peaks.push(peak);
                }
            }
        }
        (on, peaks)
    }

    #[test]
    fn beats_vary_and_come_in_bouts() {
        // A cruising finch: beats of varying force, in bouts separated by glides.
        let (on, peaks) = beats(WingStyle::Passerine, 0.5, 20.0);
        assert!(on.len() > 40, "{} beats", on.len());
        let gaps: Vec<f32> = on.windows(2).map(|w| w[1] - w[0]).collect();
        let glides = gaps.iter().filter(|g| **g > 0.1).count();
        assert!(glides > 8, "glides between bouts: {glides}");
        // Not a fixed rhythm: the intervals within bouts spread.
        let short: Vec<f32> = gaps.iter().copied().filter(|g| *g < 0.1).collect();
        let mean = short.iter().sum::<f32>() / short.len() as f32;
        let sd = (short.iter().map(|g| (g - mean).powi(2)).sum::<f32>() / short.len() as f32).sqrt();
        assert!(sd / mean > 0.03, "period spread {:.3}", sd / mean);
        // Nor a fixed force.
        let (lo, hi) = peaks.iter().fold((f32::MAX, 0.0f32), |a, p| (a.0.min(*p), a.1.max(*p)));
        assert!(hi / lo > 2.0, "force spread {:.2}", hi / lo);
    }
}
