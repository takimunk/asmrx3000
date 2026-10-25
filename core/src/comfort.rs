//! Comfort analysis of a stereo (binaural) render: does anything in it hurt, startle, grate
//! or float? Offline, streaming, about 50× faster than real time. Used by
//! `examples/comfort.rs` (any scene, N minutes, optional head turns, self-test) and by the
//! CI comfort test. Thresholds and their sources: docs/comfort.md.
//!
//! What it measures (all on the signal that reaches the ears):
//! * **Clicks**: the second difference of each channel (≈ +12 dB/oct, all of its energy
//!   above ~8 kHz) against its RMS in the surrounding ±5 ms (the central ±0.5 ms excluded).
//!   A discontinuity is a one- or two-sample spike tens of dB above its neighbourhood; noise,
//!   even impulsive natural noise heard through air, stays far below.
//! * **Loudness** (ITU-R BS.1770-4 / EBU R128): momentary (400 ms), short-term (3 s), gated
//!   integrated, loudness range (EBU Tech 3342), true peak (4× oversampled), and two kinds of
//!   surprise: momentary loudness over the level of the last 30 s the ear has adapted to, and
//!   a sudden jump of the 25 ms level over the last 2 s. Salient events per minute (the
//!   momentary loudness rising 6 LU over the context) as the focus metric.
//! * **Sharpness** (DIN 45692 on a Zwicker-style specific loudness, reuses mix::loudness)
//!   and an approximate **roughness** (Daniel & Weber style: band envelopes, 30–120 Hz
//!   modulation, weighted by inter-band envelope correlation; 1 asper = 1 kHz, 60 dB,
//!   100 % AM at 70 Hz).
//! * **Tonality**: per second, tone-to-noise ratio of spectral peaks in their critical band
//!   with the prominence criterion of ECMA-74 Annex D; tones prominent for many seconds at
//!   one frequency are reported (whistles, whines, a tonal insect chorus).
//! * **Spectral balance**: octave-band levels per 10 s against a −5 dB/oct reference tilt:
//!   presence (2–5.6 kHz), hiss (5.6–22 kHz) and rumble (22–89 Hz) excess; infrasound
//!   (3–20 Hz) and DC.
//! * **Pitch wobble**: spectral peaks are tracked frame to frame (4096-point FFT, 21 ms hop);
//!   a tone sustained ≥ 0.8 s whose detrended frequency swings ≥ 2 cycles at 0.5–10 Hz is
//!   a floating tone (Doppler of an oscillating source, a modulated delay, resampling).
//! * **Left/right balance** of the K-weighted loudness over 60 s windows, and its drift.
//!
//! Attribution: the caller may push per-group levels (the engine's per-group loudness at
//! the ears) as it goes; every event is attributed to the group that dominates or rose most
//! at that moment.

// Band, octave and filter tables are indexed in step with each other.
#![allow(clippy::needless_range_loop)]

use crate::fft::fft;
use crate::mix::loudness::{self, BandModel, BANDS, EDGES};

/// STFT size and hop (21 ms at 48 kHz).
pub const FFT_N: usize = 4096;
pub const HOP: usize = 1024;
/// Octave bands, centre frequencies (Hz).
pub const OCTAVES: [f32; 10] = [31.5, 63.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0];
/// Prominent tones below this are listed but not judged (hoots and coos are tonal by nature).
pub const TONE_MIN_HZ: f32 = 1000.0;
/// Reference tilt of the octave-band levels, dB per octave (open-plan masking spectra,
/// docs/comfort.md).
pub const REF_TILT_DB_OCT: f32 = -5.0;

/// Pass/warn/fail limits. Every value is "worse when higher". See docs/comfort.md.
#[derive(Clone, Debug)]
pub struct Thresholds {
    /// Click: second difference over its local RMS, dB.
    pub click_score_db: f32,
    /// ... and over the third-largest transient of the surrounding ±150 ms, dB.
    pub click_isolation_db: f32,
    /// Clicks smaller than this (second-difference peak, dBFS) are inaudible: ignored...
    pub click_floor_dbfs: f32,
    /// ... and so are clicks this far below the integrated loudness (LUFS ≈ dBFS), dB.
    pub click_rel_db: f32,
    /// Allowed clicks per hour (warn, fail).
    pub clicks_per_hour: (f32, f32),
    pub true_peak_dbtp: (f32, f32),
    /// Max momentary loudness over the integrated loudness, LU.
    pub peak_over_integrated_lu: (f32, f32),
    /// Max momentary loudness over the 30 s context, LU.
    pub surprise_lu: (f32, f32),
    /// Max 25 ms level jump over the previous 2 s, dB.
    pub sudden_db: (f32, f32),
    /// Loudness range (EBU Tech 3342), LU.
    pub lra_lu: (f32, f32),
    /// Salient events per minute (momentary +6 LU over the context).
    pub events_per_min: (f32, f32),
    /// Sharpness, acum: the worst 10 s median.
    pub sharpness_acum: (f32, f32),
    /// Roughness, asper: the worst 10 s median.
    pub roughness_asper: (f32, f32),
    /// A tone counts when its TNR exceeds the ECMA-74 criterion by this, dB.
    pub tone_margin_db: f32,
    /// ... and its level is at most this far below the total, dB.
    pub tone_rel_db: f32,
    /// Longest prominent tone at one frequency, s.
    pub tone_s: (f32, f32),
    /// Octave-band excess over the reference tilt (10 s windows), dB: presence, hiss, rumble.
    pub presence_db: (f32, f32),
    pub hiss_db: (f32, f32),
    pub rumble_db: (f32, f32),
    /// Infrasound (3–20 Hz) over the 20 Hz–20 kHz level, dB.
    pub infra_db: (f32, f32),
    /// DC per channel (mean over 60 s), dBFS.
    pub dc_dbfs: (f32, f32),
    /// Floating tone: residual frequency modulation, cents RMS (lower limit of the band).
    pub wobble_cents: f32,
    /// Above this the modulation is a vibrato or trill, not a float (cents RMS).
    pub wobble_max_cents: f32,
    /// Floating tones per hour.
    pub wobbles_per_hour: (f32, f32),
    /// |left − right| of the K-weighted loudness over 5 min, dB.
    pub ild_db: (f32, f32),
    /// Range of that balance over the run, dB.
    pub ild_drift_db: (f32, f32),
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            click_score_db: 22.0,
            click_isolation_db: 12.0,
            click_floor_dbfs: -70.0,
            click_rel_db: -20.0,
            clicks_per_hour: (0.5, 2.0),
            true_peak_dbtp: (-3.0, -1.0),
            peak_over_integrated_lu: (12.0, 18.0),
            surprise_lu: (10.0, 14.0),
            sudden_db: (18.0, 24.0),
            lra_lu: (15.0, 20.0),
            events_per_min: (3.0, 6.0),
            sharpness_acum: (2.2, 2.6),
            roughness_asper: (0.3, 0.6),
            tone_margin_db: 6.0,
            tone_rel_db: 30.0,
            tone_s: (10.0, 30.0),
            presence_db: (16.0, 22.0),
            hiss_db: (20.0, 26.0),
            rumble_db: (8.0, 12.0),
            infra_db: (-20.0, -10.0),
            dc_dbfs: (-55.0, -40.0),
            wobble_cents: 6.0,
            wobble_max_cents: 60.0,
            wobbles_per_hour: (6.0, 20.0),
            ild_db: (3.0, 4.5),
            ild_drift_db: (4.0, 6.0),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Verdict {
    Pass,
    Warn,
    Fail,
}

impl Verdict {
    pub fn name(self) -> &'static str {
        match self {
            Verdict::Pass => "pass",
            Verdict::Warn => "WARN",
            Verdict::Fail => "FAIL",
        }
    }
}

/// One metric against its limits.
#[derive(Clone, Debug)]
pub struct Check {
    pub name: &'static str,
    pub value: f32,
    pub limits: (f32, f32),
    pub unit: &'static str,
    /// Where the worst value was, s.
    pub at: Option<f64>,
    pub group: String,
    pub verdict: Verdict,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventKind {
    Click,
    Surprise,
    Sudden,
    Tone,
    Wobble,
}

impl EventKind {
    pub fn name(self) -> &'static str {
        match self {
            EventKind::Click => "click",
            EventKind::Surprise => "loudness surprise",
            EventKind::Sudden => "sudden onset",
            EventKind::Tone => "sustained tone",
            EventKind::Wobble => "floating tone",
        }
    }
}

/// A flagged moment.
#[derive(Clone, Debug)]
pub struct Event {
    pub kind: EventKind,
    /// Start, s.
    pub t: f64,
    /// Duration, s (0 for instants).
    pub dur: f32,
    /// The metric (click dB over local RMS, LU, cents RMS, TNR margin dB).
    pub value: f32,
    /// Frequency (tones), Hz; level (clicks), dBFS.
    pub freq: f32,
    pub level_db: f32,
    /// Modulation rate (wobble), Hz.
    pub rate: f32,
    pub group: String,
}

/// Per-second timeline (CSV).
#[derive(Clone, Debug, Default)]
pub struct Second {
    pub momentary_max: f32,
    pub short_term: f32,
    pub sone: f32,
    pub sharpness: f32,
    pub roughness: f32,
    pub octaves_db: [f32; 10],
    pub infra_db: f32,
    pub ild_db: f32,
    pub clicks: u32,
    pub tones: u32,
    pub wobbles: u32,
    pub group: String,
}

pub struct Report {
    pub seconds: f64,
    pub integrated: f32,
    pub lra: f32,
    pub true_peak: f32,
    pub checks: Vec<Check>,
    pub events: Vec<Event>,
    pub timeline: Vec<Second>,
    pub groups: Vec<String>,
}

// ---------------------------------------------------------------------------------------
// Small filters
// ---------------------------------------------------------------------------------------

/// f64 biquad (RBJ cookbook), direct form I.
#[derive(Clone, Copy, Default)]
struct Bq {
    b: [f64; 3],
    a: [f64; 2],
    x: [f64; 2],
    y: [f64; 2],
}

impl Bq {
    fn lowpass(fs: f32, f: f32, q: f32) -> Self {
        let w = std::f64::consts::TAU * f as f64 / fs as f64;
        let (s, c) = w.sin_cos();
        let al = s / (2.0 * q as f64);
        let a0 = 1.0 + al;
        Self::new([(1.0 - c) / 2.0 / a0, (1.0 - c) / a0, (1.0 - c) / 2.0 / a0], [-2.0 * c / a0, (1.0 - al) / a0])
    }
    fn highpass(fs: f32, f: f32, q: f32) -> Self {
        let w = std::f64::consts::TAU * f as f64 / fs as f64;
        let (s, c) = w.sin_cos();
        let al = s / (2.0 * q as f64);
        let a0 = 1.0 + al;
        Self::new([(1.0 + c) / 2.0 / a0, -(1.0 + c) / a0, (1.0 + c) / 2.0 / a0], [-2.0 * c / a0, (1.0 - al) / a0])
    }
    /// Band pass, 0 dB peak.
    fn bandpass(fs: f32, f: f32, q: f32) -> Self {
        let w = std::f64::consts::TAU * f as f64 / fs as f64;
        let (s, c) = w.sin_cos();
        let al = s / (2.0 * q as f64);
        let a0 = 1.0 + al;
        Self::new([al / a0, 0.0, -al / a0], [-2.0 * c / a0, (1.0 - al) / a0])
    }
    fn new(b: [f64; 3], a: [f64; 2]) -> Self {
        Self { b, a, x: [0.0; 2], y: [0.0; 2] }
    }
    #[inline]
    fn run(&mut self, x: f64) -> f64 {
        let y = self.b[0] * x + self.b[1] * self.x[0] + self.b[2] * self.x[1] - self.a[0] * self.y[0] - self.a[1] * self.y[1];
        self.x = [x, self.x[0]];
        self.y = [y, self.y[0]];
        y
    }
}

#[inline]
fn run2(f: &mut [Bq; 2], x: f64) -> f64 {
    let y = f[0].run(x);
    f[1].run(y)
}

/// 4× oversampling true-peak meter (BS.1770-4 Annex 2 in spirit: 48-tap windowed sinc).
struct TruePeak {
    taps: [[f32; 12]; 4],
    /// The last 12 samples, mirrored so they read contiguously from `pos + 1`.
    hist: [f32; 24],
    pos: usize,
    peak: f32,
    /// Samples left to interpolate around the last loud one (inter-sample peaks exceed the
    /// sample peak by a few dB at most, so quieter stretches need no oversampling).
    hot: u32,
}

impl TruePeak {
    fn new() -> Self {
        let n = 48;
        let mut h = [0.0f32; 48];
        for (i, v) in h.iter_mut().enumerate() {
            let x = (i as f32 - (n as f32 - 1.0) / 2.0) / 4.0;
            let sinc = if x.abs() < 1e-6 { 1.0 } else { (std::f32::consts::PI * x).sin() / (std::f32::consts::PI * x) };
            // Blackman window.
            let t = i as f32 / (n as f32 - 1.0);
            let w = 0.42 - 0.5 * (std::f32::consts::TAU * t).cos() + 0.08 * (2.0 * std::f32::consts::TAU * t).cos();
            *v = sinc * w;
        }
        let mut taps = [[0.0f32; 12]; 4];
        for p in 0..4 {
            let mut sum = 0.0;
            for k in 0..12 {
                taps[p][k] = h[k * 4 + p];
                sum += taps[p][k];
            }
            for k in 0..12 {
                taps[p][k] /= sum;
            }
        }
        Self { taps, hist: [0.0; 24], pos: 0, peak: 0.0, hot: 0 }
    }
    #[inline]
    fn push(&mut self, x: f32) {
        self.pos = (self.pos + 1) % 12;
        self.hist[self.pos] = x;
        self.hist[self.pos + 12] = x;
        if x.abs() > 0.5 * self.peak {
            self.hot = 12;
        }
        if self.hot == 0 {
            return;
        }
        self.hot -= 1;
        // Oldest .. newest.
        let w = &self.hist[self.pos + 1..self.pos + 13];
        for p in 0..4 {
            let mut acc = 0.0f32;
            for k in 0..12 {
                acc += self.taps[p][k] * w[11 - k];
            }
            self.peak = self.peak.max(acc.abs());
        }
        self.peak = self.peak.max(x.abs());
    }
}

/// Click detector on one channel.
struct Clicks {
    ring: Vec<f64>,
    pos: usize,
    sum_all: f64,
    sum_mid: f64,
    x1: f32,
    x2: f32,
    n: u64,
    /// The event being collected: (sample, score², |d2|).
    open: Option<(u64, f64, f32)>,
    found: Vec<(u64, f32, f32)>,
    /// Peak |second difference| per 5 ms block, for the isolation test.
    blk_max: f32,
    peaks: Vec<f32>,
}

/// Half-widths of the click detector's windows, samples at 48 kHz (scaled with fs).
const CLICK_HALF_MS: f32 = 5.0;
const CLICK_GUARD_MS: f32 = 0.5;
/// Isolation test: blocks of 5 ms, neighbourhood ±150 ms without the central ±10 ms.
const CLICK_BLOCK_MS: f32 = 5.0;
const CLICK_NEIGH_BLOCKS: usize = 30;
const CLICK_SELF_BLOCKS: usize = 2;

impl Clicks {
    fn new(half: usize) -> Self {
        Self { ring: vec![0.0; 2 * half + 1], pos: 0, sum_all: 0.0, sum_mid: 0.0, x1: 0.0, x2: 0.0, n: 0, open: None, found: Vec::new(), blk_max: 0.0, peaks: Vec::new() }
    }
}

// ---------------------------------------------------------------------------------------
// Analyzer
// ---------------------------------------------------------------------------------------

/// Roughness bands: every other critical band from 2 to 22.
const ROUGH_BANDS: usize = 11;
const ROUGH_DECIM: usize = 32;
/// Roughness carrier weighting g(z) (after Aures 1985 / Daniel & Weber 1997, approximate).
const ROUGH_G: [f32; ROUGH_BANDS] = [0.45, 0.75, 0.95, 1.0, 1.0, 0.95, 0.85, 0.75, 0.68, 0.62, 0.58];
/// Calibration: 1 kHz, 60 dB SPL, 100 % AM at 70 Hz = 1 asper (unit test `roughness_reference`).
const ROUGH_CAL: f32 = 0.48;
/// Roughness window, s.
const ROUGH_WIN_S: f32 = 0.5;

struct Rough {
    bands: [[Bq; 2]; ROUGH_BANDS],
    env_lp: [[Bq; 2]; ROUGH_BANDS],
    mod_bp: [[Bq; 2]; ROUGH_BANDS],
    decim: usize,
    /// Per window: per band sum of envelope, of filtered² and filtered products with the
    /// next band; envelope power for level weighting.
    sum_env: [f64; ROUGH_BANDS],
    sum_f2: [f64; ROUGH_BANDS],
    sum_x: [f64; ROUGH_BANDS],
    last_f: [f64; ROUGH_BANDS],
    count: usize,
    win: usize,
    out: Vec<f32>,
}

impl Rough {
    fn new(fs: f32) -> Self {
        let fe = fs / ROUGH_DECIM as f32;
        let mut bands = [[Bq::default(); 2]; ROUGH_BANDS];
        let mut env_lp = [[Bq::default(); 2]; ROUGH_BANDS];
        let mut mod_bp = [[Bq::default(); 2]; ROUGH_BANDS];
        for i in 0..ROUGH_BANDS {
            let z = 2 + 2 * i;
            let (lo, hi) = (EDGES[z], EDGES[z + 1]);
            let fc = (lo * hi).sqrt();
            // One broad stage (about 2 Bark at −3 dB): an excitation pattern rather than a
            // filter bank, so AM sidebands stay in the band of their carrier.
            let q = fc / (2.0 * (hi - lo));
            bands[i] = [Bq::bandpass(fs, fc, q), Bq::new([1.0, 0.0, 0.0], [0.0, 0.0])];
            env_lp[i] = [Bq::lowpass(fs, 250.0, 0.707), Bq::new([1.0, 0.0, 0.0], [0.0, 0.0])];
            mod_bp[i] = [Bq::highpass(fe, 30.0, 0.707), Bq::lowpass(fe, 120.0, 0.707)];
        }
        Self {
            bands,
            env_lp,
            mod_bp,
            decim: 0,
            sum_env: [0.0; ROUGH_BANDS],
            sum_f2: [0.0; ROUGH_BANDS],
            sum_x: [0.0; ROUGH_BANDS],
            last_f: [0.0; ROUGH_BANDS],
            count: 0,
            win: (ROUGH_WIN_S * fe) as usize,
            out: Vec::new(),
        }
    }

    #[inline]
    fn push(&mut self, x: f64) {
        let take = self.decim == 0;
        self.decim = (self.decim + 1) % ROUGH_DECIM;
        let mut f = [0.0f64; ROUGH_BANDS];
        for i in 0..ROUGH_BANDS {
            let b = self.bands[i][0].run(x);
            let e = self.env_lp[i][0].run(b.abs());
            if take {
                f[i] = run2(&mut self.mod_bp[i], e);
                self.sum_env[i] += e;
                self.sum_f2[i] += f[i] * f[i];
            }
        }
        if !take {
            return;
        }
        for i in 0..ROUGH_BANDS - 1 {
            self.sum_x[i] += f[i] * f[i + 1];
        }
        self.last_f = f;
        self.count += 1;
        if self.count >= self.win {
            self.out.push(self.window());
            self.sum_env = [0.0; ROUGH_BANDS];
            self.sum_f2 = [0.0; ROUGH_BANDS];
            self.sum_x = [0.0; ROUGH_BANDS];
            self.count = 0;
        }
    }

    fn window(&self) -> f32 {
        let n = self.count.max(1) as f64;
        let mean: Vec<f64> = self.sum_env.iter().map(|s| s / n).collect();
        let emax = mean.iter().cloned().fold(1e-30, f64::max);
        let corr = |i: usize| -> f64 {
            let d = (self.sum_f2[i] * self.sum_f2[i + 1]).sqrt();
            if d <= 0.0 {
                0.0
            } else {
                (self.sum_x[i] / d).max(0.0)
            }
        };
        let mut r = 0.0f64;
        for i in 0..ROUGH_BANDS {
            if mean[i] <= 1e-12 {
                continue;
            }
            let m = (self.sum_f2[i] / n).sqrt() / mean[i];
            let k_lo = if i > 0 { corr(i - 1) } else { corr(i) };
            let k_hi = if i + 1 < ROUGH_BANDS { corr(i) } else { corr(i - 1) };
            // Bands far below the loudest one hardly add (envelope ratio in dB → 0..1).
            let rel_db = 20.0 * (mean[i] / emax).log10();
            let w = ((rel_db + 30.0) / 20.0).clamp(0.0, 1.0);
            let v = ROUGH_G[i] as f64 * m * k_lo * k_hi;
            r += w * v * v;
        }
        (ROUGH_CAL as f64 * r) as f32
    }
}

struct Track {
    t: Vec<f32>,
    f: Vec<f32>,
    db: Vec<f32>,
    last: u64,
    matched: bool,
}

/// Seconds-level accumulators.
#[derive(Default)]
struct SecAcc {
    frames: u32,
    spec_loud: [f64; BANDS],
    oct: [f64; 10],
    power: Vec<f64>,
}

#[derive(Clone, Copy, Debug)]
struct Tone {
    sec: usize,
    f: f32,
    margin: f32,
    rel_db: f32,
}

pub struct Analyzer {
    fs: f32,
    /// dB SPL of a full-scale signal (1.0 RMS); the engine's unit is 1 Pa = 94 dB.
    pub fs_db_spl: f32,
    n: u64,
    // Loudness (BS.1770).
    kw: [[loudness::Biquad; 2]; 2],
    e100: (f64, f64),
    e25: f64,
    blocks100: Vec<(f32, f32)>,
    blocks25: Vec<f32>,
    n100: usize,
    n25: usize,
    tp: [TruePeak; 2],
    clicks: [Clicks; 2],
    click_floor: f32,
    click_score: f32,
    click_blk: u64,
    // Time-domain per-second accumulators.
    sec_len: usize,
    sec_pos: usize,
    dc: [f64; 2],
    infra: [Bq; 4],
    hp20: Bq,
    e_infra: f64,
    e_full: f64,
    dc_sec: Vec<(f32, f32)>,
    infra_sec: Vec<f32>,
    rough: Rough,
    // STFT.
    ring: [Vec<f32>; 2],
    ring_pos: usize,
    since: usize,
    window: Vec<f32>,
    re: Vec<f32>,
    im: Vec<f32>,
    p: Vec<f32>,
    frame: u64,
    model: BandModel,
    band_bins: [(usize, usize); BANDS],
    oct_bins: [(usize, usize); 10],
    acc: SecAcc,
    sec_stats: Vec<(f32, f32, [f32; 10])>,
    tones: Vec<Tone>,
    tracks: Vec<Track>,
    wobbles: Vec<Event>,
    wobble_band: (f32, f32),
    // Attribution.
    groups: Vec<String>,
    ctx_t: Vec<f64>,
    ctx: Vec<f32>,
}

impl Analyzer {
    pub fn new(fs: f32) -> Self {
        Self::with_thresholds(fs, &Thresholds::default())
    }

    /// The detectors that work while streaming (clicks, wobble) take their limits here.
    pub fn with_thresholds(fs: f32, th: &Thresholds) -> Self {
        let k = loudness::k_weighting(fs);
        let half = (CLICK_HALF_MS * 1e-3 * fs).round() as usize;
        let mut window = vec![0.0f32; FFT_N];
        for (i, w) in window.iter_mut().enumerate() {
            *w = 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / FFT_N as f32).cos();
        }
        let bin = fs / FFT_N as f32;
        let mut band_bins = [(0, 0); BANDS];
        for z in 0..BANDS {
            let lo = ((EDGES[z] / bin).round() as usize).max(1);
            let hi = ((EDGES[z + 1] / bin).round() as usize).min(FFT_N / 2).max(lo + 1);
            band_bins[z] = (lo, hi);
        }
        let mut oct_bins = [(0, 0); 10];
        for (o, fc) in OCTAVES.iter().enumerate() {
            let lo = ((fc / std::f32::consts::SQRT_2 / bin).round() as usize).max(1);
            let hi = ((fc * std::f32::consts::SQRT_2 / bin).round() as usize).min(FFT_N / 2).max(lo + 1);
            oct_bins[o] = (lo, hi);
        }
        Self {
            fs,
            fs_db_spl: 94.0,
            n: 0,
            kw: [k, k],
            e100: (0.0, 0.0),
            e25: 0.0,
            blocks100: Vec::new(),
            blocks25: Vec::new(),
            n100: (0.1 * fs).round() as usize,
            n25: (0.025 * fs).round() as usize,
            tp: [TruePeak::new(), TruePeak::new()],
            clicks: [Clicks::new(half), Clicks::new(half)],
            click_floor: 10f32.powf(th.click_floor_dbfs / 20.0),
            click_score: 10f32.powf(th.click_score_db / 20.0),
            click_blk: (CLICK_BLOCK_MS * 1e-3 * fs).round() as u64,
            sec_len: fs.round() as usize,
            sec_pos: 0,
            dc: [0.0; 2],
            infra: [Bq::highpass(fs, 3.0, 0.707), Bq::highpass(fs, 3.0, 0.707), Bq::lowpass(fs, 20.0, 0.707), Bq::lowpass(fs, 20.0, 0.707)],
            hp20: Bq::highpass(fs, 20.0, 0.707),
            e_infra: 0.0,
            e_full: 0.0,
            dc_sec: Vec::new(),
            infra_sec: Vec::new(),
            rough: Rough::new(fs),
            ring: [vec![0.0; FFT_N], vec![0.0; FFT_N]],
            ring_pos: 0,
            since: 0,
            window,
            re: vec![0.0; FFT_N],
            im: vec![0.0; FFT_N],
            p: vec![0.0; FFT_N / 2],
            frame: 0,
            model: BandModel::new(fs),
            band_bins,
            oct_bins,
            acc: SecAcc { power: vec![0.0; FFT_N / 2], ..Default::default() },
            sec_stats: Vec::new(),
            tones: Vec::new(),
            tracks: Vec::new(),
            wobbles: Vec::new(),
            wobble_band: (th.wobble_cents, th.wobble_max_cents),
            groups: Vec::new(),
            ctx_t: Vec::new(),
            ctx: Vec::new(),
        }
    }

    /// Seconds analysed so far.
    pub fn seconds(&self) -> f64 {
        self.n as f64 / self.fs as f64
    }

    /// Names of the attribution groups (`context` takes their levels in this order).
    pub fn set_groups(&mut self, names: &[&str]) {
        self.groups = names.iter().map(|s| s.to_string()).collect();
    }

    /// Per-group levels (dB, any reference) at the current time.
    pub fn context(&mut self, levels_db: &[f32]) {
        if self.groups.is_empty() {
            return;
        }
        self.ctx_t.push(self.seconds());
        for g in 0..self.groups.len() {
            self.ctx.push(levels_db.get(g).copied().unwrap_or(-120.0));
        }
    }

    pub fn push(&mut self, l: &[f32], r: &[f32]) {
        for (&a, &b) in l.iter().zip(r) {
            self.sample(a, b);
        }
    }

    #[inline]
    fn sample(&mut self, l: f32, r: f32) {
        // Loudness.
        let kl = { let [a, b] = &mut self.kw[0]; b.run(a.run(l)) as f64 };
        let kr = { let [a, b] = &mut self.kw[1]; b.run(a.run(r)) as f64 };
        self.e100.0 += kl * kl;
        self.e100.1 += kr * kr;
        self.e25 += kl * kl + kr * kr;
        let idx = self.n as usize + 1;
        if idx.is_multiple_of(self.n25) {
            self.blocks25.push((self.e25 / self.n25 as f64) as f32);
            self.e25 = 0.0;
        }
        if idx.is_multiple_of(self.n100) {
            let n = self.n100 as f64;
            self.blocks100.push(((self.e100.0 / n) as f32, (self.e100.1 / n) as f32));
            self.e100 = (0.0, 0.0);
        }
        self.tp[0].push(l);
        self.tp[1].push(r);
        self.click(0, l);
        self.click(1, r);

        // Time domain, per second.
        let m = 0.5 * (l as f64 + r as f64);
        self.dc[0] += l as f64;
        self.dc[1] += r as f64;
        let mut v = m;
        for f in self.infra.iter_mut() {
            v = f.run(v);
        }
        self.e_infra += v * v;
        let h = self.hp20.run(m);
        self.e_full += h * h;
        self.sec_pos += 1;
        if self.sec_pos >= self.sec_len {
            let n = self.sec_len as f64;
            self.dc_sec.push(((self.dc[0] / n) as f32, (self.dc[1] / n) as f32));
            // Seconds of (near) digital silence say nothing about the balance.
            let v = if self.e_full / n > 1e-9 { 10.0 * (self.e_infra / self.e_full).max(1e-12).log10() } else { -120.0 };
            self.infra_sec.push(v as f32);
            self.dc = [0.0; 2];
            self.e_infra = 0.0;
            self.e_full = 0.0;
            self.sec_pos = 0;
        }
        self.rough.push(m);

        // STFT.
        self.ring[0][self.ring_pos] = l;
        self.ring[1][self.ring_pos] = r;
        self.ring_pos = (self.ring_pos + 1) % FFT_N;
        self.since += 1;
        self.n += 1;
        if self.since >= HOP && self.n >= FFT_N as u64 {
            self.since = 0;
            self.stft_frame();
        }
    }

    #[inline]
    fn click(&mut self, ch: usize, x: f32) {
        let c = &mut self.clicks[ch];
        let d2 = x - 2.0 * c.x1 + c.x2;
        c.x2 = c.x1;
        c.x1 = x;
        c.blk_max = c.blk_max.max(d2.abs());
        if (c.n + 1).is_multiple_of(self.click_blk) {
            c.peaks.push(c.blk_max);
            c.blk_max = 0.0;
        }
        let len = c.ring.len();
        let half = len / 2;
        let guard = ((CLICK_GUARD_MS / CLICK_HALF_MS) * half as f32) as usize;
        // Running sums: the whole window and its centre.
        let old = c.ring[c.pos];
        let v = (d2 as f64) * (d2 as f64);
        c.sum_all += v - old;
        c.ring[c.pos] = v;
        // Centre window: [pos - half - guard, pos - half + guard]; entering: pos-half+guard.
        let enter = c.ring[(c.pos + len - half + guard) % len];
        let leave = c.ring[(c.pos + len - half - guard - 1) % len];
        c.sum_mid += enter - leave;
        c.pos = (c.pos + 1) % len;
        c.n += 1;
        if c.n.is_multiple_of(65536) {
            // Re-sum to keep rounding from drifting.
            c.sum_all = c.ring.iter().sum();
            c.sum_mid = (0..=2 * guard).map(|k| c.ring[(c.pos + len - half - guard - 1 + k) % len]).sum();
        }
        if c.n < len as u64 {
            return;
        }
        let centre = c.ring[(c.pos + len - 1 - half) % len];
        let rest = ((c.sum_all - c.sum_mid) / (len - 2 * guard - 1) as f64).max(1e-24);
        let score2 = centre / rest;
        let at = c.n - 1 - half as u64;
        let mag = centre.sqrt() as f32;
        let hit = mag >= self.click_floor && score2 >= (self.click_score as f64).powi(2);
        match (&mut c.open, hit) {
            (Some(o), true) => {
                if score2 > o.1 {
                    *o = (at, score2, mag);
                }
            }
            (None, true) => c.open = Some((at, score2, mag)),
            (Some(o), false) if at > o.0 + (0.02 * self.fs) as u64 => {
                c.found.push((o.0, (10.0 * o.1.log10()) as f32, 20.0 * o.2.log10()));
                c.open = None;
            }
            _ => {}
        }
    }

    fn stft_frame(&mut self) {
        let n = FFT_N;
        for i in 0..n {
            let k = (self.ring_pos + i) % n;
            self.re[i] = self.ring[0][k] * self.window[i];
            self.im[i] = self.ring[1][k] * self.window[i];
        }
        fft(&mut self.re, &mut self.im);
        // Two real spectra from one complex FFT; mean power of the ears, one-sided, FS².
        let scale = 2.0 / (n as f32 * n as f32 * 0.375);
        self.p[0] = 0.0;
        for k in 1..n / 2 {
            let (rk, ik, rn, inn) = (self.re[k], self.im[k], self.re[n - k], self.im[n - k]);
            let (lr, li) = (0.5 * (rk + rn), 0.5 * (ik - inn));
            let (rr, ri) = (0.5 * (ik + inn), -0.5 * (rk - rn));
            self.p[k] = 0.5 * (lr * lr + li * li + rr * rr + ri * ri) * scale;
        }
        let t = (self.n as f64 - n as f64 / 2.0) / self.fs as f64;
        self.frame += 1;

        // Specific loudness of this frame (Pa² at the ears), octave bands, power average.
        let pa = 10f32.powf((self.fs_db_spl - 94.0) / 10.0);
        let mut bp = [0.0f32; BANDS];
        for z in 0..BANDS {
            let (lo, hi) = self.band_bins[z];
            bp[z] = self.p[lo..hi].iter().sum::<f32>() * pa;
        }
        let mut e = [0.0f32; BANDS];
        self.model.excitation(&bp, &mut e);
        for z in 0..BANDS {
            self.acc.spec_loud[z] += self.model.specific(z, e[z], 0.0) as f64;
        }
        for o in 0..10 {
            let (lo, hi) = self.oct_bins[o];
            self.acc.oct[o] += self.p[lo..hi].iter().map(|v| *v as f64).sum::<f64>();
        }
        for (a, v) in self.acc.power.iter_mut().zip(&self.p) {
            *a += *v as f64;
        }
        self.acc.frames += 1;
        self.track_peaks(t);

        // A second is complete when the frame centre passes its end.
        let sec = self.sec_stats.len();
        if t >= (sec + 1) as f64 {
            self.close_second(sec);
        }
    }

    fn close_second(&mut self, sec: usize) {
        let k = self.acc.frames.max(1) as f64;
        let mut ns = [0.0f32; BANDS];
        for z in 0..BANDS {
            ns[z] = (self.acc.spec_loud[z] / k) as f32;
        }
        let total: f32 = ns.iter().sum();
        // DIN 45692 weighting, band z centred at z + 0.5 Bark.
        let mut num = 0.0f32;
        for (z, v) in ns.iter().enumerate() {
            let zb = z as f32 + 0.5;
            let g = if zb <= 15.8 { 1.0 } else { 0.15 * (0.42 * (zb - 15.8)).exp() + 0.85 };
            num += v * g * zb;
        }
        let sharp = if total > 1e-4 { 0.11 * num / total } else { 0.0 };
        let mut oct = [0.0f32; 10];
        for o in 0..10 {
            oct[o] = (10.0 * (self.acc.oct[o] / k).max(1e-20).log10()) as f32;
        }
        // Tonality on the second's mean spectrum.
        let mean: Vec<f32> = self.acc.power.iter().map(|v| (*v / k) as f32).collect();
        self.find_tones(sec, &mean);
        self.sec_stats.push((total, sharp, oct));
        self.acc.frames = 0;
        self.acc.spec_loud = [0.0; BANDS];
        self.acc.oct = [0.0; 10];
        self.acc.power.iter_mut().for_each(|v| *v = 0.0);
    }

    /// Prominent tones of a mean power spectrum (TNR, ECMA-74 Annex D criterion).
    fn find_tones(&mut self, sec: usize, p: &[f32]) {
        let bin = self.fs / FFT_N as f32;
        let total: f32 = p.iter().sum::<f32>().max(1e-30);
        let lo = (100.0 / bin) as usize;
        let hi = ((16_000.0f32.min(0.45 * self.fs)) / bin) as usize;
        let mut found: Vec<Tone> = Vec::new();
        let mut k = lo.max(14);
        while k < hi.min(p.len() - 14) {
            let v = p[k];
            if !(v > p[k - 1] && v >= p[k + 1] && v > p[k - 2] && v >= p[k + 2]) {
                k += 1;
                continue;
            }
            let side = (p[k - 12..k - 4].iter().sum::<f32>() + p[k + 5..k + 13].iter().sum::<f32>()) / 16.0;
            if v < 10.0 * side.max(1e-30) {
                k += 1;
                continue;
            }
            let f = k as f32 * bin;
            let bw = 25.0 + 75.0 * (1.0 + 1.4 * (f / 1000.0).powi(2)).powf(0.69);
            let half = ((0.5 * bw / bin).round() as usize).max(4);
            let (c0, c1) = (k.saturating_sub(half), (k + half + 1).min(p.len()));
            let pc: f32 = p[c0..c1].iter().sum();
            let pt: f32 = p[k - 2..=k + 2].iter().sum();
            let nb = (c1 - c0) as f32;
            let noise = ((pc - pt).max(0.0) / (nb - 5.0).max(1.0) * nb).max(1e-30);
            let tnr = 10.0 * (pt / noise).log10();
            let crit = if f >= 1000.0 { 8.0 } else { 8.0 + 8.33 * (1000.0 / f).log10() };
            let rel = 10.0 * (pt / total).log10();
            if tnr - crit > 0.0 {
                found.push(Tone { sec, f, margin: tnr - crit, rel_db: rel });
            }
            k += 3;
        }
        found.sort_by(|a, b| b.margin.total_cmp(&a.margin));
        found.truncate(8);
        self.tones.extend(found);
    }

    /// Spectral peak tracking for floating tones.
    fn track_peaks(&mut self, t: f64) {
        let bin = self.fs / FFT_N as f32;
        let p = &self.p;
        let lo = ((150.0 / bin) as usize).max(12);
        let hi = ((12_000.0f32.min(0.45 * self.fs)) / bin) as usize;
        let frame_total: f32 = p.iter().sum::<f32>().max(1e-30);
        let mut peaks: Vec<(f32, f32)> = Vec::new();
        let mut k = lo;
        while k < hi.min(p.len() - 12) {
            let v = p[k];
            if v > p[k - 1] && v >= p[k + 1] && v > p[k - 2] && v >= p[k + 2] && v > 1e-5 * frame_total && v > 1e-14 {
                let side = (p[k - 10..k - 3].iter().sum::<f32>() + p[k + 4..k + 11].iter().sum::<f32>()) / 14.0;
                if v >= 15.8 * side {
                    let (a, b, c) = ((p[k - 1] + 1e-30).ln(), (v + 1e-30).ln(), (p[k + 1] + 1e-30).ln());
                    let den = a - 2.0 * b + c;
                    let d = if den.abs() > 1e-12 { (0.5 * (a - c) / den).clamp(-0.5, 0.5) } else { 0.0 };
                    peaks.push(((k as f32 + d) * bin, 10.0 * v.log10()));
                    k += 3;
                    continue;
                }
            }
            k += 1;
        }
        peaks.sort_by(|a, b| b.1.total_cmp(&a.1));
        peaks.truncate(12);
        for tr in self.tracks.iter_mut() {
            tr.matched = false;
        }
        let frame = self.frame;
        for (f, db) in peaks {
            let mut best: Option<(usize, f32)> = None;
            for (i, tr) in self.tracks.iter().enumerate() {
                if tr.matched {
                    continue;
                }
                let cents = (1200.0 * (f / tr.f[tr.f.len() - 1]).log2()).abs();
                if cents < 50.0 && best.is_none_or(|(_, c)| cents < c) {
                    best = Some((i, cents));
                }
            }
            match best {
                Some((i, _)) => {
                    let tr = &mut self.tracks[i];
                    tr.t.push(t as f32);
                    tr.f.push(f);
                    tr.db.push(db);
                    tr.last = frame;
                    tr.matched = true;
                }
                None if self.tracks.len() < 64 => {
                    self.tracks.push(Track { t: vec![t as f32], f: vec![f], db: vec![db], last: frame, matched: true });
                }
                None => {}
            }
        }
        let mut i = 0;
        while i < self.tracks.len() {
            if frame > self.tracks[i].last + 2 {
                let tr = self.tracks.swap_remove(i);
                self.close_track(tr);
            } else {
                i += 1;
            }
        }
    }

    fn close_track(&mut self, tr: Track) {
        let n = tr.t.len();
        let dur = tr.t[n - 1] - tr.t[0];
        if dur < 0.8 || n < 30 {
            return;
        }
        // Detrend the pitch (cents) with a straight line: glides are natural, swings are not.
        let f0 = tr.f[0];
        let c: Vec<f32> = tr.f.iter().map(|f| 1200.0 * (f / f0).log2()).collect();
        let tm = tr.t.iter().sum::<f32>() / n as f32;
        let cm = c.iter().sum::<f32>() / n as f32;
        let (mut sxy, mut sxx) = (0.0f32, 0.0f32);
        for i in 0..n {
            sxy += (tr.t[i] - tm) * (c[i] - cm);
            sxx += (tr.t[i] - tm).powi(2);
        }
        let slope = if sxx > 0.0 { sxy / sxx } else { 0.0 };
        let res: Vec<f32> = (0..n).map(|i| c[i] - cm - slope * (tr.t[i] - tm)).collect();
        // Light smoothing against estimator jitter.
        let sm: Vec<f32> = (0..n).map(|i| {
            let a = res[i.saturating_sub(1)];
            let b = res[(i + 1).min(n - 1)];
            (a + res[i] + b) / 3.0
        }).collect();
        let sigma = (sm.iter().map(|v| v * v).sum::<f32>() / n as f32).sqrt();
        let fmean = tr.f.iter().sum::<f32>() / n as f32;
        let sigma_hz = sigma / 1200.0 * std::f32::consts::LN_2 * fmean;
        // Swings: zero crossings with hysteresis.
        let h = 0.5 * sigma;
        let (mut state, mut crossings) = (0i8, 0u32);
        for v in &sm {
            let s = if *v > h { 1 } else if *v < -h { -1 } else { 0 };
            if s != 0 && s != state {
                if state != 0 {
                    crossings += 1;
                }
                state = s;
            }
        }
        let rate = crossings as f32 / (2.0 * dur);
        let bin = self.fs / FFT_N as f32;
        let level = tr.db.iter().cloned().fold(f32::MIN, f32::max);
        // A sustained tone: its quietest 5 % of frames within 10 dB of its median level (a run
        // of syllables bridged by reverberation is a melody, not a floating tone; a fade in or
        // out is only a few frames).
        let mut lv = tr.db.clone();
        lv.sort_by(|a, b| a.total_cmp(b));
        let sustained = lv[n / 2] - lv[n / 20] < 10.0;
        // COMFORT_DEBUG_TRACKS=1 lists every long tonal track (calibrating the wobble test).
        if std::env::var("COMFORT_DEBUG_TRACKS").is_ok() && dur > 1.5 {
            eprintln!("track {:.2}s +{dur:.2}s f {fmean:.0} sigma {sigma:.1} c ({sigma_hz:.2} Hz) crossings {crossings} rate {rate:.2} sustained {sustained} ({:.1} dB)", tr.t[0], lv[n / 2] - lv[n / 20]);
        }
        if sustained && sigma >= self.wobble_band.0 && sigma <= self.wobble_band.1 && sigma_hz >= 0.1 * bin && crossings >= 4 && (0.5..=10.0).contains(&rate) {
            self.wobbles.push(Event {
                kind: EventKind::Wobble,
                t: tr.t[0] as f64,
                dur,
                value: sigma,
                freq: fmean,
                level_db: level,
                rate,
                group: String::new(),
            });
        }
    }

    /// Gated integrated loudness (BS.1770-4), LUFS.
    fn integrated(&self) -> f32 {
        let lufs = |e: f64| (-0.691 + 10.0 * e.max(1e-20).log10()) as f32;
        let both: Vec<f64> = self.blocks100.iter().map(|(l, r)| (*l + *r) as f64).collect();
        // Momentary blocks (400 ms) with 75 % overlap.
        let mom_e: Vec<f64> = (3..both.len()).map(|i| both[i - 3..=i].iter().sum::<f64>() / 4.0).collect();
        let abs_g: Vec<f64> = mom_e.iter().copied().filter(|e| lufs(*e) > -70.0).collect();
        if abs_g.is_empty() {
            return -70.0;
        }
        let rel = lufs(abs_g.iter().sum::<f64>() / abs_g.len() as f64) - 10.0;
        let g: Vec<f64> = abs_g.iter().copied().filter(|e| lufs(*e) > rel).collect();
        lufs(g.iter().sum::<f64>() / g.len().max(1) as f64)
    }

    /// Group at time `t` that dominates (loudest, or rose most over the last 3 s).
    fn attribute(&self, t: f64) -> String {
        let g = self.groups.len();
        if g == 0 || self.ctx_t.is_empty() {
            return String::new();
        }
        let at = |t: f64| self.ctx_t.partition_point(|x| *x <= t).saturating_sub(1);
        let (i, j) = (at(t + 0.05), at(t - 3.0));
        let now = &self.ctx[i * g..(i + 1) * g];
        let before = &self.ctx[j * g..(j + 1) * g];
        let loudest = now.iter().cloned().fold(f32::MIN, f32::max);
        let mut best = (0usize, f32::MIN);
        for k in 0..g {
            if now[k] < loudest - 12.0 {
                continue;
            }
            let score = now[k] + 0.7 * (now[k] - before[k]).max(0.0);
            if score > best.1 {
                best = (k, score);
            }
        }
        self.groups[best.0].clone()
    }

    pub fn finish(mut self, th: &Thresholds) -> Report {
        // Flush open state.
        for ch in 0..2 {
            if let Some(o) = self.clicks[ch].open.take() {
                self.clicks[ch].found.push((o.0, (10.0 * o.1.log10()) as f32, 20.0 * o.2.log10()));
            }
        }
        let open: Vec<Track> = std::mem::take(&mut self.tracks);
        for tr in open {
            self.close_track(tr);
        }
        let secs = self.seconds();
        let hours = (secs / 3600.0).max(1e-9) as f32;
        let mut events: Vec<Event> = Vec::new();
        let mut checks: Vec<Check> = Vec::new();
        let verdict = |v: f32, l: (f32, f32)| if v > l.1 { Verdict::Fail } else if v > l.0 { Verdict::Warn } else { Verdict::Pass };

        let integrated = self.integrated();
        // --- Clicks (both channels, merged within 20 ms).
        // A click must stand out of the texture: over the third-largest transient of the
        // surrounding ±150 ms (a brook or rain is full of transients of its own).
        let blk = self.click_blk;
        let mut cl: Vec<(u64, f32, f32)> = Vec::new();
        for c in &self.clicks {
            for &(at, score, level) in &c.found {
                let b = (at / blk) as usize;
                let lo = b.saturating_sub(CLICK_NEIGH_BLOCKS);
                let hi = (b + CLICK_NEIGH_BLOCKS + 1).min(c.peaks.len());
                let mut neigh: Vec<f32> = (lo..hi).filter(|&k| k.abs_diff(b) > CLICK_SELF_BLOCKS).map(|k| c.peaks[k]).collect();
                neigh.sort_by(|a, b| b.total_cmp(a));
                let reference = neigh.get(2).copied().unwrap_or(0.0).max(1e-9);
                let isolation = level - 20.0 * reference.log10();
                if isolation >= th.click_isolation_db && level >= integrated + th.click_rel_db {
                    cl.push((at, score.min(isolation + 100.0), level));
                }
            }
        }
        cl.sort_by_key(|c| c.0);
        let mut merged: Vec<(u64, f32, f32)> = Vec::new();
        for c in cl {
            match merged.last_mut() {
                Some(m) if c.0 < m.0 + (0.02 * self.fs) as u64 => {
                    if c.1 > m.1 {
                        *m = c;
                    }
                }
                _ => merged.push(c),
            }
        }
        for c in &merged {
            let t = c.0 as f64 / self.fs as f64;
            events.push(Event { kind: EventKind::Click, t, dur: 0.0, value: c.1, freq: 0.0, level_db: c.2, rate: 0.0, group: self.attribute(t) });
        }
        let worst_click = merged.iter().max_by(|a, b| a.1.total_cmp(&b.1));
        checks.push(Check {
            name: "clicks per hour",
            value: merged.len() as f32 / hours,
            limits: th.clicks_per_hour,
            unit: "/h",
            at: worst_click.map(|c| c.0 as f64 / self.fs as f64),
            group: worst_click.map(|c| self.attribute(c.0 as f64 / self.fs as f64)).unwrap_or_default(),
            verdict: verdict(merged.len() as f32 / hours, th.clicks_per_hour),
        });

        // --- Loudness.
        let b = &self.blocks100;
        let nb = b.len();
        let both: Vec<f64> = b.iter().map(|(l, r)| (*l + *r) as f64).collect();
        let lufs = |e: f64| (-0.691 + 10.0 * e.max(1e-20).log10()) as f32;
        let mean = |a: usize, z: usize| -> f64 {
            if z <= a {
                return 0.0;
            }
            both[a..z].iter().sum::<f64>() / (z - a) as f64
        };
        let momentary: Vec<f32> = (0..nb).map(|i| lufs(mean((i + 1).saturating_sub(4), i + 1))).collect();
        let short: Vec<f32> = (0..nb).map(|i| lufs(mean((i + 1).saturating_sub(30), i + 1))).collect();
        // LRA (EBU Tech 3342).
        let lra = {
            let st: Vec<f32> = (29..nb).map(|i| short[i]).filter(|v| *v > -70.0).collect();
            if st.len() < 10 {
                0.0
            } else {
                let e: f64 = st.iter().map(|v| 10f64.powf((*v as f64 + 0.691) / 10.0)).sum::<f64>() / st.len() as f64;
                let rel = lufs(e) - 20.0;
                let mut g: Vec<f32> = st.into_iter().filter(|v| *v > rel).collect();
                g.sort_by(|a, b| a.total_cmp(b));
                if g.len() < 2 {
                    0.0
                } else {
                    g[(g.len() * 95 / 100).min(g.len() - 1)] - g[g.len() / 10]
                }
            }
        };
        let (mut pk, mut pk_at) = (f32::MIN, 0usize);
        for (i, m) in momentary.iter().enumerate().skip(3) {
            if *m > pk {
                pk = *m;
                pk_at = i;
            }
        }
        let t100 = |i: usize| (i + 1) as f64 * 0.1;
        let v = pk - integrated;
        checks.push(Check { name: "max momentary − integrated", value: v, limits: th.peak_over_integrated_lu, unit: "LU", at: Some(t100(pk_at) - 0.2), group: self.attribute(t100(pk_at) - 0.2), verdict: verdict(v, th.peak_over_integrated_lu) });

        // Surprise: momentary over the energy mean of [t − 30 s, t − 2 s]; judged once 30 s of
        // past exist (the scene's own fade-in is not a surprise).
        let mut prefix = vec![0.0f64; nb + 1];
        for i in 0..nb {
            prefix[i + 1] = prefix[i] + both[i];
        }
        let ctx = |i: usize, back: usize, gap: usize| -> Option<f32> {
            if i < gap + 10 {
                return None;
            }
            let z = i - gap;
            let a = i.saturating_sub(back);
            if z <= a {
                return None;
            }
            Some(lufs((prefix[z] - prefix[a]) / (z - a) as f64))
        };
        let (mut sur, mut sur_at) = (f32::MIN, 0usize);
        let (mut events_n, mut armed) = (0u32, true);
        let mut surprise_events: Vec<(usize, f32)> = Vec::new();
        for i in 300..nb {
            let Some(c) = ctx(i, 300, 20) else { continue };
            let d = momentary[i] - c;
            if d > sur {
                sur = d;
                sur_at = i;
            }
            if armed && d > 6.0 {
                events_n += 1;
                armed = false;
            } else if !armed && d < 3.0 {
                armed = true;
            }
            if d > th.surprise_lu.0 {
                match surprise_events.last_mut() {
                    Some(e) if i < e.0 + 30 => {
                        if d > e.1 {
                            *e = (i, d);
                        }
                    }
                    _ => surprise_events.push((i, d)),
                }
            }
        }
        if sur == f32::MIN {
            sur = 0.0;
        }
        for (i, d) in &surprise_events {
            let t = t100(*i) - 0.2;
            events.push(Event { kind: EventKind::Surprise, t, dur: 0.4, value: *d, freq: 0.0, level_db: momentary[*i], rate: 0.0, group: self.attribute(t) });
        }
        checks.push(Check { name: "loudness surprise (vs 30 s)", value: sur, limits: th.surprise_lu, unit: "LU", at: Some(t100(sur_at) - 0.2), group: self.attribute(t100(sur_at) - 0.2), verdict: verdict(sur, th.surprise_lu) });

        // Sudden: 25 ms level over the energy mean of the previous 2 s (ending 100 ms before).
        let b25 = &self.blocks25;
        let mut pre25 = vec![0.0f64; b25.len() + 1];
        for i in 0..b25.len() {
            pre25[i + 1] = pre25[i] + b25[i] as f64;
        }
        let (mut sud, mut sud_at) = (0.0f32, 0usize);
        let mut sudden_events: Vec<(usize, f32)> = Vec::new();
        for i in 84..b25.len() {
            let (a, z) = (i - 84, i - 4);
            let c = (pre25[z] - pre25[a]) / 80.0;
            let lvl = lufs(b25[i] as f64);
            if lvl < -60.0 {
                continue;
            }
            let d = lvl - lufs(c).max(-70.0);
            if d > sud {
                sud = d;
                sud_at = i;
            }
            if d > th.sudden_db.0 {
                match sudden_events.last_mut() {
                    Some(e) if i < e.0 + 40 => {
                        if d > e.1 {
                            *e = (i, d);
                        }
                    }
                    _ => sudden_events.push((i, d)),
                }
            }
        }
        for (i, d) in &sudden_events {
            let t = *i as f64 * 0.025;
            events.push(Event { kind: EventKind::Sudden, t, dur: 0.025, value: *d, freq: 0.0, level_db: lufs(b25[*i] as f64), rate: 0.0, group: self.attribute(t) });
        }
        checks.push(Check { name: "sudden onset (25 ms vs 2 s)", value: sud, limits: th.sudden_db, unit: "dB", at: Some(sud_at as f64 * 0.025), group: self.attribute(sud_at as f64 * 0.025), verdict: verdict(sud, th.sudden_db) });
        checks.push(Check { name: "loudness range (LRA)", value: lra, limits: th.lra_lu, unit: "LU", at: None, group: String::new(), verdict: verdict(lra, th.lra_lu) });
        let minutes = ((nb.saturating_sub(300)) as f32 / 600.0).max(1e-3);
        let epm = events_n as f32 / minutes;
        checks.push(Check { name: "salient events per minute", value: epm, limits: th.events_per_min, unit: "/min", at: None, group: String::new(), verdict: verdict(epm, th.events_per_min) });
        let true_peak = 20.0 * self.tp[0].peak.max(self.tp[1].peak).max(1e-9).log10();
        checks.push(Check { name: "true peak", value: true_peak, limits: th.true_peak_dbtp, unit: "dBTP", at: None, group: String::new(), verdict: verdict(true_peak, th.true_peak_dbtp) });

        // --- Per-second spectral metrics, sustained over 10 s.
        let ns = self.sec_stats.len();
        let win = 10usize;
        let median = |v: &mut Vec<f32>| -> f32 {
            if v.is_empty() {
                return 0.0;
            }
            v.sort_by(|a, b| a.total_cmp(b));
            v[v.len() / 2]
        };
        // Quiet seconds (under 1 sone ≈ 40 phon) cannot be harsh: skipped for sharpness.
        let (mut sh, mut sh_at) = (0.0f32, 0usize);
        for s in (0..ns.saturating_sub(win - 1)).step_by(2) {
            let mut v: Vec<f32> = (s..s + win).filter(|&i| self.sec_stats[i].0 > 1.0).map(|i| self.sec_stats[i].1).collect();
            if v.len() < win / 2 {
                continue;
            }
            let m = median(&mut v);
            if m > sh {
                sh = m;
                sh_at = s;
            }
        }
        checks.push(Check { name: "sharpness (10 s median)", value: sh, limits: th.sharpness_acum, unit: "acum", at: Some(sh_at as f64), group: self.attribute(sh_at as f64 + 5.0), verdict: verdict(sh, th.sharpness_acum) });
        let rough = &self.rough.out;
        let rw = (win as f32 / ROUGH_WIN_S) as usize;
        let (mut ro, mut ro_at) = (0.0f32, 0usize);
        for s in (0..rough.len().saturating_sub(rw - 1)).step_by(4) {
            let mut v: Vec<f32> = rough[s..s + rw].to_vec();
            let m = median(&mut v);
            if m > ro {
                ro = m;
                ro_at = s;
            }
        }
        let ro_t = ro_at as f64 * ROUGH_WIN_S as f64;
        checks.push(Check { name: "roughness (10 s median)", value: ro, limits: th.roughness_asper, unit: "asper", at: Some(ro_t), group: self.attribute(ro_t + 5.0), verdict: verdict(ro, th.roughness_asper) });

        // Octave balance over 10 s windows (power mean), against the reference tilt.
        // Per-second excess over the reference tilt; a window's value is the 20th percentile
        // of its seconds: the steady background between bird phrases, not the phrases.
        let tilt = |o: usize| REF_TILT_DB_OCT * (OCTAVES[o] / 500.0).log2();
        let sec_excess = |i: usize, bands: &[usize]| -> f32 {
            let o = &self.sec_stats[i].2;
            let e: f64 = bands.iter().map(|&k| 10f64.powf((o[k] - o[4] - tilt(k)) as f64 / 10.0)).sum::<f64>() / bands.len() as f64;
            (10.0 * e.max(1e-20).log10()) as f32
        };
        let excess = |s: usize, bands: &[usize]| -> f32 {
            let mut v: Vec<f32> = (s..s + win).map(|i| sec_excess(i, bands)).collect();
            v.sort_by(|a, b| a.total_cmp(b));
            v[win / 5]
        };
        let mut worst = [(f32::MIN, 0usize); 3];
        for s in (0..ns.saturating_sub(win - 1)).step_by(2) {
            // Only where the mix is audible at all (10 s loudness above 0.5 sone).
            let sone: f32 = (s..s + win).map(|i| self.sec_stats[i].0).sum::<f32>() / win as f32;
            if sone < 0.5 {
                continue;
            }
            for (k, bands) in [&[6usize, 7][..], &[8, 9][..], &[0, 1][..]].iter().enumerate() {
                let v = excess(s, bands);
                if v > worst[k].0 {
                    worst[k] = (v, s);
                }
            }
        }
        for (k, (name, lim)) in [("presence 2–5.6 kHz excess", th.presence_db), ("hiss 5.6–22 kHz excess", th.hiss_db), ("rumble 22–89 Hz excess", th.rumble_db)].into_iter().enumerate() {
            let (v, s) = worst[k];
            let v = if v == f32::MIN { 0.0 } else { v };
            checks.push(Check { name, value: v, limits: lim, unit: "dB", at: Some(s as f64), group: self.attribute(s as f64 + 5.0), verdict: verdict(v, lim) });
        }
        // Infrasound and DC, sustained over 10 s.
        let (mut inf, mut inf_at) = (-120.0f32, 0usize);
        for s in 0..self.infra_sec.len().saturating_sub(win - 1) {
            let e: f32 = (s..s + win).map(|i| 10f32.powf(self.infra_sec[i] / 10.0)).sum::<f32>() / win as f32;
            let v = 10.0 * e.max(1e-12).log10();
            if v > inf {
                inf = v;
                inf_at = s;
            }
        }
        checks.push(Check { name: "infrasound 3–20 Hz", value: inf, limits: th.infra_db, unit: "dB", at: Some(inf_at as f64), group: self.attribute(inf_at as f64 + 5.0), verdict: verdict(inf, th.infra_db) });
        // DC: the mean over 60 s windows (the whole run if shorter): shorter means also see
        // the slowest infrasound, which the infrasound check judges.
        let dw = 60.min(self.dc_sec.len()).max(1);
        let dc = (0..=self.dc_sec.len().saturating_sub(dw))
            .step_by(10)
            .map(|s| {
                let (a, b) = self.dc_sec[s..(s + dw).min(self.dc_sec.len())].iter().fold((0.0f32, 0.0f32), |m, x| (m.0 + x.0, m.1 + x.1));
                (a / dw as f32).abs().max((b / dw as f32).abs())
            })
            .fold(0.0f32, f32::max);
        let dc_db = 20.0 * dc.max(1e-9).log10();
        checks.push(Check { name: "DC offset", value: dc_db, limits: th.dc_dbfs, unit: "dBFS", at: None, group: String::new(), verdict: verdict(dc_db, th.dc_dbfs) });

        // --- Sustained tones: chains of seconds with a prominent tone within 3 %.
        struct Chain {
            start: usize,
            last: usize,
            f: f32,
            margins: Vec<f32>,
            rel: f32,
        }
        let mut chains: Vec<Chain> = Vec::new();
        let mut done: Vec<Chain> = Vec::new();
        let mut tones = self.tones.clone();
        tones.retain(|t| t.margin >= th.tone_margin_db && t.rel_db >= -th.tone_rel_db);
        tones.sort_by_key(|t| t.sec);
        for t in tones {
            let mut i = 0;
            while i < chains.len() {
                if chains[i].last + 2 < t.sec {
                    done.push(chains.swap_remove(i));
                } else {
                    i += 1;
                }
            }
            if let Some(c) = chains.iter_mut().find(|c| c.last < t.sec && ((t.f / c.f).ln()).abs() < 0.03) {
                c.last = t.sec;
                c.f = 0.8 * c.f + 0.2 * t.f;
                c.margins.push(t.margin);
                c.rel = c.rel.max(t.rel_db);
            } else {
                chains.push(Chain { start: t.sec, last: t.sec, f: t.f, margins: vec![t.margin], rel: t.rel_db });
            }
        }
        done.extend(chains);
        let mut longest = (0.0f32, 0usize, 0.0f32);
        for c in done.iter_mut() {
            let dur = (c.last - c.start + 1) as f32;
            // The check counts tones from 1 kHz up (whistles, whines, a shrill insect): a low
            // hoot or coo is tonal by nature and not harsh. Lower ones are listed, not judged.
            if dur > longest.0 && c.f >= TONE_MIN_HZ {
                longest = (dur, c.start, c.f);
            }
            if dur >= th.tone_s.0.min(10.0) {
                let m = median(&mut c.margins);
                let t = c.start as f64;
                events.push(Event { kind: EventKind::Tone, t, dur, value: m, freq: c.f, level_db: c.rel, rate: 0.0, group: self.attribute(t + (dur as f64) / 2.0) });
            }
        }
        checks.push(Check {
            name: "longest prominent tone ≥ 1 kHz",
            value: longest.0,
            limits: th.tone_s,
            unit: "s",
            at: Some(longest.1 as f64),
            group: if longest.0 > 0.0 { format!("{} @ {:.0} Hz", self.attribute(longest.1 as f64 + longest.0 as f64 / 2.0), longest.2) } else { String::new() },
            verdict: verdict(longest.0, th.tone_s),
        });

        // --- Floating tones.
        let wob = std::mem::take(&mut self.wobbles);
        let worst_w = wob.iter().max_by(|a, b| a.value.total_cmp(&b.value)).map(|w| (w.t, w.value));
        let wph = wob.len() as f32 / hours;
        for mut w in wob {
            w.group = self.attribute(w.t + w.dur as f64 / 2.0);
            events.push(w);
        }
        checks.push(Check {
            name: "floating tones per hour",
            value: wph,
            limits: th.wobbles_per_hour,
            unit: "/h",
            at: worst_w.map(|w| w.0),
            group: worst_w.map(|w| self.attribute(w.0)).unwrap_or_default(),
            verdict: verdict(wph, th.wobbles_per_hour),
        });

        // --- Left/right balance over 5 min windows (step 30 s; the whole run if shorter).
        let span = 3000.min(nb);
        let mut ild60: Vec<(f64, f32)> = Vec::new();
        let mut s = 0usize;
        while span >= 600 && s + span <= nb {
            let (l, r) = b[s..s + span].iter().fold((0.0f64, 0.0f64), |a, x| (a.0 + x.0 as f64, a.1 + x.1 as f64));
            if l + r > 1e-12 {
                ild60.push((s as f64 * 0.1, (10.0 * (l.max(1e-20) / r.max(1e-20)).log10()) as f32));
            }
            s += 300;
        }
        let worst_ild = ild60.iter().copied().max_by(|a, b| a.1.abs().total_cmp(&b.1.abs())).unwrap_or((0.0, 0.0));
        let drift = if ild60.is_empty() { 0.0 } else { ild60.iter().map(|x| x.1).fold(f32::MIN, f32::max) - ild60.iter().map(|x| x.1).fold(f32::MAX, f32::min) };
        checks.push(Check { name: "L/R imbalance (5 min)", value: worst_ild.1.abs(), limits: th.ild_db, unit: "dB", at: Some(worst_ild.0), group: if worst_ild.1 > 0.0 { "left louder".into() } else { "right louder".into() }, verdict: verdict(worst_ild.1.abs(), th.ild_db) });
        checks.push(Check { name: "L/R balance drift", value: drift, limits: th.ild_drift_db, unit: "dB", at: None, group: String::new(), verdict: verdict(drift, th.ild_drift_db) });

        // --- Timeline.
        let nsec = ns.max(nb / 10);
        let mut timeline = vec![Second::default(); nsec];
        for (i, sec) in timeline.iter_mut().enumerate() {
            let (a, z) = (i * 10, ((i + 1) * 10).min(nb));
            if z > a {
                sec.momentary_max = momentary[a..z].iter().cloned().fold(f32::MIN, f32::max);
                sec.short_term = short[z - 1];
                let (l, r) = b[a..z].iter().fold((0.0f64, 0.0f64), |s, x| (s.0 + x.0 as f64, s.1 + x.1 as f64));
                sec.ild_db = (10.0 * (l.max(1e-20) / r.max(1e-20)).log10()) as f32;
            }
            if let Some(st) = self.sec_stats.get(i) {
                sec.sone = st.0;
                sec.sharpness = st.1;
                sec.octaves_db = st.2;
            }
            let r0 = (i as f32 / ROUGH_WIN_S) as usize;
            let rv: Vec<f32> = rough.iter().skip(r0).take((1.0 / ROUGH_WIN_S) as usize).copied().collect();
            sec.roughness = if rv.is_empty() { 0.0 } else { rv.iter().sum::<f32>() / rv.len() as f32 };
            sec.infra_db = self.infra_sec.get(i).copied().unwrap_or(-120.0);
            sec.group = self.attribute(i as f64 + 0.5);
        }
        for e in &events {
            let i = e.t as usize;
            if let Some(s) = timeline.get_mut(i) {
                match e.kind {
                    EventKind::Click => s.clicks += 1,
                    EventKind::Wobble => s.wobbles += 1,
                    _ => {}
                }
            }
        }
        for t in &self.tones {
            if t.margin >= th.tone_margin_db && t.rel_db >= -th.tone_rel_db {
                if let Some(s) = timeline.get_mut(t.sec) {
                    s.tones += 1;
                }
            }
        }
        events.sort_by(|a, b| a.t.total_cmp(&b.t));
        Report { seconds: secs, integrated, lra, true_peak, checks, events, timeline, groups: self.groups.clone() }
    }
}

impl Report {
    /// Joins reports of consecutive takes (analysed separately, e.g. in parallel) into one:
    /// rates are pooled over the total time, levels and lengths take the worst take, events
    /// and timelines are shifted to one clock.
    pub fn merge(parts: Vec<Report>) -> Report {
        let total: f64 = parts.iter().map(|p| p.seconds).sum();
        let mut out = Report { seconds: total, integrated: -70.0, lra: 0.0, true_peak: -120.0, checks: Vec::new(), events: Vec::new(), timeline: Vec::new(), groups: Vec::new() };
        let mut energy = 0.0f64;
        let mut offset = 0.0f64;
        for (k, p) in parts.into_iter().enumerate() {
            energy += p.seconds * 10f64.powf((p.integrated as f64 + 0.691) / 10.0);
            out.lra = out.lra.max(p.lra);
            out.true_peak = out.true_peak.max(p.true_peak);
            for mut e in p.events {
                e.t += offset;
                out.events.push(e);
            }
            out.timeline.extend(p.timeline);
            out.groups = p.groups;
            for c in p.checks {
                let rate = c.unit.starts_with('/');
                match out.checks.iter_mut().find(|x| x.name == c.name) {
                    None => {
                        let mut c = c;
                        c.at = c.at.map(|t| t + offset);
                        if rate {
                            c.value *= p.seconds as f32;
                        }
                        out.checks.push(c);
                    }
                    Some(x) if rate => x.value += c.value * p.seconds as f32,
                    Some(x) => {
                        if c.value > x.value {
                            *x = Check { at: c.at.map(|t| t + offset), ..c };
                        }
                    }
                }
            }
            let _ = k;
            offset += p.seconds;
        }
        for c in out.checks.iter_mut() {
            if c.unit.starts_with('/') {
                c.value /= total.max(1e-9) as f32;
            }
            c.verdict = if c.value > c.limits.1 {
                Verdict::Fail
            } else if c.value > c.limits.0 {
                Verdict::Warn
            } else {
                Verdict::Pass
            };
        }
        out.integrated = (-0.691 + 10.0 * (energy / total.max(1e-9)).max(1e-20).log10()) as f32;
        out.events.sort_by(|a, b| a.t.total_cmp(&b.t));
        out
    }

    pub fn verdict(&self) -> Verdict {
        self.checks.iter().map(|c| c.verdict).max().unwrap_or(Verdict::Pass)
    }

    pub fn check(&self, name: &str) -> Option<&Check> {
        self.checks.iter().find(|c| c.name.starts_with(name))
    }

    pub fn count(&self, kind: EventKind) -> usize {
        self.events.iter().filter(|e| e.kind == kind).count()
    }

    /// Human-readable report: checks, then the worst offenders of each kind.
    pub fn text(&self, worst: usize) -> String {
        use std::fmt::Write;
        let mut s = String::new();
        let _ = writeln!(s, "analysed {}  integrated {:.1} LUFS  LRA {:.1} LU  true peak {:.1} dBTP", clock(self.seconds), self.integrated, self.lra, self.true_peak);
        let _ = writeln!(s);
        let _ = writeln!(s, "  {:<30} {:>12}  {:>15} {:<5}  worst at", "check", "value", "warn / fail", "");
        for c in &self.checks {
            let at = c.at.map(clock).unwrap_or_default();
            let _ = writeln!(
                s,
                "  {:<30} {:>7.2} {:<4} {:>6.1} / {:<6.1} {:<5}  {} {}",
                c.name,
                c.value,
                c.unit,
                c.limits.0,
                c.limits.1,
                c.verdict.name(),
                at,
                c.group
            );
        }
        for kind in [EventKind::Click, EventKind::Surprise, EventKind::Sudden, EventKind::Tone, EventKind::Wobble] {
            let mut ev: Vec<&Event> = self.events.iter().filter(|e| e.kind == kind).collect();
            if ev.is_empty() {
                continue;
            }
            ev.sort_by(|a, b| b.value.total_cmp(&a.value));
            let _ = writeln!(s, "\n  {} ({}), worst first:", kind.name(), ev.len());
            for e in ev.iter().take(worst) {
                let what = match kind {
                    EventKind::Click => format!("{:5.1} dB over local HF, peak {:5.1} dBFS", e.value, e.level_db),
                    EventKind::Surprise => format!("{:+5.1} LU over the last 30 s (M {:5.1} LUFS)", e.value, e.level_db),
                    EventKind::Sudden => format!("{:+5.1} dB in 25 ms over the last 2 s", e.value),
                    EventKind::Tone => format!("{:6.0} Hz for {:4.0} s, TNR {:4.1} dB over the ECMA criterion, {:5.1} dB re total", e.freq, e.dur, e.value, e.level_db),
                    EventKind::Wobble => format!("{:6.0} Hz for {:3.1} s, {:4.1} cents RMS at {:3.1} Hz", e.freq, e.dur, e.value, e.rate),
                };
                let _ = writeln!(s, "    {}  {:<50}  {}", clock(e.t), what, e.group);
            }
        }
        let _ = writeln!(s, "\nverdict: {}", self.verdict().name());
        s
    }

    /// Per-second timeline as CSV.
    pub fn csv(&self) -> String {
        use std::fmt::Write;
        let mut s = String::from("second,momentary_max_lufs,short_term_lufs,sone,sharpness_acum,roughness_asper");
        for o in OCTAVES {
            let _ = write!(s, ",oct{o}_db");
        }
        s.push_str(",infra_db,ild_db,clicks,tones,wobbles,group\n");
        for (i, t) in self.timeline.iter().enumerate() {
            let _ = write!(s, "{i},{:.2},{:.2},{:.3},{:.3},{:.3}", t.momentary_max, t.short_term, t.sone, t.sharpness, t.roughness);
            for v in t.octaves_db {
                let _ = write!(s, ",{v:.1}");
            }
            let _ = writeln!(s, ",{:.1},{:.2},{},{},{},{}", t.infra_db, t.ild_db, t.clicks, t.tones, t.wobbles, t.group);
        }
        s
    }
}

/// h:mm:ss.s
pub fn clock(t: f64) -> String {
    let h = (t / 3600.0) as u32;
    let m = ((t % 3600.0) / 60.0) as u32;
    let s = t % 60.0;
    if h > 0 {
        format!("{h}:{m:02}:{s:04.1}")
    } else {
        format!("{m:02}:{s:04.1}")
    }
}

#[cfg(test)]
#[allow(clippy::excessive_precision)]
mod tests {
    use super::*;
    use crate::rng::Rng;

    const FS: f32 = 48_000.0;

    /// Pink-ish noise bed (Kellet filter) at about `db` dBFS RMS.
    fn pink(secs: f32, db: f32, seed: u64) -> Vec<f32> {
        let mut rng = Rng::new(seed);
        let n = (secs * FS) as usize;
        let mut b = [0.0f32; 7];
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            let w = rng.bipolar();
            b[0] = 0.99886 * b[0] + w * 0.0555179;
            b[1] = 0.99332 * b[1] + w * 0.0750759;
            b[2] = 0.96900 * b[2] + w * 0.1538520;
            b[3] = 0.86650 * b[3] + w * 0.3104856;
            b[4] = 0.55000 * b[4] + w * 0.5329522;
            b[5] = -0.7616 * b[5] - w * 0.0168980;
            out.push(b[0] + b[1] + b[2] + b[3] + b[4] + b[5] + b[6] + w * 0.5362);
            b[6] = w * 0.115926;
        }
        // No DC or infrasound in the test bed.
        let mut hp = [Bq::highpass(FS, 30.0, 0.707), Bq::highpass(FS, 30.0, 0.707)];
        out.iter_mut().for_each(|v| *v = run2(&mut hp, *v as f64) as f32);
        let rms = (out.iter().map(|v| v * v).sum::<f32>() / n as f32).sqrt();
        let g = 10f32.powf(db / 20.0) / rms;
        out.iter_mut().for_each(|v| *v *= g);
        out
    }

    fn analyse(l: &[f32], r: &[f32]) -> Report {
        let mut a = Analyzer::new(FS);
        a.push(l, r);
        a.finish(&Thresholds::default())
    }

    #[test]
    fn clean_noise_passes() {
        let l = pink(40.0, -30.0, 1);
        let r = pink(40.0, -30.0, 2);
        let rep = analyse(&l, &r);
        eprintln!("{}", rep.text(3));
        assert_eq!(rep.count(EventKind::Click), 0);
        assert_eq!(rep.count(EventKind::Wobble), 0);
        assert_eq!(rep.count(EventKind::Tone), 0);
        assert!(rep.check("loudness surprise").unwrap().value < 3.0);
        assert!(rep.check("roughness").unwrap().value < 0.15, "noise roughness");
    }

    #[test]
    fn finds_injected_defects() {
        let secs = 40.0;
        let mut l = pink(secs, -30.0, 3);
        let mut r = pink(secs, -30.0, 4);
        // A click (one sample, -12 dBFS) at 12.5 s.
        let i = (12.5 * FS) as usize;
        l[i] += 0.25;
        r[i] += 0.25;
        // A floating tone: 1.5 kHz with ±0.8 % at 3 Hz for 4 s at 18 s.
        let mut ph = 0.0f32;
        for k in 0..(4.0 * FS) as usize {
            let t = k as f32 / FS;
            let f = 1500.0 * (1.0 + 0.008 * (std::f32::consts::TAU * 3.0 * t).sin());
            ph += std::f32::consts::TAU * f / FS;
            let v = 0.01 * ph.sin();
            l[(18.0 * FS) as usize + k] += v;
            r[(18.0 * FS) as usize + k] += v;
        }
        let rep = analyse(&l, &r);
        eprintln!("{}", rep.text(3));
        let clicks: Vec<&Event> = rep.events.iter().filter(|e| e.kind == EventKind::Click).collect();
        assert_eq!(clicks.len(), 1);
        assert!((clicks[0].t - 12.5).abs() < 0.01);
        let w: Vec<&Event> = rep.events.iter().filter(|e| e.kind == EventKind::Wobble).collect();
        assert!(!w.is_empty() && w.iter().all(|e| (17.9..22.0).contains(&e.t)), "{w:?}");
        assert!((w[0].rate - 3.0).abs() < 0.8, "{w:?}");
    }

    #[test]
    fn steady_tone_is_not_a_wobble_but_a_tone() {
        let mut l = pink(40.0, -30.0, 5);
        let mut r = pink(40.0, -30.0, 6);
        for k in 0..(30.0 * FS) as usize {
            let v = 0.05 * (std::f32::consts::TAU * 4000.0 * k as f32 / FS).sin();
            l[(5.0 * FS) as usize + k] += v;
            r[(5.0 * FS) as usize + k] += v;
        }
        let rep = analyse(&l, &r);
        eprintln!("{}", rep.text(3));
        assert_eq!(rep.count(EventKind::Wobble), 0);
        let t = rep.check("longest prominent tone").unwrap();
        assert!(t.value >= 28.0, "{t:?}");
    }

    #[test]
    fn burst_is_a_surprise() {
        let mut l = pink(45.0, -30.0, 7);
        let mut r = pink(45.0, -30.0, 8);
        let g = 10f32.powf(15.0 / 20.0);
        for k in (35.0 * FS) as usize..(36.0 * FS) as usize {
            l[k] *= g;
            r[k] *= g;
        }
        let rep = analyse(&l, &r);
        eprintln!("{}", rep.text(3));
        let s = rep.check("loudness surprise").unwrap();
        assert!(s.value > 13.0 && (s.at.unwrap() - 35.5).abs() < 0.8, "{s:?}");
    }

    #[test]
    fn sharpness_reference() {
        // Critical-band noise at 1 kHz, 60 dB SPL = 1 acum (DIN 45692).
        let n = pink(12.0, 0.0, 9);
        let mut f = [Bq::bandpass(FS, 1000.0, 1000.0 / 160.0), Bq::bandpass(FS, 1000.0, 1000.0 / 160.0)];
        let mut x: Vec<f32> = n.iter().map(|v| run2(&mut f, *v as f64) as f32).collect();
        let rms = (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt();
        let g = 10f32.powf((60.0 - 94.0) / 20.0) / rms;
        x.iter_mut().for_each(|v| *v *= g);
        let rep = analyse(&x, &x);
        let s = rep.check("sharpness").unwrap().value;
        assert!((0.8..1.25).contains(&s), "sharpness {s}");
    }

    #[test]
    fn roughness_reference() {
        let tone = |m: f32| -> Vec<f32> {
            let a = 10f32.powf((60.0 - 94.0) / 20.0) * std::f32::consts::SQRT_2;
            (0..(12.0 * FS) as usize)
                .map(|k| {
                    let t = k as f32 / FS;
                    a * (1.0 + m * (std::f32::consts::TAU * 70.0 * t).sin()) / (1.0 + m * m / 2.0).sqrt() * (std::f32::consts::TAU * 1000.0 * t).sin()
                })
                .collect()
        };
        let x = tone(1.0);
        let r1 = analyse(&x, &x).check("roughness").unwrap().value;
        let x = tone(0.0);
        let r0 = analyse(&x, &x).check("roughness").unwrap().value;
        eprintln!("roughness: AM {r1:.3} asper, steady {r0:.3}");
        assert!((0.8..1.25).contains(&r1), "AM tone {r1}");
        assert!(r0 < 0.05, "steady tone {r0}");
    }
}
