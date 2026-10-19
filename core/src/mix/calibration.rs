//! Source-level calibration: every emitter class gets a physically grounded level, so the
//! mix starts from the loudness relations of a real forest instead of hand-set kind gains.
//!
//! **Unit.** After calibration an emitter's signal is the sound pressure in pascals it
//! produces at 1 m (1.0 RMS = 94 dB SPL). Propagation then turns it into pascals at the ears.
//! The generators' own physics (rain rate, wind speed, flow regime, number of crickets, a
//! bird's individual `loudness`) is kept: calibration is one constant per class, fitted at a
//! stated reference condition (`examples/calibrate.rs` measures the raw generators and checks
//! the result; `docs/sound-director.md` lists sources).
//!
//! **Playback.** Real forest levels (40–60 dB(A) at the listener) would be inaudibly quiet at
//! digital full scale, so the output stage maps `0 dBFS = PLAYBACK_REF_DB` dB SPL; the mix
//! dynamics (dynamics.rs) then levels around the target loudness.

use crate::director::Species;
use crate::sources::ModelKind;

/// Sound-pressure unit of calibrated signals: 1.0 = 1 Pa = 94 dB SPL.
pub const PA_DB: f32 = 93.979;

/// dB SPL at the ears that corresponds to 0 dBFS before the mix dynamics: a lively scene at
/// ~55–60 dB(A) lands near the −22 LUFS target, a quiet night at 35 dB(A) well below it (the
/// leveller lifts it only partly).
pub const PLAYBACK_REF_DB: f32 = 78.0;

/// What made a sound: the calibration and attention class of an emitter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SoundClass {
    Water,
    /// Wind in tree crowns (foliage model).
    Foliage,
    /// Free wind sources: distant roar, whistles, gaps.
    Wind,
    Rain,
    Thunder,
    Fire,
    Insects,
    Bird(Species),
}

impl SoundClass {
    pub fn of_model(m: ModelKind) -> Self {
        match m {
            ModelKind::Wind => SoundClass::Wind,
            ModelKind::Foliage => SoundClass::Foliage,
            ModelKind::Water => SoundClass::Water,
            ModelKind::Rain => SoundClass::Rain,
            ModelKind::Thunder => SoundClass::Thunder,
            ModelKind::Fire => SoundClass::Fire,
            ModelKind::Insects => SoundClass::Insects,
        }
    }

    /// Best guess from the engine kind alone (emitters without a model config).
    pub fn of_kind(k: crate::engine::SourceKind) -> Self {
        use crate::engine::SourceKind as K;
        match k {
            K::Water => SoundClass::Water,
            K::Wind => SoundClass::Foliage,
            K::Bird => SoundClass::Bird(Species::Chaffinch),
            K::Rain => SoundClass::Rain,
            K::Thunder => SoundClass::Thunder,
            K::Fire => SoundClass::Fire,
            K::Insect => SoundClass::Insects,
        }
    }

    /// Mix group (telemetry, focus intent, masking tracks).
    pub fn group(self) -> usize {
        match self {
            SoundClass::Water => group::WATER,
            SoundClass::Foliage | SoundClass::Wind => group::WIND,
            SoundClass::Bird(_) => group::BIRDS,
            SoundClass::Rain => group::RAIN,
            SoundClass::Thunder => group::THUNDER,
            SoundClass::Insects => group::INSECTS,
            SoundClass::Fire => group::FIRE,
        }
    }

    /// Event-like sounds that can be lifted into the foreground as soloists. Continuous
    /// textures (water, wind, rain) form the bed; the whole group can still be put in focus.
    pub fn soloable(self) -> bool {
        matches!(self, SoundClass::Bird(_) | SoundClass::Thunder | SoundClass::Fire | SoundClass::Insects)
    }

    /// How rare (and so how attention-grabbing) an event of this class is, 0..1.
    pub fn rarity(self) -> f32 {
        match self {
            SoundClass::Thunder => 1.0,
            SoundClass::Bird(s) => match s {
                Species::Cuckoo => 0.8,
                Species::TawnyOwl => 0.75,
                Species::Nightingale => 0.55,
                Species::Swallow => 0.45,
                Species::SongThrush => 0.35,
                Species::WoodPigeon => 0.3,
                Species::Robin => 0.3,
                Species::Chaffinch => 0.2,
                Species::Blackbird => 0.3,
                Species::Skylark => 0.45,
                Species::Gull => 0.4,
                Species::Raven => 0.6,
            },
            SoundClass::Fire => 0.3,
            SoundClass::Insects => 0.1,
            _ => 0.0,
        }
    }

    /// Calibration gain: raw generator units -> pascals at 1 m.
    pub fn gain(self) -> f32 {
        let e = entry(self);
        10f32.powf((e.target_db - e.raw_db) / 20.0) * e.post
    }
}

/// Mix groups (mirror `ASMR_MIX_GROUP_*`).
pub mod group {
    pub const WATER: usize = 0;
    pub const WIND: usize = 1;
    pub const BIRDS: usize = 2;
    pub const RAIN: usize = 3;
    pub const THUNDER: usize = 4;
    pub const INSECTS: usize = 5;
    pub const FIRE: usize = 6;
    pub const COUNT: usize = 7;
    pub const NAMES: [&str; COUNT] = ["water", "wind", "birds", "rain", "thunder", "insects", "fire"];
}

/// How a target level is defined.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Metric {
    /// A-weighted fast-time-weighted maximum at 1 m while vocalising (bird song).
    LaFmax1m,
    /// A-weighted equivalent level at 1 m of a continuous source.
    LaEq1m,
    /// A-weighted equivalent level at the listener in the default scene (distributed sources).
    LaEqScene,
    /// Mean LAFmax at the listener per thunder clap (storm 1, strikes ~0.5–3 km).
    LaFmaxScene,
}

#[derive(Clone, Copy, Debug)]
pub struct Entry {
    pub class: SoundClass,
    pub metric: Metric,
    /// Target level, dB SPL (A-weighted).
    pub target_db: f32,
    /// Measured level of the raw generator at the reference condition with unit gain,
    /// in dB re 1 Pa-equivalent (i.e. as if raw 1.0 RMS were 94 dB) — `examples/calibrate.rs`.
    pub raw_db: f32,
    /// Extra linear factor (the thunder proxy's 1/r undo).
    pub post: f32,
    pub condition: &'static str,
    pub source: &'static str,
}

const fn e(class: SoundClass, metric: Metric, target_db: f32, raw_db: f32, condition: &'static str, source: &'static str) -> Entry {
    Entry { class, metric, target_db, raw_db, post: 1.0, condition, source }
}

use Metric::*;
use SoundClass as C;

/// The calibration table. `raw_db` comes from `cargo run --release --example calibrate`.
#[rustfmt::skip]
pub static TABLE: [Entry; 18] = [
    e(C::Bird(Species::Chaffinch), LaFmax1m, 86.0, 79.7, "full song", "Brumm & Todt 2002; Brumm 2004 review: passerine song 80–95 dB(A) at 1 m"),
    e(C::Bird(Species::Robin), LaFmax1m, 82.0, 77.3, "song", "Brumm & Todt 2002 (robins 78–87 dB at 1 m)"),
    e(C::Bird(Species::SongThrush), LaFmax1m, 88.0, 78.6, "song", "Brackenbury 1979 (thrushes ~90 dB at 1 m)"),
    e(C::Bird(Species::Nightingale), LaFmax1m, 90.0, 80.7, "song", "Brumm 2004 (nightingale 84–95 dB at 1 m, noise dependent)"),
    e(C::Bird(Species::TawnyOwl), LaFmax1m, 80.0, 81.6, "hoot", "Galeotti & Pavan 1991 / Appleby & Redpath 1997 (hoots audible >1 km)"),
    e(C::Bird(Species::Cuckoo), LaFmax1m, 88.0, 79.2, "cu-coo call", "carries 1–2 km; ~85–90 dB at 1 m"),
    e(C::Bird(Species::WoodPigeon), LaFmax1m, 74.0, 77.7, "cooing", "low-pitched coo, ~70–78 dB at 1 m"),
    e(C::Bird(Species::Swallow), LaFmax1m, 74.0, 77.5, "twitter", "soft twittering song, ~70–78 dB at 1 m"),
    e(C::Bird(Species::Blackbird), LaFmax1m, 84.0, 80.6, "song", "blackbird song ~78–90 dB at 1 m (Brumm 2004 review; Nemeth & Brumm 2009)"),
    e(C::Bird(Species::Skylark), LaFmax1m, 80.0, 76.8, "song (perched)", "skylark song ~75–85 dB at 1 m, heard from 50 m up"),
    e(C::Bird(Species::Gull), LaFmax1m, 90.0, 81.0, "long call", "herring gull long call ~85–95 dB at 1 m (estimate)"),
    e(C::Bird(Species::Raven), LaFmax1m, 86.0, 77.9, "croak", "raven croaks carry >1 km; ~80–90 dB at 1 m (estimate)"),
    e(C::Water, LaEq1m, 57.0, 52.6, "brook_pebbles segment, WATER 0.6 (0.8 m/s, 12 cm deep)", "small stream 55–70 dB(A) at 1 m; 45–55 at 5 m (soundscape surveys)"),
    e(C::Foliage, LaEqScene, 44.0, 69.5, "default scene canopy, 4.8 m/s at crown height", "Fégeant 1999; Bolin 2009: deciduous vegetation 40–50 dB(A) at 5 m/s"),
    e(C::Wind, LaEqScene, 36.0, 34.7, "forest_roar 60 m away, 4.8 m/s", "distant forest roar 30–40 dB(A) in a breeze"),
    e(C::Rain, LaEqScene, 51.0, 63.0, "default scene, 4 mm/h steady rain", "rain 1 mm/h ~45, 4 mm/h ~50, 15 mm/h ~58, 50 mm/h ~65 dB(A) (in-forest measurements)"),
    e(C::Insects, LaEq1m, 72.0, 81.4, "field_cricket chorus (4 males), 22 °C, night", "Gryllus campestris calling song ~90 dB at 10 cm (~70 dB at 1 m each)"),
    e(C::Fire, LaEq1m, 62.0, 62.8, "campfire 20 kW", "campfire crackle ~55–65 dB(A) at 1 m"),
];

/// Thunder: loudest clap LAFmax at the listener in 4 minutes at storm 1 (~10 strikes,
/// 0.5–3 km).
/// Kept separate: the generator includes the long-range physics and its emitter is a
/// direction proxy at `director::THUNDER_PROXY_M`, whose 1/r the gain undoes.
pub static THUNDER: Entry = Entry {
    class: C::Thunder,
    metric: LaFmaxScene,
    target_db: 84.0,
    raw_db: 71.5,
    post: crate::director::THUNDER_PROXY_M,
    condition: "storm 1, loudest clap in 4 min",
    source: "thunder 1–3 km: 80–100 dB peak, ~75–90 dB(A) Fmax (Few 1969; Bass 1980)",
};

pub fn entry(class: SoundClass) -> &'static Entry {
    if class == C::Thunder {
        return &THUNDER;
    }
    TABLE.iter().find(|e| e.class == class).unwrap_or(&TABLE[0])
}

/// Level match of the trained neural voices to the procedural ones (same species, same
/// LAFmax at 1 m), dB: the neural models come out of training at their own scale.
/// Measured by `examples/calibrate.rs` (procedural − neural).
pub fn neural_trim_db(species: Species) -> f32 {
    match species {
        Species::Chaffinch => 13.4,
        Species::Robin => 9.3,
        Species::SongThrush => 10.1,
        Species::Nightingale => 16.5,
        Species::Blackbird => 13.1,
        _ => 0.0,
    }
}
