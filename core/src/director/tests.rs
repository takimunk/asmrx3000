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
            assert!((p.y.atan2(p.x) - bearing).abs() < 1e-3 || (p.y.atan2(p.x) - bearing).abs() > core::f32::consts::TAU - 1e-3);
            let em = &world.emitters[world.entities[thunder].emitters.start];
            assert!(em.controls.env.strike > 0 && em.controls.env.strike_distance_m >= 300.0);
        }
    }
    assert!(struck > 0);
    // Rain reached the emitters and the birds noticed.
    let em = &world.emitters[0];
    assert!(em.controls.env.rain_mm_h > 5.0);
}

// ---------------------------------------------------------------------------------------
// Life model
// ---------------------------------------------------------------------------------------

const OPEN_SCENE: &str = include_str!("../../scenes/open_country.json");

#[test]
fn residents_feed_on_the_ground_by_day_and_return_to_trees() {
    let mut sim = Sim::default_scene();
    let ground = sim.world.ground_z;
    let birds: Vec<usize> = [100, 101, 106, 110, 111].iter().map(|&id| sim.idx(id)).collect();
    let mut on_ground = vec![0.0f32; birds.len()];
    let mut in_tree_after = vec![false; birds.len()];
    sim.run(400.0, &Environment { hour: 10.0, ..Default::default() }, |_, w| {
        for (k, &b) in birds.iter().enumerate() {
            let z = w.entities[b].position.z;
            if z < ground + 0.3 {
                on_ground[k] += DT;
            } else if on_ground[k] > 0.0 && z > ground + 2.0 && w.entities[b].flags & object_flags::MOVING == 0 {
                in_tree_after[k] = true;
            }
        }
    });
    let feeders = on_ground.iter().filter(|&&t| t > 10.0).count();
    assert!(feeders >= 3, "ground time {on_ground:?}");
    assert!(in_tree_after.iter().filter(|&&b| b).count() >= 2, "back up: {in_tree_after:?}");
}

#[test]
fn flight_styles_shape_the_path_and_the_wingbeat() {
    // Chaffinch flights undulate with closed-wing glides; a swallow hawks low over the area.
    let mut sim = Sim::default_scene();
    let finch = sim.idx(106);
    let em = sim.world.entities[finch].emitters.start;
    let (mut glides, mut beats, mut flying) = (0, 0, 0);
    let mut prev_z: Option<f32> = None;
    let mut turns = 0;
    let mut rising = None;
    sim.run(400.0, &Environment { hour: 10.0, ..Default::default() }, |_, w| {
        let e = &w.entities[finch];
        if e.flags & object_flags::MOVING != 0 {
            flying += 1;
            let f = w.emitters[em].controls.flap;
            if f == 0.0 {
                glides += 1;
            } else {
                beats += 1;
            }
            if let Some(pz) = prev_z {
                let up = e.position.z > pz;
                if rising.is_some_and(|r| r != up) {
                    turns += 1;
                }
                rising = Some(up);
            }
            prev_z = Some(e.position.z);
        } else {
            prev_z = None;
            rising = None;
        }
    });
    assert!(flying > 500, "the chaffinch flew ({flying} blocks)");
    assert!(glides > flying / 6 && beats > flying / 3, "glides {glides}, beats {beats} of {flying}");
    assert!(turns > 6, "undulating flight: {turns} up/down turns");
    assert!(sim.world.emitters[em].controls.wings_driven);
}

/// Renders an engine for `secs`, calling `f(t, birds, objects)` every block.
fn engine_run(e: &crate::Engine, secs: f32, mut f: impl FnMut(f32, &[BirdInfo], &[crate::ObjectInfo])) {
    let (mut l, mut r) = (vec![0.0f32; 512], vec![0.0f32; 512]);
    let mut b = [BirdInfo::default(); 64];
    let mut o = [crate::ObjectInfo::default(); 64];
    let mut t = 0.0;
    while t < secs {
        e.render(&mut l, &mut r);
        t += 512.0 / 48_000.0;
        let nb = e.birds(&mut b);
        let no = e.objects(&mut o);
        f(t, &b[..nb], &o[..no]);
    }
}

#[test]
fn neighbours_take_turns_and_answer() {
    // Robins and blackbirds at dawn: same-species neighbours take turns (wait while the
    // other sings, answer when it stops), so they overlap far less than chance.
    let e = crate::Engine::new(48_000.0);
    e.set_param(crate::param::PLAYING, 1.0);
    e.set_param(crate::param::TIME_OF_DAY, 5.2);
    let song = |b: &BirdInfo| b.flags & bird_flags::VOCAL != 0 && b.flags & bird_flags::CALLING == 0;
    let pairs = [(101u32, 112u32), (110, 111), (100, 106)];
    let mut n = 0usize;
    let mut stats = [(0usize, 0usize, 0usize); 3];
    engine_run(&e, 240.0, |_, b, _| {
        n += 1;
        for (k, (x, y)) in pairs.iter().enumerate() {
            let sx = b.iter().find(|i| i.id == *x).is_some_and(song);
            let sy = b.iter().find(|i| i.id == *y).is_some_and(song);
            stats[k].0 += sx as usize;
            stats[k].1 += sy as usize;
            stats[k].2 += (sx && sy) as usize;
        }
    });
    let (mut both, mut chance) = (0.0, 0.0);
    for (a, b, ab) in stats {
        let (pa, pb) = (a as f32 / n as f32, b as f32 / n as f32);
        assert!(pa > 0.03 && pb > 0.03, "both sing: {pa} {pb}");
        both += ab as f32 / n as f32;
        chance += pa * pb;
    }
    assert!(both < 0.5 * chance, "overlap {both:.4} vs {chance:.4} by chance");
}

#[test]
fn dragging_a_tree_alarms_its_birds_and_far_makes_them_flee() {
    let mut sim = Sim::default_scene();
    let day = Environment { hour: 10.0, ..Default::default() };
    sim.run(2.0, &day, |_, _| {});
    let tree = sim.idx(2);
    let birds: Vec<usize> = [100, 106].iter().map(|&id| sim.idx(id)).filter(|&b| sim.director.agents().iter().any(|a| a.entity == b && a.perched_tree() == Some(tree))).collect();
    assert!(!birds.is_empty());
    // A short nudge: alarm calls, no flight.
    let p0 = sim.world.entities[tree].position;
    sim.world.set_position(tree, p0 + Vec3::new(0.5, 0.0, 0.0));
    let mut alarm = false;
    sim.run(3.0, &day, |_, w| {
        alarm |= birds.iter().any(|&b| w.emitters[w.entities[b].emitters.start].controls.alarm);
    });
    assert!(alarm, "scolding");
    // Carried 6 m: they leave the tree.
    sim.world.set_position(tree, p0 + Vec3::new(6.5, 0.0, 0.0));
    let mut fled = false;
    sim.run(3.0, &day, |_, w| {
        fled |= birds.iter().any(|&b| w.entities[b].flags & object_flags::MOVING != 0);
    });
    assert!(fled, "fled the moving tree");
}

#[test]
fn skylark_song_flight_and_gulls_soar_in_the_open() {
    let e = crate::Engine::from_json(48_000.0, OPEN_SCENE).unwrap();
    e.set_param(crate::param::PLAYING, 1.0);
    let ground = -1.7;
    let (mut lark_top, mut sang_high, mut gull_top, mut gull_in_tree) = (f32::MIN, false, f32::MIN, false);
    engine_run(&e, 300.0, |_, b, o| {
        let z = |id: u32| o.iter().find(|x| x.id == id).map_or(ground, |x| x.z) - ground;
        let lark = b.iter().find(|x| x.id == 100).unwrap();
        lark_top = lark_top.max(z(100));
        sang_high |= z(100) > 15.0 && lark.state == bird_state::SONG_FLIGHT && lark.flags & bird_flags::VOCAL != 0;
        let gull = b.iter().find(|x| x.id == 102).unwrap();
        gull_top = gull_top.max(z(102));
        gull_in_tree |= gull.state != bird_state::FLYING && gull.state != bird_state::SOARING && z(102) > 1.0;
    });
    assert!(lark_top > 20.0 && sang_high, "lark climbed to {lark_top} m, sang up there: {sang_high}");
    assert!(gull_top > 8.0, "gull soared to {gull_top} m");
    assert!(!gull_in_tree, "gulls do not perch in trees");
}

#[test]
fn bird_telemetry_through_the_c_abi() {
    let e = crate::Engine::new(48_000.0);
    e.set_param(crate::param::PLAYING, 1.0);
    let (mut l, mut r) = (vec![0.0f32; 512], vec![0.0f32; 512]);
    for _ in 0..200 {
        e.render(&mut l, &mut r);
    }
    let mut info = [BirdInfo::default(); 64];
    // SAFETY: live engine, writable buffer of 64.
    let n = unsafe { crate::asmr_get_birds(&e, info.as_mut_ptr(), 64) } as usize;
    assert_eq!(n, 13);
    let mut objs = [crate::ObjectInfo::default(); 64];
    let no = e.objects(&mut objs);
    for b in &info[..n] {
        let o = objs[..no].iter().find(|o| o.id == b.id).expect("bird id is an object id");
        assert_eq!(o.kind, crate::ObjectKind::Bird as u32);
        assert!((b.species as usize) < Species::ALL.len() && b.state <= bird_state::ABSENT && b.flight <= 6);
    }
    let blackbird = info[..n].iter().find(|b| b.id == 110).unwrap();
    assert_eq!(blackbird.species, Species::Blackbird.code());
    assert_eq!(blackbird.flight, FlightStyle::Direct.code());
    assert_eq!(core::mem::size_of::<BirdInfo>(), 36);
}

#[test]
fn blackbird_sings_with_its_neural_voice() {
    assert_eq!(neural_model_name(Species::Blackbird), Some("blackbird"));
    for s in Species::ALL {
        let neural = neural_model_name(s).is_some();
        let expect = matches!(s, Species::Chaffinch | Species::Robin | Species::SongThrush | Species::Nightingale | Species::Blackbird);
        assert_eq!(neural, expect, "{s:?}");
    }
}
