//! The director decides what happens in the scene over time: which animals are active at
//! this hour, where each one is, when it sings, takes off, flies past or circles.
//!
//! It owns one [`Agent`] per animal and a little scene-wide policy (flock events). Every
//! control block the engine calls [`Director::step`], which moves bird entities in the
//! [`World`] and writes their emitters' behavioural [`Controls`](crate::sources::Controls).
//! Real-time safe: all state is allocated in [`Director::new`].

mod agent;
mod species;

pub use agent::{Agent, State};
pub use species::{day_phase, daylight, DayPhase, Species, SpeciesDef};

use crate::rng::Rng;
use crate::scene::{object_flags, BehaviourDesc, DirectorDesc, EntityDesc, ObjectKind, SceneDesc, World};

/// Environment the director reacts to. Sliders feed it every block.
#[derive(Clone, Copy, Debug)]
pub struct Environment {
    /// Local time, hours 0..24.
    pub hour: f32,
    /// BIRDS slider 0..1 (0.6 = species-default activity).
    pub birds: f32,
    /// Rain 0..1 (placeholder: suppresses song and flight).
    pub rain: f32,
    /// ORBIT slider 0..1.
    pub orbit: f32,
}

impl Default for Environment {
    fn default() -> Self {
        Self { hour: 9.0, birds: 0.6, rain: 0.0, orbit: 0.0 }
    }
}

pub struct Director {
    agents: Vec<Agent>,
    trees: Vec<usize>,
    rng: Rng,
    policy: DirectorDesc,
    flock_left: f32,
    time: f64,
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
        Self { agents, trees, rng, policy, flock_left, time: 0.0 }
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
