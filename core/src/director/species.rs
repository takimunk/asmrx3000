//! Species catalogue: the per-species policy the director consults (when it sings, how it
//! flies, where it feeds, how it reacts to neighbours and danger, what it sounds like). Pure
//! data, so a future LLM-driven policy can extend or re-weight it without touching the
//! behaviour code.
//!
//! Also the biome hook for the procedural world: [`species_weights_for_biome`] and
//! [`bird_density_for_biome`] say which birds live in a landscape and how many.

use core::f32::consts::{PI, TAU};

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
    /// Common blackbird (Turdus merula): neural voice (adapter on the shared base).
    Blackbird,
    /// Eurasian skylark: meadows, steppe, dunes; sings in a hovering song flight.
    Skylark,
    /// Herring gull: sea coast, lakes, big rivers; soars, long-calls.
    Gull,
    /// Common raven: mountains, desert, coast; soars, croaks. One of the few birds in snow.
    Raven,
}

/// How a species moves through the air. Drives the path shape (`flight_motion`) and the
/// wingbeat rhythm the wings generator renders, so sound and silhouette agree.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FlightStyle {
    /// Finches: bursts of flapping (rising) and closed-wing glides (falling): undulating.
    Bounding,
    /// Thrushes, blackbird, robin, cuckoo: fast, low and straight, near-continuous flapping.
    Direct,
    /// Swallows: swooping arcs and dips, soft continuous wingbeats with short glides.
    Swooping,
    /// Wood pigeon: wing-clap take-off, strong steady beats, long glide into the landing.
    Clatter,
    /// Owls: a few soundless beats, long glides.
    Silent,
    /// Gulls, ravens: slow rowing beats between long glides, gentle banking.
    Soaring,
    /// Skylark: fluttering, hovering.
    Hovering,
}

impl FlightStyle {
    /// Mirrors `ASMR_FLIGHT_*` in asmr.h.
    pub fn code(self) -> u32 {
        self as u32
    }
}

pub struct SpeciesDef {
    /// Scene-file name (`"song_thrush"`).
    pub name: &'static str,
    pub voice: VoiceStyle,
    pub wings: WingStyle,
    pub flight: FlightStyle,
    /// Song activity by local hour (index 0 = 00:00), interpolated, 0..1.
    pub song: [f32; 24],
    /// Flies (and calls) at night rather than by day.
    pub nocturnal: bool,
    /// Seconds perched between decisions (min, max) for resident birds.
    pub dwell: [f32; 2],
    /// Flight speed range, m/s.
    pub speed: [f32; 2],
    /// Call activity while flying, 0..1.
    pub call_rate: f32,
    /// Sings / long-calls in flight (swallow twitter, skylark, gull, raven) instead of short calls.
    pub sings_in_flight: bool,
    /// Share of awake, non-singing time spent feeding on the ground, 0..1.
    pub ground: f32,
    /// Share of awake time spent on the wing (hawking insects, soaring), 0..1.
    pub aerial: f32,
    /// Territory radius around the home perch, m (song posts, feeding spots).
    pub territory: f32,
    /// Chance to answer a neighbour of its species when it stops singing (counter-singing).
    pub answer: f32,
    /// Range at which it reacts to a neighbour's song, m.
    pub hearing: f32,
    /// Flees when a disturbance comes closer than this, m; alarm-calls within 2.5×.
    pub flush: f32,
    /// Tendency to join flock flights, 0..1.
    pub flocking: f32,
    /// Mobs owls with alarm calls when one flies past by day.
    pub mobs: bool,
    /// Typical song-post height above the canopy centre (+) or below (-), m.
    pub post: f32,
    /// Perches in trees (gulls and skylarks stay on the ground).
    pub trees: bool,
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
    // Blackbird: first in the dawn chorus (well before sunrise), strong again at dusk.
    pub const BLACKBIRD:   [f32; 24] = [0.0, 0.0, 0.0, 0.35,0.95,1.0, 0.85,0.6, 0.45,0.35,0.3, 0.3, 0.3, 0.3, 0.3, 0.3, 0.35,0.45,0.6, 0.9, 1.0, 0.6, 0.05,0.0];
    // Skylark: starts singing in the dark before dawn, sings all day.
    pub const SKYLARK:     [f32; 24] = [0.0, 0.0, 0.0, 0.3, 0.8, 1.0, 1.0, 0.9, 0.85,0.8, 0.8, 0.75,0.7, 0.7, 0.7, 0.7, 0.75,0.75,0.7, 0.6, 0.4, 0.1, 0.0, 0.0];
    pub const GULL:        [f32; 24] = [0.05,0.05,0.05,0.1, 0.4, 0.7, 0.8, 0.7, 0.6, 0.6, 0.6, 0.6, 0.6, 0.6, 0.6, 0.6, 0.6, 0.6, 0.7, 0.7, 0.5, 0.2, 0.1, 0.05];
    pub const RAVEN:       [f32; 24] = [0.0, 0.0, 0.0, 0.0, 0.1, 0.4, 0.6, 0.6, 0.5, 0.5, 0.5, 0.5, 0.45,0.45,0.45,0.5, 0.5, 0.55,0.6, 0.5, 0.3, 0.05,0.0, 0.0];
}

const fn def(name: &'static str, voice: VoiceStyle, wings: WingStyle, flight: FlightStyle, song: [f32; 24]) -> SpeciesDef {
    SpeciesDef {
        name,
        voice,
        wings,
        flight,
        song,
        nocturnal: false,
        dwell: [20.0, 60.0],
        speed: [6.0, 9.0],
        call_rate: 0.5,
        sings_in_flight: false,
        ground: 0.3,
        aerial: 0.0,
        territory: 15.0,
        answer: 0.5,
        hearing: 60.0,
        flush: 3.0,
        flocking: 0.3,
        mobs: true,
        post: 0.5,
        trees: true,
    }
}

use FlightStyle as F;
use VoiceStyle as V;
use WingStyle as W;

static CHAFFINCH: SpeciesDef = SpeciesDef {
    dwell: [15.0, 50.0],
    call_rate: 0.6,
    ground: 0.35,
    answer: 0.55,
    flush: 2.5,
    flocking: 0.7,
    ..def("chaffinch", V::Chaffinch, W::Passerine, F::Bounding, curves::CHAFFINCH)
};
static ROBIN: SpeciesDef = SpeciesDef {
    speed: [5.0, 8.0],
    ground: 0.45,
    territory: 10.0,
    answer: 0.7,
    flush: 1.5,
    flocking: 0.0,
    post: -0.3,
    ..def("robin", V::Robin, W::Passerine, F::Direct, curves::ROBIN)
};
static SONG_THRUSH: SpeciesDef = SpeciesDef {
    dwell: [25.0, 70.0],
    speed: [7.0, 10.0],
    call_rate: 0.4,
    ground: 0.45,
    territory: 20.0,
    answer: 0.45,
    hearing: 90.0,
    flush: 3.5,
    flocking: 0.1,
    post: 1.2,
    ..def("song_thrush", V::SongThrush, W::Passerine, F::Direct, curves::SONG_THRUSH)
};
static NIGHTINGALE: SpeciesDef = SpeciesDef {
    dwell: [40.0, 90.0],
    speed: [5.0, 8.0],
    call_rate: 0.3,
    ground: 0.5,
    territory: 12.0,
    answer: 0.6,
    hearing: 100.0,
    flush: 2.0,
    flocking: 0.0,
    post: -1.0,
    ..def("nightingale", V::Nightingale, W::Passerine, F::Direct, curves::NIGHTINGALE)
};
static TAWNY_OWL: SpeciesDef = SpeciesDef {
    nocturnal: true,
    dwell: [40.0, 120.0],
    speed: [5.0, 7.0],
    call_rate: 0.4,
    ground: 0.15,
    territory: 40.0,
    answer: 0.6,
    hearing: 200.0,
    flush: 4.0,
    flocking: 0.0,
    mobs: false,
    post: -0.5,
    ..def("tawny_owl", V::TawnyOwl, W::Owl, F::Silent, curves::TAWNY_OWL)
};
static CUCKOO: SpeciesDef = SpeciesDef {
    dwell: [30.0, 80.0],
    speed: [8.0, 11.0],
    call_rate: 0.0,
    ground: 0.05,
    territory: 60.0,
    answer: 0.4,
    hearing: 250.0,
    flush: 5.0,
    flocking: 0.0,
    mobs: false,
    post: 1.0,
    ..def("cuckoo", V::Cuckoo, W::Passerine, F::Direct, curves::CUCKOO)
};
static WOOD_PIGEON: SpeciesDef = SpeciesDef {
    dwell: [30.0, 90.0],
    speed: [9.0, 13.0],
    call_rate: 0.0,
    ground: 0.55,
    territory: 30.0,
    answer: 0.35,
    hearing: 80.0,
    flush: 6.0,
    flocking: 0.8,
    mobs: false,
    post: 0.8,
    ..def("wood_pigeon", V::WoodPigeon, W::Pigeon, F::Clatter, curves::WOOD_PIGEON)
};
static SWALLOW: SpeciesDef = SpeciesDef {
    dwell: [10.0, 30.0],
    speed: [10.0, 15.0],
    call_rate: 0.9,
    sings_in_flight: true,
    ground: 0.0,
    aerial: 0.75,
    territory: 25.0,
    answer: 0.2,
    flush: 1.0,
    flocking: 0.6,
    post: 1.5,
    ..def("swallow", V::Swallow, W::Swallow, F::Swooping, curves::SWALLOW)
};
static BLACKBIRD: SpeciesDef = SpeciesDef {
    dwell: [25.0, 70.0],
    speed: [7.0, 10.0],
    call_rate: 0.35,
    ground: 0.55,
    territory: 18.0,
    answer: 0.6,
    hearing: 90.0,
    flush: 3.0,
    flocking: 0.05,
    post: 1.5,
    ..def("blackbird", V::Blackbird, W::Passerine, F::Direct, curves::BLACKBIRD)
};
static SKYLARK: SpeciesDef = SpeciesDef {
    dwell: [15.0, 40.0],
    speed: [6.0, 9.0],
    call_rate: 0.6,
    sings_in_flight: true,
    ground: 0.6,
    aerial: 0.5,
    territory: 30.0,
    answer: 0.3,
    hearing: 150.0,
    flush: 4.0,
    flocking: 0.3,
    mobs: false,
    post: 0.0,
    trees: false,
    ..def("skylark", V::Skylark, W::Lark, F::Hovering, curves::SKYLARK)
};
static GULL: SpeciesDef = SpeciesDef {
    dwell: [20.0, 60.0],
    speed: [8.0, 12.0],
    call_rate: 0.35,
    sings_in_flight: true,
    ground: 0.35,
    aerial: 0.6,
    territory: 60.0,
    answer: 0.55,
    hearing: 300.0,
    flush: 8.0,
    flocking: 0.8,
    mobs: false,
    post: 0.0,
    trees: false,
    ..def("gull", V::Gull, W::Large, F::Soaring, curves::GULL)
};
static RAVEN: SpeciesDef = SpeciesDef {
    dwell: [25.0, 80.0],
    speed: [8.0, 12.0],
    call_rate: 0.4,
    sings_in_flight: true,
    ground: 0.25,
    aerial: 0.5,
    territory: 80.0,
    answer: 0.5,
    hearing: 400.0,
    flush: 10.0,
    flocking: 0.2,
    mobs: false,
    post: 1.0,
    ..def("raven", V::Raven, W::Large, F::Soaring, curves::RAVEN)
};

impl Species {
    pub const ALL: [Species; 12] = [
        Species::Chaffinch,
        Species::Robin,
        Species::SongThrush,
        Species::Nightingale,
        Species::TawnyOwl,
        Species::Cuckoo,
        Species::WoodPigeon,
        Species::Swallow,
        Species::Blackbird,
        Species::Skylark,
        Species::Gull,
        Species::Raven,
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
            Species::Blackbird => &BLACKBIRD,
            Species::Skylark => &SKYLARK,
            Species::Gull => &GULL,
            Species::Raven => &RAVEN,
        }
    }

    /// Index in [`Species::ALL`] (= `ASMR_SPECIES_*` in asmr.h).
    pub fn code(self) -> u32 {
        Self::ALL.iter().position(|&s| s == self).unwrap_or(0) as u32
    }

    /// Scene-file name, e.g. `"song_thrush"`.
    pub fn name(self) -> &'static str {
        self.def().name
    }

    pub fn from_name(name: &str) -> Option<Species> {
        Self::ALL.iter().copied().find(|s| s.name() == name)
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

// ---------------------------------------------------------------------------------------
// Flight styles: path shape and wingbeat rhythm
// ---------------------------------------------------------------------------------------

/// Where a flying bird is relative to its smooth path, and how hard it flaps, `t` seconds
/// after take-off. `side` is the unit vector to the right of the path (horizontal), `phase`
/// a per-individual constant 0..1. Pure and allocation-free: the director's agents use it,
/// and the streamed flight-mode birds can too.
///
/// Returns `(offset, flap)`: `offset` is added to the path position (already faded in and
/// out near take-off and landing over `dur`), `flap` 0..1 is the wingbeat effort (0 = wings
/// closed or held in a glide: the wings generator is silent).
pub fn flight_motion(style: FlightStyle, t: f32, dur: f32, side: [f32; 3], phase: f32) -> ([f32; 3], f32) {
    let u = if dur > 0.0 { (t / dur).clamp(0.0, 1.0) } else { 0.5 };
    // Fades the style in after take-off and out before landing.
    let env = ((PI * u).sin() * 1.6).min(1.0) * (dur / 1.5).clamp(0.0, 1.0);
    let landing = dur - t;
    let (mut dz, mut lat, mut flap);
    match style {
        FlightStyle::Bounding => {
            // ~0.3 s of beats rising, ~0.2 s closed-wing glide falling: 1.9 bounds/s.
            let (burst, glide) = (0.30 + 0.06 * phase, 0.20 + 0.04 * (1.0 - phase));
            let c = (t + phase * 0.5) % (burst + glide);
            let a = 0.35;
            dz = if c < burst {
                a * c / burst
            } else {
                let g = (c - burst) / glide;
                a * (1.0 - g * g)
            } - 0.5 * a;
            lat = 0.0;
            flap = if c < burst { 0.65 } else { 0.0 };
        }
        FlightStyle::Direct => {
            // Steady fast beats, a short glide every ~1.2 s, slight bobbing.
            let c = (t + phase) % 1.2;
            flap = if c > 1.06 { 0.0 } else { 0.65 };
            dz = 0.05 * (TAU * 1.6 * t + phase * TAU).sin();
            lat = 0.0;
        }
        FlightStyle::Swooping => {
            // Wide S-curves with dips; glides on the outside of each swoop.
            let w = TAU / (1.5 + 0.4 * phase);
            let s = (w * t + phase * TAU).sin();
            lat = 2.2 * s;
            dz = -0.9 * (0.5 * w * t + phase * TAU).sin().abs() + 0.45;
            flap = if s.abs() > 0.7 { 0.0 } else { 0.45 };
        }
        FlightStyle::Clatter => {
            // Strong steady beats with glides on bowed wings every ~2 s.
            let c = (t + 2.0 * phase) % 2.2;
            flap = if c > 1.6 { 0.0 } else { 0.7 };
            dz = 0.15 * (TAU * t / 2.2 + phase * TAU).sin();
            lat = 0.0;
        }
        FlightStyle::Silent => {
            let c = (t + phase) % 2.3;
            flap = if c < 0.8 { 0.5 } else { 0.0 };
            dz = -0.1 * (TAU * t / 2.3).sin();
            lat = 0.3 * (TAU * t / 4.0 + phase * TAU).sin();
        }
        FlightStyle::Soaring => {
            // A few slow rowing beats, then a long glide; gentle banking.
            let c = (t + 5.0 * phase) % 6.0;
            flap = if c < 1.6 { 0.6 } else { 0.0 };
            dz = 0.4 * (TAU * t / 6.0 + phase * TAU).sin();
            lat = 0.8 * (TAU * t / 9.0 + phase * TAU).sin();
        }
        FlightStyle::Hovering => {
            let c = (t + phase * 0.4) % 0.42;
            let a = 0.25;
            dz = if c < 0.28 { a * c / 0.28 } else { a * (1.0 - ((c - 0.28) / 0.14).powi(2)) } - 0.5 * a;
            lat = 0.0;
            flap = if c < 0.28 { 0.55 } else { 0.0 };
        }
    }
    dz *= env;
    lat *= env;
    // Take-off effort and the landing flare (pigeons glide in; owls stay silent anyway).
    if t < 0.45 {
        flap = 1.0;
    } else if landing < 0.4 && dur > 0.8 {
        flap = 0.85;
    } else if style == FlightStyle::Clatter && landing < 1.6 && dur > 3.0 {
        flap = 0.0;
    }
    ([side[0] * lat, side[1] * lat, side[2] * lat + dz], flap)
}

// ---------------------------------------------------------------------------------------
// Biomes
// ---------------------------------------------------------------------------------------

/// Bird communities of the landscapes the procedural world builds.
static FOREST: [(Species, f32); 10] = [
    (Species::Chaffinch, 0.22),
    (Species::Robin, 0.15),
    (Species::Blackbird, 0.14),
    (Species::SongThrush, 0.11),
    (Species::WoodPigeon, 0.10),
    (Species::Nightingale, 0.06),
    (Species::Cuckoo, 0.06),
    (Species::Swallow, 0.06),
    (Species::TawnyOwl, 0.06),
    (Species::Raven, 0.04),
];
static MEADOW: [(Species, f32); 8] = [
    (Species::Skylark, 0.36),
    (Species::Swallow, 0.20),
    (Species::Cuckoo, 0.09),
    (Species::Chaffinch, 0.09),
    (Species::WoodPigeon, 0.08),
    (Species::Blackbird, 0.07),
    (Species::Raven, 0.06),
    (Species::Robin, 0.05),
];
static DESERT: [(Species, f32); 3] = [(Species::Skylark, 0.45), (Species::Raven, 0.40), (Species::Swallow, 0.15)];
static SNOW: [(Species, f32); 3] = [(Species::Raven, 0.75), (Species::Robin, 0.15), (Species::Chaffinch, 0.10)];
static LAKE: [(Species, f32); 9] = [
    (Species::Gull, 0.24),
    (Species::Swallow, 0.20),
    (Species::Nightingale, 0.12),
    (Species::Blackbird, 0.10),
    (Species::Chaffinch, 0.09),
    (Species::Cuckoo, 0.08),
    (Species::Robin, 0.07),
    (Species::WoodPigeon, 0.06),
    (Species::TawnyOwl, 0.04),
];
static COAST: [(Species, f32); 4] = [(Species::Gull, 0.70), (Species::Skylark, 0.12), (Species::Raven, 0.10), (Species::Swallow, 0.08)];
static RIVER: [(Species, f32); 10] = [
    (Species::Swallow, 0.20),
    (Species::Nightingale, 0.12),
    (Species::Blackbird, 0.12),
    (Species::Chaffinch, 0.12),
    (Species::Robin, 0.10),
    (Species::Gull, 0.10),
    (Species::SongThrush, 0.08),
    (Species::Cuckoo, 0.07),
    (Species::WoodPigeon, 0.05),
    (Species::TawnyOwl, 0.04),
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Biome {
    Forest,
    Meadow,
    Desert,
    Snow,
    Lake,
    Coast,
    River,
}

/// Biome names [`species_weights_for_biome`] understands (aliases in the docs).
pub const BIOMES: [&str; 7] = ["forest", "meadow", "desert", "snowy_mountains", "lake_shore", "sea_coast", "big_river"];

fn biome(name: &str) -> Biome {
    let mut buf = [0u8; 32];
    let n = name.len().min(buf.len());
    for (b, c) in buf.iter_mut().zip(name.bytes().take(n)) {
        *b = match c.to_ascii_lowercase() {
            b' ' | b'-' => b'_',
            c => c,
        };
    }
    match core::str::from_utf8(&buf[..n]).unwrap_or("") {
        "meadow" | "field" | "steppe" | "grassland" | "heath" | "plains" => Biome::Meadow,
        "desert" | "dunes" | "sand" | "savanna" => Biome::Desert,
        "snowy_mountains" | "snow" | "snowy" | "mountains" | "mountain" | "alpine" | "tundra" | "glacier" => Biome::Snow,
        "lake_shore" | "lake" | "lakeside" | "pond" | "marsh" | "wetland" => Biome::Lake,
        "sea_coast" | "coast" | "sea" | "beach" | "shore" | "cliffs" | "ocean" => Biome::Coast,
        "big_river" | "river" | "riverbank" | "floodplain" | "delta" => Biome::River,
        _ => Biome::Forest,
    }
}

/// Which birds live in a biome: `(species, weight)`, weights summing to 1 (relative share
/// of individuals). Unknown names fall back to `"forest"`. Allocation-free, any thread.
///
/// Names (case-insensitive, `-`/space = `_`): `forest` (default; woodland), `meadow`
/// (field, steppe, grassland), `desert` (dunes), `snowy_mountains` (snow, mountains, alpine),
/// `lake_shore` (lake, marsh), `sea_coast` (coast, beach, sea), `big_river` (river).
/// Combine with [`bird_density_for_biome`] for how many.
pub fn species_weights_for_biome(name: &str) -> &'static [(Species, f32)] {
    match biome(name) {
        Biome::Forest => &FOREST,
        Biome::Meadow => &MEADOW,
        Biome::Desert => &DESERT,
        Biome::Snow => &SNOW,
        Biome::Lake => &LAKE,
        Biome::Coast => &COAST,
        Biome::River => &RIVER,
    }
}

/// Relative bird abundance of a biome (forest = 1): multiply the BIRDS density by it.
pub fn bird_density_for_biome(name: &str) -> f32 {
    match biome(name) {
        Biome::Forest => 1.0,
        Biome::Meadow => 0.7,
        Biome::Desert => 0.2,
        Biome::Snow => 0.12,
        Biome::Lake => 0.85,
        Biome::Coast => 0.75,
        Biome::River => 0.95,
    }
}

/// Picks a species for a biome from a uniform `u` in 0..1 (deterministic).
pub fn pick_species_for_biome(name: &str, u: f32) -> Species {
    let w = species_weights_for_biome(name);
    let mut x = u.clamp(0.0, 0.999_999) * w.iter().map(|p| p.1).sum::<f32>();
    for &(s, p) in w {
        if x < p {
            return s;
        }
        x -= p;
    }
    w[w.len() - 1].0
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
            [Species::Chaffinch, Species::Robin, Species::SongThrush, Species::Blackbird]
                .iter()
                .map(|s| s.song_activity(h))
                .sum::<f32>()
        };
        assert!(total(5.5) > 1.5 * total(13.0));
        // The blackbird starts before the chaffinch.
        assert!(Species::Blackbird.song_activity(3.5) > Species::Chaffinch.song_activity(3.5) + 0.2);
        assert_eq!(day_phase(5.5), DayPhase::Dawn);
        assert!(Species::TawnyOwl.flight_activity(0.0) > 0.9 && Species::Robin.flight_activity(0.0) < 0.01);
    }

    #[test]
    fn names_round_trip() {
        for s in Species::ALL {
            assert_eq!(Species::from_name(s.name()), Some(s));
            let json = serde_json::to_string(&s).unwrap();
            assert_eq!(json, format!("\"{}\"", s.name()));
            assert_eq!(Species::ALL[s.code() as usize], s);
        }
    }

    #[test]
    fn biomes_have_their_birds() {
        let has = |b: &str, s: Species| species_weights_for_biome(b).iter().any(|p| p.0 == s);
        for b in BIOMES {
            let w = species_weights_for_biome(b);
            let sum: f32 = w.iter().map(|p| p.1).sum();
            assert!((sum - 1.0).abs() < 0.02, "{b}: {sum}");
        }
        assert!(has("sea_coast", Species::Gull) && !has("sea_coast", Species::Nightingale));
        assert!(has("Meadow", Species::Skylark) && has("desert", Species::Skylark));
        assert!(has("snowy mountains", Species::Raven));
        assert!(bird_density_for_biome("snow") < 0.2 && bird_density_for_biome("forest") == 1.0);
        assert_eq!(species_weights_for_biome("unknown planet").len(), FOREST.len());
        assert_eq!(pick_species_for_biome("sea-coast", 0.1), Species::Gull);
    }

    #[test]
    fn flight_styles_differ() {
        let side = [0.0, 1.0, 0.0];
        // Undulating finch: height swings, glides with closed wings.
        let (mut zmin, mut zmax, mut glide) = (f32::MAX, f32::MIN, 0);
        for k in 0..400 {
            let t = 1.0 + k as f32 * 0.01;
            let (o, f) = flight_motion(FlightStyle::Bounding, t, 6.0, side, 0.3);
            zmin = zmin.min(o[2]);
            zmax = zmax.max(o[2]);
            glide += (f == 0.0) as u32;
        }
        assert!(zmax - zmin > 0.25 && glide > 80, "bounding {zmin}..{zmax}, {glide}");
        // Swallow swoops sideways, the thrush flies straight.
        let lat = |s| (0..300).map(|k| flight_motion(s, 1.0 + k as f32 * 0.01, 6.0, side, 0.2).0[1].abs()).fold(0.0f32, f32::max);
        assert!(lat(FlightStyle::Swooping) > 1.5 && lat(FlightStyle::Direct) < 0.01);
        // Take-off is always full effort.
        assert_eq!(flight_motion(FlightStyle::Soaring, 0.1, 10.0, side, 0.0).1, 1.0);
    }
}
