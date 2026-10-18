//! Critical-band analysis of the mix tracks and masking-aware ducking.
//!
//! Tracks are mono estimates of what reaches the listener: every emitter's calibrated signal
//! times its mix gain over its distance, summed per mix group (the bed) or kept alone (each
//! soloist). Every `HOP` samples each track gets a 512-point Hann FFT (two real tracks per
//! complex FFT) -> Bark band powers -> excitation pattern (loudness.rs). From those:
//! * which foreground source is masked by which bed group, band by band;
//! * a duck curve per bed group: a few dB, only in the bands where it hides a foreground
//!   source, shared among the bed groups by their part of the masker, smoothed (fast
//!   attack, slow release) so it breathes rather than pumps;
//! * per-group loudness (sone), level and masking index for telemetry.

use super::loudness::{self, BandModel, BANDS, EDGES, MASK_INDEX_DB};
use crate::fft::fft;

pub const FFT_N: usize = 512;
/// Analysis hop, samples (≈ 5 ms at 48 kHz).
pub const HOP: usize = 256;

/// Per-band duck/attenuation curve, dB (≤ 0 = cut).
pub type Curve = [f32; BANDS];

pub struct Analysis {
    pub tracks: usize,
    ring: Vec<[f32; FFT_N]>,
    pos: usize,
    pub since: usize,
    window: [f32; FFT_N],
    /// FFT bin range of each band.
    bins: [(usize, usize); BANDS],
    norm: f32,
    re: [f32; FFT_N],
    im: [f32; FFT_N],
    /// Band powers at the listener, Pa², per track: raw frame, fast-smoothed (decisions),
    /// slow-smoothed (telemetry).
    pub frame: Vec<[f32; BANDS]>,
    pub fast: Vec<[f32; BANDS]>,
    pub slow: Vec<[f32; BANDS]>,
    /// Excitation of `fast`.
    pub exc: Vec<[f32; BANDS]>,
    pub model: BandModel,
    a_fast: [f32; 2],
    a_slow: f32,
    /// Band 0 (20–100 Hz) is below the FFT's resolution: a time-domain band pass per track
    /// (one-pole high pass at 20 Hz, two one-pole low passes at 80 Hz) and its energy.
    lf: Vec<[f32; 3]>,
    lf_energy: Vec<f32>,
    lf_n: usize,
    lf_coef: [f32; 2],
}

impl Analysis {
    pub fn new(sample_rate: f32, tracks: usize) -> Self {
        let tracks = tracks + tracks % 2;
        let mut window = [0.0f32; FFT_N];
        let mut w2 = 0.0f32;
        for (i, w) in window.iter_mut().enumerate() {
            *w = 0.5 - 0.5 * (core::f32::consts::TAU * i as f32 / FFT_N as f32).cos();
            w2 += *w * *w;
        }
        let bin_hz = sample_rate / FFT_N as f32;
        let mut bins = [(0usize, 0usize); BANDS];
        // Each bin belongs to the band its centre falls in (narrow low bands may get none).
        let first = |f: f32| ((f / bin_hz).ceil() as usize).clamp(1, FFT_N / 2);
        for z in 0..BANDS {
            bins[z] = (first(EDGES[z]), first(EDGES[z + 1]));
        }
        let hop_s = HOP as f32 / sample_rate;
        let coef = |tau: f32| 1.0 - (-hop_s / tau).exp();
        Self {
            tracks,
            ring: vec![[0.0; FFT_N]; tracks],
            pos: 0,
            since: 0,
            window,
            bins,
            norm: 2.0 / (FFT_N as f32 * w2),
            re: [0.0; FFT_N],
            im: [0.0; FFT_N],
            frame: vec![[0.0; BANDS]; tracks],
            fast: vec![[0.0; BANDS]; tracks],
            slow: vec![[0.0; BANDS]; tracks],
            exc: vec![[0.0; BANDS]; tracks],
            model: BandModel::new(sample_rate),
            a_fast: [coef(0.015), coef(0.12)],
            a_slow: coef(0.8),
            lf: vec![[0.0; 3]; tracks],
            lf_energy: vec![0.0; tracks],
            lf_n: 0,
            lf_coef: [1.0 - (-core::f32::consts::TAU * 20.0 / sample_rate).exp(), 1.0 - (-core::f32::consts::TAU * 80.0 / sample_rate).exp()],
        }
    }

    /// Appends one block of every track (`taps[t][..len]`). Returns true when a hop is due.
    pub fn push(&mut self, taps: &[[f32; crate::engine::BLOCK]], len: usize) -> bool {
        let [hp, lp] = self.lf_coef;
        for (t, tap) in taps.iter().enumerate().take(self.tracks) {
            let r = &mut self.ring[t];
            for (i, v) in tap[..len].iter().enumerate() {
                r[(self.pos + i) % FFT_N] = *v;
            }
            let s = &mut self.lf[t];
            if s[0] == 0.0 && s[1] == 0.0 && s[2] == 0.0 && tap[..len].iter().all(|v| *v == 0.0) {
                continue;
            }
            let mut e = 0.0f32;
            for v in &tap[..len] {
                s[0] += (v - s[0]) * hp;
                let x = v - s[0];
                s[1] += (x - s[1]) * lp;
                s[2] += (s[1] - s[2]) * lp;
                e += s[2] * s[2];
            }
            for q in s.iter_mut() {
                if q.abs() < 1e-20 {
                    *q = 0.0;
                }
            }
            self.lf_energy[t] += e;
        }
        self.lf_n += len;
        self.pos = (self.pos + len) % FFT_N;
        self.since += len;
        self.since >= HOP
    }

    /// Band powers of every track for the newest FFT_N samples, smoothed.
    pub fn analyse(&mut self) {
        self.since = 0;
        let n2 = FFT_N / 2;
        for t in (0..self.tracks).step_by(2) {
            let mut energy = [0.0f32; 2];
            for i in 0..FFT_N {
                let k = (self.pos + i) % FFT_N;
                let w = self.window[i];
                let a = self.ring[t][k] * w;
                let b = self.ring[t + 1][k] * w;
                energy[0] += a * a;
                energy[1] += b * b;
                self.re[i] = a;
                self.im[i] = b;
            }
            if energy[0] == 0.0 && energy[1] == 0.0 {
                self.frame[t] = [0.0; BANDS];
                self.frame[t + 1] = [0.0; BANDS];
            } else {
                fft(&mut self.re, &mut self.im);
                for z in 0..BANDS {
                    let (k0, k1) = self.bins[z];
                    let (mut pa, mut pb) = (0.0f32, 0.0f32);
                    for k in k0..k1.min(n2) {
                        let (zr, zi, nr, ni) = (self.re[k], self.im[k], self.re[FFT_N - k], self.im[FFT_N - k]);
                        pa += (zr + nr) * (zr + nr) + (zi - ni) * (zi - ni);
                        pb += (zr - nr) * (zr - nr) + (zi + ni) * (zi + ni);
                    }
                    self.frame[t][z] = 0.25 * pa * self.norm;
                    self.frame[t + 1][z] = 0.25 * pb * self.norm;
                }
            }
        }
        // Band 0 from the time-domain band pass (×1.6: the 2-pole skirt is narrower than 80 Hz).
        let n = self.lf_n.max(1) as f32;
        for t in 0..self.tracks {
            self.frame[t][0] = 1.6 * self.lf_energy[t] / n;
            self.lf_energy[t] = 0.0;
        }
        self.lf_n = 0;
        for t in 0..self.tracks {
            for z in 0..BANDS {
                let x = self.frame[t][z];
                let f = &mut self.fast[t][z];
                *f += (x - *f) * if x > *f { self.a_fast[0] } else { self.a_fast[1] };
                let s = &mut self.slow[t][z];
                *s += (x - *s) * self.a_slow;
            }
            let fast = self.fast[t];
            self.model.excitation(&fast, &mut self.exc[t]);
        }
    }

    /// Masked threshold (Pa²) that the tracks flagged in `from` put on band z.
    #[inline]
    pub fn masker(&self, from: &[bool], z: usize, duck: Option<&[Curve]>) -> f32 {
        let mut m = 0.0f32;
        for (t, on) in from.iter().enumerate() {
            if *on {
                let d = duck.map_or(1.0, |c| db_gain_sq(c[t][z]));
                m += self.exc[t][z] * d;
            }
        }
        m * MASK_INDEX
    }

    /// Signal-to-mask ratio (dB) of track `t` against `others`, averaged over its bands with
    /// its specific loudness as weight: (without duck, with duck, total weight).
    pub fn smr(&self, t: usize, others: &[bool], duck: &[Curve]) -> (f32, f32, f32) {
        let (mut s0, mut s1, mut w) = (0.0, 0.0, 0.0);
        for z in 0..BANDS {
            let e = self.exc[t][z];
            let n = self.model.specific(z, e, 0.0);
            if n <= 0.0 {
                continue;
            }
            let tq = self.model.e_tq[z];
            s0 += n * 10.0 * (e / (self.masker(others, z, None) + tq)).max(1e-9).log10();
            s1 += n * 10.0 * (e / (self.masker(others, z, Some(duck)) + tq)).max(1e-9).log10();
            w += n;
        }
        if w > 0.0 {
            (s0 / w, s1 / w, w)
        } else {
            (0.0, 0.0, 0.0)
        }
    }

    /// Unmasked and partial loudness (sone) of the sum of the tracks in `members` against the
    /// tracks in `others` (duck curves applied to both if given).
    pub fn partial_union(&self, members: &[bool], others: &[bool], duck: Option<&[Curve]>) -> (f32, f32) {
        let (mut n, mut np) = (0.0, 0.0);
        for z in 0..BANDS {
            let mut e = 0.0;
            for (t, on) in members.iter().enumerate() {
                if *on {
                    e += self.exc[t][z] * duck.map_or(1.0, |c| db_gain_sq(c[t][z]));
                }
            }
            if e <= 0.0 {
                continue;
            }
            n += self.model.specific(z, e, 0.0);
            np += self.model.specific(z, e, self.masker(others, z, duck));
        }
        (n, np)
    }

    /// Unmasked and partial loudness (sone) of track `t` against the tracks in `others`,
    /// with optional duck curves applied to the maskers (and to `t` itself if it is ducked).
    pub fn partial(&self, t: usize, others: &[bool], duck: Option<&[Curve]>) -> (f32, f32) {
        let (mut n, mut np) = (0.0, 0.0);
        let own = |z: usize| duck.map_or(1.0, |c| db_gain_sq(c[t][z]));
        for z in 0..BANDS {
            let e = self.exc[t][z] * own(z);
            if e <= 0.0 {
                continue;
            }
            n += self.model.specific(z, e, 0.0);
            np += self.model.specific(z, e, self.masker(others, z, duck));
        }
        (n, np)
    }
}

pub const MASK_INDEX: f32 = 0.398_107_17; // 10^(-MASK_INDEX_DB / 10)
const _: () = assert!(MASK_INDEX_DB == 4.0);

#[inline]
pub fn db_gain_sq(db: f32) -> f32 {
    if db == 0.0 {
        1.0
    } else {
        10f32.powf(0.1 * db)
    }
}

/// Settings of one ducking pass.
#[derive(Clone, Copy, Debug)]
pub struct DuckParams {
    /// Signal-to-mask ratio a foreground source should keep in its important bands, dB.
    pub target_smr_db: f32,
    /// Deepest cut, dB (positive).
    pub depth_db: f32,
}

/// Desired duck (dB ≤ 0) per bed track and band so that every foreground track keeps its
/// target signal-to-mask ratio in the bands that carry its loudness. `weight[t]` scales how
/// much foreground track t may ask for (0 = none, 1 = full).
pub fn duck_targets(a: &Analysis, fg: &[f32], bed: &[bool], p: &DuckParams, out: &mut [Curve]) {
    let m = &a.model;
    let mut need = [0.0f32; BANDS];
    let mut masker = [0.0f32; BANDS];
    for (z, mz) in masker.iter_mut().enumerate() {
        *mz = a.masker(bed, z, None);
    }
    for (t, &w) in fg.iter().enumerate() {
        if w <= 0.0 {
            continue;
        }
        // Is it audible at all, and where does its energy live? (Its own excitation spreads
        // over neighbouring bands; cutting the bed there would not unmask it.)
        let loud: f32 = (0..BANDS).map(|z| m.specific(z, a.exc[t][z], 0.0)).sum();
        let total: f32 = a.fast[t].iter().sum();
        if loud < 0.02 || total <= 0.0 {
            continue;
        }
        for z in 0..BANDS {
            let share = a.fast[t][z] / total;
            if share < 0.03 {
                continue;
            }
            let smr = 10.0 * (a.exc[t][z] / (masker[z] + m.e_tq[z])).max(1e-9).log10();
            // Important bands ask for the full target, minor ones for less.
            let importance = (share / 0.15).min(1.0) * w;
            let d = ((p.target_smr_db - smr).max(0.0) * importance).min(p.depth_db);
            need[z] = need[z].max(d);
        }
    }
    // Narrow but not surgical: a band's neighbours get half.
    let mut spread = need;
    for z in 0..BANDS {
        let lo = if z > 0 { need[z - 1] } else { 0.0 };
        let hi = if z + 1 < BANDS { need[z + 1] } else { 0.0 };
        spread[z] = need[z].max(0.5 * lo.max(hi));
    }
    for (t, c) in out.iter_mut().enumerate() {
        if !bed.get(t).copied().unwrap_or(false) {
            *c = [0.0; BANDS];
            continue;
        }
        for z in 0..BANDS {
            let total = masker[z] / MASK_INDEX;
            let share = if total > 0.0 { a.exc[t][z] / total } else { 0.0 };
            // The main maskers take the cut; a group contributing < ~10 % is left alone.
            let k = ((share - 0.1) / 0.4).clamp(0.0, 1.0);
            c[z] = -spread[z] * k;
        }
    }
}

/// Spectral niches of the background textures (acoustic niche partitioning, Krause 1993;
/// the stream-segregation benefit of spectral separation, Bregman 1990): each texture track
/// claims the bands that carry its loudness (specific loudness in quiet within `CLAIM` of
/// its loudest band). Where several tracks claim a band, the one with the
/// smallest share is the one that would be lost there: the others step aside, by what it
/// needs to reach 0 dB signal-to-mask ratio, at most `depth_db`. A dominant texture (a
/// brook) thus keeps most of the spectrum but leaves, e.g., the top octave to the leaves.
/// Cuts (dB ≤ 0) are added to `out[t]` for the tracks in `tracks` flagged in `bed`.
pub fn niche_targets(a: &Analysis, tracks: &[usize], bed: &[bool], depth_db: f32, out: &mut [Curve]) {
    const CLAIM: f32 = 0.5;
    if depth_db <= 0.0 {
        return;
    }
    let m = &a.model;
    let n = tracks.len().min(8);
    let on = |k: usize| bed.get(tracks[k]).copied().unwrap_or(false);
    // Shares of the texture excitation per band; each track's loudiest band (specific loudness).
    let mut best = [0.0f32; 8];
    let mut sum = [0.0f32; BANDS];
    for (z, s) in sum.iter_mut().enumerate() {
        *s = (0..n).filter(|&k| on(k)).map(|k| a.exc[tracks[k]][z]).sum();
    }
    let share = |k: usize, z: usize| if sum[z] > 0.0 { a.exc[tracks[k]][z] / sum[z] } else { 0.0 };
    for (k, b) in best.iter_mut().enumerate().take(n) {
        if !on(k) {
            continue;
        }
        // Inaudible tracks claim nothing.
        let loud: f32 = (0..BANDS).map(|z| m.specific(z, a.exc[tracks[k]][z], 0.0)).sum();
        if loud < 0.05 {
            continue;
        }
        *b = (0..BANDS).map(|z| m.specific(z, a.exc[tracks[k]][z], 0.0)).fold(0.0, f32::max);
    }
    if false {
        static C: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        if C.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % 2000 == 0 {
            for z in 0..BANDS {
                let sh: Vec<String> = (0..n).map(|k| format!("{:.2}", share(k, z))).collect();
                let lv: Vec<String> = (0..n).map(|k| format!("{:.0}", super::loudness::db_spl(a.exc[tracks[k]][z]))).collect();
                eprintln!("z{z:2} sum {:.0} dB | shares {} | lv {} | best {:?}", super::loudness::db_spl(sum[z]), sh.join(" "), lv.join(" "), &best[..n]);
            }
        }
    }
    for z in 0..BANDS {
        if sum[z] < m.e_tq[z] {
            continue;
        }
        // The weakest claimant of the band (a share below 3 % is beyond help).
        let mut weak: Option<(usize, f32)> = None;
        let mut claimants = 0;
        for k in 0..n {
            let s = share(k, z);
            if best[k] > 0.0 && m.specific(z, a.exc[tracks[k]][z], 0.0) >= CLAIM * best[k] && s >= 0.03 {
                claimants += 1;
                if weak.map_or(true, |(_, w)| s < w) {
                    weak = Some((k, s));
                }
            }
        }
        let Some((p, _)) = weak else { continue };
        if claimants < 2 {
            continue;
        }
        let tp = tracks[p];
        let others = sum[z] - a.exc[tp][z];
        let smr = 10.0 * (a.exc[tp][z] / (others * MASK_INDEX + m.e_tq[z])).max(1e-9).log10();
        let need = (-smr).clamp(0.0, depth_db);
        if need <= 0.0 || others <= 0.0 {
            continue;
        }
        for k in 0..n {
            let t = tracks[k];
            if k == p || !on(k) {
                continue;
            }
            // The main maskers take the cut.
            let part = ((a.exc[t][z] / others - 0.1) / 0.4).clamp(0.0, 1.0);
            out[t][z] -= need * part;
        }
    }
}

/// Bark-band curve -> the propagation's fit grid (`freqs`), dB, linear in band position.
pub fn to_grid(c: &Curve, freqs: &[f64], out: &mut [f32]) {
    for (o, &f) in out.iter_mut().zip(freqs) {
        let f = f as f32;
        // Fractional band index of f (centre-based).
        let mut x = 0.0f32;
        if f <= loudness::centre(0) {
            x = 0.0;
        } else if f >= loudness::centre(BANDS - 1) {
            x = (BANDS - 1) as f32;
        } else {
            for z in 0..BANDS - 1 {
                let (c0, c1) = (loudness::centre(z), loudness::centre(z + 1));
                if f < c1 {
                    x = z as f32 + (f / c0).ln() / (c1 / c0).ln();
                    break;
                }
            }
        }
        let i = (x.floor() as usize).min(BANDS - 1);
        let j = (i + 1).min(BANDS - 1);
        let u = x - i as f32;
        *o = c[i] * (1.0 - u) + c[j] * u;
    }
}
