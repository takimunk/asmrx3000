//! The director decides what happens in the scene over time: which animals are active at
//! this hour, where each one is, when it sings, feeds, takes off, flies past or circles.
//!
//! It owns one [`Agent`] per animal (each with its own life: territory, song posts, feeding,
//! species flight style; see agent.rs) and the scene-wide social layer:
//! * **counter-singing**: a bird waits while a neighbour of its species sings and often
//!   answers when it stops (so several birds alternate instead of singing over each other;
//!   this works with the mix layer's phrase permits, it does not replace them);
//! * **alarms**: a dragged tree, an owl flying past by day, a pigeon's clattering take-off
//!   ([`Director::disturb`] for anything else) make birds scold or flee to cover;
//! * **flocks**: several residents of one tree leave together along one shared path, in
//!   formation;
//! * **direction** (optional, encounters.rs): a song budget by hour, a slow pulse with lulls,
//!   and the scene's encounters (fly-bys, circling, visitors, flocks, gusts) cued one at a time.
//!
//! Every control block the engine calls [`Director::step`], which moves bird entities in the
//! [`World`] and writes their emitters' behavioural [`Controls`](crate::sources::Controls).
//! Real-time safe: all state is allocated in [`Director::new`].

mod agent;
pub mod bridge;
pub mod encounters;
mod species;
pub mod weather;
mod voices;
pub use voices::{bird_voice, neural_model_name};

pub use agent::{code as bird_state, Agent, Mode, State};
pub use bridge::{bird_flags, BirdBridge, BirdInfo};
pub use species::{
    bird_density_for_biome, day_phase, daylight, flight_motion, pick_species_for_biome, species_weights_for_biome, DayPhase, FlightStyle,
    Species, SpeciesDef, BIOMES,
};
pub use encounters::{Encounters, EncountersDesc};
pub use weather::Weather;

use crate::math::Vec3;
use crate::rng::Rng;
use crate::scene::{object_flags, BehaviourDesc, DirectorDesc, EntityDesc, Geometry, ObjectKind, SceneDesc, World};
use crate::sources::ModelKind;

/// Distance of the thunder direction proxy from the listener, m (see `sources::thunder`).
pub const THUNDER_PROXY_M: f32 = 40.0;

/// Unit direction to a strike at `bearing` (rad) and `distance` m, raised a little towards
/// the channel (less for far strikes): where the thunder proxies point.
pub fn thunder_direction(bearing: f32, distance: f32) -> Vec3 {
    let elev = (2000.0 / distance.max(1.0)).atan().min(0.9) * 0.6;
    Vec3::new(bearing.cos() * elev.cos(), bearing.sin() * elev.cos(), elev.sin())
}

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
    /// Per agent: singing on a song post now / in the previous block.
    singing: Vec<bool>,
    was_singing: Vec<bool>,
    /// Per tree: position last block; (metres dragged recently, seconds since the last drag).
    tree_pos: Vec<Vec3>,
    dragged: Vec<(f32, f32)>,
    encounters: Option<Encounters>,
    /// Song budget: seconds each agent has not sung.
    quiet: Vec<f32>,
    /// A gust the encounter director asked for (strength), for the engine's wind field.
    gust: Option<f32>,
}

/// A directed scene keeps feeding birds (and their calls) this far from the listener, m.
const PERSONAL_SPACE: f32 = 6.0;

/// Residents that may take an allowed phrase: the ones quiet longest.
const TURNS: usize = 3;

/// Trees dragged further than this within a couple of seconds make their birds flee.
const DRAG_FLEE_M: f32 = 4.0;

impl Director {
    /// `world` must have been built from the same `desc`.
    pub fn new(desc: &SceneDesc, world: &World) -> Self {
        let mut rng = Rng::new(desc.seed ^ 0xD1EC_7000);
        let trees: Vec<usize> = world.entities.iter().enumerate().filter(|(_, e)| e.kind == ObjectKind::Tree).map(|(i, _)| i).collect();
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
                agents.push(Agent::new(i, emitter, b.species, &b.behaviour, home, &world.entities, world.ground_z, rng.next_u64()));
            }
        }
        let policy = desc.director.clone();
        let encounters = policy.encounters.as_ref().map(|d| Encounters::new(d, desc.seed));
        if encounters.is_some() {
            for a in agents.iter_mut() {
                a.set_directed();
                a.set_personal_space(PERSONAL_SPACE);
            }
        }
        let flock_left = if encounters.is_some() {
            f32::INFINITY
        } else {
            policy.first_flock.unwrap_or_else(|| {
                let [a, b] = policy.flock_interval;
                rng.range(a, b.max(a))
            })
        };
        let thunder = world
            .entities
            .iter()
            .enumerate()
            .filter(|(_, e)| matches!(e.geometry, Geometry::Source { model: ModelKind::Thunder, .. }))
            .map(|(i, _)| i)
            .collect();
        let weather = Weather::new(&desc.environment, desc.seed ^ 0x5EA7_4E12);
        let n = agents.len();
        let tree_pos: Vec<Vec3> = trees.iter().map(|&t| world.entities[t].position).collect();
        let dragged = vec![(0.0, 10.0); tree_pos.len()];
        Self {
            agents,
            trees,
            rng,
            policy,
            flock_left,
            time: 0.0,
            weather,
            thunder,
            singing: vec![false; n],
            was_singing: vec![false; n],
            tree_pos,
            dragged,
            encounters,
            quiet: vec![0.0; n],
            gust: None,
        }
    }

    /// The encounter director, if the scene is directed.
    pub fn encounters(&self) -> Option<&Encounters> {
        self.encounters.as_ref()
    }

    /// A gust to sweep the crowns, asked for by the encounter director (taken once).
    pub fn take_gust(&mut self) -> Option<f32> {
        self.gust.take()
    }

    /// Something alarming happened at `at` (a predator, a loud event, the listener rushing
    /// past): birds within reach scold, the nearest flee if `flee`. `strength` 0..2 scales
    /// the species' flush distances. Audio thread (or before rendering starts).
    pub fn disturb(&mut self, at: Vec3, strength: f32, flee: bool) {
        for a in self.agents.iter_mut() {
            a.disturb(at, strength, flee);
        }
    }

    /// Scene-wide social signals: counter-singing (from the voices' phrase state after the
    /// previous block) and alarms (dragged trees, owls by day, pigeon take-offs).
    fn social(&mut self, dt: f32, env: &Environment, world: &World) {
        let n = self.agents.len();
        for (i, a) in self.agents.iter().enumerate() {
            let busy = world.emitters.get(a.emitter).is_some_and(|e| e.generator.busy());
            self.singing[i] = busy && a.on_song_post();
        }
        // Song budget: count the phrases residents start; while the bucket is empty, every
        // resident that is not singing waits (as for a singing neighbour). When a phrase is
        // allowed, only the few birds that have been quiet longest may take it, so the budget
        // is shared instead of going to whichever voice is quickest to start again.
        let mut gate = true;
        let mut turn = f32::NEG_INFINITY;
        if let Some(enc) = self.encounters.as_mut() {
            for i in 0..n {
                self.quiet[i] = if self.singing[i] { 0.0 } else { self.quiet[i] + dt };
                if self.singing[i] && !self.was_singing[i] && self.agents[i].is_resident() {
                    enc.song_started();
                }
            }
            gate = enc.gate_open();
            if gate {
                let mut top = [f32::NEG_INFINITY; TURNS];
                for i in 0..n {
                    let a = &self.agents[i];
                    if !self.singing[i] && a.is_resident() && a.on_song_post() && a.last().activity > 0.05 {
                        let q = self.quiet[i];
                        if let Some(k) = top.iter().position(|t| q > *t) {
                            top.copy_within(k..TURNS - 1, k + 1);
                            top[k] = q;
                        }
                    }
                }
                turn = top.iter().copied().filter(|t| t.is_finite()).fold(f32::INFINITY, f32::min);
                for i in 0..n {
                    if !self.singing[i] && self.agents[i].is_resident() && self.quiet[i] >= turn {
                        self.agents[i].give_turn();
                    }
                }
            }
        }
        for i in 0..n {
            let (sp, p) = (self.agents[i].species, self.agents[i].position());
            let range = sp.def().hearing;
            let resident = self.agents[i].is_resident();
            let mut hold = self.encounters.is_some() && resident && (!gate || self.quiet[i] < turn);
            for j in 0..n {
                if j != i && self.singing[j] && self.agents[j].species == sp && (self.agents[j].position() - p).length() < range {
                    hold = true;
                    break;
                }
            }
            let me = self.singing[i];
            self.agents[i].set_hold(hold && !me);
        }
        for j in 0..n {
            if self.was_singing[j] && !self.singing[j] {
                let (sp, p) = (self.agents[j].species, self.agents[j].position());
                for i in 0..n {
                    if i != j && !self.singing[i] && self.agents[i].species == sp && (self.agents[i].position() - p).length() < sp.def().hearing {
                        self.agents[i].invite_answer();
                    }
                }
            }
        }
        self.was_singing.copy_from_slice(&self.singing);

        // A tree dragged by the user: its birds scold; dragged far, they flee.
        for k in 0..self.trees.len() {
            let t = self.trees[k];
            let p = world.entities[t].position;
            let moved = (p - self.tree_pos[k]).length();
            self.tree_pos[k] = p;
            let (amount, since) = &mut self.dragged[k];
            *since += dt;
            if *since > 2.0 {
                *amount = 0.0;
            }
            if moved > 0.005 {
                *amount += moved;
                *since = 0.0;
                let flee = *amount > DRAG_FLEE_M;
                if flee {
                    *amount = 0.0;
                }
                let at = world.entities[t].canopy();
                for a in self.agents.iter_mut() {
                    // Its own birds are right at the danger; the neighbours hear the commotion.
                    let here = if a.perched_tree() == Some(t) { a.position() } else { at };
                    a.disturb(here, 1.0, flee);
                }
            }
        }
        // Owls flying by day are mobbed; a pigeon's clattering take-off startles neighbours.
        let day = daylight(env.hour);
        for i in 0..n {
            let a = &self.agents[i];
            let owl = a.species == Species::TawnyOwl && matches!(a.state, State::Flying { .. }) && day > 0.05;
            let startle = a.last().startle;
            if !owl && !startle {
                continue;
            }
            let p = a.position();
            for j in 0..n {
                if j == i {
                    continue;
                }
                let b = &mut self.agents[j];
                let bp = b.position();
                if owl && b.species.def().mobs && (bp - p).length() < 15.0 {
                    b.disturb(bp, 1.0, false);
                } else if startle && (bp - p).length() < 6.0 {
                    b.disturb(p, 0.6, false);
                }
            }
        }
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

    /// Several resident birds sharing a tree leave it together for another tree, along one
    /// shared path in a loose formation.
    fn flock_event(&mut self, env: &Environment, world: &World) -> bool {
        let n = self.trees.len();
        if n < 2 {
            return false;
        }
        let joins = |a: &Agent, from: usize| {
            a.is_resident() && a.perched_tree() == Some(from) && a.species.def().flocking >= 0.2 && a.species.flight_activity(env.hour) > 0.3
        };
        let start = self.rng.next_u32() as usize % n;
        for k in 0..n {
            let from = self.trees[(start + k) % n];
            let count = self.agents.iter().filter(|a| joins(a, from)).count();
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
            // Shared path: from the flock's centre to the target crown.
            let mut centre = Vec3::default();
            let mut speed = 0.0;
            for a in self.agents.iter().filter(|a| joins(a, from)) {
                centre = centre + a.position();
                let s = a.species.def().speed;
                speed += 0.5 * (s[0] + s[1]);
            }
            let inv = 1.0 / count as f32;
            let (centre, speed) = (centre * inv, (speed * inv).max(1.0));
            let target = world.entities[to].canopy();
            let d = target - centre;
            let l = d.length();
            let up = Vec3::new(0.0, 0.0, 0.12 * l + 1.0);
            let side = Vec3::new(-d.y, d.x, 0.0) * self.rng.range(-0.2, 0.2);
            let (c1, c2) = (centre + d * 0.33 + up + side, centre + d * 0.66 + up + side);
            let dur = (l * 1.15 / speed).max(1.0);
            for a in self.agents.iter_mut() {
                if joins(a, from) {
                    let delay = self.rng.range(0.0, 0.8);
                    let o = Vec3::new(self.rng.range(-1.0, 1.0), self.rng.range(-1.2, 1.2), self.rng.range(-0.6, 0.6));
                    let k = self.rng.range(0.97, 1.03);
                    a.flock_to(to, delay, c1 + o, c2 + o, dur * k);
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
            let dir = thunder_direction(bearing, distance);
            for &i in &self.thunder {
                world.entities[i].position = dir * THUNDER_PROXY_M;
            }
        }
        let env = &Environment { rain: env.rain.max(self.weather.rain_unit()), ..*env };
        if let Some(enc) = self.encounters.as_mut() {
            let agents = &self.agents;
            let candidates = agents.iter().enumerate().filter(|(_, a)| a.waiting() && a.can_perform(env)).filter_map(|(i, a)| {
                a.performance().map(|k| (i, [encounters::FLYBY, encounters::CIRCLE, encounters::VISITOR][k as usize]))
            });
            let ambient = self.weather.ambient(env);
            let gusts_ok = env.wind > 0.15 && env.storm < 0.3 && ambient.rain_mm_h < 2.0;
            if let Some(cue) = enc.step(dt, env, candidates, gusts_ok) {
                let ok = match cue {
                    encounters::Cue::Performer(i) => self.agents[i].cue(),
                    encounters::Cue::Flock => self.flock_event(env, world),
                    encounters::Cue::Gust(r) => {
                        // 1.5–3 mean gusts at once: a wave of wind that is heard coming
                        // through the crowns from upwind, passes and dies away downwind.
                        self.gust = Some(1.5 + 1.5 * (r as f32 / u32::MAX as f32));
                        true
                    }
                };
                if let Some(enc) = self.encounters.as_mut() {
                    enc.done(ok);
                }
            }
        }
        self.flock_left -= dt;
        if self.flock_left <= 0.0 {
            let fired = self.flock_event(env, world);
            let [a, b] = self.policy.flock_interval;
            self.flock_left = if fired { self.rng.range(a, b.max(a)) } else { self.rng.range(5.0, 15.0) };
        }
        self.social(dt, env, world);
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
            c.alarm = out.alarm;
            c.reply = out.reply;
            c.wait = out.wait;
            c.flap = if out.absent { 0.0 } else { out.flap };
            c.wings_driven = true;
        }
    }
}

#[cfg(test)]
mod tests;
