//! Splits the neural voice cost into network (per 5 ms frame) and synthesis (per sample).
//!
//!   cargo run --release --example neural_bench -- [species]

use std::time::Instant;

use asmr_core::neural::{self, voice::Frame};

fn main() {
    let species = std::env::args().nth(1).unwrap_or_else(|| "chaffinch".into());
    let m = neural::builtin(&species).expect("model");
    let sr = 48_000.0;
    let mut st = m.voice.new_state(sr, 1);
    let f = |i: usize| Frame { f0: 3000.0 + 500.0 * (i as f32 * 0.1).sin(), loud: -20.0, effort: 0.3 };

    let n = 200_000;
    let t = Instant::now();
    let mut acc = 0.0;
    for i in 0..n {
        acc += m.voice.frame_outputs(&mut st, f(i)).0[0];
    }
    let net = t.elapsed().as_secs_f64() / n as f64;

    let mut out = vec![0.0f32; 48_000];
    m.voice.start(&mut st, f(0));
    let mut i = 0;
    let reps = 20;
    let t = Instant::now();
    for _ in 0..reps {
        m.voice.render(&mut st, &mut out, || {
            i += 1;
            f(i)
        });
    }
    let total = t.elapsed().as_secs_f64() / reps as f64; // seconds of CPU per second of audio
    let frames_per_s = 1.0 / m.voice.hop_s as f64;
    println!("{species} ({} params), sink {acc:.1}", m.params());
    println!("  network:   {:.2} us/frame -> {:.3} % of a core", net * 1e6, net * frames_per_s * 100.0);
    println!("  voice total (network + synthesis): {:.3} % of a core while singing", total * 100.0);
    println!("  synthesis: {:.3} % of a core", (total - net * frames_per_s) * 100.0);
}
