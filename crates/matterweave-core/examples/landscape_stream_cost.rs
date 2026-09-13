//! Cost of publishing a landscape streaming window, host-side.
//!
//! Two cases matter on the frame path: the first publish (a whole 7x7 window of
//! the source's vertical band) and the incremental publish after crossing one
//! chunk boundary (91 chunks). The asynchronous preparation path exists to keep
//! both off the frame, but the synchronous fallback and the load path still run
//! them, so their cost needs a number rather than an estimate.
//!
//! Usage: `landscape_stream_cost [seed] [repeats]`

use matterweave_core::World;
use std::time::Instant;

fn median(mut samples: Vec<f64>) -> f64 {
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    samples[samples.len() / 2]
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let seed: u64 = args.get(1).and_then(|v| v.parse().ok()).unwrap_or(20260913);
    let repeats: usize = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(9);

    let mut fresh = Vec::new();
    let mut step = Vec::new();
    for index in 0..repeats {
        let eye = [40.0 + (index as f32) * 512.0, 40.0, -80.0];
        let start = Instant::now();
        let mut world = World::landscape(seed);
        world.stream_around(eye);
        fresh.push(start.elapsed().as_secs_f64() * 1000.0);

        // One 16 m step inside the same window: the incremental case.
        let start = Instant::now();
        world.stream_around([eye[0] + 16.0, eye[1], eye[2]]);
        step.push(start.elapsed().as_secs_f64() * 1000.0);

        let chunks = world.stream_resident_chunks().unwrap().len();
        if index == 0 {
            println!(
                "window: {chunks} chunks, {} solid voxels, {} KiB payload, source {:?}",
                world.stats().solid_voxels,
                world.stats().allocated_bytes / 1024,
                world.terrain_source()
            );
        }
    }
    println!(
        "fresh window: median {:.2} ms (min {:.2}, max {:.2}) over {repeats} calls",
        median(fresh.clone()),
        fresh.iter().cloned().fold(f64::INFINITY, f64::min),
        fresh.iter().cloned().fold(0.0, f64::max)
    );
    println!(
        "16 m step:    median {:.2} ms (min {:.2}, max {:.2}) over {repeats} calls",
        median(step.clone()),
        step.iter().cloned().fold(f64::INFINITY, f64::min),
        step.iter().cloned().fold(0.0, f64::max)
    );
    println!("host numbers, release/debug profile as built; not device evidence");
}
