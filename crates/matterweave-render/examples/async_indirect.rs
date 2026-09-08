//! Headless native/Android validation of bounded background lighting.
use matterweave_core::World;
use matterweave_render::{
    async_indirect::{AsyncIndirectConfig, AsyncIndirectLight},
    indirect::{IndirectVolume, UpdateBudget},
    Sun,
};
use std::time::{Duration, Instant};

fn main() {
    let mut world = World::new(9);
    for x in -3_i32..=3 {
        for z in -3_i32..=3 {
            world.set([x, -1, z], 1);
            for y in 0..3 {
                if x.abs() == 3 || z.abs() == 3 {
                    world.set([x, y, z], 2);
                }
            }
        }
    }
    let mut palette = [[0.65; 3]; 256];
    palette[1] = [0.9, 0.05, 0.02];
    let sun = Sun {
        direction_to_sun: [0., 1., 0.],
        intensity: 1.,
    };
    let config = AsyncIndirectConfig::new([-3, -1, -3], [7, 5, 7], 64, 32., palette).unwrap();
    let mut worker = AsyncIndirectLight::new(config);
    assert!(worker.available());
    for closed in [false, true, false] {
        for x in -3..=3 {
            for z in -3..=3 {
                world.set([x, 3, z], if closed { 2 } else { 0 });
            }
        }
        assert!(worker.request(&world, 1, sun).unwrap());
        let deadline = Instant::now() + Duration::from_secs(10);
        let result = loop {
            if let Some(result) = worker.poll(&world, 1, sun) {
                break result.unwrap();
            }
            assert!(worker.available() && Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        };
        assert!(result.valid_for(&world, 1, sun));
        let sample = result.sample([-3, 1, 0], 0);
        if closed {
            assert_eq!(sample, [0.; 3]);
        } else {
            assert!(sample[0] > 0.02 && sample[0] > sample[1]);
        }
        let mut reference = IndirectVolume::new([-3, -1, -3], [7, 5, 7], 64, 32., palette).unwrap();
        // Deliberately different slicing from the worker's4096 budget.
        loop {
            let update = reference
                .update(&world, 1, sun, UpdateBudget { rays: 61, work: 37 })
                .unwrap();
            if update.complete {
                break;
            }
            assert!(Instant::now() < deadline);
        }
        for x in -3..=3 {
            for y in -1..=3 {
                for z in -3..=3 {
                    for face in 0..6 {
                        assert_eq!(
                            result.sample([x, y, z], face),
                            reference.sample([x, y, z], face)
                        );
                    }
                }
            }
        }
        println!(
            "closed={closed} sample={sample:?} completed={}",
            worker.stats().completed
        );
    }
    println!(
        "PASS async indirect native: open/closed/reopened radiance matches synchronous reference"
    );
}
