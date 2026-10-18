//! The render path of flight mode must not allocate: a counting global allocator watches the
//! thread that calls `Engine::render` while the listener flies through streamed landscape
//! (tiles generated and retired while steering by head through two U-turns, slots changing
//! hands, birds flushed, a reseed).

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};

use asmr_core::controls::id;
use asmr_core::flight::FlightDesc;
use asmr_core::{param, Engine, SceneDesc};

struct Counting;

static ALLOCS: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    static WATCH: Cell<bool> = const { Cell::new(false) };
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if WATCH.with(|w| w.get()) {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        System.alloc(layout)
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if WATCH.with(|w| w.get()) {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        System.dealloc(ptr, layout)
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new: usize) -> *mut u8 {
        if WATCH.with(|w| w.get()) {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        System.realloc(ptr, layout, new)
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

#[test]
fn flight_render_does_not_allocate() {
    let mut desc = SceneDesc::default_scene();
    desc.flight = Some(FlightDesc { start: true, speed: Some(20.0), biome: Some("highlands".into()), ..Default::default() });
    let engine = Engine::with_scene(48_000.0, &desc).unwrap();
    engine.set_param(param::PLAYING, 1.0);
    engine.set_param(param::BIRDS, 1.0);
    engine.set_param(param::RAIN, 4.0);
    let (mut l, mut r) = (vec![0.0f32; 512], vec![0.0f32; 512]);
    // Warm-up: the flight pack is handed over on the first block.
    for _ in 0..20 {
        engine.render(&mut l, &mut r);
    }
    let blocks = 48_000 * 40 / 512;
    WATCH.with(|w| w.set(true));
    for i in 0..blocks {
        // AirPods: look left for a while, then right (the course turns around and back).
        let yaw: f32 = if i < blocks / 3 { 1.2 } else if i < 2 * blocks / 3 { -1.2 } else { 0.0 };
        engine.set_head_pose([(yaw / 2.0).cos(), 0.0, 0.0, (yaw / 2.0).sin()], [0.0; 3], 0.01);
        if i == blocks / 2 {
            // Control changes are lock-free stores; a reseed rebuilds the landscape in place.
            engine.set_control(id::FLIGHT_SEED, 9.0);
            engine.set_control(id::FLIGHT_SPEED, 25.0);
        }
        engine.render(&mut l, &mut r);
    }
    WATCH.with(|w| w.set(false));
    let n = ALLOCS.load(Ordering::Relaxed);
    assert_eq!(n, 0, "{n} allocations in 40 s of flight rendering");
    assert!(l.iter().all(|v| v.is_finite()));
}
