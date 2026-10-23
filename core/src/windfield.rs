//! Global wind field: one mean wind with a world-fixed direction, and gusts that travel along
//! it.
//!
//! **Direction.** `set_direction(from_deg)` takes the meteorological convention: the compass
//! direction the wind blows *from*, clockwise from north (270 = westerly, the default). The
//! scene and the flight world share one frame: x = north (the scene's forward), y = west (the
//! scene's left), z = up. So the default westerly blows from the listener's left (+y) to the
//! right (−y), as the scene always had. A change of direction turns the vector smoothly
//! (time constant ~4 s), so the gust history stays continuous.
//!
//! **Gusts.** Measured near-surface wind is intermittent, not Gaussian: gusts rise within a
//! second or two and die away over several, separated by lulls; the speed distribution is
//! skewed towards the gusts (gust factor 1.4–1.6, turbulence intensity 0.2–0.4 over
//! vegetation; Wieringa 1973, Kaimal & Finnigan 1994 ch. 2). The gust strength here is
//! shot noise of asymmetric pulses (rise τ_r ≈ 0.25·τ_d, decay τ_d ≈ 18 m / U, so a gust
//! lasts as long as an eddy of ~20 m takes to pass; rate ∝ U), minus its mean, plus a slow
//! Ornstein–Uhlenbeck drift (τ 15 s) for the mesoscale wandering. Between gusts the field
//! falls below the mean: the lulls.
//!
//! **Travel.** Turbulence is carried with the mean flow (Taylor's frozen-turbulence
//! hypothesis), so a gust reaches each point later the farther downwind it is: the strength
//! at position p is the history value delayed by (S + p·ŵ)/v_front, ŵ the unit vector the
//! wind blows towards, S = `UPWIND_SPAN` the upwind reach of the history. Upwind trees roar
//! first, the gust sweeps across the listener, downwind trees answer.

use crate::engine::BLOCK;
use crate::rng::Rng;

const HISTORY: usize = 32_768; // ~44 s at 750 Hz control rate
/// Upwind distance (m) beyond which every point sees the same (newest) gust.
pub const UPWIND_SPAN: f32 = 80.0;
const DRIFT_TIMESCALE: f32 = 15.0; // s
const DIRECTION_TIMESCALE: f32 = 4.0; // s

pub struct WindField {
    history: Vec<f32>,
    pos: usize,
    drift: f32,
    /// Gust pulses: `kick` jumps at each gust and decays with τ_d; `pulse` follows it with
    /// τ_r (fast rise, slow decay).
    kick: f32,
    pulse: f32,
    gust: f32,
    front_speed: f32,
    control_rate: f32,
    /// Unit vector the wind blows towards (x north, y west), smoothed, and its target.
    to: (f32, f32),
    to_target: (f32, f32),
    from_deg: f32,
    started: bool,
}

/// Unit vector (x north, y west) the wind blows *towards*, for a wind *from* `from_deg`.
pub fn blow_vector(from_deg: f32) -> (f32, f32) {
    let t = from_deg.to_radians();
    (-t.cos(), t.sin())
}

impl WindField {
    pub fn new(sample_rate: f32) -> Self {
        let to = blow_vector(270.0);
        Self {
            history: vec![0.0; HISTORY],
            pos: 0,
            drift: 0.0,
            kick: 0.0,
            pulse: 0.0,
            gust: 0.0,
            front_speed: 4.0,
            control_rate: sample_rate / BLOCK as f32,
            to,
            to_target: to,
            from_deg: 270.0,
            started: false,
        }
    }

    /// Where the wind blows from, degrees clockwise from north (meteorological).
    pub fn set_direction(&mut self, from_deg: f32) {
        if from_deg.is_finite() && from_deg != self.from_deg {
            self.from_deg = from_deg;
            self.to_target = blow_vector(from_deg);
        }
    }

    /// Like `set_direction`, without the glide (tests, scene start).
    pub fn snap_direction(&mut self, from_deg: f32) {
        self.set_direction(from_deg);
        self.to = self.to_target;
    }

    /// Unit vector the wind blows towards, world frame (x north, y west).
    pub fn blow_to(&self) -> (f32, f32) {
        self.to
    }

    /// Advance by one control block. `wind` is the mean wind in 0..1.
    pub fn step(&mut self, wind: f32, dt: f32, rng: &mut Rng) {
        // Direction glide: rotate the unit vector towards its target.
        let a = (dt / DIRECTION_TIMESCALE).min(1.0);
        let (mut x, mut y) = (self.to.0 + (self.to_target.0 - self.to.0) * a, self.to.1 + (self.to_target.1 - self.to.1) * a);
        let n = (x * x + y * y).sqrt();
        if n < 1e-3 {
            (x, y) = self.to_target; // turned right around: take the target at once
        } else {
            (x, y) = (x / n, y / n);
        }
        self.to = (x, y);

        let u_ms = crate::sources::common::wind_speed_ms(wind).max(0.5);
        let scale = 0.35 + 0.3 * wind;
        // Gust pulses: rate ∝ U, decay ∝ 1/U, so the mean and variance of the pulse train do
        // not depend on the speed (shot noise: mean = λ·E[A]·τ_d, var ≈ λ·E[A²]·τ_d / 2).
        let tau_d = (18.0 / u_ms).clamp(1.5, 8.0);
        let tau_r = 0.25 * tau_d;
        let rate = u_ms / 120.0;
        const AMP: f32 = 1.0;
        if rng.uniform() < rate * dt {
            // Exponentially distributed gust strength.
            self.kick += AMP * -(rng.uniform().max(1e-6)).ln();
        }
        self.kick -= self.kick * (dt / tau_d).min(1.0);
        self.pulse += (self.kick - self.pulse) * (dt / tau_r).min(1.0);
        let mean = rate * AMP * tau_d;
        if !self.started {
            // Start in the statistical steady state, the whole history at the mean.
            self.started = true;
            self.kick = mean;
            self.pulse = mean;
            self.history.fill(wind);
        }
        self.drift += -self.drift / DRIFT_TIMESCALE * dt + 0.12 * (2.0 * dt / DRIFT_TIMESCALE).sqrt() * rng.gauss();
        let deviation = scale * (self.pulse - mean) + self.drift;
        self.gust = (wind * (1.0 + deviation)).max(0.0);
        self.front_speed = 3.0 + 6.0 * wind; // m/s
        self.pos = (self.pos + 1) % HISTORY;
        self.history[self.pos] = self.gust;
    }

    pub fn gust(&self) -> f32 {
        self.gust
    }

    /// An extra gust pulse of `strength` (mean gusts are 1): it travels with the wind like
    /// any other (the scene director's "wave through the crowns").
    pub fn kick(&mut self, strength: f32) {
        self.kick += strength.max(0.0);
    }

    /// Gust strength at world offset (`x` north, `y` west, metres) from the listener.
    pub fn at(&self, x: f32, y: f32) -> f32 {
        let along = x * self.to.0 + y * self.to.1; // downwind distance
        let lag_s = (UPWIND_SPAN + along).max(0.0) / self.front_speed;
        let lag = ((lag_s * self.control_rate) as usize).min(HISTORY - 1);
        self.history[(self.pos + HISTORY - lag) % HISTORY]
    }

    /// Gust strength at a point given in a frame turned by `heading` (rad, counter-clockwise
    /// from north: flight's listener frame, x forward, y left). The wind stays world-fixed
    /// while the listener turns.
    pub fn at_heading(&self, x: f32, y: f32, heading: f32) -> f32 {
        let (s, c) = heading.sin_cos();
        self.at(c * x - s * y, s * x + c * y)
    }

    /// The wind's blow-to vector in a frame turned by `heading` (see `at_heading`).
    pub fn blow_to_heading(&self, heading: f32) -> (f32, f32) {
        let (s, c) = heading.sin_cos();
        let (x, y) = self.to;
        (c * x + s * y, -s * x + c * y)
    }

    /// Wind speed at the listener now relative to the mean (m/s ratio), for `mean` in slider
    /// units.
    pub fn speed_ratio(&self, mean: f32) -> f32 {
        use crate::sources::common::wind_speed_ms;
        let m = wind_speed_ms(mean);
        if m < 1e-3 { 1.0 } else { (wind_speed_ms(self.at(0.0, 0.0)) / m).clamp(0.0, 3.0) }
    }

    /// Gust strength at lateral scene position `y` (metres, +left) on the listener's line.
    pub fn local(&self, y: f32) -> f32 {
        self.at(0.0, y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(field: &mut WindField, secs: f32) {
        let mut rng = Rng::new(7);
        let dt = BLOCK as f32 / 48_000.0;
        for _ in 0..(secs / dt) as usize {
            field.step(0.6, dt, &mut rng);
        }
    }

    /// Delay (s) of the cross-correlation peak of `b` behind `a`.
    fn arrival_lag(field: &mut WindField, a: (f32, f32), b: (f32, f32)) -> f32 {
        let mut rng = Rng::new(11);
        let dt = BLOCK as f32 / 48_000.0;
        let n = (60.0 / dt) as usize;
        let (mut sa, mut sb) = (Vec::with_capacity(n), Vec::with_capacity(n));
        for _ in 0..n {
            field.step(0.6, dt, &mut rng);
            sa.push(field.at(a.0, a.1));
            sb.push(field.at(b.0, b.1));
        }
        let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len() as f32;
        let (ma, mb) = (mean(&sa), mean(&sb));
        let (mut best, mut best_k) = (f32::MIN, 0i64);
        for k in -3000i64..=3000 {
            let mut acc = 0.0;
            for i in 3000..n - 3000 {
                acc += (sa[i] - ma) * (sb[(i as i64 + k) as usize] - mb);
            }
            if acc > best {
                best = acc;
                best_k = k;
            }
        }
        best_k as f32 * dt
    }

    #[test]
    fn gusts_arrive_from_upwind() {
        // Westerly: the west tree (+y) hears the gust first.
        let mut f = WindField::new(48_000.0);
        let lag = arrival_lag(&mut f, (0.0, 15.0), (0.0, -15.0));
        assert!(lag > 2.0, "west -> east lag {lag}");
        // Northerly: the north tree (+x) first, east-west pair simultaneous.
        let mut f = WindField::new(48_000.0);
        f.snap_direction(0.0);
        let lag = arrival_lag(&mut f, (15.0, 0.0), (-15.0, 0.0));
        assert!(lag > 2.0, "north -> south lag {lag}");
        let mut f = WindField::new(48_000.0);
        f.snap_direction(0.0);
        let lag = arrival_lag(&mut f, (0.0, 15.0), (0.0, -15.0));
        assert!(lag.abs() < 0.1, "crosswind pair lag {lag}");
        // Easterly reverses the westerly order.
        let mut f = WindField::new(48_000.0);
        f.snap_direction(90.0);
        let lag = arrival_lag(&mut f, (0.0, -15.0), (0.0, 15.0));
        assert!(lag > 2.0, "east -> west lag {lag}");
    }

    #[test]
    fn direction_is_world_fixed_under_heading() {
        let mut f = WindField::new(48_000.0);
        f.snap_direction(0.0); // from the north
        run(&mut f, 30.0);
        // Facing east (heading −90°), north is on the listener's left (+y).
        let h = -core::f32::consts::FRAC_PI_2;
        assert_eq!(f.at_heading(0.0, 25.0, h), f.at(25.0, 0.0));
        let (x, y) = f.blow_to_heading(h);
        assert!(x.abs() < 1e-5 && (y + 1.0).abs() < 1e-5, "blows to the right: ({x}, {y})");
        // Facing north, the wind blows into the listener's face.
        let (x, y) = f.blow_to_heading(0.0);
        assert!((x + 1.0).abs() < 1e-5 && y.abs() < 1e-5);
    }

    #[test]
    fn gusts_are_intermittent() {
        // Skewed towards the gusts: rare strong peaks, longer lulls below the mean.
        let mut f = WindField::new(48_000.0);
        let mut rng = Rng::new(3);
        let dt = BLOCK as f32 / 48_000.0;
        let mut v = Vec::new();
        for _ in 0..(600.0 / dt) as usize {
            f.step(0.6, dt, &mut rng);
            v.push(f.gust());
        }
        let n = v.len() as f32;
        let m = v.iter().sum::<f32>() / n;
        let sd = (v.iter().map(|x| (x - m).powi(2)).sum::<f32>() / n).sqrt();
        let skew = v.iter().map(|x| ((x - m) / sd).powi(3)).sum::<f32>() / n;
        let below = v.iter().filter(|x| **x < m).count() as f32 / n;
        eprintln!("gust field: mean {m:.3} (wind 0.6), sd/mean {:.2}, skewness {skew:.2}, time below mean {below:.2}", sd / m);
        assert!((m - 0.6).abs() < 0.08);
        assert!(skew > 0.3);
        assert!(below > 0.55);
        assert!(sd / m > 0.2 && sd / m < 0.6);
    }
}
