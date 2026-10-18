//! The director decides what happens in the scene over time: which animals are active at
//! this hour, where each one is, when it sings, takes off, flies past or circles.
//!
//! It owns one [`Agent`] per animal and a little scene-wide policy (flock events). Every
//! control block the engine calls [`Director::step`], which moves bird entities in the
//! [`World`] and writes their emitters' behavioural [`Controls`](crate::sources::Controls).
//! Real-time safe: all state is allocated in [`Director::new`].

mod agent;
mod species;
pub mod weather;

pub use agent::{Agent, State};
pub use species::{day_phase, daylight, DayPhase, Species, SpeciesDef};
pub use weather::Weather;

use crate::math::Vec3;
use crate::rng::Rng;
use crate::scene::{object_flags, BehaviourDesc, DirectorDesc, EntityDesc, Geometry, ObjectKind, SceneDesc, World};
use crate::sources::ModelKind;

/// Distance of the thunder direction proxy from the listener, m (see `sources::thunder`).
pub const THUNDER_PROXY_M: f32 = 40.0;

/// Environment the director reacts to. Sliders feed it every block.
#[derive(Clone, Copy, Debug)]
pub struct Environment {
    /// Local time, hours 0..24.
    pub hour: f32,
    /// BIRDS slider 0..1 (0.6 = species-default activity).
    pub birds: f32,
    /// Extra rain suppression of birds 0..1 (the director also derives it from the actual
    /// rain rate, whichever is larger).
    pub rain: f32,
    /// ORBIT slider 0..1.
    pub orbit: f32,
    /// WIND slider 0..1 (`sources::common::wind_speed_ms` maps it to m/s).
    pub wind: f32,
    /// Rain rate in the open, mm/h (RAIN param).
    pub rain_mm_h: f32,
    /// Air temperature, °C.
    pub temperature_c: f32,
    /// Relative humidity 0..1.
    pub humidity: f32,
    /// Thunderstorm activity 0..1 (lightning rate and closeness).
    pub storm: f32,
}

impl Default for Environment {
    fn default() -> Self {
        Self { hour: 9.0, birds: 0.6, rain: 0.0, orbit: 0.0, wind: 0.45, rain_mm_h: 0.0, temperature_c: 16.0, humidity: 0.75, storm: 0.0 }
    }
}

pub struct Director {
    agents: Vec<Agent>,
    trees: Vec<usize>,
    rng: Rng,
    policy: DirectorDesc,
    flock_left: f32,
    time: f64,
    weather: Weather,
    thunder: Vec<usize>,
}

impl Director {
    /// `world` must have been built from the same `desc`.
    pub fn new(desc: &SceneDesc, world: &World) -> Self {
        let mut rng = Rng::new(desc.seed ^ 0xD1EC_7000);
        let trees = world.entities.iter().enumerate().filter(|(_, e)| e.kind == ObjectKind::Tree).map(|(i, _)| i).collect();
        let mut agents = Vec::new();
        for (i, e) in desc.entities.iter().enumerate() {
            if let EntityDesc::Bird(b) = e {
                let home_id = match b.behaviour {
                    BehaviourDesc::Resident { home, .. } => Some(home),
                    BehaviourDesc::Circle { home, .. } => home,
                    _ => None,
                };
                let home = home_id.and_then(|h| world.index_of(h));
                let emitter = world.entities[i].emitters.start;
                agents.push(Agent::new(i, emitter, b.species, &b.behaviour, home, &world.entities, rng.next_u64()));
            }
        }
        let policy = desc.director.clone();
        let flock_left = policy.first_flock.unwrap_or_else(|| {
            let [a, b] = policy.flock_interval;
            rng.range(a, b.max(a))
        });
        let thunder = world
            .entities
            .iter()
            .enumerate()
            .filter(|(_, e)| matches!(e.geometry, Geometry::Source { model: ModelKind::Thunder, .. }))
            .map(|(i, _)| i)
            .collect();
        let weather = Weather::new(&desc.environment, desc.seed ^ 0x5EA7_4E12);
        Self { agents, trees, rng, policy, flock_left, time: 0.0, weather, thunder }
    }

    pub fn weather(&self) -> &Weather {
        &self.weather
    }

    pub fn weather_mut(&mut self) -> &mut Weather {
        &mut self.weather
    }

    pub fn agents(&self) -> &[Agent] {
        &self.agents
    }

    pub fn time(&self) -> f64 {
        self.time
    }

    /// Several resident birds sharing a tree leave it together for another tree.
    fn flock_event(&mut self, env: &Environment, world: &World) -> bool {
        let n = self.trees.len();
        if n < 2 {
            return false;
        }
        let start = self.rng.next_u32() as usize % n;
        for k in 0..n {
            let from = self.trees[(start + k) % n];
            let count = self
                .agents
                .iter()
                .filter(|a| a.is_resident() && a.perched_tree() == Some(from) && a.species.flight_activity(env.hour) > 0.3)
                .count();
            if count < 2 {
                continue;
            }
            // Prefer a far tree so the flock crosses the scene.
            let p = world.entities[from].position;
            let mut to = from;
            let mut best = -1.0f32;
            for &t in &self.trees {
                let d = (world.entities[t].position - p).length() * self.rng.range(0.6, 1.0);
                if t != from && d > best {
                    best = d;
                    to = t;
                }
            }
            for a in self.agents.iter_mut() {
                if a.is_resident() && a.perched_tree() == Some(from) {
                    let delay = self.rng.range(0.0, 0.8);
                    a.flock_to(to, delay);
                }
            }
            return true;
        }
        false
    }

    /// Advance the scene by `dt` seconds (one control block). Audio thread.
    pub fn step(&mut self, dt: f32, env: &Environment, world: &mut World) {
        self.time += dt as f64;
        // Weather first: every emitter sees the same environment this block.
        self.weather.step(dt, env);
        let ambient = self.weather.ambient(env);
        for em in world.emitters.iter_mut() {
            em.controls.env = ambient;
        }
        if self.weather.struck() {
            // Point the thunder proxies at the strike (elevation shrinks with distance).
            let (bearing, distance) = self.weather.strike_position();
            let elev = (2000.0 / distance.max(1.0)).atan().min(0.9) * 0.6;
            let dir = Vec3::new(bearing.cos() * elev.cos(), bearing.sin() * elev.cos(), elev.sin());
            for &i in &self.thunder {
                world.entities[i].position = dir * THUNDER_PROXY_M;
            }
        }
        let env = &Environment { rain: env.rain.max(self.weather.rain_unit()), ..*env };
        self.flock_left -= dt;
        if self.flock_left <= 0.0 {
            let fired = self.flock_event(env, world);
            let [a, b] = self.policy.flock_interval;
            self.flock_left = if fired { self.rng.range(a, b.max(a)) } else { self.rng.range(5.0, 15.0) };
        }
        for a in self.agents.iter_mut() {
            let out = a.step(dt, env, &world.entities, &self.trees);
            let e = &mut world.entities[a.entity];
            e.position = out.position;
            let mut flags = e.flags & object_flags::MOVABLE;
            if out.moving {
                flags |= object_flags::MOVING;
            }
            if out.absent {
                flags |= object_flags::ABSENT;
            }
            e.flags = flags;
            let c = &mut world.emitters[a.emitter].controls;
            c.activity = if out.absent { 0.0 } else { out.activity };
            c.calling = out.calling;
            c.flap = if out.absent { 0.0 } else { out.flap };
        }
    }
}

#[cfg(test)]
mod tests;
