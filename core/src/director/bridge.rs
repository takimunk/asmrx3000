//! Bird telemetry for the UI (`asmr_get_birds`): species, behaviour state, flight style,
//! wingbeat effort and velocity per bird, so the scene can draw each silhouette flying its
//! own way. Written by the audio thread after every block, read lock-free from any thread
//! (fields are individually atomic; a reader may see one bird half a block newer than
//! another, which does not matter for drawing).

use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use super::Director;
use crate::scene::{World, MAX_OBJECTS};

/// Mirrors `AsmrBirdInfo` in asmr.h.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BirdInfo {
    /// Object id (as in `asmr_get_objects`).
    pub id: u32,
    /// `ASMR_SPECIES_*` = index in `Species::ALL`.
    pub species: u32,
    /// `ASMR_BIRD_*` (`agent::code`).
    pub state: u32,
    /// `ASMR_FLIGHT_*` (`FlightStyle`).
    pub flight: u32,
    /// Wingbeat effort now, 0..1 (0 = perched, gliding or wings closed in a bound).
    pub flap: f32,
    /// Velocity, m/s (engine frame).
    pub vx: f32,
    pub vy: f32,
    pub vz: f32,
    /// `bird_flags::*`.
    pub flags: u32,
}

pub mod bird_flags {
    /// Voice sounding now (song or call).
    pub const VOCAL: u32 = 1 << 0;
    /// The voice does short calls rather than song.
    pub const CALLING: u32 = 1 << 1;
    /// Alarm calls / fleeing.
    pub const ALARM: u32 = 1 << 2;
}

const FIELDS: usize = 9;

pub struct BirdBridge {
    count: AtomicUsize,
    slots: Box<[AtomicU32]>,
}

impl Default for BirdBridge {
    fn default() -> Self {
        Self::new()
    }
}

impl BirdBridge {
    pub fn new() -> Self {
        Self { count: AtomicUsize::new(0), slots: (0..MAX_OBJECTS * FIELDS).map(|_| AtomicU32::new(0)).collect() }
    }

    /// Audio thread, after the director step and the voices rendered.
    pub fn publish(&self, director: &Director, world: &World) {
        let mut n = 0;
        for a in director.agents().iter().take(MAX_OBJECTS) {
            let out = a.last();
            let busy = world.emitters.get(a.emitter).is_some_and(|e| e.generator.busy());
            let mut flags = 0;
            if busy {
                flags |= bird_flags::VOCAL;
            }
            if out.calling {
                flags |= bird_flags::CALLING;
            }
            if out.alarm {
                flags |= bird_flags::ALARM;
            }
            let v = a.velocity();
            let fields = [
                world.entities[a.entity].id,
                a.species.code(),
                a.code(),
                a.flight_style().code(),
                out.flap.to_bits(),
                v.x.to_bits(),
                v.y.to_bits(),
                v.z.to_bits(),
                flags,
            ];
            for (k, f) in fields.iter().enumerate() {
                self.slots[n * FIELDS + k].store(*f, Ordering::Relaxed);
            }
            n += 1;
        }
        self.count.store(n, Ordering::Release);
    }

    /// Any thread. Returns the number of birds written.
    pub fn read(&self, out: &mut [BirdInfo]) -> usize {
        let n = self.count.load(Ordering::Acquire).min(out.len());
        for (i, o) in out.iter_mut().take(n).enumerate() {
            let f = |k: usize| self.slots[i * FIELDS + k].load(Ordering::Relaxed);
            *o = BirdInfo {
                id: f(0),
                species: f(1),
                state: f(2),
                flight: f(3),
                flap: f32::from_bits(f(4)),
                vx: f32::from_bits(f(5)),
                vy: f32::from_bits(f(6)),
                vz: f32::from_bits(f(7)),
                flags: f(8),
            };
        }
        n
    }
}
