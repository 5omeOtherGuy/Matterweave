//! Behaviour of the reusable [`AsyncDetailCollision`] controller.
//!
//! Assertions are on observable outcomes: published floor contact after a
//! worker-built preparation, preservation of live walls on stale/unrelated
//! rejection, error propagation for the *current* scene only, bounded queue
//! counts, supersession, reset/cancellation and safe shutdown.
//!
//! Synchronisation is deterministic: helpers spin on the public `stats`/`poll`
//! surface with an eventual bounded deadline instead of assuming a fixed timing
//! for worker progress. A blown deadline panics rather than hanging the suite.

use matterweave_detail::{material, DetailScene, DetailVolume, Scale, Transform, Yaw};
use matterweave_physics::{
    AsyncDetailCollision, Physics, PreparedDetailCollision, FIXED_DT, MAX_DETAIL_COLLIDERS,
};
use std::time::{Duration, Instant};

const TILE: f32 = 0.25;
const FEET: f32 = 0.85;
const EYE: f32 = 0.65;
/// Generous upper bound; the worker prepares these small scenes far faster.
const DEADLINE: Duration = Duration::from_secs(10);

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

fn floor_volume() -> DetailVolume {
    volume("floor", [32, 1, 32], material::BANK_STONE)
}

fn scene_with_floor() -> DetailScene {
    let mut scene = DetailScene::new();
    scene.add_prototype(floor_volume()).expect("prototype");
    scene
        .place("floor.0", "floor", Transform::identity())
        .expect("placement");
    scene
}

fn empty_physics() -> Physics {
    Physics::new(&matterweave_core::World::new(1))
}

fn settle(physics: &mut Physics, steps: usize) {
    for _ in 0..steps {
        physics.step(FIXED_DT, [0.0; 3], false);
    }
}

/// Spins on the controller until `poll(scene)` yields a result for `scene` or
/// the bounded deadline elapses. Stale/superseded results are discarded by the
/// controller itself during this wait, so only a result current for `scene`
/// returns. Panics on a blown deadline instead of hanging.
fn wait_poll(
    ctrl: &mut AsyncDetailCollision,
    scene: &DetailScene,
) -> Result<PreparedDetailCollision, String> {
    let started = Instant::now();
    loop {
        if let Some(result) = ctrl.poll(scene) {
            return result;
        }
        assert!(
            started.elapsed() < DEADLINE,
            "no current result within the deadline; stats {:?}",
            ctrl.stats()
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Spins until at least one result is buffered, so a test can inspect a result
/// before deciding how to poll it.
fn wait_for_buffered(ctrl: &AsyncDetailCollision) {
    let started = Instant::now();
    while ctrl.stats().results == 0 {
        assert!(
            started.elapsed() < DEADLINE,
            "worker produced no buffered result; stats {:?}",
            ctrl.stats()
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn worker_preparation_publishes_a_floor_the_character_stands_on() {
    let scene = scene_with_floor();
    let mut ctrl = AsyncDetailCollision::new();
    assert!(ctrl.available(), "worker started");
    assert!(ctrl.request(&scene), "a fresh source is queued");

    let prepared = wait_poll(&mut ctrl, &scene).expect("valid preparation");
    assert_eq!(prepared.stats().static_colliders, 1);

    let mut physics = empty_physics();
    assert_eq!(physics.detail_collider_count(), 0, "nothing published yet");
    physics
        .publish_detail_scene(&scene, prepared)
        .expect("published");
    assert!(physics.teleport([2.0, 2.0, 2.0]));
    settle(&mut physics, 120);
    assert!(physics.grounded(), "supported by the worker-built slab");
    let eye = physics.character_eye();
    assert!(
        (eye[1] - (TILE + FEET + EYE)).abs() < 0.05,
        "feet rest on the slab, eye {eye:?}"
    );
}

#[test]
fn edit_between_request_and_poll_rejects_and_keeps_live_walls() {
    // Publish an initial floor synchronously so there is live collision to protect.
    let mut scene = scene_with_floor();
    let mut physics = empty_physics();
    physics.replace_detail_scene(&scene).expect("initial floor");
    let before = physics.detail_collision_stats();
    assert!(physics.teleport([2.0, 2.0, 2.0]));
    settle(&mut physics, 120);
    assert!(physics.grounded());

    let mut ctrl = AsyncDetailCollision::new();
    assert!(ctrl.request(&scene));
    wait_for_buffered(&ctrl);

    // Edit the authoritative scene after the worker captured its snapshot.
    scene
        .edit_prototype("floor", [0, 0, 0], material::AIR)
        .expect("edit");

    // Polling against the edited scene must reject the now-stale preparation.
    assert!(
        ctrl.poll(&scene).is_none(),
        "a stale preparation is not publishable"
    );
    // Live collision is untouched: the character keeps standing on the floor.
    assert_eq!(physics.detail_collision_stats(), before);
    settle(&mut physics, 60);
    assert!(physics.grounded(), "live walls remain intact");
}

#[test]
fn unrelated_scene_with_matching_counters_is_rejected() {
    let scene = scene_with_floor();
    let mut ctrl = AsyncDetailCollision::new();
    assert!(ctrl.request(&scene));
    wait_for_buffered(&ctrl);
    // A different scene object with identical content/counters has a different
    // opaque source identity, so the preparation is not current for it.
    let unrelated = scene_with_floor();
    assert!(
        ctrl.poll(&unrelated).is_none(),
        "matching revision counters do not make an unrelated scene current"
    );
    // The result is still valid for the scene it was actually prepared from.
    assert!(
        ctrl.poll(&scene).is_some(),
        "current scene still publishable"
    );
}

#[test]
fn duplicate_same_source_requests_are_refused() {
    let scene = scene_with_floor();
    let mut ctrl = AsyncDetailCollision::new();
    assert!(ctrl.request(&scene), "first request accepted");
    assert!(
        !ctrl.request(&scene),
        "an identical source is deduplicated while pending/running/buffered"
    );
    let prepared = wait_poll(&mut ctrl, &scene).expect("valid preparation");
    assert_eq!(prepared.stats().static_colliders, 1);
}

#[test]
fn rapid_supersession_keeps_bounds_and_yields_the_latest() {
    let mut ctrl = AsyncDetailCollision::new();
    let mut scenes = Vec::new();
    // Each scene has a distinct translation, so their source identities differ.
    for index in 0..32 {
        let mut scene = DetailScene::new();
        scene.add_prototype(floor_volume()).expect("prototype");
        scene
            .place(
                "floor.0",
                "floor",
                Transform::new([index as f32, 0.0, 0.0], Yaw::Deg0).expect("transform"),
            )
            .expect("placement");
        scenes.push(scene);
    }
    for scene in &scenes {
        ctrl.request(scene);
        let stats = ctrl.stats();
        assert!(stats.queued <= 1, "at most one pending snapshot: {stats:?}");
        assert!(stats.inflight <= 1, "at most one running job: {stats:?}");
        assert!(stats.results <= 1, "at most one buffered result: {stats:?}");
    }
    let latest = scenes.last().expect("scene");
    let prepared = wait_poll(&mut ctrl, latest).expect("latest preparation");
    assert_eq!(prepared.stats().static_colliders, 1);
    // Publishing the latest must succeed against its own scene.
    let mut physics = empty_physics();
    let published = physics
        .publish_detail_scene(latest, prepared)
        .expect("latest publishes");
    assert_eq!(published.static_colliders, 1);
    assert!(
        ctrl.stats().discarded > 0,
        "superseded snapshots were discarded"
    );
}

#[test]
fn reset_cancels_work_and_allows_a_fresh_request() {
    let scene = scene_with_floor();
    let mut ctrl = AsyncDetailCollision::new();
    assert!(ctrl.request(&scene));
    ctrl.reset();
    let gen_after_reset = ctrl.stats().generation;
    assert!(gen_after_reset >= 1, "reset advanced the generation");

    // Any result from the cancelled generation must never be published.
    let started = Instant::now();
    while started.elapsed() < Duration::from_millis(50) {
        assert!(
            ctrl.poll(&scene).is_none(),
            "cancelled work is never publishable"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(ctrl.stats().results, 0, "no buffered result survives reset");

    // A fresh request after reset works normally.
    assert!(ctrl.request(&scene), "re-request accepted after reset");
    let prepared = wait_poll(&mut ctrl, &scene).expect("fresh preparation");
    assert_eq!(prepared.stats().static_colliders, 1);
}

#[test]
fn invalid_preparation_propagates_for_the_current_scene() {
    // One tiny stone pebble per instance, one more instance than the collider cap.
    let mut scene = DetailScene::new();
    scene
        .add_prototype(volume("pebble", [1, 1, 1], material::BANK_STONE))
        .expect("prototype");
    for index in 0..=MAX_DETAIL_COLLIDERS {
        scene
            .place(
                format!("pebble.{index}"),
                "pebble",
                Transform::new([index as f32 * 0.001, 60.0, 0.0], Yaw::Deg0).expect("transform"),
            )
            .expect("placement");
    }
    let mut ctrl = AsyncDetailCollision::new();
    assert!(ctrl.request(&scene));
    let error = match wait_poll(&mut ctrl, &scene) {
        Ok(_) => panic!("an over-budget scene must fail preparation"),
        Err(error) => error,
    };
    assert!(
        error.contains("static colliders"),
        "explicit limit: {error}"
    );
}

#[test]
fn stale_error_does_not_reject_the_current_scene() {
    // An over-budget scene produces an error result, but if the current scene is
    // a different (valid) one, that stale error must not surface as the current
    // scene's result. Polling the current scene simply finds nothing yet.
    let mut oversized = DetailScene::new();
    oversized
        .add_prototype(volume("pebble", [1, 1, 1], material::BANK_STONE))
        .expect("prototype");
    for index in 0..=MAX_DETAIL_COLLIDERS {
        oversized
            .place(
                format!("pebble.{index}"),
                "pebble",
                Transform::new([index as f32 * 0.001, 60.0, 0.0], Yaw::Deg0).expect("transform"),
            )
            .expect("placement");
    }
    let mut ctrl = AsyncDetailCollision::new();
    assert!(ctrl.request(&oversized));
    wait_for_buffered(&ctrl);
    // A valid, unrelated current scene must not receive the stale error.
    let current = scene_with_floor();
    assert!(
        ctrl.poll(&current).is_none(),
        "a stale error must not reject the current scene"
    );
}

#[test]
fn stats_stay_bounded_and_shutdown_is_clean() {
    let scene = scene_with_floor();
    {
        let mut ctrl = AsyncDetailCollision::new();
        assert_eq!(
            ctrl.stats(),
            Default::default(),
            "fresh controller is empty"
        );
        assert!(ctrl.request(&scene));
        // Sample bounds repeatedly across the worker lifecycle.
        let started = Instant::now();
        while started.elapsed() < Duration::from_millis(30) {
            let stats = ctrl.stats();
            assert!(stats.queued <= 1 && stats.inflight <= 1 && stats.results <= 1);
            std::thread::sleep(Duration::from_millis(1));
        }
        // Drop with work still in flight must not hang or panic.
    }
    // Reaching here means shutdown joined the worker cleanly.
}
