//! Bounded background terrain preparation and chunk meshing.
//!
//! The caller's [`World`] stays authoritative and single threaded. Background work
//! runs on immutable snapshots: a cloned window for generation, a fixed 18³ material
//! halo for meshing. A result is published only when the authoritative world still
//! matches the input it was derived from, so background work can never undo an edit,
//! resurrect an evicted chunk or publish geometry for a changed neighborhood.
//!
//! Every buffer is explicitly bounded, so a saturated frame drops work instead of
//! growing. Dropped work leaves no record: the caller may request the same chunk or
//! window again on the next frame. These are correctness bounds only; no throughput
//! or latency behaviour is claimed before measurement on the target device.
//!
//! # Declared bounds
//!
//! Each count is a named constant in this module, except residency, which is fixed
//! by the streaming window definition:
//!
//! - pending mesh jobs: [`MAX_QUEUED_MESH_JOBS`], each holding one 18³ halo;
//! - completed meshes: [`MAX_MESH_RESULTS`] and [`MAX_MESH_RESULT_BYTES`], with the
//!   single-oversized-result exception documented on the byte bound;
//! - pending window preparations: [`MAX_QUEUED_STREAM_JOBS`], because a newer
//!   request replaces the pending one instead of queueing behind it;
//! - prepared windows awaiting publication: [`MAX_STREAM_RESULTS`];
//! - units executing on the worker: [`MAX_INFLIGHT_JOBS`];
//! - resident chunks: the 7x7x3 window selected by [`crate::STREAM_RADIUS_CHUNKS`],
//!   147 keys, and the stored edit overrides that back re-entry, capped by the
//!   streaming module's `MAX_OVERRIDES` (512).
//!
//! [`AsyncStats::within_bounds`] restates the queue, staging and mesh-byte bounds
//! for callers and tests. Residency and the stored-override cap are enforced by the
//! streaming module (`stream_around`, [`World::set`]) and are not observable in
//! [`AsyncStats`], so they are declared here but not checked by `within_bounds`.

use crate::mesh::{mesh_halo, Halo};
use crate::{Mesh, Vertex, World};
use std::collections::{BTreeSet, VecDeque};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;

/// Pending mesh jobs, each holding one 18³ material halo (32 * 5832 B = 182 KiB).
pub const MAX_QUEUED_MESH_JOBS: usize = 32;
/// Completed meshes awaiting `poll_mesh`.
pub const MAX_MESH_RESULTS: usize = 8;
/// Total allocated vertex plus index capacity bytes held in completed meshes. A single result is always
/// accepted into an empty queue, so an unusually large mesh cannot stall forever.
/// Worst case for one edited 16³ chunk is a full checkerboard: 2048 cubes with six
/// unmergeable faces each, 12288 quads = 4 * 12288 * 36 B vertices + 6 * 12288 * 4 B
/// indices, about 2.0 MiB. Four such chunks fit; more are dropped and re-requestable.
pub const MAX_MESH_RESULT_BYTES: usize = 8 * 1024 * 1024;
/// Pending window preparations retained: at most one, always the latest request.
/// A newer request replaces the pending one instead of queueing behind it.
pub const MAX_QUEUED_STREAM_JOBS: usize = 1;
/// Prepared windows staged for `poll_stream`: at most one.
pub const MAX_STREAM_RESULTS: usize = 1;
/// Jobs executing on the worker at once. One worker keeps ordering obvious, and a
/// cancelled unit cannot be preempted, so it keeps this slot until it finishes.
pub const MAX_INFLIGHT_JOBS: usize = 1;

/// Live counts for debugging and integration; not a stable metrics contract.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AsyncStats {
    pub queued_meshes: usize,
    /// Pending window preparations, bounded by [`MAX_QUEUED_STREAM_JOBS`].
    pub queued_streams: usize,
    /// Jobs currently executing on the worker, bounded by [`MAX_INFLIGHT_JOBS`].
    pub inflight: usize,
    pub mesh_results: usize,
    pub mesh_result_bytes: usize,
    /// Prepared windows awaiting `poll_stream`, bounded by [`MAX_STREAM_RESULTS`].
    pub stream_results: usize,
    /// Results dropped by saturation, cancellation or superseding requests.
    pub discarded: u64,
    /// Consecutive edit-stale stream preparations rejected for the current
    /// destination, wherever they were rejected: upstream in the worker or at
    /// `poll_stream`. Stream-specific: mesh churn never changes it. Drives the
    /// bounded synchronous fallback ([`AsyncWorld::STALE_STREAM_FALLBACK_AFTER`]).
    pub consecutive_stale_streams: u32,
    pub generation: u64,
}

impl AsyncStats {
    /// Whether every queue and staging count, and the buffered mesh bytes, are
    /// inside their declared bounds.
    ///
    /// Scope: only the counts [`AsyncStats`] exposes. The resident window
    /// ([`crate::STREAM_RADIUS_CHUNKS`]) and the stored-override cap enforced by
    /// `World::set` are not represented here and are not checked by this method.
    ///
    /// The mesh byte bound is only compared once a second result is buffered. A
    /// single result is admitted into an empty queue even when it alone exceeds
    /// [`MAX_MESH_RESULT_BYTES`], so an unusually large chunk cannot stall
    /// publication forever; that exception is the whole rule, not a second one.
    ///
    /// [`AsyncStats::consecutive_stale_streams`] is a progress counter, not a
    /// queue bound, and is not checked here.
    pub fn within_bounds(&self) -> bool {
        self.queued_meshes <= MAX_QUEUED_MESH_JOBS
            && self.queued_streams <= MAX_QUEUED_STREAM_JOBS
            && self.inflight <= MAX_INFLIGHT_JOBS
            && self.mesh_results <= MAX_MESH_RESULTS
            && self.stream_results <= MAX_STREAM_RESULTS
            && (self.mesh_results <= 1 || self.mesh_result_bytes <= MAX_MESH_RESULT_BYTES)
    }
}

struct StreamJob {
    world: World,
    position: [f32; 3],
    center: [i32; 2],
    source_revision: u64,
    seed: u64,
    generation: u64,
}

struct StreamResult {
    world: World,
    center: [i32; 2],
    source_revision: u64,
    seed: u64,
    generation: u64,
}

struct MeshJob {
    key: [i32; 3],
    voxels: Halo,
    revision: u64,
    generation: u64,
}

struct MeshResult {
    key: [i32; 3],
    mesh: Mesh,
    generation: u64,
}

#[derive(Default)]
struct Queue {
    stream: Option<StreamJob>,
    meshes: VecDeque<MeshJob>,
    /// Chunk keys queued or executing, so duplicate requests are refused and a
    /// dropped job's key becomes requestable again. Bounded by the job bounds.
    active: BTreeSet<[i32; 3]>,
    stream_result: Option<StreamResult>,
    stream_active: Option<([i32; 2], u64, u64, u64)>,
    stream_requested: Option<([i32; 2], u64, u64, u64)>,
    /// Consecutive edit-stale stream preparations rejected for the current
    /// destination. A completed preparation is rejected in exactly one place, so
    /// it is counted in exactly one place: either upstream in `run_job` (the
    /// request already advanced, or the staged result it displaces) or at
    /// `poll_stream`. It counts only when the rejected preparation is current in
    /// generation, seed and destination but predates an edit (its
    /// `source_revision` is not the one now wanted).
    ///
    /// Nothing else increments it, and the two ways it stops counting are
    /// distinct. Left unchanged: mesh work of any kind, pending stream work that
    /// has not been rejected yet, a queued replacement that never ran, and a
    /// completion stamped with a retired generation. Cleared: a successful
    /// publication, a rejection that is not edit-stale (superseded destination or
    /// seed replacement), a new or reversed destination, a reached destination and
    /// an explicit [`AsyncWorld::reset`]. Saturates instead of wrapping.
    stale_streams: u32,
    /// Destination the `stale_streams` count belongs to. A new destination
    /// resets the count instead of inheriting it.
    stale_center: Option<[i32; 2]>,
    prefer_mesh: bool,
    mesh_results: VecDeque<MeshResult>,
    mesh_result_bytes: usize,
    inflight: usize,
    shutdown: bool,
    generation: u64,
    discarded: u64,
}

impl Queue {
    /// Records one rejected stream preparation for `center`. `edit_stale` means it
    /// was current in destination, seed and generation but predates an edit; every
    /// other rejection clears the count. The single place the count grows, so the
    /// upstream and `poll_stream` rejection paths cannot drift apart.
    fn note_rejected_stream(&mut self, center: [i32; 2], edit_stale: bool) {
        if !edit_stale {
            self.clear_stall();
            return;
        }
        if self.stale_center == Some(center) {
            self.stale_streams = self.stale_streams.saturating_add(1);
        } else {
            self.stale_center = Some(center);
            self.stale_streams = 1;
        }
    }

    /// Drops any outstanding stall: no destination owes a synchronous fallback.
    fn clear_stall(&mut self) {
        self.stale_streams = 0;
        self.stale_center = None;
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

fn mesh_bytes(mesh: &Mesh) -> usize {
    mesh.vertices.capacity() * std::mem::size_of::<Vertex>() + mesh.indices.capacity() * 4
}

/// Bounded background preparation attached to one authoritative world.
///
/// Snapshot cost: three retained window clones (pending, executing, completed),
/// plus one transient clone while replacing a pending job. Chunk payloads are
/// shared across clones; edits detach only the changed payload. Generating a new
/// window allocates its new chunks. Each snapshot has at most (147 + 512) * 4096 B,
/// about 2.6 MiB of logical payload, excluding attachment/collection overhead.
/// Fully diverged snapshots can still reach that payload bound independently.
pub struct AsyncWorld {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
    /// Center of the most recent stream request; older prepared windows are dropped.
    requested_center: Option<[i32; 2]>,
}

impl Default for AsyncWorld {
    fn default() -> Self {
        Self::new()
    }
}

impl AsyncWorld {
    /// Edit-stale stream completions for one destination that trigger one
    /// synchronous fallback on the current authoritative world. At most one
    /// synchronous rewindow (a single 7x7x3 window, at most 147 generated chunks;
    /// stored edit overrides are preserved, not merged) per this many
    /// consecutive stale completions, so synchronous work stays bounded while
    /// sustained edits still advance. Small and explicit: 3. Cost is host-only;
    /// no mobile performance is claimed. See `sync_fallback_if_stalled`.
    pub const STALE_STREAM_FALLBACK_AFTER: u32 = 3;

    /// Starts the single background worker. One worker keeps ordering obvious;
    /// widening it is a measurement-led change, not a correctness requirement.
    pub fn new() -> Self {
        let shared = Arc::new(Shared {
            queue: Mutex::new(Queue::default()),
            wake: Condvar::new(),
        });
        let worker = std::thread::Builder::new()
            .name("matterweave-prepare".into())
            .spawn({
                let shared = Arc::clone(&shared);
                move || run(&shared)
            })
            .ok();
        if worker.is_none() {
            // Without a worker every request is refused; the caller keeps the
            // synchronous path instead of accumulating work nobody executes.
            shared.lock().shutdown = true;
        }
        Self {
            shared,
            worker,
            requested_center: None,
        }
    }

    /// Requests preparation of the window around `eye` from a snapshot of `world`.
    /// Returns whether a job was queued. Nonfinite positions, worlds without
    /// streaming and a window that is already published are refused. A newer
    /// request replaces a pending older one; only the latest center is publishable.
    pub fn request_stream(&mut self, world: &World, eye: [f32; 3]) -> bool {
        let Some(center) = World::stream_center_of(eye) else {
            return false;
        };
        let previous = self.requested_center;
        self.requested_center = Some(center);
        if !world.is_streaming() || world.stream_center() == Some(center) {
            let mut queue = self.shared.lock();
            queue.stream = None;
            queue.stream_result = None;
            queue.stream_requested = None;
            // Destination reached or streaming off: no stall is outstanding.
            queue.clear_stall();
            return false;
        }
        let mut queue = self.shared.lock();
        if queue.shutdown {
            return false;
        }
        if previous != Some(center) {
            // Reversal or a new destination starts a fresh stall count; the old
            // destination's refusals must not force a fallback for the new one.
            queue.clear_stall();
        }
        let generation = queue.generation;
        let identity = (center, world.revision(), world.seed(), generation);
        queue.stream_requested = Some(identity);
        // This request is the latest publishable center. Any pending window for a
        // different identity is now superseded: drop it before deduping so it can
        // neither run last and overwrite the window the caller now wants nor be
        // published for a center `poll_stream` will reject. Only one pending window
        // is retained, and it always matches the latest request.
        if queue
            .stream
            .as_ref()
            .is_some_and(|j| (j.center, j.source_revision, j.seed, j.generation) != identity)
        {
            queue.stream = None;
            queue.discarded += 1;
        }
        if queue.stream_active == Some(identity)
            || queue
                .stream
                .as_ref()
                .is_some_and(|j| (j.center, j.source_revision, j.seed, j.generation) == identity)
            || queue
                .stream_result
                .as_ref()
                .is_some_and(|j| (j.center, j.source_revision, j.seed, j.generation) == identity)
        {
            return false;
        }
        // The superseded pending window, if any, was already dropped above, so this
        // replace only ever installs the latest request into an empty pending slot.
        queue.stream = Some(StreamJob {
            world: world.clone(),
            position: eye,
            center,
            source_revision: world.revision(),
            seed: world.seed(),
            generation,
        });
        drop(queue);
        self.requested_center = Some(center);
        self.shared.wake.notify_all();
        true
    }

    /// Publishes a prepared window into `world` when it is still valid, replacing the
    /// previously resident window atomically. Returns whether `world` changed; the
    /// caller then rebuilds colliders and may resume movement gating on residency.
    ///
    /// A prepared window is rejected when the world was edited or otherwise advanced
    /// since the snapshot was taken, when the seed differs, when the request was
    /// superseded or when the controller generation changed. Until a valid window
    /// arrives the old complete window stays resident and traversable.
    ///
    /// A rejection here that is current in generation, seed, center and streaming
    /// mode but predates an edit counts toward the bounded synchronous fallback
    /// (`sync_fallback_if_stalled`); any other rejection resets that count. A
    /// completed preparation the worker already rejected never reaches this
    /// function and is counted there instead, exactly once (see `run_job`).
    pub fn poll_stream(&mut self, world: &mut World) -> bool {
        let mut queue = self.shared.lock();
        let Some(result) = queue.stream_result.take() else {
            return false;
        };
        let acceptable = result.generation == queue.generation
            && Some(result.center) == self.requested_center
            && result.seed == world.seed()
            && result.source_revision == world.revision()
            && world.is_streaming();
        if !acceptable {
            queue.discarded += 1;
            let edit_stale = result.generation == queue.generation
                && Some(result.center) == self.requested_center
                && result.seed == world.seed()
                && world.is_streaming()
                && result.source_revision != world.revision();
            queue.note_rejected_stream(result.center, edit_stale);
            return false;
        }
        queue.clear_stall();
        drop(queue);
        *world = result.world;
        true
    }

    /// Consecutive edit-stale stream preparations counted for the current
    /// destination, whether the worker rejected them upstream or `poll_stream`
    /// did. Mesh churn and pending work that never ran change nothing.
    pub fn consecutive_stale_streams(&self) -> u32 {
        self.shared.lock().stale_streams
    }

    /// Bounded synchronous fallback for sustained edits. Apply after `poll_stream`
    /// in the same frame (see the explorer `stream_begin` sequence): when the
    /// counted destination `eye` is still outstanding and the stale count reached
    /// [`AsyncWorld::STALE_STREAM_FALLBACK_AFTER`], rewindow synchronously on the
    /// current authoritative world and reset the count. Returns whether `world`
    /// changed. A failed rewindow (for example revision exhaustion) keeps the
    /// count so the caller retries cheaply next frame instead of spinning.
    pub fn sync_fallback_if_stalled(&mut self, world: &mut World, eye: [f32; 3]) -> bool {
        let Some(center) = World::stream_center_of(eye) else {
            return false;
        };
        {
            let mut queue = self.shared.lock();
            if !world.is_streaming() || world.stream_center() == Some(center) {
                // Destination already reached (or streaming is off): nothing is
                // outstanding, so clear the stall this destination owns instead of
                // leaving it armed for a later frame. A stall counted for another
                // destination is left alone.
                if queue.stale_center == Some(center) {
                    queue.clear_stall();
                }
                return false;
            }
            if queue.stale_streams < Self::STALE_STREAM_FALLBACK_AFTER
                || queue.stale_center != Some(center)
                || self.requested_center != Some(center)
            {
                return false;
            }
        }
        // The destination differs from the resident centre, the world streams and
        // `eye` has a centre, all checked above and unchanged since (`world` is
        // exclusively borrowed), so only revision exhaustion can refuse here. A
        // refusal keeps the count, and the caller retries cheaply next frame
        // instead of spinning on a synchronous window.
        if !world.stream_around(eye) {
            return false;
        }
        self.shared.lock().clear_stall();
        true
    }

    /// Requests a mesh for `key` from a copy of only that chunk and its halo.
    /// Returns whether a job was queued. An absent chunk, a duplicate request for a
    /// key already queued or executing, and a saturated job queue are refused; the
    /// caller may request the key again on a later frame.
    pub fn request_mesh(&mut self, world: &World, key: [i32; 3]) -> bool {
        let Some(revision) = world.chunk_revision(key) else {
            return false;
        };
        let queue = self.shared.lock();
        if queue.shutdown
            || queue.meshes.len() >= MAX_QUEUED_MESH_JOBS
            || queue.active.contains(&key)
            || queue
                .mesh_results
                .iter()
                .any(|r| r.key == key && r.mesh.revision == revision)
        {
            return false;
        }
        // The public API requires &mut self, so only the worker can change the
        // queue during this unlocked copy; it can only free capacity.
        drop(queue);
        let Some(voxels) = world.chunk_halo(key) else {
            return false;
        };
        let mut queue = self.shared.lock();
        queue.active.insert(key);
        let generation = queue.generation;
        queue.meshes.push_back(MeshJob {
            key,
            voxels,
            revision,
            generation,
        });
        drop(queue);
        self.shared.wake.notify_all();
        true
    }

    /// Returns at most one completed mesh that is still valid for `world`: the chunk
    /// still exists and its revision, including shared faces with edited neighbors,
    /// still matches the meshed data. Stale results are discarded here, and their
    /// chunks can be requested again.
    pub fn poll_mesh(&mut self, world: &World) -> Option<([i32; 3], Mesh)> {
        let mut queue = self.shared.lock();
        while let Some(result) = queue.mesh_results.pop_front() {
            queue.mesh_result_bytes -= mesh_bytes(&result.mesh);
            if result.generation == queue.generation
                && world.chunk_revision(result.key) == Some(result.mesh.revision)
            {
                return Some((result.key, result.mesh));
            }
            queue.discarded += 1;
        }
        None
    }

    /// Cancels all queued and completed work and invalidates results of the job in
    /// flight. Use on world replacement, synchronous rewindowing or load. Chunks
    /// remain requestable immediately afterwards.
    pub fn reset(&mut self) {
        let mut queue = self.shared.lock();
        let dropped = queue.meshes.len()
            + queue.mesh_results.len()
            + usize::from(queue.stream.is_some())
            + usize::from(queue.stream_result.is_some())
            + queue.inflight;
        queue.discarded += dropped as u64;
        queue.meshes.clear();
        queue.mesh_results.clear();
        queue.mesh_result_bytes = 0;
        queue.active.clear();
        queue.stream = None;
        queue.stream_result = None;
        queue.stream_requested = None;
        queue.clear_stall();
        // The generation is an opaque, strictly increasing cancellation token that is
        // never reused: each reset mints a fresh value that invalidates every job
        // stamped with an older one, including the job in flight. Saturating at
        // u64::MAX would reuse the current token, letting a same-revision in-flight
        // job escape cancellation. Rather than reuse a token, retire the worker at
        // exhaustion; the caller keeps the synchronous path (`available()` is false
        // and requests are refused), so no stale background result can be published.
        let retired = match queue.generation.checked_add(1) {
            Some(next) => {
                queue.generation = next;
                false
            }
            None => {
                queue.shutdown = true;
                true
            }
        };
        drop(queue);
        self.requested_center = None;
        if retired {
            // Wake an idle worker so it observes shutdown and exits promptly.
            self.shared.wake.notify_all();
        }
    }

    /// False after worker startup failure or an unexpected worker exit.
    pub fn available(&self) -> bool {
        !self.shared.lock().shutdown
            && self
                .worker
                .as_ref()
                .is_some_and(|worker| !worker.is_finished())
    }

    pub fn stats(&self) -> AsyncStats {
        let queue = self.shared.lock();
        AsyncStats {
            queued_meshes: queue.meshes.len(),
            queued_streams: usize::from(queue.stream.is_some()),
            inflight: queue.inflight,
            mesh_results: queue.mesh_results.len(),
            mesh_result_bytes: queue.mesh_result_bytes,
            stream_results: usize::from(queue.stream_result.is_some()),
            discarded: queue.discarded,
            consecutive_stale_streams: queue.stale_streams,
            generation: queue.generation,
        }
    }
}

impl Drop for AsyncWorld {
    fn drop(&mut self) {
        let mut queue = self.shared.lock();
        queue.shutdown = true;
        queue.meshes.clear();
        queue.stream = None;
        drop(queue);
        self.shared.wake.notify_all();
        // The worker never blocks on a full result buffer, so it can always observe
        // shutdown after at most one bounded job and this join cannot deadlock.
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

enum Job {
    Stream(StreamJob),
    Mesh(MeshJob),
}

fn run(shared: &Shared) {
    loop {
        let mut queue = shared.lock();
        // Cancellation is observed between jobs; each job is one bounded unit,
        // a single window (at most 147 generated chunks) or a single 18³ mesh.
        while !queue.shutdown && queue.stream.is_none() && queue.meshes.is_empty() {
            queue = shared
                .wake
                .wait(queue)
                .unwrap_or_else(PoisonError::into_inner);
        }
        if queue.shutdown {
            return;
        }
        let job = take_job(&mut queue);
        drop(queue);
        run_job(shared, job);
    }
}

/// Selects the next bounded unit under the lock. Alternates when both classes are
/// ready so sustained travel cannot starve retained-chunk geometry. The caller
/// guarantees at least one job is present (a window of at most 147 generated
/// chunks or a single 18³ mesh).
fn take_job(queue: &mut Queue) -> Job {
    let job = if !queue.meshes.is_empty() && (queue.prefer_mesh || queue.stream.is_none()) {
        queue.prefer_mesh = false;
        Job::Mesh(queue.meshes.pop_front().expect("nonempty queue"))
    } else {
        queue.prefer_mesh = true;
        let job = queue.stream.take().expect("nonempty stream queue");
        queue.stream_active = Some((job.center, job.source_revision, job.seed, job.generation));
        Job::Stream(job)
    };
    queue.inflight = 1;
    job
}

/// Executes one bounded unit on its snapshot, then publishes or discards the
/// result under the lock according to shutdown, generation and world validity.
fn run_job(shared: &Shared, job: Job) {
    match job {
        Job::Stream(job) => {
            let mut world = job.world;
            world.stream_around(job.position);
            let mut queue = shared.lock();
            queue.inflight = 0;
            queue.stream_active = None;
            if queue.shutdown || queue.generation != job.generation {
                // Cancelled or retired: completed work nobody can use. The stall
                // count belongs to live work of the current generation, so a
                // late pre-generation completion leaves it unchanged.
                queue.discarded += 1;
                return;
            }
            if queue.stream_requested
                != Some((job.center, job.source_revision, job.seed, job.generation))
            {
                // The completed preparation is rejected here and never reaches
                // `poll_stream`, so this is its one classification: a newer
                // request for the SAME destination, seed and generation means an
                // edit advanced the revision mid-preparation (the sustained-edit
                // schedule with the next request landing before completion).
                // Anything else is a superseded center, a seed replacement or a
                // cleared request, which resets instead of counting. Queued
                // replacements never complete and are never counted.
                queue.discarded += 1;
                let edit_stale =
                    queue
                        .stream_requested
                        .is_some_and(|(center, revision, seed, generation)| {
                            center == job.center
                                && seed == job.seed
                                && generation == job.generation
                                && revision != job.source_revision
                        });
                queue.note_rejected_stream(job.center, edit_stale);
                return;
            }
            let displaced = queue.stream_result.replace(StreamResult {
                world,
                center: job.center,
                source_revision: job.source_revision,
                seed: job.seed,
                generation: job.generation,
            });
            if let Some(old) = displaced {
                // The displaced staged preparation is rejected here and never
                // reaches `poll_stream`, so classify it exactly once by the same
                // rule: same destination, seed and generation with an older
                // revision is edit-stale work; anything else resets. The freshly
                // staged preparation is not counted here; `poll_stream` counts it
                // if and only if it is rejected there.
                queue.discarded += 1;
                let edit_stale = old.center == job.center
                    && old.seed == job.seed
                    && old.generation == job.generation
                    && old.source_revision != job.source_revision;
                queue.note_rejected_stream(old.center, edit_stale);
            }
        }
        Job::Mesh(job) => {
            let mesh = mesh_halo(job.key, &job.voxels, job.revision);
            let bytes = mesh_bytes(&mesh);
            let mut queue = shared.lock();
            queue.inflight = 0;
            if queue.generation == job.generation {
                queue.active.remove(&job.key);
            }
            let full = queue.mesh_results.len() >= MAX_MESH_RESULTS
                || (!queue.mesh_results.is_empty()
                    && queue.mesh_result_bytes + bytes > MAX_MESH_RESULT_BYTES);
            if queue.shutdown || queue.generation != job.generation || full {
                // Nothing records the drop, so the caller can request the key again.
                queue.discarded += 1;
                return;
            }
            queue.mesh_result_bytes += bytes;
            queue.mesh_results.push_back(MeshResult {
                key: job.key,
                mesh,
                generation: job.generation,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CHUNK_EDGE, STREAM_RADIUS_CHUNKS, WORLD_LIMIT};

    impl AsyncWorld {
        /// Test-only controller with no background worker. The queue stays live so a
        /// test can drive the state machine deterministically through `take_job`
        /// and `run_job` instead of racing a real thread. Private: not a public API.
        fn manual() -> Self {
            Self {
                shared: Arc::new(Shared {
                    queue: Mutex::new(Queue::default()),
                    wake: Condvar::new(),
                }),
                worker: None,
                requested_center: None,
            }
        }

        /// Runs every currently pending unit to completion, simulating the worker
        /// deterministically with no timing dependence.
        fn drain(&self) {
            loop {
                let mut queue = self.shared.lock();
                if queue.stream.is_none() && queue.meshes.is_empty() {
                    return;
                }
                let job = take_job(&mut queue);
                drop(queue);
                run_job(&self.shared, job);
            }
        }
    }

    fn streamed(seed: u64, eye: [f32; 3]) -> World {
        let mut world = World::generate(seed);
        world.enable_streaming();
        assert!(world.stream_around(eye));
        world
    }

    /// Every field publication is allowed to deliver, compared exactly: revision,
    /// residency, the chunk set, per-chunk revisions and every material.
    fn assert_worlds_equal(actual: &World, expected: &World) {
        assert_eq!(actual.revision(), expected.revision());
        assert_eq!(
            actual.stream_resident_chunks(),
            expected.stream_resident_chunks()
        );
        assert_eq!(actual.chunk_keys(), expected.chunk_keys());
        assert_eq!(actual.stats(), expected.stats());
        for key in expected.chunk_keys() {
            assert_eq!(
                actual.chunk_revision(key),
                expected.chunk_revision(key),
                "chunk {key:?}"
            );
            for index in 0..4096_i32 {
                let cell = [
                    key[0] * CHUNK_EDGE + index % CHUNK_EDGE,
                    key[1] * CHUNK_EDGE + (index / CHUNK_EDGE) % CHUNK_EDGE,
                    key[2] * CHUNK_EDGE + index / (CHUNK_EDGE * CHUNK_EDGE),
                ];
                assert_eq!(actual.get(cell), expected.get(cell), "cell {cell:?}");
            }
        }
    }

    /// The declared residency window for an eye position: a 7x7x3 chunk span around
    /// the stream centre, clamped to the world limits.
    fn window_keys(eye: [f32; 3]) -> Vec<[i32; 3]> {
        let center = World::stream_center_of(eye).expect("finite eye");
        let limit = WORLD_LIMIT / CHUNK_EDGE;
        let mut keys = BTreeSet::new();
        for x in (center[0] - STREAM_RADIUS_CHUNKS).max(-limit)
            ..=(center[0] + STREAM_RADIUS_CHUNKS).min(limit - 1)
        {
            for z in (center[1] - STREAM_RADIUS_CHUNKS).max(-limit)
                ..=(center[1] + STREAM_RADIUS_CHUNKS).min(limit - 1)
            {
                for y in -1..2 {
                    keys.insert([x, y, z]);
                }
            }
        }
        keys.into_iter().collect()
    }

    /// The published window is exactly the declared span and sampled interior
    /// positions report support. `stream_contains_position` is the contract that
    /// keeps movement and collision off an evicted window; whether a physics body is
    /// physically supported is an integration-level gate.
    fn assert_window_supported(world: &World, eye: [f32; 3]) {
        assert_eq!(world.stream_resident_chunks().unwrap(), window_keys(eye));
        let base = [eye[0].floor(), eye[2].floor()];
        for dx in [-40.0, -24.0, -8.0, 8.0, 24.0, 40.0] {
            for dz in [-40.0, -24.0, -8.0, 8.0, 24.0, 40.0] {
                for y in [-14.0, 4.0, 30.0] {
                    let position = [base[0] + dx, y, base[1] + dz];
                    assert!(
                        world.stream_contains_position(position, 1.0),
                        "{position:?} is unsupported inside the window around {eye:?}"
                    );
                }
            }
        }
    }

    /// One deterministic frame with no worker: the caller requests `eye`, the
    /// simulated worker starts the preparation, the caller optionally edits, the
    /// preparation completes, then the caller polls. Returns whether the request was
    /// queued, whether a preparation completed, and whether one was published.
    fn async_frame(
        jobs: &mut AsyncWorld,
        world: &mut World,
        eye: [f32; 3],
        edit: Option<([i32; 3], u8)>,
    ) -> (bool, bool, bool) {
        let queued = jobs.request_stream(world, eye);
        let job = queued.then(|| take_job(&mut jobs.shared.lock()));
        if let Some((cell, material)) = edit {
            assert!(world.set(cell, material), "edit of {cell:?} was refused");
        }
        if let Some(job) = job {
            run_job(&jobs.shared, job);
        }
        let prepared = jobs.stats().stream_results == MAX_STREAM_RESULTS;
        (queued, prepared, jobs.poll_stream(world))
    }

    // Bug 1: with A in flight and B pending, returning to A must drop the now
    // superseded pending B so B cannot run last and overwrite A's valid result.
    #[test]
    fn latest_request_matching_inflight_drops_a_superseded_pending_window() {
        let world = streamed(8712, [0.0, 4.0, 0.0]);
        let eye_a = [120.0, 4.0, 0.0];
        let eye_b = [-120.0, 4.0, 0.0];
        let center_a = World::stream_center_of(eye_a).unwrap();
        let center_b = World::stream_center_of(eye_b).unwrap();
        assert_ne!(center_a, center_b);

        let mut jobs = AsyncWorld::manual();

        // Request A, then let the worker take it: A is now in flight.
        assert!(jobs.request_stream(&world, eye_a));
        let job_a = take_job(&mut jobs.shared.lock());
        assert!(matches!(job_a, Job::Stream(_)));
        assert_eq!(
            jobs.shared.lock().stream_active.map(|(c, ..)| c),
            Some(center_a)
        );

        // A different destination B is requested and queued while A runs.
        assert!(jobs.request_stream(&world, eye_b));
        assert!(jobs.shared.lock().stream.is_some());

        // The caller returns to A. A is in flight, so no new job is queued, but the
        // now-superseded pending B must be discarded so it cannot overwrite A.
        assert!(!jobs.request_stream(&world, eye_a));
        assert_eq!(jobs.requested_center, Some(center_a));
        assert!(
            jobs.shared.lock().stream.is_none(),
            "superseded pending B was not dropped: {:?}",
            jobs.shared.lock().stream.as_ref().map(|j| j.center)
        );

        // Finish A, then any remaining pending unit, then publish. The latest
        // request A must win; a surviving B would replace and reject it.
        run_job(&jobs.shared, job_a);
        jobs.drain();
        let mut out = world.clone();
        assert!(
            jobs.poll_stream(&mut out),
            "latest request A was never published"
        );
        assert!(out.stream_contains_position(eye_a, 1.0));
        assert!(!out.stream_contains_position(eye_b, 1.0));
    }

    // Bug 2: reset must cancel in-flight work. A saturating generation at u64::MAX
    // cannot mint a fresh token, so the controller retires to the synchronous path
    // instead of silently accepting the stale in-flight result.
    #[test]
    fn reset_at_generation_exhaustion_retires_instead_of_leaking_inflight_work() {
        let mut world = World::new(0);
        for cell in [[15, 5, 5], [16, 5, 5], [20, 5, 5]] {
            world.set(cell, 1);
        }
        let key = [1, 0, 0];
        assert!(world.chunk_revision(key).is_some());

        let mut jobs = AsyncWorld::manual();
        jobs.shared.lock().generation = u64::MAX;

        // Queue and start a mesh job at the exhausted generation: now in flight.
        assert!(jobs.request_mesh(&world, key));
        let job = take_job(&mut jobs.shared.lock());
        assert!(matches!(job, Job::Mesh(_)));

        jobs.reset();
        assert!(
            jobs.shared.lock().shutdown,
            "exhausted reset did not retire the worker"
        );

        // Completing the in-flight job after reset must not publish a stale result.
        run_job(&jobs.shared, job);
        assert!(
            jobs.poll_mesh(&world).is_none(),
            "reset guarantee violated: a stale in-flight mesh escaped cancellation"
        );
        // The retired controller refuses new work; the caller keeps the sync path.
        assert!(!jobs.request_mesh(&world, key));
    }

    // A normal (non-exhausted) reset still mints a fresh token and stays available.
    #[test]
    fn reset_below_exhaustion_bumps_generation_and_keeps_running() {
        let mut jobs = AsyncWorld::manual();
        jobs.reset();
        let queue = jobs.shared.lock();
        assert_eq!(queue.generation, 1);
        assert!(!queue.shutdown);
    }

    #[test]
    fn returning_to_buffered_window_retains_it_after_other_work_finishes() {
        let mut world = streamed(8712, [0., 4., 0.]);
        let mut jobs = AsyncWorld::manual();
        let a = [120., 4., 0.];
        let b = [-120., 4., 0.];
        assert!(jobs.request_stream(&world, a));
        let first = take_job(&mut jobs.shared.lock());
        run_job(&jobs.shared, first);
        assert!(jobs.request_stream(&world, b));
        let second = take_job(&mut jobs.shared.lock());
        assert!(!jobs.request_stream(&world, a));
        run_job(&jobs.shared, second);
        assert!(
            jobs.poll_stream(&mut world),
            "latest requested buffered window was lost"
        );
        assert_eq!(world.stream_center(), World::stream_center_of(a));
    }

    #[test]
    fn retirement_reports_unavailable_before_the_worker_exits() {
        let mut jobs = AsyncWorld::manual();
        let (release, waiting) = std::sync::mpsc::channel();
        jobs.worker = Some(std::thread::spawn(move || waiting.recv().unwrap()));
        assert!(jobs.available());
        jobs.shared.lock().generation = u64::MAX;
        jobs.reset();
        let available = jobs.available();
        release.send(()).unwrap();
        drop(jobs);
        assert!(
            !available,
            "retired worker must expose synchronous fallback immediately"
        );
    }

    /// The recorded candidate: "streaming stale-result rejection starves under
    /// continuous edits". Workerless and deterministic: one preparation per
    /// interval, and an edit inside an interval makes that interval's completed
    /// result stale. The discriminator is not that an edited interval refuses, but
    /// whether the refusing condition can clear, so the same loop continues with
    /// edits every second interval and then with edits stopped.
    ///
    /// Verdict: the safety core is correct, and this test proves *conditional*
    /// progress, not sustained-edit liveness. Refusal is per preparation interval:
    /// eight edited intervals completed eight preparations and refused all eight,
    /// one edit-free interval published while edits were still arriving, and the
    /// refusals lost nothing, because the synchronous replay of the same operations
    /// reaches the identical world. An edit inside *every* interval is still never
    /// published by this design, so streaming liveness under sustained edits stays
    /// OPEN until a caller-side fallback (or another edit-preserving approach) is
    /// implemented and device-verified.
    #[test]
    fn stale_stream_refusal_defers_until_one_preparation_interval_is_edit_free() {
        const SEED: u64 = 20260912;
        let origin_eye = [0.0, 4.0, 0.0];
        let storm_eye = [120.0, 4.0, 0.0];
        let slow_eye = [-120.0, 4.0, 0.0];
        let settled_eye = [40.0, 4.0, 0.0];
        let edit = [0, 20, 0];
        let origin = World::stream_center_of(origin_eye).unwrap();
        let mut world = streamed(SEED, origin_eye);
        assert_eq!(world.get(edit), 0);
        let mut jobs = AsyncWorld::manual();
        let mut edits: Vec<([i32; 3], u8)> = Vec::new();

        // Phase 1, the reported shape: an edit lands inside every preparation
        // interval for eight intervals. Each preparation completes and is refused,
        // because publishing it would revert the edit that arrived after its
        // snapshot was taken.
        for frame in 0..8 {
            let material = 9 + (frame % 2) as u8;
            let (queued, prepared, published) =
                async_frame(&mut jobs, &mut world, storm_eye, Some((edit, material)));
            edits.push((edit, material));
            assert!(queued, "frame {frame}: the request was not queued");
            assert!(prepared, "frame {frame}: preparation did not complete");
            assert!(!published, "frame {frame}: a stale window was published");
        }
        assert_eq!(
            world.stream_center(),
            Some(origin),
            "the window advanced while every interval carried an edit"
        );
        assert_eq!(
            jobs.stats().stream_results,
            0,
            "a refused window stayed staged"
        );
        assert!(jobs.stats().discarded >= 8);
        assert!(jobs.stats().within_bounds());

        // Phase 2: edits keep arriving, now only every second preparation interval.
        // The edit-free interval completes at the current revision and publishes,
        // which is the discriminator itself: the refusing condition clears while the
        // edit stream continues.
        let (queued, prepared, published) =
            async_frame(&mut jobs, &mut world, slow_eye, Some((edit, 9)));
        edits.push((edit, 9));
        assert!(queued, "the edited interval did not queue");
        assert!(prepared, "the edited interval did not complete");
        assert!(!published, "the edited interval published a stale window");
        let (queued, prepared, published) = async_frame(&mut jobs, &mut world, slow_eye, None);
        assert!(queued, "the edit-free interval did not queue");
        assert!(prepared, "the edit-free interval did not complete");
        assert!(published, "the edit-free interval did not publish");
        assert_window_supported(&world, slow_eye);

        // Phase 3, the control: with edits stopped, the next preparation publishes
        // immediately. A stall could not do this either.
        let (queued, prepared, published) = async_frame(&mut jobs, &mut world, settled_eye, None);
        assert!(queued && prepared && published);
        assert_window_supported(&world, settled_eye);

        // Nothing was lost by the refusals: replaying the same edits and the two
        // publications that did happen through the synchronous path reaches the
        // identical world, edits included.
        let mut reference = streamed(SEED, origin_eye);
        for (cell, material) in &edits {
            assert!(
                reference.set(*cell, *material),
                "reference edit of {cell:?}"
            );
        }
        assert!(reference.stream_around(slow_eye));
        assert!(reference.stream_around(settled_eye));
        assert_worlds_equal(&world, &reference);
        assert_eq!(world.get(edit), 9);
    }

    /// Cancellation mid-flight. The executing unit cannot be preempted, so the stated
    /// baseline is: the cancelled unit still holds the single in-flight slot, every
    /// queue and staging slot is empty, and residency is untouched. Completing a
    /// cancelled unit afterwards must publish nothing and must not strip the
    /// registration of work requested after the cancellation.
    #[test]
    fn reset_mid_flight_states_the_baseline_and_cancels_every_pending_unit() {
        let mut world = streamed(20260912, [0.0, 4.0, 0.0]);
        let origin = world.stream_center().unwrap();
        let resident = world.stream_resident_chunks().unwrap();
        let revision = world.revision();
        let keys: Vec<[i32; 3]> = world.chunk_keys().into_iter().take(2).collect();
        assert_eq!(keys.len(), 2);
        let mut jobs = AsyncWorld::manual();

        // A window and two meshes pending, with the window already executing.
        assert!(jobs.request_stream(&world, [120.0, 4.0, 0.0]));
        assert!(jobs.request_mesh(&world, keys[0]));
        assert!(jobs.request_mesh(&world, keys[1]));
        let cancelled_stream = take_job(&mut jobs.shared.lock());
        assert!(matches!(cancelled_stream, Job::Stream(_)));
        assert_eq!(jobs.stats().queued_meshes, 2);

        jobs.reset();

        let stats = jobs.stats();
        assert_eq!(stats.queued_meshes, 0);
        assert_eq!(stats.queued_streams, 0);
        assert_eq!(stats.mesh_results, 0);
        assert_eq!(stats.mesh_result_bytes, 0);
        assert_eq!(stats.stream_results, 0);
        assert_eq!(
            stats.inflight, MAX_INFLIGHT_JOBS,
            "the cancelled unit is one bounded, unpreemptable job"
        );
        assert_eq!(stats.generation, 1);
        assert!(stats.within_bounds(), "{stats:?}");
        assert_eq!(world.revision(), revision);
        assert_eq!(world.stream_center(), Some(origin));
        assert_eq!(world.stream_resident_chunks().unwrap(), resident);

        // Re-request a cancelled key before the cancelled unit completes. The
        // cancelled completion must neither publish nor unfreeze the new request.
        assert!(jobs.request_mesh(&world, keys[0]));
        run_job(&jobs.shared, cancelled_stream);
        assert_eq!(jobs.stats().inflight, 0);
        // The cancelled completion must not occupy a staging slot either: a refused
        // result still holds its bytes until `poll_*` drops it.
        assert_eq!(
            jobs.stats().stream_results,
            0,
            "a cancelled window stayed staged"
        );
        assert!(
            !jobs.poll_stream(&mut world),
            "a cancelled window was published"
        );
        assert_eq!(world.stream_center(), Some(origin));
        assert!(
            !jobs.request_mesh(&world, keys[0]),
            "the cancelled job cleared the new request"
        );
        let job = take_job(&mut jobs.shared.lock());
        run_job(&jobs.shared, job);
        let (key, mesh) = jobs.poll_mesh(&world).expect("the re-requested mesh");
        assert_eq!(key, keys[0]);
        assert_eq!(world.chunk_revision(key), Some(mesh.revision));

        // A mesh cancelled while executing, then re-requested after the cancel.
        assert!(jobs.request_mesh(&world, keys[1]));
        let cancelled_mesh = take_job(&mut jobs.shared.lock());
        assert!(matches!(cancelled_mesh, Job::Mesh(_)));
        jobs.reset();
        assert_eq!(jobs.stats().generation, 2);
        assert!(jobs.request_mesh(&world, keys[1]));
        run_job(&jobs.shared, cancelled_mesh);
        assert_eq!(
            jobs.stats().mesh_results,
            0,
            "a cancelled mesh stayed staged"
        );
        assert_eq!(jobs.stats().mesh_result_bytes, 0);
        assert!(
            jobs.poll_mesh(&world).is_none(),
            "a cancelled mesh was published"
        );
        assert!(
            !jobs.request_mesh(&world, keys[1]),
            "the cancelled mesh cleared the new request"
        );
        let job = take_job(&mut jobs.shared.lock());
        run_job(&jobs.shared, job);
        let (key, mesh) = jobs.poll_mesh(&world).expect("the re-requested mesh");
        assert_eq!(key, keys[1]);
        assert_eq!(world.chunk_revision(key), Some(mesh.revision));
        assert!(jobs.stats().within_bounds());
    }

    /// At the declared limits, requests are refused cleanly: no count passes its
    /// bound, a refused request is refused again rather than partially admitted, and
    /// a dropped result frees its key for re-request.
    #[test]
    fn queue_and_staging_limits_refuse_cleanly_at_the_declared_bounds() {
        let mut world = streamed(20260912, [0.0, 4.0, 0.0]);
        assert!(world.set([0, 20, 0], 9));
        let keys = world.chunk_keys();
        assert!(keys.len() > MAX_QUEUED_MESH_JOBS);
        let mut jobs = AsyncWorld::manual();

        // Fill the mesh job queue to exactly its bound, then observe one refusal.
        let mut refused = None;
        for &key in &keys {
            if !jobs.request_mesh(&world, key) {
                refused = Some(key);
                break;
            }
        }
        let refused = refused.expect("the mesh queue never reached MAX_QUEUED_MESH_JOBS");
        let stats = jobs.stats();
        assert_eq!(stats.queued_meshes, MAX_QUEUED_MESH_JOBS);
        assert!(stats.within_bounds(), "{stats:?}");
        assert!(!jobs.request_mesh(&world, refused));
        assert!(jobs.stats().within_bounds());

        // Complete meshes until the staging buffer is also at its bound.
        while jobs.stats().mesh_results < MAX_MESH_RESULTS {
            let job = take_job(&mut jobs.shared.lock());
            run_job(&jobs.shared, job);
            let stats = jobs.stats();
            assert!(stats.within_bounds(), "{stats:?}");
        }
        assert_eq!(jobs.stats().mesh_results, MAX_MESH_RESULTS);

        // One more completion is dropped instead of buffered, and its key becomes
        // requestable again rather than leaking a permanently blocked key.
        let job = take_job(&mut jobs.shared.lock());
        let Job::Mesh(dropped) = &job else {
            panic!("expected a queued mesh, not a window preparation");
        };
        let dropped_key = dropped.key;
        run_job(&jobs.shared, job);
        let stats = jobs.stats();
        assert_eq!(
            stats.mesh_results, MAX_MESH_RESULTS,
            "a full staging buffer admitted a result"
        );
        assert!(stats.within_bounds(), "{stats:?}");
        assert!(
            jobs.request_mesh(&world, dropped_key),
            "a dropped result did not free its key"
        );

        // The byte bound and its documented single-oversized-result exception.
        let single = AsyncStats {
            mesh_results: 1,
            mesh_result_bytes: MAX_MESH_RESULT_BYTES + 1,
            ..AsyncStats::default()
        };
        assert!(
            single.within_bounds(),
            "one oversized result is admitted by design"
        );
        let second = AsyncStats {
            mesh_results: 2,
            mesh_result_bytes: MAX_MESH_RESULT_BYTES + 1,
            ..AsyncStats::default()
        };
        assert!(!second.within_bounds());

        // Window staging: a newer request replaces the pending window instead of
        // queueing behind it, and never displaces a staged window, so the controller
        // holds at most MAX_QUEUED_STREAM_JOBS + MAX_STREAM_RESULTS prepared worlds.
        assert!(jobs.request_stream(&world, [120.0, 4.0, 0.0]));
        assert!(jobs.request_stream(&world, [-120.0, 4.0, 0.0]));
        let stats = jobs.stats();
        assert_eq!(stats.queued_streams, MAX_QUEUED_STREAM_JOBS);
        assert!(stats.within_bounds(), "{stats:?}");
        assert_eq!(
            jobs.shared.lock().stream.as_ref().map(|job| job.center),
            World::stream_center_of([-120.0, 4.0, 0.0]),
            "the pending window is not the latest request"
        );
        let job = take_job(&mut jobs.shared.lock());
        assert!(matches!(job, Job::Stream(_)));
        run_job(&jobs.shared, job);
        assert_eq!(jobs.stats().stream_results, MAX_STREAM_RESULTS);
        assert!(jobs.request_stream(&world, [40.0, 4.0, 0.0]));
        let stats = jobs.stats();
        assert_eq!(
            stats.queued_streams + stats.stream_results,
            MAX_QUEUED_STREAM_JOBS + MAX_STREAM_RESULTS
        );
        assert!(stats.within_bounds(), "{stats:?}");
    }

    /// The real byte-admission branch in `run_job`, exercised with production meshes
    /// instead of synthetic `AsyncStats` literals. A 3D checkerboard chunk defeats
    /// greedy merging, so each result is several MiB and the byte bound is reached
    /// while the result count is still below `MAX_MESH_RESULTS`. The refusal must
    /// leave the retained count and bytes unchanged and must free the key for
    /// re-request; deleting the byte clause in `run_job` would admit the completion
    /// and this test would find no refusal at all.
    #[test]
    fn mesh_result_byte_bound_refuses_a_completion_below_the_count_cap() {
        // Eight independent checkerboard chunks, no streaming, so `set` needs no
        // residency gate. Parity is global, so no two solid voxels share a face and
        // every one of the 2048 solids per chunk contributes six unmergeable quads
        // (12288 quads). Eight keys is deliberate: with the byte clause removed all
        // eight fit under the count cap and no refusal happens.
        let keys: [[i32; 3]; 8] = [
            [0, 0, 0],
            [1, 0, 0],
            [2, 0, 0],
            [3, 0, 0],
            [4, 0, 0],
            [5, 0, 0],
            [6, 0, 0],
            [7, 0, 0],
        ];
        let mut world = World::new(0);
        for key in keys {
            for z in 0..16 {
                for y in 0..16 {
                    for x in 0..16 {
                        if (x + y + z) % 2 == 0 {
                            assert!(world.set([key[0] * 16 + x, y, z], 3));
                        }
                    }
                }
            }
        }
        // Real meshes, measured from actual allocate capacities rather than assumed.
        let sizes: Vec<usize> = keys
            .iter()
            .map(|&key| mesh_bytes(&world.mesh_chunk(key)))
            .collect();
        assert!(
            sizes.iter().sum::<usize>() > MAX_MESH_RESULT_BYTES,
            "workload cannot reach the byte bound: {sizes:?}"
        );

        let mut jobs = AsyncWorld::manual();
        for (admitted, &key) in keys.iter().enumerate() {
            assert!(jobs.request_mesh(&world, key), "request {key:?} refused");
            let job = take_job(&mut jobs.shared.lock());
            let refused_key = match &job {
                Job::Mesh(job) => job.key,
                Job::Stream(_) => panic!("expected a mesh job"),
            };
            let before = jobs.stats();
            run_job(&jobs.shared, job);
            let after = jobs.stats();
            if after.mesh_results == before.mesh_results {
                // The byte branch, not the count cap, refused this completion.
                assert_eq!(
                    after.mesh_result_bytes, before.mesh_result_bytes,
                    "a byte-refused completion changed the retained bytes"
                );
                assert!(
                    before.mesh_results < MAX_MESH_RESULTS,
                    "the count cap fired at {} results before the byte bound: {sizes:?}",
                    before.mesh_results
                );
                assert!(
                    before.mesh_result_bytes + sizes[admitted] > MAX_MESH_RESULT_BYTES,
                    "the byte bound did not actually fire: {before:?} + {}",
                    sizes[admitted]
                );
                assert!(
                    jobs.request_mesh(&world, refused_key),
                    "a byte-refused key was not requestable again"
                );
                eprintln!(
                    "byte saturation: {admitted} results retained, {} bytes; refused {refused_key:?} of {} bytes (bound {}); sizes {sizes:?}",
                    before.mesh_result_bytes, sizes[admitted], MAX_MESH_RESULT_BYTES
                );
                assert!(jobs.stats().within_bounds());
                return;
            }
        }
        panic!("no byte refusal occurred; workload too small: {sizes:?}");
    }

    /// Eviction and re-entry. Publication replaces the window in one step, so a
    /// resident window is never partially supported: the old window stays complete
    /// until the replacement is published, and the replacement is complete the
    /// moment it is. The evicted window is gone until it is requested again, when
    /// the authoritative source restores it, edit included.
    #[test]
    fn eviction_and_rerequest_keep_every_published_window_complete() {
        const SEED: u64 = 20260912;
        let origin_eye = [0.0, 4.0, 0.0];
        let far_eye = [120.0, 4.0, 0.0];
        let edit = [0, 20, 0];
        let mut world = streamed(SEED, origin_eye);
        assert_eq!(world.get(edit), 0);
        assert!(world.set(edit, 9));
        let mut jobs = AsyncWorld::manual();

        // The published window is exactly the declared span, not a residue of the
        // previous one, and every sampled interior position is supported.
        assert_window_supported(&world, origin_eye);

        // Preparation does not disturb the resident window: while the replacement is
        // in flight and while it is staged, the origin window stays complete.
        assert!(jobs.request_stream(&world, far_eye));
        let job = take_job(&mut jobs.shared.lock());
        assert_window_supported(&world, origin_eye);
        run_job(&jobs.shared, job);
        assert_eq!(jobs.stats().stream_results, MAX_STREAM_RESULTS);
        assert_window_supported(&world, origin_eye);

        // One publication moves residency: the far window is complete and the origin
        // window is gone, rather than half-evicted.
        assert!(jobs.poll_stream(&mut world));
        assert_window_supported(&world, far_eye);
        assert!(!world.stream_contains_position(origin_eye, 1.0));
        assert_eq!(world.get(edit), 0, "an evicted chunk stayed materialized");

        // Re-entry restores equivalent content from the authoritative overrides.
        assert!(jobs.request_stream(&world, origin_eye));
        let job = take_job(&mut jobs.shared.lock());
        run_job(&jobs.shared, job);
        assert!(jobs.poll_stream(&mut world));
        assert_window_supported(&world, origin_eye);
        assert_eq!(
            world.get(edit),
            9,
            "the edit did not survive eviction and re-entry"
        );

        // The restored window equals a world that reached it through the synchronous
        // path in the same order, so eviction dropped nothing and duplicated nothing.
        let mut reference = streamed(SEED, origin_eye);
        assert!(reference.set(edit, 9));
        assert!(reference.stream_around(far_eye));
        assert!(reference.stream_around(origin_eye));
        assert_worlds_equal(&world, &reference);
    }

    /// D2.3 primary-plan gap: sustained edits must still advance toward the current
    /// destination without accepting a stale snapshot or losing an edit. Drives the
    /// exact sequence the explorer tick invokes per frame (`request_stream`, then
    /// `poll_stream`, then `sync_fallback_if_stalled`, with physics sync on either
    /// publication), with an edit inside every preparation interval. Without the
    /// fallback the same loop publishes nothing (see
    /// `stale_stream_refusal_defers_until_one_preparation_interval_is_edit_free`);
    /// with it, exactly one bounded synchronous rewindow fires after
    /// `STALE_STREAM_FALLBACK_AFTER` consecutive edit-stale completions.
    #[test]
    fn sustained_edits_advance_through_the_app_invoked_fallback_without_edit_loss() {
        const SEED: u64 = 20260912;
        let origin_eye = [0.0, 4.0, 0.0];
        let storm_eye = [120.0, 4.0, 0.0];
        let edit = [0, 20, 0];
        let mut world = streamed(SEED, origin_eye);
        assert_eq!(world.get(edit), 0);
        let mut jobs = AsyncWorld::manual();
        let after = AsyncWorld::STALE_STREAM_FALLBACK_AFTER as usize;
        // Mirror log for the synchronous replay: the edit and whether the
        // fallback fired that frame, in order.
        let mut frames: Vec<(([i32; 3], u8), bool)> = Vec::new();
        let mut fallbacks = 0_usize;
        let mut published_at = None;
        for frame in 0..after + 4 {
            let material = 9 + (frame % 2) as u8;
            // App tick order: latest destination first, then publish, then the
            // bounded fallback. Each frame carries an edit, so every async
            // completion in this loop is stale by design.
            assert!(
                jobs.request_stream(&world, storm_eye),
                "frame {frame}: the request was not queued"
            );
            let job = take_job(&mut jobs.shared.lock());
            assert!(world.set(edit, material), "frame {frame}: edit refused");
            run_job(&jobs.shared, job);
            assert!(
                !jobs.poll_stream(&mut world),
                "frame {frame}: a stale window was published"
            );
            let fallback = jobs.sync_fallback_if_stalled(&mut world, storm_eye);
            frames.push(((edit, material), fallback));
            if fallback {
                fallbacks += 1;
                published_at = Some(frame);
                // The fallback moved residency to the storm window, so the origin
                // edit is evicted with its chunk (not materialized), preserved as
                // a stored override until re-entry. Assert both halves.
                assert_eq!(world.get(edit), 0, "an evicted chunk stayed materialized");
                break;
            }
            assert_eq!(
                jobs.consecutive_stale_streams(),
                (frame + 1) as u32,
                "stale count did not track consecutive edit-stale completions"
            );
            assert!(jobs.stats().within_bounds());
        }
        assert_eq!(fallbacks, 1, "exactly one bounded fallback must fire");
        assert_eq!(
            published_at,
            Some(after - 1),
            "fallback must fire after STALE_STREAM_FALLBACK_AFTER stale completions"
        );
        assert_eq!(jobs.consecutive_stale_streams(), 0);
        assert_eq!(jobs.stats().consecutive_stale_streams, 0);
        assert!(jobs.stats().within_bounds());
        assert_window_supported(&world, storm_eye);
        // Re-entry restores the edits the fallback carried past eviction.
        let last_material = frames.last().unwrap().0 .1;
        let (queued, prepared, published) = async_frame(&mut jobs, &mut world, origin_eye, None);
        assert!(queued && prepared && published);
        assert_window_supported(&world, origin_eye);
        assert_eq!(
            world.get(edit),
            last_material,
            "the edit did not survive fallback eviction and re-entry"
        );
        // Nothing was lost or merged: replaying the same edits with a synchronous
        // rewindow exactly where the fallback fired, plus the edit-free return,
        // reaches the identical world.
        let mut reference = streamed(SEED, origin_eye);
        for ((cell, material), fallback) in &frames {
            assert!(reference.set(*cell, *material));
            if *fallback {
                assert!(reference.stream_around(storm_eye));
            }
        }
        assert!(reference.stream_around(origin_eye));
        assert_worlds_equal(&world, &reference);
        assert_eq!(world.get(edit), last_material);
    }

    /// The fallback fires only for repeated stream-specific stale completions for
    /// the still-outstanding destination. Ordinary pending work and mesh-only
    /// churn never count; reversal, cancellation and successful publication reset.
    #[test]
    fn stall_fallback_ignores_pending_and_mesh_churn_and_resets_on_reversal_cancel_and_success() {
        let origin_eye = [0.0, 4.0, 0.0];
        let eye_a = [120.0, 4.0, 0.0];
        let eye_b = [-120.0, 4.0, 0.0];
        let edit = [0, 20, 0];
        let after = AsyncWorld::STALE_STREAM_FALLBACK_AFTER;

        // Pending work alone never counts and never falls back.
        let mut world = streamed(20260912, origin_eye);
        let mut jobs = AsyncWorld::manual();
        assert!(jobs.request_stream(&world, eye_a));
        assert_eq!(jobs.consecutive_stale_streams(), 0);
        assert!(!jobs.sync_fallback_if_stalled(&mut world, eye_a));
        assert!(jobs.stats().within_bounds());
        // Drain edit-free: publishes without counting anything.
        let job = take_job(&mut jobs.shared.lock());
        run_job(&jobs.shared, job);
        assert!(jobs.poll_stream(&mut world));
        assert_eq!(jobs.consecutive_stale_streams(), 0);
        assert_window_supported(&world, eye_a);

        // Mesh-only churn never counts and never falls back.
        let mut world = streamed(20260912, origin_eye);
        let mut jobs = AsyncWorld::manual();
        let key = world
            .chunk_keys()
            .into_iter()
            .next()
            .expect("resident chunk");
        let cell = [
            key[0] * CHUNK_EDGE,
            key[1] * CHUNK_EDGE,
            key[2] * CHUNK_EDGE,
        ];
        let material = if world.get(cell) == 9 { 8 } else { 9 };
        assert!(jobs.request_mesh(&world, key));
        assert!(world.set(cell, material), "mesh-churn edit refused");
        jobs.drain();
        assert!(jobs.poll_mesh(&world).is_none(), "stale mesh was published");
        assert!(jobs.stats().discarded >= 1);
        assert_eq!(
            jobs.consecutive_stale_streams(),
            0,
            "mesh churn changed the stream-specific stall count"
        );
        assert!(!jobs.sync_fallback_if_stalled(&mut world, eye_a));
        assert!(jobs.stats().within_bounds());

        // Reversal resets: stale completions for A must not force a fallback for
        // B. Two stales build a count of 2 for A; one stale interval toward B
        // must then count 1 for B, not 3, which proves the reversal reset.
        let mut world = streamed(20260912, origin_eye);
        let mut jobs = AsyncWorld::manual();
        for material in [9, 8] {
            let (queued, prepared, published) =
                async_frame(&mut jobs, &mut world, eye_a, Some((edit, material)));
            assert!(queued && prepared && !published);
        }
        assert_eq!(jobs.consecutive_stale_streams(), 2);
        let (queued, prepared, published) =
            async_frame(&mut jobs, &mut world, eye_b, Some((edit, 7)));
        assert!(queued && prepared && !published);
        assert_eq!(
            jobs.consecutive_stale_streams(),
            1,
            "reversal inherited the abandoned destination's stall count"
        );
        assert!(!jobs.sync_fallback_if_stalled(&mut world, eye_b));
        // Asking for the abandoned destination also refuses the fallback: the
        // count belongs to the current destination only.
        assert!(!jobs.sync_fallback_if_stalled(&mut world, eye_a));
        assert!(jobs.stats().within_bounds());

        // Success resets: two stales, then an edit-free interval publishes.
        let mut world = streamed(20260912, origin_eye);
        let mut jobs = AsyncWorld::manual();
        for material in [9, 8] {
            let (queued, prepared, published) =
                async_frame(&mut jobs, &mut world, eye_a, Some((edit, material)));
            assert!(queued && prepared && !published);
        }
        assert_eq!(jobs.consecutive_stale_streams(), 2);
        let (queued, prepared, published) = async_frame(&mut jobs, &mut world, eye_a, None);
        assert!(queued && prepared && published);
        assert_eq!(jobs.consecutive_stale_streams(), 0);
        assert_window_supported(&world, eye_a);

        // Cancellation resets, and the fallback stays bounded to one sync window
        // per STALE_STREAM_FALLBACK_AFTER stale completions, twice in a row.
        let mut world = streamed(20260912, origin_eye);
        let mut jobs = AsyncWorld::manual();
        for frame in 0..after {
            let material = 9 + (frame % 2) as u8;
            let (queued, prepared, published) =
                async_frame(&mut jobs, &mut world, eye_a, Some((edit, material)));
            assert!(queued && prepared && !published);
        }
        assert_eq!(jobs.consecutive_stale_streams(), after);
        jobs.reset();
        assert_eq!(jobs.consecutive_stale_streams(), 0);
        assert!(!jobs.sync_fallback_if_stalled(&mut world, eye_a));
        let mut applied = 0;
        for leg in 0..2 {
            for frame in 0..after {
                // Every edit lands while the origin window is still resident;
                // the fallback moves residency only on its own frame. Materials
                // start at 7/8: the pre-reset loop above left 9 in the cell, and
                // `set` refuses an unchanged material.
                let material = 7 + ((leg * after + frame) % 2) as u8;
                let (queued, prepared, published) =
                    async_frame(&mut jobs, &mut world, eye_a, Some((edit, material)));
                assert!(queued && prepared && !published);
                assert!(jobs.stats().within_bounds());
            }
            assert_eq!(jobs.consecutive_stale_streams(), after);
            assert!(
                jobs.sync_fallback_if_stalled(&mut world, eye_a),
                "leg {leg}: threshold reached but no fallback fired"
            );
            applied += 1;
            assert_eq!(jobs.consecutive_stale_streams(), 0);
            assert_window_supported(&world, eye_a);
            if leg == 0 {
                // Return edit-free so the next leg's edits are resident again.
                let (queued, prepared, published) =
                    async_frame(&mut jobs, &mut world, origin_eye, None);
                assert!(queued && prepared && published);
                assert_eq!(jobs.consecutive_stale_streams(), 0);
                assert_window_supported(&world, origin_eye);
            }
        }
        assert_eq!(applied, 2, "fallback must fire exactly once per threshold");
        assert_window_supported(&world, eye_a);
        assert!(jobs.stats().within_bounds());

        // A reached destination clears an outstanding stall instead of leaving it
        // armed for a later frame. Built from a real count, then the destination is
        // reached by another means (the synchronous path), so the assertion is not
        // satisfied by the reset a fallback or a publication would have done.
        let mut world = streamed(20260912, origin_eye);
        let mut jobs = AsyncWorld::manual();
        for frame in 0..after {
            let material = 9 + (frame % 2) as u8;
            let (queued, prepared, published) =
                async_frame(&mut jobs, &mut world, eye_a, Some((edit, material)));
            assert!(queued && prepared && !published);
        }
        assert_eq!(jobs.consecutive_stale_streams(), after);
        assert!(world.stream_around(eye_a), "synchronous rewindow refused");
        assert!(!jobs.sync_fallback_if_stalled(&mut world, eye_a));
        assert_eq!(
            jobs.consecutive_stale_streams(),
            0,
            "a reached destination kept an armed stall"
        );
        // A stall counted for another destination is left alone.
        let mut world = streamed(20260912, origin_eye);
        let mut jobs = AsyncWorld::manual();
        for material in [9, 8] {
            let (queued, prepared, published) =
                async_frame(&mut jobs, &mut world, eye_a, Some((edit, material)));
            assert!(queued && prepared && !published);
        }
        assert_eq!(jobs.consecutive_stale_streams(), 2);
        assert!(!jobs.sync_fallback_if_stalled(&mut world, origin_eye));
        assert_eq!(
            jobs.consecutive_stale_streams(),
            2,
            "reaching an unrelated destination cleared another destination's stall"
        );
    }

    /// Upstream rejection path: `run_job` discards a completed preparation when
    /// `stream_requested` has already advanced to a newer revision, so the work
    /// never reaches `poll_stream`. Real schedule from the second frame on: the
    /// next frame's request lands, then the previous frame's job completes and is
    /// rejected upstream, then the new job is taken, an edit lands, and the poll
    /// finds nothing staged. Repeating this must still drive the bounded fallback
    /// through the exact app-invoked sequence; counting only in `poll_stream`
    /// never fires here, so the sandbox would stall under sustained edits.
    #[test]
    fn upstream_discard_with_next_request_before_completion_still_drives_fallback() {
        const SEED: u64 = 20260912;
        let origin_eye = [0.0, 4.0, 0.0];
        let storm_eye = [120.0, 4.0, 0.0];
        let edit = [0, 20, 0];
        let mut world = streamed(SEED, origin_eye);
        assert_eq!(world.get(edit), 0);
        let mut jobs = AsyncWorld::manual();
        let after = AsyncWorld::STALE_STREAM_FALLBACK_AFTER as usize;
        // Job taken last frame: it always completes after this frame's request.
        let mut carried: Option<Job> = None;
        let mut edits: Vec<([i32; 3], u8)> = Vec::new();
        let mut fallbacks = 0_usize;
        let mut published_at = None;
        for frame in 0..after + 4 {
            let material = 9 + (frame % 2) as u8;
            // Latest destination first, exactly like the explorer tick.
            assert!(
                jobs.request_stream(&world, storm_eye),
                "frame {frame}: the request was not queued"
            );
            // The previous frame's job completes after this request: with an edit
            // already past its snapshot revision, the worker rejects it upstream
            // and stages nothing.
            if let Some(job) = carried.take() {
                let discarded_before = jobs.stats().discarded;
                run_job(&jobs.shared, job);
                assert_eq!(
                    jobs.stats().stream_results,
                    0,
                    "frame {frame}: an edit-stale completion was staged"
                );
                assert!(
                    jobs.stats().discarded > discarded_before,
                    "frame {frame}: the upstream completion was not discarded"
                );
            }
            carried = Some(take_job(&mut jobs.shared.lock()));
            assert!(world.set(edit, material), "frame {frame}: edit refused");
            edits.push((edit, material));
            assert!(
                !jobs.poll_stream(&mut world),
                "frame {frame}: a stale window was published"
            );
            if jobs.sync_fallback_if_stalled(&mut world, storm_eye) {
                fallbacks += 1;
                published_at = Some(frame);
                // Residency moved: the origin edit is evicted with its chunk and
                // kept as a stored override until re-entry.
                assert_eq!(world.get(edit), 0, "an evicted chunk stayed materialized");
                break;
            }
            // One upstream edit-stale discard per frame after the first.
            assert_eq!(
                jobs.consecutive_stale_streams(),
                frame as u32,
                "upstream discard did not count toward the stall"
            );
            assert!(jobs.stats().within_bounds());
        }
        assert_eq!(fallbacks, 1, "exactly one bounded fallback must fire");
        assert_eq!(
            published_at,
            Some(after),
            "fallback must fire after STALE_STREAM_FALLBACK_AFTER upstream discards"
        );
        assert_eq!(jobs.consecutive_stale_streams(), 0);
        assert!(jobs.stats().within_bounds());
        assert_window_supported(&world, storm_eye);
        // Re-entry restores the edits the fallback carried past eviction, and the
        // round trip equals the ordered synchronous replay.
        let last_material = edits.last().copied().unwrap().1;
        let (queued, prepared, published) = async_frame(&mut jobs, &mut world, origin_eye, None);
        assert!(queued && prepared && published);
        assert_window_supported(&world, origin_eye);
        assert_eq!(
            world.get(edit),
            last_material,
            "the edit did not survive fallback eviction and re-entry"
        );
        let mut reference = streamed(SEED, origin_eye);
        for (cell, material) in &edits {
            assert!(reference.set(*cell, *material));
        }
        assert!(reference.stream_around(storm_eye));
        assert!(reference.stream_around(origin_eye));
        assert_worlds_equal(&world, &reference);
    }

    /// Controls for the upstream counting path: only an edit on the still-wanted
    /// destination counts there. An abandoned destination, a seed replacement, a
    /// retired generation and mesh completions must not raise or disturb the
    /// count, and one rejected preparation must be counted exactly once even when
    /// it is displaced while staged rather than rejected at `poll_stream`.
    #[test]
    fn upstream_discard_counts_only_edit_stale_work_for_the_wanted_destination() {
        const SEED: u64 = 20260912;
        let origin_eye = [0.0, 4.0, 0.0];
        let eye_a = [120.0, 4.0, 0.0];
        let eye_b = [-120.0, 4.0, 0.0];
        let edit = [0, 20, 0];

        // Two stales toward A, then a reversal to B while A's job is in flight:
        // A's late completion belongs to the abandoned destination and must not
        // re-raise the count the reversal cleared.
        let mut world = streamed(SEED, origin_eye);
        let mut jobs = AsyncWorld::manual();
        for material in [9, 8] {
            let (queued, prepared, published) =
                async_frame(&mut jobs, &mut world, eye_a, Some((edit, material)));
            assert!(queued && prepared && !published);
        }
        assert_eq!(jobs.consecutive_stale_streams(), 2);
        assert!(jobs.request_stream(&world, eye_a));
        let job = take_job(&mut jobs.shared.lock());
        assert!(world.set(edit, 9));
        assert!(jobs.request_stream(&world, eye_b));
        assert_eq!(jobs.consecutive_stale_streams(), 0);
        run_job(&jobs.shared, job);
        assert_eq!(
            jobs.consecutive_stale_streams(),
            0,
            "a completion for the abandoned destination counted upstream"
        );

        // A world replacement with a different seed is not an edit stall: the
        // completion is discarded and the count clears.
        let mut world = streamed(SEED, origin_eye);
        let mut jobs = AsyncWorld::manual();
        for material in [9, 8] {
            let (queued, prepared, published) =
                async_frame(&mut jobs, &mut world, eye_a, Some((edit, material)));
            assert!(queued && prepared && !published);
        }
        assert_eq!(jobs.consecutive_stale_streams(), 2);
        assert!(jobs.request_stream(&world, eye_a));
        let job = take_job(&mut jobs.shared.lock());
        let world = streamed(SEED + 1, origin_eye);
        assert!(jobs.request_stream(&world, eye_a));
        run_job(&jobs.shared, job);
        assert_eq!(
            jobs.consecutive_stale_streams(),
            0,
            "a seed replacement counted as an edit stall"
        );

        // A completion stamped with a retired generation is nobody's work: it
        // neither counts nor disturbs the live count rebuilt after the reset.
        let mut world = streamed(SEED, origin_eye);
        let mut jobs = AsyncWorld::manual();
        assert!(jobs.request_stream(&world, eye_a));
        let job = take_job(&mut jobs.shared.lock());
        jobs.reset();
        assert_eq!(jobs.consecutive_stale_streams(), 0);
        for material in [9, 8] {
            let (queued, prepared, published) =
                async_frame(&mut jobs, &mut world, eye_a, Some((edit, material)));
            assert!(queued && prepared && !published);
        }
        assert_eq!(jobs.consecutive_stale_streams(), 2);
        run_job(&jobs.shared, job);
        assert_eq!(
            jobs.consecutive_stale_streams(),
            2,
            "a retired completion disturbed the live stall count"
        );
        // Mesh work completing under an outstanding stall changes nothing either.
        let key = world
            .chunk_keys()
            .into_iter()
            .next()
            .expect("resident chunk");
        assert!(jobs.request_mesh(&world, key));
        jobs.drain();
        assert_eq!(
            jobs.consecutive_stale_streams(),
            2,
            "a mesh completion disturbed the stream-specific stall count"
        );
        assert!(jobs.stats().within_bounds());

        // Displacement counts the displaced preparation exactly once, and staging
        // the fresh one counts nothing: the fresh one is still current, so it
        // publishes and clears instead of counting a second time.
        let mut world = streamed(SEED, origin_eye);
        let mut jobs = AsyncWorld::manual();
        assert!(jobs.request_stream(&world, eye_a));
        let job = take_job(&mut jobs.shared.lock());
        run_job(&jobs.shared, job);
        assert_eq!(jobs.stats().stream_results, 1);
        assert_eq!(jobs.consecutive_stale_streams(), 0);
        assert!(world.set(edit, 9));
        assert!(jobs.request_stream(&world, eye_a));
        let job = take_job(&mut jobs.shared.lock());
        run_job(&jobs.shared, job);
        assert_eq!(jobs.stats().stream_results, 1);
        assert_eq!(
            jobs.consecutive_stale_streams(),
            1,
            "the displaced preparation was not counted exactly once"
        );
        assert!(jobs.poll_stream(&mut world));
        assert_eq!(jobs.consecutive_stale_streams(), 0);
        assert_window_supported(&world, eye_a);
        // The window moved, so the origin edit is evicted with its chunk and kept
        // as a stored override until re-entry.
        assert_eq!(world.get(edit), 0);
        assert!(jobs.stats().within_bounds());
    }
}
