//! Behaviour agents: one stochastic state machine per animal, with a small life of its own.
//!
//! ```text
//!                 ┌──────────── decide (time budget by hour, territory) ─────────────┐
//!                 ▼                                                                  │
//!   Perched{Sing|Rest|Alert} ──hop / move post──► Flying(style) ──land──► Perched ──┤
//!        │    ▲  (song post)                         │   ▲                           │
//!        │    └──────────── fly up ──────── Ground (forage, hop) ◄── fly down ───────┤
//!        │                                           │                               │
//!        ├──► Hawking (swallows feeding, gulls / ravens soaring) ──► Perched|Ground ─┤
//!        ├──► SongFlight (skylark: climb, hover singing, plummet) ──► Ground ────────┘
//!        └──► Circling / Absent (performers: fly-by, approach, circle; ORBIT slider)
//! ```
//!
//! *Resident* birds own a territory around their home tree: song posts (trees within
//! `SpeciesDef::territory`), feeding spots on the ground. When a bout ends the bird picks its
//! next activity from an hourly budget: singing (song curve), feeding on the ground, resting,
//! moving to another song post, feeding on the wing. Diurnal birds roost at night, owls the
//! other way round. Flights follow the species' style (`species::flight_motion`): finches
//! bound, thrushes fly low and straight, swallows swoop, pigeons clatter and glide, owls glide
//! silently, gulls and ravens soar. *Open* birds (no tree: skylark, gull, raven) live on the
//! ground and in the air of their area.
//!
//! Social signals come from the director: a neighbour of the same species singing (`hold`:
//! wait), a neighbour that just finished (`answer`: reply soon), a disturbance (`disturb`:
//! alarm calls, flee to cover).
//!
//! Everything here runs on the audio thread at control rate: no allocation, fixed arrays.

use core::f32::consts::{PI, TAU};

use super::species::{flight_motion, FlightStyle, Species};
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

/// What a perched bird is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// On a song post, singing at the species' hourly rate.
    Sing,
    /// Resting / preening / roosting: an occasional phrase at most.
    Rest,
    /// Disturbed: alarm calls.
    Alert,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Then {
    Perch(Mode),
    Ground,
    Hawk,
    Circle,
    Leave,
}

#[derive(Clone, Copy, Debug)]
pub enum State {
    Perched { at: Anchor, left: f32, mode: Mode },
    /// Feeding on the ground: hops from spot to spot.
    Ground { at: Vec3, from: Vec3, hop: f32, next_hop: f32, left: f32 },
    Flying { from: Vec3, c1: Vec3, c2: Vec3, to: Anchor, t: f32, dur: f32, then: Then, display: bool },
    /// Feeding on the wing (swallows) or soaring (gulls, ravens) over an area.
    Hawking { center: Vec3, radius: f32, height: f32, t: f32, omega: f32, left: f32 },
    /// Skylark song flight from `base` (on the ground): climb, hover singing, drop.
    SongFlight { base: Vec3, height: f32, t: f32, dur: f32 },
    Circling { center: Vec3, radius: f32, angle: f32, omega: f32, left: f32, endless: bool },
    Absent { left: f32 },
}

/// Telemetry codes (`ASMR_BIRD_*` in asmr.h).
pub mod code {
    pub const RESTING: u32 = 0;
    pub const SINGING: u32 = 1;
    pub const ALERT: u32 = 2;
    pub const FORAGING: u32 = 3;
    pub const FLYING: u32 = 4;
    pub const SOARING: u32 = 5;
    pub const SONG_FLIGHT: u32 = 6;
    pub const CIRCLING: u32 = 7;
    pub const ABSENT: u32 = 8;
    pub const NAMES: [&str; 9] = ["resting", "singing", "alert", "foraging", "flying", "soaring", "song-flight", "circling", "absent"];
}

#[derive(Clone, Copy, Debug)]
enum Behaviour {
    Resident { roam: f32, orbit: bool },
    /// No home tree: ground and sky of an area.
    Open { center: Vec3, radius: f32, roam: f32 },
    Flyby { interval: [f32; 2], speed: Option<f32>, min_distance: f32, height: f32, reach: f32 },
    Circle { center: Vec3, radius: f32, height: f32, speed: Option<f32>, laps: [f32; 2], interval: [f32; 2] },
    Approach { interval: [f32; 2], far: f32, near: f32, hops: u32, linger: [f32; 2], speed: Option<f32> },
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Reaction {
    None,
    Alert,
    Flee(Vec3),
}

/// What the agent wants the world to show this block.
#[derive(Clone, Copy, Debug, Default)]
pub struct Output {
    pub position: Vec3,
    pub activity: f32,
    pub calling: bool,
    pub alarm: bool,
    pub flap: f32,
    pub moving: bool,
    pub absent: bool,
    /// A clattering take-off that startles birds nearby (pigeons).
    pub startle: bool,
    /// Answering a neighbour: sing the next phrase soon.
    pub reply: bool,
    /// A neighbour of the species sings: wait for it to finish.
    pub wait: bool,
}

pub struct Agent {
    pub entity: usize,
    pub emitter: usize,
    pub species: Species,
    behaviour: Behaviour,
    pub state: State,
    rng: Rng,
    home: Option<usize>,
    /// Flock event: take off along this shared path when the current perch ends.
    forced: Option<(usize, Vec3, Vec3, f32)>,
    way: [Vec3; MAX_WAY],
    nway: usize,
    iway: usize,
    position: Vec3,
    velocity: Vec3,
    ground_z: f32,
    /// Individual offset of the daily rhythm, hours (dawn chorus does not start in unison).
    wake: f32,
    /// Per-individual phase of the flight rhythm, 0..1.
    phase: f32,
    /// Seconds of reluctance to sing after a scare.
    calm: f32,
    /// Seconds of alarm calling left (also while fleeing).
    alarm: f32,
    reaction: Reaction,
    react_in: f32,
    /// Social input from the director (set before each step).
    hold: bool,
    answer: f32,
    takeoff: bool,
    /// Hawking loop to start once the bird reaches its entry point.
    pending_hawk: Option<State>,
    last: Output,
    /// A perched bird now and then shifts its wings (one to four beats): seconds left of the
    /// flick, seconds to the next one, and its own little generator (so flicks never change
    /// the bird's other decisions).
    flick: f32,
    next_flick: f32,
    flick_rng: u64,
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

fn flat(v: Vec3) -> Vec3 {
    Vec3::new(v.x, v.y, 0.0)
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

/// Horizontal distance from `p` to the nearest river bed, m.
fn water_distance(entities: &[Entity], p: Vec3) -> f32 {
    let mut best = f32::MAX;
    for e in entities {
        if let Geometry::River { path, .. } = &e.geometry {
            for w in path.windows(2) {
                let (a, b) = (flat(e.position + w[0]), flat(e.position + w[1]));
                let ab = b - a;
                let u = ((flat(p) - a).dot(ab) / ab.dot(ab).max(1e-6)).clamp(0.0, 1.0);
                best = best.min((flat(p) - (a + ab * u)).length());
            }
        }
    }
    best
}

impl Agent {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        entity: usize,
        emitter: usize,
        species: Species,
        desc: &BehaviourDesc,
        home: Option<usize>,
        entities: &[Entity],
        ground_z: f32,
        seed: u64,
    ) -> Self {
        let mut rng = Rng::new(seed);
        let (behaviour, state) = match *desc {
            BehaviourDesc::Resident { roam, orbit, .. } => {
                (Behaviour::Resident { roam: roam.clamp(0.01, 20.0), orbit }, State::Absent { left: 0.0 })
            }
            BehaviourDesc::Open { center, radius, roam } => (
                Behaviour::Open { center: Vec3::new(center[0], center[1], ground_z), radius: radius.clamp(2.0, 300.0), roam: roam.clamp(0.01, 20.0) },
                State::Absent { left: 0.0 },
            ),
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
        let wake = rng.range(-0.25, 0.25);
        let phase = rng.uniform();
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
            velocity: Vec3::default(),
            ground_z,
            wake,
            phase,
            calm: 0.0,
            alarm: 0.0,
            reaction: Reaction::None,
            react_in: 0.0,
            hold: false,
            answer: 0.0,
            takeoff: false,
            pending_hawk: None,
            last: Output::default(),
            flick: 0.0,
            next_flick: 8.0 + 40.0 * phase,
            flick_rng: seed ^ 0xF11C_F11C,
        };
        if let Some(h) = home {
            let at = a.perch_in(h, entities, false);
            let left = match a.behaviour {
                Behaviour::Circle { .. } => match *desc {
                    BehaviourDesc::Circle { first, .. } => first,
                    _ => 0.0,
                },
                _ => a.dwell(),
            };
            a.position = a.resolve(at, entities);
            a.state = State::Perched { at, left, mode: Mode::Sing };
        } else if let Behaviour::Open { .. } = a.behaviour {
            let p = a.ground_spot(None, entities);
            a.position = p;
            let left = a.rng.range(2.0, 12.0);
            a.state = State::Ground { at: p, from: p, hop: 1.0, next_hop: 0.5, left };
        } else if let State::Absent { .. } = a.state {
            let dir = rng_dir(&mut a.rng);
            a.position = dir * OFFSTAGE + Vec3::new(0.0, 0.0, 10.0);
        }
        a
    }

    pub fn position(&self) -> Vec3 {
        self.position
    }

    pub fn velocity(&self) -> Vec3 {
        self.velocity
    }

    /// The last block's output (telemetry).
    pub fn last(&self) -> &Output {
        &self.last
    }

    pub fn is_resident(&self) -> bool {
        matches!(self.behaviour, Behaviour::Resident { .. } | Behaviour::Open { .. })
    }

    /// Tree this agent is currently perched on, if any.
    pub fn perched_tree(&self) -> Option<usize> {
        match self.state {
            State::Perched { at: Anchor::Tree { tree, .. }, .. } => Some(tree),
            _ => None,
        }
    }

    /// On a song post in singing mode: a voice phrase now counts as song (counter-singing).
    pub fn on_song_post(&self) -> bool {
        matches!(self.state, State::Perched { mode: Mode::Sing, .. })
    }

    /// Telemetry state code (`code::*`).
    pub fn code(&self) -> u32 {
        match self.state {
            State::Perched { mode: Mode::Sing, .. } => code::SINGING,
            State::Perched { mode: Mode::Alert, .. } => code::ALERT,
            State::Perched { .. } => code::RESTING,
            State::Ground { .. } => code::FORAGING,
            State::Flying { .. } => code::FLYING,
            State::Hawking { .. } => code::SOARING,
            State::SongFlight { .. } => code::SONG_FLIGHT,
            State::Circling { .. } => code::CIRCLING,
            State::Absent { .. } => code::ABSENT,
        }
    }

    pub fn flight_style(&self) -> FlightStyle {
        self.species.def().flight
    }

    /// Social input for the next step: a same-species neighbour is singing now.
    pub fn set_hold(&mut self, hold: bool) {
        self.hold = hold;
    }

    /// A neighbour just finished its song: reply within the next few seconds.
    pub fn invite_answer(&mut self) {
        if self.answer <= 0.0 && self.rng.uniform() < self.species.def().answer {
            self.answer = self.rng.range(2.5, 5.0);
        }
    }

    /// Something alarming at `at` (`strength` 0..1 scales the reaction distances; `flee`
    /// allows fleeing, otherwise only alarm calls). Reacts after a short, individual delay.
    pub fn disturb(&mut self, at: Vec3, strength: f32, flee: bool) {
        if matches!(self.state, State::Absent { .. } | State::Circling { .. }) || !self.is_resident() {
            return;
        }
        let def = self.species.def();
        let d = (self.position - at).length();
        let flush = def.flush * strength.clamp(0.0, 2.0);
        let r = if flee && d < flush {
            Reaction::Flee(at)
        } else if d < 2.5 * flush.max(1.0) {
            Reaction::Alert
        } else {
            return;
        };
        let stronger = matches!(r, Reaction::Flee(_)) && !matches!(self.reaction, Reaction::Flee(_));
        if self.reaction == Reaction::None || stronger {
            self.reaction = r;
            self.react_in = self.rng.range(0.15, 0.6);
        }
    }

    /// Flock event: leave for `tree` within `delay` seconds along the shared path `c1, c2`
    /// (formation offset already applied), taking `dur` seconds.
    pub fn flock_to(&mut self, tree: usize, delay: f32, c1: Vec3, c2: Vec3, dur: f32) {
        if let State::Perched { left, .. } = &mut self.state {
            *left = left.min(delay);
            self.forced = Some((tree, c1, c2, dur));
        }
    }

    fn roam(&self) -> f32 {
        match self.behaviour {
            Behaviour::Resident { roam, .. } | Behaviour::Open { roam, .. } => roam,
            _ => 1.0,
        }
    }

    fn dwell(&mut self) -> f32 {
        let roam = self.roam();
        range(&mut self.rng, self.species.def().dwell) / roam
    }

    fn speed(&mut self, over: Option<f32>) -> f32 {
        over.unwrap_or_else(|| range(&mut self.rng, self.species.def().speed)).clamp(1.0, 40.0)
    }

    fn perch_in(&mut self, tree: usize, entities: &[Entity], song_post: bool) -> Anchor {
        let r = match entities[tree].geometry {
            Geometry::Tree { canopy_radius, .. } => canopy_radius,
            _ => 1.0,
        };
        let phi = self.rng.uniform() * TAU;
        let d = 0.8 * r * self.rng.uniform().sqrt();
        // Song posts sit at the species' typical height (thrushes at the top, robins inside).
        let z = if song_post { (self.species.def().post + self.rng.range(-0.4, 0.4)).clamp(-1.5, 2.0) } else { self.rng.range(-0.5, 1.0) };
        Anchor::Tree { tree, offset: Vec3::new(d * phi.cos(), d * phi.sin(), z) }
    }

    fn resolve(&self, at: Anchor, entities: &[Entity]) -> Vec3 {
        match at {
            Anchor::Tree { tree, offset } => entities[tree].canopy() + offset,
            Anchor::Point(p) => p,
        }
    }

    /// Centre and radius of the bird's living area.
    fn area(&self, entities: &[Entity]) -> (Vec3, f32) {
        match self.behaviour {
            Behaviour::Open { center, radius, .. } => (center, radius),
            _ => match self.home {
                Some(h) => (entities[h].position, self.species.def().territory),
                None => (flat(self.position), self.species.def().territory),
            },
        }
    }

    /// A feeding spot on the ground: near `near` (a short hop) or anywhere in the territory,
    /// not in the river.
    fn ground_spot(&mut self, near: Option<(Vec3, f32)>, entities: &[Entity]) -> Vec3 {
        let (c, r) = match near {
            Some(n) => n,
            None => {
                let (c, r) = self.area(entities);
                // Most feeding happens near the home tree, under and around its crown.
                if self.home.is_some() { (c, (0.45 * r).clamp(2.5, 9.0)) } else { (c, r) }
            }
        };
        let mut p = Vec3::new(c.x, c.y, self.ground_z + 0.05);
        for _ in 0..6 {
            let a = self.rng.uniform() * TAU;
            let d = r * self.rng.uniform().sqrt().max(0.15);
            p = Vec3::new(c.x + d * a.cos(), c.y + d * a.sin(), self.ground_z + 0.05);
            // Keep clear of the water and of the listener's feet.
            if water_distance(entities, p) > 1.2 && flat(p).length() > 1.2 {
                break;
            }
        }
        p
    }

    /// Trees within the territory (other than `except`); falls back to any tree.
    fn territory_tree(&mut self, entities: &[Entity], trees: &[usize], except: Option<usize>) -> Option<usize> {
        if trees.is_empty() || !self.species.def().trees {
            return None;
        }
        let open = matches!(self.behaviour, Behaviour::Open { .. });
        let (c, r) = self.area(entities);
        let mut picks = [0usize; 16];
        let mut n = 0;
        for &t in trees {
            if Some(t) != except && n < picks.len() && (flat(entities[t].position) - flat(c)).length() <= r {
                picks[n] = t;
                n += 1;
            }
        }
        if open && n == 0 {
            return None;
        }
        // Now and then a longer excursion across the scene.
        if n == 0 || (!open && self.rng.uniform() < 0.12) {
            let t = trees[(self.rng.next_u32() as usize) % trees.len()];
            if Some(t) != except {
                return Some(t);
            }
            return if n > 0 { Some(picks[0]) } else { trees.iter().copied().find(|&x| Some(x) != except) };
        }
        Some(picks[(self.rng.next_u32() as usize) % n])
    }

    /// A gently arched path from `a` to `b`, shaped by the flight style.
    fn arc(&mut self, a: Vec3, b: Vec3, display: bool) -> (Vec3, Vec3) {
        let d = b - a;
        let l = d.length();
        let style = self.species.def().flight;
        let k = match style {
            // Low and straight: thrushes and robins keep under cover.
            FlightStyle::Direct => 0.05,
            FlightStyle::Silent => 0.04,
            FlightStyle::Clatter => 0.1,
            _ => 0.12,
        };
        let mut up = (k * l + 0.5) * self.rng.range(0.5, 1.5);
        if display {
            // Wood pigeon display flight: a steep climb, wing claps at the top, glide down.
            up = 5.0 + 0.2 * l;
        }
        let side = normalize(cross(d, Vec3::new(0.0, 0.0, 1.0))) * (self.rng.range(-0.25, 0.25) * l);
        let up = Vec3::new(0.0, 0.0, up);
        let (u1, u2) = if display { (0.3, 0.55) } else { (0.33, 0.66) };
        (a + d * u1 + up + side, a + d * u2 + up + side)
    }

    fn fly_path(&mut self, to: Anchor, then: Then, c1: Vec3, c2: Vec3, dur: f32, display: bool) {
        self.state = State::Flying { from: self.position, c1, c2, to, t: 0.0, dur, then, display };
        self.takeoff = true;
    }

    fn fly(&mut self, to: Anchor, then: Then, speed: f32, entities: &[Entity]) {
        self.fly_display(to, then, speed, entities, false);
    }

    fn fly_display(&mut self, to: Anchor, then: Then, speed: f32, entities: &[Entity], display: bool) {
        let from = self.position;
        let target = self.resolve(to, entities);
        let (c1, c2) = self.arc(from, target, display);
        let k = if display { 1.6 } else { 1.15 };
        let dur = ((target - from).length() * k / speed).max(0.8);
        self.fly_path(to, then, c1, c2, dur, display);
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

    fn hour(&self, env: &Environment) -> f32 {
        env.hour + self.wake
    }

    fn awake(&self, env: &Environment) -> f32 {
        self.species.flight_activity(self.hour(env)) * (1.0 - 0.9 * env.rain.clamp(0.0, 1.0))
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
                self.state = State::Flying { from, c1, c2, to: Anchor::Point(to), t: 0.0, dur, then: Then::Leave, display: false };
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
                self.state = State::Perched { at: Anchor::Point(self.way[0]), left: linger, mode: Mode::Sing };
            }
            Behaviour::Resident { .. } | Behaviour::Open { .. } => {
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
            Behaviour::Resident { .. } | Behaviour::Open { .. } => 10.0,
        }
    }

    /// Flock take-off along the shared path.
    fn take_forced(&mut self, entities: &[Entity]) -> bool {
        let Some((tree, c1, c2, dur)) = self.forced.take() else { return false };
        let at = self.perch_in(tree, entities, false);
        self.fly_path(at, Then::Perch(Mode::Rest), c1, c2, dur, false);
        true
    }

    /// Hawking / soaring entry point for an area.
    fn start_hawk(&mut self, entities: &[Entity]) {
        let def = self.species.def();
        let (c, r) = self.area(entities);
        let water = water_distance(entities, c) < 30.0;
        let (radius, height) = match def.flight {
            // Swallows feed low over the meadow or the river.
            FlightStyle::Swooping => (self.rng.range(7.0, 14.0), self.rng.range(2.5, 6.0)),
            _ => (self.rng.range(12.0, 25.0).min(r.max(12.0)), self.rng.range(10.0, 25.0)),
        };
        // Swallows like the water: centre the loops over the river if there is one near.
        let mut center = Vec3::new(c.x, c.y, self.ground_z);
        if water && def.flight == FlightStyle::Swooping {
            for e in entities {
                if let Geometry::River { .. } = e.geometry {
                    center = Vec3::new(e.position.x, e.position.y, self.ground_z);
                    break;
                }
            }
        }
        let v = self.speed(None);
        let omega = v / (1.2 * radius) * if self.rng.uniform() < 0.5 { 1.0 } else { -1.0 };
        let entry = hawk_pos(center, radius, height, 0.0, omega, self.phase, def.flight, self.ground_z);
        let left = self.rng.range(20.0, 80.0) * self.roam().sqrt();
        self.state = State::Hawking { center, radius, height, t: 0.0, omega, left };
        // Fly there first.
        let p = self.position;
        let (c1, c2) = self.arc(p, entry, false);
        let dur = ((entry - p).length() * 1.15 / v).max(0.8);
        let hawk = self.state;
        self.fly_path(Anchor::Point(entry), Then::Hawk, c1, c2, dur, false);
        self.pending_hawk = Some(hawk);
    }

    /// End of a perch or ground bout: what next? The hourly budget of a resident.
    fn decide(&mut self, env: &Environment, entities: &[Entity], trees: &[usize]) {
        if self.take_forced(entities) {
            return;
        }
        let def = self.species.def();
        let awake = self.awake(env);
        let song = self.species.song_activity(self.hour(env)) * (1.0 - 0.8 * env.rain.clamp(0.0, 1.0));
        let on_ground = matches!(self.state, State::Ground { .. }) || matches!(self.state, State::Perched { at: Anchor::Point(_), .. });
        let current = self.perched_tree();
        let open = matches!(self.behaviour, Behaviour::Open { .. });
        let roam = self.roam();

        if awake < 0.2 {
            // Roost (diurnal birds at night, owls by day): up into a tree and stay there.
            if on_ground && !open {
                if let Some(t) = self.territory_tree(entities, trees, None) {
                    let at = self.perch_in(t, entities, false);
                    let v = self.speed(None);
                    self.fly(at, Then::Perch(Mode::Rest), v, entities);
                    return;
                }
            }
            let left = range(&mut self.rng, def.dwell) * 2.0;
            let mode = if song > 0.05 { Mode::Sing } else { Mode::Rest };
            match &mut self.state {
                State::Perched { left: l, mode: m, .. } => {
                    *l = left;
                    *m = mode;
                }
                State::Ground { left: l, .. } => *l = left,
                _ => {}
            }
            return;
        }

        let calm = if self.calm > 0.0 { 0.2 } else { 1.0 };
        let w_sing = song * calm * if self.answer > 0.0 { 2.0 } else { 1.0 };
        let w_ground = def.ground * (1.0 - 0.5 * song) * awake;
        let w_air = def.aerial * awake * if def.flight == FlightStyle::Hovering { song } else { 1.0 };
        let w_rest = 0.2;
        let w_move = if trees.len() > 1 && !open { 0.3 * roam.min(2.0) * awake } else { 0.0 };
        let total = w_sing + w_ground + w_air + w_rest + w_move;
        let mut x = self.rng.uniform() * total;
        let mut pick = |w: f32| {
            let hit = x < w;
            x -= w;
            hit
        };
        let v = self.speed(None);
        if pick(w_sing) {
            let tree_singer = !open && def.flight != FlightStyle::Hovering;
            if on_ground && tree_singer {
                if let Some(t) = self.territory_tree(entities, trees, None) {
                    let at = self.perch_in(t, entities, true);
                    self.fly(at, Then::Perch(Mode::Sing), v, entities);
                    return;
                }
            }
            // Sometimes move to another song post first.
            if current.is_some() && self.rng.uniform() < 0.3 * roam.min(1.5) {
                if let Some(t) = self.territory_tree(entities, trees, current) {
                    let at = self.perch_in(t, entities, true);
                    self.fly(at, Then::Perch(Mode::Sing), v, entities);
                    return;
                }
            }
            let left = self.dwell();
            match self.state {
                State::Perched { at, .. } => self.state = State::Perched { at, left, mode: Mode::Sing },
                State::Ground { at, .. } => self.state = State::Perched { at: Anchor::Point(at), left, mode: Mode::Sing },
                _ => {}
            }
        } else if pick(w_ground) {
            if let State::Ground { at, .. } = self.state {
                if self.rng.uniform() < 0.5 {
                    // Keep feeding here a while longer.
                    let left = self.rng.range(8.0, 25.0);
                    self.state = State::Ground { at, from: at, hop: 1.0, next_hop: 0.3, left };
                    return;
                }
            }
            let spot = self.ground_spot(None, entities);
            self.fly(Anchor::Point(spot), Then::Ground, v, entities);
        } else if pick(w_air) {
            if def.flight == FlightStyle::Hovering {
                if on_ground {
                    let base = self.position;
                    let height = self.rng.range(25.0, 50.0);
                    let dur = self.rng.range(40.0, 100.0);
                    self.state = State::SongFlight { base, height, t: 0.0, dur };
                    self.takeoff = true;
                } else {
                    let spot = self.ground_spot(None, entities);
                    self.fly(Anchor::Point(spot), Then::Ground, v, entities);
                }
            } else {
                self.start_hawk(entities);
            }
        } else if pick(w_rest) {
            let left = 0.6 * self.dwell();
            match self.state {
                State::Perched { at, .. } => self.state = State::Perched { at, left, mode: Mode::Rest },
                State::Ground { at, .. } if open => self.state = State::Perched { at: Anchor::Point(at), left, mode: Mode::Rest },
                _ => {
                    if let Some(t) = self.territory_tree(entities, trees, None) {
                        let at = self.perch_in(t, entities, false);
                        self.fly(at, Then::Perch(Mode::Rest), v, entities);
                    }
                }
            }
        } else if let Some(t) = self.territory_tree(entities, trees, current) {
            // Change trees; a wood pigeon sometimes makes a display flight of it.
            let at = self.perch_in(t, entities, song > 0.3);
            let mode = if self.rng.uniform() < song { Mode::Sing } else { Mode::Rest };
            let display = def.flight == FlightStyle::Clatter && self.rng.uniform() < 0.25;
            self.fly_display(at, Then::Perch(mode), v, entities, display);
        }
    }

    fn perch_expired(&mut self, env: &Environment, entities: &[Entity], trees: &[usize]) {
        match self.behaviour {
            Behaviour::Resident { .. } | Behaviour::Open { .. } => self.decide(env, entities, trees),
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
                    self.fly(Anchor::Point(self.way[self.iway]), Then::Perch(Mode::Sing), v, entities);
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
            Then::Perch(mode) => {
                let left = match self.behaviour {
                    Behaviour::Approach { .. } => {
                        // Linger longest at the closest stop.
                        let l = self.linger();
                        if self.iway + 3 == self.nway { l * 1.5 } else { l }
                    }
                    Behaviour::Circle { interval, .. } => range(&mut self.rng, interval),
                    _ if mode == Mode::Alert => self.rng.range(4.0, 10.0),
                    _ => self.dwell(),
                };
                self.state = State::Perched { at: to, left, mode };
            }
            Then::Ground => {
                let p = self.position;
                let left = self.rng.range(8.0, 30.0) / self.roam().sqrt();
                self.state = State::Ground { at: p, from: p, hop: 1.0, next_hop: self.rng.range(0.3, 1.0), left };
            }
            Then::Hawk => {
                if let Some(h) = self.pending_hawk.take() {
                    self.state = h;
                } else {
                    self.state = State::Perched { at: to, left: 1.0, mode: Mode::Rest };
                }
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
            let at = self.perch_in(h, entities, false);
            self.fly(at, Then::Perch(Mode::Sing), v, entities);
        } else if let Behaviour::Open { .. } = self.behaviour {
            let spot = self.ground_spot(None, entities);
            self.fly(Anchor::Point(spot), Then::Ground, v, entities);
        } else {
            let dir = normalize(Vec3::new(self.position.x, self.position.y, 0.0));
            let side = Vec3::new(-dir.y, dir.x, 0.0);
            let exit = (dir + side * 0.5) * OFFSTAGE + Vec3::new(0.0, 0.0, 8.0);
            self.fly(Anchor::Point(exit), Then::Leave, v, entities);
        }
    }

    /// Carries out a pending reaction to a disturbance.
    fn react(&mut self, entities: &[Entity], trees: &[usize]) {
        let r = core::mem::replace(&mut self.reaction, Reaction::None);
        let def = self.species.def();
        match r {
            Reaction::None => {}
            Reaction::Alert => {
                self.alarm = self.rng.range(4.0, 10.0);
                self.calm = self.rng.range(20.0, 45.0);
                match self.state {
                    State::Perched { at, .. } => {
                        let left = self.alarm;
                        self.state = State::Perched { at, left, mode: Mode::Alert };
                    }
                    // Ground feeders fly up into cover and scold from there.
                    State::Ground { .. } if !matches!(self.behaviour, Behaviour::Open { .. }) => {
                        if let Some(t) = self.territory_tree(entities, trees, None) {
                            let at = self.perch_in(t, entities, false);
                            let v = self.speed(None);
                            self.fly(at, Then::Perch(Mode::Alert), v, entities);
                        }
                    }
                    _ => {}
                }
            }
            Reaction::Flee(from) => {
                self.calm = self.rng.range(30.0, 60.0);
                if matches!(self.state, State::Flying { .. } | State::Hawking { .. } | State::SongFlight { .. }) {
                    self.alarm = 2.0;
                    return;
                }
                // The tree farthest from the danger (within reason), else away on the ground.
                let mut best: Option<(usize, f32)> = None;
                for &t in trees {
                    let d = (flat(entities[t].position) - flat(from)).length();
                    let home = (flat(entities[t].position) - flat(self.position)).length();
                    let score = d - 0.3 * home;
                    if Some(t) != self.perched_tree() && d > 6.0 && best.is_none_or(|b| score > b.1) {
                        best = Some((t, score));
                    }
                }
                let v = self.speed(None) * 1.2;
                match best {
                    Some((t, _)) if !matches!(self.behaviour, Behaviour::Open { .. }) || def.aerial < 0.5 => {
                        let at = self.perch_in(t, entities, false);
                        self.fly(at, Then::Perch(Mode::Alert), v, entities);
                    }
                    _ => {
                        let away = normalize(flat(self.position - from));
                        let spot = self.ground_spot(Some((self.position + away * 12.0, 4.0)), entities);
                        self.fly(Anchor::Point(spot), Then::Ground, v, entities);
                    }
                }
                if let State::Flying { dur, .. } = self.state {
                    self.alarm = dur.min(4.0);
                }
            }
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
        if self.reaction != Reaction::None {
            self.react_in -= dt;
            if self.react_in <= 0.0 {
                self.react(entities, trees);
            }
        }
        self.calm = (self.calm - dt).max(0.0);
        self.alarm = (self.alarm - dt).max(0.0);
        self.answer = (self.answer - dt).max(0.0);

        let def = self.species.def();
        let song = self.species.song_activity(self.hour(env)) * (1.0 - 0.8 * env.rain.clamp(0.0, 1.0));
        let density = (env.birds / 0.6).clamp(0.0, 1.67);
        let awake = self.awake(env);
        // Entrances (fly-bys, approaches, circling) are performances: once on stage the bird
        // sings at least moderately, but only species that are awake at this hour enter.
        let performer = !self.is_resident();
        let before = self.position;
        let mut out = Output { position: self.position, ..Default::default() };

        match self.state {
            State::Perched { at, left, mode } => {
                self.position = self.resolve(at, entities);
                // Not the hour for song (an owl by day): this is a rest, not a song post.
                let mode = if mode == Mode::Sing && song < 0.03 && !performer { Mode::Rest } else { mode };
                if let State::Perched { mode: m, .. } = &mut self.state {
                    *m = mode;
                }
                match mode {
                    Mode::Alert => {
                        out.calling = true;
                        out.alarm = true;
                        out.activity = if self.alarm > 0.0 { (0.85 * density.max(0.5)).min(1.0) } else { 0.0 };
                    }
                    Mode::Sing | Mode::Rest => {
                        let base = if performer { song.max(0.6) } else { song };
                        let mut a = base * density;
                        if mode == Mode::Rest {
                            a *= 0.12;
                        }
                        if self.calm > 0.0 {
                            a *= 0.2;
                        }
                        // Counter-singing: reply to a neighbour that just finished.
                        if self.answer > 0.0 && song > 0.05 && mode == Mode::Sing {
                            a = a.max((0.95 * density).min(1.0));
                            out.reply = !self.hold;
                        }
                        // ... and do not start while one of them sings (still wanting to: the
                        // mix keeps this bird's place in the phrase queue).
                        out.wait = self.hold && !performer;
                        out.activity = a.min(1.0);
                    }
                }
                // Shifting on the branch, settling the feathers: a few soft beats.
                self.next_flick -= dt;
                if self.next_flick <= 0.0 {
                    let u = |r: &mut u64| {
                        *r = r.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                        (*r >> 40) as f32 / 16_777_216.0
                    };
                    self.flick = 0.05 + 0.2 * u(&mut self.flick_rng);
                    self.next_flick = 12.0 + 50.0 * u(&mut self.flick_rng);
                }
                if self.flick > 0.0 {
                    self.flick -= dt;
                    out.flap = 0.3;
                }
                let left = left - dt;
                if let State::Perched { left: l, .. } = &mut self.state {
                    *l = left;
                }
                if mode == Mode::Alert && self.alarm <= 0.0 && left > 0.0 {
                    if let State::Perched { mode: m, left: l, .. } = &mut self.state {
                        *m = Mode::Rest;
                        *l = l.max(3.0);
                    }
                }
                if left <= 0.0 {
                    self.perch_expired(env, entities, trees);
                }
            }
            State::Ground { at, from, hop, next_hop, left } => {
                // Hop: 0.12 s parabola to a spot a few decimetres away, with a wing flick.
                let (mut at, mut from, mut hop, mut next_hop) = (at, from, hop, next_hop);
                if hop < 1.0 {
                    hop = (hop + dt / 0.12).min(1.0);
                    let p = from + (at - from) * hop;
                    self.position = Vec3::new(p.x, p.y, p.z + 0.12 * (PI * hop).sin());
                    out.flap = if def.flight == FlightStyle::Clatter { 0.0 } else { 0.25 };
                } else {
                    self.position = at;
                    next_hop -= dt;
                    if next_hop <= 0.0 {
                        from = at;
                        let a = self.rng.uniform() * TAU;
                        let d = self.rng.range(0.1, 0.45);
                        let mut to = Vec3::new(at.x + d * a.cos(), at.y + d * a.sin(), self.ground_z + 0.05);
                        // Stay in the area: drift back towards where feeding started.
                        let (c, r) = self.area(entities);
                        if (flat(to) - flat(c)).length() > r {
                            to = at + normalize(flat(c - at)) * d;
                        }
                        at = to;
                        hop = 0.0;
                        // Thrushes run-and-stop; finches and pigeons potter about.
                        next_hop = match def.flight {
                            FlightStyle::Direct => self.rng.range(0.6, 2.5),
                            _ => self.rng.range(0.25, 1.2),
                        };
                    }
                }
                out.calling = true;
                out.alarm = self.alarm > 0.0;
                out.activity = if out.alarm { 0.8 } else { (0.06 * density).min(1.0) };
                let left = left - dt;
                self.state = State::Ground { at, from, hop, next_hop, left };
                if left <= 0.0 {
                    self.perch_expired(env, entities, trees);
                }
            }
            State::Flying { from, c1, c2, to, t, dur, then, display } => {
                let t = t + dt;
                let target = self.resolve(to, entities);
                let landing = matches!(then, Then::Perch(_) | Then::Ground);
                let u = if landing { ease(t / dur) } else { (t / dur).min(1.0) };
                let base = bezier(from, c1, c2, target, u);
                let dir = normalize(flat(target - from));
                let side = [dir.y, -dir.x, 0.0];
                let (off, mut flap) = flight_motion(def.flight, t, dur, side, self.phase);
                let off = if performer { Vec3::new(0.0, 0.0, 0.5 * off[2]) } else { v3(off) };
                let mut p = base + off;
                p.z = p.z.max(self.ground_z + 0.05);
                self.position = p;
                if display {
                    // Climb with strong beats, clap at the top, then glide down.
                    flap = if u < 0.32 {
                        0.75
                    } else if u < 0.42 {
                        1.0
                    } else if dur - t < 0.4 {
                        0.85
                    } else {
                        0.0
                    };
                }
                out.flap = flap;
                out.startle = core::mem::take(&mut self.takeoff) && def.flight == FlightStyle::Clatter && !display;
                out.calling = !def.sings_in_flight;
                out.activity = (def.call_rate * awake.max(0.3) * density).min(1.0);
                if self.alarm > 0.0 {
                    out.calling = true;
                    out.alarm = true;
                    out.activity = 0.9;
                }
                out.moving = true;
                if t >= dur {
                    self.position = target;
                    self.arrived(to, then, env);
                } else if let State::Flying { t: tt, .. } = &mut self.state {
                    *tt = t;
                }
            }
            State::Hawking { center, radius, height, t, omega, left } => {
                let t = t + dt;
                self.position = hawk_pos(center, radius, height, t, omega, self.phase, def.flight, self.ground_z);
                let side = [0.0, 1.0, 0.0];
                let (_, flap) = flight_motion(def.flight, t + 1.0, t + 11.0, side, self.phase);
                out.flap = flap;
                out.calling = !def.sings_in_flight;
                out.activity = (song.max(0.2) * def.call_rate.max(0.4) * density).min(1.0);
                if self.alarm > 0.0 {
                    out.calling = true;
                    out.alarm = true;
                    out.activity = 0.9;
                }
                out.moving = true;
                let left = left - dt;
                self.state = State::Hawking { center, radius, height, t, omega, left };
                if left <= 0.0 || awake < 0.15 {
                    // Done feeding: rest on a tree in the territory, or on the ground.
                    let v = self.speed(None);
                    match self.territory_tree(entities, trees, None).filter(|_| def.ground < 0.3 || self.rng.uniform() < 0.5) {
                        Some(tr) => {
                            let at = self.perch_in(tr, entities, false);
                            let mode = if def.sings_in_flight { Mode::Sing } else { Mode::Rest };
                            self.fly(at, Then::Perch(mode), v, entities);
                        }
                        None => {
                            let spot = self.ground_spot(None, entities);
                            self.fly(Anchor::Point(spot), Then::Ground, v, entities);
                        }
                    }
                }
            }
            State::SongFlight { base, height, t, dur } => {
                let t = t + dt;
                let u = (t / dur).min(1.0);
                // Climb in widening circles, hover high up singing, then drop.
                let (z, r, flap) = if u < 0.3 {
                    (height * ease(u / 0.3), 2.0 + 10.0 * u, 0.6)
                } else if u < 0.85 {
                    (height + 2.0 * (0.3 * t).sin(), 5.0, 0.55)
                } else {
                    let d = (u - 0.85) / 0.15;
                    // Down in steps, the last stretch a plummet on closed wings.
                    (height * (1.0 - ease(d)), 5.0 * (1.0 - d), if d < 0.6 { 0.5 } else { 0.0 })
                };
                let a = 0.5 * t + self.phase * TAU;
                self.position = Vec3::new(base.x + r * a.cos() - r, base.y + r * a.sin(), self.ground_z + 0.05 + z);
                let (_, beat) = flight_motion(FlightStyle::Hovering, t, dur, [0.0, 1.0, 0.0], self.phase);
                out.flap = if flap > 0.0 { beat.max(0.0).min(flap + 0.1) } else { 0.0 };
                out.calling = false;
                out.activity = (song.max(0.6) * density).min(1.0);
                out.moving = true;
                self.state = State::SongFlight { base, height, t, dur };
                if t >= dur {
                    let p = Vec3::new(self.position.x, self.position.y, self.ground_z + 0.05);
                    self.position = p;
                    let left = self.rng.range(10.0, 30.0);
                    self.state = State::Ground { at: p, from: p, hop: 1.0, next_hop: 1.0, left };
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
        if dt > 0.0 && !out.absent {
            let v = (self.position - before) * (1.0 / dt);
            // A perched bird that the user carries with its tree is not "flying".
            self.velocity = if out.moving { v } else { Vec3::default() };
        }
        out.position = self.position;
        self.last = out;
        out
    }
}

/// Feeding / soaring loop over an area: a slow Lissajous figure (no two laps alike).
/// Swallows dip low now and then (to the water or the grass), soarers drift up and down.
#[allow(clippy::too_many_arguments)]
fn hawk_pos(center: Vec3, r: f32, h: f32, t: f32, omega: f32, phase: f32, style: FlightStyle, ground_z: f32) -> Vec3 {
    let a = omega * t + phase * TAU;
    let x = r * a.cos() + 0.3 * r * (2.3 * a + 1.0).cos();
    let y = 0.75 * r * (1.31 * a).sin() + 0.2 * r * (0.7 * a).sin();
    let z = match style {
        FlightStyle::Swooping => h * (1.0 - 0.8 * (0.6 * a).sin().powi(8)) + 0.4 * (3.1 * a).sin(),
        _ => h * (1.0 + 0.15 * (0.4 * a).sin()),
    };
    Vec3::new(center.x + x, center.y + y, (ground_z + z).max(ground_z + 0.4))
}

fn rng_dir(rng: &mut Rng) -> Vec3 {
    let a = rng.uniform() * TAU;
    Vec3::new(a.cos(), a.sin(), 0.0)
}
