//! Loudspeaker-bed output for an external binaural renderer.
//!
//! Apple keeps the listener's personalised HRTF (and its own AirPods head tracking) inside its
//! renderers: AUSpatialMixer, AVAudioEnvironmentNode and PHASE. Their HRTF can't be read, but
//! they render a multichannel bed. In that mode the order-3 bus is decoded to Dolby's 7.1.4 layout
//! (`kAudioChannelLayoutTag_Atmos_7_1_4`, channel order L R C LFE Ls Rs Rls Rrs Vhl Vhr Ltr Rtr)
//! and AUSpatialMixer renders it binaurally.
//!
//! Decoder: AllRAD-style (Zotter & Frank 2012). The bus is decoded with max-rE weights onto 240
//! virtual directions (a near-uniform grid, where the sampling decoder is well behaved). Each
//! virtual direction is panned to its three nearest loudspeakers with energy-normalised
//! inverse-angle weights. The layout has no speakers below the horizon, so below-horizon
//! directions add an imaginary nadir speaker whose share is spread over the seven ear-level
//! speakers (energy kept, elevation lost, as in AllRAD). Output is scaled to unit mean energy
//! gain over the sphere.

use crate::ambisonics::{self, max_re_weights, order_of};
use crate::binaural::fibonacci_sphere;
use crate::engine::BLOCK;
use crate::math::Vec3;

/// Bed channels (7.1.4).
pub const CHANNELS: usize = 12;
const CHANNELS_BED: usize = CHANNELS;

/// LFE channel index (kept silent: the bed carries full-range signals).
pub const LFE: usize = 3;

/// (azimuth, elevation) in degrees, +azimuth = left, in `kAudioChannelLayoutTag_Atmos_7_1_4`
/// order. The LFE entry is unused.
pub const LAYOUT: [(f32, f32); CHANNELS_BED] = [
    (30.0, 0.0),    // L
    (-30.0, 0.0),   // R
    (0.0, 0.0),     // C
    (0.0, -90.0),   // LFE (unused)
    (90.0, 0.0),    // Ls (side)
    (-90.0, 0.0),   // Rs
    (150.0, 0.0),   // Rls (rear)
    (-150.0, 0.0),  // Rrs
    (45.0, 45.0),   // Vhl (top front)
    (-45.0, 45.0),  // Vhr
    (135.0, 45.0),  // Ltr (top rear)
    (-135.0, 45.0), // Rtr
];

fn dir(az: f32, el: f32) -> Vec3 {
    let (a, e) = (az.to_radians(), el.to_radians());
    Vec3::new(e.cos() * a.cos(), e.cos() * a.sin(), e.sin())
}

pub struct BedDecoder {
    /// [speaker][sh channel]
    m: [[f32; ambisonics::CHANNELS]; CHANNELS_BED],
}

impl BedDecoder {
    pub fn new() -> Self {
        let speakers: Vec<(usize, Vec3)> = LAYOUT.iter().enumerate().filter(|(i, _)| *i != LFE).map(|(i, &(a, e))| (i, dir(a, e))).collect();
        let ear_level: Vec<usize> = LAYOUT.iter().enumerate().filter(|(i, l)| *i != LFE && l.1 == 0.0).map(|(i, _)| i).collect();
        let nadir = Vec3::new(0.0, 0.0, -1.0);
        let virt = fibonacci_sphere(240);
        let w = max_re_weights();
        let mut m = [[0.0f64; ambisonics::CHANNELS]; CHANNELS_BED];
        for v in &virt {
            // Candidates: real speakers plus the imaginary nadir.
            let mut cand: Vec<(Option<usize>, f32)> = speakers.iter().map(|(i, d)| (Some(*i), d.dot(*v).clamp(-1.0, 1.0).acos())).collect();
            cand.push((None, nadir.dot(*v).clamp(-1.0, 1.0).acos()));
            cand.sort_by(|a, b| a.1.total_cmp(&b.1));
            let near = &cand[..3];
            let raw: Vec<f32> = near.iter().map(|(_, a)| 1.0 / (a + 0.05)).collect();
            let norm = raw.iter().map(|g| g * g).sum::<f32>().sqrt();
            let mut y = [0.0f32; ambisonics::CHANNELS];
            ambisonics::encode(*v, &mut y);
            for ((spk, _), g) in near.iter().zip(&raw) {
                let g = (g / norm) as f64;
                let targets: Vec<(usize, f64)> = match spk {
                    Some(s) => vec![(*s, g)],
                    None => ear_level.iter().map(|s| (*s, g / (ear_level.len() as f64).sqrt())).collect(),
                };
                for (s, gs) in targets {
                    for ch in 0..ambisonics::CHANNELS {
                        m[s][ch] += gs * (y[ch] * w[order_of(ch)]) as f64 / virt.len() as f64;
                    }
                }
            }
        }
        // Unit mean energy gain for plane waves over the sphere.
        let test = fibonacci_sphere(500);
        let mut e = 0.0f64;
        for d in &test {
            let mut y = [0.0f32; ambisonics::CHANNELS];
            ambisonics::encode(*d, &mut y);
            for row in &m {
                let s: f64 = row.iter().zip(&y).map(|(a, b)| a * *b as f64).sum();
                e += s * s;
            }
        }
        let scale = (test.len() as f64 / e).sqrt();
        let m = m.map(|row| row.map(|v| (v * scale) as f32));
        Self { m }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn gains(&self, d: Vec3) -> [f32; CHANNELS_BED] {
        let mut y = [0.0f32; ambisonics::CHANNELS];
        ambisonics::encode(d, &mut y);
        core::array::from_fn(|s| self.m[s].iter().zip(&y).map(|(a, b)| a * b).sum())
    }

    pub fn decode(&self, bus: &[[f32; BLOCK]; CHANNELS_SH], len: usize, out: &mut [[f32; BLOCK]; CHANNELS_BED]) {
        for (s, o) in out.iter_mut().enumerate() {
            o[..len].fill(0.0);
            if s == LFE {
                continue;
            }
            for ch in 0..CHANNELS_SH {
                let g = self.m[s][ch];
                if g != 0.0 {
                    for (v, x) in o[..len].iter_mut().zip(&bus[ch][..len]) {
                        *v += g * x;
                    }
                }
            }
        }
    }
}

const CHANNELS_SH: usize = ambisonics::CHANNELS;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sources_land_on_the_right_speakers() {
        let d = BedDecoder::new();
        let loudest = |v: Vec3| {
            let g = d.gains(v);
            (0..CHANNELS_BED).max_by(|a, b| g[*a].abs().total_cmp(&g[*b].abs())).unwrap()
        };
        assert_eq!(loudest(dir(30.0, 0.0)), 0);
        assert_eq!(loudest(dir(-90.0, 0.0)), 5);
        assert_eq!(loudest(dir(-140.0, 40.0)), 11);
        assert_eq!(d.gains(dir(10.0, 20.0))[LFE], 0.0);
    }

    #[test]
    fn energy_is_roughly_uniform() {
        let d = BedDecoder::new();
        let mut lo = f32::MAX;
        let mut hi = 0.0f32;
        for v in fibonacci_sphere(200).iter().filter(|v| v.z > -0.3) {
            let e: f32 = d.gains(*v).iter().map(|g| g * g).sum();
            lo = lo.min(e);
            hi = hi.max(e);
        }
        assert!(10.0 * (hi / lo).log10() < 3.0, "energy spread {lo}..{hi}");
    }
}
