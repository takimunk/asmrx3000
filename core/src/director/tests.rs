//! Director behaviour tests. Everything is seeded, so these are deterministic.

use super::*;
use crate::math::Vec3;
use crate::scene::{object_flags, SceneDesc, World, DEMO_SCENE_JSON};

const DT: f32 = 64.0 / 48_000.0;

struct Sim {
    world: World,
    director: Director,
    t: f32,
}

impl Sim {
    fn new(json: &str) -> Self {
        let desc = SceneDesc::from_json(json).unwrap();
        let world = World::build(&desc, 48_000.0).unwrap();
        let director = Director::new(&desc, &world);
        Self { world, director, t: 0.0 }
    }

    fn default_scene() -> Self {
        Self::new(crate::scene::DEFAULT_SCENE_JSON)
    }

    fn run(&mut self, secs: f32, env: &Environment, mut f: impl FnMut(f32, &World)) {
        let end = self.t + secs;
        while self.t < end {
            self.director.step(DT, env, &mut self.world);
            self.world.update_emitters();
            self.t += DT;
            f(self.t, &self.world);
        }
    }

    fn idx(&self, id: u32) -> usize {
        self.world.index_of(id).unwrap()
    }
}

fn demo_env() -> Environment {
    Environment { hour: 7.5, ..Default::default() }
}

#[test]
fn deterministic_for_a_seed() {
    let trace = || {
        let mut sim = Sim::default_scene();
        let mut out = Vec::new();
        sim.run(120.0, &Environment::default(), |_, w| {
            out.extend(w.entities.iter().map(|e| e.position));
        });
        out
    };
    assert_eq!(trace(), trace());
}

#[test]
fn flyby_passes_close_and_leaves() {
    let mut sim = Sim::new(DEMO_SCENE_JSON);
    let bird = sim.idx(200);
    let (mut closest, mut at) = (f32::MAX, 0.0);
    let mut start_dist = 0.0;
    let mut gone_after = None;
    sim.run(14.0, &demo_env(), |t, w| {
        let e = &w.entities[bird];
        let d = e.position.length();
        if (3.0..3.1).contains(&t) {
            start_dist = d;
        }
        if e.flags & object_flags::MOVING != 0 && d < closest {
            closest = d;
            at = t;
        }
        if t > 3.5 && gone_after.is_none() && e.flags & object_flags::ABSENT != 0 {
            gone_after = Some((t, d));
        }
    });
    assert!(start_dist > 25.0, "starts far: {start_dist}");
    assert!(closest < 3.0 && closest > 0.5, "closest {closest} at {at}");
    let (t, d) = gone_after.expect("fly-by never ended");
    assert!(d > 25.0 && t < 12.0, "left at {t}s, {d} m");
}

#[test]
fn approach_comes_close_then_recedes() {
    let mut sim = Sim::new(DEMO_SCENE_JSON);
    let bird = sim.idx(201);
    let mut track = Vec::new();
    sim.run(70.0, &demo_env(), |t, w| {
        let e = &w.entities[bird];
        if e.flags & object_flags::ABSENT == 0 {
            track.push((t, e.position.length()));
        }
    });
    let (t0, d0) = track[0];
    let &(tn, dn) = track.last().unwrap();
    let (tc, dc) = track.iter().cloned().fold((0.0, f32::MAX), |a, b| if b.1 < a.1 { b } else { a });
    assert!(d0 > 30.0 && (13.0..16.0).contains(&t0), "appears far: {d0} m at {t0}");
    assert!(dc < 5.0 && tc > t0 && tc < tn, "closest {dc} m at {tc}");
    assert!(dn > 30.0, "recedes: {dn} m at {tn}");
}

#[test]
fn flock_moves_together_to_another_tree() {
    let mut sim = Sim::new(DEMO_SCENE_JSON);
    let tree6 = sim.idx(6);
    let tree7 = sim.idx(7);
    let flock: Vec<usize> = [101, 102, 103, 104].iter().map(|&id| sim.idx(id)).collect();
    let near = |w: &World, b: usize, tree: usize| (w.entities[b].position - w.entities[tree].canopy()).length() < 4.5;
    sim.run(9.9, &demo_env(), |_, _| {});
    assert!(flock.iter().all(|&b| near(&sim.world, b, tree6)));
    let mut all_flying = false;
    sim.run(12.0, &demo_env(), |_, w| {
        all_flying |= flock.iter().all(|&b| w.entities[b].flags & object_flags::MOVING != 0);
    });
    assert!(all_flying, "the flock should be in the air together");
    assert!(flock.iter().all(|&b| near(&sim.world, b, tree7)), "landed on the far oak");
    assert!(flock.iter().all(|&b| sim.world.entities[b].flags & object_flags::MOVING == 0));
}

#[test]
fn circles_the_listener() {
    let mut sim = Sim::new(DEMO_SCENE_JSON);
    let bird = sim.idx(202);
    let mut on_circle = 0.0;
    let mut angle_travel = 0.0f32;
    let mut last: Option<f32> = None;
    sim.run(75.0, &demo_env(), |_, w| {
        let p = w.entities[bird].position;
        let r = (p.x * p.x + p.y * p.y).sqrt();
        if (r - 3.0).abs() < 0.2 && w.entities[bird].flags & object_flags::MOVING != 0 {
            on_circle += DT;
            let a = p.y.atan2(p.x);
            if let Some(l) = last {
                let mut d = a - l;
                if d > 3.0 {
                    d -= core::f32::consts::TAU;
                } else if d < -3.0 {
                    d += core::f32::consts::TAU;
                }
                angle_travel += d.abs();
            }
            last = Some(a);
        } else {
            last = None;
        }
    });
    assert!(on_circle > 4.0, "{on_circle}s on the circle");
    assert!(angle_travel > 3.5 * core::f32::consts::PI, "two laps: {angle_travel} rad");
}

#[test]
fn time_of_day_selects_voices() {
    let mut sim = Sim::default_scene();
    let night = Environment { hour: 23.66, ..Default::default() };
    let chaffinch = sim.world.entities[sim.idx(100)].emitters.start;
    let owl = sim.world.entities[sim.idx(105)].emitters.start;
    let nightingale = sim.world.entities[sim.idx(104)].emitters.start;
    let diurnal: Vec<usize> = [100, 101, 102, 103, 106].iter().map(|&id| sim.idx(id)).collect();
    let (mut finch, mut owl_a, mut ng_a, mut moved) = (0.0f32, 0.0f32, 0.0f32, false);
    sim.run(200.0, &night, |_, w| {
        finch = finch.max(w.emitters[chaffinch].controls.activity);
        owl_a = owl_a.max(w.emitters[owl].controls.activity);
        ng_a = ng_a.max(w.emitters[nightingale].controls.activity);
        moved |= diurnal.iter().any(|&b| w.entities[b].flags & object_flags::MOVING != 0);
    });
    assert_eq!(finch, 0.0);
    assert!(owl_a > 0.8 && ng_a > 0.8, "owl {owl_a}, nightingale {ng_a}");
    assert!(!moved, "diurnal birds roost at night");

    // Dawn: the chaffinch sings more than at noon.
    let act = |hour| {
        let mut sim = Sim::default_scene();
        let mut a = 0.0f32;
        sim.run(1.0, &Environment { hour, ..Default::default() }, |_, w| a = w.emitters[chaffinch].controls.activity);
        a
    };
    assert!(act(5.8) > act(13.0) * 1.5);
}

#[test]
fn moving_a_tree_carries_its_birds_and_rustle() {
    let mut sim = Sim::default_scene();
    sim.run(1.0, &Environment::default(), |_, _| {});
    let tree = sim.idx(2);
    let bird = sim.idx(100);
    let canopy = sim.world.entities[tree].emitters.start;
    let (b0, c0) = (sim.world.entities[bird].position, sim.world.emitters[canopy].position);
    let p = sim.world.entities[tree].position + Vec3::new(0.0, 5.0, 0.0);
    sim.world.set_position(tree, p);
    sim.run(DT, &Environment::default(), |_, _| {});
    let db = sim.world.entities[bird].position - b0;
    let dc = sim.world.emitters[canopy].position - c0;
    assert!((db - Vec3::new(0.0, 5.0, 0.0)).length() < 1e-3, "bird moved by {db:?}");
    assert!((dc - Vec3::new(0.0, 5.0, 0.0)).length() < 1e-3, "canopy moved by {dc:?}");
    // Birds are not user-movable.
    sim.world.set_position(bird, Vec3::new(0.0, 0.0, 0.0));
    assert!(sim.world.entities[bird].position.length() > 1.0);
}

#[test]
fn orbit_slider_circles_then_returns_home() {
    let mut sim = Sim::default_scene();
    let bird = sim.idx(100);
    let center = Vec3::new(0.0, 0.0, 1.8);
    let orbit = Environment { orbit: 0.5, ..Default::default() };
    sim.run(10.0, &orbit, |_, _| {});
    let r = (sim.world.entities[bird].position - center).length();
    assert!((r - 3.0).abs() < 0.3, "orbit radius {r}");
    sim.run(20.0, &Environment::default(), |_, _| {});
    let e = &sim.world.entities[bird];
    assert_eq!(e.flags & object_flags::MOVING, 0, "landed again");
    assert!((e.position - Vec3::new(0.0, 0.0, 1.8)).length() > 4.0);
}

#[test]
fn thunder_proxy_points_at_the_strike() {
    use crate::scene::{Geometry, SceneDesc, World};
    use crate::sources::ModelKind;
    let desc = SceneDesc::default_scene();
    let mut world = World::build(&desc, 48_000.0).unwrap();
    let mut dir = Director::new(&desc, &world);
    let storm = Environment { storm: 1.0, rain_mm_h: 10.0, ..Default::default() };
    let thunder = world.entities.iter().position(|e| matches!(e.geometry, Geometry::Source { model: ModelKind::Thunder, .. })).unwrap();
    let mut struck = 0;
    for _ in 0..(90.0 * 750.0) as usize {
        dir.step(1.0 / 750.0, &storm, &mut world);
        if dir.weather().struck() {
            struck += 1;
            let p = world.entities[thunder].position;
            assert!((p.length() - super::THUNDER_PROXY_M).abs() < 0.01);
            let (bearing, _) = dir.weather().strike_position();
            assert!((p.y.atan2(p.x) - bearing).abs() < 1e-3 || (p.y.atan2(p.x) - bearing).abs() > 6.28);
            let em = &world.emitters[world.entities[thunder].emitters.start];
            assert!(em.controls.env.strike > 0 && em.controls.env.strike_distance_m >= 300.0);
        }
    }
    assert!(struck > 0);
    // Rain reached the emitters and the birds noticed.
    let em = &world.emitters[0];
    assert!(em.controls.env.rain_mm_h > 5.0);
}
