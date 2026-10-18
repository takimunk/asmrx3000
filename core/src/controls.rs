//! STUB of the physical-models branch's control tree (`asmr_get_controls` / `asmr_set_control`),
//! so the UI can be built against the real ABI. Replace with that branch's implementation on merge.
//!
//! Macros map onto the existing engine params; a few children are derived from their macro
//! (moving the macro moves them), others are only stored.

use std::sync::atomic::{AtomicU32, Ordering};

use crate::engine::{param, Engine};

pub const GROUP_WATER: u32 = 0;
pub const GROUP_WIND: u32 = 1;
pub const GROUP_BIRDS: u32 = 4;
pub const GROUP_MASTER: u32 = 5;
pub const TOP_LEVEL: u32 = u32::MAX;
pub const FLAG_MACRO: u32 = 1;
pub const FLAG_LOG: u32 = 2;
pub const FLAG_DISCRETE: u32 = 4;

/// Mirrors `AsmrControlInfo` in asmr.h.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ControlInfo {
    pub id: u32,
    pub group: u32,
    pub parent: u32,
    pub flags: u32,
    pub min: f32,
    pub max: f32,
    pub value: f32,
    pub default_value: f32,
    pub key: [u8; 24],
    pub unit: [u8; 8],
}

impl Default for ControlInfo {
    fn default() -> Self {
        Self { id: 0, group: 0, parent: TOP_LEVEL, flags: 0, min: 0.0, max: 1.0, value: 0.0, default_value: 0.0, key: [0; 24], unit: [0; 8] }
    }
}

/// How a control's value is held.
#[derive(Clone, Copy)]
enum Source {
    /// The engine param itself.
    Param(u32),
    /// Linear in an engine param: value = a + b * param (setting it writes the param back).
    Derived(u32, f32, f32),
    /// Stored here; reset from the macro when the macro moves: value = a + b * macro.
    Stored(f32, f32),
}

struct Def {
    id: u32,
    group: u32,
    parent: u32,
    flags: u32,
    min: f32,
    max: f32,
    default: f32,
    key: &'static str,
    unit: &'static str,
    source: Source,
}

const DEFS: &[Def] = &[
    Def { id: 100, group: GROUP_WATER, parent: TOP_LEVEL, flags: FLAG_MACRO, min: 0.0, max: 1.0, default: 0.6, key: "water", unit: "", source: Source::Param(param::WATER) },
    Def { id: 101, group: GROUP_WATER, parent: 100, flags: 0, min: 0.1, max: 2.0, default: 1.24, key: "water.flow", unit: "m/s", source: Source::Derived(param::WATER, 0.1, 1.9) },
    Def { id: 102, group: GROUP_WATER, parent: 100, flags: FLAG_LOG, min: 0.05, max: 1.5, default: 0.4, key: "water.depth", unit: "m", source: Source::Stored(0.05, 0.9) },
    Def { id: 103, group: GROUP_WATER, parent: 100, flags: FLAG_DISCRETE, min: 0.0, max: 3.0, default: 1.0, key: "water.character", unit: "", source: Source::Stored(1.0, 0.0) },
    Def { id: 200, group: GROUP_WIND, parent: TOP_LEVEL, flags: FLAG_MACRO, min: 0.0, max: 1.0, default: 0.45, key: "wind", unit: "", source: Source::Param(param::WIND) },
    Def { id: 201, group: GROUP_WIND, parent: 200, flags: 0, min: 0.0, max: 12.0, default: 5.4, key: "wind.speed", unit: "m/s", source: Source::Derived(param::WIND, 0.0, 12.0) },
    Def { id: 202, group: GROUP_WIND, parent: 200, flags: 0, min: 0.0, max: 1.0, default: 0.5, key: "wind.gustiness", unit: "", source: Source::Stored(0.25, 0.55) },
    Def { id: 300, group: GROUP_BIRDS, parent: TOP_LEVEL, flags: FLAG_MACRO, min: 0.0, max: 1.0, default: 0.6, key: "birds", unit: "", source: Source::Param(param::BIRDS) },
    Def { id: 301, group: GROUP_BIRDS, parent: 300, flags: 0, min: 0.0, max: 1.0, default: 0.0, key: "birds.flight", unit: "", source: Source::Param(param::ORBIT) },
    Def { id: 500, group: GROUP_MASTER, parent: TOP_LEVEL, flags: FLAG_MACRO, min: 0.0, max: 1.0, default: 0.7, key: "master", unit: "", source: Source::Param(param::VOLUME) },
];

pub struct Controls {
    stored: Box<[AtomicU32]>,
}

impl Controls {
    pub fn new() -> Self {
        Self { stored: DEFS.iter().map(|d| AtomicU32::new(d.default.to_bits())).collect() }
    }

    fn value(&self, engine: &Engine, i: usize) -> f32 {
        let d = &DEFS[i];
        match d.source {
            Source::Param(p) => engine.param(p),
            Source::Derived(p, a, b) => a + b * engine.param(p),
            Source::Stored(..) => f32::from_bits(self.stored[i].load(Ordering::Relaxed)),
        }
    }

    pub fn list(&self, engine: &Engine, out: &mut [ControlInfo]) -> usize {
        let n = DEFS.len().min(out.len());
        for (i, o) in out.iter_mut().take(n).enumerate() {
            let d = &DEFS[i];
            let mut key = [0u8; 24];
            key[..d.key.len()].copy_from_slice(d.key.as_bytes());
            let mut unit = [0u8; 8];
            unit[..d.unit.len()].copy_from_slice(d.unit.as_bytes());
            *o = ControlInfo {
                id: d.id,
                group: d.group,
                parent: d.parent,
                flags: d.flags,
                min: d.min,
                max: d.max,
                value: self.value(engine, i),
                default_value: d.default,
                key,
                unit,
            };
        }
        n
    }

    pub fn set(&self, engine: &Engine, id: u32, value: f32) {
        let Some(i) = DEFS.iter().position(|d| d.id == id) else { return };
        if !value.is_finite() {
            return;
        }
        let d = &DEFS[i];
        let v = value.clamp(d.min, d.max);
        match d.source {
            Source::Param(p) => engine.set_param(p, v),
            Source::Derived(p, a, b) => engine.set_param(p, (v - a) / b),
            Source::Stored(..) => self.stored[i].store(v.to_bits(), Ordering::Relaxed),
        }
        if d.flags & FLAG_MACRO != 0 {
            // The macro carries its stored children along.
            for (j, c) in DEFS.iter().enumerate() {
                if let (true, Source::Stored(a, b)) = (c.parent == d.id, c.source) {
                    if b != 0.0 {
                        self.stored[j].store((a + b * v).clamp(c.min, c.max).to_bits(), Ordering::Relaxed);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macro_moves_children() {
        let e = Engine::new(48_000.0);
        let mut out = [ControlInfo::default(); 16];
        let n = e.controls().list(&e, &mut out);
        assert!(n >= 8);
        e.controls().set(&e, 100, 1.0);
        e.controls().list(&e, &mut out);
        let flow = out.iter().find(|c| c.id == 101).unwrap().value;
        assert!((flow - 2.0).abs() < 1e-5);
        e.controls().set(&e, 101, 0.1);
        assert!(e.param(param::WATER).abs() < 1e-5);
    }
}
