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

/// Live counts for debugging and integration; not a stable metrics contract.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AsyncStats {
    pub queued_meshes: usize,
    /// Pending window preparations: at most one, the most recent request.
    pub queued_streams: usize,
    /// Jobs currently executing on the worker: at most one.
    pub inflight: usize,
    pub mesh_results: usize,
    pub mesh_result_bytes: usize,
    /// Prepared windows awaiting `poll_stream`: at most one.
    pub stream_results: usize,
    /// Results dropped by saturation, cancellation or superseding requests.
    pub discarded: u64,
    pub generation: u64,
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
    prefer_mesh: bool,
    mesh_results: VecDeque<MeshResult>,
    mesh_result_bytes: usize,
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

fn mesh_bytes(mesh: &Mesh) -> usize {
    mesh.vertices.capacity() * std::mem::size_of::<Vertex>() + mesh.indices.capacity() * 4
}

/// Bounded background preparation attached to one authoritative world.
///
/// Snapshot cost: three retained window clones (pending, executing, completed),
/// plus one transient clone while replacing a pending job. Each holds resident
/// voxels and edit overrides: at most (147 + 512) * 4096 B, about 2.6 MiB of
/// voxel payload per clone, excluding attachment and collection overhead.
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
        self.requested_center = Some(center);
        if !world.is_streaming() || world.stream_center() == Some(center) {
            let mut queue = self.shared.lock();
            queue.stream = None;
            queue.stream_result = None;
            return false;
        }
        let mut queue = self.shared.lock();
        if queue.shutdown {
            return false;
        }
        let generation = queue.generation;
        let identity = (center, world.revision(), world.seed(), generation);
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
        let superseded = queue
            .stream
            .replace(StreamJob {
                world: world.clone(),
                position: eye,
                center,
                source_revision: world.revision(),
                seed: world.seed(),
                generation,
            })
            .is_some();
        queue.discarded += u64::from(superseded);
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
            return false;
        }
        drop(queue);
        *world = result.world;
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
        // Saturation would stop invalidating in-flight results; queues still clear,
        // and revision validation continues to reject anything the world outgrew.
        queue.generation = queue.generation.saturating_add(1);
        drop(queue);
        self.requested_center = None;
    }

    /// False after worker startup failure or an unexpected worker exit.
    pub fn available(&self) -> bool {
        self.worker
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
        // Alternate when both classes are ready: sustained travel cannot starve
        // retained-chunk geometry. Each service unit remains bounded.
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
        drop(queue);

        match job {
            Job::Stream(job) => {
                let mut world = job.world;
                world.stream_around(job.position);
                let mut queue = shared.lock();
                queue.inflight = 0;
                queue.stream_active = None;
                if queue.shutdown || queue.generation != job.generation {
                    queue.discarded += 1;
                    continue;
                }
                let superseded = queue
                    .stream_result
                    .replace(StreamResult {
                        world,
                        center: job.center,
                        source_revision: job.source_revision,
                        seed: job.seed,
                        generation: job.generation,
                    })
                    .is_some();
                queue.discarded += u64::from(superseded);
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
                    continue;
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
}
