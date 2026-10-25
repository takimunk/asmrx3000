//! Per-emitter outdoor sound propagation: one `Propagator` per emitter, one call per block.
//!
//!   mono signal + emitter/listener geometry + environment  ->  contribution to the Ambisonic
//!   bus (world frame), plus sends to the forest reverb.
//!
//! Paths and effects (all derived from physics, see docs/spatial-engine.md):
//!  * direct path: retarded-time delay r/c through a band-limited fractional delay line, so
//!    Doppler of moving sources is exact (including the source/listener asymmetry);
//!    spherical spreading 1/r; ISO 9613-1 air absorption; source directivity; Fresnel-Kirchhoff
//!    diffraction by obstacles (extra detour delay in the shadow); statistical excess
//!    attenuation by trunk scattering and canopy foliage; near-field (NFC-HOA range
//!    extrapolation) below the HRTF measurement radius;
//!  * ground-reflected path: image source with its own retarded delay, direction and the
//!    spherical-wave reflection coefficient Q of the local ground impedance -- the
//!    interference with the direct path produces the physical ground-effect dip;
//!  * early reflections from the nearest trunks (sends to `reverb`), late-tail send;
//!  * wind (physics/wind.rs): refraction by the wind gradient — an upwind shadow zone that
//!    darkens and quietens far sources, a downwind gain — as part of the path filter, and
//!    turbulent level wander of far paths as a slow gain on both paths.
//! All spectral effects of a path are fused into one fixed-pole parallel filter refitted at
//! control rate (only when geometry changed) and interpolated per sample; delays, gains and
//! SH gains are interpolated per sample, so fast motion produces no zipper noise.

use crate::acoustics::{Context, Emitter, Scratch};
use crate::ambisonics::{self, order_of, CHANNELS, ORDER};
use crate::delay::{DelayLine, MIN_DELAY, TAPS};
use crate::engine::BLOCK;
use crate::math::Vec3;
use crate::nearfield::NearField;
use crate::num::C64;
use crate::physics::{diffraction, directivity::{self, Directivity}, ground, wind};
use crate::rng::Rng;
use crate::reverb::{Sends, ER_TRUNKS};
use crate::shaping::{CoeffRamp, FilterBank, NF};
use crate::simd::F4;

/// Closest distance used for 1/r spreading (m).
pub const R_MIN: f32 = 0.15;
/// Longest propagation delay (s) a propagator supports.
pub const MAX_DELAY_S: f32 = 0.7;
const BARK_REFLECTION: f32 = 0.975; // sqrt(1 - alpha_bark), alpha_bark = 0.05
/// Early-reflection taps rendered per emitter (the strongest of the ER_TRUNKS candidates).
const ER_TAPS: usize = 4;
/// Silence after the last sound has left the delay line before an emitter sleeps: the path
/// filters' slowest pole (30 Hz, tau ~13 ms) decays ~80 dB in it.
const SLEEP_TAIL_S: f32 = 0.15;

struct Path {
    coeffs: CoeffRamp,
    /// Delay at the end of the previous block (samples).
    delay: f64,
    sh: [f32; CHANNELS],
}

impl Path {
    fn new() -> Self {
        Self { coeffs: CoeffRamp::new(), delay: 0.0, sh: [0.0; CHANNELS] }
    }
}

#[derive(Clone, Copy, Default)]
struct ErTap {
    delay: f64,
    gain: f32,
}

#[derive(Clone, Copy)]
struct SpecKey {
    pos: Vec3,
    forward: Vec3,
    r1: f32,
    r2: f32,
    sin_psi: f32,
    /// Whether any obstacle affected the path at the last refit.
    obstructed: bool,
    epoch: u32,
    valid: bool,
    /// Mix EQ (dB) the filters were fitted with.
    eq: [f32; NF],
    /// Wind shadow penetration and downwind weight the filters were fitted with.
    shadow: f32,
    favour: f32,
}

pub struct Propagator {
    delay: DelayLine,
    direct: Path,
    ground: Path,
    near: NearField,
    er: [ErTap; ER_TRUNKS],
    send: f32,
    extra_m: f64,
    last_pos: Vec3,
    vel: Vec3,
    started: bool,
    idle: usize,
    sleeping: bool,
    key: SpecKey,
    /// Set by `stage`, consumed by `finish`: (block length, r1, direct and ground SH targets).
    pending: Option<(usize, f32, [f32; CHANNELS], [f32; CHANNELS])>,
    /// Turbulent level wander: unit-variance OU state and its own random stream.
    turb: f32,
    rng: Rng,
    /// Wind state for the next refit (shadow, favour).
    wind_sf: (f32, f32),
}

/// Retarded propagation time: solves |d - v tau| = c tau for a source moving with constant
/// velocity v (relative position d = source - listener at reception time).
pub fn retarded_time(d: Vec3, v: Vec3, c: f32) -> f32 {
    let (dx, dy, dz) = (d.x as f64, d.y as f64, d.z as f64);
    let (mut vx, mut vy, mut vz) = (v.x as f64, v.y as f64, v.z as f64);
    let c = c as f64;
    let speed = (vx * vx + vy * vy + vz * vz).sqrt();
    if speed > 0.5 * c {
        let s = 0.5 * c / speed;
        vx *= s;
        vy *= s;
        vz *= s;
    }
    let dv = dx * vx + dy * vy + dz * vz;
    let dd = dx * dx + dy * dy + dz * dz;
    let a = c * c - (vx * vx + vy * vy + vz * vz);
    ((-dv + (dv * dv + a * dd).sqrt()) / a) as f32
}

impl Propagator {
    /// `seed` decorrelates the turbulent wander of different emitters (deterministic).
    pub fn new(ctx: &Context, seed: u64) -> Self {
        let max = (MAX_DELAY_S * ctx.sample_rate) as usize;
        Self {
            delay: DelayLine::new(max),
            direct: Path::new(),
            ground: Path::new(),
            near: NearField::new(),
            er: [ErTap::default(); ER_TRUNKS],
            send: 0.0,
            extra_m: 0.0,
            last_pos: Vec3::default(),
            vel: Vec3::default(),
            started: false,
            idle: 0,
            sleeping: false,
            key: SpecKey { pos: Vec3::default(), forward: Vec3::default(), r1: 0.0, r2: 0.0, sin_psi: 0.0, obstructed: false, epoch: 0, valid: false, eq: [0.0; NF], shadow: 0.0, favour: 0.0 },
            pending: None,
            turb: 0.0,
            rng: Rng::new(0x9e37_79b9_7f4a_7c15 ^ seed.wrapping_mul(0x2545_f491_4f6c_dd1d)),
            wind_sf: (0.0, 0.0),
        }
    }

    fn sleep(&mut self, bank: &mut FilterBank, path: usize) {
        bank.reset(path);
        bank.reset(path + 1);
        // Asleep, the bank skips this path's filter (no ramp advance), so land on the target.
        self.direct.coeffs.settle();
        self.ground.coeffs.settle();
        self.near.reset();
        self.delay.clear();
        self.sleeping = true;
        self.started = false;
        self.key.valid = false;
    }

    /// First half of a block (`input.len()` <= BLOCK): delays, Doppler, spectral refits,
    /// reverb and early-reflection sends; the direct and ground signals go into `bank` at
    /// paths `path` and `path + 1`, to be filtered in batch. `phase` staggers refits across
    /// emitters. `finish` then adds the filtered paths to the bus.
    #[allow(clippy::too_many_arguments)]
    pub fn stage(
        &mut self,
        ctx: &Context,
        em: &Emitter,
        input: &[f32],
        sends: &mut Sends,
        scratch: &mut Scratch,
        phase: u64,
        bank: &mut FilterBank,
        path: usize,
    ) {
        self.pending = None;
        let len = input.len();
        if len == 0 {
            return;
        }
        let fs = ctx.sample_rate;
        let dt = len as f32 / fs;

        // Sleep once nothing is left in flight: the input has been silent for longer than the
        // longest path reads back, plus the filters' tail. Paths cannot outgrow the silence
        // (a source recedes slower than sound), so the reads stay silent while asleep.
        if input.iter().all(|v| *v == 0.0) {
            self.idle += len;
        } else {
            self.idle = 0;
        }
        if self.idle > self.reach(fs) {
            if !self.sleeping {
                self.sleep(bank, path);
            }
            // Asleep it leaves the bank alone: its lanes may be handed to an emitter that
            // wakes (Acoustics::place), and nothing reads them back for this one.
            self.delay.skip(len);
            self.last_pos = em.position;
            return;
        }
        self.sleeping = false;
        let start = self.delay.write(input);

        // Kinematics.
        let s = em.position;
        let moved = (s - self.last_pos).length();
        let jump = self.started && moved > (40.0 * dt).max(1.0);
        let vel = match em.velocity {
            Some(v) => v,
            None if self.started && !jump => {
                let raw = (s - self.last_pos) * (1.0 / dt);
                let a = 1.0 - (-dt / 0.03).exp();
                self.vel + (raw - self.vel) * a
            }
            None => Vec3::default(),
        };
        self.vel = vel;
        self.last_pos = s;

        let l = ctx.listener;
        let c = ctx.speed_of_sound;
        // The reflecting ground: the scene's plane, or the emitter's own (terrain), always
        // under both ends of the path.
        let gz = match em.ground_z {
            Some(z) => z.min(l.z.min(s.z) - 0.05),
            None => ctx.ground_z,
        };
        let d1 = s - l;
        let r1 = d1.length().max(0.05);
        let sz = s.z.max(gz + 0.02);
        let img = Vec3::new(s.x, s.y, 2.0 * gz - sz);
        let d2 = img - l;
        let r2 = d2.length().max(0.05);
        let sin_psi = ((sz - gz) + (l.z - gz)) / r2;

        // Wind: refraction state of this path (cheap scalars; the spectrum is a fixed shape
        // per state, see `Context::wind_shadow_db`) and the turbulent wander of its level.
        let (shadow, favour, wander) = self.wind(ctx, s, l, r1, sz, gz, dt);

        // Spectral refit when the predicted change of the path filters exceeds ~0.4 dB:
        // air absorption over the distance change, ground coefficient (grazing angle,
        // image distance), obstacles (Fresnel-zone scale), directivity angle.
        let forward = em.forward.unwrap_or_default();
        let k = &self.key;
        let dr_air = (0.05 / ctx.air_np[NF - 1].max(1e-6)) as f32;
        let moved = (s - k.pos).length();
        let need = !k.valid
            || k.epoch != ctx.epoch
            || (r1 - k.r1).abs() > (0.03 * r1).max(0.3).min(dr_air)
            || (r2 - k.r2).abs() > 0.03 * r2
            || (sin_psi - k.sin_psi).abs() > 0.01
            || moved > if k.obstructed { 0.15 } else { 1.0 }
            || (em.directivity != Directivity::Omni && forward.dot(k.forward) < 0.995)
            || eq_moved(&em.eq_db, &k.eq)
            // 0.75 dB of the deepest wind-shadow / downwind band.
            || (shadow - k.shadow).abs() * ctx.wind_shadow_db.iter().fold(0.0f32, |m, v| m.max(-v)) > 0.75
            || (favour - k.favour).abs() * 5.0 > 0.75;
        if need && (!k.valid || phase % 2 == 0) {
            self.wind_sf = (shadow, favour);
            let obstructed = self.refit(ctx, em, s, r1, r2, img, sin_psi, gz);
            self.key = SpecKey { pos: s, forward, r1, r2, sin_psi, obstructed, epoch: ctx.epoch, valid: true, eq: em.eq_db, shadow, favour };
        }

        let tau1 = retarded_time(d1, vel, c) as f64 + self.extra_m / c as f64;
        let tau2 = retarded_time(d2, Vec3::new(vel.x, vel.y, -vel.z), c) as f64;
        let maxd = self.delay.max_delay();
        let dl1 = (tau1 * fs as f64).clamp(MIN_DELAY, maxd);
        let dl2 = (tau2 * fs as f64).clamp(MIN_DELAY, maxd);
        let fresh = !self.started;
        if fresh {
            self.direct.delay = dl1;
            self.ground.delay = dl2;
        }

        let g1 = em.gain * wander / r1.max(R_MIN);
        let g2 = em.gain * wander / r2.max(R_MIN);
        let mut sh1 = [0.0f32; CHANNELS];
        let mut sh2 = [0.0f32; CHANNELS];
        ambisonics::encode(d1 * (1.0 / r1), &mut sh1);
        ambisonics::encode(d2 * (1.0 / r2), &mut sh2);
        for ch in 0..CHANNELS {
            sh1[ch] *= g1;
            sh2[ch] *= if ctx.features.ground { g2 } else { 0.0 };
        }
        if fresh {
            self.direct.sh = sh1;
            self.ground.sh = sh2;
        }

        // Direct and ground-reflected paths: read, send to the tail, queue for filtering.
        let x = &mut scratch.a[..len];
        read_path(&self.delay, start, self.direct.delay, dl1, jump, x, &mut scratch.c[..len]);
        self.direct.delay = dl1;
        if ctx.features.reverb {
            // The emitter's level scales its tail like its direct and early sound.
            let target = self.send * em.send * em.gain;
            for (o, v) in sends.reverb[..len].iter_mut().zip(x.iter()) {
                *o += v * target;
            }
        }
        bank.write(path, x);
        if ctx.features.ground {
            let xg = &mut scratch.b[..len];
            read_path(&self.delay, start, self.ground.delay, dl2, jump, xg, &mut scratch.c[..len]);
            bank.write(path + 1, xg);
        } else {
            bank.silence(path + 1, len);
        }
        self.ground.delay = dl2;
        self.pending = Some((len, r1, sh1, sh2));

        // Early reflections from the nearest trunks: per emitter the strongest ER_TAPS of
        // them, and only those within 20 dB of the direct sound (others fade out).
        if ctx.features.early_reflections {
            let mut taps = [(0.0f64, 0.0f32); ER_TRUNKS];
            for (j, t) in ctx.er_trunks.iter().enumerate().take(ER_TRUNKS) {
                let (delay, gain) = trunk_reflection(t, s, l, ctx.ground_z, c, fs);
                taps[j] = (delay, gain * em.gain);
            }
            let mut sorted: [f32; ER_TRUNKS] = core::array::from_fn(|j| taps[j].1);
            sorted.sort_unstable_by(|a, b| b.total_cmp(a));
            let floor = (0.1 * g1).max(sorted[ER_TAPS - 1]);
            for (j, &(delay, gain)) in taps.iter().enumerate().take(ctx.er_trunks.len()) {
                let gain = if gain >= floor && gain > 0.0 { gain } else { 0.0 };
                let tap = &mut self.er[j];
                if fresh {
                    tap.delay = delay;
                    tap.gain = gain;
                }
                if gain > 0.0 || tap.gain > 0.0 {
                    self.delay.read_linear_add(start, tap.delay, delay, tap.gain, gain, &mut sends.trunks[j][..len]);
                }
                tap.delay = delay;
                tap.gain = gain;
            }
        }
        self.started = true;
    }

    /// Samples of silent input after which every path reads silence and the filters have rung
    /// out: the longest delay in use (direct, ground, early-reflection taps) plus the tail.
    fn reach(&self, fs: f32) -> usize {
        let mut d = if self.started { self.direct.delay.max(self.ground.delay) } else { 0.0 };
        if self.started {
            for t in self.er.iter().filter(|t| t.gain > 0.0) {
                d = d.max(t.delay);
            }
        }
        d.ceil() as usize + 2 * TAPS + (SLEEP_TAIL_S * fs) as usize
    }

    /// Nothing in flight and the path filters cleared: the bank need not run this emitter.
    pub fn asleep(&self) -> bool {
        self.sleeping
    }

    /// Coefficient ramps of the (direct, ground) path filters, for `FilterBank::process_group`.
    pub fn coeffs_mut(&mut self) -> (&mut CoeffRamp, &mut CoeffRamp) {
        (&mut self.direct.coeffs, &mut self.ground.coeffs)
    }

    /// Second half of a block: near field on the filtered direct path, then both paths onto
    /// the bus (world frame).
    pub fn finish(&mut self, ctx: &Context, bank: &FilterBank, path: usize, bus: &mut [[f32; BLOCK]; CHANNELS], scratch: &mut Scratch) {
        let Some((len, r1, sh1, sh2)) = self.pending.take() else { return };
        let x = &mut scratch.a[..len];
        let xg = &mut scratch.b[..len];
        bank.read(path, x);
        bank.read(path + 1, xg);
        let near = ctx.features.near_field && self.near.process(&ctx.nfc, r1 as f64, x, &mut scratch.orders);
        encode_add(
            bus,
            x,
            if near { Some(&scratch.orders) } else { None },
            &mut self.direct.sh,
            sh1,
            xg,
            &mut self.ground.sh,
            sh2,
            len,
        );
    }

    /// Wind on this path: (shadow penetration, downwind weight, turbulent amplitude factor).
    #[allow(clippy::too_many_arguments)]
    fn wind(&mut self, ctx: &Context, s: Vec3, l: Vec3, r1: f32, sz: f32, gz: f32, dt: f32) -> (f32, f32, f32) {
        if !ctx.features.wind || ctx.wind_mean_ms < 0.3 {
            self.turb = 0.0;
            return (0.0, 0.0, 1.0);
        }
        // Source -> listener, along the wind.
        let (wx, wy) = ctx.wind_to;
        let cos_phi = ((l.x - s.x) * wx + (l.y - s.y) * wy) / r1;
        let p = wind::WindPath { cos_phi: cos_phi as f64, r: r1 as f64, hs: (sz - gz) as f64, hr: (l.z - gz) as f64, u: ctx.wind_ms as f64, c: ctx.speed_of_sound as f64 };
        let rf = wind::refraction(&p);
        let (shadow, favour) = (rf.shadow as f32, rf.favour as f32);
        // Lognormal amplitude with unit-variance OU in dB, normalised to keep the mean power.
        let sigma = wind::fluctuation_db(r1, ctx.wind_mean_ms, shadow);
        let tau = wind::fluctuation_tau(ctx.wind_mean_ms);
        self.turb += -self.turb / tau * dt + (2.0 * dt / tau).sqrt() * self.rng.gauss();
        let k = sigma * (core::f32::consts::LN_10 / 20.0);
        let wander = (k * self.turb - k * k).exp();
        (shadow, favour, wander)
    }

    #[allow(clippy::too_many_arguments)]
    /// Refits both path filters. Returns whether an obstacle affected the path.
    fn refit(&mut self, ctx: &Context, em: &Emitter, s: Vec3, r1: f32, r2: f32, img: Vec3, sin_psi: f32, gz: f32) -> bool {
        let mut obstructed = false;
        let l = ctx.listener;
        let mut ln1 = [0.0f64; NF];
        let mut ln2 = [0.0f64; NF];

        // Air absorption.
        for i in 0..NF {
            ln1[i] = -ctx.air_np[i] * r1 as f64;
            ln2[i] = -ctx.air_np[i] * r2 as f64;
        }

        // Directivity: angle between the emitter's facing and the emission direction.
        if let (Directivity::Sphere { radius }, Some(fwd)) = (em.directivity, em.forward) {
            let fl = fwd.length();
            if fl > 1e-6 {
                let fwd = fwd * (1.0 / fl);
                let e1 = (l - s) * (1.0 / r1);
                // Emission towards the specular point on the ground.
                let tp = (l.z - gz) / (l.z - img.z).max(1e-3);
                let p = l + (img - l) * tp;
                let e2 = p - s;
                let e2 = e2 * (1.0 / e2.length().max(1e-3));
                let th1 = (fwd.dot(e1) as f64).clamp(-1.0, 1.0).acos();
                let th2 = (fwd.dot(e2) as f64).clamp(-1.0, 1.0).acos();
                let c = ctx.speed_of_sound as f64;
                for i in 0..NF {
                    let f = ctx.design.freqs[i];
                    let m = ctx.directivity_mean(i, radius as f64);
                    ln1[i] += directivity::ln_gain(f, th1, radius as f64, c, m);
                    ln2[i] += directivity::ln_gain(f, th2, radius as f64, c, m);
                }
            }
        }

        // Statistical forest: trunk scattering beyond the explicit-obstacle radius, foliage
        // inside the canopy layer.
        if ctx.features.forest {
            let (lt, lc) = ctx.forest_lengths(s, l);
            let scale2 = (r2 / r1) as f64;
            for i in 0..NF {
                let a = ctx.forest_np[i] * lt + ctx.foliage_np[i] * lc;
                ln1[i] -= a;
                ln2[i] -= a * scale2;
            }
        }

        // Explicit obstacles.
        self.extra_m = 0.0;
        if ctx.features.occlusion {
            let c = ctx.speed_of_sound as f64;
            let fmax = ctx.design.freqs[NF - 1];
            for ob in ctx.obstacles.iter() {
                let Some(ap) = diffraction::aperture(ob, s, l, ctx.ground_z) else { continue };
                if ap.negligible(fmax, c) {
                    continue;
                }
                for i in 0..NF {
                    let p = ap.band_power(ctx.design.freqs[i], c).max(1e-4);
                    let a = 0.5 * p.ln();
                    ln1[i] += a;
                    ln2[i] += a;
                }
                self.extra_m += ap.detour();
                obstructed = true;
            }
        }

        // Mix EQ (masking ducks, distance cues) and the wind's refraction: same on both paths.
        let (shadow, favour) = self.wind_sf;
        for i in 0..NF {
            let db = em.eq_db[i] + shadow * ctx.wind_shadow_db[i] + favour * ctx.wind_favour_db[i];
            let a = db as f64 * (core::f64::consts::LN_10 / 20.0);
            ln1[i] += a;
            ln2[i] += a;
        }

        let co1 = ctx.design.fit_min_phase(&ln1, None);
        let beta = &ctx.beta[em.ground.unwrap_or(ctx.ground) as usize];
        let mut q = [C64::ZERO; NF];
        for i in 0..NF {
            q[i] = ground::reflection_dsp(beta[i], ctx.k[i], r2 as f64, sin_psi as f64);
        }
        let co2 = ctx.design.fit_min_phase(&ln2, Some(&q));
        if self.key.valid {
            self.direct.coeffs.ramp_to(co1, 2);
            self.ground.coeffs.ramp_to(co2, 2);
        } else {
            self.direct.coeffs.set(co1);
            self.ground.coeffs.set(co2);
        }

        // Late-tail send: the diffuse field near the listener comes from scattering within
        // ~L of it, so emitters farther away feed it less.
        let ext = ctx.tail_extent;
        self.send = ctx.reverb_send * (-(r1 - ext).max(0.0) / ext).exp();
        obstructed
    }
}

/// Whether the mix EQ moved enough (0.5 dB in any grid band) to refit the path filters.
#[inline]
fn eq_moved(a: &[f32; NF], b: &[f32; NF]) -> bool {
    a.iter().zip(b).any(|(x, y)| (x - y).abs() > 0.5)
}

/// Fractional-delay read of one path. On a position jump (teleport) the old and new delays
/// are crossfaded over the block instead of sweeping the delay (which would chirp).
fn read_path(dl: &DelayLine, start: usize, d0: f64, d1: f64, jump: bool, out: &mut [f32], tmp: &mut [f32]) {
    if !jump {
        dl.read_sinc(start, d0, d1, out);
        return;
    }
    dl.read_sinc(start, d0, d0, out);
    dl.read_sinc(start, d1, d1, tmp);
    let n = out.len() as f32;
    for (i, (o, t)) in out.iter_mut().zip(tmp.iter()).enumerate() {
        let w = (i + 1) as f32 / n;
        *o = *o * (1.0 - w) + t * w;
    }
}

/// Adds the direct path `x` (orders 1..=3 from `orders` if given) and the ground path `xg` to
/// the bus, each with SH gains ramping linearly from `prev` to `target` over the block.
#[allow(clippy::too_many_arguments)]
fn encode_add(
    bus: &mut [[f32; BLOCK]; CHANNELS],
    x: &[f32],
    orders: Option<&[[f32; BLOCK]; ORDER]>,
    prev: &mut [f32; CHANNELS],
    target: [f32; CHANNELS],
    xg: &[f32],
    prev_g: &mut [f32; CHANNELS],
    target_g: [f32; CHANNELS],
    len: usize,
) {
    let inv = 1.0 / len as f32;
    for ch in 0..CHANNELS {
        let src: &[f32] = match (orders, order_of(ch)) {
            (Some(o), m) if m > 0 => &o[m - 1][..len],
            _ => &x[..len],
        };
        let (a, ag) = (prev[ch], prev_g[ch]);
        let (step, step_g) = ((target[ch] - a) * inv, (target_g[ch] - ag) * inv);
        let out = &mut bus[ch][..len];
        // Closed-form ramps, 4 samples at a time.
        let ramp = F4::load(&[1.0, 2.0, 3.0, 4.0]);
        let (mut g1, mut g2) = (F4::splat(a).fma(F4::splat(step), ramp), F4::splat(ag).fma(F4::splat(step_g), ramp));
        let (d1, d2) = (F4::splat(4.0 * step), F4::splat(4.0 * step_g));
        let mut i = 0;
        while i + 4 <= len {
            F4::load(&out[i..]).fma(F4::load(&src[i..]), g1).fma(F4::load(&xg[i..]), g2).store(&mut out[i..]);
            g1 = g1.add(d1);
            g2 = g2.add(d2);
            i += 4;
        }
        for j in i..len {
            let t = (j + 1) as f32;
            out[j] += src[j] * (a + step * t) + xg[j] * (ag + step_g * t);
        }
    }
    *prev = target;
    *prev_g = target_g;
}

/// Geometric-optics specular reflection off a vertical rigid cylinder: (delay in samples,
/// amplitude relative to the emitter's 1-m level).
fn trunk_reflection(t: &crate::acoustics::Trunk, s: Vec3, l: Vec3, gz: f32, c: f32, fs: f32) -> (f64, f32) {
    let (sx, sy) = (s.x - t.x, s.y - t.y);
    let (lx, ly) = (l.x - t.x, l.y - t.y);
    let ds = (sx * sx + sy * sy).sqrt().max(1e-3);
    let dl = (lx * lx + ly * ly).sqrt().max(1e-3);
    let cos2 = (sx * lx + sy * ly) / (ds * dl);
    let cos_i = ((1.0 + cos2) * 0.5).max(0.0).sqrt();
    let h1 = (ds - t.radius).max(0.3);
    let h2 = (dl - t.radius).max(0.3);
    let zr = s.z + (l.z - s.z) * h1 / (h1 + h2);
    let r1 = (h1 * h1 + (zr - s.z).powi(2)).sqrt().max(0.5);
    let r2 = (h2 * h2 + (l.z - zr).powi(2)).sqrt().max(0.5);
    let delay = ((r1 + r2) / c * fs) as f64;
    if zr < gz || zr > gz + t.height {
        return (delay, 0.0);
    }
    let gain = (t.radius * cos_i * 0.5).sqrt() / (r1 * r2 * (r1 + r2)).sqrt() * BARK_REFLECTION;
    (delay, gain)
}
