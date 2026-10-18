//! Song syntax (GRU over syllable tokens) and the per-token contour bank.
//! Mirrors `ml/asmr_ml/song.py`.

use super::layers::{Gru, GruState, Linear};
use super::weights::{Error, Weights};
use crate::rng::Rng;

pub struct SongModel {
    emb: Vec<f32>,
    embed: usize,
    gru: Gru,
    out: Linear,
    /// Number of syllable tokens; index `k` is BOS on the input side and EOS on the output side.
    pub k: usize,
    /// Contour points per syllable.
    pub p: usize,
    modes: usize,
    /// Real syllables kept per token (exemplars).
    ex: usize,
    f0: Vec<f32>,
    loud: Vec<f32>,
    effort: Vec<f32>,
    dur: Vec<f32>,
    count: Vec<f32>,
    mode_vec: Vec<f32>,
    coef_sd: Vec<f32>,
}

pub struct SongState {
    gru: GruState,
    y: Vec<f32>,
}

/// One syllable's control contour (log2 f0, loudness dB, effort) at `p` points + duration.
pub struct Contour {
    pub f0: Vec<f32>,
    pub loud: Vec<f32>,
    pub effort: Vec<f32>,
    pub dur_s: f32,
}

impl SongModel {
    pub fn load(w: &Weights) -> Result<Self, Error> {
        let ed = w.dims("song.emb.weight")?.to_vec();
        let k = ed[0] - 1;
        let fd = w.dims("contour.f0")?.to_vec();
        if fd.len() != 3 {
            return Err(Error("contour.f0: expected [tokens, exemplars, points]".into()));
        }
        let (ex, p) = (fd[1], fd[2]);
        let modes = w.dims("contour.coef_sd")?[1];
        let m = Self {
            emb: w.get("song.emb.weight", &[k + 1, ed[1]])?.to_vec(),
            embed: ed[1],
            gru: Gru::load(w, "song.gru")?,
            out: Linear::load(w, "song.out")?,
            k,
            p,
            modes,
            ex,
            f0: w.get("contour.f0", &[k, ex, p])?.to_vec(),
            loud: w.get("contour.loud", &[k, ex, p])?.to_vec(),
            effort: w.get("contour.effort", &[k, ex, p])?.to_vec(),
            dur: w.get("contour.dur", &[k, ex])?.to_vec(),
            count: w.get("contour.count", &[k])?.to_vec(),
            mode_vec: w.get("contour.modes", &[k, modes, 2 * p])?.to_vec(),
            coef_sd: w.get("contour.coef_sd", &[k, modes])?.to_vec(),
        };
        if m.out.n_out != k + 3 || m.gru.ih.n_in != m.embed {
            return Err(Error("song: layer sizes do not fit together".into()));
        }
        Ok(m)
    }

    pub fn params(&self) -> usize {
        self.emb.len() + [&self.out, &self.gru.ih, &self.gru.hh].iter().map(|l| l.w.len() + l.b.len()).sum::<usize>()
    }

    pub fn new_state(&self) -> SongState {
        SongState { gru: self.gru.state(), y: vec![0.0; self.out.n_out] }
    }

    pub fn new_contour(&self) -> Contour {
        Contour { f0: vec![0.0; self.p], loud: vec![0.0; self.p], effort: vec![0.0; self.p], dur_s: 0.1 }
    }

    pub fn reset(&self, st: &mut SongState) {
        st.gru.reset();
    }

    /// Feeds the previous token (`None` = start of song) and samples the next one.
    /// Returns `None` at end of song, else (token, gap in seconds before it).
    pub fn next(&self, st: &mut SongState, prev: Option<usize>, temperature: f32, rng: &mut Rng) -> Option<(usize, f32)> {
        let i = prev.unwrap_or(self.k);
        self.gru.step(&self.emb[i * self.embed..(i + 1) * self.embed], &mut st.gru);
        self.out.forward(&st.gru.h, &mut st.y);
        let t = temperature.max(0.05);
        let logits = &st.y[..self.k + 1];
        let mx = logits.iter().fold(f32::NEG_INFINITY, |a, &b| a.max(b));
        let total: f32 = logits.iter().map(|l| ((l - mx) / t).exp()).sum();
        let mut u = rng.uniform() * total;
        let mut tok = self.k;
        for (j, l) in logits.iter().enumerate() {
            u -= ((l - mx) / t).exp();
            if u <= 0.0 {
                tok = j;
                break;
            }
        }
        // A song never ends before its first syllable.
        if tok == self.k && prev.is_none() {
            tok = (0..self.k).max_by(|&a, &b| logits[a].total_cmp(&logits[b])).unwrap_or(0);
        }
        if tok == self.k {
            return None;
        }
        let mu = st.y[self.k + 1];
        let sd = st.y[self.k + 2].clamp(-4.0, 2.0).exp();
        let gap = (mu + sd * rng.gauss() * t.min(1.0)).exp().clamp(0.005, 1.5);
        Some((tok, gap))
    }

    /// Samples a contour for `tok`: one of its real exemplar syllables, nudged along the token's
    /// PCA modes; `variation` scales the nudge (0 = exact exemplar).
    pub fn contour(&self, tok: usize, variation: f32, rng: &mut Rng, c: &mut Contour) {
        let p = self.p;
        let n = (self.count[tok] as usize).clamp(1, self.ex);
        let e = (rng.next_u32() as usize) % n;
        let base = (tok * self.ex + e) * p;
        c.f0.copy_from_slice(&self.f0[base..base + p]);
        c.loud.copy_from_slice(&self.loud[base..base + p]);
        c.effort.copy_from_slice(&self.effort[base..base + p]);
        for m in 0..self.modes {
            let coef = rng.gauss().clamp(-2.0, 2.0) * self.coef_sd[tok * self.modes + m] * 0.5 * variation;
            let v = &self.mode_vec[(tok * self.modes + m) * 2 * p..(tok * self.modes + m + 1) * 2 * p];
            for j in 0..p {
                c.f0[j] += coef * v[j];
                c.loud[j] += coef * v[p + j] * 10.0;
            }
        }
        let jitter = 0.1 * variation * rng.gauss().clamp(-2.0, 2.0);
        c.dur_s = (self.dur[tok * self.ex + e] + jitter).exp().clamp(0.015, 0.8);
    }
}

/// Linear interpolation of a contour at u in [0, 1].
#[inline]
pub fn at(v: &[f32], u: f32) -> f32 {
    let x = u.clamp(0.0, 1.0) * (v.len() - 1) as f32;
    let i = (x as usize).min(v.len() - 2);
    let t = x - i as f32;
    v[i] + (v[i + 1] - v[i]) * t
}
