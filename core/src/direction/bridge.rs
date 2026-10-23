//! Direction telemetry for the UI (`asmr_get_direction`) and what the user's hand did to the
//! driven controls (touches), lock-free between the control and audio threads.

use std::sync::atomic::{AtomicU32, Ordering};

use super::{Info, CHANNELS, CONTROL_IDS};

/// Mirrors `AsmrDirectionInfo` in asmr.h.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DirectionInfo {
    pub available: u32,
    pub active: u32,
    pub episode: u32,
    pub segment: u32,
    pub segments: u32,
    pub segment_left_s: f32,
    pub episode_left_s: f32,
    pub next: u32,
    pub cued: u32,
    pub hour: f32,
    pub liveliness: f32,
    pub weather: f32,
    pub energy: f32,
    pub change: f32,
    pub held: u32,
}

const FIELDS: usize = 15;

/// Latest [`DirectionInfo`], written by the audio thread a few times a second.
pub struct DirectionBridge {
    v: [AtomicU32; FIELDS],
}

impl Default for DirectionBridge {
    fn default() -> Self {
        Self { v: std::array::from_fn(|_| AtomicU32::new(0)) }
    }
}

impl DirectionBridge {
    pub fn store(&self, available: bool, i: &Info) {
        let f = |x: f32| x.to_bits();
        let vals = [
            available as u32,
            (available && i.active) as u32,
            i.episode as u32,
            i.segment as u32,
            i.segments as u32,
            f(i.segment_left_s),
            f(i.episode_left_s),
            i.next as u32,
            i.cued as u32,
            f(i.hour),
            f(i.liveliness),
            f(i.weather),
            f(i.energy),
            f(i.change),
            i.held,
        ];
        for (a, v) in self.v.iter().zip(vals) {
            a.store(v, Ordering::Relaxed);
        }
    }

    pub fn load(&self) -> DirectionInfo {
        let u = |k: usize| self.v[k].load(Ordering::Relaxed);
        let f = |k: usize| f32::from_bits(u(k));
        DirectionInfo {
            available: u(0),
            active: u(1),
            episode: u(2),
            segment: u(3),
            segments: u(4),
            segment_left_s: f(5),
            episode_left_s: f(6),
            next: u(7),
            cued: u(8),
            hour: f(9),
            liveliness: f(10),
            weather: f(11),
            energy: f(12),
            change: f(13),
            held: u(14),
        }
    }
}

/// Counters per driven channel, bumped by the user's writes (sliders, prompt patches): the
/// audio thread compares them with what it saw last.
#[derive(Default)]
pub struct Touches {
    touch: [AtomicU32; CHANNELS],
    release: [AtomicU32; CHANNELS],
}

/// What the audio thread saw last.
#[derive(Clone, Copy, Default)]
pub struct Seen {
    touch: [u32; CHANNELS],
    release: [u32; CHANNELS],
}

impl Touches {
    /// The driven channel of control `id`, if any.
    pub fn channel(id: u32) -> Option<usize> {
        CONTROL_IDS.iter().position(|&c| c == id)
    }

    /// The user set control `id` (`release` = handed it back to its curve).
    pub fn mark(&self, id: u32, release: bool) {
        if let Some(c) = Self::channel(id) {
            let a = if release { &self.release[c] } else { &self.touch[c] };
            a.fetch_add(1, Ordering::Release);
        }
    }

    /// Audio thread: which channels were touched / released since `seen`, updating it.
    pub fn take(&self, seen: &mut Seen) -> ([bool; CHANNELS], [bool; CHANNELS]) {
        let mut t = [false; CHANNELS];
        let mut r = [false; CHANNELS];
        for c in 0..CHANNELS {
            let a = self.touch[c].load(Ordering::Acquire);
            let b = self.release[c].load(Ordering::Acquire);
            t[c] = a != seen.touch[c];
            r[c] = b != seen.release[c];
            seen.touch[c] = a;
            seen.release[c] = b;
        }
        (t, r)
    }
}
