//! Bounded background preparation of detail collision shapes.
//!
//! The authoritative [`DetailScene`] stays owned by the simulation and single
//! threaded. This controller runs [`PreparedDetailCollision::build`] on an
//! immutable source snapshot on one worker thread, then hands the finished
//! shapes back to the simulation owner, which alone mutates the live `Physics`
//! world via [`Physics::publish_detail_scene`]. Preparation never touches a live
//! world, so background work can never delete a wall, publish shapes for a scene
//! the caller has since edited, or resurrect a superseded state.
//!
//! Reuse: this mirrors the bounded-queue / snapshot-revalidation shape of
//! `matterweave_core::AsyncWorld` (one worker, `Condvar` wake, a poll that
//! revalidates against the live authoritative state) rather than adding a
//! generic job framework. The differences are inherent to this workload: the
//! job identity is the scene's opaque [`SceneVersion`] rather than a chunk key,
//! and there is a single pending/running/completed slot instead of a chunk
//! queue, because the whole detail scene is replaced as one unit.
//!
//! Bounds (all fixed, no growth):
//! - at most one *pending* source snapshot (the latest request wins);
//! - at most one job *running* on the worker;
//! - at most one *completed* result buffered for [`AsyncDetailCollision::poll`].
//!
//! The worker never blocks on a full result slot: it replaces the single slot
//! and records the drop, so shutdown is always observable after one bounded job
//! and the join in [`Drop`] cannot deadlock. No thread is ever detached.

use crate::PreparedDetailCollision;
use matterweave_detail::{DetailScene, SceneVersion};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;

/// Live counts for debugging and integration; not a stable metrics contract.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AsyncDetailStats {
    /// Pending source snapshots awaiting the worker: at most one.
    pub queued: usize,
    /// Jobs currently executing on the worker: at most one.
    pub inflight: usize,
    /// Prepared results awaiting `poll`: at most one.
    pub results: usize,
    /// Results dropped by supersession, cancellation, reset or a stale poll.
    pub discarded: u64,
    /// Advances on every [`AsyncDetailCollision::reset`].
    pub generation: u64,
}

/// One queued source snapshot to prepare.
struct Job {
    scene: DetailScene,
    version: SceneVersion,
    epoch: Arc<()>,
}

/// Identity of the job currently running on the worker.
struct Running {
    version: SceneVersion,
    epoch: Arc<()>,
}

/// One completed preparation, tagged with the identity it was built from.
struct Prepared {
    outcome: Result<PreparedDetailCollision, String>,
    version: SceneVersion,
    epoch: Arc<()>,
}

#[derive(Default)]
struct Queue {
    epoch: Arc<()>,
    requested: Option<SceneVersion>,
    /// Latest requested source not yet started. Replacing it supersedes.
    pending: Option<Job>,
    /// The job currently running, for deduplication.
    running: Option<Running>,
    /// Single completed slot.
    result: Option<Prepared>,
    inflight: usize,
    shutdown: bool,
    generation: u64,
    discarded: u64,
}

impl Queue {
    /// Applies a request for `version`, honoring latest-request-wins and dedup.
    /// Returns whether a new build job was enqueued; `fork` is called only then.
    fn enqueue_request(
        &mut self,
        version: SceneVersion,
        fork: impl FnOnce() -> DetailScene,
    ) -> bool {
        self.requested = Some(version.clone());
        if self
            .pending
            .as_ref()
            .is_some_and(|job| job.version == version)
        {
            return false;
        }
        let already = self
            .running
            .as_ref()
            .is_some_and(|r| Arc::ptr_eq(&r.epoch, &self.epoch) && r.version == version)
            || self
                .result
                .as_ref()
                .is_some_and(|r| Arc::ptr_eq(&r.epoch, &self.epoch) && r.version == version);
        if already {
            self.discarded = self
                .discarded
                .saturating_add(u64::from(self.pending.take().is_some()));
            return false;
        }
        let superseded = self
            .pending
            .replace(Job {
                scene: fork(),
                version,
                epoch: self.epoch.clone(),
            })
            .is_some();
        self.discarded = self.discarded.saturating_add(u64::from(superseded));
        true
    }

    /// Worker: take the pending job and mark it running.
    fn begin_job(&mut self) -> Job {
        let job = self.pending.take().expect("nonempty pending queue");
        self.running = Some(Running {
            version: job.version.clone(),
            epoch: job.epoch.clone(),
        });
        self.inflight = 1;
        job
    }

    /// Worker: buffer the built result if it is still current, else discard it.
    fn finish_job(&mut self, job: Job, outcome: Result<PreparedDetailCollision, String>) {
        self.inflight = 0;
        self.running = None;
        if self.shutdown
            || !Arc::ptr_eq(&self.epoch, &job.epoch)
            || self.requested.as_ref() != Some(&job.version)
        {
            self.discarded = self.discarded.saturating_add(1);
            return;
        }
        let superseded = self
            .result
            .replace(Prepared {
                outcome,
                version: job.version,
                epoch: job.epoch.clone(),
            })
            .is_some();
        self.discarded = self.discarded.saturating_add(u64::from(superseded));
    }

    /// Poll: consume and return the buffered result only if it is current for
    /// `version` and `gate` accepts it; drop a reset-cancelled result; leave a
    /// foreign-source or gate-refused result buffered for a later retry.
    fn take_result(
        &mut self,
        version: &SceneVersion,
        gate: impl FnOnce(&Result<PreparedDetailCollision, String>) -> bool,
    ) -> Option<Result<PreparedDetailCollision, String>> {
        let accept = match self.result.as_ref() {
            None => return None,
            Some(result) if !Arc::ptr_eq(&result.epoch, &self.epoch) => {
                self.result = None;
                self.discarded = self.discarded.saturating_add(1);
                return None;
            }
            Some(result)
                if result.version != *version || self.requested.as_ref() != Some(version) =>
            {
                return None
            }
            Some(result) => gate(&result.outcome),
        };
        accept.then(|| self.result.take().expect("result present").outcome)
    }

    /// Reset: cancel pending and buffered work and advance the generation.
    fn cancel(&mut self) {
        let dropped = usize::from(self.pending.is_some()) + usize::from(self.result.is_some());
        self.discarded = self.discarded.saturating_add(dropped as u64);
        self.pending = None;
        self.result = None;
        self.generation = self.generation.saturating_add(1);
        self.epoch = Arc::new(());
        self.requested = None;
    }

    #[cfg(test)]
    fn set_generation(&mut self, generation: u64) {
        self.generation = generation;
    }
}

struct Shared {
    queue: Mutex<Queue>,
    wake: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Queue> {
        // A panicking job must not disable the remaining synchronous engine.
        self.queue.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Single-worker asynchronous detail-collision preparation controller.
///
/// Typical simulation-loop use: call [`request`](Self::request) whenever the
/// authoritative scene changes, then each frame call [`poll`](Self::poll) with
/// the current scene and, on a returned `Ok`, hand it to
/// [`Physics::publish_detail_scene`]. Old live collision stays intact until a
/// publication succeeds.
pub struct AsyncDetailCollision {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
}

impl Default for AsyncDetailCollision {
    fn default() -> Self {
        Self::new()
    }
}

impl AsyncDetailCollision {
    /// Fallback-path constructor: no worker thread is spawned and every
    /// request is refused, so the caller exercises the synchronous path.
    /// Used by [`DetailCollisionCadence::without_worker`] for fallback tests.
    pub fn without_worker() -> Self {
        let shared = Arc::new(Shared {
            queue: Mutex::new(Queue {
                shutdown: true,
                ..Queue::default()
            }),
            wake: Condvar::new(),
        });
        Self {
            shared,
            worker: None,
        }
    }

    /// Starts the single background worker. One worker keeps ordering obvious;
    /// widening it is a measurement-led change, not a correctness requirement.
    pub fn new() -> Self {
        let shared = Arc::new(Shared {
            queue: Mutex::new(Queue::default()),
            wake: Condvar::new(),
        });
        let worker = std::thread::Builder::new()
            .name("matterweave-detail-collision".into())
            .spawn({
                let shared = Arc::clone(&shared);
                move || run(&shared)
            })
            .ok();
        if worker.is_none() {
            // Without a worker every request is refused; the caller keeps the
            // synchronous replace_detail_scene path instead of queuing work
            // nobody will execute.
            shared.lock().shutdown = true;
        }
        Self { shared, worker }
    }

    /// Requests preparation from an immutable snapshot of `scene`. Returns
    /// whether a job was queued. Shapes are never built on the caller: only a
    /// bounded [`DetailScene::fork_source`] copy is taken here.
    ///
    /// The latest request wins: a newer source replaces a pending older one
    /// (counted as discarded). A source whose opaque identity already matches
    /// the pending, running or buffered work is deduplicated and refused, so an
    /// unchanged scene is not prepared twice. Requests are refused after worker
    /// startup failure or exit.
    pub fn request(&mut self, scene: &DetailScene) -> bool {
        if !self.available() {
            return false;
        }
        let version = scene.source_version();
        let mut queue = self.shared.lock();
        if queue.shutdown {
            return false;
        }
        // Clone the bounded authoritative source under the lock only when a job
        // is actually enqueued. Only the worker could otherwise touch the queue.
        let queued = queue.enqueue_request(version, || scene.fork_source());
        drop(queue);
        if queued {
            self.shared.wake.notify_all();
        }
        queued
    }

    /// Returns at most one completed preparation that is *current* for `current`.
    ///
    /// A cancelled (reset) result is dropped here; a result for a *different*
    /// source is left buffered and `None` is returned, so polling with the wrong
    /// scene is harmless and an error produced for some other (already-replaced)
    /// source can never reject the current one. On `Some(Ok(prepared))` the
    /// simulation owner publishes via [`Physics::publish_detail_scene`]; on
    /// `Some(Err(_))` the current source genuinely failed preparation.
    /// Nonblocking.
    pub fn poll(
        &mut self,
        current: &DetailScene,
    ) -> Option<Result<PreparedDetailCollision, String>> {
        self.poll_retaining(current, |_| true)
    }

    /// Like [`poll`](Self::poll), but *retains* the buffered result when `gate`
    /// refuses it, so a publication the live world cannot safely accept yet —
    /// for example new collision overlapping a body that moved into its region
    /// while preparation was pending — stays buffered and is retried on a later
    /// frame instead of being dropped or forced through.
    pub fn poll_retaining(
        &mut self,
        current: &DetailScene,
        gate: impl FnOnce(&Result<PreparedDetailCollision, String>) -> bool,
    ) -> Option<Result<PreparedDetailCollision, String>> {
        let version = current.source_version();
        let mut queue = self.shared.lock();
        queue.take_result(&version, gate)
    }

    /// Cancels the pending snapshot and any buffered result, and invalidates the
    /// job in flight. Use on scene replacement or load. A fresh
    /// [`request`](Self::request) is accepted immediately after.
    pub fn reset(&mut self) {
        let mut queue = self.shared.lock();
        queue.cancel();
    }

    /// False after worker startup failure or an unexpected worker exit.
    pub fn available(&self) -> bool {
        self.worker
            .as_ref()
            .is_some_and(|worker| !worker.is_finished())
    }

    pub fn stats(&self) -> AsyncDetailStats {
        let queue = self.shared.lock();
        AsyncDetailStats {
            queued: usize::from(queue.pending.is_some()),
            inflight: queue.inflight,
            results: usize::from(queue.result.is_some()),
            discarded: queue.discarded,
            generation: queue.generation,
        }
    }
}

impl Drop for AsyncDetailCollision {
    fn drop(&mut self) {
        let mut queue = self.shared.lock();
        queue.shutdown = true;
        queue.pending = None;
        drop(queue);
        self.shared.wake.notify_all();
        // The worker never blocks on the result slot, so it observes shutdown
        // after at most one bounded job and this join cannot deadlock.
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn run(shared: &Shared) {
    loop {
        let mut queue = shared.lock();
        while !queue.shutdown && queue.pending.is_none() {
            queue = shared
                .wake
                .wait(queue)
                .unwrap_or_else(PoisonError::into_inner);
        }
        if queue.shutdown {
            return;
        }
        let job = queue.begin_job();
        drop(queue);

        // Build shapes off the simulation thread. This may fail (over budget);
        // the error is carried back tagged with the source identity.
        let outcome = PreparedDetailCollision::build(&job.scene);

        let mut queue = shared.lock();
        queue.finish_job(job, outcome);
    }
}

#[cfg(test)]
mod queue_tests {
    use super::*;
    use matterweave_detail::{material, DetailScene, DetailVolume, Scale, Transform, Yaw};

    /// A distinct one-cell floor scene per `tag`, each with its own opaque
    /// source identity even though their revision counters match.
    fn scene(tag: i32) -> DetailScene {
        let mut volume = DetailVolume::new("floor", Scale::new(0.25).expect("scale"));
        volume.set([0, 0, 0], material::BANK_STONE).expect("cell");
        let mut scene = DetailScene::new();
        scene.add_prototype(volume).expect("prototype");
        scene
            .place(
                "floor.0",
                "floor",
                Transform::new([tag as f32, 0.0, 0.0], Yaw::Deg0).expect("transform"),
            )
            .expect("placement");
        scene
    }

    #[test]
    fn fresh_request_is_accepted_after_reset_while_the_old_build_runs() {
        let a = scene(0);
        let mut q = Queue::default();
        assert!(q.enqueue_request(a.source_version(), || a.fork_source()));
        let _running = q.begin_job();
        // Reset cancels; the old build keeps running under its old generation.
        q.cancel();
        // The now-authoritative request for the SAME scene must be accepted.
        let queued = q.enqueue_request(a.source_version(), || a.fork_source());
        assert!(
            queued,
            "a fresh request after reset must be accepted, not deduped against the retired build"
        );
        assert!(q.pending.is_some());
    }

    #[test]
    fn requesting_the_running_source_drops_a_superseded_pending() {
        let a = scene(0);
        let b = scene(1);
        let mut q = Queue::default();
        assert!(q.enqueue_request(a.source_version(), || a.fork_source()));
        let _running = q.begin_job();
        // B is queued behind the running A.
        assert!(q.enqueue_request(b.source_version(), || b.fork_source()));
        assert!(q.pending.is_some());
        // The latest request is A (already running): B must be dropped and no
        // new build is queued.
        let queued = q.enqueue_request(a.source_version(), || a.fork_source());
        assert!(!queued, "A is already running; no new build is queued");
        assert!(
            q.pending.is_none(),
            "the superseded pending B must be dropped so B is not published over A"
        );
    }

    #[test]
    fn latest_distinct_request_supersedes_pending() {
        let a = scene(0);
        let b = scene(1);
        let c = scene(2);
        let mut q = Queue::default();
        assert!(q.enqueue_request(a.source_version(), || a.fork_source()));
        let _running = q.begin_job();
        assert!(q.enqueue_request(b.source_version(), || b.fork_source()));
        assert!(q.enqueue_request(c.source_version(), || c.fork_source()));
        assert!(
            q.pending.as_ref().expect("pending").version == c.source_version(),
            "the newest distinct source wins the single pending slot"
        );
    }

    #[test]
    fn duplicate_pending_request_is_refused() {
        let a = scene(0);
        let mut q = Queue::default();
        assert!(q.enqueue_request(a.source_version(), || a.fork_source()));
        assert!(
            !q.enqueue_request(a.source_version(), || a.fork_source()),
            "an unchanged pending source is not queued twice"
        );
    }

    #[test]
    fn returning_to_buffered_source_rejects_newer_inflight_result() {
        let a = scene(0);
        let b = scene(1);
        let mut q = Queue::default();
        q.enqueue_request(a.source_version(), || a.fork_source());
        let first = q.begin_job();
        q.finish_job(first, Err("A".into()));
        q.enqueue_request(b.source_version(), || b.fork_source());
        let second = q.begin_job();
        assert!(!q.enqueue_request(a.source_version(), || a.fork_source()));
        q.finish_job(second, Err("B".into()));
        assert!(matches!(q.take_result(&a.source_version(), |_| true), Some(Err(e)) if e == "A"));
    }

    #[test]
    fn reset_work_never_validates_after_generation_saturates() {
        let a = scene(0);
        let mut q = Queue::default();
        // Push the display counter to the wrap boundary.
        q.set_generation(u64::MAX);
        assert!(q.enqueue_request(a.source_version(), || a.fork_source()));
        let job = q.begin_job();
        // Reset while the job is in flight.
        q.cancel();
        // The worker finishes and tries to buffer its now-retired result.
        q.finish_job(job, Err("built".into()));
        assert!(
            q.result.is_none(),
            "a job retired by reset must never buffer a result, even when the display generation saturated"
        );
    }
}
