//! Species catalogue: the per-species policy the director consults (when it sings, how it
//! flies, what it sounds like). Pure data, so a future LLM-driven policy can extend or
//! re-weight it without touching the behaviour code.

use serde::{Deserialize, Serialize};

use crate::sources::{VoiceStyle, WingStyle};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Species {
    Chaffinch,
    Robin,
    SongThrush,
    Nightingale,
    TawnyOwl,
    Cuckoo,
    WoodPigeon,
    Swallow,
}

pub struct SpeciesDef {
    pub voice: VoiceStyle,
    pub wings: WingStyle,
    /// Song activity by local hour (index 0 = 00:00), interpolated, 0..1.
    pub song: [f32; 24],
    /// Flies (and calls) at night rather than by day.
    pub nocturnal: bool,
    /// Seconds perched between flights (min, max) for resident birds.
    pub dwell: [f32; 2],
    /// Flight speed range, m/s.
    pub speed: [f32; 2],
    /// Call activity while flying, 0..1.
    pub call_rate: f32,
    /// Sings its full song in flight (swallows) instead of short calls.
    pub sings_in_flight: bool,
}

#[rustfmt::skip]
mod curves {
    //                          0    1    2    3    4    5    6    7    8    9   10   11   12   13   14   15   16   17   18   19   20   21   22   23
    pub const CHAFFINCH:   [f32; 24] = [0.0, 0.0, 0.0, 0.0, 0.3, 0.9, 1.0, 0.9, 0.8, 0.6, 0.55,0.5, 0.5, 0.5, 0.5, 0.5, 0.55,0.6, 0.6, 0.5, 0.35,0.1, 0.0, 0.0];
    pub const ROBIN:       [f32; 24] = [0.05,0.05,0.05,0.2, 0.8, 1.0, 0.9, 0.7, 0.5, 0.4, 0.4, 0.4, 0.4, 0.4, 0.4, 0.4, 0.4, 0.45,0.5, 0.8, 0.9, 0.6, 0.15,0.05];
    pub const SONG_THRUSH: [f32; 24] = [0.0, 0.0, 0.0, 0.1, 0.6, 1.0, 0.9, 0.6, 0.4, 0.35,0.35,0.3, 0.3, 0.3, 0.3, 0.35,0.35,0.4, 0.5, 0.7, 1.0, 0.7, 0.1, 0.0];
    pub const NIGHTINGALE: [f32; 24] = [1.0, 0.9, 0.9, 0.9, 1.0, 0.8, 0.5, 0.35,0.3, 0.3, 0.3, 0.25,0.25,0.25,0.25,0.25,0.3, 0.3, 0.3, 0.35,0.4, 0.7, 0.9, 1.0];
    pub const TAWNY_OWL:   [f32; 24] = [0.8, 0.6, 0.6, 0.5, 0.4, 0.1, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.05,0.2, 0.5, 0.8, 1.0, 1.0];
    pub const CUCKOO:      [f32; 24] = [0.05,0.05,0.05,0.1, 0.5, 0.9, 1.0, 0.8, 0.8, 0.8, 0.8, 0.8, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5, 0.6, 0.6, 0.6, 0.3, 0.1, 0.05];
    pub const WOOD_PIGEON: [f32; 24] = [0.0, 0.0, 0.0, 0.0, 0.1, 0.5, 0.8, 0.7, 0.7, 0.7, 0.7, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5, 0.4, 0.1, 0.0, 0.0];
    pub const SWALLOW:     [f32; 24] = [0.0, 0.0, 0.0, 0.0, 0.1, 0.6, 0.8, 0.8, 0.8, 0.8, 0.8, 0.8, 0.8, 0.8, 0.8, 0.8, 0.8, 0.8, 0.8, 0.8, 0.6, 0.2, 0.0, 0.0];
}

static CHAFFINCH: SpeciesDef = SpeciesDef {
    voice: VoiceStyle::Chaffinch,
    wings: WingStyle::Passerine,
    song: curves::CHAFFINCH,
    nocturnal: false,
    dwell: [15.0, 50.0],
    speed: [6.0, 9.0],
    call_rate: 0.6,
    sings_in_flight: false,
};
static ROBIN: SpeciesDef = SpeciesDef {
    voice: VoiceStyle::Robin,
    wings: WingStyle::Passerine,
    song: curves::ROBIN,
    nocturnal: false,
    dwell: [20.0, 60.0],
    speed: [5.0, 8.0],
    call_rate: 0.5,
    sings_in_flight: false,
};
static SONG_THRUSH: SpeciesDef = SpeciesDef {
    voice: VoiceStyle::SongThrush,
    wings: WingStyle::Passerine,
    song: curves::SONG_THRUSH,
    nocturnal: false,
    dwell: [25.0, 70.0],
    speed: [7.0, 10.0],
    call_rate: 0.4,
    sings_in_flight: false,
};
static NIGHTINGALE: SpeciesDef = SpeciesDef {
    voice: VoiceStyle::Nightingale,
    wings: WingStyle::Passerine,
    song: curves::NIGHTINGALE,
    nocturnal: false,
    dwell: [40.0, 90.0],
    speed: [5.0, 8.0],
    call_rate: 0.3,
    sings_in_flight: false,
};
static TAWNY_OWL: SpeciesDef = SpeciesDef {
    voice: VoiceStyle::TawnyOwl,
    wings: WingStyle::Owl,
    song: curves::TAWNY_OWL,
    nocturnal: true,
    dwell: [40.0, 120.0],
    speed: [5.0, 7.0],
    call_rate: 0.4,
    sings_in_flight: false,
};
static CUCKOO: SpeciesDef = SpeciesDef {
    voice: VoiceStyle::Cuckoo,
    wings: WingStyle::Passerine,
    song: curves::CUCKOO,
    nocturnal: false,
    dwell: [30.0, 80.0],
    speed: [8.0, 11.0],
    call_rate: 0.0,
    sings_in_flight: false,
};
static WOOD_PIGEON: SpeciesDef = SpeciesDef {
    voice: VoiceStyle::WoodPigeon,
    wings: WingStyle::Pigeon,
    song: curves::WOOD_PIGEON,
    nocturnal: false,
    dwell: [30.0, 90.0],
    speed: [9.0, 13.0],
    call_rate: 0.0,
    sings_in_flight: false,
};
static SWALLOW: SpeciesDef = SpeciesDef {
    voice: VoiceStyle::Swallow,
    wings: WingStyle::Swallow,
    song: curves::SWALLOW,
    nocturnal: false,
    dwell: [10.0, 30.0],
    speed: [10.0, 15.0],
    call_rate: 0.9,
    sings_in_flight: true,
};

impl Species {
    pub const ALL: [Species; 8] = [
        Species::Chaffinch,
        Species::Robin,
        Species::SongThrush,
        Species::Nightingale,
        Species::TawnyOwl,
        Species::Cuckoo,
        Species::WoodPigeon,
        Species::Swallow,
    ];

    pub fn def(self) -> &'static SpeciesDef {
        match self {
            Species::Chaffinch => &CHAFFINCH,
            Species::Robin => &ROBIN,
            Species::SongThrush => &SONG_THRUSH,
            Species::Nightingale => &NIGHTINGALE,
            Species::TawnyOwl => &TAWNY_OWL,
            Species::Cuckoo => &CUCKOO,
            Species::WoodPigeon => &WOOD_PIGEON,
            Species::Swallow => &SWALLOW,
        }
    }

    /// Song activity at `hour` (0..24, wraps), 0..1.
    pub fn song_activity(self, hour: f32) -> f32 {
        let c = &self.def().song;
        let h = hour.rem_euclid(24.0);
        let i = h.floor() as usize % 24;
        let f = h - h.floor();
        c[i] * (1.0 - f) + c[(i + 1) % 24] * f
    }

    /// Readiness to fly at `hour`: daylight for diurnal birds, darkness for nocturnal ones.
    pub fn flight_activity(self, hour: f32) -> f32 {
        let light = daylight(hour);
        if self.def().nocturnal {
            1.0 - 0.9 * light
        } else {
            light
        }
    }
}

/// 0 at night, 1 by day, smooth ramps over dawn (04-06) and dusk (20-22). Summer-ish.
pub fn daylight(hour: f32) -> f32 {
    let h = hour.rem_euclid(24.0);
    let ramp = |x: f32| {
        let x = x.clamp(0.0, 1.0);
        x * x * (3.0 - 2.0 * x)
    };
    ramp((h - 4.0) / 2.0) * (1.0 - ramp((h - 20.0) / 2.0))
}

/// Which part of the day it is. Used for logs and the docs; behaviour uses the curves.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DayPhase {
    Night,
    Dawn,
    Day,
    Dusk,
}

pub fn day_phase(hour: f32) -> DayPhase {
    let h = hour.rem_euclid(24.0);
    match h {
        h if (3.5..7.0).contains(&h) => DayPhase::Dawn,
        h if (7.0..19.0).contains(&h) => DayPhase::Day,
        h if (19.0..22.0).contains(&h) => DayPhase::Dusk,
        _ => DayPhase::Night,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn night_and_dawn_voices() {
        // Midnight: nightingale and owl sing, chaffinch is asleep.
        assert_eq!(Species::Chaffinch.song_activity(0.0), 0.0);
        assert!(Species::Nightingale.song_activity(23.66) > 0.8);
        assert!(Species::TawnyOwl.song_activity(23.66) > 0.8);
        // Dawn chorus: the diurnal songbirds sing more at 05:30 than at 13:00.
        let total = |h| {
            [Species::Chaffinch, Species::Robin, Species::SongThrush]
                .iter()
                .map(|s| s.song_activity(h))
                .sum::<f32>()
        };
        assert!(total(5.5) > 1.5 * total(13.0));
        assert_eq!(day_phase(5.5), DayPhase::Dawn);
        assert!(Species::TawnyOwl.flight_activity(0.0) > 0.9 && Species::Robin.flight_activity(0.0) < 0.01);
    }
}
