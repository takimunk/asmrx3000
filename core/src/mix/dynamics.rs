//! Scene-level dynamics for headphone listening: a slow loudness leveller towards the target
//! (BS.1770 short-term loudness, partial correction, limited boost so a quiet night stays
//! quiet), a comfort compressor on momentary loudness (thunder claps, cloudbursts, a bird on
//! the next branch) and a peak limiter. Feed-forward: decisions come from the pre-gain signal,
//! so the loop cannot oscillate.

use super::calibration::{PA_DB, PLAYBACK_REF_DB};
use super::loudness::Meter;

#[derive(Clone, Copy, Debug)]
pub struct DynParams {
    /// Integrated-loudness target, LUFS.
    pub target_lufs: f32,
    /// 0 = fixed playback gain, 1 = level fully to the target (short-term).
    pub strength: f32,
}

/// Most the leveller adds: a quiet scene stays quieter than a busy one.
const MAX_BOOST_DB: f32 = 9.0;
const MAX_CUT_DB: f32 = 20.0;
/// Comfort compressor: threshold above the target (LU), ratio.
const COMFORT_LU: f32 = 6.0;
const COMFORT_RATIO: f32 = 4.0;
/// Loudness ceiling over the target that the comfort stage holds at strength ≥ 0.6, LU.
const COMFORT_CEIL_LU: f32 = 10.0;
/// Peak ceiling, linear (−1 dBFS).
const CEILING: f32 = 0.891;

pub struct Dynamics {
    fs: f32,
    /// Pa at the ears -> digital before dynamics.
    playback: f32,
    pre: Meter,
    pub post: Meter,
    /// Leveller and compressor gains, dB.
    pub level_db: f32,
    pub comp_db: f32,
    lim: f32,
    lim_release: f32,
    applied: f32,
    pub bypass: bool,
}

impl Dynamics {
    pub fn new(fs: f32) -> Self {
        Self {
            fs,
            playback: 10f32.powf((PA_DB - PLAYBACK_REF_DB) / 20.0),
            pre: Meter::new(fs),
            post: Meter::new(fs),
            level_db: 0.0,
            comp_db: 0.0,
            lim: 1.0,
            lim_release: 1.0 - (-1.0 / (0.15 * fs)).exp(),
            applied: 1.0,
            bypass: false,
        }
    }

    pub fn playback(&self) -> f32 {
        if self.bypass {
            1.0
        } else {
            self.playback
        }
    }

    /// Total gain right now (dB): leveller + compressor + limiter.
    pub fn gain_db(&self) -> f32 {
        20.0 * self.applied.max(1e-6).log10() - 20.0 * self.playback().log10()
    }

    /// Computes per-sample output gains (`out`, linear, including the playback mapping) for
    /// the block `l`/`r` (pascals at the ears) and meters the result.
    pub fn process(&mut self, l: &[f32], r: &[f32], p: &DynParams, out: &mut [f32]) {
        let len = l.len().min(r.len()).min(out.len());
        let pb = self.playback();
        if self.bypass {
            out[..len].fill(1.0);
            self.post.process(&l[..len], &r[..len], None);
            self.applied = 1.0;
            return;
        }
        // Pre-gain loudness at the playback mapping.
        let pbv = [pb; crate::engine::BLOCK];
        self.pre.process(&l[..len], &r[..len], Some(&pbv[..len]));
        let dt = len as f32 / self.fs;

        // Leveller: correction of the robust 10 s loudness (bursts are clipped out of it and
        // left to the compressor, so a clap does not leave a hole), held while nearly silent.
        let st = self.pre.long_term;
        if st > -55.0 {
            // Too loud comes down nearly all the way (comfort); too quiet is lifted partly and
            // at most MAX_BOOST_DB, so a still night keeps its stillness.
            let s = p.strength.clamp(0.0, 1.0);
            let err = p.target_lufs - st;
            let want = if err < 0.0 { ((2.0 * s).min(1.0) * err).max(-MAX_CUT_DB) } else { (0.6 * s * err).min(MAX_BOOST_DB * s) };
            // Down in ~1 s, up in ~4 s.
            let tau = if want < self.level_db { 1.0 } else { 4.0 };
            self.level_db += (want - self.level_db) * (1.0 - (-dt / tau).exp());
        }
        // Comfort compressor on momentary loudness after the leveller.
        // The detector follows the newest 100 ms too: a clap is caught before the 400 ms
        // momentary window has filled (events stand further out of a budgeted bed).
        let det = self.pre.momentary.max(self.pre.fast - 1.0);
        let over = det + self.level_db - (p.target_lufs + COMFORT_LU);
        let s = p.strength.clamp(0.0, 1.0);
        let mut want = if over > 0.0 { -over * (1.0 - 1.0 / COMFORT_RATIO) * s } else { 0.0 };
        // Comfort ceiling: whatever the ratio leaves of a very loud event (a close thunder clap
        // sits ~30 LU over the target), the loudness stays within COMFORT_CEIL_LU of the target
        // once the dynamics are on (fades in over strength 0.3..0.6).
        let rest = det + self.level_db + want - (p.target_lufs + COMFORT_CEIL_LU);
        let k = ((s - 0.3) / 0.3).clamp(0.0, 1.0);
        if rest > 0.0 {
            want -= rest * k * k * (3.0 - 2.0 * k);
        }
        let tau = if want < self.comp_db { 0.03 } else { 1.5 };
        self.comp_db += (want - self.comp_db) * (1.0 - (-dt / tau).exp());

        // Ramp the smooth gain over the block, limiter per sample.
        let target = pb * 10f32.powf((self.level_db + self.comp_db) / 20.0);
        let start = self.applied / self.lim.max(1e-6);
        for i in 0..len {
            let u = (i + 1) as f32 / len as f32;
            let g = start + (target - start) * u;
            let peak = l[i].abs().max(r[i].abs()) * g;
            let need = if peak > CEILING { CEILING / peak } else { 1.0 };
            self.lim = need.min(self.lim + (1.0 - self.lim) * self.lim_release);
            out[i] = g * self.lim;
        }
        self.applied = target * self.lim;
        self.post.process(&l[..len], &r[..len], Some(&out[..len]));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noise(n: usize, amp: f32, seed: &mut u32) -> Vec<f32> {
        (0..n)
            .map(|_| {
                *seed ^= *seed << 13;
                *seed ^= *seed >> 17;
                *seed ^= *seed << 5;
                amp * ((*seed as f32 / u32::MAX as f32) * 2.0 - 1.0)
            })
            .collect()
    }

    fn run(d: &mut Dynamics, amp: f32, secs: f32, p: &DynParams, seed: &mut u32) -> f32 {
        let blocks = (secs * 48_000.0 / 64.0) as usize;
        let mut g = [0.0f32; 64];
        let mut peak = 0.0f32;
        for _ in 0..blocks {
            let x = noise(64, amp, seed);
            d.process(&x, &x, p, &mut g);
            peak = x.iter().zip(&g).fold(peak, |m, (v, g)| m.max((v * g).abs()));
        }
        peak
    }

    #[test]
    fn levels_towards_target_and_limits_peaks() {
        let p = DynParams { target_lufs: -22.0, strength: 1.0 };
        let mut seed = 1u32;
        // A loud scene (≈ 75 dB at the ears) comes down to the target.
        let mut d = Dynamics::new(48_000.0);
        run(&mut d, 0.03, 12.0, &p, &mut seed);
        assert!((d.post.short_term + 22.0).abs() < 1.5, "loud: {}", d.post.short_term);
        // A quiet one is lifted, but by at most MAX_BOOST_DB.
        let mut q = Dynamics::new(48_000.0);
        run(&mut q, 0.0015, 20.0, &p, &mut seed);
        assert!(q.level_db > 8.0 && q.level_db <= MAX_BOOST_DB + 0.01, "{}", q.level_db);
        assert!(q.post.short_term < -24.0);
        // A sudden clap: the compressor and limiter keep the output under the ceiling.
        let peak = run(&mut d, 8.0, 0.3, &p, &mut seed);
        assert!(peak <= CEILING * 1.001, "{peak}");
        assert!(d.comp_db < -3.0, "{}", d.comp_db);
    }
}
