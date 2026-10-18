//! Measured HRTFs and the Ambisonics -> binaural decoder built from them.
//!
//! Asset: "AHRT" v1 (written by tools/hrtf/sofa_to_ahrt.py), little endian:
//!   "AHRT", u32 version, u32 sample_rate, u32 num_dirs, u32 taps, f32 scale,
//!   u32 name_len, name (UTF-8, zero-padded to 4 bytes),
//!   num_dirs x [f32 x, y, z] (engine frame: +x forward, +y left, +z up),
//!   num_dirs x [i16 left[taps], i16 right[taps]].
//! The default set (SADIE II KU100, Apache-2.0, see assets/hrtf/README.md) is embedded.
//!
//! Decoder: magnitude least squares (MagLS; Schoerkhuber, Zaunschirm & Hoeldrich, DAGA 2018)
//! with the diffuse-field covariance constraint (Zaunschirm, Schoerkhuber & Hoeldrich, JASA
//! 143(6) 2018). Below f_c = N c / (2 pi r_head) ~ 1.9 kHz an order-3 SH expansion represents
//! HRTF phase (ITD) accurately and the decoder is a plain LS fit; above it, phase cannot be
//! matched at order 3 anyway, so only magnitudes are fitted (phase is carried over from the
//! previous bin), which removes the high-frequency roll-off and blur of LS/mode matching.
//! The 2x2 covariance constraint then restores diffuse-field ear energies and interaural
//! coherence, which matters for the forest's reverberant field.

use crate::ambisonics::{self, CHANNELS, ORDER};
use crate::math::Vec3;
use crate::num::{fft, pinv, C64};
use core::f64::consts::PI;

pub static DEFAULT_SET: &[u8] = include_bytes!("../../assets/hrtf/sadie2_ku100_48k.ahrt");
/// Measurement radius of the default set (SADIE II: 1.2 m), the NFC reference radius.
pub const DEFAULT_RADIUS: f64 = 1.2;
/// SH-domain decoder filter length.
pub const DECODER_TAPS: usize = 256;

pub struct HrirSet {
    pub sample_rate: f32,
    pub name: String,
    pub dirs: Vec<Vec3>,
    pub taps: usize,
    /// dir-major: [d][ear][t]
    pub data: Vec<f32>,
}

impl HrirSet {
    pub fn parse(bytes: &[u8]) -> Result<Self, &'static str> {
        let mut p = 0usize;
        let mut take = |n: usize| -> Result<&[u8], &'static str> {
            let s = bytes.get(p..p + n).ok_or("truncated AHRT file")?;
            p += n;
            Ok(s)
        };
        if take(4)? != b"AHRT" {
            return Err("not an AHRT file");
        }
        let u32_at = |s: &[u8]| u32::from_le_bytes([s[0], s[1], s[2], s[3]]);
        if u32_at(take(4)?) != 1 {
            return Err("unsupported AHRT version");
        }
        let sample_rate = u32_at(take(4)?) as f32;
        let num_dirs = u32_at(take(4)?) as usize;
        let taps = u32_at(take(4)?) as usize;
        let scale = f32::from_le_bytes(take(4)?.try_into().unwrap());
        let name_len = u32_at(take(4)?) as usize;
        if num_dirs < 16 || taps == 0 || taps > 4096 || num_dirs > 100_000 || !(8000.0..=384_000.0).contains(&sample_rate) {
            return Err("implausible AHRT header");
        }
        let name = String::from_utf8_lossy(take(name_len)?).into_owned();
        take((4 - name_len % 4) % 4)?;
        let mut dirs = Vec::with_capacity(num_dirs);
        for _ in 0..num_dirs {
            let s = take(12)?;
            let f = |o: usize| f32::from_le_bytes([s[o], s[o + 1], s[o + 2], s[o + 3]]);
            let d = Vec3::new(f(0), f(4), f(8));
            let l = d.length();
            if !(0.5..2.0).contains(&l) {
                return Err("bad direction vector");
            }
            dirs.push(d * (1.0 / l));
        }
        let raw = take(num_dirs * 2 * taps * 2)?;
        let data = raw.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 * scale).collect();
        Ok(Self { sample_rate, name, dirs, taps, data })
    }

    pub fn ir(&self, dir: usize, ear: usize) -> &[f32] {
        let o = (dir * 2 + ear) * self.taps;
        &self.data[o..o + self.taps]
    }

    /// Band-limited resampling of every IR to `rate` (init time only).
    pub fn resampled(&self, rate: f32) -> HrirSet {
        if (rate - self.sample_rate).abs() < 0.5 {
            return HrirSet { sample_rate: self.sample_rate, name: self.name.clone(), dirs: self.dirs.clone(), taps: self.taps, data: self.data.clone() };
        }
        let ratio = rate as f64 / self.sample_rate as f64;
        let taps = ((self.taps as f64 * ratio).ceil() as usize).max(8);
        let cutoff = ratio.min(1.0) * 0.95;
        let half = 24.0;
        let mut data = vec![0.0f32; self.dirs.len() * 2 * taps];
        for d in 0..self.dirs.len() {
            for ear in 0..2 {
                let src = self.ir(d, ear);
                let dst = &mut data[(d * 2 + ear) * taps..(d * 2 + ear + 1) * taps];
                for (n, o) in dst.iter_mut().enumerate() {
                    let t = n as f64 / ratio; // position in source samples
                    let lo = ((t - half).floor().max(0.0)) as usize;
                    let hi = ((t + half).ceil() as usize).min(src.len() - 1);
                    let mut acc = 0.0;
                    for (m, &s) in src.iter().enumerate().take(hi + 1).skip(lo) {
                        let x = t - m as f64;
                        let sinc = if x.abs() < 1e-9 { 1.0 } else { (PI * cutoff * x).sin() / (PI * cutoff * x) };
                        let w = 0.5 + 0.5 * (PI * x / half).cos();
                        acc += s as f64 * cutoff * sinc * w;
                    }
                    *o = acc as f32;
                }
            }
        }
        HrirSet { sample_rate: rate, name: self.name.clone(), dirs: self.dirs.clone(), taps, data }
    }
}

/// Hermitian square root of a 2x2 positive semi-definite matrix [[a, b], [conj b, d]].
fn sqrt2(m: [[C64; 2]; 2]) -> [[C64; 2]; 2] {
    let det = (m[0][0] * m[1][1] - m[0][1] * m[1][0]).re.max(0.0);
    let s = det.sqrt();
    let t = (m[0][0].re + m[1][1].re + 2.0 * s).max(1e-30).sqrt();
    [
        [(m[0][0] + s) * (1.0 / t), m[0][1] * (1.0 / t)],
        [m[1][0] * (1.0 / t), (m[1][1] + s) * (1.0 / t)],
    ]
}

fn inv2(m: [[C64; 2]; 2]) -> [[C64; 2]; 2] {
    let det = m[0][0] * m[1][1] - m[0][1] * m[1][0];
    let id = if det.abs() < 1e-30 { C64::ZERO } else { det.inv() };
    [[m[1][1] * id, -(m[0][1] * id)], [-(m[1][0] * id), m[0][0] * id]]
}

fn mul2(a: [[C64; 2]; 2], b: [[C64; 2]; 2]) -> [[C64; 2]; 2] {
    let mut r = [[C64::ZERO; 2]; 2];
    for i in 0..2 {
        for j in 0..2 {
            r[i][j] = a[i][0] * b[0][j] + a[i][1] * b[1][j];
        }
    }
    r
}

/// SH-domain binaural filters, layout [ch][ear][t] (forward time), `DECODER_TAPS` long.
pub fn magls_filters(set: &HrirSet, sample_rate: f32) -> Vec<f32> {
    let set = set.resampled(sample_rate);
    let fs = sample_rate as f64;
    let nfft = (2 * set.taps.max(DECODER_TAPS)).next_power_of_two();
    let nb = nfft / 2 + 1;
    let q = set.dirs.len();

    let y: Vec<Vec<f64>> = set
        .dirs
        .iter()
        .map(|d| {
            let mut s = [0.0f32; CHANNELS];
            ambisonics::encode(*d, &mut s);
            s.iter().map(|v| *v as f64).collect()
        })
        .collect();
    let p = pinv(&y, None, 1e-6); // 16 x Q

    // HRTF spectra per direction and ear.
    let mut spec = vec![C64::ZERO; q * 2 * nb];
    let mut buf = vec![C64::ZERO; nfft];
    for d in 0..q {
        for ear in 0..2 {
            buf.iter_mut().for_each(|v| *v = C64::ZERO);
            for (t, v) in set.ir(d, ear).iter().enumerate() {
                buf[t] = C64::new(*v as f64, 0.0);
            }
            fft(&mut buf, false);
            spec[(d * 2 + ear) * nb..(d * 2 + ear + 1) * nb].copy_from_slice(&buf[..nb]);
        }
    }
    let h = |d: usize, ear: usize, b: usize| spec[(d * 2 + ear) * nb + b];

    let f_cut = ORDER as f64 * 343.0 / (2.0 * PI * 0.0875);
    let b_cut = (f_cut * nfft as f64 / fs).ceil() as usize;
    // Decoder spectra: [ear][ch][b]
    let mut dec = vec![C64::ZERO; 2 * CHANNELS * nb];
    let mut target = vec![C64::ZERO; q];
    for ear in 0..2 {
        for b in 0..nb {
            if b < b_cut {
                for (d, t) in target.iter_mut().enumerate() {
                    *t = h(d, ear, b);
                }
            } else {
                for (d, t) in target.iter_mut().enumerate() {
                    let mut prev = C64::ZERO;
                    for ch in 0..CHANNELS {
                        prev = prev + dec[(ear * CHANNELS + ch) * nb + b - 1] * y[d][ch];
                    }
                    *t = C64::from_polar(h(d, ear, b).abs(), prev.arg());
                }
            }
            for ch in 0..CHANNELS {
                let mut s = C64::ZERO;
                for d in 0..q {
                    s = s + target[d] * p[ch][d];
                }
                dec[(ear * CHANNELS + ch) * nb + b] = s;
            }
        }
    }

    // Diffuse-field covariance constraint above f_c.
    for b in b_cut..nb {
        let mut r = [[C64::ZERO; 2]; 2];
        let mut rh = [[C64::ZERO; 2]; 2];
        for d in 0..q {
            let hv = [h(d, 0, b), h(d, 1, b)];
            let mut est = [C64::ZERO; 2];
            for ear in 0..2 {
                for ch in 0..CHANNELS {
                    est[ear] = est[ear] + dec[(ear * CHANNELS + ch) * nb + b] * y[d][ch];
                }
            }
            for i in 0..2 {
                for j in 0..2 {
                    r[i][j] = r[i][j] + hv[i].conj() * hv[j];
                    rh[i][j] = rh[i][j] + est[i].conj() * est[j];
                }
            }
        }
        let m = mul2(inv2(sqrt2(rh)), sqrt2(r));
        for ch in 0..CHANNELS {
            let c = [dec[ch * nb + b], dec[(CHANNELS + ch) * nb + b]];
            for ear in 0..2 {
                dec[(ear * CHANNELS + ch) * nb + b] = c[0] * m[0][ear] + c[1] * m[1][ear];
            }
        }
    }

    // Back to time domain, truncate with a short fade-out.
    let mut out = vec![0.0f32; CHANNELS * 2 * DECODER_TAPS];
    for ch in 0..CHANNELS {
        for ear in 0..2 {
            for b in 0..nb {
                let v = dec[(ear * CHANNELS + ch) * nb + b];
                buf[b] = v;
                if b > 0 && b < nfft / 2 {
                    buf[nfft - b] = v.conj();
                }
            }
            fft(&mut buf, true);
            let o = &mut out[(ch * 2 + ear) * DECODER_TAPS..(ch * 2 + ear + 1) * DECODER_TAPS];
            let fade = 32;
            for t in 0..DECODER_TAPS {
                let w = if t >= DECODER_TAPS - fade {
                    0.5 + 0.5 * (PI * (t + fade + 1 - DECODER_TAPS) as f64 / (fade + 1) as f64).cos()
                } else {
                    1.0
                };
                o[t] = (buf[t].re * w) as f32;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_set_parses() {
        let s = HrirSet::parse(DEFAULT_SET).unwrap();
        assert_eq!(s.sample_rate, 48_000.0);
        assert!(s.dirs.len() > 500);
        assert_eq!(s.data.len(), s.dirs.len() * 2 * s.taps);
        // Left ear louder for a source on the left.
        let (i, _) = s.dirs.iter().enumerate().max_by(|a, b| a.1.y.total_cmp(&b.1.y)).unwrap();
        let e = |ear| s.ir(i, ear).iter().map(|v| v * v).sum::<f32>();
        assert!(e(0) > 10.0 * e(1));
    }

    #[test]
    fn rejects_garbage() {
        assert!(HrirSet::parse(b"nope").is_err());
        assert!(HrirSet::parse(&DEFAULT_SET[..100]).is_err());
    }

    #[test]
    fn resampling_keeps_low_frequency_level() {
        let s = HrirSet::parse(DEFAULT_SET).unwrap();
        let r = s.resampled(44_100.0);
        let dc = |set: &HrirSet| set.ir(0, 0).iter().sum::<f32>();
        assert!((dc(&r) * 48_000.0 / 44_100.0 - dc(&s)).abs() < 0.05 * dc(&s).abs().max(0.1));
    }
}
