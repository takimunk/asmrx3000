//! Shared building blocks for the physical models: physical constants and laws, event
//! clocks, damped modes, bubbles, noise grains and parameter smoothing.
//!
//! Everything here is `Copy`/fixed-size so the models can keep it inline and stay
//! allocation-free on the audio thread.

use core::f32::consts::TAU;

pub use crate::dsp::Svf;
use crate::rng::Rng;

// ---------------------------------------------------------------------------------------
// Physics
// ---------------------------------------------------------------------------------------

/// Gravity, m/s².
pub const G: f32 = 9.81;
/// Kinematic viscosity of air at ~15 °C, m²/s.
pub const NU_AIR: f32 = 1.5e-5;
/// Atmospheric pressure, Pa.
pub const P0: f32 = 101_325.0;
/// Speed of sound in air at 20 °C, m/s (models that care call [`sound_speed`]).
pub const C_AIR: f32 = 343.0;

/// Speed of sound in air, m/s, at `t_c` °C.
pub fn sound_speed(t_c: f32) -> f32 {
    331.3 * (1.0 + t_c.max(-60.0) / 273.15).sqrt()
}

/// Maps the WIND slider (and the travelling gust field, which is in the same units) to wind
/// speed at canopy height, m/s. 0.45 (app default) = 4.8 m/s, 1.0 = 16 m/s (near gale);
/// gusts above 1 go higher. One mapping for the whole engine, so a scene may also give the
/// wind in m/s ([`wind_slider`] is the inverse).
pub fn wind_speed_ms(slider: f32) -> f32 {
    16.0 * slider.clamp(0.0, 1.8).powf(1.5)
}

/// Relative level for a flow-noise intensity law I ∝ U^n (returned as amplitude, U/U_ref
/// to the n/2), with the growth above 12 m/s reduced to linear: at storm speeds shedding
/// loses spanwise coherence and vegetation streamlines, so it does not keep rising as U^n.
pub fn flow_level(u: f32, u_ref: f32, n: f32) -> f32 {
    const KNEE: f32 = 12.0;
    (u.min(KNEE) / u_ref).powf(0.5 * n) * (u.max(KNEE) / KNEE).sqrt()
}

/// Inverse of [`wind_speed_ms`].
pub fn wind_slider(ms: f32) -> f32 {
    (ms.max(0.0) / 16.0).powf(1.0 / 1.5)
}

/// Vortex shedding frequency behind a cylinder of diameter `d` (m) in a flow of `u` m/s:
/// f = St·U/d (Strouhal). St ≈ 0.2 for 300 < Re < 2·10⁵.
pub fn strouhal_hz(st: f32, u: f32, d: f32) -> f32 {
    st * u / d.max(1e-5)
}

/// Reynolds number of a cylinder of diameter `d` in air.
pub fn reynolds(u: f32, d: f32) -> f32 {
    u * d / NU_AIR
}

/// Minnaert resonance of an air bubble of radius `r` (m) near the water surface, Hz:
/// f = (1/2πr)·sqrt(3γp₀/ρ) ≈ 3.26 / r.
pub fn minnaert_hz(r: f32) -> f32 {
    3.26 / r.max(1e-5)
}

/// Bubble damping (1/s) after van den Doel (2005): thermal + viscous + radiation losses.
pub fn bubble_damping(f: f32) -> f32 {
    0.043 * f + 0.0014 * f.powf(1.5)
}

/// Saturation vapour pressure, kPa (Tetens).
pub fn vapour_pressure_kpa(t_c: f32) -> f32 {
    0.6108 * (17.27 * t_c / (t_c + 237.3)).exp()
}

/// Rain intensity (0..1, the director's legacy `rain` scale) from a rain rate in mm/h.
/// 0.5 mm/h drizzle = 0.1, 12.5 mm/h heavy = 0.5, 50 mm/h cloudburst = 1.
pub fn rain_unit(mm_h: f32) -> f32 {
    (mm_h.max(0.0) / 50.0).sqrt().min(1.0)
}

/// Inverse of [`rain_unit`].
pub fn rain_mm_h(unit: f32) -> f32 {
    50.0 * unit.clamp(0.0, 1.0).powi(2)
}

#[inline]
pub fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[inline]
pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Flush values that would decay into subnormals.
#[inline]
pub fn flush(x: f32) -> f32 {
    if x.abs() < 1e-18 {
        0.0
    } else {
        x
    }
}

/// Activity window over the day: 1 inside `[from, to)` hours (wrapping past midnight),
/// with `soft` hours of smooth fade on both sides.
pub fn hour_window(hour: f32, from: f32, to: f32, soft: f32) -> f32 {
    if (to - from).abs() >= 24.0 {
        return 1.0;
    }
    let len = (to - from).rem_euclid(24.0);
    let t = (hour - from).rem_euclid(24.0);
    // Distance in hours into the window, measured from either edge (negative outside).
    let inside = if t <= len { t.min(len - t) } else { -((t - len).min(24.0 - t)) };
    smoothstep(-soft * 0.5, soft * 0.5, inside)
}

// ---------------------------------------------------------------------------------------
// Event clock
// ---------------------------------------------------------------------------------------

/// Inhomogeneous Poisson process by time change: integrates the (possibly changing) rate
/// and fires when the integral crosses an exponentially distributed threshold. Exact for
/// any rate trajectory, so rates can follow the wind sample by sample without bias.
#[derive(Clone, Copy)]
pub struct Events {
    acc: f32,
    thresh: f32,
}

impl Events {
    pub fn new(rng: &mut Rng) -> Self {
        Self { acc: 0.0, thresh: exp1(rng) }
    }

    /// `rate` is events per sample. Returns how many events happen in this sample.
    #[inline]
    pub fn tick(&mut self, rate: f32, rng: &mut Rng) -> u32 {
        self.acc += rate.max(0.0);
        let mut n = 0;
        while self.acc >= self.thresh && n < 8 {
            self.acc -= self.thresh;
            self.thresh = exp1(rng);
            n += 1;
        }
        if n == 8 {
            self.acc = 0.0;
        }
        n
    }
}

/// Exponential(1) variate.
#[inline]
pub fn exp1(rng: &mut Rng) -> f32 {
    -(rng.uniform().max(1e-7)).ln()
}

/// Ornstein–Uhlenbeck process: slow, mean-reverting random modulation.
#[derive(Clone, Copy, Default)]
pub struct Ou {
    pub x: f32,
}

impl Ou {
    /// Steady-state standard deviation `sigma`, correlation time `tau` seconds.
    #[inline]
    pub fn step(&mut self, dt: f32, tau: f32, sigma: f32, rng: &mut Rng) -> f32 {
        let tau = tau.max(1e-3);
        self.x += -self.x / tau * dt + sigma * (2.0 * dt / tau).sqrt() * rng.gauss();
        self.x
    }
}

/// The recent history of the gust arriving at an extended source (a crown, a stretch of
/// forest), so its parts can hear the same gust at different times: an element `x` metres
/// downwind inside the source hears it x / U later (frozen turbulence). Sampled at 100 Hz,
/// ~10 s deep; fixed size, no allocation.
#[derive(Clone, Copy)]
pub struct GustTrail {
    hist: [f32; GustTrail::N],
    pos: usize,
    acc: f32,
    filled: bool,
}

impl Default for GustTrail {
    fn default() -> Self {
        Self { hist: [0.0; Self::N], pos: 0, acc: 0.0, filled: false }
    }
}

impl GustTrail {
    const N: usize = 1024;
    const DT: f32 = 0.01;

    /// Records the gust now; call once per block with the block's duration.
    pub fn push(&mut self, g: f32, dt: f32) {
        if !self.filled {
            self.filled = true;
            self.hist.fill(g);
        }
        self.acc += dt;
        while self.acc >= Self::DT {
            self.acc -= Self::DT;
            self.pos = (self.pos + 1) % Self::N;
            self.hist[self.pos] = g;
        }
    }

    /// The gust `delay_s` seconds ago (linear interpolation, clamped to the history).
    pub fn at(&self, delay_s: f32) -> f32 {
        let x = (delay_s.max(0.0) / Self::DT).min((Self::N - 2) as f32);
        let k = x as usize;
        let f = x - k as f32;
        let a = self.hist[(self.pos + Self::N - k) % Self::N];
        let b = self.hist[(self.pos + Self::N - k - 1) % Self::N];
        a + (b - a) * f
    }
}

// ---------------------------------------------------------------------------------------
// Smoothing
// ---------------------------------------------------------------------------------------

/// One-pole smoother advanced once per block (time constant in seconds).
#[derive(Clone, Copy, Default)]
pub struct Smooth {
    pub value: f32,
}

impl Smooth {
    pub fn new(v: f32) -> Self {
        Self { value: v }
    }

    #[inline]
    pub fn step(&mut self, target: f32, dt: f32, tau: f32) -> f32 {
        let a = if tau <= 0.0 { 1.0 } else { 1.0 - (-dt / tau).exp() };
        self.value += (target - self.value) * a;
        self.value = flush(self.value);
        self.value
    }
}

/// Per-sample linear ramp from the previous block's gain to this block's, so gain changes
/// never step (no zipper noise or clicks).
#[derive(Clone, Copy, Default)]
pub struct Ramp {
    pub cur: f32,
    inc: f32,
    target: f32,
}

impl Ramp {
    pub fn new(v: f32) -> Self {
        Self { cur: v, inc: 0.0, target: v }
    }

    /// Start a ramp to `target` over `len` samples.
    #[inline]
    pub fn to(&mut self, target: f32, len: usize) {
        self.target = target;
        self.inc = (target - self.cur) / len.max(1) as f32;
    }

    /// End of block: land exactly on the target.
    #[inline]
    pub fn finish(&mut self) {
        self.cur = self.target;
        self.inc = 0.0;
    }

    #[inline]
    pub fn next(&mut self) -> f32 {
        self.cur += self.inc;
        self.cur
    }

    #[inline]
    pub fn inc(&self) -> f32 {
        self.inc
    }

    /// Snap exactly onto `target` at the end of a block (avoids drift).
    #[inline]
    pub fn settle(&mut self, target: f32) {
        self.cur = target;
        self.inc = 0.0;
    }
}

/// Gain that brings uniform white noise (variance 1/3) through an [`Svf`] bandpass
/// (unity peak, centre `fc`, quality `q`) to unit RMS.
#[inline]
pub fn band_norm(fc: f32, q: f32, sr: f32) -> f32 {
    (3.0 * sr * q / (core::f32::consts::PI * fc.max(10.0))).sqrt()
}

/// Coefficient of a one-pole lowpass with cutoff `fc`.
#[inline]
pub fn one_pole_coef(fc: f32, sr: f32) -> f32 {
    1.0 - (-TAU * fc.clamp(1.0, 0.49 * sr) / sr).exp()
}

// ---------------------------------------------------------------------------------------
// Damped modes and bubbles
// ---------------------------------------------------------------------------------------

/// A damped sinusoid as a rotating complex phasor (2 complex multiplies per sample).
#[derive(Clone, Copy, Default)]
pub struct Mode {
    re: f32,
    im: f32,
    cr: f32,
    ci: f32,
}

impl Mode {
    /// Set frequency (Hz) and damping (1/s, amplitude decays as e^{-d t}). Keeps the state.
    #[inline]
    pub fn tune(&mut self, freq: f32, damping: f32, sr: f32) {
        let (s, c) = (TAU * freq.clamp(1.0, 0.48 * sr) / sr).sin_cos();
        let g = (-damping.max(0.0) / sr).exp();
        self.cr = c * g;
        self.ci = s * g;
    }

    /// Add `amp` of excitation (starts at zero phase, so no click).
    #[inline]
    pub fn strike(&mut self, amp: f32) {
        self.re += amp;
    }

    #[inline]
    pub fn tick(&mut self) -> f32 {
        let re = self.re * self.cr - self.im * self.ci;
        let im = self.re * self.ci + self.im * self.cr;
        self.re = re;
        self.im = im;
        im
    }

    #[inline]
    pub fn energy(&self) -> f32 {
        self.re * self.re + self.im * self.im
    }

    pub fn clear(&mut self) {
        self.re = 0.0;
        self.im = 0.0;
    }
}

const BUBBLE_UPDATE: u32 = 16;

/// One ringing bubble: Minnaert frequency, van den Doel damping, rising chirp.
#[derive(Clone, Copy, Default)]
struct Bubble {
    mode: Mode,
    freq: f32,
    dfreq: f32,
    damping: f32,
    delay: u32,
    /// Samples until the next chirp update.
    sub: u32,
}

/// Fixed pool of bubbles, rendered a block at a time. `spawn` drops the request when full.
pub struct BubbleBank<const N: usize> {
    b: [Bubble; N],
    n: usize,
    spawned: u32,
}

impl<const N: usize> BubbleBank<N> {
    pub fn new() -> Self {
        Self { b: [Bubble::default(); N], n: 0, spawned: 0 }
    }

    pub fn active(&self) -> usize {
        self.n
    }

    pub fn is_full(&self) -> bool {
        self.n >= N
    }

    /// Bubbles spawned since the last call (for telemetry).
    pub fn take_spawned(&mut self) -> u32 {
        core::mem::take(&mut self.spawned)
    }

    /// Radius `r` (m), linear amplitude, chirp factor `xi` (σ = ξ·d; 0.1 after van den Doel;
    /// larger for bubbles pinched off right at the surface), start `delay` in samples from
    /// the start of the next rendered block.
    pub fn spawn(&mut self, r: f32, amp: f32, xi: f32, delay: u32, sr: f32) -> bool {
        self.spawn_damped(r, amp, xi, delay, sr, 1.0)
    }

    /// As `spawn`, with the damping multiplied by `damp`: large bubbles pinched off right at
    /// the surface wobble out of shape and break up, so they ring far shorter than a free
    /// spherical bubble would.
    pub fn spawn_damped(&mut self, r: f32, amp: f32, xi: f32, delay: u32, sr: f32, damp: f32) -> bool {
        let f = minnaert_hz(r);
        if self.n >= N || f > 0.45 * sr || f < 20.0 {
            return false;
        }
        let d = bubble_damping(f) * damp.max(0.1);
        let mut b = Bubble { mode: Mode::default(), freq: f, dfreq: f * xi * d * BUBBLE_UPDATE as f32 / sr, damping: d, delay, sub: BUBBLE_UPDATE };
        b.mode.tune(f, d, sr);
        b.mode.strike(amp);
        self.b[self.n] = b;
        self.n += 1;
        self.spawned += 1;
        true
    }

    /// Adds every bubble's next `out.len()` samples to `out`.
    pub fn render_add(&mut self, out: &mut [f32], sr: f32) {
        let len = out.len();
        let mut i = 0;
        while i < self.n {
            let b = &mut self.b[i];
            let start = (b.delay as usize).min(len);
            b.delay -= start as u32;
            let mut k = start;
            let mut dead = false;
            while k < len {
                let run = (b.sub as usize).min(len - k);
                let mut m = b.mode;
                for o in &mut out[k..k + run] {
                    *o += m.tick();
                }
                b.mode = m;
                k += run;
                b.sub -= run as u32;
                if b.sub == 0 {
                    b.sub = BUBBLE_UPDATE;
                    if b.mode.energy() < 1e-8 || b.freq > 0.47 * sr {
                        dead = true;
                        break;
                    }
                    b.freq += b.dfreq;
                    b.mode.tune(b.freq, b.damping, sr);
                }
            }
            if dead {
                self.n -= 1;
                self.b[i] = self.b[self.n];
                continue;
            }
            i += 1;
        }
    }
}

impl<const N: usize> Default for BubbleBank<N> {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------------------
// Noise grains (impacts, collisions, pops)
// ---------------------------------------------------------------------------------------

/// What one impact sounds like: a band-limited noise burst plus up to two struck modes.
#[derive(Clone, Copy, Debug)]
pub struct GrainSpec {
    /// Linear amplitude of the burst.
    pub amp: f32,
    /// Centre of the noise band, Hz, and its Q (≈0.7 broad .. 10 tonal).
    pub fc: f32,
    pub q: f32,
    /// Envelope attack and decay time constants, s.
    pub attack: f32,
    pub decay: f32,
    /// Structural modes: (frequency Hz, decay time s, amplitude). amp 0 = unused.
    pub modes: [(f32, f32, f32); 2],
    /// Start this many samples after the start of the next rendered block.
    pub delay: u32,
}

impl Default for GrainSpec {
    fn default() -> Self {
        Self { amp: 0.0, fc: 2000.0, q: 1.0, attack: 0.0002, decay: 0.002, modes: [(0.0, 0.0, 0.0); 2], delay: 0 }
    }
}

impl GrainSpec {
    /// Energy of this grain in "unit-noise" terms (amp² × effective length in samples),
    /// used for Campbell's theorem when many grains are rendered as a noise bed instead.
    pub fn energy(&self, sr: f32) -> f32 {
        let burst = self.amp * self.amp * 0.5 * self.decay * sr;
        let modes: f32 = self.modes.iter().map(|&(_, t, a)| a * a * 0.25 * t * sr).sum();
        burst + modes
    }
}

#[derive(Clone, Copy, Default)]
struct Grain {
    a1: f32,
    d1: f32,
    a2: f32,
    d2: f32,
    band: Svf,
    m: [Mode; 2],
    modes: u8,
    /// Samples of noise burst left, and of the whole grain.
    burst: u32,
    life: u32,
    delay: u32,
    /// Per-grain LCG noise state (cheaper than xorshift, and independent per grain).
    noise: u32,
}

/// Fixed pool of noise grains, rendered a block at a time.
pub struct GrainPool<const N: usize> {
    g: [Grain; N],
    n: usize,
    spawns: u32,
}

impl<const N: usize> GrainPool<N> {
    pub fn new() -> Self {
        Self { g: [Grain::default(); N], n: 0, spawns: 0x1234_5678 }
    }

    pub fn active(&self) -> usize {
        self.n
    }

    pub fn is_full(&self) -> bool {
        self.n >= N
    }

    pub fn spawn(&mut self, s: &GrainSpec, sr: f32) -> bool {
        self.spawns = self.spawns.wrapping_mul(0x9E37_79B9).wrapping_add(0x7F4A_7C15);
        if self.n >= N || !(s.amp.is_finite()) {
            return false;
        }
        let decay = s.decay.max(2.0 / sr);
        let attack = s.attack.clamp(0.5 / sr, decay * 0.5);
        let burst = (decay * sr * 4.0) as u32 + 1;
        let mut g = Grain {
            a1: s.amp,
            d1: (-1.0 / (decay * sr)).exp(),
            a2: s.amp,
            d2: (-1.0 / (attack * sr)).exp(),
            band: Svf::new(s.fc, s.q, sr),
            m: [Mode::default(); 2],
            modes: 0,
            burst,
            life: burst,
            delay: s.delay,
            noise: self.spawns,
        };
        for &(f, t, a) in s.modes.iter() {
            if a != 0.0 && f > 0.0 && t > 0.0 && f < 0.45 * sr {
                let k = g.modes as usize;
                g.m[k].tune(f, 1.0 / t, sr);
                g.m[k].strike(a);
                g.modes += 1;
                g.life = g.life.max((t * sr * 4.0) as u32 + 1);
            }
        }
        self.g[self.n] = g;
        self.n += 1;
        true
    }

    /// Adds every grain's next `out.len()` samples to `out`.
    pub fn render_add(&mut self, out: &mut [f32]) {
        let len = out.len();
        let mut i = 0;
        while i < self.n {
            let g = &mut self.g[i];
            let start = (g.delay as usize).min(len);
            g.delay -= start as u32;
            let avail = len - start;
            let n = (g.life as usize).min(avail);
            let nb = (g.burst as usize).min(n);
            // Noise burst.
            let (mut a1, mut a2, d1, d2, mut noise, mut band) = (g.a1, g.a2, g.d1, g.d2, g.noise, g.band);
            for o in &mut out[start..start + nb] {
                let env = a1 - a2;
                a1 *= d1;
                a2 *= d2;
                noise = noise.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let white = (noise as i32) as f32 * (1.0 / 2_147_483_648.0);
                let (_, bp, _) = band.process(white);
                *o += bp * env;
            }
            g.a1 = a1;
            g.a2 = a2;
            g.noise = noise;
            g.band = band;
            g.burst -= nb as u32;
            // Structural modes.
            for k in 0..g.modes as usize {
                let mut m = g.m[k];
                for o in &mut out[start..start + n] {
                    *o += m.tick();
                }
                g.m[k] = m;
            }
            g.life -= n as u32;
            if g.life == 0 {
                self.n -= 1;
                self.g[i] = self.g[self.n];
                continue;
            }
            i += 1;
        }
    }
}

impl<const N: usize> Default for GrainPool<N> {
    fn default() -> Self {
        Self::new()
    }
}

/// Band-limited noise with a ramped gain: the "bed" that stands in for events too many to
/// render one by one. By Campbell's theorem a Poisson stream of rate λ of grains with energy
/// E is, in the limit, Gaussian noise of power λ·E with the grains' average spectrum, so
/// the bed takes over exactly the energy of the events it replaces.
#[derive(Clone, Copy, Default)]
pub struct NoiseBand {
    pub svf: Svf,
    pub gain: Ramp,
}

impl NoiseBand {
    pub fn new(fc: f32, q: f32, sr: f32) -> Self {
        Self { svf: Svf::new(fc, q, sr), gain: Ramp::new(0.0) }
    }

    #[inline]
    pub fn tick(&mut self, white: f32) -> f32 {
        let g = self.gain.next();
        let (_, bp, _) = self.svf.process(white);
        bp * g
    }

    /// Adds the band of `white` to `out` (after `gain.to(..)` for this block), then settles.
    pub fn render_add(&mut self, white: &[f32], out: &mut [f32]) {
        if self.gain.cur == 0.0 && self.gain_target_zero(out.len()) {
            return;
        }
        let mut svf = self.svf;
        let mut g = self.gain;
        for (o, &w) in out.iter_mut().zip(white) {
            let (_, bp, _) = svf.process(w);
            *o += bp * g.next();
        }
        svf.flush();
        self.svf = svf;
        self.gain = g;
        self.gain.finish();
    }

    fn gain_target_zero(&self, _len: usize) -> bool {
        self.gain.target == 0.0
    }
}

/// Amplitude for a noise bed that carries `rate` grains/sample of `energy` each (see
/// [`GrainSpec::energy`]), assuming the bed uses the same band filter as the grains.
#[inline]
pub fn campbell_amp(rate_per_sample: f32, energy: f32) -> f32 {
    (rate_per_sample.max(0.0) * energy.max(0.0)).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minnaert_frequency_vs_radius() {
        // 1 mm bubble rings at ~3.3 kHz; f·r is constant.
        assert!((minnaert_hz(0.001) - 3260.0).abs() < 1.0);
        assert!((minnaert_hz(0.002) * 0.002 - minnaert_hz(0.0005) * 0.0005).abs() < 1e-4);
        // A rendered bubble's zero crossings match the Minnaert frequency.
        let sr = 48_000.0;
        let mut bank = BubbleBank::<4>::new();
        bank.spawn(0.002, 1.0, 0.0, 0, sr);
        let mut buf = vec![0.0f32; 4800];
        for chunk in buf.chunks_mut(64) {
            bank.render_add(chunk, sr);
        }
        let mut prev = 0.0;
        let mut ups = Vec::new();
        for (i, &v) in buf.iter().enumerate() {
            if prev <= 0.0 && v > 0.0 {
                ups.push(i);
            }
            prev = v;
        }
        // Ten periods between the 1st and 11th upward zero crossing.
        let f = 10.0 * sr / (ups[10] - ups[0]) as f32;
        assert!((f - 1630.0).abs() < 1630.0 * 0.02, "f = {f}");
    }

    #[test]
    fn events_follow_rate() {
        let mut rng = Rng::new(3);
        let mut ev = Events::new(&mut rng);
        let n: u32 = (0..480_000).map(|_| ev.tick(100.0 / 48_000.0, &mut rng)).sum();
        // 10 s at 100/s: 1000 ± 3σ (σ ≈ 32).
        assert!((900..1100).contains(&n), "n = {n}");
    }

    #[test]
    fn hour_window_wraps_midnight() {
        assert!(hour_window(23.0, 20.0, 5.0, 1.0) > 0.99);
        assert!(hour_window(2.0, 20.0, 5.0, 1.0) > 0.99);
        assert!(hour_window(12.0, 20.0, 5.0, 1.0) < 0.01);
        assert!((hour_window(20.0, 20.0, 5.0, 1.0) - 0.5).abs() < 0.01);
        assert!(hour_window(13.0, 9.0, 18.0, 2.0) > 0.99);
    }

    #[test]
    fn wind_mapping_round_trips() {
        for ms in [0.0f32, 1.0, 4.8, 10.0, 16.0] {
            assert!((wind_speed_ms(wind_slider(ms)) - ms).abs() < 1e-3);
        }
        assert!((wind_speed_ms(0.45) - 4.83).abs() < 0.05);
    }
}
