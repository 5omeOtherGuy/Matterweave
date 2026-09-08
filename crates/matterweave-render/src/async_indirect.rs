//! Bounded background preparation of the CPU indirect-light volume.
//!
//! The authoritative [`World`] stays owned by the caller and single threaded.
//! This controller runs [`IndirectVolume::update`] on an immutable COW world
//! snapshot on one worker thread, then hands the completed, immutable volume
//! back to the render owner, which alone publishes it via
//! [`crate::Renderer::upload_indirect`]. Preparation never touches a live
//! world, so background rays can never observe an edit in progress or publish
//! radiance for a source the caller has since replaced or relit.
//!
//! Reuse: this mirrors the bounded-queue / snapshot-revalidation shape of the
//! corrected `matterweave_physics::AsyncDetailCollision` (one worker, one
//! `Condvar` wake, a poll that revalidates against the live authoritative
//! source) rather than adding a generic job framework. The job identity is the
//! source key (caller replacement epoch, world revision, validated sun) instead
//! of a scene version, because [`IndirectVolume`] validity is exactly that key.
//!
//! Caller replacement-epoch contract: the caller assigns a fresh
//! `replacement_epoch: u64` whenever it replaces the authoritative `World`
//! instance (load, reset, reseed), including replacements at equal revision.
//! The epoch is part of the source key, so a snapshot from the previous world
//! can never validate against the new one; `Renderer::upload_indirect`
//! re-checks the same pair. A reset additionally swaps an opaque `Arc`
//! generation token, so in-flight work is invalidated without any counter
//! that could wrap into accidental validity.
//!
//! Bounds (all fixed, no growth):
//! - at most one *pending* world snapshot (the latest request wins);
//! - at most one job *running* on the worker, sliced into bounded
//!   [`IndirectVolume::update`] calls that recheck shutdown, reset and the
//!   latest requested key between slices;
//! - at most one *completed* result buffered for [`AsyncIndirectLight::poll`].
//!
//! The worker never blocks on a full result slot: it replaces the single slot
//! and records the supersession, so shutdown is always observable after at
//! most one bounded update slice. Allocation and snapshot destruction are
//! additional costs; this is a work bound, not a wall-clock deadline. No
//! thread is ever detached and no per-request threads exist. A duplicate
//! request for the already-pending, running or buffered key is deduplicated
//! without cloning the world again. The result is never GPU-published
//! automatically; the render owner decides when and whether to upload.

use crate::indirect::{light_key, IndirectVolume, UpdateBudget};
use crate::Sun;
use matterweave_core::World;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;

/// Worker slice budget. Each iteration is bounded so shutdown and
/// latest-wins cancellation are observed after at most this much ray work.
const WORKER_BUDGET: UpdateBudget = UpdateBudget {
    rays: 4096,
    work: 4096,
};

/// Live counts for debugging and integration; not a stable metrics contract.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AsyncIndirectStats {
    /// Pending world snapshots awaiting the worker: at most one.
    pub queued: usize,
    /// Jobs currently executing on the worker: at most one.
    pub inflight: usize,
    /// Completed volumes awaiting `poll`: at most one.
    pub results: usize,
    /// Results dropped by supersession, cancellation, reset, stale polls or
    /// abandoned mid-flight slices.
    pub discarded: u64,
    /// Volumes fully prepared and buffered (before any later discard).
    pub completed: u64,
    /// Advances on every [`AsyncIndirectLight::reset`].
    pub generation: u64,
}

/// Fixed, validated volume configuration. One controller prepares exactly this
/// volume layout; validation happens once at construction by building a
/// throwaway [`IndirectVolume`], so every later worker allocation uses
/// identical, already-accepted parameters.
#[derive(Clone, Debug)]
pub struct AsyncIndirectConfig {
    origin: [i32; 3],
    dimensions: [u32; 3],
    samples: u32,
    distance: f32,
    palette: [[f32; 3]; 256],
}

impl AsyncIndirectConfig {
    /// Validates through the authoritative [`IndirectVolume::new`] checks
    /// (face residency cap, sample/range/palette limits, coordinate range).
    pub fn new(
        origin: [i32; 3],
        dimensions: [u32; 3],
        samples: u32,
        distance: f32,
        palette: [[f32; 3]; 256],
    ) -> Result<Self, String> {
        IndirectVolume::new(origin, dimensions, samples, distance, palette)?;
        Ok(Self {
            origin,
            dimensions,
            samples,
            distance,
            palette,
        })
    }

    fn build(&self) -> Result<IndirectVolume, String> {
        IndirectVolume::new(
            self.origin,
            self.dimensions,
            self.samples,
            self.distance,
            self.palette,
        )
    }
}

/// Identity of the desired lighting state: caller replacement epoch, world
/// revision and the normalized, validated sun. Equal keys mean equal radiance.
#[derive(Clone, Debug, PartialEq)]
struct SourceKey {
    epoch: u64,
    revision: u64,
    sun: [f32; 4],
}

fn source_key(world: &World, epoch: u64, sun: Sun) -> Option<SourceKey> {
    light_key(sun).ok().map(|sun| SourceKey {
        epoch,
        revision: world.revision(),
        sun,
    })
}

/// One queued source snapshot to prepare.
struct Job {
    world: World,
    key: SourceKey,
    sun: Sun,
    /// Opaque controller generation; a reset swaps it, retiring this job.
    generation: Arc<()>,
}

/// Identity of the job currently running on the worker.
struct Running {
    key: SourceKey,
    generation: Arc<()>,
}

/// One completed preparation, tagged with the identity it was built from.
struct Prepared {
    outcome: Result<IndirectVolume, String>,
    key: SourceKey,
    generation: Arc<()>,
}

#[derive(Default)]
struct Queue {
    generation: Arc<()>,
    requested: Option<SourceKey>,
    /// Latest requested source not yet started. Replacing it supersedes.
    pending: Option<Job>,
    /// The job currently running, for deduplication and stale retirement.
    running: Option<Running>,
    /// Single completed slot.
    result: Option<Prepared>,
    inflight: usize,
    shutdown: bool,
    resets: u64,
    discarded: u64,
    completed: u64,
}

impl Queue {
    /// Applies a request for `key`, honoring latest-request-wins and dedup.
    /// Returns whether a new job was enqueued; `fork` (the bounded COW world
    /// clone) is called only then, so duplicate requests never clone. The job
    /// is assembled here under the current generation. An invalid sun is
    /// rejected by the caller before this runs and can never displace queued
    /// work.
    fn enqueue_request(&mut self, key: SourceKey, sun: Sun, fork: impl FnOnce() -> World) -> bool {
        if self.shutdown {
            return false;
        }
        self.requested = Some(key.clone());
        if self.pending.as_ref().is_some_and(|job| job.key == key) {
            return false;
        }
        let already = self
            .running
            .as_ref()
            .is_some_and(|r| Arc::ptr_eq(&r.generation, &self.generation) && r.key == key)
            || self
                .result
                .as_ref()
                .is_some_and(|r| Arc::ptr_eq(&r.generation, &self.generation) && r.key == key);
        if already {
            self.discarded = self
                .discarded
                .saturating_add(u64::from(self.pending.take().is_some()));
            return false;
        }
        let superseded = self
            .pending
            .replace(Job {
                world: fork(),
                key,
                sun,
                generation: self.generation.clone(),
            })
            .is_some();
        self.discarded = self.discarded.saturating_add(u64::from(superseded));
        true
    }

    /// Worker: take the pending job and mark it running.
    fn adopt_job(&mut self) -> Option<Job> {
        if self.running.is_some() || self.shutdown {
            return None;
        }
        let job = self.pending.take()?;
        self.running = Some(Running {
            key: job.key.clone(),
            generation: job.generation.clone(),
        });
        self.inflight = 1;
        Some(job)
    }

    /// Worker: clear the running slot when it no longer matches the latest
    /// request, the current generation or a shutdown. Checked between update
    /// slices and while waiting, bounding cancellation latency and drop joins.
    fn retire_stale_running(&mut self) -> bool {
        let stale = self.running.as_ref().is_some_and(|r| {
            self.shutdown
                || !Arc::ptr_eq(&r.generation, &self.generation)
                || self.requested.as_ref() != Some(&r.key)
        });
        if stale {
            self.running = None;
            self.inflight = 0;
            self.discarded = self.discarded.saturating_add(1);
        }
        stale
    }

    /// Worker: buffer the completed result if it is still current, else
    /// discard it. Never blocks on the occupied slot.
    fn finish_job(&mut self, job: Job, outcome: Result<IndirectVolume, String>) {
        self.inflight = 0;
        self.running = None;
        if self.shutdown
            || !Arc::ptr_eq(&self.generation, &job.generation)
            || self.requested.as_ref() != Some(&job.key)
        {
            self.discarded = self.discarded.saturating_add(1);
            return;
        }
        self.completed = self.completed.saturating_add(1);
        let superseded = self
            .result
            .replace(Prepared {
                outcome,
                key: job.key,
                generation: job.generation,
            })
            .is_some();
        self.discarded = self.discarded.saturating_add(u64::from(superseded));
    }

    /// Poll: consume and return the buffered result only if it is current for
    /// `key`; drop a reset-cancelled result; leave a foreign-source result so
    /// a stale poll cannot erase newer work.
    fn take_result(&mut self, key: &SourceKey) -> Option<Result<IndirectVolume, String>> {
        match self.result.as_ref() {
            None => return None,
            Some(result) if !Arc::ptr_eq(&result.generation, &self.generation) => {
                self.result = None;
                self.discarded = self.discarded.saturating_add(1);
                return None;
            }
            Some(result) if result.key != *key || self.requested.as_ref() != Some(key) => {
                return None;
            }
            Some(_) => {}
        }
        Some(self.result.take().expect("result present").outcome)
    }

    /// Reset: cancel pending and buffered work and retire the in-flight job
    /// via a fresh opaque generation. A fresh identical request is accepted.
    fn cancel(&mut self) {
        let dropped = usize::from(self.pending.is_some()) + usize::from(self.result.is_some());
        self.discarded = self.discarded.saturating_add(dropped as u64);
        self.pending = None;
        self.result = None;
        self.requested = None;
        self.resets = self.resets.saturating_add(1);
        self.generation = Arc::new(());
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

/// Single-worker asynchronous indirect-light preparation controller.
///
/// Typical render-loop use: construct once with a fixed
/// [`AsyncIndirectConfig`]; call [`request`](Self::request) whenever the
/// authoritative world, its replacement epoch or the sun changes; each frame
/// call [`poll`](Self::poll) with the *current* world and epoch and, on a
/// returned `Ok`, hand the volume to [`crate::Renderer::upload_indirect`] on
/// the render owner thread. Nothing is uploaded automatically.
pub struct AsyncIndirectLight {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
}

impl AsyncIndirectLight {
    /// Validates `config` and starts the single background worker. One worker
    /// keeps ordering obvious; widening it is a measurement-led change, not a
    /// correctness requirement.
    pub fn new(config: AsyncIndirectConfig) -> Self {
        let shared = Arc::new(Shared {
            queue: Mutex::new(Queue::default()),
            wake: Condvar::new(),
        });
        let worker = std::thread::Builder::new()
            .name("matterweave-indirect".into())
            .spawn({
                let shared = Arc::clone(&shared);
                move || run(&shared, &config)
            })
            .ok();
        if worker.is_none() {
            // Without a worker every request is refused; the caller keeps the
            // synchronous IndirectVolume::update path instead of queuing work
            // nobody will execute.
            shared.lock().shutdown = true;
        }
        Self { shared, worker }
    }

    /// Requests preparation from an immutable COW snapshot of `world`.
    /// Returns whether a job was queued. The world is cloned under the lock
    /// only when a job is actually enqueued, so duplicate requests are cheap.
    ///
    /// The latest request wins: a newer source replaces a pending older one
    /// (counted as discarded). A source whose key already matches the pending,
    /// running or buffered work is deduplicated and refused, so an unchanged
    /// scene is not prepared twice. An invalid sun is rejected *before* any
    /// queued work is touched. Requests are refused after worker startup
    /// failure or exit. Callers must pass a fresh `replacement_epoch` for each
    /// new `World` instance (see module docs).
    pub fn request(
        &mut self,
        world: &World,
        replacement_epoch: u64,
        sun: Sun,
    ) -> Result<bool, String> {
        // Validate before taking the lock: an invalid sun must never replace
        // valid queued work.
        let key = source_key(world, replacement_epoch, sun).ok_or_else(|| {
            "Indirect sun must be finite, nonzero, intensity in 0..=16".to_string()
        })?;
        if !self.available() {
            return Ok(false);
        }
        let mut queue = self.shared.lock();
        if queue.shutdown {
            return Ok(false);
        }
        let queued = queue.enqueue_request(key, sun, || world.clone());
        drop(queue);
        if queued {
            self.shared.wake.notify_all();
        }
        Ok(queued)
    }

    /// Returns at most one completed volume that is *current* for the current
    /// world, replacement epoch and sun. A cancelled (reset) result is dropped
    /// here; a result for a *different* source is left buffered and `None` is
    /// returned, so polling with a stale key is harmless and can never erase
    /// newer work. On `Some(Ok(volume))` the render owner may publish via
    /// [`crate::Renderer::upload_indirect`], which re-validates the same
    /// source pair. On `Some(Err(_))` the current source genuinely failed
    /// preparation. Nonblocking; the returned volume is immutable.
    pub fn poll(
        &mut self,
        world: &World,
        replacement_epoch: u64,
        sun: Sun,
    ) -> Option<Result<IndirectVolume, String>> {
        let key = source_key(world, replacement_epoch, sun)?;
        let mut queue = self.shared.lock();
        queue.take_result(&key)
    }

    /// Cancels the pending snapshot and any buffered result, and invalidates
    /// the job in flight even if the same key is re-requested afterwards. Use
    /// on world replacement or load, in addition to a fresh replacement epoch.
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

    pub fn stats(&self) -> AsyncIndirectStats {
        let queue = self.shared.lock();
        AsyncIndirectStats {
            queued: usize::from(queue.pending.is_some()),
            inflight: queue.inflight,
            results: usize::from(queue.result.is_some()),
            discarded: queue.discarded,
            completed: queue.completed,
            generation: queue.resets,
        }
    }
}

impl Drop for AsyncIndirectLight {
    fn drop(&mut self) {
        let mut queue = self.shared.lock();
        queue.shutdown = true;
        queue.pending = None;
        drop(queue);
        self.shared.wake.notify_all();
        // Cancellation is checked between update slices. Allocation and
        // snapshot destruction also contribute to shutdown wall time.
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn run(shared: &Shared, config: &AsyncIndirectConfig) {
    'worker: loop {
        // Adopt work, retiring anything stale between slices. Waiting here is
        // the only place the worker blocks, and only while it has no job.
        let job = {
            let mut queue = shared.lock();
            loop {
                queue.retire_stale_running();
                if let Some(job) = queue.adopt_job() {
                    break job;
                }
                if queue.shutdown {
                    return;
                }
                queue = shared
                    .wake
                    .wait(queue)
                    .unwrap_or_else(PoisonError::into_inner);
            }
        };
        let mut volume = match config.build() {
            Ok(volume) => volume,
            Err(e) => {
                // Allocation can still fail after configuration validation;
                // return that failure tagged with its source identity.
                shared.lock().finish_job(job, Err(e));
                continue;
            }
        };
        // Slice the authoritative bounded update; between slices, give up the
        // job when it is no longer the latest request or the runtime stopped.
        let outcome = loop {
            let stats = match volume.update(&job.world, job.key.epoch, job.sun, WORKER_BUDGET) {
                Ok(stats) => stats,
                // Sun validated at request; a failure here is still carried
                // back tagged with the source identity.
                Err(e) => break Err(e),
            };
            let mut queue = shared.lock();
            if queue.retire_stale_running() {
                if queue.shutdown {
                    return;
                }
                continue 'worker;
            }
            drop(queue);
            if stats.complete {
                break Ok(volume);
            }
        };
        shared.lock().finish_job(job, outcome);
    }
}

#[cfg(test)]
#[path = "async_indirect_tests.rs"]
mod tests;
