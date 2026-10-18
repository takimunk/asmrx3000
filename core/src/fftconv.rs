//! Uniformly partitioned overlap-save convolution of the 16-channel Ambisonic bus with the
//! 32 SH-domain binaural filters (block B = 64, FFT 128). ~15x cheaper than direct-form FIRs
//! for 256-tap decoders. Internally block-based: input is collected into B-sample frames, so
//! the decoder adds a fixed latency of B samples (1.3 ms at 48 kHz) and accepts any `len`.

use crate::ambisonics::CHANNELS;
use crate::engine::BLOCK;

const B: usize = BLOCK;
const N: usize = 2 * B;
const BINS: usize = N / 2 + 1;

#[derive(Clone, Copy, Default)]
struct Cf {
    re: f32,
    im: f32,
}

/// Fixed-size radix-2 complex FFT with precomputed twiddles (no allocation after `new`).
struct Fft {
    tw: Vec<Cf>,
    rev: Vec<usize>,
}

impl Fft {
    fn new() -> Self {
        let bits = N.trailing_zeros();
        let rev = (0..N).map(|i| i.reverse_bits() >> (usize::BITS - bits)).collect();
        let tw = (0..N / 2)
            .map(|k| {
                let a = -2.0 * core::f64::consts::PI * k as f64 / N as f64;
                Cf { re: a.cos() as f32, im: a.sin() as f32 }
            })
            .collect();
        Self { tw, rev }
    }

    /// Forward transform; `inverse` conjugates twiddles (no 1/N scaling).
    fn run(&self, x: &mut [Cf; N], inverse: bool) {
        for i in 0..N {
            let j = self.rev[i];
            if i < j {
                x.swap(i, j);
            }
        }
        let mut len = 2;
        while len <= N {
            let half = len / 2;
            let stride = N / len;
            for start in (0..N).step_by(len) {
                for k in 0..half {
                    let w = self.tw[k * stride];
                    let wi = if inverse { -w.im } else { w.im };
                    let a = x[start + k];
                    let b = x[start + k + half];
                    let t = Cf { re: b.re * w.re - b.im * wi, im: b.re * wi + b.im * w.re };
                    x[start + k] = Cf { re: a.re + t.re, im: a.im + t.im };
                    x[start + k + half] = Cf { re: a.re - t.re, im: a.im - t.im };
                }
            }
            len <<= 1;
        }
    }
}

pub struct PartitionedDecoder {
    fft: Fft,
    parts: usize,
    /// Filter spectra [ch][ear][part][bin], real and imaginary parts apart (vectorises).
    h_re: Vec<f32>,
    h_im: Vec<f32>,
    /// Frequency-domain delay line: [part][ch][bin], ring over parts.
    fdl_re: Vec<f32>,
    fdl_im: Vec<f32>,
    fdl_pos: usize,
    /// Time-domain input history (previous + current block) per channel.
    frame: [[f32; N]; CHANNELS],
    fill: usize,
    out: [[f32; B]; 2],
    out_pos: usize,
}

impl PartitionedDecoder {
    /// `filters`: layout [ch][ear][t] forward time, `taps` long.
    pub fn new(filters: &[f32], taps: usize) -> Self {
        let fft = Fft::new();
        let parts = taps.div_ceil(B);
        let mut h_re = vec![0.0f32; CHANNELS * 2 * parts * BINS];
        let mut h_im = vec![0.0f32; CHANNELS * 2 * parts * BINS];
        for ch in 0..CHANNELS {
            for ear in 0..2 {
                let f = &filters[(ch * 2 + ear) * taps..(ch * 2 + ear + 1) * taps];
                for p in 0..parts {
                    let mut buf = [Cf::default(); N];
                    for t in 0..B {
                        if let Some(v) = f.get(p * B + t) {
                            buf[t].re = *v;
                        }
                    }
                    fft.run(&mut buf, false);
                    let o = ((ch * 2 + ear) * parts + p) * BINS;
                    // Fold the 1/N of the inverse transform into the filter.
                    for k in 0..BINS {
                        h_re[o + k] = buf[k].re / N as f32;
                        h_im[o + k] = buf[k].im / N as f32;
                    }
                }
            }
        }
        Self {
            fft,
            parts,
            h_re,
            h_im,
            fdl_re: vec![0.0; parts * CHANNELS * BINS],
            fdl_im: vec![0.0; parts * CHANNELS * BINS],
            fdl_pos: 0,
            frame: [[0.0; N]; CHANNELS],
            fill: 0,
            out: [[0.0; B]; 2],
            out_pos: 0,
        }
    }

    pub fn reset(&mut self) {
        self.fdl_re.fill(0.0);
        self.fdl_im.fill(0.0);
        self.frame = [[0.0; N]; CHANNELS];
        self.out = [[0.0; B]; 2];
    }

    pub fn process(&mut self, bus: &[[f32; BLOCK]; CHANNELS], len: usize, left: &mut [f32], right: &mut [f32]) {
        let mut i = 0;
        while i < len {
            let n = (len - i).min(B - self.fill);
            for ch in 0..CHANNELS {
                self.frame[ch][B + self.fill..B + self.fill + n].copy_from_slice(&bus[ch][i..i + n]);
            }
            left[i..i + n].copy_from_slice(&self.out[0][self.out_pos..self.out_pos + n]);
            right[i..i + n].copy_from_slice(&self.out[1][self.out_pos..self.out_pos + n]);
            self.fill += n;
            self.out_pos += n;
            i += n;
            if self.fill == B {
                self.block();
                self.fill = 0;
                self.out_pos = 0;
            }
        }
    }

    fn block(&mut self) {
        // Spectra of the 2B-sample frames, two real channels per complex FFT.
        self.fdl_pos = (self.fdl_pos + self.parts - 1) % self.parts;
        let base = self.fdl_pos * CHANNELS * BINS;
        let mut z = [Cf::default(); N];
        for pair in 0..CHANNELS / 2 {
            let (a, b) = (2 * pair, 2 * pair + 1);
            for t in 0..N {
                z[t] = Cf { re: self.frame[a][t], im: self.frame[b][t] };
            }
            self.fft.run(&mut z, false);
            for k in 0..BINS {
                let zk = z[k];
                let zn = z[(N - k) % N];
                // A = (Z[k] + conj Z[N-k]) / 2, B = (Z[k] - conj Z[N-k]) / (2i)
                self.fdl_re[base + a * BINS + k] = 0.5 * (zk.re + zn.re);
                self.fdl_im[base + a * BINS + k] = 0.5 * (zk.im - zn.im);
                self.fdl_re[base + b * BINS + k] = 0.5 * (zk.im + zn.im);
                self.fdl_im[base + b * BINS + k] = -0.5 * (zk.re - zn.re);
            }
        }
        for ch in 0..CHANNELS {
            self.frame[ch].copy_within(B..N, 0);
        }

        // Multiply-accumulate over channels and partitions.
        let mut acc_re = [[0.0f32; BINS]; 2];
        let mut acc_im = [[0.0f32; BINS]; 2];
        for p in 0..self.parts {
            let slot = (self.fdl_pos + p) % self.parts;
            for ch in 0..CHANNELS {
                let xo = (slot * CHANNELS + ch) * BINS;
                let xr = &self.fdl_re[xo..xo + BINS];
                let xi = &self.fdl_im[xo..xo + BINS];
                for ear in 0..2 {
                    let ho = ((ch * 2 + ear) * self.parts + p) * BINS;
                    let hr = &self.h_re[ho..ho + BINS];
                    let hi = &self.h_im[ho..ho + BINS];
                    let (ar, ai) = (&mut acc_re[ear], &mut acc_im[ear]);
                    for k in 0..BINS {
                        ar[k] += xr[k] * hr[k] - xi[k] * hi[k];
                        ai[k] += xr[k] * hi[k] + xi[k] * hr[k];
                    }
                }
            }
        }
        let acc = |ear: usize, k: usize| Cf { re: acc_re[ear][k], im: acc_im[ear][k] };

        // One inverse FFT for both ears: W = L + i R (both real in time).
        for k in 0..N {
            let (l, r) = if k < BINS {
                (acc(0, k), acc(1, k))
            } else {
                let m = N - k;
                (Cf { re: acc(0, m).re, im: -acc(0, m).im }, Cf { re: acc(1, m).re, im: -acc(1, m).im })
            };
            z[k] = Cf { re: l.re - r.im, im: l.im + r.re };
        }
        self.fft.run(&mut z, true);
        for t in 0..B {
            self.out[0][t] = z[B + t].re;
            self.out[1][t] = z[B + t].im;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_direct_convolution() {
        let taps = 200;
        let mut filters = vec![0.0f32; CHANNELS * 2 * taps];
        let mut seed = 1u32;
        let mut rnd = || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 8) as f32 / 16_777_216.0 - 0.5
        };
        for v in filters.iter_mut() {
            *v = rnd();
        }
        let total = 700;
        let input: Vec<[f32; CHANNELS]> = (0..total).map(|_| core::array::from_fn(|_| rnd())).collect();
        let mut dec = PartitionedDecoder::new(&filters, taps);
        let mut out_l = Vec::new();
        let mut out_r = Vec::new();
        let mut i = 0;
        // Irregular chunk sizes exercise the internal buffering.
        for chunk in [13usize, 64, 1, 50, 64, 37].iter().cycle() {
            if i >= total {
                break;
            }
            let n = (*chunk).min(total - i);
            let mut bus = [[0.0f32; BLOCK]; CHANNELS];
            for j in 0..n {
                for ch in 0..CHANNELS {
                    bus[ch][j] = input[i + j][ch];
                }
            }
            let mut l = vec![0.0f32; n];
            let mut r = vec![0.0f32; n];
            dec.process(&bus, n, &mut l, &mut r);
            out_l.extend(l);
            out_r.extend(r);
            i += n;
        }
        for t in B..total {
            let src = t - B; // fixed latency of one block
            let mut want = [0.0f64; 2];
            for ear in 0..2 {
                for ch in 0..CHANNELS {
                    for k in 0..taps.min(src + 1) {
                        want[ear] += filters[(ch * 2 + ear) * taps + k] as f64 * input[src - k][ch] as f64;
                    }
                }
            }
            assert!((out_l[t] as f64 - want[0]).abs() < 1e-3, "t={t}: {} vs {}", out_l[t], want[0]);
            assert!((out_r[t] as f64 - want[1]).abs() < 1e-3, "t={t}: {} vs {}", out_r[t], want[1]);
        }
    }
}
