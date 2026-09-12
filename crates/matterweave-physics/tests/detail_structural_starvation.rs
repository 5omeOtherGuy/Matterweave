//! Structural-mode publication starvation: can the gate withhold a completed
//! preparation forever because a dynamic body permanently overlaps a prepared
//! collider, or does it only defer while a body is genuinely in the way of new
//! solid material?
//!
//! Publication states, using the cadence's own vocabulary:
//!
//! - **queued**: `on_edit` stored an immutable source snapshot and the worker
//!   has not picked it up yet. Never withheld by the gate.
//! - **inflight**: the worker is building `PreparedDetailCollision` for it.
//!   Never withheld by the gate.
//! - **completed-unpublished**: preparation finished and the result is
//!   buffered (worker path) or staged (workerless path), but the publication
//!   gate refuses to commit it.
//! - **published**: `Physics::publish_detail_scene` committed the result; live
//!   collision and the cadence's pending gate state update together.
//!
//! The only transition the gate withholds is **completed-unpublished ->
//! published**:
//!
//! - Region mode (`on_edit(.., Some(added))`) withholds it while a dynamic
//!   body AABB overlaps an accumulated *added-solid* region. A body resting on
//!   unchanged geometry overlaps no such region, so such an edit publishes.
//! - Structural mode (`on_edit(.., None)`) withholds it while a dynamic body
//!   AABB overlaps a prepared collider that is not already live at the same
//!   pose with the same shape, i.e. while a body overlaps *new* solid
//!   material. A body resting on an unchanged collider cannot block: no body
//!   has to move out of the way of material that is already live.
//!
//! These tests discriminate a stall from correct deferral without sleeps or
//! wall-clock signals:
//!
//! - completion is established structurally: the workerless path builds inside
//!   `on_edit` (so `Ok(true)` is completed-and-withheld and `Ok(false)` is
//!   already published), and the worker path waits on the controller's own
//!   `stats().results` counter, which is set only after preparation finished;
//! - the withholding loop runs a fixed number of *simulated* frames (not
//!   wall-clock), stepping the body each frame, and asserts on every frame
//!   that the body's overlap is only with live, unchanged collision while the
//!   added material is clear of every body;
//! - the control case in the sibling test ends the blocking condition for real
//!   by moving the body off the new material, and the same retained result
//!   then publishes.

use matterweave_detail::{material, DetailScene, DetailVolume, Scale, Transform, Yaw};
use matterweave_physics::{
    BodySnapshot, DetailCollisionCadence, Physics, PhysicsSnapshot, FIXED_DT,
};
use rapier3d::geometry::BoundingVolume;
use rapier3d::prelude::Aabb;
use std::time::{Duration, Instant};

const TILE: f32 = 0.25;
/// Hang guard for the worker-completion wait only; never a publication signal.
const WORKER_DEADLINE: Duration = Duration::from_secs(30);
/// Simulated frames a completed result may stay unpublished before the test
/// calls it a stall. Frames, not wall-clock: independent of worker/device
/// speed, unlike a sleep or deadline-based publication signal.
const STALL_FRAMES: usize = 256;
/// Prepared floor slab: one 8 m x 8 m x 0.25 m collider.
const FLOOR_BOX: ([f32; 3], [f32; 3]) = ([0.0, 0.0, 0.0], [8.0, 0.25, 8.0]);
/// Wall added far from the resting crate: clear of every dynamic body AABB.
const FAR_WALL_BOX: ([f32; 3], [f32; 3]) = ([6.0, 0.25, 6.0], [6.25, 0.5, 6.25]);
/// Wall added inside the resting crate: publishing it would place solid
/// material inside a dynamic body.
const CRATE_WALL_BOX: ([f32; 3], [f32; 3]) = ([3.0, 0.25, 4.0], [3.25, 0.5, 4.25]);
/// The 0.5 m crate is restored here and settles onto the floor.
const CRATE_START: [f32; 3] = [3.0, 0.60, 4.0];
/// The same crate parked in clear air, off every prepared collider AABB.
const CRATE_CLEAR: [f32; 3] = [3.0, 5.0, 4.0];
/// Sparse two-cell frame instance on the floor: outer AABB x 0..3.25 m.
const FRAME_ARM_BOX: ([f32; 3], [f32; 3]) = ([0.0, 0.25, 0.3], [0.25, 0.5, 0.55]);
/// The filled replacement's extra cell, inside the resting crate.
const FILL_CELL_BOX: ([f32; 3], [f32; 3]) = ([1.5, 0.25, 0.3], [1.75, 0.5, 0.55]);

fn volume(id: &str, extent: [i32; 3], material: u8) -> DetailVolume {
    let mut volume = DetailVolume::new(id, Scale::new(TILE).expect("valid scale"));
    for x in 0..extent[0] {
        for y in 0..extent[1] {
            for z in 0..extent[2] {
                volume.set([x, y, z], material).expect("in-budget cell");
            }
        }
    }
    volume
}

fn empty_physics() -> Physics {
    Physics::new(&matterweave_core::World::new(1))
}

fn scene_with_floor() -> DetailScene {
    let mut scene = DetailScene::new();
    scene
        .add_prototype(volume("floor", [32, 1, 32], material::BANK_STONE))
        .expect("prototype");
    scene
        .place("floor.0", "floor", Transform::identity())
        .expect("placement");
    scene
}

/// Adds one single-cell wall instance at `translation`.
fn add_wall(scene: &mut DetailScene, translation: [f32; 3]) {
    scene
        .add_prototype(volume("wall", [1, 1, 1], material::BANK_STONE))
        .expect("prototype");
    scene
        .place(
            "wall.0",
            "wall",
            Transform::new(translation, Yaw::Deg0).expect("transform"),
        )
        .expect("placement");
}

/// One 0.5 m dynamic cube at `position`, with the character parked high above
/// the scene so only the crate can overlap prepared detail colliders.
fn crate_snapshot(position: [f32; 3]) -> PhysicsSnapshot {
    PhysicsSnapshot {
        version: 1,
        eye: [1.0, 20.0, 4.125],
        bodies: vec![BodySnapshot {
            position,
            rotation: [0.0, 0.0, 0.0, 1.0],
            velocity: [0.0; 3],
            angular_velocity: [0.0; 3],
            dimensions: [1, 1, 1],
            material: 8,
        }],
    }
}

/// Whether any dynamic body AABB intersects `region`, using parry's own
/// inclusive `Aabb::intersects` — the publication gate's semantics.
fn any_body_overlaps(physics: &Physics, region: ([f32; 3], [f32; 3])) -> bool {
    let region = Aabb::new(region.0.into(), region.1.into());
    physics
        .dynamic_body_aabbs()
        .iter()
        .any(|body| body.intersects(&region))
}

fn body_boxes(physics: &Physics) -> String {
    physics
        .dynamic_body_aabbs()
        .iter()
        .map(|aabb| format!("[{:?}..{:?}]", aabb.mins, aabb.maxs))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Advances only the dynamic voxel bodies; the character stays parked.
fn settle(physics: &mut Physics, steps: usize) {
    for _ in 0..steps {
        physics.step_objects(FIXED_DT);
    }
}

/// Waits until the worker has finished preparing the current source, using the
/// controller's own counters. `Instant` is a hang guard only: the loop exits on
/// state, never on elapsed time.
fn wait_completed(cadence: &DetailCollisionCadence) {
    let started = Instant::now();
    loop {
        let stats = cadence.stats();
        if stats.results == 1 && stats.queued == 0 && stats.inflight == 0 {
            return;
        }
        assert!(
            started.elapsed() < WORKER_DEADLINE,
            "worker never completed preparation: {stats:?}"
        );
        std::thread::yield_now();
    }
}

fn cadence(worker: bool) -> DetailCollisionCadence {
    let cadence = if worker {
        DetailCollisionCadence::new()
    } else {
        DetailCollisionCadence::without_worker()
    };
    assert_eq!(cadence.available(), worker, "publication path under test");
    cadence
}

/// REPRODUCTION / REGRESSION: a structural edit adds material clear of every
/// body while a dynamic body rests permanently on an *unchanged* live collider.
/// The completed preparation must publish without waiting for the body to move
/// off material that is already live.
fn structural_edit_clear_of_a_resting_body(worker: bool) {
    let mut scene = scene_with_floor();
    let mut physics = empty_physics();
    let load = physics.replace_detail_scene(&scene).expect("floor load");
    assert_eq!(load.static_colliders, 1, "one prepared floor collider");

    // A grazing contact keeps the crate's AABB a fraction of a millimetre
    // inside the floor collider AABB, exactly as the gate compares them.
    physics
        .restore(&crate_snapshot(CRATE_START))
        .expect("valid crate snapshot");
    settle(&mut physics, 600);
    assert_eq!(physics.body_count(), 1, "the crate is the only voxel body");
    assert!(
        any_body_overlaps(&physics, FLOOR_BOX),
        "the resting crate AABB overlaps the unchanged floor collider AABB: {}",
        body_boxes(&physics)
    );

    add_wall(&mut scene, [6.0, TILE, 6.0]);
    assert!(
        !any_body_overlaps(&physics, FAR_WALL_BOX),
        "the added wall is clear of every dynamic body: {}",
        body_boxes(&physics)
    );

    let mut cadence = cadence(worker);
    let pending = cadence
        .on_edit(&scene, &mut physics, None)
        .expect("valid scene");
    if !pending {
        // Workerless path with a clear gate: publication already happened in
        // `on_edit` (documented `Ok(false)`), so there is no deferral at all.
        assert!(!worker, "the worker path always queues");
        assert_eq!(
            physics.detail_collision_stats().static_colliders,
            load.static_colliders + 1,
            "the clear structural edit published synchronously"
        );
        return;
    }
    assert_eq!(
        physics.detail_collision_stats().static_colliders,
        load.static_colliders,
        "queueing or staging alone must never touch live collision"
    );
    if worker {
        wait_completed(&cadence);
    }

    // completed-unpublished. The only body/prepared overlap is the crate on the
    // unchanged live floor; the added wall is clear of every body. Publication
    // must therefore happen immediately rather than waiting for a body to
    // leave material that is already live.
    let mut published = None;
    for frame in 0..STALL_FRAMES {
        match cadence.step(&scene, &mut physics).expect("valid scene") {
            Some(stats) => {
                published = Some((frame, stats));
                break;
            }
            None => {
                assert!(
                    any_body_overlaps(&physics, FLOOR_BOX),
                    "frame {frame}: the crate must still rest on the floor: {}",
                    body_boxes(&physics)
                );
                assert!(
                    !any_body_overlaps(&physics, FAR_WALL_BOX),
                    "frame {frame}: no body is near the added wall: {}",
                    body_boxes(&physics)
                );
                assert_eq!(
                    physics.detail_collision_stats().static_colliders,
                    load.static_colliders,
                    "frame {frame}: a withheld result must stay out of live collision"
                );
                if worker {
                    assert_eq!(
                        cadence.stats().results,
                        1,
                        "frame {frame}: the completed result is retained, not dropped"
                    );
                }
            }
        }
        settle(&mut physics, 1);
    }
    let Some((frame, stats)) = published else {
        panic!(
            "structural-mode result starved for {STALL_FRAMES} simulated frames: the completed \
             preparation was retained (stats {:?}) while the only prepared collider any body \
             overlapped was the unchanged live floor, and the added wall is clear of every \
             body; publication must not wait for a body to leave material that is already live",
            cadence.stats()
        );
    };
    assert_eq!(
        frame, 0,
        "the gate is clear immediately: no body overlaps any newly added material"
    );
    assert_eq!(stats.static_colliders, load.static_colliders + 1);
    assert_eq!(
        physics.detail_collision_stats().static_colliders,
        load.static_colliders + 1,
        "the added wall is live"
    );
    if worker {
        assert_eq!(
            cadence.stats().results,
            0,
            "the buffered result was consumed"
        );
    }
}

/// CONTROL: the same structural edit, but the added wall occupies the resting
/// crate. Publication would place solid material inside a dynamic body, so the
/// gate must defer while the crate stays there and publish the instant it
/// genuinely leaves.
fn structural_edit_inside_a_resting_body(worker: bool) {
    let mut scene = scene_with_floor();
    let mut physics = empty_physics();
    let load = physics.replace_detail_scene(&scene).expect("floor load");
    assert_eq!(load.static_colliders, 1);

    physics
        .restore(&crate_snapshot(CRATE_START))
        .expect("valid crate snapshot");
    settle(&mut physics, 600);
    assert!(any_body_overlaps(&physics, FLOOR_BOX));

    add_wall(&mut scene, [3.0, TILE, 4.0]);
    assert!(
        any_body_overlaps(&physics, CRATE_WALL_BOX),
        "the added wall region is inside the resting crate: {}",
        body_boxes(&physics)
    );

    let mut cadence = cadence(worker);
    assert!(
        cadence
            .on_edit(&scene, &mut physics, None)
            .expect("valid scene"),
        "a structural edit whose material is inside a body stays pending"
    );
    if worker {
        wait_completed(&cadence);
    }

    // Withheld through simulated frames: no publication may place the wall
    // through the resting crate, and the completed result stays retained.
    for frame in 0..64 {
        assert!(
            cadence
                .step(&scene, &mut physics)
                .expect("valid scene")
                .is_none(),
            "frame {frame}: a new wall must not publish inside a resting body"
        );
        assert_eq!(
            physics.detail_collision_stats().static_colliders,
            load.static_colliders,
            "frame {frame}: live collision stays untouched while withheld"
        );
        if worker {
            assert_eq!(
                cadence.stats().results,
                1,
                "frame {frame}: the completed result is retained, not dropped"
            );
        }
        settle(&mut physics, 1);
    }
    assert!(
        physics.detail_collision_stats().source_collision_cells == load.source_collision_cells,
        "the withheld wall never reached live collision"
    );

    // Control: end the blocking condition for real. Clear air is off every
    // prepared collider AABB, so the same retained result must publish.
    physics
        .restore(&crate_snapshot(CRATE_CLEAR))
        .expect("valid crate snapshot");
    assert!(!any_body_overlaps(&physics, CRATE_WALL_BOX));
    assert!(!any_body_overlaps(&physics, FLOOR_BOX));
    let mut published = None;
    for frame in 0..STALL_FRAMES {
        if let Some(stats) = cadence.step(&scene, &mut physics).expect("valid scene") {
            published = Some((frame, stats));
            break;
        }
        settle(&mut physics, 1);
    }
    let Some((frame, stats)) = published else {
        panic!(
            "a result retained across a genuine body-in-the-way overlap never published after \
             the body cleared: stats {:?}",
            cadence.stats()
        );
    };
    assert_eq!(
        frame, 0,
        "publication follows the instant the body genuinely clears the new material"
    );
    assert_eq!(stats.static_colliders, load.static_colliders + 1);
    assert_eq!(
        physics.detail_collision_stats().static_colliders,
        load.static_colliders + 1,
        "the added wall is live"
    );
    if worker {
        assert_eq!(
            cadence.stats().results,
            0,
            "the buffered result was consumed"
        );
    }
}

/// INVARIANT GUARD for the live-vs-prepared diff: a structural *added*
/// instance whose outer AABB is byte-identical to a live sparse instance but
/// which fills a hole inside the resting crate. An ignored-AABB diff would
/// mistake the replacement for the unchanged frame and publish new solid
/// material through the body; pose+shape comparison must keep it gated until
/// the body clears, then publish it.
fn structural_same_aabb_replacement_inside_a_resting_body(worker: bool) {
    let mut scene = scene_with_floor();
    let mut frame = DetailVolume::new("frame", Scale::new(TILE).expect("valid scale"));
    frame.set([0, 0, 0], material::BANK_STONE).expect("cell");
    frame.set([12, 0, 0], material::BANK_STONE).expect("cell");
    scene.add_prototype(frame).expect("prototype");
    let frame_transform = Transform::new([0.0, TILE, 0.3], Yaw::Deg0).expect("transform");
    scene
        .place("frame.0", "frame", frame_transform)
        .expect("placement");

    let mut physics = empty_physics();
    let load = physics.replace_detail_scene(&scene).expect("load");
    assert_eq!(load.static_colliders, 2, "floor plus sparse frame");

    physics
        .restore(&crate_snapshot([1.65, 0.60, 0.30]))
        .expect("valid crate snapshot");
    settle(&mut physics, 600);
    assert!(
        any_body_overlaps(&physics, FILL_CELL_BOX),
        "the resting crate occupies the cell the replacement fills: {}",
        body_boxes(&physics)
    );
    assert!(
        !any_body_overlaps(&physics, FRAME_ARM_BOX),
        "the crate is in the frame gap, clear of the live arms: {}",
        body_boxes(&physics)
    );

    // The filled prototype has the same outer AABB as the live frame but adds
    // a middle cell inside the crate.
    let mut filled = DetailVolume::new("filled", Scale::new(TILE).expect("valid scale"));
    for x in [0, 6, 12] {
        filled.set([x, 0, 0], material::BANK_STONE).expect("cell");
    }
    scene.add_prototype(filled).expect("prototype");
    scene
        .place("filled.0", "filled", frame_transform)
        .expect("placement");

    let mut cadence = cadence(worker);
    assert!(
        cadence
            .on_edit(&scene, &mut physics, None)
            .expect("valid scene"),
        "the structural replacement stays pending while it would enter the body"
    );
    if worker {
        wait_completed(&cadence);
    }

    for frame_index in 0..64 {
        assert!(
            cadence
                .step(&scene, &mut physics)
                .expect("valid scene")
                .is_none(),
            "frame {frame_index}: a same-outer-AABB replacement must not publish through a body"
        );
        assert_eq!(
            physics.detail_collision_stats().static_colliders,
            load.static_colliders,
            "frame {frame_index}: live collision stays untouched while withheld"
        );
        if worker {
            assert_eq!(
                cadence.stats().results,
                1,
                "frame {frame_index}: the completed result is retained, not dropped"
            );
        }
        settle(&mut physics, 1);
    }

    // Control: the body genuinely clears the replacement's added cell.
    physics
        .restore(&crate_snapshot(CRATE_CLEAR))
        .expect("valid crate snapshot");
    assert!(!any_body_overlaps(&physics, FILL_CELL_BOX));
    let mut published = None;
    for _ in 0..STALL_FRAMES {
        if let Some(stats) = cadence.step(&scene, &mut physics).expect("valid scene") {
            published = Some(stats);
            break;
        }
        settle(&mut physics, 1);
    }
    let stats = published.expect("the retained replacement publishes once the body clears");
    assert_eq!(stats.static_colliders, load.static_colliders + 1);
    if worker {
        assert_eq!(
            cadence.stats().results,
            0,
            "the buffered result was consumed"
        );
    }
}

#[test]
fn worker_structural_edit_clear_of_a_resting_body_publishes_immediately() {
    structural_edit_clear_of_a_resting_body(true);
}

#[test]
fn workerless_structural_edit_clear_of_a_resting_body_publishes_immediately() {
    structural_edit_clear_of_a_resting_body(false);
}

#[test]
fn worker_structural_new_material_inside_a_resting_body_defers_until_it_clears() {
    structural_edit_inside_a_resting_body(true);
}

#[test]
fn workerless_structural_new_material_inside_a_resting_body_defers_until_it_clears() {
    structural_edit_inside_a_resting_body(false);
}

#[test]
fn worker_structural_same_aabb_replacement_inside_a_resting_body_still_defers() {
    structural_same_aabb_replacement_inside_a_resting_body(true);
}

#[test]
fn workerless_structural_same_aabb_replacement_inside_a_resting_body_still_defers() {
    structural_same_aabb_replacement_inside_a_resting_body(false);
}
