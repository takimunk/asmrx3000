//! Loudness measurement: ITU-R BS.1770-4 (K-weighting, momentary / short-term / gated
//! integrated LUFS) for the mix, and a Zwicker-style (ISO 532-1 / DIN 45631 in spirit,
//! simplified) critical-band loudness model for the per-group analysis: band powers on the
//! Bark scale -> excitation with level-dependent spreading -> specific loudness N'(z) in
//! sone/Bark, and *partial* loudness of a signal in the presence of a masker.
//!
//! Units: band powers are in Pa² at the listener (the engine's emitter unit is 1 Pa at 1 m,
//! see calibration.rs), levels in dB SPL re 20 µPa.

use core::f32::consts::PI;

/// Critical bands (Zwicker 1961), edges in Hz. Band z spans EDGES[z]..EDGES[z + 1].
pub const BANDS: usize = 24;
pub const EDGES: [f32; BANDS + 1] = [
    20.0, 100.0, 200.0, 300.0, 400.0, 510.0, 630.0, 770.0, 920.0, 1080.0, 1270.0, 1480.0, 1720.0, 2000.0, 2320.0, 2700.0, 3150.0, 3700.0,
    4400.0, 5300.0, 6400.0, 7700.0, 9500.0, 12000.0, 15500.0,
];

/// Geometric band centre, Hz.
pub fn centre(z: usize) -> f32 {
    (EDGES[z] * EDGES[z + 1]).sqrt()
}

/// Band containing `f` (clamped to the table).
pub fn band_of(f: f32) -> usize {
    (0..BANDS).find(|&z| f < EDGES[z + 1]).unwrap_or(BANDS - 1)
}

/// Reference pressure squared, Pa².
pub const P0_SQ: f32 = 20e-6 * 20e-6;

#[inline]
pub fn db_spl(p_sq: f32) -> f32 {
    10.0 * (p_sq.max(1e-20) / P0_SQ).log10()
}

#[inline]
pub fn from_db_spl(db: f32) -> f32 {
    P0_SQ * 10f32.powf(0.1 * db)
}

/// Threshold in quiet, dB SPL (Terhardt 1979), free field.
pub fn threshold_db(f: f32) -> f32 {
    let k = f.max(20.0) / 1000.0;
    3.64 * k.powf(-0.8) - 6.5 * (-0.6 * (k - 3.3) * (k - 3.3)).exp() + 1e-3 * k.powi(4)
}

/// A-weighting gain (power, linear) at `f` Hz (IEC 61672), 1 at 1 kHz.
pub fn a_weight_sq(f: f32) -> f32 {
    let f2 = f * f;
    let ra = (12194.0f32 * 12194.0).powi(2) * f2 * f2 / ((f2 + 20.6 * 20.6) * ((f2 + 107.7 * 107.7) * (f2 + 737.9 * 737.9)).sqrt() * (f2 + 12194.0 * 12194.0));
    // +2.0 dB normalisation at 1 kHz.
    (ra * 1.258_925_4).powi(2)
}

// ---------------------------------------------------------------------------------------
// BS.1770
// ---------------------------------------------------------------------------------------

#[derive(Clone, Copy, Default, Debug)]
pub struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl Biquad {
    #[inline]
    pub fn run(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }

    fn flush(&mut self) {
        if !(self.z1.abs() > 1e-20) {
            self.z1 = 0.0;
        }
        if !(self.z2.abs() > 1e-20) {
            self.z2 = 0.0;
        }
    }

    /// Power gain at `f`.
    pub fn response_sq(&self, f: f32, fs: f32) -> f32 {
        let w = 2.0 * PI * f / fs;
        let (c1, s1, c2, s2) = (w.cos(), w.sin(), (2.0 * w).cos(), (2.0 * w).sin());
        let nr = self.b0 + self.b1 * c1 + self.b2 * c2;
        let ni = -(self.b1 * s1 + self.b2 * s2);
        let dr = 1.0 + self.a1 * c1 + self.a2 * c2;
        let di = -(self.a1 * s1 + self.a2 * s2);
        (nr * nr + ni * ni) / (dr * dr + di * di)
    }
}

/// The two K-weighting stages of BS.1770 for any sample rate (the 48 kHz coefficients of
/// the standard are reproduced exactly by this bilinear design).
pub fn k_weighting(fs: f32) -> [Biquad; 2] {
    let fs = fs as f64;
    // Stage 1: high shelf (head acoustics).
    let (f0, g, q) = (1681.974450955533f64, 3.999843853973347f64, 0.7071752369554196f64);
    let k = (core::f64::consts::PI * f0 / fs).tan();
    let vh = 10f64.powf(g / 20.0);
    let vb = vh.powf(0.4996667741545416);
    let a0 = 1.0 + k / q + k * k;
    let shelf = Biquad {
        b0: ((vh + vb * k / q + k * k) / a0) as f32,
        b1: (2.0 * (k * k - vh) / a0) as f32,
        b2: ((vh - vb * k / q + k * k) / a0) as f32,
        a1: (2.0 * (k * k - 1.0) / a0) as f32,
        a2: ((1.0 - k / q + k * k) / a0) as f32,
        ..Default::default()
    };
    // Stage 2: RLB high pass.
    let (f0, q) = (38.13547087602444f64, 0.5003270373238773f64);
    let k = (core::f64::consts::PI * f0 / fs).tan();
    let a0 = 1.0 + k / q + k * k;
    let hp = Biquad { b0: 1.0, b1: -2.0, b2: 1.0, a1: (2.0 * (k * k - 1.0) / a0) as f32, a2: ((1.0 - k / q + k * k) / a0) as f32, ..Default::default() };
    [shelf, hp]
}

/// Loudness of a mean-square K-weighted sum (both channels), LUFS.
#[inline]
pub fn lufs(ms_sum: f32) -> f32 {
    -0.691 + 10.0 * ms_sum.max(1e-20).log10()
}

const HIST_MIN: f32 = -70.0;
const HIST_STEP: f32 = 0.1;
const HIST_BINS: usize = 760;
/// Momentary blocks of 400 ms every 100 ms.
const SUB_BLOCKS: usize = 4;

/// Stereo BS.1770 meter: momentary (400 ms), short-term (3 s) and gated integrated loudness.
/// Allocation-free after construction; integrated gating uses a 0.1 LU histogram.
pub struct Meter {
    k: [[Biquad; 2]; 2],
    /// Samples per 100 ms step.
    step: usize,
    acc: f32,
    n: usize,
    /// Last 30 sub-blocks (3 s) of mean square, ring.
    ring: [f32; 30],
    pos: usize,
    filled: usize,
    hist: Box<[u32; HIST_BINS]>,
    hist_energy: Box<[f64; HIST_BINS]>,
    pub momentary: f32,
    pub short_term: f32,
    pub integrated: f32,
}

impl Meter {
    pub fn new(fs: f32) -> Self {
        let k = k_weighting(fs);
        Self {
            k: [k, k],
            step: ((0.1 * fs) as usize).max(1),
            acc: 0.0,
            n: 0,
            ring: [0.0; 30],
            pos: 0,
            filled: 0,
            hist: Box::new([0; HIST_BINS]),
            hist_energy: Box::new([0.0; HIST_BINS]),
            momentary: -70.0,
            short_term: -70.0,
            integrated: -70.0,
        }
    }

    pub fn reset_integrated(&mut self) {
        self.hist.iter_mut().for_each(|h| *h = 0);
        self.hist_energy.iter_mut().for_each(|h| *h = 0.0);
        self.integrated = -70.0;
    }

    /// Feeds one block. `gain` multiplies the input (per sample) if given.
    pub fn process(&mut self, l: &[f32], r: &[f32], gain: Option<&[f32]>) {
        for i in 0..l.len().min(r.len()) {
            let g = gain.map_or(1.0, |g| g[i]);
            let a = kr_run(&mut self.k[0], l[i] * g);
            let b = kr_run(&mut self.k[1], r[i] * g);
            self.acc += a * a + b * b;
            self.n += 1;
            if self.n >= self.step {
                let ms = self.acc / self.n as f32;
                self.acc = 0.0;
                self.n = 0;
                self.push(ms);
            }
        }
        for s in self.k.iter_mut().flatten() {
            s.flush();
        }
    }

    fn push(&mut self, ms: f32) {
        self.ring[self.pos] = ms;
        self.pos = (self.pos + 1) % self.ring.len();
        self.filled = (self.filled + 1).min(self.ring.len());
        let back = |k: usize| self.ring[(self.pos + self.ring.len() - 1 - k) % self.ring.len()];
        let m: f32 = (0..SUB_BLOCKS.min(self.filled)).map(back).sum::<f32>() / SUB_BLOCKS as f32;
        let s: f32 = (0..self.filled).map(back).sum::<f32>() / self.ring.len() as f32;
        self.momentary = lufs(m).max(-70.0);
        self.short_term = lufs(s).max(-70.0);
        // Gating block = the momentary block (75 % overlap).
        if self.filled >= SUB_BLOCKS {
            let l = lufs(m);
            if l > HIST_MIN {
                let b = (((l - HIST_MIN) / HIST_STEP) as usize).min(HIST_BINS - 1);
                self.hist[b] += 1;
                self.hist_energy[b] += m as f64;
            }
            self.integrated = self.gated();
        }
    }

    fn gated(&self) -> f32 {
        // Absolute gate -70 LUFS (the histogram floor), relative gate -10 LU.
        let (mut n, mut e) = (0u64, 0.0f64);
        for b in 0..HIST_BINS {
            n += self.hist[b] as u64;
            e += self.hist_energy[b];
        }
        if n == 0 {
            return -70.0;
        }
        let rel = lufs((e / n as f64) as f32) - 10.0;
        let first = (((rel - HIST_MIN) / HIST_STEP).max(0.0) as usize).min(HIST_BINS - 1);
        let (mut n2, mut e2) = (0u64, 0.0f64);
        for b in first..HIST_BINS {
            n2 += self.hist[b] as u64;
            e2 += self.hist_energy[b];
        }
        if n2 == 0 {
            -70.0
        } else {
            lufs((e2 / n2 as f64) as f32)
        }
    }
}

#[inline]
fn kr_run(k: &mut [Biquad; 2], x: f32) -> f32 {
    let y = k[0].run(x);
    k[1].run(y)
}

// ---------------------------------------------------------------------------------------
// Critical-band loudness (Zwicker)
// ---------------------------------------------------------------------------------------

/// Lower spreading slope, dB/Bark (towards lower bands).
const LOWER_SLOPE_DB: f32 = 27.0;
/// Masking index for noise-like maskers: the masked threshold sits this far below the
/// masker's excitation (Zwicker & Fastl: ~2–6 dB for noise; 4 dB here).
pub const MASK_INDEX_DB: f32 = 4.0;

/// Static tables of the band model.
pub struct BandModel {
    /// Threshold-in-quiet excitation per band, Pa².
    pub e_tq: [f32; BANDS],
    /// 0.08 (E_TQ / E0)^0.23 · CAL, the per-band prefactor of the specific loudness.
    pre: [f32; BANDS],
    /// Upper slope coefficients: s(L) = 24 + 230/f - 0.2 L (dB/Bark), per masker band.
    upper_base: [f32; BANDS],
    lower: f32,
    /// A- and K-weighting power gains at band centres.
    pub a_w: [f32; BANDS],
    pub k_w: [f32; BANDS],
}


impl BandModel {
    pub fn new(fs: f32) -> Self {
        let mut e_tq = [0.0; BANDS];
        let mut pre = [0.0; BANDS];
        let mut upper_base = [0.0; BANDS];
        let mut a_w = [0.0; BANDS];
        let mut k_w = [0.0; BANDS];
        let k = k_weighting(fs);
        for z in 0..BANDS {
            let f = centre(z);
            // Threshold excitation: Terhardt's threshold (minimum 0 dB) for the band.
            let tq = threshold_db(f).max(-2.0);
            e_tq[z] = from_db_spl(tq);
            pre[z] = 0.08 * (e_tq[z] / P0_SQ).powf(0.23);
            upper_base[z] = 24.0 + 230.0 / f;
            a_w[z] = a_weight_sq(f);
            k_w[z] = k[0].response_sq(f, fs) * k[1].response_sq(f, fs);
        }
        let mut m = Self { e_tq, pre, upper_base, lower: 10f32.powf(-0.1 * LOWER_SLOPE_DB), a_w, k_w };
        // Calibrate: a 1 kHz tone at 40 dB SPL is 1 sone by definition.
        let mut p = [0.0f32; BANDS];
        p[band_of(1000.0)] = from_db_spl(40.0);
        let mut e = [0.0f32; BANDS];
        m.excitation(&p, &mut e);
        let s = 1.0 / m.loudness(&e, None);
        m.pre.iter_mut().for_each(|v| *v *= s);
        m
    }

    /// Excitation pattern (Pa² per band) from band powers, with Terhardt's level-dependent
    /// upper slope (shallower for loud maskers) and a fixed 27 dB/Bark lower slope.
    pub fn excitation(&self, p: &[f32; BANDS], out: &mut [f32; BANDS]) {
        // Upward spread: each masker with its own slope.
        out.copy_from_slice(p);
        for k in 0..BANDS - 1 {
            let pk = p[k];
            if pk <= 1e-18 {
                continue;
            }
            let slope = (self.upper_base[k] - 0.2 * db_spl(pk)).clamp(5.0, 40.0);
            let r = 10f32.powf(-0.1 * slope);
            let mut v = pk;
            for o in out.iter_mut().skip(k + 1) {
                v *= r;
                if v < 1e-18 {
                    break;
                }
                *o += v;
            }
        }
        // Downward spread: one recursion.
        let mut v = 0.0f32;
        for z in (0..BANDS - 1).rev() {
            v = (v + p[z + 1]) * self.lower;
            out[z] += v;
        }
    }

    /// Specific loudness N'(z), sone/Bark, of excitation `e` with an extra masked threshold
    /// `mask` (Pa², 0 = in quiet). With a masker this is Zwicker's partial loudness: the
    /// threshold term is replaced by threshold-in-quiet + masked threshold.
    #[inline]
    pub fn specific(&self, z: usize, e: f32, mask: f32) -> f32 {
        let et = self.e_tq[z] + mask;
        // N' = 0.08 (E_T/E0)^0.23 [(0.5 + 0.5 E/E_T)^0.23 - 1]; E_T/E0 prefactor scales with
        // the raised threshold.
        let pre = if mask > 0.0 { self.pre[z] * (et / self.e_tq[z]).powf(0.23) } else { self.pre[z] };
        let x = 0.5 + 0.5 * e / et;
        if x <= 1.0 {
            0.0
        } else {
            pre * (x.powf(0.23) - 1.0)
        }
    }

    /// Total loudness in sone (1 Bark per band).
    pub fn loudness(&self, e: &[f32; BANDS], mask: Option<&[f32; BANDS]>) -> f32 {
        (0..BANDS).map(|z| self.specific(z, e[z], mask.map_or(0.0, |m| m[z]))).sum()
    }
}

/// Loudness level in phon from sone (≥ 1 sone: 40 + 33.22 log10 N).
pub fn phon(sone: f32) -> f32 {
    if sone >= 1.0 {
        40.0 + 33.22 * sone.log10()
    } else {
        40.0 * (sone + 0.0005).powf(0.35)
    }
}

// ---------------------------------------------------------------------------------------
// A-weighted sound level meter (offline measurement probes)
// ---------------------------------------------------------------------------------------

/// Time-domain A-weighting (IEC 61672 poles, bilinear) with Fast (125 ms) time weighting:
/// LAeq and LAFmax of a signal in Pa. Used by calibration probes, not the live mix.
#[derive(Clone)]
pub struct SoundLevelMeter {
    a: [Biquad; 3],
    fast: f32,
    coef: f32,
    pub energy: f64,
    pub samples: u64,
    pub max_fast: f32,
}

fn bilinear(b: [f64; 3], a: [f64; 3], fs: f64) -> Biquad {
    // H(s) = (b0 s² + b1 s + b2) / (a0 s² + a1 s + a2), s = 2 fs (1 - z^-1) / (1 + z^-1).
    let c = 2.0 * fs;
    let c2 = c * c;
    let nb = [b[0] * c2 + b[1] * c + b[2], 2.0 * (b[2] - b[0] * c2), b[0] * c2 - b[1] * c + b[2]];
    let na = [a[0] * c2 + a[1] * c + a[2], 2.0 * (a[2] - a[0] * c2), a[0] * c2 - a[1] * c + a[2]];
    Biquad {
        b0: (nb[0] / na[0]) as f32,
        b1: (nb[1] / na[0]) as f32,
        b2: (nb[2] / na[0]) as f32,
        a1: (na[1] / na[0]) as f32,
        a2: (na[2] / na[0]) as f32,
        ..Default::default()
    }
}

impl SoundLevelMeter {
    pub fn new(fs: f32) -> Self {
        let w = |f: f64| 2.0 * core::f64::consts::PI * f;
        let (w1, w2, w3, w4) = (w(20.598997), w(107.65265), w(737.86223), w(12194.217));
        let fsd = fs as f64;
        let mut a = [
            bilinear([1.0, 0.0, 0.0], [1.0, 2.0 * w1, w1 * w1], fsd),
            bilinear([1.0, 0.0, 0.0], [1.0, w2 + w3, w2 * w3], fsd),
            bilinear([0.0, 0.0, 1.0], [1.0, 2.0 * w4, w4 * w4], fsd),
        ];
        // Normalise to 0 dB at 1 kHz.
        let g: f32 = a.iter().map(|q| q.response_sq(1000.0, fs)).product();
        let s = 1.0 / g.sqrt();
        a[0].b0 *= s;
        a[0].b1 *= s;
        a[0].b2 *= s;
        Self { a, fast: 0.0, coef: 1.0 - (-1.0 / (0.125 * fs)).exp(), energy: 0.0, samples: 0, max_fast: 0.0 }
    }

    pub fn process(&mut self, x: &[f32], gain: f32) {
        for &v in x {
            let mut y = v * gain;
            for q in self.a.iter_mut() {
                y = q.run(y);
            }
            let p = y * y;
            self.energy += p as f64;
            self.fast += (p - self.fast) * self.coef;
            self.max_fast = self.max_fast.max(self.fast);
        }
        self.samples += x.len() as u64;
        for q in self.a.iter_mut() {
            q.flush();
        }
    }

    pub fn laeq(&self) -> f32 {
        db_spl((self.energy / self.samples.max(1) as f64) as f32)
    }

    pub fn lafmax(&self) -> f32 {
        db_spl(self.max_fast)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(f: f32, fs: f32, n: usize, amp: f32) -> Vec<f32> {
        (0..n).map(|i| amp * (2.0 * PI * f * i as f32 / fs).sin()).collect()
    }

    #[test]
    fn k_weighting_matches_the_standard_at_48k() {
        let k = k_weighting(48_000.0);
        // BS.1770-4 Table 1 / 2.
        assert!((k[0].b0 - 1.53512485958697).abs() < 1e-5);
        assert!((k[0].a1 + 1.69065929318241).abs() < 1e-5);
        assert!((k[1].a1 + 1.99004745483398).abs() < 1e-5);
        assert!((k[1].a2 - 0.99007225036621).abs() < 1e-5);
    }

    #[test]
    fn full_scale_1k_sine_reads_minus_3_lufs_per_channel() {
        // BS.1770: a 0 dBFS 997 Hz sine in one channel reads -3.01 LKFS.
        let fs = 48_000.0;
        let mut m = Meter::new(fs);
        let x = sine(997.0, fs, 48_000 * 4, 1.0);
        let z = vec![0.0; x.len()];
        m.process(&x, &z, None);
        assert!((m.momentary + 3.01).abs() < 0.1, "{}", m.momentary);
        assert!((m.integrated + 3.01).abs() < 0.1, "{}", m.integrated);
    }

    #[test]
    fn integrated_loudness_gates_silence() {
        let fs = 48_000.0;
        let mut m = Meter::new(fs);
        let x = sine(997.0, fs, 48_000 * 4, 0.1);
        let z = vec![0.0; x.len()];
        m.process(&x, &x, None);
        let loud = m.integrated;
        m.process(&z, &z, None);
        assert!((m.integrated - loud).abs() < 0.2, "silence must be gated: {} vs {loud}", m.integrated);
        assert!(m.momentary <= -69.0);
    }

    #[test]
    fn one_sone_at_40_phon() {
        let bm = BandModel::new(48_000.0);
        let mut p = [0.0f32; BANDS];
        let z = (0..BANDS).find(|&z| EDGES[z] <= 1000.0 && 1000.0 < EDGES[z + 1]).unwrap();
        p[z] = from_db_spl(40.0);
        let mut e = [0.0; BANDS];
        bm.excitation(&p, &mut e);
        let n = bm.loudness(&e, None);
        assert!((n - 1.0).abs() < 0.05, "{n} sone");
        // +10 dB doubles loudness (roughly, above 40 phon).
        p[z] = from_db_spl(60.0);
        bm.excitation(&p, &mut e);
        let n60 = bm.loudness(&e, None);
        assert!(n60 > 3.0 && n60 < 5.0, "{n60} sone at 60 dB");
    }

    #[test]
    fn masker_reduces_partial_loudness_in_its_bands_only() {
        let bm = BandModel::new(48_000.0);
        let mut sig = [0.0f32; BANDS];
        sig[17] = from_db_spl(50.0); // ~4 kHz
        let mut es = [0.0; BANDS];
        bm.excitation(&sig, &mut es);
        let alone = bm.loudness(&es, None);
        let mut mask = [0.0f32; BANDS];
        let masker_at = |z: usize, mask: &mut [f32; BANDS]| {
            let mut m = [0.0f32; BANDS];
            m[z] = from_db_spl(60.0);
            bm.excitation(&m, mask);
            mask.iter_mut().for_each(|v| *v *= 10f32.powf(-0.1 * MASK_INDEX_DB));
        };
        masker_at(17, &mut mask);
        let near = bm.loudness(&es, Some(&mask));
        masker_at(3, &mut mask);
        let far = bm.loudness(&es, Some(&mask));
        assert!(near < 0.3 * alone, "on-band masker: {near} of {alone}");
        assert!(far > 0.9 * alone, "off-band masker: {far} of {alone}");
    }

    #[test]
    fn a_weighting_meter_reads_sine_levels() {
        let fs = 48_000.0;
        // 1 Pa RMS = 94 dB SPL.
        for (f, expect) in [(1000.0, 94.0), (100.0, 94.0 - 19.1), (8000.0, 94.0 - 1.1)] {
            let mut m = SoundLevelMeter::new(fs);
            m.process(&sine(f, fs, 48_000, 2f32.sqrt()), 1.0);
            assert!((m.laeq() - expect).abs() < 0.6, "{f} Hz: {}", m.laeq());
            assert!((a_weight_sq(f).log10() * 10.0 - (expect - 94.0)).abs() < 0.3);
        }
    }
}
