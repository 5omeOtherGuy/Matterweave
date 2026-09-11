//! Behaviour of the [`DetailCollisionCadence`] edit-to-collision policy.
//!
//! Assertions are on observable simulation outcomes, mirroring how the
//! explorer wetland runtime drives the cadence: an edit during active
//! movement, repeated/reversed edits, a scene reset with work in flight, a
//! stale completed result for an edited scene, and a current-scene
//! preparation failure. Live collision must always match the last accepted
//! publication; movement correctness must hold while work is pending.

use matterweave_detail::{material, DetailScene, DetailVolume, Scale, Transform, Yaw};
use matterweave_physics::{DetailCollisionCadence, Physics, FIXED_DT, MAX_DETAIL_BOXES};
use std::time::{Duration, Instant};

const TILE: f32 = 0.25;
const DEADLINE: Duration = Duration::from_secs(30);
/// Capsule radius plus solver slack: while the blocking cell (face at x=2.0)
/// is solid, the capsule centre cannot pass this.
const BLOCK_LIMIT_X: f32 = 1.78;
/// Beyond the far face of the removed cell (2.25) plus radius and slack.
const PAST_WALL_X: f32 = 2.7;
const WALK_SPEED: f32 = 1.0;
/// World-space box of the `wall.0` prototype cell: instance at
/// `[2.0, TILE, 4.0]`, cell `[0, 0, 0]`, 0.25 m scale.
const WALL_BOX: ([f32; 3], [f32; 3]) = ([2.0, 0.25, 4.0], [2.25, 0.5, 4.25]);

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

/// Floor slab 8 m by 8 m without a wall.
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

fn scene_with_floor_and_wall() -> DetailScene {
    let mut scene = scene_with_floor();
    scene
        .add_prototype(volume("wall", [1, 1, 1], material::BANK_STONE))
        .expect("prototype");
    scene
        .place(
            "wall.0",
            "wall",
            Transform::new([2.0, TILE, 4.0], Yaw::Deg0).expect("transform"),
        )
        .expect("placement");
    scene
}

fn settle(physics: &mut Physics, steps: usize) {
    for _ in 0..steps {
        physics.step(FIXED_DT, [0.0; 3], false);
    }
}

fn spawn_on_floor(physics: &mut Physics) -> [f32; 3] {
    assert!(physics.teleport([1.0, 3.0, 4.125]));
    settle(physics, 120);
    let eye = physics.character_eye();
    assert!(physics.grounded(), "supported by the floor slab");
    assert!(
        (eye[1] - 1.75).abs() < 0.05,
        "standing on the slab, eye {eye:?}"
    );
    eye
}

/// Spins on the cadence until a result current for `scene` publishes.
fn wait_published(
    cadence: &mut DetailCollisionCadence,
    scene: &DetailScene,
    physics: &mut Physics,
) {
    let started = Instant::now();
    loop {
        assert!(
            started.elapsed() < DEADLINE,
            "no publication within the deadline; stats {:?}",
            cadence.stats()
        );
        if cadence
            .step(scene, physics)
            .expect("the scene under test is valid")
            .is_some()
        {
            return;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// A prototype whose checkerboard cells merge into exactly one box each, so
/// `cells` boxes are charged against [`MAX_DETAIL_BOXES`].
fn checkerboard(id: &str, chunk_keys: usize) -> DetailVolume {
    let mut volume = DetailVolume::new(id, Scale::new(TILE).expect("valid scale"));
    for key in 0..chunk_keys {
        let base = (key * 16) as i32;
        for x in 0..16 {
            for y in 0..16 {
                for z in 0..16 {
                    if (x + y + z) % 2 == 0 {
                        volume
                            .set([base + x, y, z], material::BANK_STONE)
                            .expect("in-budget cell");
                    }
                }
            }
        }
    }
    volume
}

#[test]
fn edit_during_active_movement_blocks_until_publication_then_frees_the_path() {
    let mut scene = scene_with_floor_and_wall();
    let mut physics = empty_physics();
    physics.replace_detail_scene(&scene).expect("load");
    spawn_on_floor(&mut physics);
    let mut cadence = DetailCollisionCadence::new();

    // The player removes the blocking cell and keeps walking toward it.
    scene.edit_instance("wall.0", [0, 0, 0], 0).expect("edit");
    assert!(cadence
        .on_edit(&scene, &mut physics, Some(&[]))
        .expect("queued"));
    assert!(
        cadence.stats().queued + cadence.stats().inflight >= 1,
        "the edit is pending: {:?}",
        cadence.stats()
    );

    // While the work is pending, the live (old) collision keeps blocking and
    // the floor keeps carrying the character: no hole, no silent wall change.
    let started = Instant::now();
    let mut pending_frames = 0usize;
    loop {
        assert!(started.elapsed() < DEADLINE, "deadline blown");
        match cadence.step(&scene, &mut physics).expect("valid scene") {
            None => {
                physics.step(FIXED_DT, [WALK_SPEED, 0.0, 0.0], false);
                pending_frames += 1;
                let eye = physics.character_eye();
                assert!(
                    eye[0] < BLOCK_LIMIT_X,
                    "a pending edit must not free the path early: eye {eye:?} \
                     after {pending_frames} pending frames"
                );
                assert!(
                    physics.grounded() && eye[1] > 1.5,
                    "a pending edit must not disturb the floor: eye {eye:?}"
                );
            }
            Some(_) => break,
        }
    }
    assert!(
        pending_frames > 0,
        "the scenario must actually observe pending frames"
    );

    // After publication the same walk passes the removed cell; the floor
    // still carries the character the whole way.
    let started = Instant::now();
    let mut published_frames = 0usize;
    while physics.character_eye()[0] <= PAST_WALL_X {
        assert!(started.elapsed() < DEADLINE, "deadline blown walking out");
        physics.step(FIXED_DT, [WALK_SPEED, 0.0, 0.0], false);
        published_frames += 1;
        let eye = physics.character_eye();
        assert!(
            physics.grounded() && eye[1] > 1.5,
            "the floor must still carry the character: eye {eye:?}"
        );
    }
    assert!(published_frames > 0, "the character actually moved out");
}

#[test]
fn repeated_and_reversed_edits_publish_only_the_final_source() {
    let mut scene = scene_with_floor_and_wall();
    let mut physics = empty_physics();
    physics.replace_detail_scene(&scene).expect("load");
    let initial_stats = physics.detail_collision_stats();
    let mut cadence = DetailCollisionCadence::new();

    // Remove the cell, then reverse the edit before anything publishes.
    scene.edit_instance("wall.0", [0, 0, 0], 0).expect("remove");
    assert!(cadence
        .on_edit(&scene, &mut physics, Some(&[]))
        .expect("queued"));
    scene
        .edit_instance("wall.0", [0, 0, 0], material::BANK_STONE)
        .expect("restore");
    assert!(cadence
        .on_edit(&scene, &mut physics, Some(&[WALL_BOX]))
        .expect("queued"));
    assert!(
        cadence.stats().discarded >= 1,
        "the superseded removal was discarded: {:?}",
        cadence.stats()
    );

    // Repeated edits of the unchanged final source are deduplicated (the
    // request is refused or the result is already buffered); nothing publishes
    // in between: live collision is still the load state.
    let stats_before_repeat = cadence.stats();
    cadence
        .on_edit(&scene, &mut physics, Some(&[WALL_BOX]))
        .expect("dedup");
    let stats_after_repeat = cadence.stats();
    assert!(
        stats_after_repeat.discarded == stats_before_repeat.discarded
            && stats_after_repeat.queued <= 1
            && stats_after_repeat.results <= 1,
        "a repeated edit neither queues duplicate work nor discards: {stats_after_repeat:?}"
    );
    assert_eq!(
        physics.detail_collision_stats(),
        initial_stats,
        "no intermediate publication touched live collision"
    );

    // The restored source publishes exactly once with the original content.
    wait_published(&mut cadence, &scene, &mut physics);
    let stats = physics.detail_collision_stats();
    assert_eq!(stats.static_colliders, initial_stats.static_colliders);
    assert_eq!(
        stats.source_collision_cells, initial_stats.source_collision_cells,
        "the reversed edit restored the original collision content"
    );
    assert_eq!(cadence.stats().results, 0, "nothing left buffered");
}

#[test]
fn reset_with_work_in_flight_never_publishes_retired_results() {
    let mut scene = scene_with_floor_and_wall();
    let mut physics = empty_physics();
    physics.replace_detail_scene(&scene).expect("load");
    let before = physics.detail_collision_stats();
    let mut cadence = DetailCollisionCadence::new();

    scene.edit_instance("wall.0", [0, 0, 0], 0).expect("edit");
    assert!(cadence
        .on_edit(&scene, &mut physics, Some(&[]))
        .expect("queued"));
    // Scene replacement / load cancels everything; the in-flight job is
    // invalidated and can never publish.
    cadence.reset();
    assert!(
        cadence.stats().generation >= 1,
        "reset advanced the generation"
    );

    let started = Instant::now();
    while started.elapsed() < Duration::from_millis(100) {
        assert!(
            cadence
                .step(&scene, &mut physics)
                .expect("no failure published")
                .is_none(),
            "a retired result must never publish after reset"
        );
        assert_eq!(
            physics.detail_collision_stats(),
            before,
            "live collision is untouched by cancelled work"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    // A fresh edit after reset is accepted and publishes normally.
    scene.edit_instance("wall.0", [0, 0, 0], 0).expect("edit");
    assert!(
        cadence
            .on_edit(&scene, &mut physics, Some(&[]))
            .expect("queued after reset"),
        "a fresh request after reset is accepted"
    );
    wait_published(&mut cadence, &scene, &mut physics);
    assert!(
        physics.detail_collision_stats().source_collision_cells < before.source_collision_cells,
        "the fresh edit published and removed the blocking cell"
    );
}

#[test]
fn stale_completion_for_an_edited_scene_is_never_published() {
    let mut scene = scene_with_floor_and_wall();
    let mut physics = empty_physics();
    physics.replace_detail_scene(&scene).expect("load");
    let load_stats = physics.detail_collision_stats();
    let mut cadence = DetailCollisionCadence::new();

    // Publish the removal (cell count drops by one).
    scene.edit_instance("wall.0", [0, 0, 0], 0).expect("edit");
    assert!(cadence
        .on_edit(&scene, &mut physics, Some(&[]))
        .expect("queued"));
    wait_published(&mut cadence, &scene, &mut physics);
    assert_eq!(
        physics.detail_collision_stats().source_collision_cells,
        load_stats.source_collision_cells - 1
    );

    // Build a stale completion: buffer the restore result, then edit again so
    // the buffered result no longer matches the authoritative scene.
    scene
        .edit_instance("wall.0", [0, 0, 0], material::BANK_STONE)
        .expect("restore");
    assert!(cadence
        .on_edit(&scene, &mut physics, Some(&[WALL_BOX]))
        .expect("queued"));
    let started = Instant::now();
    while cadence.stats().results == 0 {
        assert!(started.elapsed() < DEADLINE, "no buffered result");
        std::thread::sleep(Duration::from_millis(1));
    }
    scene
        .edit_instance("wall.0", [0, 0, 0], 0)
        .expect("edit again");
    assert!(cadence
        .on_edit(&scene, &mut physics, Some(&[]))
        .expect("queued"));

    // Until the final source's result arrives, every frame is either idle or
    // leaves the live state untouched; the stale (restored) result must never
    // publish. When publication happens it carries the final (removed) state.
    let started = Instant::now();
    loop {
        assert!(started.elapsed() < DEADLINE, "deadline blown");
        match cadence.step(&scene, &mut physics).expect("valid scene") {
            None => {
                assert_ne!(
                    physics.detail_collision_stats().source_collision_cells,
                    load_stats.source_collision_cells,
                    "the stale restored result published: cell count back at load value"
                );
                std::thread::sleep(Duration::from_millis(1));
            }
            Some(_) => break,
        }
    }
    assert_eq!(
        physics.detail_collision_stats().source_collision_cells,
        load_stats.source_collision_cells - 1,
        "the final (removed) source is what published"
    );
    assert!(
        cadence.stats().discarded >= 1,
        "the stale completion was discarded: {:?}",
        cadence.stats()
    );
}

#[test]
fn current_scene_preparation_failure_preserves_live_collision() {
    // The load-time scene fills the aggregate merged-box budget exactly; the
    // next one-cell edit deterministically fails preparation. This is the
    // only public-API way to make the *current* source fail, because the
    // other budgets exceed what a single-cell edit can reach.
    let mut scene = DetailScene::new();
    scene
        .add_prototype(checkerboard("sponge", 128))
        .expect("prototype");
    scene
        .place("sponge.0", "sponge", Transform::identity())
        .expect("placement");
    let mut physics = empty_physics();
    let load_stats = physics
        .replace_detail_scene(&scene)
        .expect("the 128-chunk checkerboard fills the box budget exactly");
    assert_eq!(load_stats.merged_boxes, MAX_DETAIL_BOXES);
    let mut cadence = DetailCollisionCadence::new();

    // One more isolated cell exceeds the budget; the current source fails.
    scene
        .edit_instance("sponge.0", [128 * 16, 0, 0], material::BANK_STONE)
        .expect("edit");
    assert!(cadence
        .on_edit(
            &scene,
            &mut physics,
            Some(&[([512.0, 0.0, 0.0], [512.25, 0.25, 0.25])])
        )
        .expect("queued"));
    let started = Instant::now();
    let error = loop {
        match cadence.step(&scene, &mut physics) {
            Err(error) => break error,
            Ok(None) => std::thread::sleep(Duration::from_millis(1)),
            Ok(Some(_)) => panic!("an over-budget source must not publish"),
        }
        assert!(started.elapsed() < DEADLINE, "deadline blown");
    };
    assert!(error.contains("merged boxes"), "explicit limit: {error}");
    assert_eq!(
        physics.detail_collision_stats(),
        load_stats,
        "live collision preserved across the failure"
    );

    // The owner reverts the offending edit and re-requests; consistency is
    // restored against the still-live collision.
    scene
        .edit_instance("sponge.0", [128 * 16, 0, 0], 0)
        .expect("revert");
    assert!(cadence
        .on_edit(&scene, &mut physics, Some(&[]))
        .expect("queued"));
    wait_published(&mut cadence, &scene, &mut physics);
    assert_eq!(
        physics.detail_collision_stats(),
        load_stats,
        "collision matches the reverted authoritative scene again"
    );
}

/// The capsule overlaps the wall cell (x in [2.0, 2.25]) while its centre is in
/// this band: radius 0.30 either side.
const ADD_OVERLAP_BAND: (f32, f32) = (1.70, 2.55);

/// A pending edit that ADDS a wall must never publish through a character that
/// advanced into its region while preparation was pending. The old policy
/// ("old collision stays live, publish on completion") passed here: the new
/// collider would materialise inside the capsule. The publication gate defers
/// it until the body has left the added region, then publishes normally.
#[test]
fn added_wall_publishes_only_after_the_character_clears_it() {
    let mut scene = scene_with_floor();
    let mut physics = empty_physics();
    physics.replace_detail_scene(&scene).expect("load");
    spawn_on_floor(&mut physics);
    let floor_colliders = physics.detail_collision_stats().static_colliders;

    // Walk toward the future wall position while it does not exist yet.
    let started = Instant::now();
    while physics.character_eye()[0] < 1.8 {
        assert!(started.elapsed() < DEADLINE, "deadline blown approaching");
        physics.step(FIXED_DT, [WALK_SPEED, 0.0, 0.0], false);
    }
    let mut cadence = DetailCollisionCadence::new();
    scene
        .add_prototype(volume("wall", [1, 1, 1], material::BANK_STONE))
        .expect("prototype");
    scene
        .place(
            "wall.0",
            "wall",
            Transform::new([2.0, TILE, 4.0], Yaw::Deg0).expect("transform"),
        )
        .expect("placement");
    assert!(cadence
        .on_edit(&scene, &mut physics, Some(&[WALL_BOX]))
        .expect("queued"));

    // Wait until the wall's preparation is buffered, with the character
    // standing inside the region the wall will occupy.
    let started = Instant::now();
    while cadence.stats().results == 0 {
        assert!(started.elapsed() < DEADLINE, "no buffered result");
        std::thread::sleep(Duration::from_millis(1));
    }

    // Walk through the future wall region: no publication may happen while
    // the capsule overlaps the added collider, and the retained result stays
    // buffered.
    let mut overlap_frames = 0usize;
    let published = loop {
        assert!(started.elapsed() < DEADLINE, "deadline blown");
        match cadence.step(&scene, &mut physics).expect("valid scene") {
            Some(stats) => break stats,
            None => {
                physics.step(FIXED_DT, [WALK_SPEED, 0.0, 0.0], false);
                let eye = physics.character_eye();
                if eye[0] > ADD_OVERLAP_BAND.0 && eye[0] < ADD_OVERLAP_BAND.1 {
                    overlap_frames += 1;
                    assert_eq!(
                        physics.detail_collision_stats().static_colliders,
                        floor_colliders,
                        "new collision must not publish through the character: eye {eye:?}"
                    );
                    assert_eq!(
                        cadence.stats().results,
                        1,
                        "the gated result stays buffered for a retry: {eye:?}"
                    );
                }
            }
        }
    };
    assert!(
        overlap_frames > 0,
        "the scenario must actually observe overlapping frames"
    );
    assert_eq!(
        published.static_colliders,
        floor_colliders + 1,
        "the wall publishes once the character has left its region"
    );
    let eye = physics.character_eye();
    assert!(
        eye[0] >= ADD_OVERLAP_BAND.1,
        "publication must wait until the character is clear: eye {eye:?}"
    );

    // The character is not trapped: it keeps walking freely and stays grounded.
    let started = Instant::now();
    while physics.character_eye()[0] < 3.0 {
        assert!(started.elapsed() < DEADLINE, "deadline blown walking on");
        physics.step(FIXED_DT, [WALK_SPEED, 0.0, 0.0], false);
        let eye = physics.character_eye();
        assert!(
            physics.grounded() && eye[1] > 1.5,
            "the floor must still carry the character: eye {eye:?}"
        );
    }
}

/// An add edit reversed before publication must never publish the added wall,
/// even though its preparation completed while the character was already
/// inside the region the wall would occupy: the stale buffered result is
/// superseded and only the final (wall-free) source publishes.
#[test]
fn reversed_addition_never_publishes_through_the_character() {
    let mut scene = scene_with_floor();
    let mut physics = empty_physics();
    physics.replace_detail_scene(&scene).expect("load");
    spawn_on_floor(&mut physics);
    let floor_colliders = physics.detail_collision_stats().static_colliders;
    let mut cadence = DetailCollisionCadence::new();

    // Add the wall while the character is still clear of it, and let its
    // preparation complete into the buffer.
    scene
        .add_prototype(volume("wall", [1, 1, 1], material::BANK_STONE))
        .expect("prototype");
    scene
        .place(
            "wall.0",
            "wall",
            Transform::new([2.0, TILE, 4.0], Yaw::Deg0).expect("transform"),
        )
        .expect("placement");
    assert!(cadence
        .on_edit(&scene, &mut physics, Some(&[WALL_BOX]))
        .expect("queued"));
    let started = Instant::now();
    while cadence.stats().results == 0 {
        assert!(started.elapsed() < DEADLINE, "no buffered result");
        std::thread::sleep(Duration::from_millis(1));
    }

    // Rapid reversal before anything publishes: the wall is removed again.
    scene.edit_instance("wall.0", [0, 0, 0], 0).expect("remove");
    assert!(cadence
        .on_edit(&scene, &mut physics, Some(&[]))
        .expect("queued"));

    // Walk through the region: the added wall must never appear. Then keep
    // pumping until the superseded wall preparation has actually been retired.
    // Discard is the worker's own bookkeeping: the stale buffered result is
    // only counted once the reversing job runs to completion, so the length of
    // the walk is no bound on it. A starved worker leaves the walk finished
    // with the stale result still buffered, which is a scheduling accident
    // rather than a behavioural fault. Every pumped frame, walking or waiting,
    // still asserts that the added wall never reaches live collision.
    let started = Instant::now();
    loop {
        assert!(started.elapsed() < DEADLINE, "deadline blown walking");
        match cadence.step(&scene, &mut physics).expect("valid scene") {
            Some(stats) => assert_eq!(
                stats.static_colliders,
                floor_colliders,
                "only the wall-free source may publish: eye {:?}",
                physics.character_eye()
            ),
            None => assert_eq!(
                physics.detail_collision_stats().static_colliders,
                floor_colliders,
                "no frame may show the added wall: eye {:?}",
                physics.character_eye()
            ),
        }
        if physics.character_eye()[0] < 3.0 {
            physics.step(FIXED_DT, [WALK_SPEED, 0.0, 0.0], false);
        } else if cadence.stats().discarded >= 1 && cadence.stats().results == 0 {
            break;
        } else {
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    assert!(
        cadence.stats().discarded >= 1,
        "the superseded wall preparation was discarded: {:?}",
        cadence.stats()
    );
    assert_eq!(cadence.stats().results, 0, "nothing left buffered");
}

/// Filling an interior hole leaves the whole-collider AABB identical while new
/// solid material appears inside it. A gate comparing whole-collider AABBs
/// (the previous policy) sees "no added collider" and publishes through a body
/// standing in the hole region. The journal-sourced added-cell region gates at
/// cell granularity instead: publication defers while the capsule AABB
/// overlaps the filled cell, then publishes once the body clears it.
#[test]
fn interior_fill_with_equal_outer_aabb_never_publishes_through_the_body() {
    let mut bar = DetailVolume::new("bar", Scale::new(TILE).expect("valid scale"));
    for x in [0, 1, 6, 7] {
        bar.set([x, 0, 0], material::BANK_STONE).expect("cell");
    }
    let mut scene = DetailScene::new();
    scene.add_prototype(bar).expect("prototype");
    scene
        .place("bar.0", "bar", Transform::identity())
        .expect("placement");
    let mut physics = empty_physics();
    let load_stats = physics.replace_detail_scene(&scene).expect("load");
    // The capsule stands in the empty 1 m gap (x in [0.5, 1.5]); its AABB
    // reaches into the cell x = 2 (world [0.5, 0.75]) that is about to fill.
    assert!(physics.teleport([1.0, 0.95, 0.125]), "gap fits the capsule");
    // No floor here; one step only syncs the collider transform (gravity
    // cannot shift x/z, so the cell overlap is preserved).
    physics.step(FIXED_DT, [0.0; 3], false);
    let mut cadence = DetailCollisionCadence::new();
    scene
        .edit_instance("bar.0", [2, 0, 0], material::BANK_STONE)
        .expect("fill");
    const FILL_BOX: ([f32; 3], [f32; 3]) = ([0.5, 0.0, 0.0], [0.75, 0.25, 0.25]);
    assert!(cadence
        .on_edit(&scene, &mut physics, Some(&[FILL_BOX]))
        .expect("queued"));

    // The outer collider AABB is identical before and after ([0, 2.0] in x),
    // so a whole-AABB comparison would publish here. The cell gate retains.
    let started = Instant::now();
    let mut retained_frames = 0usize;
    while cadence.stats().results == 0 {
        assert!(started.elapsed() < DEADLINE, "no buffered result");
        assert!(
            cadence
                .step(&scene, &mut physics)
                .expect("valid scene")
                .is_none(),
            "the fill must not publish while the capsule overlaps its cell"
        );
        retained_frames += 1;
        std::thread::sleep(Duration::from_millis(1));
    }
    // Buffered and still blocked: every further frame retains without
    // touching live collision.
    for _ in 0..5 {
        assert!(
            cadence
                .step(&scene, &mut physics)
                .expect("valid scene")
                .is_none(),
            "the retained fill must not publish through the capsule"
        );
        retained_frames += 1;
    }
    assert!(retained_frames > 0, "retention actually observed");
    assert_eq!(
        physics.detail_collision_stats(),
        load_stats,
        "live collision untouched while the fill is gated"
    );

    // Clearing the cell region publishes the fill exactly once.
    assert!(physics.teleport([5.0, 0.95, 0.125]), "clear of the bar");
    physics.step(FIXED_DT, [0.0; 3], false);
    wait_published(&mut cadence, &scene, &mut physics);
    assert_eq!(
        physics.detail_collision_stats().source_collision_cells,
        load_stats.source_collision_cells + 1,
        "the fill published once the body cleared its cell"
    );
    assert_eq!(
        physics.detail_collision_stats().static_colliders,
        load_stats.static_colliders
    );
}

/// The synchronous fallback (worker unavailable) applies the same gate: a
/// blocked source is staged, not published through the body. The previous
/// policy published the fallback unconditionally during pending runtime edits.
#[test]
fn unavailable_worker_stages_blocked_fallback_and_publishes_after_clearing() {
    workerless_gate_after_optional_rejection(false);
}

#[test]
fn rejected_structural_request_preserves_the_prior_staged_addition_gate() {
    workerless_gate_after_optional_rejection(true);
}

fn workerless_gate_after_optional_rejection(reject_structural: bool) {
    let mut scene = scene_with_floor();
    let mut physics = empty_physics();
    physics.replace_detail_scene(&scene).expect("load");
    spawn_on_floor(&mut physics);
    let floor_colliders = physics.detail_collision_stats().static_colliders;
    let eye = physics.character_eye();

    let mut cadence = DetailCollisionCadence::without_worker();
    assert!(!cadence.available(), "fallback path under test");
    // Lean against the future wall: the capsule AABB overlaps the wall cell
    // box while the shapes stay clear (the wall does not exist yet).
    assert!(
        physics.teleport([1.75, eye[1], eye[2]]),
        "leaning pose fits the empty region"
    );
    physics.step(FIXED_DT, [0.0; 3], false);
    scene
        .add_prototype(volume("wall", [1, 1, 1], material::BANK_STONE))
        .expect("prototype");
    scene
        .place(
            "wall.0",
            "wall",
            Transform::new([2.0, TILE, 4.0], Yaw::Deg0).expect("transform"),
        )
        .expect("placement");
    assert!(
        cadence
            .on_edit(&scene, &mut physics, Some(&[WALL_BOX]))
            .expect("staged"),
        "a blocked fallback stays pending instead of publishing through the body"
    );
    if reject_structural {
        let mut bad = DetailScene::new();
        bad.add_prototype(checkerboard("over-limit", 129)).unwrap();
        bad.place("over-limit", "over-limit", Transform::identity())
            .unwrap();
        assert!(cadence.on_edit(&bad, &mut physics, None).is_err());
        // Owner rejects the failed request and continues with the prior source.
        // Its retained preparation must keep its original addition regions.
    }
    assert!(
        cadence
            .step(&scene, &mut physics)
            .expect("valid scene")
            .is_none(),
        "the staged wall must not publish through the capsule"
    );
    assert_eq!(
        physics.detail_collision_stats().static_colliders,
        floor_colliders,
        "live collision untouched while the fallback is staged"
    );

    assert!(physics.teleport([5.0, eye[1], eye[2]]), "clear of the wall");
    physics.step(FIXED_DT, [0.0; 3], false);
    let mut published = None;
    let started = Instant::now();
    while published.is_none() {
        assert!(
            started.elapsed() < DEADLINE,
            "staged result never published"
        );
        published = cadence.step(&scene, &mut physics).expect("valid scene");
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(
        published.expect("published").static_colliders,
        floor_colliders + 1,
        "the staged wall publishes once the body clears its cell"
    );
}

#[test]
fn teleported_character_is_gated_before_the_next_physics_step() {
    let floor = scene_with_floor();
    let mut physics = empty_physics();
    physics.replace_detail_scene(&floor).unwrap();
    let eye = spawn_on_floor(&mut physics);
    let scene = scene_with_floor_and_wall();
    assert!(physics.teleport([2.1, eye[1], eye[2]]));
    // Publication happens before stepping in the native loop. Collider caches
    // may still describe the old pose; the gate must use the current body pose.
    let mut cadence = DetailCollisionCadence::without_worker();
    assert!(cadence
        .on_edit(&scene, &mut physics, Some(&[WALL_BOX]))
        .unwrap());
    assert_eq!(physics.detail_collision_stats().static_colliders, 1);
}
