use super::land::{self, Land, CHUNK};
use super::*;
use crate::controls::id;
use crate::engine::{param, telemetry, Engine};
use crate::scene::{ObjectInfo, ObjectKind, SceneDesc};

fn flying_engine(speed: f32, seed: u32) -> Engine {
    let mut desc = SceneDesc::default_scene();
    desc.flight = Some(FlightDesc { start: true, speed: Some(speed), seed: Some(seed), ..Default::default() });
    let e = Engine::with_scene(48_000.0, &desc).unwrap();
    e.set_param(param::PLAYING, 1.0);
    e
}

fn run(e: &Engine, seconds: f32) -> f64 {
    let (mut l, mut r) = (vec![0.0f32; 512], vec![0.0f32; 512]);
    let mut energy = 0.0f64;
    let blocks = (seconds * 48_000.0 / 512.0) as usize;
    for _ in 0..blocks {
        e.render(&mut l, &mut r);
        for (a, b) in l.iter().zip(&r) {
            assert!(a.is_finite() && b.is_finite() && a.abs() <= 1.0 && b.abs() <= 1.0);
            energy += (*a as f64).powi(2) + (*b as f64).powi(2);
        }
    }
    (energy / (2.0 * (blocks * 512) as f64)).sqrt()
}

#[test]
fn landscape_is_deterministic() {
    let make = |seed| {
        let mut l = Land::new(seed, 0);
        l.stream(120.0, 0.35, usize::MAX);
        // Stream in odd steps: the content must not depend on the timing.
        let mut s = 120.0;
        while s < 900.0 {
            s += 7.3;
            l.stream(s, 0.35, 1);
        }
        l
    };
    let (a, b, c) = (make(7), make(7), make(8));
    assert_eq!(a.tree_tail - a.tree_head, b.tree_tail - b.tree_head);
    for abs in a.tree_head..a.tree_tail {
        let (x, y) = (a.tree(abs), b.tree(abs));
        assert_eq!((x.x, x.y, x.species, x.rank), (y.x, y.y, y.species, y.rank));
    }
    let fa: Vec<_> = a.features.iter().filter(|f| f.alive).map(|f| (f.kind, f.x)).collect();
    let fb: Vec<_> = b.features.iter().filter(|f| f.alive).map(|f| (f.kind, f.x)).collect();
    assert_eq!(fa, fb);
    let first = |l: &Land| l.tree(l.tree_head).x;
    assert_ne!(first(&a), first(&c), "another seed, another forest");
    // Streaming keeps a bounded window around the listener.
    assert!(a.chunk_tail - a.chunk_head <= land::MAX_CHUNKS as i64);
    assert!(((a.chunk_head + 1) as f64) * CHUNK >= 900.0 - land::BEHIND - CHUNK);
    assert!((a.chunk_tail as f64) * CHUNK >= 900.0 + land::AHEAD);
}

#[test]
fn landscapes_have_their_features() {
    // Over 5 km every biome brings trees, birds and water; the wild ones rock and falls.
    for (bi, b) in land::BIOMES.iter().enumerate() {
        let mut l = Land::new(42, bi as u32);
        let (mut trees, mut rivers, mut lakes, mut ridges, mut sites) = (0u64, 0, 0, 0, 0);
        let mut seen = std::collections::HashSet::new();
        let mut s = 120.0;
        while s < 5000.0 {
            l.stream(s, 0.5, usize::MAX);
            for f in l.features.iter().filter(|f| f.alive) {
                if seen.insert(f.id) {
                    match f.kind {
                        land::FeatureKind::River => rivers += 1,
                        land::FeatureKind::Lake => lakes += 1,
                        land::FeatureKind::Ridge => ridges += 1,
                        _ => {}
                    }
                }
            }
            sites = sites.max(l.sites.iter().filter(|x| x.alive).count());
            s += 50.0;
        }
        trees += l.tree_tail;
        assert!(trees > 1000, "{}: {trees} trees", b.name);
        assert!(rivers >= 5, "{}: {rivers} rivers", b.name);
        assert!(lakes + ridges >= 2, "{}: {lakes} lakes {ridges} ridges", b.name);
        assert!(sites > 5 && sites < land::MAX_SITES, "{}: {sites} sites", b.name);
    }
}

#[test]
fn flight_renders_and_stays_bounded() {
    let e = flying_engine(25.0, 3);
    let mut objs = [ObjectInfo::default(); 512];
    let mut most = 0;
    let mut rms = 0.0;
    for _ in 0..20 {
        rms += run(&e, 1.0) / 20.0;
        most = most.max(e.objects(&mut objs));
    }
    assert!(rms > 1e-3, "rms {rms}");
    assert!(most > 150, "at most {most} objects");
    assert!(e.telemetry(telemetry::FLIGHT) >= 1.0);
    assert!((e.telemetry(telemetry::FLIGHT_SPEED) - 25.0).abs() < 0.5);
    assert!(e.telemetry(telemetry::FLIGHT_ALTITUDE) > 8.0, "fast flight skims the canopy");
    let mut src = [crate::SourceInfo::default(); crate::MAX_SOURCES];
    let n = e.sources(&mut src);
    assert!(n > 10 && n <= EMITTERS, "{n} sounding");
    let n = e.objects(&mut objs);
    assert!(n <= bridge::OBJECTS, "{n} objects");
    assert!(objs[..n].iter().any(|o| o.kind == ObjectKind::Tree as u32));
    assert!(objs[..n].iter().all(|o| o.id >= land::ID_BASE));
}

#[test]
fn objects_enter_and_leave() {
    let e = flying_engine(10.0, 1);
    let (mut l, mut r) = (vec![0.0f32; 480], vec![0.0f32; 480]);
    let mut objs = [ObjectInfo::default(); 512];
    let mut before: Vec<ObjectInfo> = Vec::new();
    for _ in 0..100 {
        e.render(&mut l, &mut r);
    }
    let mut entered = 0;
    let mut left = 0;
    for _ in 0..40 {
        for _ in 0..25 {
            e.render(&mut l, &mut r); // 0.25 s
        }
        let n = e.objects(&mut objs);
        let now = &objs[..n];
        if !before.is_empty() {
            entered += now.iter().filter(|o| !before.iter().any(|b| b.id == o.id)).count();
            left += before.iter().filter(|b| !now.iter().any(|o| o.id == b.id)).count();
            // Trees are rigid: they come towards the listener at the flight speed.
            for o in now.iter().filter(|o| o.kind == ObjectKind::Tree as u32) {
                if let Some(b) = before.iter().find(|b| b.id == o.id) {
                    let moved = ((o.x - b.x).powi(2) + (o.y - b.y).powi(2)).sqrt();
                    assert!((moved - 2.5).abs() < 0.6, "tree {} moved {moved} m in 0.25 s", o.id);
                    assert!(o.x < b.x + 0.5);
                }
            }
        }
        before = now.to_vec();
    }
    assert!(entered > 50 && left > 50, "entered {entered}, left {left}");
}

#[test]
fn audio_is_deterministic() {
    let take = || {
        let e = flying_engine(12.0, 5);
        let (mut l, mut r) = (vec![0.0f32; 512], vec![0.0f32; 512]);
        let mut out = Vec::new();
        for _ in 0..(48_000 * 4 / 512) {
            e.render(&mut l, &mut r);
            out.extend_from_slice(&l);
        }
        out
    };
    assert_eq!(take(), take());
}

#[test]
fn controls_and_toggle() {
    let e = Engine::new(48_000.0);
    e.set_param(param::PLAYING, 1.0);
    // The FLIGHT group: speed macro, altitude follows it unless set.
    let low = e.control(id::FLIGHT_ALTITUDE);
    e.set_control(id::FLIGHT_SPEED, 25.0);
    assert!(e.control(id::FLIGHT_ALTITUDE) > low + 8.0);
    e.set_control(id::FLIGHT_ALTITUDE, 3.0);
    assert_eq!(e.control(id::FLIGHT_ALTITUDE), 3.0);
    assert_eq!(crate::controls::option_name(id::FLIGHT_BIOME, 2), Some("taiga"));
    // Scene objects until flight mode is on, streamed ones after the cross-fade.
    let mut objs = [ObjectInfo::default(); 512];
    let scene = e.objects(&mut objs);
    assert_eq!(scene, 31);
    e.set_param(param::FLIGHT, 1.0);
    run(&e, 2.0);
    assert_eq!(e.telemetry(telemetry::FLIGHT), 1.0);
    let n = e.objects(&mut objs);
    assert!(objs[..n].iter().all(|o| o.id >= land::ID_BASE));
    let river = objs[..n].iter().find(|o| o.kind == ObjectKind::River as u32 || o.kind == ObjectKind::Lake as u32);
    if let Some(o) = river {
        let mut pts = [0.0f32; 3 * 64];
        assert!(e.object_path(o.id, &mut pts) >= 2);
    }
    e.set_param(param::FLIGHT, 0.0);
    run(&e, 2.0);
    assert_eq!(e.telemetry(telemetry::FLIGHT), 0.0);
    assert_eq!(e.objects(&mut objs), scene);
}

#[test]
fn new_seed_new_landscape() {
    let e = flying_engine(6.0, 0);
    run(&e, 1.0);
    let mut objs = [ObjectInfo::default(); 512];
    let n = e.objects(&mut objs);
    let old: Vec<u32> = objs[..n].iter().map(|o| o.id).collect();
    e.set_control(id::FLIGHT_SEED, 77.0);
    run(&e, 1.5);
    let n = e.objects(&mut objs);
    assert!(n > 20);
    assert!(objs[..n].iter().all(|o| !old.contains(&o.id)), "old objects survived the reseed");
}

#[test]
fn birds_take_off_as_you_pass() {
    let mut desc = SceneDesc::default_scene();
    desc.environment.time_of_day = 8.0;
    desc.flight = Some(FlightDesc { start: true, speed: Some(12.0), altitude: Some(2.0), biome: Some("meadows".into()), ..Default::default() });
    let e = Engine::with_scene(48_000.0, &desc).unwrap();
    e.set_param(param::PLAYING, 1.0);
    e.set_param(param::BIRDS, 1.0);
    let (mut l, mut r) = (vec![0.0f32; 480], vec![0.0f32; 480]);
    let mut objs = [ObjectInfo::default(); 512];
    let mut flying = std::collections::HashSet::new();
    for _ in 0..60 * 20 {
        for _ in 0..5 {
            e.render(&mut l, &mut r);
        }
        let n = e.objects(&mut objs);
        for o in &objs[..n] {
            if o.kind == ObjectKind::Bird as u32 && o.flags & crate::scene::object_flags::MOVING != 0 {
                flying.insert(o.id);
            }
        }
    }
    assert!(flying.len() >= 3, "{} birds flushed in 60 s", flying.len());
}

#[test]
fn airflow_level_follows_speed() {
    assert!(airflow::Airflow::rms(6.0, 1.0) < 0.002);
    assert!(airflow::Airflow::rms(25.0, 1.0) > 0.02);
    let mut a = airflow::Airflow::new(1);
    let (mut l, mut r) = (vec![0.0f32; 64], vec![0.0f32; 64]);
    let mut e = (0.0f64, 0.0f64);
    for i in 0..(48_000 * 3 / 64) {
        l.fill(0.0);
        r.fill(0.0);
        // Head turned right: the flow hits the left ear.
        a.render(20.0, 1.0, -1.2, 48_000.0, &mut l, &mut r);
        if i > 48_000 / 64 {
            e.0 += l.iter().map(|v| (*v as f64).powi(2)).sum::<f64>();
            e.1 += r.iter().map(|v| (*v as f64).powi(2)).sum::<f64>();
        }
    }
    let rms = ((e.0 + e.1) / (2.0 * 2.0 * 48_000.0)).sqrt() as f32;
    let target = airflow::Airflow::rms(20.0, 1.0);
    assert!(rms > 0.5 * target && rms < 2.0 * target, "rms {rms} vs {target}");
    assert!(e.0 > 1.5 * e.1, "windward ear louder");
}

#[test]
#[ignore]
fn chunk_generation_cost() {
    // cargo test --release chunk_generation_cost -- --ignored --nocapture
    for (bi, b) in land::BIOMES.iter().enumerate() {
        let mut l = Land::new(9, bi as u32);
        l.stream(120.0, 0.35, usize::MAX);
        let mut worst = 0.0f64;
        let mut total = 0.0f64;
        let mut s = 120.0;
        let mut n = 0;
        while s < 4000.0 {
            s += 0.0333; // 25 m/s, one 64-sample block
            let t = std::time::Instant::now();
            l.stream(s, 0.35, 1);
            let dt = t.elapsed().as_secs_f64();
            worst = worst.max(dt);
            total += dt;
            n += 1;
        }
        println!("{:>12}: mean {:.2} us/block, worst {:.0} us", b.name, 1e6 * total / n as f64, 1e6 * worst);
    }
}

#[test]
fn flight_reaches_the_bed_output() {
    let e = flying_engine(10.0, 2);
    let mut chans = vec![vec![0.0f32; 512]; crate::bed::CHANNELS];
    let mut energy = 0.0f64;
    for k in 0..(48_000 * 3 / 512) {
        let mut refs: Vec<&mut [f32]> = chans.iter_mut().map(|c| c.as_mut_slice()).collect();
        e.render_bed(&mut refs, k % 2 == 0);
        for c in &chans {
            assert!(c.iter().all(|v| v.is_finite() && v.abs() <= 1.0));
            energy += c.iter().map(|v| (*v as f64).powi(2)).sum::<f64>();
        }
    }
    assert!(energy > 1e-3, "silent bed");
}
