//! Clips: recorded (or generated) sounds placed and moved in the scene.
//!
//! A clip is a set of mono files (one source each, as dry as possible: a whale phrase, a
//! dolphin's whistles, a deep drone). The engine spatialises them like any other emitter, so
//! a flat file gets the same space as a procedural model: HRTF, distance, air, Doppler, head
//! tracking. Two parts:
//! * [`ClipPlayer`]: the generator. Plays one file per cue (`events`) or an endless texture
//!   (`loop`: random file and offset, cross-faded before the end, so it never repeats exactly).
//! * [`Choreo`]: moves clip entities every control block ([`Motion`]: fixed, fly-by, drift past
//!   a listener who swims forward, escort, orbit, a wave sweeping through the head and the
//!   screen, a pull into the screen) and cues their players.
//!
//! Levels: every file is normalised to unit loudness at load, `level_db` sets the sound
//! pressure at 1 m (RMS, dB SPL) like the calibrated physical models.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::engine::SourceKind;
use crate::math::Vec3;
use crate::mix::SoundClass;
use crate::rng::Rng;
use crate::scene::{object_flags, EntityDesc, SceneDesc, World};
use crate::sources::{Controls, Generator, ModelConfig};

// ---------------------------------------------------------------------------------------
// Description
// ---------------------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClipDesc {
    #[serde(default)]
    pub id: Option<u32>,
    /// User-facing group ("whales", "current"): clips of a group share the panel's controls
    /// ([`ClipGroups`]). Clips without one are only reached by the scene-wide macros.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// WAV files (PCM 16/24/32 or float, any rate, mixed to mono). Relative paths resolve
    /// against the scene's `assets`, then `$ASMR_SOUNDS_DIR`, then the repository's
    /// assets/sounds.
    #[serde(default)]
    pub files: Vec<String>,
    /// A physical model instead of files (model + preset + params, as for sources): the
    /// choreography moves it and fades it in and out like a clip.
    #[serde(default)]
    pub sound: Option<crate::sources::SoundDesc>,
    /// Echo on the clip's own signal (files only).
    #[serde(default)]
    pub echo: Option<Echo>,
    #[serde(default)]
    pub class: ClipClass,
    /// Sound pressure at 1 m, dB SPL (RMS while sounding).
    #[serde(default = "default_level")]
    pub level_db: f32,
    #[serde(default)]
    pub playback: Playback,
    /// Seconds to the first event (or appearance).
    #[serde(default = "default_first")]
    pub first: f32,
    /// `first` exactly, not drawn up to 20 % later (a timed accent in a scripted scene).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub exact: bool,
    /// Seconds between events (or appearances), drawn uniformly.
    #[serde(default = "default_interval")]
    pub interval: [f32; 2],
    /// Playback rate per event (pitch and tempo together), drawn uniformly.
    #[serde(default = "unit_range")]
    pub rate: [f32; 2],
    /// Decorrelated emitters (1..8): a ring of radius `spread` around the position, or a line
    /// across a sweeping wave.
    #[serde(default = "one_u32")]
    pub emitters: u32,
    #[serde(default)]
    pub spread: f32,
    /// Ring of emitters around the forward axis (a tunnel the listener moves through)
    /// instead of a horizontal ring.
    #[serde(default)]
    pub tunnel: bool,
    /// Fade in / out, seconds, at appearances, departures and the ends of a pass.
    #[serde(default = "default_fade")]
    pub fade: f32,
    pub motion: Motion,
}

/// Two feedback delays (`delay` and `delay`·φ), damped above `damp_hz`: the long, soft
/// repeats heard from calls far away underwater (or the cinematic version of them).
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Echo {
    /// Seconds to the first repeat (0.05..1.5).
    pub delay: f32,
    /// Repeat level, 0..0.85.
    pub feedback: f32,
    /// Echo level relative to the dry call, 0..2.
    #[serde(default = "one_f32")]
    pub mix: f32,
    #[serde(default = "default_damp")]
    pub damp_hz: f32,
}

fn one_f32() -> f32 {
    1.0
}
fn default_damp() -> f32 {
    2500.0
}

fn default_level() -> f32 {
    60.0
}
fn default_first() -> f32 {
    5.0
}
fn default_interval() -> [f32; 2] {
    [30.0, 90.0]
}
fn unit_range() -> [f32; 2] {
    [1.0, 1.0]
}
fn one_u32() -> u32 {
    1
}
fn default_fade() -> f32 {
    1.5
}

/// What the mix layer treats the clip as (calibration, attention, focus group).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClipClass {
    /// Continuous enveloping bed (ocean, space hum): the masking layer.
    #[default]
    Bed,
    /// Airy, slowly moving textures (cosmic swells, drones).
    Texture,
    /// Animals: event-like, can be lifted as soloists.
    Creature,
    /// Rare one-off events (a comet, a distant burst).
    Event,
}

impl ClipClass {
    pub fn kinds(self) -> (SourceKind, SoundClass) {
        match self {
            ClipClass::Bed => (SourceKind::Water, SoundClass::Water),
            ClipClass::Texture => (SourceKind::Wind, SoundClass::Wind),
            ClipClass::Creature => (SourceKind::Insect, SoundClass::Insects),
            ClipClass::Event => (SourceKind::Fire, SoundClass::Fire),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Playback {
    /// One file from its start per event.
    #[default]
    Events,
    /// Endless texture while active.
    Loop,
}

/// Coordinates are the engine's: head at the origin, +x forward (the screen), +y left, +z up.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Motion {
    /// Stays put; events (or the loop) play there.
    Fixed { position: [f32; 3] },
    /// Each event travels from `from` to `to` (or into the screen) in `duration` seconds
    /// (default: the file's length); both ends are jittered by `jitter` metres.
    Pass {
        from: [f32; 3],
        #[serde(default)]
        to: Option<[f32; 3]>,
        #[serde(default)]
        to_screen: bool,
        #[serde(default)]
        duration: Option<f32>,
        #[serde(default)]
        jitter: f32,
    },
    /// The listener moves forward at `speed`: the source appears `ahead` metres in front, off
    /// to the side by `lateral` (min..max, either side) at `height`, slides past and fades out
    /// `behind` metres back; reappears after `interval`.
    Drift {
        speed: f32,
        #[serde(default = "default_ahead")]
        ahead: f32,
        #[serde(default = "default_behind")]
        behind: f32,
        lateral: [f32; 2],
        #[serde(default)]
        height: [f32; 2],
        /// Own swimming speed along the drift, m/s (+ = overtakes the listener slowly).
        #[serde(default)]
        own_speed: f32,
    },
    /// Keeps company: `offset` from the listener, wandering by `wander` metres.
    Escort {
        offset: [f32; 3],
        #[serde(default = "default_wander")]
        wander: f32,
    },
    /// Circles `center` at `radius`, `height` above it, one lap per `period` seconds
    /// (counter-clockwise seen from above; a negative period circles the other way).
    Orbit {
        #[serde(default)]
        center: [f32; 3],
        radius: f32,
        #[serde(default)]
        height: f32,
        period: f32,
    },
    /// A long wavefront: from `from_x` (behind) to `to_x` (far in front) in `duration`.
    /// Wide behind, it narrows to the screen's width as it reaches the screen plane and
    /// widens again beyond it, as if it went through the frame and spread out.
    Sweep {
        #[serde(default = "default_sweep_from")]
        from_x: f32,
        #[serde(default = "default_sweep_to")]
        to_x: f32,
        duration: f32,
        #[serde(default = "default_sweep_width")]
        width: f32,
        #[serde(default)]
        height: f32,
    },
    /// Comes from `from` along a curve (`curve` = sideways bulge as a share of the distance)
    /// and disappears into the centre of the screen.
    Portal {
        from: [f32; 3],
        duration: f32,
        #[serde(default = "default_curve")]
        curve: f32,
    },
    /// Follows keyframes `[t, x, y, z]` or `[t, x, y, z, gain]` (seconds from the scene's
    /// start, metres about the head, gain ×, default 1) along a smooth Catmull–Rom curve:
    /// a sound that travels a planned route (around the head, down an arm). Played once and
    /// held at the last point, or looped with `repeat`. Present from the start.
    Path {
        points: Vec<Vec<f32>>,
        #[serde(default)]
        repeat: bool,
    },
}

fn default_ahead() -> f32 {
    40.0
}
fn default_behind() -> f32 {
    25.0
}
fn default_wander() -> f32 {
    1.0
}
fn default_sweep_from() -> f32 {
    -40.0
}
fn default_sweep_to() -> f32 {
    60.0
}
fn default_sweep_width() -> f32 {
    16.0
}
fn default_curve() -> f32 {
    0.35
}

impl ClipDesc {
    pub fn validate(&self, id: u32) -> Result<(), String> {
        let bad = |what: &str| Err(format!("clip {id}: {what}"));
        if self.files.is_empty() == self.sound.is_none() {
            return bad("needs either `files` or `sound`");
        }
        if let Some(e) = self.echo {
            if !((0.05..=1.5).contains(&e.delay) && (0.0..=0.85).contains(&e.feedback) && (0.0..=2.0).contains(&e.mix) && e.damp_hz > 100.0) {
                return bad("echo: delay 0.05..1.5 s, feedback 0..0.85, mix 0..2, damp_hz > 100");
            }
        }
        if !(0.0..=140.0).contains(&self.level_db) {
            return bad("level_db must be 0..140");
        }
        if !(self.interval[0] >= 0.0 && self.interval[1] >= self.interval[0]) {
            return bad("interval must be [min, max] with 0 <= min <= max");
        }
        if !(self.rate[0] > 0.1 && self.rate[1] >= self.rate[0] && self.rate[1] <= 4.0) {
            return bad("rate must be [min, max] within 0.1..4");
        }
        if self.emitters == 0 || self.emitters > 8 {
            return bad("emitters must be 1..8");
        }
        if !(self.fade >= 0.0 && self.spread >= 0.0 && self.first >= 0.0) {
            return bad("fade, spread and first must be >= 0");
        }
        match &self.motion {
            Motion::Pass { to: None, to_screen: false, .. } => bad("pass needs `to` or `to_screen`"),
            Motion::Drift { speed, ahead, behind, .. } if !(*speed > 0.0 && *ahead > 0.0 && *behind > 0.0) => {
                bad("drift needs speed, ahead, behind > 0")
            }
            Motion::Orbit { period, radius, .. } if !(period.abs() > 0.5 && *radius > 0.0) => bad("orbit needs radius > 0, |period| > 0.5"),
            Motion::Sweep { duration, from_x, to_x, .. } if !(*duration > 0.5 && to_x > from_x) => bad("sweep needs duration > 0.5, to_x > from_x"),
            Motion::Portal { duration, .. } if *duration <= 0.1 => bad("portal needs duration > 0.1"),
            Motion::Path { points, .. } => {
                if points.len() < 2 || points.iter().any(|p| !(4..=5).contains(&p.len()) || p.iter().any(|v| !v.is_finite())) {
                    return bad("path needs 2+ points of [t, x, y, z] or [t, x, y, z, gain]");
                }
                if points.windows(2).any(|w| w[1][0] <= w[0][0]) || points[0][0] < 0.0 {
                    return bad("path times must start at >= 0 and increase");
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Continuous presence (no gaps between appearances).
    fn continuous(&self) -> bool {
        matches!(self.motion, Motion::Fixed { .. } | Motion::Escort { .. } | Motion::Orbit { .. } | Motion::Path { .. })
    }
}

/// Position and gain at `t` seconds along path keyframes (validated: 2+, increasing times).
/// Catmull–Rom between keyframes (no corners: a moving source is easier to follow when it
/// turns smoothly), gain linear.
fn path_at(points: &[Vec<f32>], t: f32, repeat: bool) -> (Vec3, f32) {
    let n = points.len();
    let (t0, t1) = (points[0][0], points[n - 1][0]);
    let t = if repeat && t > t1 { t0 + (t - t0).rem_euclid((t1 - t0).max(1e-3)) } else { t.clamp(t0, t1) };
    let k = points.windows(2).position(|w| t <= w[1][0]).unwrap_or(n - 2);
    let at = |i: isize| {
        let p = &points[i.clamp(0, n as isize - 1) as usize];
        Vec3::new(p[1], p[2], p[3])
    };
    let gain = |i: usize| points[i].get(4).copied().unwrap_or(1.0);
    let u = ((t - points[k][0]) / (points[k + 1][0] - points[k][0]).max(1e-6)).clamp(0.0, 1.0);
    let (p0, p1, p2, p3) = (at(k as isize - 1), at(k as isize), at(k as isize + 1), at(k as isize + 2));
    let (u2, u3) = (u * u, u * u * u);
    let pos = (p1 * 2.0 + (p2 - p0) * u + (p0 * 2.0 - p1 * 5.0 + p2 * 4.0 - p3) * u2 + (p1 * 3.0 - p0 - p2 * 3.0 + p3) * u3) * 0.5;
    (pos, gain(k) + (gain(k + 1) - gain(k)) * u)
}

// ---------------------------------------------------------------------------------------
// Audio files
// ---------------------------------------------------------------------------------------

/// Decoded files at the engine's sample rate, each normalised to unit loudness.
pub struct ClipBank {
    pub files: Vec<Vec<f32>>,
    pub names: Vec<String>,
    pub sample_rate: f32,
}

impl core::fmt::Debug for ClipBank {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "ClipBank({:?})", self.names)
    }
}

impl ClipBank {
    /// Loads and resamples. Allocates and reads files: call off the audio thread.
    pub fn load(files: &[String], assets: Option<&str>, sample_rate: f32) -> Result<ClipBank, String> {
        let mut out = Vec::with_capacity(files.len());
        for f in files {
            let path = resolve(f, assets).ok_or_else(|| format!("sound file not found: {f}"))?;
            let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            let (mono, rate) = decode_wav(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
            let mut s = resample(&mono, rate, sample_rate);
            normalise(&mut s, sample_rate);
            if s.len() < (0.05 * sample_rate) as usize {
                return Err(format!("{}: too short", path.display()));
            }
            out.push(s);
        }
        Ok(ClipBank { files: out, names: files.to_vec(), sample_rate })
    }

    /// Playing time of file `i`; a model-driven clip (no files) counts as 10 s.
    pub fn seconds(&self, i: usize) -> f32 {
        self.files.get(i).map_or(10.0, |f| f.len() as f32 / self.sample_rate)
    }
}

fn resolve(f: &str, assets: Option<&str>) -> Option<std::path::PathBuf> {
    use std::path::{Path, PathBuf};
    let p = Path::new(f);
    if p.is_absolute() {
        return p.exists().then(|| p.to_path_buf());
    }
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(a) = assets {
        roots.push(PathBuf::from(a));
    }
    if let Ok(d) = std::env::var("ASMR_SOUNDS_DIR") {
        roots.push(PathBuf::from(d));
    }
    roots.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../assets/sounds"));
    roots.into_iter().map(|r| r.join(p)).find(|c| c.exists())
}

/// Minimal RIFF/WAVE reader: PCM 8/16/24/32 int and 32/64 float, plain or extensible,
/// channels averaged to mono. Returns the samples and their rate.
pub fn decode_wav(b: &[u8]) -> Result<(Vec<f32>, f32), String> {
    let u16le = |o: usize| u16::from_le_bytes([b[o], b[o + 1]]);
    let u32le = |o: usize| u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
    if b.len() < 12 || &b[0..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        return Err("not a RIFF/WAVE file".into());
    }
    let mut fmt: Option<(u16, u16, u32, u16)> = None;
    let mut data: Option<&[u8]> = None;
    let mut o = 12;
    while o + 8 <= b.len() {
        let id = &b[o..o + 4];
        let len = u32le(o + 4) as usize;
        let body = o + 8;
        let end = (body + len).min(b.len());
        if id == b"fmt " && len >= 16 {
            let mut tag = u16le(body);
            if tag == 0xFFFE && len >= 26 {
                tag = u16le(body + 24);
            }
            fmt = Some((tag, u16le(body + 2), u32le(body + 4), u16le(body + 14)));
        } else if id == b"data" {
            data = Some(&b[body..end]);
        }
        o = body + len + (len & 1);
    }
    let (tag, channels, rate, bits) = fmt.ok_or("no fmt chunk")?;
    let data = data.ok_or("no data chunk")?;
    if channels == 0 || rate == 0 {
        return Err("bad format".into());
    }
    let bytes = (bits as usize).div_ceil(8);
    let frame = bytes * channels as usize;
    let n = data.len() / frame;
    let sample = |p: &[u8]| -> Result<f32, String> {
        Ok(match (tag, bits) {
            (1, 8) => (p[0] as f32 - 128.0) / 128.0,
            (1, 16) => i16::from_le_bytes([p[0], p[1]]) as f32 / 32768.0,
            (1, 24) => (i32::from_le_bytes([0, p[0], p[1], p[2]]) >> 8) as f32 / 8_388_608.0,
            (1, 32) => i32::from_le_bytes([p[0], p[1], p[2], p[3]]) as f32 / 2_147_483_648.0,
            (3, 32) => f32::from_le_bytes([p[0], p[1], p[2], p[3]]),
            (3, 64) => f64::from_le_bytes([p[0], p[1], p[2], p[3], p[4], p[5], p[6], p[7]]) as f32,
            _ => return Err(format!("unsupported sample format (tag {tag}, {bits} bit)")),
        })
    };
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let mut acc = 0.0;
        for c in 0..channels as usize {
            let at = i * frame + c * bytes;
            acc += sample(&data[at..at + bytes])?;
        }
        out.push(acc / channels as f32);
    }
    Ok((out, rate as f32))
}

/// Windowed-sinc resampling (8 zero crossings each side, Blackman), low-passed at the lower
/// Nyquist. Load time only.
fn resample(x: &[f32], from: f32, to: f32) -> Vec<f32> {
    if (from - to).abs() < 0.5 {
        return x.to_vec();
    }
    let ratio = to as f64 / from as f64;
    let n = (x.len() as f64 * ratio) as usize;
    let cutoff = ratio.min(1.0);
    let half = 8.0 / cutoff;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f64 / ratio;
        let lo = (t - half).ceil().max(0.0) as usize;
        let hi = ((t + half).floor() as usize).min(x.len().saturating_sub(1));
        let mut acc = 0.0f64;
        for (j, &xj) in x.iter().enumerate().take(hi + 1).skip(lo) {
            let d = j as f64 - t;
            let s = if d.abs() < 1e-9 { 1.0 } else { (core::f64::consts::PI * d * cutoff).sin() / (core::f64::consts::PI * d * cutoff) };
            let w = 0.42 + 0.5 * (core::f64::consts::PI * d / half).cos() + 0.08 * (2.0 * core::f64::consts::PI * d / half).cos();
            acc += xj as f64 * s * w * cutoff;
        }
        out.push(acc as f32);
    }
    out
}

/// Scales to unit "sounding" RMS: the mean energy of the louder half of 50 ms windows, so
/// silence between calls does not inflate the gain. Removes DC.
fn normalise(x: &mut [f32], sr: f32) {
    let mean = x.iter().map(|&v| v as f64).sum::<f64>() / x.len().max(1) as f64;
    for v in x.iter_mut() {
        *v -= mean as f32;
    }
    let w = ((0.05 * sr) as usize).max(1);
    let mut e: Vec<f64> = x.chunks(w).map(|c| c.iter().map(|&v| (v as f64) * (v as f64)).sum::<f64>() / c.len() as f64).collect();
    e.sort_by(|a, b| b.partial_cmp(a).unwrap_or(core::cmp::Ordering::Equal));
    let top = &e[..e.len().div_ceil(2)];
    let rms = (top.iter().sum::<f64>() / top.len().max(1) as f64).sqrt();
    if rms > 1e-9 {
        let g = (1.0 / rms) as f32;
        for v in x.iter_mut() {
            *v *= g;
        }
    }
}

// ---------------------------------------------------------------------------------------
// Player (audio thread)
// ---------------------------------------------------------------------------------------

#[derive(Clone, Copy, Default)]
struct Voice {
    on: bool,
    file: usize,
    pos: f64,
    /// Rate chosen at the cue; the group's pitch multiplies it live.
    rate: f64,
    /// Fade position 0..1 (output weight sin(g·π/2)), moving by `step` per sample.
    g: f32,
    step: f32,
}

/// Plays a [`ClipBank`] as directed by the cue fields of [`Controls`].
pub struct ClipPlayer {
    bank: Arc<ClipBank>,
    looped: bool,
    voices: [Voice; 2],
    cur: usize,
    last_cue: u32,
    pending: bool,
    env: f32,
    gain: f32,
    xfade: f32,
    rng: Rng,
    echo: Option<EchoLine>,
    /// Smoothed live pitch factor (the group's `pitch`).
    pitch: f32,
}

/// The two damped feedback delays of an [`Echo`]. Buffers allocated at build.
struct EchoLine {
    cfg: Echo,
    buf: [Vec<f32>; 2],
    len: [usize; 2],
    pos: [usize; 2],
    lp: [f32; 2],
    a: f32,
}

impl EchoLine {
    fn new(cfg: Echo, sr: f32) -> Self {
        let l0 = ((cfg.delay * sr) as usize).max(1);
        let l1 = ((cfg.delay * 1.618 * sr) as usize).max(1);
        Self {
            cfg,
            buf: [vec![0.0; l0], vec![0.0; l1]],
            len: [l0, l1],
            pos: [0, 0],
            lp: [0.0; 2],
            a: 1.0 - (-core::f32::consts::TAU * cfg.damp_hz / sr).exp(),
        }
    }

    #[inline]
    fn tick(&mut self, x: f32) -> f32 {
        let mut wet = 0.0;
        for k in 0..2 {
            let y = self.buf[k][self.pos[k]];
            self.lp[k] += (y - self.lp[k]) * self.a;
            self.buf[k][self.pos[k]] = x * 0.5 + self.lp[k] * self.cfg.feedback;
            self.pos[k] = (self.pos[k] + 1) % self.len[k];
            wet += y;
        }
        x + wet * self.cfg.mix
    }
}

impl ClipPlayer {
    pub fn new(bank: Arc<ClipBank>, looped: bool, gain: f32, seed: u64) -> Self {
        Self { bank, looped, voices: [Voice::default(); 2], cur: 0, last_cue: 0, pending: false, env: 0.0, gain, xfade: 2.0, rng: Rng::new(seed), echo: None, pitch: 1.0 }
    }

    /// Adds an echo (allocates its delay lines: call off the audio thread).
    pub fn with_echo(mut self, echo: Option<Echo>) -> Self {
        let sr = self.bank.sample_rate;
        self.echo = echo.map(|e| EchoLine::new(e, sr));
        self
    }

    /// Cross-fade length for a file: at most a quarter of its playing time, so short files
    /// still sound mostly at full level.
    fn xfade_for(&self, file: usize, rate: f64) -> f32 {
        let file = file % self.bank.files.len();
        let secs = self.bank.files[file].len() as f64 / self.bank.sample_rate as f64 / rate;
        self.xfade.min((0.25 * secs) as f32).max(0.01)
    }

    fn start(&mut self, file: usize, offset: f32, rate: f32, fade_in: f32) {
        let sr = self.bank.sample_rate;
        let file = file % self.bank.files.len();
        let len = self.bank.files[file].len() as f64;
        let old = self.cur;
        if self.voices[old].on {
            // The old voice fades out over the same time the new one fades in.
            self.voices[old].step = -1.0 / (fade_in.max(0.02) * sr);
        }
        self.cur = 1 - old;
        self.voices[self.cur] = Voice {
            on: true,
            file,
            pos: (offset.clamp(0.0, 0.95) as f64 * len).floor(),
            rate: rate.clamp(0.1, 4.0) as f64,
            g: 0.0,
            step: 1.0 / (fade_in.max(0.005) * sr),
        };
    }
}

impl Generator for ClipPlayer {
    fn render(&mut self, out: &mut [f32], c: &Controls) {
        if c.cue != self.last_cue {
            self.last_cue = c.cue;
            self.pending = true;
        }
        if self.pending && c.activity > 0.02 {
            self.pending = false;
            let fade = if self.looped { self.xfade_for(c.cue_file as usize, c.cue_rate.max(0.1) as f64) } else { 0.01 };
            self.start(c.cue_file as usize, c.cue_offset, c.cue_rate.max(0.1), fade);
        }
        let sr = self.bank.sample_rate;
        if self.looped && c.activity > 0.02 {
            // Endless texture: before the current voice runs out, cross-fade into another file
            // at a random offset.
            let v = self.voices[self.cur];
            let left = if v.on { (self.bank.files[v.file].len() as f64 - v.pos) / v.rate } else { 0.0 };
            if !v.on || left < (self.xfade_for(v.file, v.rate) * sr) as f64 {
                let n = self.bank.files.len();
                let file = (self.rng.next_u32() as usize) % n;
                let offset = self.rng.range(0.0, 0.7);
                let rate = if v.on { v.rate as f32 } else { c.cue_rate.max(0.1) };
                let fade = if v.on { self.xfade_for(v.file, v.rate).min(self.xfade_for(file, rate as f64)) } else { 0.5 };
                self.start(file, offset, rate, fade);
            }
        }
        let a_env = 1.0 - (-1.0 / (0.05 * sr)).exp();
        let target = c.clip_env.clamp(0.0, 4.0);
        let tail = 0.03 * sr;
        let pitch_target = if c.clip_pitch > 0.0 { c.clip_pitch.clamp(0.25, 4.0) } else { 1.0 };
        let a_pitch = 1.0 - (-1.0 / (0.1 * sr)).exp();
        for o in out.iter_mut() {
            self.env += (target - self.env) * a_env;
            self.pitch += (pitch_target - self.pitch) * a_pitch;
            let mut acc = 0.0;
            for v in self.voices.iter_mut() {
                if !v.on {
                    continue;
                }
                let data = &self.bank.files[v.file];
                let i = v.pos as usize;
                if i + 1 >= data.len() {
                    v.on = false;
                    continue;
                }
                let fr = (v.pos - i as f64) as f32;
                let s = data[i] + (data[i + 1] - data[i]) * fr;
                // Events end with a short fade so a cut-off file never clicks.
                let remain = (data.len() - i) as f32 / (v.rate as f32 * self.pitch);
                let end = if remain < tail { remain / tail } else { 1.0 };
                acc += s * (v.g * core::f32::consts::FRAC_PI_2).sin() * end;
                v.g += v.step;
                if v.g >= 1.0 {
                    v.g = 1.0;
                    v.step = 0.0;
                } else if v.g <= 0.0 {
                    v.on = false;
                }
                v.pos += v.rate * self.pitch as f64;
            }
            let dry = acc * self.env;
            *o = match self.echo.as_mut() {
                Some(e) => e.tick(dry),
                None => dry,
            } * self.gain;
        }
    }

    fn busy(&self) -> bool {
        !self.looped && self.voices.iter().any(|v| v.on)
    }
}

// ---------------------------------------------------------------------------------------
// Groups (user controls, any thread)
// ---------------------------------------------------------------------------------------

/// Parameter ids of a clip group (`asmr_set_clip_group_param`).
pub mod group_param {
    /// Level of the group, dB, −30..+10 [0].
    pub const LEVEL: u32 = 0;
    /// How often its things happen, ×0..3 [1].
    pub const EVENTS: u32 = 1;
    /// Speed of its travelling sounds, ×0..3 [1].
    pub const MOTION: u32 = 2;
    /// Distance from the listener (positions, offsets, radii), ×0.3..3 [1].
    pub const DISTANCE: u32 = 3;
    /// Playback rate of its files, ×0.5..2 [1] (not for physical models).
    pub const PITCH: u32 = 4;
    /// First model parameter (water: flow speed ×0..2.5 [1]; plasma: hiss centre ×0.4..2.5 [1]).
    pub const MODEL_A: u32 = 5;
    /// Second model parameter (water: turbulence ×0..4 [1]; plasma: whistlers per minute added, 0..60 [0]).
    pub const MODEL_B: u32 = 6;
    pub const COUNT: usize = 7;
}

/// Which physical model a group's procedural clips use (the group then has MODEL_A/B).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GroupModel {
    #[default]
    None = 0,
    Water = 1,
    Plasma = 2,
}

/// (min, max, default) of group parameter `p` for a group of model `m`.
pub fn group_param_range(p: u32, m: GroupModel) -> (f32, f32, f32) {
    use group_param::*;
    match (p, m) {
        (LEVEL, _) => (-30.0, 10.0, 0.0),
        (EVENTS, _) | (MOTION, _) => (0.0, 3.0, 1.0),
        (DISTANCE, _) => (0.3, 3.0, 1.0),
        (PITCH, _) => (0.5, 2.0, 1.0),
        (MODEL_A, GroupModel::Water) => (0.0, 2.5, 1.0),
        (MODEL_A, _) => (0.4, 2.5, 1.0),
        (MODEL_B, GroupModel::Water) => (0.0, 4.0, 1.0),
        (MODEL_B, _) => (0.0, 60.0, 0.0),
        _ => (0.0, 1.0, 0.0),
    }
}

/// Names of the parameters a scene's `controls` block may list.
const PARAM_NAMES: [&str; group_param::COUNT] = ["level", "events", "motion", "distance", "pitch", "a", "b"];

/// One group's presentation in the scene file.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GroupUi {
    /// The clips' `group` id this row controls.
    pub group: String,
    /// Name in the panel (the app's language).
    #[serde(default)]
    pub title: String,
    /// Icon key for the panel.
    #[serde(default)]
    pub icon: String,
    /// Parameters shown: any of level, events, motion, distance, pitch, a, b (omitted = all
    /// that apply to the group's clips).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Vec<String>>,
}

/// A scene lever (`"levers"` in the scene): one slider with a meaning ("Ships", "Current")
/// that moves several group parameters at once, each along its own curve. Its effect comes on
/// top of the per-group values: dB added to `level`, added to `b`, the rest multiplied.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LeverDesc {
    pub key: String,
    /// Panel name, English (the app translates).
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub min: f32,
    #[serde(default = "one")]
    pub max: f32,
    /// Starting value (omitted: where every curve is neutral is not known, so `min`..`max` midpoint).
    #[serde(default)]
    pub default: Option<f32>,
    #[serde(default)]
    pub log: bool,
    /// How the direction's energy steer moves it: −1..1, a share of half its range.
    #[serde(default)]
    pub lively: f32,
    pub drives: Vec<DriveDesc>,
}

fn one() -> f32 {
    1.0
}

/// What a lever moves: group parameter `param` (level, events, motion, distance, pitch, a, b)
/// of `group`, piecewise linear through `at` = [[lever value, effect], ...].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DriveDesc {
    pub group: String,
    pub param: String,
    pub at: Vec<[f32; 2]>,
}

/// At most this many levers per scene.
pub const MAX_LEVERS: usize = 16;

impl LeverDesc {
    pub fn default_value(&self) -> f32 {
        self.default.unwrap_or(0.5 * (self.min + self.max))
    }
}

/// Checks a scene's levers against its clip groups.
pub fn validate_levers(desc: &SceneDesc) -> Result<(), String> {
    let groups: Vec<&str> = desc.entities.iter().filter_map(|e| match e {
        EntityDesc::Clip(c) => c.group.as_deref(),
        _ => None,
    }).collect();
    if desc.levers.len() > MAX_LEVERS {
        return Err(format!("levers: at most {MAX_LEVERS}"));
    }
    for (i, l) in desc.levers.iter().enumerate() {
        let bad = |m: String| Err(format!("lever `{}`: {m}", l.key));
        if l.key.is_empty() || desc.levers[..i].iter().any(|o| o.key == l.key) {
            return Err(format!("levers: empty or repeated key `{}`", l.key));
        }
        if !(l.min.is_finite() && l.max.is_finite() && l.max > l.min) || l.log && l.min < 0.0 {
            return bad("needs min < max (log: min >= 0)".into());
        }
        if l.default.is_some_and(|d| !(l.min..=l.max).contains(&d)) || !(-1.0..=1.0).contains(&l.lively) {
            return bad("default within min..max, lively −1..1".into());
        }
        for d in &l.drives {
            if !groups.contains(&d.group.as_str()) {
                return bad(format!("no clip group `{}`", d.group));
            }
            if !PARAM_NAMES.contains(&d.param.as_str()) {
                return bad(format!("unknown param `{}` (level, events, motion, distance, pitch, a, b)", d.param));
            }
            if d.at.is_empty() || d.at.windows(2).any(|w| w[1][0] <= w[0][0]) || d.at.iter().flatten().any(|v| !v.is_finite()) {
                return bad(format!("`{}.{}`: `at` needs points with increasing lever values", d.group, d.param));
            }
        }
    }
    Ok(())
}

fn curve(at: &[[f32; 2]], x: f32) -> f32 {
    if x <= at[0][0] {
        return at[0][1];
    }
    for w in at.windows(2) {
        if x <= w[1][0] {
            let u = (x - w[0][0]) / (w[1][0] - w[0][0]);
            return w[0][1] + (w[1][1] - w[0][1]) * u;
        }
    }
    at[at.len() - 1][1]
}

struct LeverSlot {
    desc: LeverDesc,
    /// (group index, parameter) of each drive, in `desc.drives` order.
    targets: Vec<(usize, usize)>,
    value: crate::engine::AtomicF32,
}

struct GroupSlot {
    id: String,
    title: String,
    icon: String,
    model: GroupModel,
    /// Waveform track of the group (`ASMR_SCOPE_*`): the first clip's source kind.
    scope: u32,
    /// Bit `p` set = parameter `p` applies and is shown.
    mask: u32,
    values: [crate::engine::AtomicF32; group_param::COUNT],
}

/// The per-group controls of a scene's clips. Values are atomics: set from any thread, read by
/// [`Choreo::step`] every block. All groups of the scene, in order of first appearance.
pub struct ClipGroups {
    slots: Vec<GroupSlot>,
    levers: Vec<LeverSlot>,
}

/// One group's values for a block, in the units the choreography uses.
#[derive(Clone, Copy, Debug)]
struct GroupVals {
    level: f32,
    events: f32,
    motion: f32,
    distance: f32,
    pitch: f32,
    a: f32,
    b: f32,
}

impl GroupVals {
    const NEUTRAL: GroupVals = GroupVals { level: 1.0, events: 1.0, motion: 1.0, distance: 1.0, pitch: 1.0, a: 1.0, b: 0.0 };
}

/// `ASMR_SCOPE_*` of the source kind a clip is mixed as.
fn scope_of(c: &ClipDesc) -> u32 {
    use crate::engine::SourceKind;
    let kind = match &c.sound {
        Some(d) => d.model.source_kind(),
        None => c.class.kinds().0,
    };
    match kind {
        SourceKind::Water => 0,
        SourceKind::Wind => 1,
        SourceKind::Bird => 2,
        SourceKind::Rain => 5,
        SourceKind::Insect => 6,
        SourceKind::Fire => 7,
        SourceKind::Thunder => 8,
    }
}

impl ClipGroups {
    pub fn new(desc: &SceneDesc) -> Self {
        let mut slots: Vec<GroupSlot> = Vec::new();
        for e in &desc.entities {
            let EntityDesc::Clip(c) = e else { continue };
            let Some(g) = c.group.as_deref().filter(|g| !g.is_empty()) else { continue };
            let at = match slots.iter().position(|s| s.id == g) {
                Some(i) => i,
                None => {
                    let ui = desc.controls.iter().find(|u| u.group == g);
                    slots.push(GroupSlot {
                        id: g.to_string(),
                        title: ui.map_or(String::new(), |u| u.title.clone()),
                        icon: ui.map_or(String::new(), |u| u.icon.clone()),
                        model: GroupModel::None,
                        scope: scope_of(c),
                        mask: 0,
                        values: core::array::from_fn(|_| crate::engine::AtomicF32::new(0.0)),
                    });
                    slots.len() - 1
                }
            };
            let s = &mut slots[at];
            s.mask |= 0b1111;
            match c.sound.as_ref().map(|d| d.model) {
                Some(crate::sources::ModelKind::Water) => {
                    s.model = GroupModel::Water;
                    s.mask |= 0b110_0000;
                }
                Some(crate::sources::ModelKind::Plasma) => {
                    s.model = GroupModel::Plasma;
                    s.mask |= 0b110_0000;
                }
                Some(_) => {}
                None => s.mask |= 1 << group_param::PITCH,
            }
        }
        for s in slots.iter_mut() {
            if let Some(list) = desc.controls.iter().find(|u| u.group == s.id).and_then(|u| u.params.as_ref()) {
                let mut keep = 0u32;
                for n in list {
                    if let Some(p) = PARAM_NAMES.iter().position(|k| k == n) {
                        keep |= 1 << p;
                    }
                }
                s.mask &= keep | 1; // level always stays: it is the row's main lever
            }
            for p in 0..group_param::COUNT {
                s.values[p].store(group_param_range(p as u32, s.model).2);
            }
        }
        let levers = desc
            .levers
            .iter()
            .map(|l| LeverSlot {
                targets: l
                    .drives
                    .iter()
                    .map(|d| (slots.iter().position(|s| s.id == d.group).unwrap_or(usize::MAX), PARAM_NAMES.iter().position(|p| *p == d.param).unwrap_or(0)))
                    .collect(),
                value: crate::engine::AtomicF32::new(l.default_value()),
                desc: l.clone(),
            })
            .collect();
        Self { slots, levers }
    }

    pub fn lever_count(&self) -> usize {
        self.levers.len()
    }

    pub fn lever(&self, i: usize) -> Option<&LeverDesc> {
        self.levers.get(i).map(|l| &l.desc)
    }

    pub fn lever_value(&self, i: usize) -> f32 {
        self.levers.get(i).map_or(0.0, |l| l.value.load())
    }

    /// Sets lever `i` (clamped; NaN = its default). Any thread.
    pub fn set_lever(&self, i: usize, v: f32) {
        if let Some(l) = self.levers.get(i) {
            let d = &l.desc;
            let v = if v.is_nan() { d.default_value() } else { v };
            if v.is_finite() {
                l.value.store(v.clamp(d.min, d.max));
            }
        }
    }

    pub fn len(&self) -> usize {
        self.slots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    pub fn index_of(&self, id: &str) -> Option<usize> {
        self.slots.iter().position(|s| s.id == id)
    }

    pub fn id(&self, i: usize) -> Option<&str> {
        self.slots.get(i).map(|s| s.id.as_str())
    }

    pub fn title(&self, i: usize) -> Option<&str> {
        self.slots.get(i).map(|s| s.title.as_str())
    }

    pub fn icon(&self, i: usize) -> Option<&str> {
        self.slots.get(i).map(|s| s.icon.as_str())
    }

    pub fn scope(&self, i: usize) -> u32 {
        self.slots.get(i).map_or(0, |s| s.scope)
    }

    pub fn model(&self, i: usize) -> GroupModel {
        self.slots.get(i).map_or(GroupModel::None, |s| s.model)
    }

    /// Bit `p` set = parameter `p` is available for group `i`.
    pub fn mask(&self, i: usize) -> u32 {
        self.slots.get(i).map_or(0, |s| s.mask)
    }

    /// Sets a parameter (clamped to its range). Unknown groups, parameters and non-finite
    /// values are ignored. Any thread.
    pub fn set(&self, i: usize, p: u32, value: f32) {
        let Some(s) = self.slots.get(i) else { return };
        if (p as usize) >= group_param::COUNT || !value.is_finite() {
            return;
        }
        let (lo, hi, _) = group_param_range(p, s.model);
        s.values[p as usize].store(value.clamp(lo, hi));
    }

    pub fn set_by_id(&self, id: &str, p: u32, value: f32) {
        if let Some(i) = self.index_of(id) {
            self.set(i, p, value);
        }
    }

    pub fn get(&self, i: usize, p: u32) -> f32 {
        self.slots.get(i).and_then(|s| s.values.get(p as usize)).map_or(0.0, |v| v.load())
    }

    fn vals(&self, i: Option<usize>) -> GroupVals {
        let Some(s) = i.and_then(|i| self.slots.get(i)) else { return GroupVals::NEUTRAL };
        let mut v: [f32; group_param::COUNT] = core::array::from_fn(|p| s.values[p].load());
        // Scene levers on top: dB and `b` add, the rest multiply.
        let gi = i.unwrap_or(usize::MAX);
        for l in &self.levers {
            let x = l.value.load();
            for (d, &(g, p)) in l.desc.drives.iter().zip(&l.targets) {
                if g == gi {
                    let y = curve(&d.at, x);
                    if p == group_param::LEVEL as usize || p == group_param::MODEL_B as usize {
                        v[p] += y;
                    } else {
                        v[p] *= y.max(0.0);
                    }
                }
            }
        }
        GroupVals {
            level: 10f32.powf(v[group_param::LEVEL as usize].max(-80.0) / 20.0),
            events: v[group_param::EVENTS as usize],
            motion: v[group_param::MOTION as usize],
            distance: v[group_param::DISTANCE as usize].max(0.05),
            pitch: v[group_param::PITCH as usize].max(0.1),
            a: v[group_param::MODEL_A as usize],
            b: v[group_param::MODEL_B as usize].max(0.0),
        }
    }
}

// ---------------------------------------------------------------------------------------
// Choreography (control rate)
// ---------------------------------------------------------------------------------------

/// The screen in head coordinates: centre straight ahead at `distance`.
#[derive(Clone, Copy, Debug)]
pub struct Screen {
    pub width: f32,
    pub height: f32,
    pub distance: f32,
    /// Elevation of its centre from the eyes, radians (− = below eye level).
    pub elevation: f32,
}

impl Screen {
    pub fn centre(&self) -> Vec3 {
        Vec3::new(self.distance * self.elevation.cos(), 0.0, self.distance * self.elevation.sin())
    }
}

/// Scene-wide user controls over the clips (ASMR_PARAM_SCENE_*), 1 = as authored.
#[derive(Clone, Copy, Debug)]
pub struct Macros {
    /// Speed of everything that travels (drift, passes, sweeps, orbits), 0..3.
    pub motion: f32,
    /// How often things happen: event and appearance rate, 0..3.
    pub events: f32,
    /// Level of beds and textures, 0..2 (linear).
    pub bed: f32,
    /// Level of creatures and events, 0..2 (linear).
    pub creatures: f32,
}

impl Default for Macros {
    fn default() -> Self {
        Self { motion: 1.0, events: 1.0, bed: 1.0, creatures: 1.0 }
    }
}

struct Track {
    group: Option<usize>,
    /// Last position before the group's distance scaling.
    centre: Vec3,
    /// Model parameters (a, b) last sent to the generators.
    model_applied: (f32, f32),
    entity: usize,
    emitters: core::ops::Range<usize>,
    desc: ClipDesc,
    bank: Arc<ClipBank>,
    rng: Rng,
    /// Active: time since the appearance started; idle: time left until the next one.
    t: f32,
    active: bool,
    dur: f32,
    from: Vec3,
    to: Vec3,
    bend: Vec3,
    /// Continuous clips: seconds to the next event cue.
    next_cue: f32,
    wander: Vec3,
    wander_mid: Vec3,
    wander_target: Vec3,
    angle: f32,
    /// Seconds along a `path` motion, and its gain there.
    path_t: f32,
    path_gain: f32,
    cue: u32,
}

/// Moves every clip entity and cues its players. Real-time safe after [`Choreo::new`].
pub struct Choreo {
    tracks: Vec<Track>,
    groups: Arc<ClipGroups>,
}

fn v3(a: [f32; 3]) -> Vec3 {
    Vec3::new(a[0], a[1], a[2])
}

impl Choreo {
    pub fn new(desc: &SceneDesc, world: &World, groups: Arc<ClipGroups>) -> Self {
        let mut tracks = Vec::new();
        let mut seed = Rng::new(desc.seed ^ 0xC11B_5000);
        for (i, e) in desc.entities.iter().enumerate() {
            let EntityDesc::Clip(c) = e else { continue };
            let entity = &world.entities[i];
            let crate::scene::Geometry::Clip { bank } = &entity.geometry else { continue };
            let mut rng = Rng::new(seed.next_u64());
            let first = if c.exact { c.first } else { c.first + rng.range(0.0, 0.2 * c.first) };
            let group = c.group.as_deref().and_then(|g| groups.index_of(g));
            tracks.push(Track {
                group,
                centre: Vec3::default(),
                model_applied: (1.0, 0.0),
                entity: i,
                emitters: entity.emitters.clone(),
                desc: c.clone(),
                bank: bank.clone(),
                rng,
                t: first,
                active: false,
                dur: 0.0,
                from: Vec3::default(),
                to: Vec3::default(),
                bend: Vec3::default(),
                next_cue: first,
                wander: Vec3::default(),
                wander_mid: Vec3::default(),
                wander_target: Vec3::default(),
                angle: 0.0,
                path_t: 0.0,
                path_gain: 1.0,
                cue: 0,
            });
        }
        Self { tracks, groups }
    }

    pub fn step(&mut self, dt: f32, world: &mut World, screen: Screen, m: Macros) {
        for tr in self.tracks.iter_mut() {
            let g = self.groups.vals(tr.group);
            tr.step(dt, world, screen, m, g);
        }
    }
}

impl Track {
    fn pick(&mut self) -> (usize, f32) {
        let n = self.bank.files.len().max(1);
        let file = (self.rng.next_u32() as usize) % n;
        let rate = self.rng.range(self.desc.rate[0], self.desc.rate[1]);
        (file, rate)
    }

    fn cue(&mut self, world: &mut World, file: usize, rate: f32) {
        self.cue = self.cue.wrapping_add(1);
        let looped = self.desc.playback == Playback::Loop;
        let base = if looped { self.rng.range(0.0, 0.6) } else { 0.0 };
        for (k, ei) in self.emitters.clone().enumerate() {
            let c = &mut world.emitters[ei].controls;
            c.cue = self.cue;
            c.cue_file = file as u32;
            c.cue_rate = rate;
            // Several emitters of a texture play different stretches of it (decorrelated).
            c.cue_offset = if looped { (base + 0.37 * k as f32).fract() * 0.7 } else { 0.0 };
        }
    }

    fn jitter(&mut self, p: Vec3, j: f32) -> Vec3 {
        if j <= 0.0 {
            return p;
        }
        p + Vec3::new(self.rng.gauss(), self.rng.gauss(), 0.5 * self.rng.gauss()) * j
    }

    /// Begins an appearance (travelling motions and drift).
    fn appear(&mut self, world: &mut World, screen: Screen, pitch: f32) {
        let (file, rate) = self.pick();
        let len = self.bank.seconds(file) / (rate * pitch.max(0.1));
        self.active = true;
        self.t = 0.0;
        match self.desc.motion.clone() {
            Motion::Pass { from, to, to_screen, duration, jitter } => {
                self.from = self.jitter(v3(from), jitter);
                self.to = if to_screen { screen.centre() } else { self.jitter(v3(to.unwrap_or(from)), jitter) };
                self.dur = duration.unwrap_or(len).max(0.5);
            }
            Motion::Drift { speed, ahead, behind, lateral, height, own_speed } => {
                let side = if self.rng.uniform() < 0.5 { -1.0 } else { 1.0 };
                let y = side * self.rng.range(lateral[0], lateral[1]);
                let z = self.rng.range(height[0], height[1].max(height[0]));
                self.from = Vec3::new(ahead, y, z);
                self.to = Vec3::new(-behind, y + self.rng.range(-2.0, 2.0), z + self.rng.range(-1.0, 1.0));
                let rel = (speed - own_speed).max(0.05);
                self.dur = (ahead + behind) / rel;
            }
            Motion::Sweep { duration, .. } => self.dur = duration,
            Motion::Portal { from, duration, curve } => {
                self.from = v3(from);
                self.to = screen.centre();
                let d = self.to - self.from;
                // Sideways bulge: perpendicular to the path in the horizontal plane.
                let side = if self.rng.uniform() < 0.5 { -1.0 } else { 1.0 };
                self.bend = Vec3::new(-d.y, d.x, 0.0) * (curve * side);
                self.dur = duration;
            }
            _ => self.dur = f32::INFINITY,
        }
        self.cue(world, file, rate);
    }

    fn step(&mut self, dt: f32, world: &mut World, screen: Screen, m: Macros, g: GroupVals) {
        let motion = (m.motion.clamp(0.0, 3.0) * g.motion).clamp(0.0, 6.0);
        // Rarer events: idle countdowns run slower (events = 0 stops new appearances).
        let rate = (m.events.clamp(0.0, 3.0) * g.events).clamp(0.0, 6.0);
        let level = match self.desc.class {
            ClipClass::Bed | ClipClass::Texture => m.bed,
            ClipClass::Creature | ClipClass::Event => m.creatures,
        }
        .clamp(0.0, 2.0)
            * g.level;
        let fade = self.desc.fade.max(0.01);
        let continuous = self.desc.continuous();
        let mut env: f32;
        let mut centre = self.centre;
        let mut line: Option<(Vec3, f32)> = None; // sweep: lateral line of this width

        if continuous {
            match self.desc.motion.clone() {
                Motion::Fixed { position } => centre = v3(position),
                Motion::Escort { offset, wander } => {
                    // Ornstein-Uhlenbeck wander (time constant 8 s) of a target point, which the
                    // animal follows through two smoothing stages (~1.2 s each). The raw OU
                    // path is rough at the control rate: as a position it jittered by
                    // centimetres every block, a visible twitch and an audible Doppler rattle.
                    let a = (dt / 8.0).min(1.0);
                    let s = wander * (2.0 * a).sqrt();
                    let n = Vec3::new(self.rng.gauss(), self.rng.gauss(), 0.4 * self.rng.gauss()) * s;
                    self.wander_target = self.wander_target * (1.0 - a) + n;
                    let f = (dt / 1.2).min(1.0);
                    self.wander_mid = self.wander_mid + (self.wander_target - self.wander_mid) * f;
                    self.wander = self.wander + (self.wander_mid - self.wander) * f;
                    centre = v3(offset) + self.wander;
                }
                Motion::Orbit { center, radius, height, period } => {
                    self.angle = (self.angle + core::f32::consts::TAU * dt * motion / period).rem_euclid(core::f32::consts::TAU);
                    centre = v3(center) + Vec3::new(radius * self.angle.cos(), radius * self.angle.sin(), height);
                }
                Motion::Path { ref points, repeat } => {
                    self.path_t += dt * motion;
                    let (p, gain) = path_at(points, self.path_t, repeat);
                    centre = p;
                    self.path_gain = gain;
                }
                _ => {}
            }
            if !self.active {
                // Continuous clips fade in once and stay.
                self.active = true;
                self.t = 0.0;
                if self.desc.playback == Playback::Loop {
                    let (file, rate) = self.pick();
                    self.cue(world, file, rate);
                }
            }
            self.t += dt;
            env = (self.t / fade).min(1.0) * self.path_gain;
            if self.desc.playback == Playback::Events {
                self.next_cue -= dt * rate;
                if self.next_cue <= 0.0 {
                    let (file, rate) = self.pick();
                    self.cue(world, file, rate);
                    self.next_cue = self.rng.range(self.desc.interval[0], self.desc.interval[1]).max(0.5);
                }
            }
        } else if !self.active {
            self.t -= dt * rate;
            env = 0.0;
            if self.t <= 0.0 {
                self.appear(world, screen, g.pitch);
                centre = self.from;
            }
        } else {
            self.t += dt * motion;
            let u = (self.t / self.dur).clamp(0.0, 1.0);
            env = (self.t / fade).min(1.0).min(((self.dur - self.t) / fade).max(0.0));
            match self.desc.motion.clone() {
                Motion::Pass { .. } | Motion::Drift { .. } => centre = self.from + (self.to - self.from) * u,
                Motion::Portal { .. } => {
                    // Quadratic Bézier through the bulge; eases in towards the screen.
                    let e = 1.0 - (1.0 - u) * (1.0 - u);
                    let mid = (self.from + self.to) * 0.5 + self.bend;
                    let a = self.from + (mid - self.from) * e;
                    let b = mid + (self.to - mid) * e;
                    centre = a + (b - a) * e;
                }
                Motion::Sweep { from_x, to_x, width, height, .. } => {
                    let x = from_x + (to_x - from_x) * u;
                    let d = screen.distance;
                    // Hourglass: wide behind, the screen's width at the screen, wide again beyond.
                    let w = if x <= d {
                        screen.width + (width - screen.width) * ((d - x) / (d - from_x).max(0.1)).clamp(0.0, 1.0)
                    } else {
                        screen.width + (width - screen.width) * ((x - d) / (to_x - d).max(0.1)).clamp(0.0, 1.0)
                    };
                    centre = Vec3::new(x, 0.0, height);
                    line = Some((centre, w));
                }
                _ => {}
            }
            if self.t >= self.dur {
                self.active = false;
                self.t = self.rng.range(self.desc.interval[0], self.desc.interval[1]);
                env = 0.0;
            }
        }

        self.centre = centre;
        let dist = g.distance;
        let e = &mut world.entities[self.entity];
        e.position = centre * dist;
        let moving = !continuous || !matches!(self.desc.motion, Motion::Fixed { .. });
        e.flags = (e.flags & !(object_flags::ABSENT | object_flags::MOVING))
            | if env <= 0.0 && !continuous { object_flags::ABSENT } else { 0 }
            | if moving && self.active { object_flags::MOVING } else { 0 };
        let n = self.emitters.len();
        for (k, ei) in self.emitters.clone().enumerate() {
            let em = &mut world.emitters[ei];
            em.offset = match line {
                Some((_, w)) if n > 1 => Vec3::new(0.0, w * (k as f32 / (n - 1) as f32 - 0.5), 0.0),
                Some(_) => Vec3::default(),
                None if n > 1 && self.desc.spread > 0.0 => {
                    let a = core::f32::consts::TAU * k as f32 / n as f32;
                    if self.desc.tunnel {
                        Vec3::new(0.0, a.cos(), a.sin()) * self.desc.spread
                    } else {
                        Vec3::new(a.cos(), a.sin(), 0.0) * self.desc.spread
                    }
                }
                None => Vec3::default(),
            };
            em.offset = em.offset * dist;
            em.controls.activity = if env > 0.0 { 1.0 } else { 0.0 };
            em.controls.clip_env = env * level;
            em.controls.clip_pitch = g.pitch;
        }
        self.apply_model(world, g);
    }
}

impl Track {
    /// The group's model parameters, applied to the clip's physical-model emitters on change.
    fn apply_model(&mut self, world: &mut World, g: GroupVals) {
        if self.group.is_none() || self.model_applied == (g.a, g.b) {
            return;
        }
        self.model_applied = (g.a, g.b);
        for ei in self.emitters.clone() {
            let em = &mut world.emitters[ei];
            let cfg = match em.base {
                Some(ModelConfig::Water(own)) => {
                    let mut c = own;
                    c.flow_ms = (own.flow_ms * g.a).clamp(0.0, 6.0);
                    c.turbulence = (own.turbulence * g.b).clamp(0.0, 1.0);
                    ModelConfig::Water(c)
                }
                Some(ModelConfig::Plasma(own)) => {
                    let mut c = own;
                    c.hiss_hz = (own.hiss_hz * g.a).clamp(100.0, 2000.0);
                    c.whistlers_per_min = (own.whistlers_per_min + g.b).clamp(0.0, 120.0);
                    ModelConfig::Plasma(c)
                }
                _ => continue,
            };
            em.generator.set_model(&cfg);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav16(samples: &[i16], rate: u32, channels: u16) -> Vec<u8> {
        let data: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        let mut b = Vec::new();
        b.extend_from_slice(b"RIFF");
        b.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        b.extend_from_slice(b"WAVEfmt ");
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&channels.to_le_bytes());
        b.extend_from_slice(&rate.to_le_bytes());
        b.extend_from_slice(&(rate * 2 * channels as u32).to_le_bytes());
        b.extend_from_slice(&(2 * channels).to_le_bytes());
        b.extend_from_slice(&16u16.to_le_bytes());
        b.extend_from_slice(b"data");
        b.extend_from_slice(&(data.len() as u32).to_le_bytes());
        b.extend_from_slice(&data);
        b
    }

    #[test]
    fn decodes_stereo_to_mono() {
        let (m, r) = decode_wav(&wav16(&[1000, 3000, -2000, 0], 44100, 2)).unwrap();
        assert_eq!(r, 44100.0);
        assert_eq!(m.len(), 2);
        assert!((m[0] - 2000.0 / 32768.0).abs() < 1e-6);
    }

    #[test]
    fn resample_keeps_tone() {
        let x: Vec<f32> = (0..44100).map(|i| (core::f32::consts::TAU * 440.0 * i as f32 / 44100.0).sin()).collect();
        let y = resample(&x, 44100.0, 48000.0);
        assert!((y.len() as i64 - 48000).abs() <= 1);
        let rms = (y[1000..47000].iter().map(|v| v * v).sum::<f32>() / 46000.0).sqrt();
        assert!((rms - core::f32::consts::FRAC_1_SQRT_2).abs() < 0.01, "{rms}");
    }

    #[test]
    fn loop_player_never_stops() {
        let sr = 48000.0;
        // Noise, like the textures this is for: equal-power cross-fades keep decorrelated
        // material level (a pure tone could cancel against itself mid-fade).
        let mut r = Rng::new(3);
        let mut noise = || -> Vec<f32> { (0..24000).map(|_| r.bipolar() * 1.7).collect() };
        let bank = Arc::new(ClipBank { files: vec![noise(), noise()], names: vec!["a".into(), "b".into()], sample_rate: sr });
        let mut p = ClipPlayer::new(bank, true, 1.0, 7);
        let c = Controls { activity: 1.0, clip_env: 1.0, cue: 1, cue_rate: 1.0, ..Default::default() };
        let mut buf = [0.0f32; 64];
        let mut quiet = 0;
        for b in 0..(10.0 * sr / 64.0) as usize {
            p.render(&mut buf, &c);
            let e: f32 = buf.iter().map(|v| v * v).sum::<f32>() / 64.0;
            if b > 200 && e < 0.05 {
                quiet += 1;
            }
        }
        assert!(quiet < 5, "{quiet} near-silent blocks in a 10 s loop");
    }

    fn scene_with(motion: &str, dir: &std::path::Path) -> (SceneDesc, World) {
        let samples: Vec<i16> = (0..48000).map(|i| ((i as f32 * 0.37).sin() * 8000.0) as i16).collect();
        std::fs::write(dir.join("t.wav"), wav16(&samples, 48000, 1)).unwrap();
        let json = format!(
            r#"{{"name":"t","assets":"{}","entities":[{{"kind":"clip","id":1,"files":["t.wav"],"first":0,"interval":[1,1],"motion":{motion}}}]}}"#,
            dir.display()
        );
        let desc = SceneDesc::from_json(&json).unwrap();
        let world = World::build(&desc, 48000.0).unwrap();
        (desc, world)
    }

    fn run(desc: &SceneDesc, world: &mut World, seconds: f32) -> Vec<Vec3> {
        let mut ch = Choreo::new(desc, world, Arc::new(ClipGroups::new(desc)));
        let screen = Screen { width: 0.3, height: 0.2, distance: 0.6, elevation: -0.0833 };
        let dt = 64.0 / 48000.0;
        let mut out = Vec::new();
        for b in 0..(seconds / dt) as usize {
            ch.step(dt, world, screen, Macros::default());
            if b % 75 == 0 {
                out.push(world.entities[0].position);
            }
        }
        out
    }

    #[test]
    fn drift_passes_from_front_to_back() {
        let dir = std::env::temp_dir().join(format!("asmr_clip_drift_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (desc, mut world) = scene_with(r#"{"type":"drift","speed":10,"ahead":30,"behind":20,"lateral":[5,6]}"#, &dir);
        let path = run(&desc, &mut world, 5.5);
        let xs: Vec<f32> = path.iter().map(|p| p.x).collect();
        assert!(xs.iter().any(|&x| x > 20.0) && xs.iter().any(|&x| x < -10.0), "{xs:?}");
        assert!(path.iter().all(|p| p.y.abs() >= 3.0), "stays off to the side");
    }

    #[test]
    fn pass_into_screen_ends_at_its_centre() {
        let dir = std::env::temp_dir().join(format!("asmr_clip_portal_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (desc, mut world) = scene_with(r#"{"type":"pass","from":[-10,-4,0],"to_screen":true,"duration":2}"#, &dir);
        let path = run(&desc, &mut world, 1.99);
        let last = *path.last().unwrap();
        assert!((last - Vec3::new(0.6, 0.0, -0.05)).length() < 0.6, "{last:?}");
        assert!(path[1].x < -5.0, "{:?}", path[1]);
    }

    /// A fixed clip in group "g" at 4 m ahead, 2 m left, with one more emitter ring.
    fn grouped(dir: &std::path::Path, motion: &str) -> (SceneDesc, World, Arc<ClipGroups>, Choreo) {
        let samples: Vec<i16> = (0..48000).map(|i| ((i as f32 * 0.37).sin() * 8000.0) as i16).collect();
        std::fs::write(dir.join("t.wav"), wav16(&samples, 48000, 1)).unwrap();
        let json = format!(
            r#"{{"name":"t","assets":"{}","controls":[{{"group":"g","title":"G","icon":"x","params":["level","distance"]}}],
            "entities":[{{"kind":"clip","id":1,"group":"g","files":["t.wav"],"first":0,"interval":[0.5,0.5],"emitters":2,"spread":1.0,"motion":{motion}}}]}}"#,
            dir.display()
        );
        let desc = SceneDesc::from_json(&json).unwrap();
        let world = World::build(&desc, 48000.0).unwrap();
        let groups = Arc::new(ClipGroups::new(&desc));
        let ch = Choreo::new(&desc, &world, groups.clone());
        (desc, world, groups, ch)
    }

    fn steps(ch: &mut Choreo, world: &mut World, seconds: f32) {
        let screen = Screen { width: 0.3, height: 0.2, distance: 0.6, elevation: -0.0833 };
        let dt = 64.0 / 48000.0;
        for _ in 0..(seconds / dt) as usize {
            ch.step(dt, world, screen, Macros::default());
        }
    }

    #[test]
    fn group_level_and_distance_take_effect() {
        let dir = std::env::temp_dir().join(format!("asmr_clip_group_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (_d, mut world, groups, mut ch) = grouped(&dir, r#"{"type":"fixed","position":[4,2,0]}"#);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups.id(0), Some("g"));
        assert_eq!(groups.title(0), Some("G"));
        // The scene shows only level and distance (+ level is always kept).
        assert_eq!(groups.mask(0), 1 | (1 << group_param::DISTANCE));
        steps(&mut ch, &mut world, 4.0);
        let (env0, pos0, off0) = (world.emitters[0].controls.clip_env, world.entities[0].position, world.emitters[1].offset);
        assert!((env0 - 1.0).abs() < 1e-3, "{env0}");
        assert!((pos0.x - 4.0).abs() < 1e-4);

        groups.set(0, group_param::LEVEL, -20.0);
        groups.set(0, group_param::DISTANCE, 2.0);
        steps(&mut ch, &mut world, 0.1);
        let env1 = world.emitters[0].controls.clip_env;
        assert!((env1 - 0.1).abs() < 1e-3, "-20 dB is x0.1, got {env1}");
        let (pos1, off1) = (world.entities[0].position, world.emitters[1].offset);
        assert!((pos1.x - 2.0 * pos0.x).abs() < 1e-4 && (pos1.y - 2.0 * pos0.y).abs() < 1e-4, "{pos1:?}");
        assert!((off1.length() - 2.0 * off0.length()).abs() < 1e-4, "{off1:?} vs {off0:?}");

        // Back to the defaults: positions do not drift (no compounding).
        groups.set(0, group_param::DISTANCE, 1.0);
        groups.set(0, group_param::LEVEL, 0.0);
        steps(&mut ch, &mut world, 0.1);
        assert!((world.entities[0].position.x - 4.0).abs() < 1e-4);
        // Values clamp to their ranges.
        groups.set(0, group_param::LEVEL, 99.0);
        assert_eq!(groups.get(0, group_param::LEVEL), 10.0);
    }

    #[test]
    fn group_events_scale_the_cue_rate() {
        let dir = std::env::temp_dir().join(format!("asmr_clip_group_ev_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let count = |events: f32| {
            let (_d, mut world, groups, mut ch) = grouped(&dir, r#"{"type":"fixed","position":[4,2,0]}"#);
            groups.set(0, group_param::EVENTS, events);
            steps(&mut ch, &mut world, 20.0);
            world.emitters[0].controls.cue
        };
        let (none, one, triple) = (count(0.0), count(1.0), count(3.0));
        assert!(none <= 1, "events 0 stops new cues: {none}");
        assert!(one >= 10 && one <= 45, "{one}");
        assert!(triple as f32 > 2.0 * one as f32, "x3 events: {triple} vs {one}");
    }

    #[test]
    fn unknown_group_is_ignored() {
        let dir = std::env::temp_dir().join(format!("asmr_clip_group_un_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (_d, mut world, groups, mut ch) = grouped(&dir, r#"{"type":"fixed","position":[4,2,0]}"#);
        groups.set(7, group_param::LEVEL, -30.0);
        groups.set_by_id("nope", group_param::LEVEL, -30.0);
        groups.set(0, 99, 1.0);
        groups.set(0, group_param::LEVEL, f32::NAN);
        assert_eq!(groups.get(7, group_param::LEVEL), 0.0);
        assert_eq!(groups.get(0, group_param::LEVEL), 0.0);
        steps(&mut ch, &mut world, 2.0);
        assert!((world.emitters[0].controls.clip_env - 1.0).abs() < 1e-3);
        // A scene without groups has none.
        let (desc, _w) = scene_with(r#"{"type":"fixed","position":[1,0,0]}"#, &dir);
        assert!(ClipGroups::new(&desc).is_empty());
    }

    #[test]
    fn model_group_params_reach_the_generator() {
        let json = r#"{"name":"t","controls":[],"entities":[{"kind":"clip","id":1,"group":"w","sound":{"model":"water","preset":"rapids"},"first":0,"motion":{"type":"fixed","position":[2,0,0]}}]}"#;
        let desc = SceneDesc::from_json(json).unwrap();
        let mut world = World::build(&desc, 48000.0).unwrap();
        let groups = Arc::new(ClipGroups::new(&desc));
        assert_eq!(groups.model(0), GroupModel::Water);
        assert_eq!(groups.mask(0) & (1 << group_param::PITCH), 0, "no pitch for models");
        assert_ne!(groups.mask(0) & (1 << group_param::MODEL_A), 0);
        let mut ch = Choreo::new(&desc, &world, groups.clone());
        groups.set(0, group_param::MODEL_A, 0.0);
        steps(&mut ch, &mut world, 0.1);
        assert_eq!(ch.tracks[0].model_applied, (0.0, 1.0));
    }

    #[test]
    fn escort_moves_smoothly() {
        let dir = std::env::temp_dir().join(format!("asmr_clip_escort_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (desc, mut world) = scene_with(r#"{"type":"escort","offset":[4,3,0.5],"wander":1.5}"#, &dir);
        let mut ch = Choreo::new(&desc, &world, Arc::new(ClipGroups::new(&desc)));
        let screen = Screen { width: 0.3, height: 0.2, distance: 0.6, elevation: -0.0833 };
        let dt = 64.0 / 48000.0;
        let mut prev = Vec3::default();
        let mut prev_v = 0.0f32;
        let mut worst_accel = 0.0f32;
        for b in 0..(30.0 / dt) as usize {
            ch.step(dt, &mut world, screen, Macros::default());
            let p = world.entities[0].position;
            if b > 2 {
                let v = (p - prev).length() / dt;
                assert!(v < 2.0, "escort speed {v} m/s at block {b}");
                worst_accel = worst_accel.max((v - prev_v).abs() / dt);
                prev_v = v;
            }
            prev = p;
        }
        assert!(worst_accel < 20.0, "jerky: {worst_accel} m/s²");
    }
}
