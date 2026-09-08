//! Minimal host loop showing request/poll/publish for asynchronous detail
//! collision. No demo content, rendering or UI: it drives the engine APIs the
//! way a simulation owner would and prints the resulting bounded diagnostics.
//!
//! Run with the crate's exclusive target set, e.g.
//! `cargo run -p matterweave-physics --example async_detail_collision`.

use matterweave_detail::{material, DetailScene, DetailVolume, Scale, Transform};
use matterweave_physics::{AsyncDetailCollision, Physics, FIXED_DT};
use std::time::{Duration, Instant};

/// One 8 m x 0.25 m x 8 m stone slab prototype placed at the origin.
fn scene_with_floor() -> DetailScene {
    let mut volume = DetailVolume::new("floor", Scale::new(0.25).expect("scale"));
    for x in 0..32 {
        for z in 0..32 {
            volume.set([x, 0, z], material::BANK_STONE).expect("cell");
        }
    }
    let mut scene = DetailScene::new();
    scene.add_prototype(volume).expect("prototype");
    scene
        .place("floor.0", "floor", Transform::identity())
        .expect("placement");
    scene
}

fn main() {
    // Authoritative, single-threaded state owned by the simulation.
    let mut world = matterweave_core::World::new(1);
    let mut physics = Physics::new(&world);
    physics.sync_world(&world);
    let _ = &mut world;

    let scene = scene_with_floor();
    let mut collision = AsyncDetailCollision::new();
    if !collision.available() {
        eprintln!("worker unavailable; a host would fall back to replace_detail_scene");
        return;
    }

    // A scene change enqueues one bounded off-thread preparation.
    let queued = collision.request(&scene);
    println!("requested off-thread preparation: queued={queued}");

    // Simulation loop: keep stepping physics while polling for a current result.
    // Old live collision (here: none yet) stays intact until publication.
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut published = false;
    while Instant::now() < deadline {
        // The controller never blocks the frame; poll is nonblocking.
        if let Some(outcome) = collision.poll(&scene) {
            match outcome {
                Ok(prepared) => {
                    let built = prepared.stats();
                    match physics.publish_detail_scene(&scene, prepared) {
                        Ok(stats) => {
                            println!(
                                "published: static_colliders={} merged_boxes={} woken_bodies={}",
                                stats.static_colliders, stats.merged_boxes, stats.woken_bodies
                            );
                            published = true;
                        }
                        Err(error) => {
                            // Source changed between poll and publish; retain live walls.
                            println!("publication rejected (source changed): {error}");
                        }
                    }
                    let _ = built;
                }
                Err(error) => println!("preparation failed for the current scene: {error}"),
            }
            break;
        }
        // Advance dynamic simulation each frame regardless of preparation state.
        physics.step(FIXED_DT, [0.0, 0.0, 0.0], false);
        std::thread::sleep(Duration::from_millis(1));
    }

    let stats = collision.stats();
    println!(
        "controller stats: queued={} inflight={} results={} discarded={} generation={}",
        stats.queued, stats.inflight, stats.results, stats.discarded, stats.generation
    );

    if published {
        // A settled character rests on the published floor.
        assert!(physics.teleport([2.0, 2.0, 2.0]));
        for _ in 0..120 {
            physics.step(FIXED_DT, [0.0, 0.0, 0.0], false);
        }
        println!(
            "character grounded on published detail floor: {}",
            physics.grounded()
        );
    } else {
        println!("no current preparation was published before the deadline");
    }
}
