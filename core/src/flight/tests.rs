use super::land::{self, Land};
use super::*;
use crate::controls::id;
use crate::engine::{param, telemetry, Engine};
use crate::scene::{ObjectInfo, ObjectKind, SceneDesc};

fn flying_engine(speed: f32, seed: u32) -> Engine {
    let mut desc = SceneDesc::default_scene();
    desc.flight = Some(FlightDesc { start: true, speed: Some(speed), seed: Some(seed), stops: Some(0.0), ..Default::default() });
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

/// Trees around (x, y) as (x, y, species, rank), sorted.
fn trees_near(l: &Land, x: f64, y: f64, r: f64) -> Vec<(i64, i64, u8, u32)> {
    let mut v: Vec<_> = l
        .tiles_near(x, y, r)
        .flat_map(|t| l.tile_trees(t).iter())
        .filter(|t| (t.x - x).hypot(t.y - y) < r)
        .map(|t| ((t.x * 1000.0) as i64, (t.y * 1000.0) as i64, t.species, t.rank.to_bits()))
        .collect();
    v.sort();
    v
}

#[test]
fn landscape_is_a_function_of_place() {
    // Arrive at (600, 0) straight along x, and by a wide loop from the other side: the same
    // trees and features stand there. Another seed, another forest.
    let fly = |seed: u64, path: &[(f64, f64)]| {
        let mut l = Land::new(seed, 0);
        l.stream(path[0].0, path[0].1, usize::MAX);
        for w in path.windows(2) {
            let n = ((w[1].0 - w[0].0).hypot(w[1].1 - w[0].1) / 0.4).ceil() as usize;
            for k in 1..=n {
                let t = k as f64 / n as f64;
                l.stream(w[0].0 + (w[1].0 - w[0].0) * t, w[0].1 + (w[1].1 - w[0].1) * t, 1);
            }
        }
        for _ in 0..400 {
            l.stream(path.last().unwrap().0, path.last().unwrap().1, 1);
        }
        l
    };
    let straight = fly(7, &[(0.0, 0.0), (600.0, 0.0)]);
    let looped = fly(7, &[(0.0, 0.0), (0.0, 500.0), (1100.0, 500.0), (1100.0, 0.0), (600.0, 0.0)]);
    let a = trees_near(&straight, 600.0, 0.0, 60.0);
    assert!(a.len() > 50, "{} trees", a.len());
    assert_eq!(a, trees_near(&looped, 600.0, 0.0, 60.0));
    let feats = |l: &Land| {
        let mut v: Vec<_> = l.features.iter().filter(|f| f.alive && (f.x - 600.0).hypot(f.y) < 200.0).map(|f| (f.kind as u8, (f.x * 100.0) as i64)).collect();
        v.sort();
        v
    };
    assert_eq!(feats(&straight), feats(&looped));
    assert_ne!(a, trees_near(&fly(8, &[(0.0, 0.0), (600.0, 0.0)]), 600.0, 0.0, 60.0));
    // Bounded: nothing far away survives.
    for t in straight.tiles.iter().filter(|t| t.alive) {
        let (cx, cy) = land::tile_centre(t.i, t.j);
        assert!((cx - 600.0).hypot(cy) <= land::KEEP + 40.0 + 1e-6);
    }
}

#[test]
fn landscapes_have_their_features() {
    // Over 5 km every biome brings trees, birds and water; the wild ones rock and falls.
    for (bi, b) in land::BIOMES.iter().enumerate() {
        let mut l = Land::new(42, bi as u32);
        let (mut rivers, mut lakes, mut ridges, mut sites, mut trees) = (0, 0, 0, 0, 0usize);
        let (mut surf, mut air, mut big) = (0, 0, 0);
        let mut seen_sites = std::collections::HashSet::new();
        let mut seen = std::collections::HashSet::new();
        let mut x = 0.0;
        while x < 5000.0 {
            l.stream(x, 0.0, usize::MAX);
            for f in l.features.iter().filter(|f| f.alive && f.y.abs() < 150.0) {
                if seen.insert(f.id) {
                    match f.kind {
                        land::FeatureKind::River => rivers += 1,
                        land::FeatureKind::Lake => lakes += 1,
                        land::FeatureKind::Ridge => ridges += 1,
                        _ => {}
                    }
                }
            }
            sites = sites.max(l.sites.iter().filter(|s| s.alive).count());
            for s in l.sites.iter().filter(|s| s.alive && seen_sites.insert(s.id)) {
                let name = crate::sources::Water::PRESETS.get(s.preset as usize).copied().unwrap_or("");
                match s.kind {
                    land::SiteKind::Air => air += 1,
                    land::SiteKind::Water if name.ends_with("surf") => surf += 1,
                    land::SiteKind::Water if name == "big_river" => big += 1,
                    _ => {}
                }
            }
            trees = trees.max(trees_near(&l, x, 0.0, 80.0).len());
            x += 50.0;
        }
        println!("{:>16}: {trees} trees, {rivers} rivers, {lakes} lakes, {ridges} ridges, {sites} sites, {surf} surf, {air} air, {big} big river", b.name);
        if b.forest > 0.05 {
            assert!(trees > 100, "{}: {trees} trees", b.name);
        }
        if b.river_every < 1000.0 {
            assert!(rivers >= 3, "{}: {rivers} rivers", b.name);
        }
        if b.relief.sea > 0.0 {
            assert!(surf > 3, "{}: {surf} surf sites", b.name);
        }
        if b.relief.big_river > 0.0 {
            assert!(big > 3, "{}: {big} big river sites", b.name);
        }
        if b.air == land::Air::Sand || b.air == land::Air::Ridge {
            assert!(air > 5, "{}: {air} air sites", b.name);
        }
        assert!(lakes + ridges >= 1, "{}: {lakes} lakes {ridges} ridges", b.name);
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
    assert_eq!(scene, 34);
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
        l.stream(0.0, 0.0, usize::MAX);
        let (mut worst, mut total, mut n) = (0.0f64, 0.0f64, 0);
        let mut x = 0.0;
        while x < 4000.0 {
            x += 0.0333; // 25 m/s, one 64-sample block
            let t = std::time::Instant::now();
            l.stream(x, 0.3 * x, 1);
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

/// Feeds the engine a head pose (as AirPods would, every block) for `seconds`.
fn look(e: &Engine, yaw_deg: f32, pitch_deg: f32, seconds: f32) {
    let q = crate::headpose::Quat::from_yaw_pitch_roll(yaw_deg.to_radians(), pitch_deg.to_radians(), 0.0);
    let (mut l, mut r) = (vec![0.0f32; 480], vec![0.0f32; 480]);
    for _ in 0..(seconds * 100.0) as usize {
        e.set_head_pose([q.w, q.x, q.y, q.z], [0.0; 3], 0.01);
        e.render(&mut l, &mut r);
    }
}

fn heading(e: &Engine) -> f32 {
    e.telemetry(telemetry::FLIGHT_HEADING)
}

fn turned(a: f32, b: f32) -> f32 {
    (b - a + core::f32::consts::PI).rem_euclid(core::f32::consts::TAU) - core::f32::consts::PI
}

#[test]
fn heading_follows_the_head() {
    let e = flying_engine(10.0, 1);
    e.set_control(id::FLIGHT_CURVINESS, 0.0);
    look(&e, 0.0, 0.0, 1.0);
    let h0 = heading(&e);
    // A glance inside the dead zone keeps the course.
    look(&e, 5.0, 0.0, 3.0);
    assert!(turned(h0, heading(&e)).abs() < 0.02, "drifted {}", turned(h0, heading(&e)));
    // A sustained look 40° left turns left, close to the full turn rate (30°/s).
    let h1 = heading(&e);
    look(&e, 40.0, 0.0, 4.0);
    let left = turned(h1, heading(&e));
    assert!(left > 1.0 && left < 2.1, "turned {left} rad in 4 s");
    let rate = e.telemetry(telemetry::FLIGHT_TURN);
    assert!(rate > 0.3, "turn rate {rate}");
    // Looking ahead again: the turn eases off within a second and the course holds.
    look(&e, 0.0, 0.0, 1.5);
    let h2 = heading(&e);
    look(&e, 0.0, 0.0, 2.0);
    assert!(turned(h2, heading(&e)).abs() < 0.02);
    // And to the right.
    look(&e, -50.0, 0.0, 3.0);
    assert!(turned(h2, heading(&e)) < -0.8);
    // A faster turn-rate setting turns faster.
    e.set_control(id::FLIGHT_TURN_RATE, 60.0);
    look(&e, 0.0, 0.0, 2.0);
    let h3 = heading(&e);
    look(&e, 60.0, 0.0, 3.0);
    assert!(turned(h3, heading(&e)) > 1.8);
}

#[test]
fn steering_off_keeps_the_course() {
    let e = flying_engine(10.0, 1);
    e.set_control(id::FLIGHT_CURVINESS, 0.0);
    e.set_control(id::FLIGHT_STEER, 0.0);
    look(&e, 0.0, 0.0, 1.0);
    let h0 = heading(&e);
    look(&e, 60.0, 20.0, 4.0);
    assert!(turned(h0, heading(&e)).abs() < 0.01);
    // Pitch climbs only with yaw_pitch.
    let alt = e.telemetry(telemetry::FLIGHT_ALTITUDE);
    e.set_control(id::FLIGHT_STEER, 2.0);
    look(&e, 0.0, 30.0, 4.0);
    assert!(e.telemetry(telemetry::FLIGHT_ALTITUDE) > alt + 5.0, "no climb");
}

#[test]
fn streaming_survives_turning_around() {
    let e = flying_engine(20.0, 4);
    let mut objs = [ObjectInfo::default(); 512];
    look(&e, 0.0, 0.0, 3.0);
    // Look back over the shoulder until the course has turned around.
    let h0 = heading(&e);
    let mut k = 0;
    while turned(h0, heading(&e)).abs() < 3.0 && k < 40 {
        look(&e, 70.0, 0.0, 0.5);
        k += 1;
    }
    assert!(k < 40, "never turned around");
    look(&e, 0.0, 0.0, 8.0);
    let n = e.objects(&mut objs);
    assert!(n > 50 && n <= bridge::OBJECTS, "{n} objects after turning around");
    assert!(objs[..n].iter().any(|o| o.kind == ObjectKind::Tree as u32 && o.x > 12.0), "nothing ahead");
    let mut src = [crate::SourceInfo::default(); 128];
    assert!(e.sources(&mut src) <= EMITTERS);
}

/// Finds seeds whose start shows the biome's water (snapshots for docs/world):
/// `BIOME=sea_coast cargo test --release find_scenic_seeds -- --ignored --nocapture`
#[test]
#[ignore]
fn find_scenic_seeds() {
    let biome = std::env::var("BIOME").unwrap_or_else(|_| "sea_coast".into());
    for s in 0..30u32 {
        let mut desc = SceneDesc::default_scene();
        desc.flight = Some(FlightDesc { start: true, speed: Some(6.0), seed: Some(s), biome: Some(biome.clone()), stops: Some(0.0), ..Default::default() });
        let e = Engine::with_scene(48_000.0, &desc).unwrap();
        e.set_param(param::PLAYING, 1.0);
        run(&e, 1.5);
        let f = e.flight_frame().unwrap();
        let mut sampler = terrain::Sampler::new(f.seed, f.biome, f.relief);
        let mut out = vec![0.0f32; terrain::SAMPLE_FLOATS * 41 * 41];
        sampler.grid(f.x - 50.0, f.y - 50.0, 2.5, 41, 41, &mut out);
        let (mut wet, mut lo, mut hi) = (0, f32::MAX, f32::MIN);
        for k in 0..41 * 41 {
            let o = &out[k * terrain::SAMPLE_FLOATS..];
            if o[1] > o[0] {
                wet += 1;
            }
            lo = lo.min(o[0]);
            hi = hi.max(o[0]);
        }
        println!("seed {s:>2}: wet {:>4}/1681  relief {:.1} m  ground {:.1}", wet, hi - lo, f.ground_z);
    }
}

/// A land streamed around the origin (all tiles made).
fn land_at(biome: &str, seed: u64, relief: f32) -> Land {
    let mut l = Land::with_relief(seed, land::biome_index(biome).unwrap() as u32, relief);
    l.stream(0.0, 0.0, usize::MAX);
    l
}

#[test]
fn hills_hide_and_muffle_sources() {
    // Somewhere in the mountains a ridge stands between the listener and a source 150 m
    // away: it takes the highs (knife edge over the crest) more than the lows.
    let l = land_at("snowy_mountains", 3, 1.0);
    let freqs = crate::shaping::grid_freqs(48_000.0);
    let nf = crate::shaping::NF;
    let (mut found, mut muffled) = (0, 0);
    for k in 0..72 {
        let a = k as f64 * core::f64::consts::TAU / 72.0;
        let (sx, sy) = (150.0 * a.cos(), 150.0 * a.sin());
        let lis = [0.0, 0.0, l.ground(0.0, 0.0) as f64 + 1.7];
        let src = [sx, sy, l.ground(sx, sy) as f64 + 0.5];
        let p = acoustic::terrain_path(&l, src, lis);
        let mut db = [0.0f32; crate::shaping::NF];
        p.loss_db(&freqs, 343.0, &mut db);
        let (lo, hi) = (db[nf / 4], db[nf - 4]);
        assert!(hi >= -25.0 - 1e-3 && lo >= -25.0 - 1e-3, "capped at 25 dB: {lo} {hi}");
        if p.weight > 0.5 && hi < -12.0 {
            found += 1;
            if hi < lo - 3.0 {
                muffled += 1;
            }
        }
    }
    assert!(found > 3 && muffled > 0, "{found} hidden paths in the mountains, {muffled} muffled");
    // On flat ground nothing hides a source at ear height.
    let m = land_at("meadows", 3, 0.0);
    let p = acoustic::terrain_path(&m, [120.0, 10.0, m.ground(120.0, 10.0) as f64 + 1.5], [0.0, 0.0, m.ground(0.0, 0.0) as f64 + 1.7]);
    let mut db = [0.0f32; crate::shaping::NF];
    p.loss_db(&freqs, 343.0, &mut db);
    assert!(db.iter().all(|v| *v > -1.0), "flat ground hides: {:?}", &db[30..]);
}

#[test]
fn valleys_answer_and_plains_are_open() {
    let enclosure = |biome: &str, seed: u64| {
        let l = land_at(biome, seed, 1.0);
        let mut e = acoustic::Echoes::new(48_000.0);
        let (mut enc, mut echoes) = (0.0f32, 0);
        for k in 0..9 {
            let (x, y) = (((k % 3) as f64 - 1.0) * 60.0, ((k / 3) as f64 - 1.0) * 60.0);
            e.scan(&l, x, y, l.ground(x, y) + 1.7, 0.0, 1.0);
            enc += e.enclosure / 9.0;
            echoes += e.found.iter().filter(|r| r.gain > 0.0).count();
        }
        (enc, echoes)
    };
    let (mountains, echoes) = enclosure("snowy_mountains", 5);
    let (meadows, _) = enclosure("meadows", 5);
    eprintln!("enclosure: mountains {mountains:.3} ({echoes} echoes), meadows {meadows:.3}");
    assert!(mountains > 2.0 * meadows + 0.02, "mountains {mountains} meadows {meadows}");
    assert!(echoes > 3, "{echoes} echoes in the mountains");
}

#[test]
fn flight_stops_at_places_and_flies_on() {
    let mut desc = SceneDesc::default_scene();
    desc.flight = Some(FlightDesc { start: true, speed: Some(10.0), seed: Some(29), biome: Some("lake_shore".into()), stops: Some(1.0), ..Default::default() });
    let e = Engine::with_scene(48_000.0, &desc).unwrap();
    e.set_param(param::PLAYING, 1.0);
    let (mut stopped, mut resumed) = (false, false);
    for _ in 0..150 {
        run(&e, 1.0);
        let f = e.flight_frame().unwrap();
        let v = e.telemetry(telemetry::FLIGHT_SPEED);
        if f.linger > 0.9 && v < 0.2 {
            stopped = true;
        }
        if stopped && v > 5.0 {
            resumed = true;
            break;
        }
    }
    assert!(stopped && resumed, "stopped {stopped} resumed {resumed}");
}

#[test]
fn frame_tells_the_ui_where_and_what() {
    let mut desc = SceneDesc::default_scene();
    desc.flight = Some(FlightDesc { start: true, speed: Some(8.0), biome: Some("desert".into()), stops: Some(0.0), ..Default::default() });
    let e = Engine::with_scene(48_000.0, &desc).unwrap();
    e.set_param(param::PLAYING, 1.0);
    run(&e, 3.0);
    let f = e.flight_frame().unwrap();
    assert_eq!(f.biome as usize, land::biome_index("desert").unwrap());
    assert!(f.sand_cover > 0.5 && f.snow_cover == 0.0);
    assert!(f.ear_z > f.ground_z && (f.x * f.x + f.y * f.y).sqrt() > 10.0);
    // The UI's sampler sees the same ground the engine flies over.
    let mut s = terrain::Sampler::new(f.seed, f.biome, f.relief);
    let mut out = [0.0f32; terrain::SAMPLE_FLOATS];
    s.grid(f.x, f.y, 1.0, 1, 1, &mut out);
    assert!((out[0].max(out[1]) - f.ground_z).abs() < 0.6, "sampler {} engine {}", out[0], f.ground_z);
}
