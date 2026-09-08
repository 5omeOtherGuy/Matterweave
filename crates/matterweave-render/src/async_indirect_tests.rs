//! Behavior tests for `async_indirect`: real radiance equality against the
//! synchronous reference, invalidation semantics, and deterministic private
//! queue-state interleavings (reversal, reset, stale results).

use super::{AsyncIndirectConfig, AsyncIndirectLight, Queue, SourceKey};
use crate::indirect::{IndirectVolume, UpdateBudget};
use crate::Sun;
use matterweave_core::World;
use std::cell::Cell;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn sun() -> Sun {
    Sun {
        direction_to_sun: [0., 1., 0.],
        intensity: 1.,
    }
}

fn bad_sun() -> Sun {
    Sun {
        direction_to_sun: [0., 0., 0.],
        intensity: 1.,
    }
}

fn palette() -> [[f32; 3]; 256] {
    let mut palette = [[0.5f32; 3]; 256];
    palette[1] = [0.9, 0.05, 0.02];
    palette
}

fn config() -> AsyncIndirectConfig {
    AsyncIndirectConfig::new([-3, -1, -3], [7, 5, 7], 64, 16., palette()).unwrap()
}

fn room(closed: bool) -> World {
    let mut w = World::new(7);
    for x in -3..=3 {
        for z in -3..=3 {
            w.set([x, -1, z], 1);
            if closed {
                w.set([x, 3, z], 2);
            }
            for y in 0..3 {
                if x.abs() == 3 || z.abs() == 3 {
                    w.set([x, y, z], 2);
                }
            }
        }
    }
    w
}

fn sync_reference(world: &World, epoch: u64, light: Sun) -> IndirectVolume {
    let mut v = IndirectVolume::new([-3, -1, -3], [7, 5, 7], 64, 16., palette()).unwrap();
    for _ in 0..1000 {
        let s = v
            .update(
                world,
                epoch,
                light,
                UpdateBudget {
                    // Deliberately differ from the worker's slice size.
                    rays: 257,
                    work: 193,
                },
            )
            .unwrap();
        if s.complete {
            return v;
        }
    }
    panic!("bounded reference fixture never completed");
}

fn assert_bounded(controller: &AsyncIndirectLight) {
    let stats = controller.stats();
    assert!(stats.queued <= 1, "pending bound exceeded: {stats:?}");
    assert!(stats.inflight <= 1, "running bound exceeded: {stats:?}");
    assert!(stats.results <= 1, "result bound exceeded: {stats:?}");
}

/// Wait without polling: callers can exercise rejection/reset with a result
/// actually buffered, rather than accidentally testing an already empty slot.
fn wait_for_buffered(controller: &AsyncIndirectLight) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        assert_bounded(controller);
        if controller.stats().results == 1 {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "controller never buffered a result"
        );
        assert!(controller.available(), "worker exited before buffering");
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Bounded wait that CONSUMES the matching result. Use wait_for_buffered when
/// the next assertion needs an occupied result slot.
fn wait_for_result(
    controller: &mut AsyncIndirectLight,
    world: &World,
    epoch: u64,
    light: Sun,
) -> Result<IndirectVolume, String> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        assert_bounded(controller);
        if let Some(result) = controller.poll(world, epoch, light) {
            return result;
        }
        assert!(Instant::now() < deadline, "controller never delivered");
        assert!(
            controller.available(),
            "worker exited before delivering a result"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn assert_radiance_eq(volume: &IndirectVolume, reference: &IndirectVolume) {
    for x in -3..=3 {
        for y in -1..=3 {
            for z in -3..=3 {
                for face in 0..6 {
                    assert_eq!(
                        volume.sample([x, y, z], face),
                        reference.sample([x, y, z], face),
                        "cell [{x},{y},{z}] face {face} radiance diverges from reference"
                    );
                }
            }
        }
    }
}

#[test]
fn async_result_equals_synchronous_reference_radiance() {
    for closed in [true, false] {
        let world = room(closed);
        let reference = sync_reference(&world, 5, sun());
        let mut controller = AsyncIndirectLight::new(config());
        assert!(controller.request(&world, 5, sun()).unwrap());
        let volume = wait_for_result(&mut controller, &world, 5, sun()).unwrap();
        assert!(volume.valid_for(&world, 5, sun()));
        assert_radiance_eq(&volume, &reference);
        let bounce = volume.sample([-3, 1, 0], 0);
        if closed {
            assert_eq!(bounce, [0.; 3], "sealed room has no inward bounce");
        } else {
            assert!(bounce[0] > 0.03, "open room must have bounce: {bounce:?}");
            assert!(
                bounce[0] > bounce[1] && bounce[1] > bounce[2] && bounce[2] > 0.,
                "red floor must produce nonzero colored bounce: {bounce:?}"
            );
        }
        assert!(
            controller.poll(&world, 5, sun()).is_none(),
            "result is consumed once"
        );
    }
}

#[test]
fn edit_invalidates_result_and_republishes_new_radiance() {
    let mut world = room(true);
    let before = world.clone();
    let mut controller = AsyncIndirectLight::new(config());
    assert!(controller.request(&world, 5, sun()).unwrap());
    let first = wait_for_result(&mut controller, &world, 5, sun()).unwrap();
    assert_eq!(
        first.sample([-3, 1, 0], 0),
        [0.; 3],
        "sealed room has no inward bounce"
    );
    // Edit after consuming the first result: its source validity must change.
    for x in -3..=3 {
        for z in -3..=3 {
            world.set([x, 3, z], 0);
        }
    }
    assert!(
        controller.poll(&world, 5, sun()).is_none(),
        "a poll after an edit must not return the pre-edit result"
    );
    assert!(!first.valid_for(&world, 5, sun()));
    assert!(first.valid_for(&before, 5, sun()));
    assert!(controller.request(&world, 5, sun()).unwrap());
    let second = wait_for_result(&mut controller, &world, 5, sun()).unwrap();
    let reference = sync_reference(&world, 5, sun());
    assert!(
        second.sample([-3, 1, 0], 0)[0] > 0.03,
        "opened roof must illuminate the inward wall: {:?}",
        second.sample([-3, 1, 0], 0)
    );
    assert!(second.valid_for(&world, 5, sun()));
    assert_radiance_eq(&second, &reference);
    // The stale pre-edit result may not reappear over the newer one.
    assert!(controller.poll(&before, 5, sun()).is_none());
}

#[test]
fn replacement_epoch_mismatch_prevents_publication() {
    let world = room(true);
    let mut controller = AsyncIndirectLight::new(config());
    assert!(controller.request(&world, 5, sun()).unwrap());
    wait_for_buffered(&controller);
    // Equal revisions still require a fresh epoch on instance replacement.
    let replacement = world.clone();
    assert_eq!(world.revision(), replacement.revision());
    assert!(controller.poll(&replacement, 6, sun()).is_none());
    let volume = controller
        .poll(&world, 5, sun())
        .expect("foreign poll preserves result")
        .unwrap();
    assert!(volume.valid_for(&world, 5, sun()));
    assert!(!volume.valid_for(&replacement, 6, sun()));
    assert!(controller.request(&replacement, 6, sun()).unwrap());
    let replacement_volume = wait_for_result(&mut controller, &replacement, 6, sun()).unwrap();
    assert!(replacement_volume.valid_for(&replacement, 6, sun()));
    assert!(!replacement_volume.valid_for(&world, 5, sun()));
}

#[test]
fn buffered_result_rejects_a_different_world_revision_without_being_consumed() {
    let world = room(false);
    let mut edited = world.clone();
    edited.set([0, 0, 0], 2);
    assert_ne!(world.revision(), edited.revision());
    let mut controller = AsyncIndirectLight::new(config());
    assert!(controller.request(&world, 5, sun()).unwrap());
    wait_for_buffered(&controller);
    assert!(controller.poll(&edited, 5, sun()).is_none());
    let volume = controller
        .poll(&world, 5, sun())
        .expect("foreign revision must leave the result buffered")
        .unwrap();
    assert!(volume.valid_for(&world, 5, sun()));
    assert!(!volume.valid_for(&edited, 5, sun()));
}

#[test]
fn changed_sun_rejects_buffered_radiance_and_prepares_the_new_light() {
    let world = room(false);
    let mut controller = AsyncIndirectLight::new(config());
    let original = sync_reference(&world, 5, sun());
    for changed in [
        Sun {
            intensity: 0.5,
            ..sun()
        },
        Sun {
            direction_to_sun: [0., -1., 0.],
            ..sun()
        },
    ] {
        assert!(controller.request(&world, 5, sun()).unwrap());
        wait_for_buffered(&controller);
        assert!(controller.poll(&world, 5, changed).is_none());
        let old = controller
            .poll(&world, 5, sun())
            .expect("foreign sun must not consume the buffered result")
            .unwrap();
        assert!(!old.valid_for(&world, 5, changed));
        assert!(controller.request(&world, 5, changed).unwrap());
        let volume = wait_for_result(&mut controller, &world, 5, changed).unwrap();
        assert!(volume.valid_for(&world, 5, changed));
        assert!(!volume.valid_for(&world, 5, sun()));
        assert_radiance_eq(&volume, &sync_reference(&world, 5, changed));
        assert_ne!(
            volume.sample([-3, 1, 0], 0),
            original.sample([-3, 1, 0], 0),
            "valid sun changes must change actual bounce, not only its key"
        );
    }
}

#[test]
fn invalid_sun_is_rejected_before_displacing_queued_work() {
    let world = room(true);
    let mut controller = AsyncIndirectLight::new(config());
    assert!(controller.request(&world, 5, sun()).unwrap());
    assert_eq!(
        controller
            .request(&world, 5, bad_sun())
            .expect_err("invalid sun must be rejected"),
        "Indirect sun must be finite, nonzero, intensity in 0..=16"
    );
    // The worker may have adopted or finished the job; completion, not an
    // exact pending count, proves the invalid request did not displace it.
    assert_bounded(&controller);
    let volume = wait_for_result(&mut controller, &world, 5, sun()).unwrap();
    assert!(volume.valid_for(&world, 5, sun()));
}

#[test]
fn invalid_sun_does_not_erase_buffered_result() {
    let world = room(true);
    let mut controller = AsyncIndirectLight::new(config());
    assert!(controller.request(&world, 5, sun()).unwrap());
    wait_for_buffered(&controller);
    assert!(controller.request(&world, 5, bad_sun()).is_err());
    assert!(controller.poll(&world, 5, bad_sun()).is_none());
    let volume = controller
        .poll(&world, 5, sun())
        .expect("an invalid sun request must not clear the buffered result")
        .unwrap();
    assert!(volume.valid_for(&world, 5, sun()));
}

#[test]
fn duplicate_request_is_refused_without_requeuing() {
    let world = room(true);
    let mut controller = AsyncIndirectLight::new(config());
    assert!(controller.request(&world, 5, sun()).unwrap());
    assert!(
        !controller.request(&world, 5, sun()).unwrap(),
        "an unchanged source must not be queued twice"
    );
    assert_bounded(&controller);
    assert_eq!(controller.stats().discarded, 0);
    let volume = wait_for_result(&mut controller, &world, 5, sun()).unwrap();
    assert!(volume.valid_for(&world, 5, sun()));
    assert_eq!(
        controller.stats().completed,
        1,
        "duplicate request must not trigger a second preparation"
    );
}

#[test]
fn latest_request_supersedes_work_regardless_of_worker_progress() {
    let mut world = room(true);
    let before = world.clone();
    let mut controller = AsyncIndirectLight::new(config());
    assert!(controller.request(&world, 5, sun()).unwrap());
    // Replace the source while A may be running, pending or even done: the
    // latest key must win in every case and only B may ever be delivered.
    for x in -3..=3 {
        for z in -3..=3 {
            world.set([x, 3, z], 0);
        }
    }
    assert!(controller.request(&world, 5, sun()).unwrap());
    let volume = wait_for_result(&mut controller, &world, 5, sun()).unwrap();
    let reference = sync_reference(&world, 5, sun());
    assert!(volume.valid_for(&world, 5, sun()));
    assert!(!volume.valid_for(&before, 5, sun()));
    assert!(volume.sample([-3, 1, 0], 0)[0] > 0.03);
    assert_radiance_eq(&volume, &reference);
    assert!(
        controller.poll(&before, 5, sun()).is_none(),
        "the superseded pre-edit result must never be published"
    );
}

#[test]
fn reset_invalidates_result_and_accepts_the_same_key_again() {
    let world = room(true);
    let mut controller = AsyncIndirectLight::new(config());
    assert!(controller.request(&world, 5, sun()).unwrap());
    wait_for_buffered(&controller);
    controller.reset();
    assert!(
        controller.poll(&world, 5, sun()).is_none(),
        "reset must drop the buffered result"
    );
    assert!(
        controller.request(&world, 5, sun()).unwrap(),
        "the same key must be accepted after reset, not deduplicated"
    );
    let volume = wait_for_result(&mut controller, &world, 5, sun()).unwrap();
    assert!(volume.valid_for(&world, 5, sun()));
    assert!(controller.stats().generation >= 1);
}

#[test]
fn drop_with_outstanding_work_returns_without_hanging() {
    let world = room(true);
    let mut controller = AsyncIndirectLight::new(config());
    assert!(controller.request(&world, 5, sun()).unwrap());
    let (done, finished) = std::sync::mpsc::channel();
    let dropper = std::thread::spawn(move || {
        drop(controller);
        done.send(()).expect("receiver alive");
    });
    finished
        .recv_timeout(Duration::from_secs(10))
        .expect("drop must join the worker, bounded by one update slice");
    dropper.join().expect("drop thread must not panic");
}

// --- Deterministic private queue-state tests ---
// Error strings serve as cheap, distinguishable completed-outcome markers.
// Real successful radiance preparation is exercised by the worker tests above.

fn key(tag: u64) -> SourceKey {
    SourceKey {
        epoch: tag,
        revision: tag,
        sun: [0., 1., 0., 1.],
    }
}

#[test]
fn fresh_request_is_accepted_after_reset_while_the_old_build_runs() {
    let mut q = Queue::default();
    assert!(q.enqueue_request(key(0), sun(), || World::new(0)));
    let running = q.adopt_job().expect("job adopted");
    // Reset cancels; the old build keeps slicing under its old generation.
    q.cancel();
    let forked = Cell::new(false);
    let world = World::new(0);
    let queued = q.enqueue_request(key(0), sun(), || {
        forked.set(true);
        world.clone()
    });
    assert!(
        queued,
        "a fresh request after reset must be accepted, not deduped against the retired build"
    );
    assert!(forked.get(), "fresh generation needs a new snapshot");
    assert!(q.pending.is_some());
    assert!(!Arc::ptr_eq(&running.generation, &q.generation));
    // The retired build's completion must never buffer over the new work.
    q.finish_job(running, Err("retired".into()));
    assert!(q.result.is_none(), "retired job must not buffer a result");
    let fresh = q
        .adopt_job()
        .expect("same-key job survives retired completion");
    assert!(Arc::ptr_eq(&fresh.generation, &q.generation));
    q.finish_job(fresh, Err("fresh".into()));
    assert!(matches!(q.take_result(&key(0)), Some(Err(e)) if e == "fresh"));
}

#[test]
fn requesting_the_running_source_drops_a_superseded_pending() {
    let mut q = Queue::default();
    assert!(q.enqueue_request(key(0), sun(), || World::new(0)));
    let running = q.adopt_job().expect("A adopted");
    assert!(q.enqueue_request(key(1), sun(), || World::new(1)));
    assert!(q.pending.is_some());
    // The latest request is A (already running): B must be dropped and no
    // new build is queued.
    assert!(!q.enqueue_request(key(0), sun(), || panic!("running A must not fork")));
    assert!(
        q.pending.is_none(),
        "the superseded pending B must be dropped so B is not published over A"
    );
    assert!(
        !q.retire_stale_running(),
        "A is the latest requested source again"
    );
    q.finish_job(running, Err("A".into()));
    assert!(matches!(q.take_result(&key(0)), Some(Err(e)) if e == "A"));
    assert!(q.take_result(&key(1)).is_none());
}

#[test]
fn returning_to_buffered_source_rejects_newer_inflight_result() {
    let mut q = Queue::default();
    q.enqueue_request(key(0), sun(), || World::new(0));
    let first = q.adopt_job().unwrap();
    q.finish_job(first, Err("A".into()));
    q.enqueue_request(key(1), sun(), || World::new(1));
    let second = q.adopt_job().unwrap();
    assert!(!q.enqueue_request(key(0), sun(), || panic!("buffered A must not fork")));
    q.finish_job(second, Err("B".into()));
    assert!(
        matches!(q.take_result(&key(0)), Some(Err(e)) if e == "A"),
        "the buffered A result must survive the abandoned B build"
    );
    assert!(q.take_result(&key(1)).is_none());
}

#[test]
fn latest_distinct_request_supersedes_pending() {
    let mut q = Queue::default();
    assert!(q.enqueue_request(key(0), sun(), || World::new(0)));
    let running = q.adopt_job().unwrap();
    assert!(q.enqueue_request(key(1), sun(), || World::new(1)));
    assert!(q.enqueue_request(key(2), sun(), || World::new(2)));
    assert_eq!(
        q.pending.as_ref().expect("pending").key,
        key(2),
        "the newest distinct source wins the single pending slot"
    );
    q.finish_job(running, Err("superseded A".into()));
    assert!(q.result.is_none());
    let latest = q.adopt_job().expect("latest C must remain adoptable");
    assert_eq!(latest.key, key(2));
    q.finish_job(latest, Err("C".into()));
    assert!(q.take_result(&key(0)).is_none());
    assert!(q.take_result(&key(1)).is_none());
    assert!(matches!(q.take_result(&key(2)), Some(Err(e)) if e == "C"));
}

#[test]
fn duplicate_pending_request_is_refused_without_forking() {
    let mut q = Queue::default();
    assert!(q.enqueue_request(key(0), sun(), || World::new(0)));
    let forked = Cell::new(false);
    let world = World::new(0);
    assert!(!q.enqueue_request(key(0), sun(), || {
        forked.set(true);
        world.clone()
    }));
    assert!(
        !forked.get(),
        "a duplicate request must not clone a world snapshot"
    );
}

#[test]
fn duplicate_running_and_buffered_requests_do_not_fork_or_build_again() {
    let mut q = Queue::default();
    assert!(q.enqueue_request(key(0), sun(), || World::new(0)));
    let job = q.adopt_job().unwrap();
    assert_eq!(q.inflight, 1);
    assert!(q.pending.is_none());
    assert!(!q.enqueue_request(key(0), sun(), || panic!("running duplicate forked")));
    assert!(!q.retire_stale_running());
    q.finish_job(job, Err("prepared".into()));
    assert_eq!(q.inflight, 0);
    assert!(!q.enqueue_request(key(0), sun(), || panic!("buffered duplicate forked")));
    assert!(q.pending.is_none());
    assert_eq!(q.completed, 1);
    assert_eq!(q.discarded, 0);
    assert!(matches!(q.take_result(&key(0)), Some(Err(e)) if e == "prepared"));
    assert!(
        q.take_result(&key(0)).is_none(),
        "result must be delivered only once"
    );
}

#[test]
fn reset_clears_pending_and_buffered_work() {
    let mut q = Queue::default();
    assert!(q.enqueue_request(key(0), sun(), || World::new(0)));
    let job = q.adopt_job().unwrap();
    q.finish_job(job, Err("A".into()));
    assert!(q.enqueue_request(key(1), sun(), || World::new(1)));
    assert!(q.result.is_some());
    assert!(q.pending.is_some());
    q.cancel();
    assert!(q.pending.is_none());
    assert!(q.result.is_none());
    assert!(q.take_result(&key(0)).is_none());
    assert!(q.adopt_job().is_none());
    assert!(q.enqueue_request(key(1), sun(), || World::new(1)));
    let fresh = q.adopt_job().unwrap();
    q.finish_job(fresh, Err("fresh B".into()));
    assert!(matches!(q.take_result(&key(1)), Some(Err(e)) if e == "fresh B"));
}

#[test]
fn current_completion_replaces_the_single_buffered_slot() {
    let mut q = Queue::default();
    assert!(q.enqueue_request(key(0), sun(), || World::new(0)));
    let first = q.adopt_job().unwrap();
    q.finish_job(first, Err("A".into()));
    assert!(q.enqueue_request(key(1), sun(), || World::new(1)));
    let second = q.adopt_job().unwrap();
    q.finish_job(second, Err("B".into()));
    assert_eq!(q.completed, 2);
    assert_eq!(q.discarded, 1);
    assert!(q.take_result(&key(0)).is_none());
    assert!(matches!(q.take_result(&key(1)), Some(Err(e)) if e == "B"));
    assert!(q.result.is_none());
}

#[test]
fn retire_stale_running_clears_the_slot_for_the_latest_request() {
    let mut q = Queue::default();
    assert!(q.enqueue_request(key(0), sun(), || World::new(0)));
    let stale = q.adopt_job().unwrap();
    assert!(q.enqueue_request(key(1), sun(), || World::new(1)));
    assert!(
        q.retire_stale_running(),
        "superseded running job is retired"
    );
    assert!(q.running.is_none());
    assert_eq!(q.inflight, 0);
    assert_eq!(q.discarded, 1);
    // The worker adopts the newest request next; the stale job's finish must
    // not buffer anything.
    q.finish_job(stale, Err("stale".into()));
    assert!(q.result.is_none());
    let next = q
        .adopt_job()
        .expect("latest request adoptable after retire");
    assert_eq!(next.key, key(1));
}

#[test]
fn stale_result_is_not_erased_by_a_foreign_poll() {
    let mut q = Queue::default();
    q.enqueue_request(key(1), sun(), || World::new(1));
    let job = q.adopt_job().unwrap();
    q.finish_job(job, Err("B".into()));
    assert!(q.take_result(&key(0)).is_none());
    assert!(
        q.result.is_some(),
        "polling with a stale key must not erase the newer buffered result"
    );
    assert!(matches!(q.take_result(&key(1)), Some(Err(e)) if e == "B"));
    assert!(q.take_result(&key(1)).is_none());
}

#[test]
fn reset_work_never_validates_after_generation_saturates() {
    let a = key(0);
    // Push the display counter to the wrap boundary.
    let mut q = Queue {
        resets: u64::MAX,
        ..Queue::default()
    };
    assert!(q.enqueue_request(a, sun(), || World::new(0)));
    let job = q.adopt_job().unwrap();
    // Reset while the job is in flight.
    let generation = Arc::clone(&job.generation);
    q.cancel();
    assert_eq!(q.resets, u64::MAX, "display counter saturates");
    assert!(!Arc::ptr_eq(&generation, &q.generation));
    // The worker finishes and tries to buffer its now-retired result.
    q.finish_job(job, Err("built".into()));
    assert!(
        q.result.is_none(),
        "a job retired by reset must never buffer a result, even when the display generation saturated"
    );
}

#[test]
fn shutdown_refuses_new_snapshots_and_retires_running_work() {
    let mut q = Queue::default();
    assert!(q.enqueue_request(key(0), sun(), || World::new(0)));
    let _job = q.adopt_job().unwrap();
    q.shutdown = true;
    assert!(!q.enqueue_request(key(1), sun(), || panic!("shutdown must not fork")));
    assert!(q.pending.is_none());
    assert!(q.retire_stale_running());
    assert!(q.running.is_none());
    assert_eq!(q.inflight, 0);
    assert!(q.result.is_none());
}

#[test]
fn finish_after_shutdown_does_not_buffer() {
    let mut q = Queue::default();
    assert!(q.enqueue_request(key(0), sun(), || World::new(0)));
    let job = q.adopt_job().unwrap();
    q.shutdown = true;
    q.finish_job(job, Err("built".into()));
    assert!(q.result.is_none(), "shutdown must discard late results");
}
