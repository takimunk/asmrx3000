//! Song syntax (GRU over syllable tokens) and the contour decoder that turns a token into a
//! syllable (f0 / loudness / effort contour + duration). Mirrors `ml/asmr_ml/multisong.py`;
//! the species conditioning has already been folded into biases by `compose`.

use super::layers::{leaky_relu, Gru, GruState, Linear};
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
    tok: Vec<f32>,
    tok_dim: usize,
    dec1: Linear,
    dec2: Linear,
    dec3: Linear,
    pub z_dim: usize,
    leak: f32,
    /// Probability of re-singing a token's earlier rendition within a song.
    reuse: f32,
}

pub struct SongState {
    gru: GruState,
    y: Vec<f32>,
    inp: Vec<f32>,
    h1: Vec<f32>,
    h2: Vec<f32>,
    c: Vec<f32>,
    /// z used by each token earlier in this song (valid where `used[tok]`), for repeats.
    z_mem: Vec<f32>,
    used: Vec<bool>,
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
        let td = w.dims("contour.tok.weight")?.to_vec();
        let dec1 = Linear::load(w, "contour.dec1")?;
        let dec3 = Linear::load(w, "contour.dec3")?;
        let p = (dec3.n_out - 1) / 3;
        let m = Self {
            emb: w.get("song.emb.weight", &[k + 1, ed[1]])?.to_vec(),
            embed: ed[1],
            gru: Gru::load(w, "song.gru")?,
            out: Linear::load(w, "song.out")?,
            k,
            p,
            tok: w.get("contour.tok.weight", &[k, td[1]])?.to_vec(),
            tok_dim: td[1],
            z_dim: dec1.n_in - td[1],
            dec1,
            dec2: Linear::load(w, "contour.dec2")?,
            dec3,
            leak: w.scalar("cfg.leak")?,
            reuse: w.scalar("cfg.reuse")?,
        };
        if m.out.n_out != k + 3 || m.gru.ih.n_in != m.embed || td[0] != k || m.dec3.n_out != 3 * p + 1 {
            return Err(Error("song: layer sizes do not fit together".into()));
        }
        Ok(m)
    }

    pub fn params(&self) -> usize {
        self.emb.len()
            + self.tok.len()
            + [&self.out, &self.gru.ih, &self.gru.hh, &self.dec1, &self.dec2, &self.dec3]
                .iter()
                .map(|l| l.w.len() + l.b.len())
                .sum::<usize>()
    }

    pub fn new_state(&self) -> SongState {
        SongState {
            gru: self.gru.state(),
            y: vec![0.0; self.out.n_out],
            inp: vec![0.0; self.dec1.n_in],
            h1: vec![0.0; self.dec1.n_out],
            h2: vec![0.0; self.dec2.n_out],
            c: vec![0.0; self.dec3.n_out],
            z_mem: vec![0.0; self.k * self.z_dim],
            used: vec![false; self.k],
        }
    }

    pub fn new_contour(&self) -> Contour {
        Contour { f0: vec![0.0; self.p], loud: vec![0.0; self.p], effort: vec![0.0; self.p], dur_s: 0.1 }
    }

    /// Starts a new song: clears the syntax state and the per-song rendition memory.
    pub fn reset(&self, st: &mut SongState) {
        st.gru.reset();
        st.used.iter_mut().for_each(|u| *u = false);
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

    /// Contour decoder on `st.inp[tok_dim..]` (= z): raw output vector into `st.c`.
    fn decode(&self, st: &mut SongState, tok: usize) {
        let td = self.tok_dim;
        st.inp[..td].copy_from_slice(&self.tok[tok * td..(tok + 1) * td]);
        self.dec1.forward(&st.inp, &mut st.h1);
        st.h1.iter_mut().for_each(|v| *v = leaky_relu(*v, self.leak));
        self.dec2.forward(&st.h1, &mut st.h2);
        st.h2.iter_mut().for_each(|v| *v = leaky_relu(*v, self.leak));
        self.dec3.forward(&st.h2, &mut st.c);
    }

    /// Raw decoder output for a given z (parity tests against PyTorch).
    pub fn decode_raw<'a>(&self, st: &'a mut SongState, tok: usize, z: &[f32]) -> &'a [f32] {
        st.inp[self.tok_dim..].copy_from_slice(z);
        self.decode(st, tok);
        &st.c
    }

    /// Decodes a syllable for `tok` with latent z ~ N(0, variation^2). Within a song a token
    /// re-uses its earlier z with the species' `reuse` probability (a chaffinch trill repeats
    /// one rendition; a robin rarely repeats itself).
    pub fn contour(&self, st: &mut SongState, tok: usize, variation: f32, rng: &mut Rng, c: &mut Contour) {
        let (zd, p) = (self.z_dim, self.p);
        let z = &mut st.z_mem[tok * zd..(tok + 1) * zd];
        if !(st.used[tok] && rng.uniform() < self.reuse) {
            for v in z.iter_mut() {
                *v = rng.gauss().clamp(-2.5, 2.5) * variation;
            }
            st.used[tok] = true;
        }
        st.inp[self.tok_dim..].copy_from_slice(z);
        self.decode(st, tok);
        // Same scaling as multisong.contour_vec: f0 - 12 (log2 Hz), loudness / 20, effort, ln dur.
        for j in 0..p {
            c.f0[j] = st.c[j] + 12.0;
            c.loud[j] = st.c[p + j] * 20.0;
            c.effort[j] = st.c[2 * p + j].clamp(0.0, 1.0);
        }
        c.dur_s = st.c[3 * p].exp().clamp(0.015, 0.8);
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

#[cfg(test)]
mod tests {
    use crate::neural::{builtin, weights::Weights};

    /// Composed base + adapter contour decoder matches PyTorch (ml/asmr_ml/export_multi.py).
    #[test]
    fn contour_decoder_matches_pytorch() {
        for (species, bytes) in [
            ("chaffinch", &include_bytes!("../../assets/neural/parity/chaffinch.asnw")[..]),
            ("blackbird", &include_bytes!("../../assets/neural/parity/blackbird.asnw")[..]),
        ] {
            let m = builtin(species).unwrap();
            let r = Weights::parse(bytes).unwrap();
            let toks = &r.tensor("contour.tok").unwrap().data;
            let z = &r.tensor("contour.z").unwrap().data;
            let want = &r.tensor("contour.out").unwrap().data;
            let n = want.len() / toks.len();
            let zd = m.song.z_dim;
            let mut st = m.song.new_state();
            for (i, &t) in toks.iter().enumerate() {
                let got = m.song.decode_raw(&mut st, t as usize, &z[i * zd..(i + 1) * zd]);
                for (a, e) in got.iter().zip(&want[i * n..(i + 1) * n]) {
                    assert!((a - e).abs() < 2e-3 * (1.0 + e.abs()), "{species}: {a} vs {e}");
                }
            }
        }
    }
}
