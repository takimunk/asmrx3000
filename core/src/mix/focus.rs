//! Perceptual laws of the mix's scene focus (docs/sound-director.md, section 9): distance
//! cues, attention that follows the head, the texture budget controller and the listener's
//! stillness. Pure functions and small state machines; no allocation.
//!
//! * **Distance.** Listeners judge distance from level, direct-to-reverberant ratio (DRR) and
//!   high-frequency loss (Zahorik, Brungart & Bronkhorst 2005; Bronkhorst & Houtgast 1999),
//!   and they compress it: perceived distance grows roughly as r^0.4 (Zahorik 2002). Physical
//!   1/r spreading alone therefore makes a dense scene sound flat. Beyond `R_NEAR` the mix
//!   steepens the level law (extra 1/r^k), darkens the source and lowers the DRR; inside it,
//!   sources get a little louder and drier (presence).
//! * **Front focus.** Attention is directed where the head points (auditory spatial
//!   attention; selective listening improves several dB for an attended direction, Kidd et al.
//!   2005). A frontal cone is lifted and kept clear of ducking, the rear is a bit lower and
//!   darker. Loud sources stay audible behind: the cut is a few dB.
//! * **Texture budget.** Only ~3–4 auditory streams are followed at once (Cowan 2001;
//!   Bregman 1990). Summing tens of decorrelated broadband textures adds loudness (their
//!   intensities add, loudness grows ~I^0.23 per band) but no information: the sum tends to
//!   stationary noise. The budget keeps a few protected texture streams and limits how much
//!   the rest may add in loudness (sone, Zwicker partial loudness of the analysis tracks).

/// Up to this distance (m) a source counts as near: no extra distance attenuation.
pub const R_NEAR: f32 = 4.0;
/// Deepest extra distance attenuation, dB (at full `distance`).
pub const DIST_MAX_DB: f32 = 18.0;
/// Texture streams always kept (beyond the strongest stream of each texture class).
pub const KEEP_STREAMS: usize = 3;
/// Deepest cut of the texture tail, dB.
pub const TAIL_MAX_DB: f32 = 24.0;
/// The tail is measured against at least this much protected loudness (sone): below a few
/// sone the whole bed is a soft wash and nothing piles up.
pub const TAIL_FLOOR_SONE: f32 = 3.0;

#[inline]
pub fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// What the scene focus does to one emitter: level (dB), extra high-frequency tilt (dB at
/// the top of the tilt shape, ≤ 0) and a reverb send multiplier.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cue {
    pub db: f32,
    pub tilt_db: f32,
    pub send: f32,
}

impl Cue {
    pub const NONE: Cue = Cue { db: 0.0, tilt_db: 0.0, send: 1.0 };
}

/// Perceptual distance law at distance `r` (m) for `amount` = mix.distance (0..1).
/// Beyond R_NEAR: extra 1/r^(0.8·amount) (−2.9 dB per doubling at 0.6), capped; up to
/// −8·amount dB of high-frequency tilt at 4 doublings (64 m); the reverb loses only 40 % of
/// the direct sound's drop (lower DRR: farther). Inside R_NEAR: up to +2·amount dB and drier.
pub fn distance_cue(r: f32, amount: f32) -> Cue {
    let a = amount.clamp(0.0, 1.0);
    if a <= 0.0 || !r.is_finite() {
        return Cue::NONE;
    }
    if r <= R_NEAR {
        let near = 1.0 - r.max(0.0) / R_NEAR;
        return Cue { db: 2.0 * a * near, tilt_db: 0.0, send: 1.0 - 0.4 * a * near };
    }
    let oct = (r / R_NEAR).log2();
    let db = (-0.8 * a * 6.0206 * oct).max(-DIST_MAX_DB * a);
    let tilt_db = -8.0 * a * (oct / 4.0).min(1.0);
    let send = 10f32.powf(-0.6 * db / 20.0).min(3.0);
    Cue { db, tilt_db, send }
}

/// Weights of a direction `cos` (cosine between the head's forward axis and the source):
/// (front, rear, side), each 0..1, front + rear + side = 1. Front is full within ~20° and
/// gone at 60°; rear starts at 90° and is full beyond ~148°.
pub fn direction_weights(cos: f32) -> (f32, f32, f32) {
    let front = smoothstep(0.5, 0.94, cos);
    let rear = smoothstep(0.0, 0.85, -cos);
    (front, rear, (1.0 - front - rear).max(0.0))
}

/// Attention that follows the head: `amount` = mix.front_focus. Front +4·amount dB, sides
/// −1·amount, rear −3·amount dB and up to −4·amount dB darker; a soloist behind loses
/// only half of it (a strong source stays audible behind).
pub fn front_cue(cos: f32, amount: f32, soloist: bool) -> Cue {
    let a = amount.clamp(0.0, 1.0);
    if a <= 0.0 {
        return Cue::NONE;
    }
    let (f, r, s) = direction_weights(cos);
    let rear = if soloist { 0.5 } else { 1.0 };
    Cue { db: a * (4.0 * f - 1.0 * s - 3.0 * r * rear), tilt_db: -4.0 * a * r * rear, send: 1.0 }
}

/// Feedback controller of the texture tail: drives `cut_db` (≤ 0) so that the joint loudness
/// of all textures stays within `limit` × the loudness of the protected streams.
#[derive(Clone, Copy, Debug, Default)]
pub struct Budget {
    pub cut_db: f32,
}

impl Budget {
    /// `ratio` = N_all / N_protected (≥ 1), `limit` ≥ 1 (None: no limit), `dt` seconds.
    pub fn update(&mut self, ratio: f32, limit: Option<f32>, max_db: f32, dt: f32) {
        let Some(limit) = limit else {
            // Released over ~2 s.
            self.cut_db *= (-dt / 2.0).exp();
            return;
        };
        if !ratio.is_finite() || ratio <= 0.0 {
            return;
        }
        // Loudness ~ intensity^0.27: the intensity change (dB) that would close the gap.
        let err = (10.0 / 0.27 * (limit / ratio).log10()).clamp(-12.0, 12.0);
        // Down in ~0.7 s, back up in ~3 s.
        let tau = if err < 0.0 { 0.7 } else { 3.0 };
        self.cut_db = (self.cut_db + err * dt / tau).clamp(-max_db, 0.0);
    }
}

/// Texture budget control (0..1) -> loudness ratio limit of the tail over the protected
/// streams; None = no limit.
pub fn budget_limit(budget: f32) -> Option<f32> {
    if budget >= 0.99 {
        None
    } else {
        Some(1.05 + 0.95 * budget.clamp(0.0, 1.0))
    }
}

/// Texture budget control (0..1) -> absolute loudness budget of all textures together at
/// the listener, sone (6 at 0, ~11 at the default 0.35, ~46 at 1 = no limit). For scale: a
/// small brook 4 m away is ~10 sone, steady rain under trees ~20, a downpour 40+.
pub fn budget_sone(budget: f32) -> Option<f32> {
    if budget >= 0.99 {
        None
    } else {
        let b = budget.clamp(0.0, 1.0);
        Some(6.0 + 40.0 * b * b)
    }
}

/// Deepest cut of the whole texture bed by the absolute budget at the default budget and
/// above, dB (protected streams take half of it); twice that at budget 0.
pub const BED_MAX_DB: f32 = 10.0;

/// Cut weight of the tail emitter ranked `k` (0 = highest priority of the tail): the first
/// ones lose half of the tail cut, from the fourth on the full cut.
pub fn tail_weight(k: usize) -> f32 {
    (0.5 + 0.5 * k as f32 / 3.0).min(1.0)
}

/// How still the listener is, 0..1 (rises over ~4 s when stopped, falls in ~1 s on the move),
/// and how much it moves, 0..1.
#[derive(Clone, Copy, Debug, Default)]
pub struct Stillness {
    pub still: f32,
    pub moving: f32,
}

impl Stillness {
    pub fn update(&mut self, speed: f32, dt: f32) {
        let speed = if speed.is_finite() { speed.max(0.0) } else { 0.0 };
        let target = 1.0 - smoothstep(0.3, 1.2, speed);
        let tau = if target > self.still { 4.0 } else { 1.0 };
        self.still += (target - self.still) * (1.0 - (-dt / tau).exp());
        let m = smoothstep(1.0, 8.0, speed);
        self.moving += (m - self.moving) * (1.0 - (-dt / 0.5).exp());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distance_law_recedes_far_and_presents_near() {
        let a = 0.6;
        let c8 = distance_cue(8.0, a);
        let c32 = distance_cue(32.0, a);
        // −2.9 dB per doubling beyond 4 m at 0.6, darker and wetter with distance.
        assert!((c8.db + 2.89).abs() < 0.05, "{c8:?}");
        assert!((c32.db + 8.67).abs() < 0.05, "{c32:?}");
        assert!(c32.tilt_db < c8.tilt_db && c8.tilt_db < 0.0);
        assert!(c32.send > c8.send && c8.send > 1.0);
        // The reverb drops by only 40 % of the direct sound's drop.
        let reverb = 20.0 * c32.send.log10() + c32.db;
        assert!((reverb - 0.4 * c32.db).abs() < 0.05);
        assert!(distance_cue(1000.0, 1.0).db >= -DIST_MAX_DB - 1e-3);
        let near = distance_cue(1.0, a);
        assert!(near.db > 0.5 && near.send < 1.0);
        assert_eq!(distance_cue(10.0, 0.0), Cue::NONE);
    }

    #[test]
    fn front_focus_lifts_the_front_and_keeps_the_rear_audible() {
        let a = 0.5;
        let front = front_cue(1.0, a, false);
        let side = front_cue(0.0, a, false);
        let rear = front_cue(-1.0, a, false);
        let rear_solo = front_cue(-1.0, a, true);
        assert!((front.db - 2.0).abs() < 1e-4 && front.tilt_db == 0.0);
        assert!(side.db < 0.0 && side.db > -1.0);
        assert!((rear.db + 1.5).abs() < 1e-4 && rear.tilt_db < 0.0);
        assert!(rear_solo.db > rear.db && rear_solo.db >= -1.0);
        let (f, r, s) = direction_weights(0.7);
        assert!((f + r + s - 1.0).abs() < 1e-5);
    }

    #[test]
    fn budget_converges_and_releases() {
        let mut b = Budget::default();
        // A plant: the ratio falls as the tail is cut (tail = 3 × protected power, overlapping).
        let ratio = |cut: f32| (1.0 + 3.0 * 10f32.powf(cut / 10.0)).powf(0.23);
        for _ in 0..2000 {
            b.update(ratio(b.cut_db), Some(1.2), TAIL_MAX_DB, 0.005);
        }
        assert!((ratio(b.cut_db) - 1.2).abs() < 0.01, "ratio {} cut {}", ratio(b.cut_db), b.cut_db);
        assert!(b.cut_db < -3.0 && b.cut_db > -TAIL_MAX_DB);
        for _ in 0..2000 {
            b.update(ratio(b.cut_db), None, TAIL_MAX_DB, 0.005);
        }
        assert!(b.cut_db > -0.5);
    }

    #[test]
    fn stillness_rises_slowly_and_falls_fast() {
        let mut s = Stillness::default();
        for _ in 0..100 {
            s.update(0.0, 0.01);
        }
        assert!(s.still > 0.15 && s.still < 0.4, "{}", s.still);
        for _ in 0..1500 {
            s.update(0.0, 0.01);
        }
        assert!(s.still > 0.95);
        for _ in 0..150 {
            s.update(6.0, 0.01);
        }
        assert!(s.still < 0.4 && s.moving > 0.7);
    }
}
