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
//! Reuse: this mirrors the bounded-queue / generation / snapshot-revalidation
//! shape of `matterweave_core::AsyncWorld` (one worker, `Condvar` wake, a poll
//! that revalidates against the live authoritative state) rather than adding a
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
//!
//! Memory: the retained cost is the source snapshots and the built shapes, not a
//! measured RSS. At most three source snapshots are live at once (pending,
//! running, and one transient clone while replacing a pending job), each a
//! [`DetailScene::fork_source`] copy bounded by the detail crate's
//! `MAX_SCENE_SOURCE_BYTES` (32 MiB) authoritative payload. At most one built
//! result is buffered, whose shape cost is bounded by the existing
//! [`crate::MAX_DETAIL_BOXES`] / [`crate::MAX_DETAIL_COLLIDERS`] caps enforced by
//! `build`. These are documented upper bounds, not device measurements.

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
    /// Advances on every [`AsyncDetailCollision::reset`]; invalidates in-flight
    /// and buffered work from an earlier generation.
    pub generation: u64,
}

/// One queued source snapshot to prepare.
struct Job {
    scene: DetailScene,
    version: SceneVersion,
    generation: u64,
}

/// One completed preparation, tagged with the identity it was built from.
struct Prepared {
    /// `Ok` shapes ready for publication, or the build error for this source.
    outcome: Result<PreparedDetailCollision, String>,
    version: SceneVersion,
    generation: u64,
}

#[derive(Default)]
struct Queue {
    /// Latest requested source not yet started. Replacing it supersedes.
    pending: Option<Job>,
    /// Identity of the job currently running, for deduplication.
    running: Option<SceneVersion>,
    /// Single completed slot.
    result: Option<Prepared>,
    inflight: usize,
    shutdown: bool,
    generation: u64,
    discarded: u64,
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
        let duplicate = queue.running.as_ref() == Some(&version)
            || queue
                .pending
                .as_ref()
                .is_some_and(|job| job.version == version)
            || queue
                .result
                .as_ref()
                .is_some_and(|res| res.version == version);
        if duplicate {
            return false;
        }
        let generation = queue.generation;
        // Clone the bounded authoritative source under the lock. Only the worker
        // could otherwise touch the queue, and the clone frees no invariants.
        let superseded = queue
            .pending
            .replace(Job {
                scene: scene.fork_source(),
                version,
                generation,
            })
            .is_some();
        queue.discarded += u64::from(superseded);
        drop(queue);
        self.shared.wake.notify_all();
        true
    }

    /// Returns at most one completed preparation that is *current* for `current`:
    /// its source identity still matches and it belongs to the live generation.
    ///
    /// A cancelled (reset) result is dropped here; a result for a *different*
    /// source is left buffered and `None` is returned, so polling with the wrong
    /// scene is harmless and an error produced for some other (already-replaced)
    /// source can never reject the current one. Only a result whose identity
    /// matches `current` is consumed and returned. On `Some(Ok(prepared))` the
    /// simulation owner publishes via [`Physics::publish_detail_scene`]; on
    /// `Some(Err(_))` the current source genuinely failed preparation.
    /// Nonblocking.
    pub fn poll(
        &mut self,
        current: &DetailScene,
    ) -> Option<Result<PreparedDetailCollision, String>> {
        let version = current.source_version();
        let mut queue = self.shared.lock();
        match queue.result.as_ref() {
            None => return None,
            Some(result) if result.generation != queue.generation => {
                // Cancelled by a reset: it can never become current again.
                queue.result = None;
                queue.discarded += 1;
                return None;
            }
            // Built for a different source: leave it buffered until its own
            // scene polls it or a newer result supersedes it.
            Some(result) if result.version != version => return None,
            Some(_) => {}
        }
        Some(queue.result.take().expect("result present").outcome)
    }

    /// Cancels the pending snapshot and any buffered result, and invalidates the
    /// job in flight by advancing the generation. Use on scene replacement or
    /// load. A fresh [`request`](Self::request) is accepted immediately after.
    pub fn reset(&mut self) {
        let mut queue = self.shared.lock();
        let dropped = usize::from(queue.pending.is_some())
            + usize::from(queue.result.is_some())
            + queue.inflight;
        queue.discarded += dropped as u64;
        queue.pending = None;
        queue.result = None;
        queue.generation = queue.generation.saturating_add(1);
        drop(queue);
        // Nothing waits on the worker here; the generation bump makes any
        // in-flight or later-arriving result from the old generation unpollable.
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
        let job = queue.pending.take().expect("nonempty pending queue");
        queue.running = Some(job.version.clone());
        queue.inflight = 1;
        drop(queue);

        // Build shapes off the simulation thread. This may fail (over budget);
        // the error is carried back tagged with the source identity.
        let outcome = PreparedDetailCollision::build(&job.scene);

        let mut queue = shared.lock();
        queue.inflight = 0;
        queue.running = None;
        if queue.shutdown || queue.generation != job.generation {
            // Cancelled or reset while running: never buffer the stale result.
            queue.discarded += 1;
            continue;
        }
        let superseded = queue
            .result
            .replace(Prepared {
                outcome,
                version: job.version,
                generation: job.generation,
            })
            .is_some();
        queue.discarded += u64::from(superseded);
    }
}
