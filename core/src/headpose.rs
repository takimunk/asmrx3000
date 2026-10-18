//! Head pose for rendering: latency-compensating prediction, smoothing and head translation.
//!
//! AirPods deliver attitude at ~25 Hz, and the sound we render reaches the ear 100-250 ms
//! later (sensor fusion + Bluetooth uplink, the main queue, the render block, the output
//! buffer, the AAC encoder and the Bluetooth downlink). Using the last measured pose as is,
//! the scene lags every head turn by that delay and moves in 40 ms steps. So:
//!
//! * prediction: the pose is extrapolated to the time the block will be *heard*,
//!   q(t_ear) = q_meas * exp(w * h), with the gyroscope rate w (head frame) and the horizon
//!   h = sensor age + time since arrival + output latency (measured by the host). The
//!   extrapolated angle is soft-limited (linear to 25 deg, saturating at 45 deg) so a sudden stop does not
//!   overshoot far, and `prediction` scales h for A/B and tuning;
//! * smoothing: each block the rendered pose slerps towards the predicted one with a 12 ms time
//!   constant. Between sensor samples the prediction itself moves continuously; at a new sample
//!   the prediction error is absorbed over ~12 ms instead of jumping. The SH rotation is then
//!   interpolated per sample within the block (rotation.rs);
//! * head translation: the head turns about the neck, not about its centre, so the ears also
//!   translate (a few cm). Close sources then show real motion parallax, an externalisation cue
//!   that rotation-only head tracking lacks.
//!
//! Hosts without quaternion input keep using the yaw/pitch/roll params; those are smoothed the
//! same way (no 25 Hz steps).
//!
//! Real-time: the host writes a sample with `HeadPoseInput::set` from any thread (seqlock over
//! atomics); the audio thread reads it once per block.

use std::sync::atomic::{fence, AtomicU32, AtomicU64, Ordering};

use crate::math::{Mat3, Vec3};

/// Longest horizon the predictor extrapolates over (s).
pub const MAX_HORIZON: f32 = 0.30;
/// Extrapolated rotation: linear up to KNEE, then saturating smoothly at MAX (rad).
const KNEE_ANGLE: f32 = 25.0 * core::f32::consts::PI / 180.0;
const MAX_PREDICTED_ANGLE: f32 = 45.0 * core::f32::consts::PI / 180.0;
/// Smoothing time constant (s).
const SMOOTHING: f32 = 0.012;
/// Head centre relative to the neck pivot (engine frame, metres): the head rotates about the
/// upper cervical spine, which lies ~3 cm behind and ~9 cm below the interaural midpoint.
pub const NECK_TO_HEAD: Vec3 = Vec3::new(0.03, 0.0, 0.09);

/// Unit quaternion (w, x, y, z), rotating head-frame vectors into the world frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quat {
    pub w: f32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Quat {
    pub const IDENTITY: Quat = Quat { w: 1.0, x: 0.0, y: 0.0, z: 0.0 };

    pub fn normalized(self) -> Quat {
        let n = (self.w * self.w + self.x * self.x + self.y * self.y + self.z * self.z).sqrt();
        if !(n > 1e-6) || !n.is_finite() {
            return Quat::IDENTITY;
        }
        Quat { w: self.w / n, x: self.x / n, y: self.y / n, z: self.z / n }
    }

    pub fn mul(self, o: Quat) -> Quat {
        Quat {
            w: self.w * o.w - self.x * o.x - self.y * o.y - self.z * o.z,
            x: self.w * o.x + self.x * o.w + self.y * o.z - self.z * o.y,
            y: self.w * o.y - self.x * o.z + self.y * o.w + self.z * o.x,
            z: self.w * o.z + self.x * o.y - self.y * o.x + self.z * o.w,
        }
    }

    /// Rotation by angle |v| about v (rotation vector, rad).
    pub fn from_rotation_vector(v: Vec3) -> Quat {
        let a = v.length();
        if a < 1e-9 {
            return Quat { w: 1.0, x: 0.5 * v.x, y: 0.5 * v.y, z: 0.5 * v.z }.normalized();
        }
        let (s, c) = (0.5 * a).sin_cos();
        let k = s / a;
        Quat { w: c, x: v.x * k, y: v.y * k, z: v.z * k }
    }

    pub fn dot(self, o: Quat) -> f32 {
        self.w * o.w + self.x * o.x + self.y * o.y + self.z * o.z
    }

    /// Shortest-path spherical interpolation.
    pub fn slerp(self, o: Quat, t: f32) -> Quat {
        let mut d = self.dot(o);
        let o = if d < 0.0 {
            d = -d;
            Quat { w: -o.w, x: -o.x, y: -o.y, z: -o.z }
        } else {
            o
        };
        if d > 0.9995 {
            return Quat {
                w: self.w + (o.w - self.w) * t,
                x: self.x + (o.x - self.x) * t,
                y: self.y + (o.y - self.y) * t,
                z: self.z + (o.z - self.z) * t,
            }
            .normalized();
        }
        let th = d.clamp(-1.0, 1.0).acos();
        let s = th.sin();
        let (a, b) = (((1.0 - t) * th).sin() / s, (t * th).sin() / s);
        Quat { w: a * self.w + b * o.w, x: a * self.x + b * o.x, y: a * self.y + b * o.y, z: a * self.z + b * o.z }
    }

    /// Angle (rad) between two orientations.
    #[allow(dead_code)] // tests and diagnostics
    pub fn angle_to(self, o: Quat) -> f32 {
        2.0 * self.dot(o).abs().min(1.0).acos()
    }

    pub fn to_mat3(self) -> Mat3 {
        let Quat { w, x, y, z } = self;
        Mat3 {
            m: [
                [1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y - w * z), 2.0 * (x * z + w * y)],
                [2.0 * (x * y + w * z), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z - w * x)],
                [2.0 * (x * z - w * y), 2.0 * (y * z + w * x), 1.0 - 2.0 * (x * x + y * y)],
            ],
        }
    }

    /// Same convention as `Mat3::from_head`: Rz(yaw) Ry(-pitch) Rx(roll).
    pub fn from_yaw_pitch_roll(yaw: f32, pitch: f32, roll: f32) -> Quat {
        let qz = Quat::from_rotation_vector(Vec3::new(0.0, 0.0, yaw));
        let qy = Quat::from_rotation_vector(Vec3::new(0.0, -pitch, 0.0));
        let qx = Quat::from_rotation_vector(Vec3::new(roll, 0.0, 0.0));
        qz.mul(qy).mul(qx)
    }

    /// Inverse of `from_yaw_pitch_roll`.
    #[allow(dead_code)] // tests and diagnostics
    pub fn to_yaw_pitch_roll(self) -> (f32, f32, f32) {
        let m = self.to_mat3().m;
        let yaw = m[1][0].atan2(m[0][0]);
        let pitch = m[2][0].clamp(-1.0, 1.0).asin();
        let roll = m[2][1].atan2(m[2][2]);
        (yaw, pitch, roll)
    }
}

/// Host -> audio thread handoff of the latest sensor sample (seqlock).
pub struct HeadPoseInput {
    seq: AtomicU32,
    data: [AtomicU32; 8],
    /// Arrival time, seconds since the engine was created (f64 bits).
    arrived: AtomicU64,
}

#[derive(Clone, Copy, Debug)]
pub struct Sample {
    pub q: Quat,
    /// Angular velocity, head frame, rad/s.
    pub w: Vec3,
    /// How old the measurement already was when it was handed over (s).
    pub age: f32,
    pub arrived: f64,
    pub seq: u32,
}

impl HeadPoseInput {
    pub fn new() -> Self {
        Self { seq: AtomicU32::new(0), data: Default::default(), arrived: AtomicU64::new(0) }
    }

    /// Any thread. `now` = seconds since engine creation.
    pub fn set(&self, q: Quat, w: Vec3, age: f32, now: f64) {
        let q = q.normalized();
        // Single writer (the host's motion queue). Odd sequence = write in progress.
        let s = self.seq.load(Ordering::Relaxed) & !1;
        self.seq.store(s.wrapping_add(1), Ordering::Relaxed);
        fence(Ordering::Release);
        for (slot, v) in self.data.iter().zip([q.w, q.x, q.y, q.z, w.x, w.y, w.z, age]) {
            slot.store(v.to_bits(), Ordering::Relaxed);
        }
        self.arrived.store(now.to_bits(), Ordering::Relaxed);
        let next = s.wrapping_add(2);
        self.seq.store(if next == 0 { 2 } else { next }, Ordering::Release);
    }

    /// Latest complete sample, or None if none was ever set (or a write is in progress).
    pub fn get(&self) -> Option<Sample> {
        for _ in 0..4 {
            let s0 = self.seq.load(Ordering::Acquire);
            if s0 == 0 {
                return None;
            }
            if s0 & 1 == 1 {
                continue;
            }
            let v: [f32; 8] = core::array::from_fn(|i| f32::from_bits(self.data[i].load(Ordering::Relaxed)));
            let arrived = f64::from_bits(self.arrived.load(Ordering::Relaxed));
            fence(Ordering::Acquire);
            if self.seq.load(Ordering::Relaxed) == s0 {
                return Some(Sample {
                    q: Quat { w: v[0], x: v[1], y: v[2], z: v[3] },
                    w: Vec3::new(v[4], v[5], v[6]),
                    age: v[7],
                    arrived,
                    seq: s0,
                });
            }
        }
        None
    }
}

/// Settings read each block (from engine params).
#[derive(Clone, Copy, Debug)]
pub struct PoseSettings {
    /// Render-to-ear delay (s): output buffering, device and Bluetooth latency.
    pub output_latency: f32,
    /// 0 = no prediction, 1 = full latency compensation.
    pub prediction: f32,
    /// Head translation about the neck pivot (motion parallax) on/off.
    pub parallax: bool,
}

/// Audio-thread state.
pub struct HeadTracker {
    out: Quat,
    started: bool,
    last_seq: u32,
    /// Horizon used in the last block (s), for telemetry.
    pub horizon: f32,
    /// Sensor samples per second (smoothed), for telemetry.
    pub rate: f32,
    last_arrival: f64,
}

/// The pose to render a block with.
#[derive(Clone, Copy, Debug)]
pub struct RenderPose {
    /// Head -> world rotation.
    pub rotation: Mat3,
    #[allow(dead_code)] // tests and diagnostics
    pub quat: Quat,
    /// Ear-centre offset from the neck-pivot rest position (world frame, m).
    pub offset: Vec3,
}

impl HeadTracker {
    pub fn new() -> Self {
        Self { out: Quat::IDENTITY, started: false, last_seq: 0, horizon: 0.0, rate: 0.0, last_arrival: 0.0 }
    }

    /// Predicted orientation at `now + output_latency` for a sensor sample.
    pub fn predict(sample: &Sample, now: f64, settings: &PoseSettings) -> (Quat, f32) {
        let since = (now - sample.arrived).max(0.0) as f32;
        // The smoothing below lags a steady turn by its time constant; predict that far too.
        let h = ((sample.age + since + settings.output_latency + SMOOTHING) * settings.prediction.clamp(0.0, 1.5)).clamp(0.0, MAX_HORIZON);
        let rv = sample.w * h;
        let a = rv.length();
        let rv = if a > KNEE_ANGLE {
            let span = MAX_PREDICTED_ANGLE - KNEE_ANGLE;
            rv * ((KNEE_ANGLE + span * ((a - KNEE_ANGLE) / span).tanh()) / a)
        } else {
            rv
        };
        (sample.q.normalized().mul(Quat::from_rotation_vector(rv)).normalized(), h)
    }

    /// One block. `fallback`: yaw/pitch/roll params, used until a quaternion sample arrives.
    pub fn step(&mut self, input: &HeadPoseInput, fallback: (f32, f32, f32), now: f64, dt: f32, settings: &PoseSettings) -> RenderPose {
        let target = match input.get() {
            Some(s) => {
                if s.seq != self.last_seq {
                    if self.last_seq != 0 {
                        let gap = (s.arrived - self.last_arrival).max(1e-3) as f32;
                        self.rate += (1.0 / gap - self.rate) * 0.1;
                    }
                    self.last_seq = s.seq;
                    self.last_arrival = s.arrived;
                }
                let (q, h) = Self::predict(&s, now, settings);
                self.horizon = h;
                q
            }
            None => {
                self.horizon = 0.0;
                Quat::from_yaw_pitch_roll(fallback.0, fallback.1, fallback.2)
            }
        };
        if !self.started {
            self.out = target;
            self.started = true;
        } else {
            let a = 1.0 - (-dt / SMOOTHING).exp();
            self.out = self.out.slerp(target, a);
        }
        let rotation = self.out.to_mat3();
        let offset = if settings.parallax { rotation.apply(NECK_TO_HEAD) - NECK_TO_HEAD } else { Vec3::default() };
        RenderPose { rotation, quat: self.out, offset }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::f32::consts::PI;

    fn close(a: Mat3, b: Mat3) -> bool {
        (0..3).all(|i| (0..3).all(|j| (a.m[i][j] - b.m[i][j]).abs() < 1e-5))
    }

    #[test]
    fn quaternion_matches_engine_euler_convention() {
        for (y, p, r) in [(0.7, 0.0, 0.0), (0.0, 0.4, 0.0), (0.0, 0.0, -0.3), (1.2, -0.5, 0.25)] {
            let q = Quat::from_yaw_pitch_roll(y, p, r);
            assert!(close(q.to_mat3(), Mat3::from_head(y, p, r)), "{y} {p} {r}");
            let (y2, p2, r2) = q.to_yaw_pitch_roll();
            assert!((y - y2).abs() < 1e-5 && (p - p2).abs() < 1e-5 && (r - r2).abs() < 1e-5);
        }
    }

    #[test]
    fn seqlock_roundtrip() {
        let inp = HeadPoseInput::new();
        assert!(inp.get().is_none());
        let q = Quat::from_yaw_pitch_roll(0.3, 0.1, 0.0);
        inp.set(q, Vec3::new(0.0, 0.0, 1.0), 0.02, 5.0);
        let s = inp.get().unwrap();
        assert!(s.q.angle_to(q) < 1e-5 && s.w.z == 1.0 && s.age == 0.02 && s.arrived == 5.0);
        inp.set(q, Vec3::new(0.0, 0.0, 2.0), 0.02, 6.0);
        let s2 = inp.get().unwrap();
        assert!(s2.seq != s.seq && s2.w.z == 2.0);
    }

    /// A head turning at constant rate, sampled at 25 Hz and heard 200 ms later: with prediction
    /// the rendered yaw tracks the true yaw at hearing time; without it, it lags by ~13 deg.
    #[test]
    fn prediction_cancels_latency_for_steady_turns() {
        let rate = 1.2f32; // rad/s (~70 deg/s)
        let latency = 0.18f32; // output latency
        let sensor_age = 0.02f32;
        let dt = 64.0 / 48_000.0;
        let run = |prediction: f32| -> f32 {
            let inp = HeadPoseInput::new();
            let mut tr = HeadTracker::new();
            let settings = PoseSettings { output_latency: latency, prediction, parallax: false };
            let mut err = 0.0f32;
            let mut n = 0;
            let mut next_sample = 0.0f64;
            let mut t = 0.0f64;
            while t < 3.0 {
                if t >= next_sample {
                    // Sample measured at next_sample - age, handed over at next_sample.
                    let yaw = rate * (next_sample as f32 - sensor_age);
                    inp.set(Quat::from_yaw_pitch_roll(yaw, 0.0, 0.0), Vec3::new(0.0, 0.0, rate), sensor_age, next_sample);
                    next_sample += 0.04;
                }
                let pose = tr.step(&inp, (0.0, 0.0, 0.0), t, dt, &settings);
                if t > 1.0 {
                    let truth = rate * (t as f32 + latency);
                    let (y, _, _) = pose.quat.to_yaw_pitch_roll();
                    let mut e = y - truth;
                    while e > PI {
                        e -= 2.0 * PI;
                    }
                    while e < -PI {
                        e += 2.0 * PI;
                    }
                    err += e * e;
                    n += 1;
                }
                t += dt as f64;
            }
            (err / n as f32).sqrt().to_degrees()
        };
        let without = run(0.0);
        let with = run(1.0);
        assert!(without > 10.0, "rms error without prediction {without:.2} deg");
        assert!(with < 1.0, "rms error with prediction {with:.2} deg");
    }

    /// Between 25 Hz samples the rendered pose moves smoothly (no 40 ms steps).
    #[test]
    fn rendered_pose_is_smooth() {
        let inp = HeadPoseInput::new();
        let mut tr = HeadTracker::new();
        let settings = PoseSettings { output_latency: 0.0, prediction: 0.0, parallax: false };
        let dt = 64.0 / 48_000.0f32;
        let mut prev = 0.0f32;
        let mut max_step = 0.0f32;
        let mut next = 0.0f64;
        let mut t = 0.0f64;
        while t < 2.0 {
            if t >= next {
                let yaw = 1.0 * next as f32;
                inp.set(Quat::from_yaw_pitch_roll(yaw, 0.0, 0.0), Vec3::default(), 0.0, next);
                next += 0.04;
            }
            let (y, _, _) = tr.step(&inp, (0.0, 0.0, 0.0), t, dt, &settings).quat.to_yaw_pitch_roll();
            if t > 0.2 {
                max_step = max_step.max((y - prev).abs());
            }
            prev = y;
            t += dt as f64;
        }
        // A raw 25 Hz staircase at 1 rad/s steps by 0.04 rad; smoothed blocks move far less.
        assert!(max_step < 0.006, "largest per-block step {max_step} rad");
    }

    #[test]
    fn neck_pivot_translates_the_ears() {
        let inp = HeadPoseInput::new();
        let mut tr = HeadTracker::new();
        let s = PoseSettings { output_latency: 0.0, prediction: 0.0, parallax: true };
        let p = tr.step(&inp, (PI / 2.0, 0.0, 0.0), 0.0, 0.001, &s);
        // Turning 90 deg left moves the head centre 3 cm back and 3 cm left.
        assert!((p.offset.x + 0.03).abs() < 1e-4 && (p.offset.y - 0.03).abs() < 1e-4, "{:?}", p.offset);
    }
}
