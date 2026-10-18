//! The few layers our models use, matching PyTorch semantics. Weights are owned `Vec<f32>`
//! loaded once; `forward`/`step` write into caller-provided buffers and never allocate.

use super::weights::{Error, Weights};

/// `y = W x + b`, `W` is `[out, in]` row-major (PyTorch `nn.Linear`).
pub struct Linear {
    pub w: Vec<f32>,
    pub b: Vec<f32>,
    pub n_in: usize,
    pub n_out: usize,
}

impl Linear {
    pub fn load(w: &Weights, name: &str) -> Result<Self, Error> {
        let dims = w.dims(&format!("{name}.weight"))?.to_vec();
        if dims.len() != 2 {
            return Err(Error(format!("{name}.weight: not 2-D")));
        }
        let (n_out, n_in) = (dims[0], dims[1]);
        Ok(Self {
            w: w.get(&format!("{name}.weight"), &[n_out, n_in])?.to_vec(),
            b: w.get(&format!("{name}.bias"), &[n_out])?.to_vec(),
            n_in,
            n_out,
        })
    }

    #[inline]
    pub fn forward(&self, x: &[f32], y: &mut [f32]) {
        debug_assert!(x.len() >= self.n_in && y.len() >= self.n_out);
        let x = &x[..self.n_in];
        for (o, (row, b)) in y.iter_mut().zip(self.w.chunks_exact(self.n_in).zip(&self.b)) {
            *o = b + dot(row, x);
        }
    }
}

/// Dot product written as 8 independent accumulators so LLVM vectorises it (NEON / SSE / wasm
/// simd128) without `unsafe` or intrinsics.
#[inline]
pub fn dot(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len().min(b.len());
    let (a, b) = (&a[..n], &b[..n]);
    let mut acc = [0.0f32; 8];
    let ca = a.chunks_exact(8);
    let cb = b.chunks_exact(8);
    let (ra, rb) = (ca.remainder(), cb.remainder());
    for (x, y) in ca.zip(cb) {
        for i in 0..8 {
            acc[i] += x[i] * y[i];
        }
    }
    let mut s = (acc[0] + acc[4]) + (acc[1] + acc[5]) + ((acc[2] + acc[6]) + (acc[3] + acc[7]));
    for (x, y) in ra.iter().zip(rb) {
        s += x * y;
    }
    s
}

#[inline]
pub fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

#[inline]
pub fn leaky_relu(x: f32, slope: f32) -> f32 {
    if x >= 0.0 {
        x
    } else {
        x * slope
    }
}

/// Single-layer GRU, PyTorch gate order (r, z, n):
/// r = s(W_ir x + b_ir + W_hr h + b_hr), z = s(W_iz x + b_iz + W_hz h + b_hz),
/// n = tanh(W_in x + b_in + r (W_hn h + b_hn)), h' = (1 - z) n + z h.
pub struct Gru {
    pub ih: Linear,
    pub hh: Linear,
    pub hidden: usize,
}

/// Per-voice GRU state and scratch.
pub struct GruState {
    pub h: Vec<f32>,
    gi: Vec<f32>,
    gh: Vec<f32>,
}

impl Gru {
    pub fn load(w: &Weights, name: &str) -> Result<Self, Error> {
        let get = |s: &str| w.get(&format!("{name}.{s}"), &[usize::MAX, usize::MAX]);
        let d = w.dims(&format!("{name}.weight_hh_l0"))?.to_vec();
        let hidden = d[1];
        let din = w.dims(&format!("{name}.weight_ih_l0"))?[1];
        if d[0] != 3 * hidden {
            return Err(Error(format!("{name}: weight_hh_l0 {d:?} is not [3H, H]")));
        }
        let ih = Linear {
            w: get("weight_ih_l0")?.to_vec(),
            b: w.get(&format!("{name}.bias_ih_l0"), &[3 * hidden])?.to_vec(),
            n_in: din,
            n_out: 3 * hidden,
        };
        let hh = Linear {
            w: get("weight_hh_l0")?.to_vec(),
            b: w.get(&format!("{name}.bias_hh_l0"), &[3 * hidden])?.to_vec(),
            n_in: hidden,
            n_out: 3 * hidden,
        };
        if ih.w.len() != 3 * hidden * din {
            return Err(Error(format!("{name}: weight_ih_l0 size")));
        }
        Ok(Self { ih, hh, hidden })
    }

    pub fn state(&self) -> GruState {
        GruState { h: vec![0.0; self.hidden], gi: vec![0.0; 3 * self.hidden], gh: vec![0.0; 3 * self.hidden] }
    }

    pub fn step(&self, x: &[f32], s: &mut GruState) {
        let n = self.hidden;
        self.ih.forward(x, &mut s.gi);
        self.hh.forward(&s.h, &mut s.gh);
        for i in 0..n {
            let r = sigmoid(s.gi[i] + s.gh[i]);
            let z = sigmoid(s.gi[n + i] + s.gh[n + i]);
            let c = (s.gi[2 * n + i] + r * s.gh[2 * n + i]).tanh();
            s.h[i] = (1.0 - z) * c + z * s.h[i];
        }
    }
}

impl GruState {
    pub fn reset(&mut self) {
        self.h.iter_mut().for_each(|v| *v = 0.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dot_matches_naive() {
        let a: Vec<f32> = (0..37).map(|i| (i as f32 * 0.37).sin()).collect();
        let b: Vec<f32> = (0..37).map(|i| (i as f32 * 0.11).cos()).collect();
        let naive: f32 = a.iter().zip(&b).map(|(x, y)| x * y).sum();
        assert!((dot(&a, &b) - naive).abs() < 1e-5);
    }

    #[test]
    fn gru_identity_when_update_gate_saturates() {
        // Huge z bias -> z = 1 -> h' = h.
        let hidden = 2;
        let mut bih = vec![0.0; 6];
        bih[2] = 100.0;
        bih[3] = 100.0;
        let g = Gru {
            ih: Linear { w: vec![0.5; 6], b: bih, n_in: 1, n_out: 6 },
            hh: Linear { w: vec![0.1; 12], b: vec![0.0; 6], n_in: 2, n_out: 6 },
            hidden,
        };
        let mut s = g.state();
        s.h.copy_from_slice(&[0.3, -0.7]);
        g.step(&[1.0], &mut s);
        assert!((s.h[0] - 0.3).abs() < 1e-6 && (s.h[1] + 0.7).abs() < 1e-6);
    }
}
