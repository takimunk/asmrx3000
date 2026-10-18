//! Behaviour agents: one stochastic state machine per animal.
//!
//! ```text
//!            ┌────────── dwell expires, awake ──────────┐
//!            ▼                                          │
//!   Perched(anchor) ──take off──► Flying(bezier) ──land─┘
//!        ▲   (sings)                (wings, calls)  │
//!        │                                          ├─► Circling(laps) ──► Flying ...
//!   Absent(off stage) ◄──── leave ──────────────────┘
//! ```
//! Everything here runs on the audio thread at control rate: no allocation, fixed arrays.

use core::f32::consts::TAU;

use super::species::Species;
use super::Environment;
use crate::math::Vec3;
use crate::rng::Rng;
use crate::scene::{BehaviourDesc, Entity, Geometry};

const MAX_WAY: usize = 8;
/// Off-stage distance for entrances and exits.
const OFFSTAGE: f32 = 40.0;

#[derive(Clone, Copy, Debug)]
pub enum Anchor {
    /// Perch in a tree's canopy (follows the tree if the user moves it).
    Tree { tree: usize, offset: Vec3 },
    /// A fixed point in space.
    Point(Vec3),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Then {
    Perch,
    Circle,
    Leave,
}

#[derive(Clone, Copy, Debug)]
pub enum State {
    Perched { at: Anchor, left: f32 },
    Flying { from: Vec3, c1: Vec3, c2: Vec3, to: Anchor, t: f32, dur: f32, then: Then },
    Circling { center: Vec3, radius: f32, angle: f32, omega: f32, left: f32, endless: bool },
    Absent { left: f32 },
}

#[derive(Clone, Copy, Debug)]
enum Behaviour {
    Resident { roam: f32, orbit: bool },
    Flyby { interval: [f32; 2], speed: Option<f32>, min_distance: f32, height: f32, reach: f32 },
    Circle { center: Vec3, radius: f32, height: f32, speed: Option<f32>, laps: [f32; 2], interval: [f32; 2] },
    Approach { interval: [f32; 2], far: f32, near: f32, hops: u32, linger: [f32; 2], speed: Option<f32> },
}

/// What the agent wants the world to show this block.
#[derive(Clone, Copy, Debug, Default)]
pub struct Output {
    pub position: Vec3,
    pub activity: f32,
    pub calling: bool,
    pub flap: f32,
    pub moving: bool,
    pub absent: bool,
}

pub struct Agent {
    pub entity: usize,
    pub emitter: usize,
    pub species: Species,
    behaviour: Behaviour,
    pub state: State,
    rng: Rng,
    home: Option<usize>,
    /// Flock event: take off to this tree when the current perch ends.
    forced: Option<usize>,
    way: [Vec3; MAX_WAY],
    nway: usize,
    iway: usize,
    position: Vec3,
}

fn v3(a: [f32; 3]) -> Vec3 {
    Vec3::new(a[0], a[1], a[2])
}

fn cross(a: Vec3, b: Vec3) -> Vec3 {
    Vec3::new(a.y * b.z - a.z * b.y, a.z * b.x - a.x * b.z, a.x * b.y - a.y * b.x)
}

fn normalize(v: Vec3) -> Vec3 {
    let l = v.length();
    if l > 1e-6 {
        v * (1.0 / l)
    } else {
        Vec3::new(1.0, 0.0, 0.0)
    }
}

fn bezier(p0: Vec3, p1: Vec3, p2: Vec3, p3: Vec3, u: f32) -> Vec3 {
    let v = 1.0 - u;
    p0 * (v * v * v) + p1 * (3.0 * v * v * u) + p2 * (3.0 * v * u * u) + p3 * (u * u * u)
}

fn ease(u: f32) -> f32 {
    let u = u.clamp(0.0, 1.0);
    u * u * (3.0 - 2.0 * u)
}

fn range(rng: &mut Rng, r: [f32; 2]) -> f32 {
    rng.range(r[0], r[1].max(r[0]))
}

impl Agent {
    pub fn new(
        entity: usize,
        emitter: usize,
        species: Species,
        desc: &BehaviourDesc,
        home: Option<usize>,
        entities: &[Entity],
        seed: u64,
    ) -> Self {
        let rng = Rng::new(seed);
        let (behaviour, state) = match *desc {
            BehaviourDesc::Resident { roam, orbit, .. } => {
                (Behaviour::Resident { roam: roam.clamp(0.01, 20.0), orbit }, State::Absent { left: 0.0 })
            }
            BehaviourDesc::Flyby { first, interval, speed, min_distance, height, reach } => (
                Behaviour::Flyby { interval, speed, min_distance: min_distance.max(0.3), height, reach: reach.max(5.0) },
                State::Absent { left: first },
            ),
            BehaviourDesc::Circle { center, radius, height, speed, laps, first, interval, .. } => (
                Behaviour::Circle { center: v3(center), radius: radius.max(0.5), height, speed, laps, interval },
                State::Absent { left: first },
            ),
            BehaviourDesc::Approach { first, interval, far, near, hops, linger, speed } => (
                Behaviour::Approach { interval, far: far.max(near + 1.0), near: near.max(0.5), hops: hops.clamp(1, 5), linger, speed },
                State::Absent { left: first },
            ),
        };
        let mut a = Self {
            entity,
            emitter,
            species,
            behaviour,
            state,
            rng,
            home,
            forced: None,
            way: [Vec3::default(); MAX_WAY],
            nway: 0,
            iway: 0,
            position: Vec3::new(0.0, 0.0, OFFSTAGE),
        };
        if let Some(h) = home {
            let at = a.perch_in(h, entities);
            let left = match a.behaviour {
                Behaviour::Circle { .. } => match *desc {
                    BehaviourDesc::Circle { first, .. } => first,
                    _ => 0.0,
                },
                _ => a.dwell(),
            };
            a.position = a.resolve(at, entities);
            a.state = State::Perched { at, left };
        } else if let State::Absent { .. } = a.state {
            let dir = rng_dir(&mut a.rng);
            a.position = dir * OFFSTAGE + Vec3::new(0.0, 0.0, 10.0);
        }
        a
    }

    pub fn position(&self) -> Vec3 {
        self.position
    }

    pub fn is_resident(&self) -> bool {
        matches!(self.behaviour, Behaviour::Resident { .. })
    }

    /// Tree this agent is currently perched on, if any.
    pub fn perched_tree(&self) -> Option<usize> {
        match self.state {
            State::Perched { at: Anchor::Tree { tree, .. }, .. } => Some(tree),
            _ => None,
        }
    }

    /// Flock event: leave for `tree` within `delay` seconds.
    pub fn flock_to(&mut self, tree: usize, delay: f32) {
        if let State::Perched { left, .. } = &mut self.state {
            *left = left.min(delay);
            self.forced = Some(tree);
        }
    }

    fn dwell(&mut self) -> f32 {
        let roam = match self.behaviour {
            Behaviour::Resident { roam, .. } => roam,
            _ => 1.0,
        };
        range(&mut self.rng, self.species.def().dwell) / roam
    }

    fn speed(&mut self, over: Option<f32>) -> f32 {
        over.unwrap_or_else(|| range(&mut self.rng, self.species.def().speed)).clamp(1.0, 40.0)
    }

    fn perch_in(&mut self, tree: usize, entities: &[Entity]) -> Anchor {
        let r = match entities[tree].geometry {
            Geometry::Tree { canopy_radius, .. } => canopy_radius,
            _ => 1.0,
        };
        let phi = self.rng.uniform() * TAU;
        let d = 0.8 * r * self.rng.uniform().sqrt();
        Anchor::Tree { tree, offset: Vec3::new(d * phi.cos(), d * phi.sin(), self.rng.range(-0.5, 1.0)) }
    }

    fn resolve(&self, at: Anchor, entities: &[Entity]) -> Vec3 {
        match at {
            Anchor::Tree { tree, offset } => entities[tree].canopy() + offset,
            Anchor::Point(p) => p,
        }
    }

    /// A gently arched path from `a` to `b`.
    fn arc(&mut self, a: Vec3, b: Vec3) -> (Vec3, Vec3) {
        let d = b - a;
        let l = d.length();
        let up = Vec3::new(0.0, 0.0, (0.12 * l + 0.5) * self.rng.range(0.5, 1.5));
        let side = normalize(cross(d, Vec3::new(0.0, 0.0, 1.0))) * (self.rng.range(-0.25, 0.25) * l);
        (a + d * 0.33 + up + side, a + d * 0.66 + up + side)
    }

    fn fly(&mut self, to: Anchor, then: Then, speed: f32, entities: &[Entity]) {
        let from = self.position;
        let target = self.resolve(to, entities);
        let (c1, c2) = self.arc(from, target);
        let dur = ((target - from).length() * 1.15 / speed).max(0.8);
        self.state = State::Flying { from, c1, c2, to, t: 0.0, dur, then };
    }

    fn circle_center(&self, env: &Environment) -> (Vec3, f32, f32, bool) {
        match self.behaviour {
            Behaviour::Circle { center, radius, height, .. } => (center + Vec3::new(0.0, 0.0, height), radius, 0.0, false),
            // ORBIT slider: 3 m around the head at 1.8 m, same as the M0 orbit.
            _ => (Vec3::new(0.0, 0.0, 1.8), 3.0, TAU * env.orbit * 0.3, true),
        }
    }

    fn start_circle(&mut self, env: &Environment, entities: &[Entity]) {
        let (center, radius, _, _) = self.circle_center(env);
        let a0 = {
            let d = self.position - center;
            d.y.atan2(d.x)
        };
        let entry = center + Vec3::new(radius * a0.cos(), radius * a0.sin(), 0.0);
        let speed = match self.behaviour {
            Behaviour::Circle { speed, .. } => self.speed(speed),
            _ => self.speed(None),
        };
        self.fly(Anchor::Point(entry), Then::Circle, speed, entities);
    }

    fn awake(&self, env: &Environment) -> f32 {
        self.species.flight_activity(env.hour) * (1.0 - 0.9 * env.rain.clamp(0.0, 1.0))
    }

    /// Off stage -> entrance.
    fn enter(&mut self, env: &Environment, entities: &[Entity]) {
        match self.behaviour {
            Behaviour::Flyby { speed, min_distance, height, reach, .. } => {
                let heading = self.rng.uniform() * TAU;
                let d = Vec3::new(heading.cos(), heading.sin(), 0.0);
                let side = if self.rng.uniform() < 0.5 { 1.0 } else { -1.0 };
                let n = Vec3::new(-heading.sin(), heading.cos(), 0.0) * side;
                let closest = n * min_distance + Vec3::new(0.0, 0.0, height);
                let from = closest - d * reach + Vec3::new(0.0, 0.0, self.rng.range(1.0, 4.0));
                let to = closest + d * reach + Vec3::new(0.0, 0.0, self.rng.range(1.0, 4.0));
                // Control points on the straight line through the closest point keep the
                // pass at ~min_distance; the small bend makes it feel less robotic.
                let bend = n * self.rng.range(-0.5, 0.5);
                let c1 = closest - d * (reach / 3.0) + bend;
                let c2 = closest + d * (reach / 3.0) + bend;
                let v = self.speed(speed);
                let dur = (to - from).length() / v;
                self.position = from;
                self.state = State::Flying { from, c1, c2, to: Anchor::Point(to), t: 0.0, dur, then: Then::Leave };
            }
            Behaviour::Circle { .. } => {
                let dir = rng_dir(&mut self.rng);
                self.position = dir * OFFSTAGE + Vec3::new(0.0, 0.0, 8.0);
                self.start_circle(env, entities);
            }
            Behaviour::Approach { far, near, hops, .. } => {
                let az = self.rng.uniform() * TAU;
                let dir = Vec3::new(az.cos(), az.sin(), 0.0);
                let out_az = az + self.rng.range(1.2, 2.2) * if self.rng.uniform() < 0.5 { 1.0 } else { -1.0 };
                let out = Vec3::new(out_az.cos(), out_az.sin(), 0.0);
                self.nway = 0;
                let push = |a: &mut Agent, p: Vec3| {
                    if a.nway < MAX_WAY {
                        a.way[a.nway] = p;
                        a.nway += 1;
                    }
                };
                // In: far, ..., near.
                for k in 0..=hops {
                    let dist = far + (near - far) * k as f32 / hops as f32;
                    let z = self.rng.range(1.0, 3.5);
                    let p = dir * dist + Vec3::new(0.0, 0.0, z);
                    push(self, p);
                }
                // Out: halfway, far.
                let z = self.rng.range(1.5, 3.5);
                push(self, out * (0.5 * (near + far)) + Vec3::new(0.0, 0.0, z));
                push(self, out * far + Vec3::new(0.0, 0.0, z));
                self.iway = 0;
                self.position = self.way[0];
                let linger = self.linger();
                self.state = State::Perched { at: Anchor::Point(self.way[0]), left: linger };
            }
            Behaviour::Resident { .. } => {
                self.state = State::Absent { left: 5.0 };
            }
        }
    }

    fn linger(&mut self) -> f32 {
        match self.behaviour {
            Behaviour::Approach { linger, .. } => range(&mut self.rng, linger),
            _ => 3.0,
        }
    }

    fn off_stage_interval(&mut self) -> f32 {
        match self.behaviour {
            Behaviour::Flyby { interval, .. } | Behaviour::Circle { interval, .. } | Behaviour::Approach { interval, .. } => {
                range(&mut self.rng, interval)
            }
            Behaviour::Resident { .. } => 10.0,
        }
    }

    fn perch_expired(&mut self, env: &Environment, entities: &[Entity], trees: &[usize]) {
        match self.behaviour {
            Behaviour::Resident { .. } => {
                let forced = self.forced.take();
                let current = self.perched_tree();
                let go = forced.is_some() || self.rng.uniform() < self.awake(env);
                if go && trees.len() > 1 {
                    let target = forced.unwrap_or_else(|| {
                        let mut t = trees[(self.rng.next_u32() as usize) % trees.len()];
                        if Some(t) == current {
                            t = trees[(trees.iter().position(|&x| x == t).unwrap() + 1) % trees.len()];
                        }
                        t
                    });
                    let at = self.perch_in(target, entities);
                    let v = self.speed(None);
                    self.fly(at, Then::Perch, v, entities);
                } else if let State::Perched { left, .. } = &mut self.state {
                    *left = range(&mut self.rng, self.species.def().dwell);
                }
            }
            Behaviour::Circle { interval, .. } => {
                if self.awake(env) > 0.2 {
                    self.start_circle(env, entities);
                } else if let State::Perched { left, .. } = &mut self.state {
                    *left = range(&mut self.rng, interval);
                }
            }
            Behaviour::Approach { speed, .. } => {
                self.iway += 1;
                if self.iway < self.nway {
                    let v = self.speed(speed);
                    self.fly(Anchor::Point(self.way[self.iway]), Then::Perch, v, entities);
                } else {
                    let left = self.off_stage_interval();
                    self.state = State::Absent { left };
                }
            }
            Behaviour::Flyby { .. } => {
                let left = self.off_stage_interval();
                self.state = State::Absent { left };
            }
        }
    }

    fn arrived(&mut self, to: Anchor, then: Then, env: &Environment) {
        match then {
            Then::Perch => {
                let left = match self.behaviour {
                    Behaviour::Approach { .. } => {
                        // Linger longest at the closest stop.
                        let l = self.linger();
                        if self.iway + 3 == self.nway { l * 1.5 } else { l }
                    }
                    Behaviour::Circle { interval, .. } => range(&mut self.rng, interval),
                    _ => self.dwell(),
                };
                self.state = State::Perched { at: to, left };
            }
            Then::Circle => {
                let (center, radius, omega_orbit, endless) = self.circle_center(env);
                let d = self.position - center;
                let angle = d.y.atan2(d.x);
                let (omega, left) = match self.behaviour {
                    Behaviour::Circle { speed, laps, .. } => {
                        let v = self.speed(speed);
                        let dir = if self.rng.uniform() < 0.5 { 1.0 } else { -1.0 };
                        (dir * v / radius, range(&mut self.rng, laps) * TAU)
                    }
                    _ => (omega_orbit, f32::INFINITY),
                };
                self.state = State::Circling { center, radius, angle, omega, left, endless };
            }
            Then::Leave => {
                let left = self.off_stage_interval();
                self.state = State::Absent { left };
            }
        }
    }

    fn circle_done(&mut self, entities: &[Entity]) {
        let v = self.speed(None);
        if let Some(h) = self.home {
            let at = self.perch_in(h, entities);
            self.fly(at, Then::Perch, v, entities);
        } else {
            let dir = normalize(Vec3::new(self.position.x, self.position.y, 0.0));
            let side = Vec3::new(-dir.y, dir.x, 0.0);
            let exit = (dir + side * 0.5) * OFFSTAGE + Vec3::new(0.0, 0.0, 8.0);
            self.fly(Anchor::Point(exit), Then::Leave, v, entities);
        }
    }

    /// Advance by `dt` seconds.
    pub fn step(&mut self, dt: f32, env: &Environment, entities: &[Entity], trees: &[usize]) -> Output {
        // ORBIT slider overrides a resident bird flagged `orbit`.
        if let Behaviour::Resident { orbit: true, .. } = self.behaviour {
            let orbiting = matches!(self.state, State::Circling { endless: true, .. })
                || matches!(self.state, State::Flying { then: Then::Circle, .. });
            if env.orbit > 1e-3 && !orbiting {
                self.start_circle(env, entities);
            } else if env.orbit <= 1e-3 {
                if let State::Circling { endless: true, .. } = self.state {
                    self.circle_done(entities);
                }
            }
        }

        let def = self.species.def();
        let song = self.species.song_activity(env.hour) * (1.0 - 0.8 * env.rain.clamp(0.0, 1.0));
        let density = (env.birds / 0.6).clamp(0.0, 1.67);
        let awake = self.awake(env);
        // Entrances (fly-bys, approaches, circling) are performances: once on stage the bird
        // sings at least moderately, but only species that are awake at this hour enter.
        let performer = !self.is_resident();
        let mut out = Output { position: self.position, ..Default::default() };

        match self.state {
            State::Perched { at, left } => {
                self.position = self.resolve(at, entities);
                let base = if performer { song.max(0.6) } else { song };
                out.activity = (base * density).min(1.0);
                let left = left - dt;
                if let State::Perched { left: l, .. } = &mut self.state {
                    *l = left;
                }
                if left <= 0.0 {
                    self.perch_expired(env, entities, trees);
                }
            }
            State::Flying { from, c1, c2, to, t, dur, then } => {
                let t = t + dt;
                let target = self.resolve(to, entities);
                let u = if then == Then::Perch { ease(t / dur) } else { (t / dur).min(1.0) };
                self.position = bezier(from, c1, c2, target, u);
                out.calling = !def.sings_in_flight;
                out.activity = (def.call_rate * awake.max(0.3) * density).min(1.0);
                out.flap = if t < 0.6 {
                    1.0
                } else if then == Then::Perch && dur - t < 0.5 {
                    0.85
                } else {
                    0.5
                };
                out.moving = true;
                if t >= dur {
                    self.position = target;
                    self.arrived(to, then, env);
                } else if let State::Flying { t: tt, .. } = &mut self.state {
                    *tt = t;
                }
            }
            State::Circling { center, radius, angle, omega, left, endless } => {
                let omega = if endless { TAU * env.orbit * 0.3 } else { omega };
                let angle = (angle + omega * dt) % TAU;
                let left = left - (omega * dt).abs();
                self.position = center + Vec3::new(radius * angle.cos(), radius * angle.sin(), 0.25 * (3.0 * angle).sin());
                out.calling = !def.sings_in_flight;
                out.activity = (def.call_rate.max(0.3) * awake.max(0.3) * density).min(1.0);
                out.flap = 0.5;
                out.moving = true;
                self.state = State::Circling { center, radius, angle, omega, left, endless };
                if left <= 0.0 {
                    self.circle_done(entities);
                }
            }
            State::Absent { left } => {
                out.absent = true;
                let left = left - dt;
                self.state = State::Absent { left };
                if left <= 0.0 {
                    if awake > 0.2 && density > 0.0 {
                        self.enter(env, entities);
                    } else {
                        // Asleep at this hour: look again in a while.
                        self.state = State::Absent { left: self.rng.range(10.0, 30.0) };
                    }
                }
            }
        }
        out.position = self.position;
        out
    }
}

fn rng_dir(rng: &mut Rng) -> Vec3 {
    let a = rng.uniform() * TAU;
    Vec3::new(a.cos(), a.sin(), 0.0)
}
