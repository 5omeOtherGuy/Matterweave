//! Background far-terrain tile meshes and a bounded CPU-side tile cache.
//!
//! A ring tile mesh is a pure function of `(seed, generator version, level,
//! tile key, normalized filter)`: it reads no world state and no renderer state.
//! That makes two things possible, and this module owns both:
//!
//! - **Generation off the main thread.** [`TileWorker`] is one thread behind a
//!   bounded queue. It is deliberately not the world-preparation pool
//!   ([`matterweave_core::AsyncWorld`]): that pool's unit of work is a chunk
//!   halo taken from an authoritative [`matterweave_core::World`] and validated
//!   against a world revision, while a tile job is a symmetric pure-function
//!   call with no world and no revision. Reusing it would mean widening its job
//!   type and its staleness rule with an unrelated class of work; keeping them
//!   apart keeps each pool's contract checkable on its own.
//! - **A cache keyed by generator identity.** A tile the plan drops and wants
//!   again is re-uploaded from [`TileCache`] instead of regenerated. The key
//!   carries the seed, `LANDSCAPE_GENERATOR_VERSION`, the level, the tile key and
//!   a canonical encoding of the normalized filter, so an entry can never be
//!   served for a different surface.
//!
//! Every buffer here is bounded: the pending queue, the result queue, the result
//! bytes and the cache bytes. A saturated queue refuses a request, and the caller
//! asks again on a later frame; nothing grows with the length of a flight.
use matterweave_core::landscape::{
    self, Clip, RingTile, TileFilter, LANDSCAPE_GENERATOR_VERSION,
};
use matterweave_core::Mesh;
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError},
    thread::JoinHandle,
    time::Instant,
};use matterweave_render::TerrainTileKey;

/// Tile jobs waiting for the worker. Each holds one identity and one filter: no
/// geometry, so the queue is small in bytes as well as in count.
pub const MAX_PENDING_TILE_JOBS: usize = 16;
/// Completed tile meshes awaiting `poll`. A caller that stops polling cannot
/// make the worker allocate without bound.
pub const MAX_TILE_RESULTS: usize = 4;
/// Allocated bytes held in completed tile meshes. A single result is always
/// admitted into an empty queue, the same rule the world pool uses, so one
/// oversized tile cannot stall progress forever.
pub const MAX_TILE_RESULT_BYTES: usize = 8 * 1024 * 1024;
/// Default CPU cache budget for tile meshes. A tile mesh is about 180 KiB for
/// the levels this sample renders, so the budget holds roughly 180 tiles: the
/// resident set plus the tiles a moving eye recently dropped.
pub const DEFAULT_TILE_CACHE_BYTES: usize = 32 * 1024 * 1024;
/// Hard entry bound, independent of the byte budget. Two caps make the bound
/// hold even if a future level produces much smaller meshes.
pub const MAX_CACHED_TILES: usize = 512;

/// Generator identity: the seed and the version of the function that produced a
/// mesh. Part of the cache key, so a stale entry can never describe a different
/// generator.
pub type GeneratorId = (u64, u32);

/// Everything that can change the mesh of one tile.
///
/// The filter is stored as a canonical integer encoding rather than as
/// [`TileFilter`], so the key is `Ord` (the cache is a `BTreeMap`, whose
/// iteration order must not depend on a random hash seed) and comparisons are
/// exact with no hash collisions to reason about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct TileId {
    pub generator: GeneratorId,
    pub level: u32,
    pub key: [i32; 2],
    pub filter: [i64; 8],
}

/// Canonical encoding of a filter: hole min/max then bound min/max, each
/// component `i64::MIN` when that part is absent. Two filters encode equally
/// exactly when they mesh a tile identically.
fn filter_code(filter: TileFilter) -> [i64; 8] {
    let part = |clip: Option<Clip>| match clip {
        Some(clip) => [
            i64::from(clip.min[0]),
            i64::from(clip.min[1]),
            i64::from(clip.max[0]),
            i64::from(clip.max[1]),
        ],
        None => [i64::MIN; 4],
    };
    let hole = part(filter.hole);
    let bound = part(filter.bound);
    [
        hole[0], hole[1], hole[2], hole[3], bound[0], bound[1], bound[2], bound[3],
    ]
}

impl TileId {
    /// Identity of one tile mesh. `filter` must already be normalized
    /// (`crate::landscape::normalized_filter`) or equal tiles look different.
    pub fn new(generator: GeneratorId, tile: &RingTile, filter: TileFilter) -> Self {
        Self {
            generator,
            level: tile.level,
            key: tile.key,
            filter: filter_code(filter),
        }
    }
}

/// One tile the worker should generate.
#[derive(Clone, Copy, Debug)]
pub struct TileJob {
    pub id: TileId,
    pub seed: u64,
    pub tile: RingTile,
    pub filter: TileFilter,
}

impl TileJob {
    pub fn new(generator: GeneratorId, seed: u64, tile: &RingTile, filter: TileFilter) -> Self {
        Self {
            id: TileId::new(generator, tile, filter),
            seed,
            tile: *tile,
            filter,
        }
    }
}

/// Live counts for the sample's log line; not a stable metrics contract.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TileWorkerStats {
    pub pending: usize,
    pub inflight: usize,
    pub results: usize,
    pub result_bytes: usize,
    pub generated: u64,
    pub generate_ms: f64,
    pub discarded: u64,
}

impl TileWorkerStats {
    /// Whether every queue and buffer is inside its declared bound.
    pub fn within_bounds(&self) -> bool {
        self.pending <= MAX_PENDING_TILE_JOBS
            && self.inflight <= 1
            && self.results <= MAX_TILE_RESULTS
            && (self.results <= 1 || self.result_bytes <= MAX_TILE_RESULT_BYTES)
    }
}

struct Queue {
    pending: VecDeque<TileJob>,
    /// The job executing on the worker, if any. One unit keeps ordering obvious.
    active: Option<TileJob>,
    /// Identities queued or executing, so a duplicate request is refused rather
    /// than meshed twice. Bounded by the job bounds.
    claimed: BTreeSet<TileId>,
    results: VecDeque<(TileId, Mesh)>,
    result_bytes: usize,
    generated: u64,
    generate_ms: f64,
    discarded: u64,
    shutdown: bool,
}

struct Shared {
    queue: Mutex<Queue>,
    wake: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Queue> {
        // A panicking job must not disable the remaining synchronous path.
        self.queue.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn mesh_bytes(mesh: &Mesh) -> usize {
    mesh.vertices.capacity() * std::mem::size_of::<matterweave_core::Vertex>() + mesh.indices.capacity() * 4
}

/// One background worker generating tile meshes from a bounded queue.
pub struct TileWorker {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
}

impl TileWorker {
    pub fn new() -> Self {
        let shared = Arc::new(Shared {
            queue: Mutex::new(Queue {
                pending: VecDeque::new(),
                active: None,
                claimed: BTreeSet::new(),
                results: VecDeque::new(),
                result_bytes: 0,
                generated: 0,
                generate_ms: 0.,
                discarded: 0,
                shutdown: false,
            }),
            wake: Condvar::new(),
        });
        let worker = std::thread::Builder::new()
            .name("matterweave-tiles".into())
            .spawn({
                let shared = Arc::clone(&shared);
                move || run(&shared)
            })
            .ok();
        if worker.is_none() {
            // Without a worker every request is refused, so the caller falls back
            // to generating what it can instead of queueing work nobody runs.
            shared.lock().shutdown = true;
        }
        Self { shared, worker }
    }

    /// Queues one job. Refused when the queue is full, the worker is retired, or
    /// this identity is already queued, executing or awaiting a poll.
    pub fn request(&mut self, job: TileJob) -> bool {
        let mut queue = self.shared.lock();
        if queue.shutdown
            || queue.pending.len() >= MAX_PENDING_TILE_JOBS
            || queue.claimed.contains(&job.id)
            || queue.results.iter().any(|(id, _)| *id == job.id)
        {
            return false;
        }
        queue.claimed.insert(job.id);
        queue.pending.push_back(job);
        drop(queue);
        self.shared.wake.notify_all();
        true
    }

    /// Whether this identity is already queued, executing or awaiting a poll.
    pub fn claimed(&self, id: TileId) -> bool {
        let queue = self.shared.lock();
        queue.claimed.contains(&id) || queue.results.iter().any(|(queued, _)| *queued == id)
    }

    /// Returns one completed mesh. The identity is the whole contract: a result
    /// is a pure function of the identity it was requested with, so no staleness
    /// check is possible or needed. Refused results are dropped and can be
    /// requested again.
    pub fn poll(&mut self) -> Option<(TileId, Mesh)> {
        let mut queue = self.shared.lock();
        let (id, mesh) = queue.results.pop_front()?;
        queue.result_bytes -= mesh_bytes(&mesh);
        queue.claimed.remove(&id);
        Some((id, mesh))
    }

    /// False after worker startup failure or an unexpected worker exit.
    pub fn available(&self) -> bool {
        !self.shared.lock().shutdown
            && self
                .worker
                .as_ref()
                .is_some_and(|worker| !worker.is_finished())
    }

    pub fn stats(&self) -> TileWorkerStats {
        let queue = self.shared.lock();
        TileWorkerStats {
            pending: queue.pending.len(),
            inflight: usize::from(queue.active.is_some()),
            results: queue.results.len(),
            result_bytes: queue.result_bytes,
            generated: queue.generated,
            generate_ms: queue.generate_ms,
            discarded: queue.discarded,
        }
    }
}

impl Default for TileWorker {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for TileWorker {
    fn drop(&mut self) {
        {
            let mut queue = self.shared.lock();
            queue.shutdown = true;
            queue.discarded += (queue.pending.len() + queue.results.len() + queue.active.iter().count()) as u64;
            queue.pending.clear();
            queue.results.clear();
            queue.result_bytes = 0;
        }
        self.shared.wake.notify_all();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn run(shared: &Shared) {
    loop {
        let mut queue = shared.lock();
        while !queue.shutdown && queue.pending.is_empty() {
            queue = shared.wake.wait(queue).unwrap_or_else(PoisonError::into_inner);
        }
        if queue.shutdown {
            return;
        }
        let job = queue.pending.pop_front().expect("nonempty queue");
        queue.active = Some(job);
        drop(queue);
        let begin = Instant::now();
        let mesh = landscape::lod_tile_mesh(job.seed, job.tile.level, job.tile.key, job.filter);
        let elapsed_ms = begin.elapsed().as_secs_f64() * 1000.;
        let bytes = mesh_bytes(&mesh);
        let mut queue = shared.lock();
        queue.active = None;
        queue.generated += 1;
        queue.generate_ms += elapsed_ms;
        let full = queue.results.len() >= MAX_TILE_RESULTS
            || (!queue.results.is_empty() && queue.result_bytes + bytes > MAX_TILE_RESULT_BYTES);
        if queue.shutdown || full {
            // Nothing records the drop, so the caller can request the identity
            // again on a later frame.
            queue.claimed.remove(&job.id);
            queue.discarded += 1;
            continue;
        }
        queue.result_bytes += bytes;
        queue.results.push_back((job.id, mesh));
    }
}

/// CPU-side tile meshes, evicted by byte budget in least-recently-wanted order.
///
/// Hysteresis lives here rather than in the drawing set. The ring plan cuts a
/// square out of the finer tile that a coarser ring covers, so keeping a tile the
/// plan dropped **resident** would draw that square twice and weaken the coverage
/// guarantee the plan exists to provide. Keeping its *mesh* instead makes a tile
/// the eye returns to a re-upload rather than a regeneration, with no change to
/// what the frame draws.
#[derive(Default)]
pub struct TileCache {
    entries: BTreeMap<TileId, Entry>,
    bytes: usize,
    budget_bytes: usize,
    /// Frame counter stamped on every wanted identity; higher is more recent.
    clock: u64,
    hits: u64,
    misses: u64,
    evictions: u64,
}

struct Entry {
    mesh: Mesh,
    bytes: usize,
    last_wanted: u64,
}

/// Live counts for the sample's log line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TileCacheStats {
    pub entries: usize,
    pub bytes: usize,
    pub budget_bytes: usize,
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
}

impl TileCache {
    pub fn new(budget_bytes: usize) -> Self {
        Self {
            entries: BTreeMap::new(),
            bytes: 0,
            budget_bytes,
            clock: 0,
            hits: 0,
            misses: 0,
            evictions: 0,
        }
    }

    /// Starts a new frame. Every identity the plan wants is marked here, so a
    /// tile the eye returns to is the most recently wanted one and is evicted
    /// last under byte pressure.
    pub fn begin_frame(&mut self) {
        self.clock = self.clock.saturating_add(1);
    }

    /// Marks one wanted identity, reporting whether its mesh is cached. This is
    /// the only place a lookup is counted, so `hits` is "tiles the plan wanted
    /// and the cache could serve", not an internal probe count.
    pub fn want(&mut self, id: TileId) -> bool {
        match self.entries.get_mut(&id) {
            Some(entry) => {
                entry.last_wanted = self.clock;
                self.hits += 1;
                true
            }
            None => {
                self.misses += 1;
                false
            }
        }
    }

    pub fn get(&self, id: TileId) -> Option<&Mesh> {
        self.entries.get(&id).map(|entry| &entry.mesh)
    }

    /// Inserts or replaces a mesh and evicts least-recently-wanted entries until
    /// the cache fits its budget. The entry just inserted is never the one
    /// evicted: a single mesh larger than the whole budget still has to be
    /// usable, or the cache could not make progress at all.
    pub fn insert(&mut self, id: TileId, mesh: Mesh) {
        if let Some(previous) = self.entries.remove(&id) {
            self.bytes = self.bytes.saturating_sub(previous.bytes);
        }
        let bytes = mesh_bytes(&mesh);
        self.bytes += bytes;
        self.entries.insert(
            id,
            Entry {
                mesh,
                bytes,
                last_wanted: self.clock,
            },
        );
        while self.bytes > self.budget_bytes || self.entries.len() > MAX_CACHED_TILES {
            let Some(oldest) = self
                .entries
                .iter()
                .filter(|(key, _)| **key != id)
                .min_by_key(|(key, entry)| (entry.last_wanted, **key))
                .map(|(key, _)| *key)
            else {
                break;
            };
            self.remove_oldest(oldest);
        }
    }

    fn remove_oldest(&mut self, id: TileId) {
        if let Some(entry) = self.entries.remove(&id) {
            self.bytes = self.bytes.saturating_sub(entry.bytes);
            self.evictions += 1;
        }
    }

    pub fn stats(&self) -> TileCacheStats {
        TileCacheStats {
            entries: self.entries.len(),
            bytes: self.bytes,
            budget_bytes: self.budget_bytes,
            hits: self.hits,
            misses: self.misses,
            evictions: self.evictions,
        }
    }
}

/// Identity of a tile mesh for this sample's generator and seed.
pub fn generator_id() -> GeneratorId {
    (crate::landscape::SEED, LANDSCAPE_GENERATOR_VERSION)
}

/// Tile identity for one planned tile, with its filter normalized first.
pub fn tile_id(tile: &RingTile) -> TileId {
    TileId::new(
        generator_id(),
        tile,
        crate::landscape::normalized_filter(tile),
    )
}

/// The renderer's key for a tile: the level is part of it, so two rings never
/// collide in the renderer's tile cache.
pub fn renderer_key(tile: &RingTile) -> TerrainTileKey {
    (tile.level, tile.key)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tile(level: u32, key: [i32; 2], filter: TileFilter) -> RingTile {
        RingTile {
            level,
            key,
            filter,
        }
    }

    fn clip(min: [i32; 2], max: [i32; 2]) -> Clip {
        Clip { min, max }
    }

    fn mesh(vertices: usize) -> Mesh {
        let mut mesh = Mesh::default();
        mesh.vertices.resize(
            vertices,
            matterweave_core::Vertex {
                position: [0.; 3],
                normal: [0.; 3],
                color: [0.; 3],
            },
        );
        mesh.indices = vec![0; vertices];
        mesh
    }

    fn id(level: u32, key: [i32; 2], filter: TileFilter) -> TileId {
        TileId::new((7, LANDSCAPE_GENERATOR_VERSION), &tile(level, key, filter), filter)
    }

    #[test]
    fn identity_distinguishes_every_generator_input() {
        let base = id(1, [2, 3], TileFilter::default());
        assert_eq!(base, id(1, [2, 3], TileFilter::default()));
        // A different level, key, filter, generator version or seed is a
        // different surface and must not share an entry.
        assert_ne!(base, id(2, [2, 3], TileFilter::default()));
        assert_ne!(base, id(1, [3, 3], TileFilter::default()));
        assert_ne!(
            base,
            id(
                1,
                [2, 3],
                TileFilter {
                    hole: Some(clip([0, 0], [64, 64])),
                    bound: None
                }
            )
        );
        assert_ne!(
            base,
            TileId::new(
                (7, LANDSCAPE_GENERATOR_VERSION + 1),
                &tile(1, [2, 3], TileFilter::default()),
                TileFilter::default()
            )
        );
        assert_ne!(
            base,
            TileId::new(
                (8, LANDSCAPE_GENERATOR_VERSION),
                &tile(1, [2, 3], TileFilter::default()),
                TileFilter::default()
            )
        );
        // A hole and a bound are not interchangeable.
        assert_ne!(
            id(
                1,
                [0, 0],
                TileFilter {
                    hole: Some(clip([0, 0], [1, 1])),
                    bound: None
                }
            ),
            id(
                1,
                [0, 0],
                TileFilter {
                    hole: None,
                    bound: Some(clip([0, 0], [1, 1]))
                }
            )
        );
    }

    #[test]
    fn the_cache_serves_a_re_requested_tile_and_reports_it() {
        let mut cache = TileCache::new(DEFAULT_TILE_CACHE_BYTES);
        let wanted = id(0, [0, 0], TileFilter::default());
        cache.begin_frame();
        assert!(!cache.want(wanted), "an empty cache cannot serve a tile");
        cache.insert(wanted, mesh(64));
        cache.begin_frame();
        assert!(cache.want(wanted), "the cached tile must be served");
        assert!(cache.get(wanted).is_some());
        let stats = cache.stats();
        assert_eq!(stats.hits, 1);
        assert_eq!(stats.misses, 1);
        assert_eq!(stats.entries, 1);
        assert!(stats.bytes > 0);
    }

    #[test]
    fn the_cache_evicts_least_recently_wanted_first_and_stays_bounded() {
        // One 32-vertex mesh is 32 * 36 + 32 * 4 = 1280 bytes; budget for three.
        let mut cache = TileCache::new(3 * 1280);
        let oldest = id(0, [0, 0], TileFilter::default());
        let middle = id(0, [1, 0], TileFilter::default());
        let kept = id(0, [2, 0], TileFilter::default());
        for entry in [oldest, middle, kept] {
            cache.begin_frame();
            cache.want(entry);
            cache.insert(entry, mesh(32));
        }
        // The plan still wants the first identity, so it becomes the most
        // recently wanted one even though it is the oldest entry.
        cache.begin_frame();
        assert!(cache.want(oldest));
        let newest = id(0, [3, 0], TileFilter::default());
        cache.insert(newest, mesh(32));
        let stats = cache.stats();
        assert!(stats.bytes <= 3 * 1280, "budget exceeded: {}", stats.bytes);
        assert!(stats.entries <= MAX_CACHED_TILES);
        assert!(
            cache.get(oldest).is_some(),
            "a tile the plan still wants was evicted"
        );
        assert!(cache.get(newest).is_some(), "the newest tile was evicted");
        assert!(
            cache.get(middle).is_none(),
            "the least recently wanted entry should have gone first"
        );
        assert!(stats.evictions >= 1);
    }

    #[test]
    fn an_oversized_tile_is_still_admitted_to_an_empty_cache() {
        // A mesh larger than the budget must not be dropped on arrival, or the
        // sample could never draw it.
        let mut cache = TileCache::new(1024);
        let big = id(2, [0, 0], TileFilter::default());
        cache.insert(big, mesh(4096));
        assert!(cache.get(big).is_some());
        assert!(cache.stats().bytes > 1024);
        // The next insert evicts it rather than keeping both.
        let second = id(2, [1, 0], TileFilter::default());
        cache.insert(second, mesh(4096));
        assert!(cache.get(second).is_some());
        assert!(
            cache.get(big).is_none(),
            "the budget must still bound the cache"
        );
        assert_eq!(cache.stats().entries, 1);
    }

    #[test]
    fn replaced_entries_keep_the_byte_total_exact() {
        let mut cache = TileCache::new(DEFAULT_TILE_CACHE_BYTES);
        let entry = id(0, [0, 0], TileFilter::default());
        cache.insert(entry, mesh(64));
        let first = cache.stats().bytes;
        cache.insert(entry, mesh(32));
        assert!(
            cache.stats().bytes < first,
            "a replacement must not double count"
        );
        assert_eq!(cache.stats().entries, 1);
    }

    /// The worker generates real meshes for real tiles, off the calling thread,
    /// and refuses duplicates and unbounded queues. Deterministic assertions
    /// only: the mesh content is checked against the synchronous call, never a
    /// timing claim.
    #[test]
    fn the_worker_generates_the_mesh_the_synchronous_call_would() {
        let mut worker = TileWorker::new();
        if !worker.available() {
            // A host that cannot start a thread cannot test a thread; the
            // caller's fallback path is exercised instead.
            return;
        }
        let tile = tile(
            0,
            [0, 0],
            TileFilter {
                hole: Some(clip([0, 0], [64, 64])),
                bound: None,
            },
        );
        let filter = filter_code(tile.filter);
        let job = TileJob::new(generator_id(), crate::landscape::SEED, &tile, tile.filter);
        assert!(worker.request(job));
        assert!(
            !worker.request(job),
            "a duplicate identity must not be meshed twice"
        );
        let mut received = None;
        for _ in 0..2_000 {
            if let Some(result) = worker.poll() {
                received = Some(result);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let (id, mesh) = received.expect("the worker never published a mesh");
        assert_eq!(id.filter, filter);
        let expected = landscape::lod_tile_mesh(crate::landscape::SEED, 0, [0, 0], tile.filter);
        assert_eq!(mesh.indices, expected.indices);
        assert_eq!(mesh.vertices.len(), expected.vertices.len());
        for (a, b) in mesh.vertices.iter().zip(&expected.vertices) {
            assert_eq!(a.position, b.position);
        }
        let stats = worker.stats();
        assert_eq!(stats.generated, 1);
        assert!(stats.within_bounds(), "{stats:?}");
        assert_eq!(stats.results, 0, "the poll removed the result");
        // The identity is requestable again once its result was polled.
        assert!(worker.request(job));
    }

    #[test]
    fn a_saturated_queue_refuses_instead_of_growing() {
        let mut worker = TileWorker::new();
        if !worker.available() {
            return;
        }
        let mut accepted = 0;
        let mut refused = 0;
        for index in 0..MAX_PENDING_TILE_JOBS * 4 {
            let tile = tile(0, [index as i32, 0], TileFilter::default());
            let job = TileJob::new(generator_id(), crate::landscape::SEED, &tile, tile.filter);
            if worker.request(job) {
                accepted += 1;
            } else {
                refused += 1;
            }
            let stats = worker.stats();
            assert!(stats.within_bounds(), "{stats:?}");
            assert!(
                stats.pending + stats.inflight + stats.results <= MAX_PENDING_TILE_JOBS + 1 + MAX_TILE_RESULTS,
                "queues grew past their bounds: {stats:?}"
            );
        }
        // The worker drains while the queue fills, so the accepted count is
        // bounded by the queue bound plus whatever it completed.
        let stats = worker.stats();
        assert!(accepted <= MAX_PENDING_TILE_JOBS + 1 + MAX_TILE_RESULTS + stats.generated as usize);
        assert!(refused > 0);
    }

    #[test]
    fn a_dropped_worker_stops_without_leaving_queued_work() {
        let mut worker = TileWorker::new();
        if !worker.available() {
            return;
        }
        let tile = tile(0, [0, 0], TileFilter::default());
        let job = TileJob::new(generator_id(), crate::landscape::SEED, &tile, tile.filter);
        assert!(worker.request(job));
        // Dropping joins the worker; a queued job must not deadlock the join.
        drop(worker);
    }
}
