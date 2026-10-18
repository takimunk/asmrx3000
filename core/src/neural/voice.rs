//! DDSP voice: per-frame controls -> network -> harmonic amplitudes + noise band gains ->
//! additive oscillator + band-pass noise bank, at any sample rate.
//!
//! Mirrors `ml/asmr_ml/voice.py` and `ml/asmr_ml/dsp.py`. Frame i is at t = i * hop_s and every
//! synthesis parameter is linearly interpolated from frame i to frame i + 1, so the voice runs
//! one frame (5 ms) ahead of its output.

use core::f32::consts::TAU;

use super::layers::{leaky_relu, sigmoid, Gru, GruState, Linear};
use super::weights::{Error, Weights};
use crate::rng::Rng;

/// Controls for one frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub f0: f32,
    /// dB, same scale as training (`prep.features`).
    pub loud: f32,
    /// 0 pure whistle .. 1 buzzy.
    pub effort: f32,
}

/// Shared, immutable model (one per species, any number of voices use it).
pub struct VoiceModel {
    inp: Linear,
    gru: Gru,
    out1: Linear,
    out2: Linear,
    norm: [f32; 4],
    leak: f32,
    noise_bias: f32,
    pub n_harm: usize,
    pub hop_s: f32,
    centers: Vec<f32>,
    q: f32,
}

#[inline]
fn exp_sigmoid(x: f32) -> f32 {
    2.0 * sigmoid(x).powf(core::f32::consts::LN_10) + 1e-7
}

impl VoiceModel {
    pub fn load(w: &Weights) -> Result<Self, Error> {
        let n = w.get("cfg.voice_norm", &[4])?;
        let centers = w.get("cfg.band_centers", &[usize::MAX])?.to_vec();
        let m = Self {
            inp: Linear::load(w, "voice.inp")?,
            gru: Gru::load(w, "voice.gru")?,
            out1: Linear::load(w, "voice.out1")?,
            out2: Linear::load(w, "voice.out2")?,
            norm: [n[0], n[1], n[2], n[3]],
            leak: w.scalar("cfg.leak")?,
            noise_bias: w.scalar("cfg.noise_bias")?,
            n_harm: w.scalar("cfg.n_harm")? as usize,
            hop_s: w.scalar("cfg.hop_s")?,
            q: w.scalar("cfg.band_q")?,
            centers,
        };
        let outs = 1 + m.n_harm + m.centers.len();
        if m.inp.n_in != 3 || m.out2.n_out != outs || m.out1.n_in != m.gru.hidden + 3 {
            return Err(Error("voice: layer sizes do not fit together".into()));
        }
        Ok(m)
    }

    pub fn n_bands(&self) -> usize {
        self.centers.len()
    }

    pub fn params(&self) -> usize {
        [&self.inp, &self.out1, &self.out2, &self.gru.ih, &self.gru.hh].iter().map(|l| l.w.len() + l.b.len()).sum()
    }

    /// Network for one frame: writes harmonic amplitudes and noise gains into `st.cur_*`.
    fn frame(&self, st: &mut VoiceState, f: Frame, sr: f32) {
        let x = [
            (f.f0.max(1.0).log2() - self.norm[0]) / self.norm[1],
            (f.loud - self.norm[2]) / self.norm[3],
            f.effort,
        ];
        self.inp.forward(&x, &mut st.z);
        st.z.iter_mut().for_each(|v| *v = leaky_relu(*v, self.leak));
        self.gru.step(&st.z, &mut st.gru);
        let h = self.gru.hidden;
        st.cat[..h].copy_from_slice(&st.gru.h);
        st.cat[h..h + 3].copy_from_slice(&x);
        self.out1.forward(&st.cat, &mut st.hid);
        st.hid.iter_mut().for_each(|v| *v = leaky_relu(*v, self.leak));
        self.out2.forward(&st.hid, &mut st.y);

        let amp = exp_sigmoid(st.y[0]);
        let k = self.n_harm;
        // Softmax over the harmonics below 0.45 sr (same mask as training).
        let mut mx = f32::NEG_INFINITY;
        for i in 0..k {
            if f.f0 * (i + 1) as f32 >= 0.45 * sr {
                st.y[1 + i] = -1e4;
            }
            mx = mx.max(st.y[1 + i]);
        }
        let mut sum = 0.0;
        for i in 0..k {
            let e = (st.y[1 + i] - mx).exp();
            st.cur_harm[i] = e;
            sum += e;
        }
        for v in &mut st.cur_harm[..k] {
            *v *= amp / sum;
        }
        for (g, y) in st.cur_noise.iter_mut().zip(&st.y[1 + k..]) {
            *g = exp_sigmoid(y - self.noise_bias);
        }
        st.cur_f0 = f.f0;
    }

    pub fn new_state(&self, sr: f32, seed: u64) -> VoiceState {
        let nb = self.centers.len();
        let mut bands = Vec::with_capacity(nb);
        for &fc in &self.centers {
            // Bands above Nyquist (low sample rates) are silenced.
            bands.push(if fc < 0.49 * sr { Biquad::bandpass(fc, self.q, sr) } else { Biquad::default() });
        }
        VoiceState {
            sr,
            gru: self.gru.state(),
            z: vec![0.0; self.inp.n_out],
            cat: vec![0.0; self.gru.hidden + 3],
            hid: vec![0.0; self.out1.n_out],
            y: vec![0.0; self.out2.n_out],
            cur_harm: vec![0.0; self.n_harm],
            cur_noise: vec![0.0; nb],
            harm: vec![0.0; self.n_harm],
            d_harm: vec![0.0; self.n_harm],
            noise: vec![0.0; nb],
            d_noise: vec![0.0; nb],
            cur_f0: 0.0,
            f0: 0.0,
            d_f0: 0.0,
            bands,
            phase: 0.0,
            frame_pos: 0.0,
            frame_inc: 1.0 / (sr * self.hop_s),
            rng: Rng::new(seed),
            active: false,
        }
    }

    /// Starts a new utterance: clears the recurrent state and primes the first frame.
    pub fn start(&self, st: &mut VoiceState, first: Frame) {
        st.gru.reset();
        for b in &mut st.bands {
            b.s1 = 0.0;
            b.s2 = 0.0;
        }
        self.frame(st, first, st.sr);
        st.harm.copy_from_slice(&st.cur_harm);
        st.noise.copy_from_slice(&st.cur_noise);
        st.f0 = st.cur_f0;
        st.frame_pos = 1.0;
        st.active = true;
    }

    /// Renders `out` (overwrites). `next` supplies the following frame's controls whenever the
    /// voice crosses a frame boundary.
    pub fn render(&self, st: &mut VoiceState, out: &mut [f32], mut next: impl FnMut() -> Frame) {
        let sr = st.sr;
        let inv_sr = 1.0 / sr;
        let nyq = 0.45 * sr;
        let k = self.n_harm;
        const SQRT3: f32 = 1.732_050_8;
        for o in out.iter_mut() {
            if st.frame_pos >= 1.0 {
                st.frame_pos -= 1.0;
                // Land exactly on the previous frame's values, then aim at the new frame.
                for i in 0..k {
                    st.harm[i] = st.cur_harm[i];
                }
                st.noise.copy_from_slice(&st.cur_noise);
                st.f0 = st.cur_f0;
                let f = next();
                self.frame(st, f, sr);
                let (t, inc) = (st.frame_pos, st.frame_inc);
                for i in 0..k {
                    let d = st.cur_harm[i] - st.harm[i];
                    st.harm[i] += d * t;
                    st.d_harm[i] = d * inc;
                }
                for i in 0..st.noise.len() {
                    let d = st.cur_noise[i] - st.noise[i];
                    st.noise[i] += d * t;
                    st.d_noise[i] = d * inc;
                }
                let d = st.cur_f0 - st.f0;
                st.f0 += d * t;
                st.d_f0 = d * inc;
            }

            // Harmonics: sin(k theta) by the Chebyshev recurrence from one sin/cos.
            st.phase += st.f0 * inv_sr;
            st.phase -= st.phase.floor();
            let (s1, c1) = (TAU * st.phase).sin_cos();
            let two_c = 2.0 * c1;
            let (mut s_prev, mut s_k) = (0.0f32, s1);
            let mut h = 0.0;
            for i in 0..k {
                if st.f0 * (i + 1) as f32 >= nyq {
                    break;
                }
                h += st.harm[i] * s_k;
                let s_next = two_c * s_k - s_prev;
                s_prev = s_k;
                s_k = s_next;
            }

            // Noise: one white sample through every band, weighted by the band gains.
            let x = st.rng.bipolar() * SQRT3;
            let mut n = 0.0;
            for (b, g) in st.bands.iter_mut().zip(&st.noise) {
                n += g * b.process(x);
            }

            *o = h + n;

            for i in 0..k {
                st.harm[i] += st.d_harm[i];
            }
            for (g, d) in st.noise.iter_mut().zip(&st.d_noise) {
                *g += d;
            }
            st.f0 += st.d_f0;
            st.frame_pos += st.frame_inc;
        }
    }

    /// Runs the network for one frame and returns (harmonic amplitudes, noise gains).
    /// For parity tests against PyTorch; the audio path uses `render`.
    pub fn frame_outputs<'a>(&self, st: &'a mut VoiceState, f: Frame) -> (&'a [f32], &'a [f32]) {
        self.frame(st, f, st.sr);
        (&st.cur_harm, &st.cur_noise)
    }
}

/// Per-voice mutable state. Allocated once at creation.
pub struct VoiceState {
    sr: f32,
    gru: GruState,
    z: Vec<f32>,
    cat: Vec<f32>,
    hid: Vec<f32>,
    y: Vec<f32>,
    cur_harm: Vec<f32>,
    cur_noise: Vec<f32>,
    harm: Vec<f32>,
    d_harm: Vec<f32>,
    noise: Vec<f32>,
    d_noise: Vec<f32>,
    cur_f0: f32,
    f0: f32,
    d_f0: f32,
    bands: Vec<Biquad>,
    phase: f32,
    frame_pos: f32,
    frame_inc: f32,
    rng: Rng,
    pub active: bool,
}

/// Transposed direct form II; band-pass with the unit-RMS normalisation folded into b.
#[derive(Clone, Copy, Default)]
struct Biquad {
    b0: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    s1: f32,
    s2: f32,
}

impl Biquad {
    /// RBJ band-pass (0 dB peak), scaled so unit white noise comes out at unit RMS:
    /// the noise power gain of this filter is alpha / (1 + alpha).
    fn bandpass(fc: f32, q: f32, sr: f32) -> Self {
        let w = TAU * fc / sr;
        let alpha = w.sin() / (2.0 * q);
        let a0 = 1.0 + alpha;
        let norm = (a0 / alpha).sqrt();
        Self {
            b0: alpha / a0 * norm,
            b2: -alpha / a0 * norm,
            a1: -2.0 * w.cos() / a0,
            a2: (1.0 - alpha) / a0,
            s1: 0.0,
            s2: 0.0,
        }
    }

    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.s1;
        self.s1 = -self.a1 * y + self.s2;
        self.s2 = self.b2 * x - self.a2 * y;
        y
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::neural::weights::Weights;

    /// Frame outputs match PyTorch (ml/asmr_ml/export.py writes the reference).
    #[test]
    fn matches_pytorch() {
        let model = crate::neural::builtin("chaffinch").unwrap();
        let r = Weights::parse(include_bytes!("../../assets/neural/parity/chaffinch.asnw")).unwrap();
        let get = |n: &str| r.tensor(n).unwrap().data.clone();
        let (f0, loud, eff) = (get("f0"), get("loud"), get("effort"));
        let (harm, noise) = (get("harm"), get("noise"));
        let (k, b) = (model.voice.n_harm, model.voice.n_bands());
        let mut st = model.voice.new_state(48_000.0, 1);
        let mut worst = 0.0f32;
        for t in 0..f0.len() {
            let (h, n) = model.voice.frame_outputs(&mut st, Frame { f0: f0[t], loud: loud[t], effort: eff[t] });
            for (a, e) in h.iter().zip(&harm[t * k..(t + 1) * k]).chain(n.iter().zip(&noise[t * b..(t + 1) * b])) {
                worst = worst.max((a - e).abs() / (e.abs() + 1e-3));
            }
        }
        assert!(worst < 1e-3, "max relative error {worst}");
    }

    #[test]
    fn bandpass_is_unit_rms_for_white_noise() {
        let mut rng = Rng::new(3);
        for fc in [800.0, 4000.0, 16000.0] {
            let mut b = Biquad::bandpass(fc, 6.33, 48_000.0);
            let n = 400_000;
            let mut e = 0.0f64;
            for _ in 0..n {
                let y = b.process(rng.bipolar() * 1.732_050_8);
                e += (y as f64).powi(2);
            }
            let rms = (e / n as f64).sqrt();
            assert!((rms - 1.0).abs() < 0.05, "fc {fc}: rms {rms}");
        }
    }
}
