//! Bounded background preparation and a byte-budgeted CPU cache for the
//! landscape sample's distance-ring tiles.
//!
//! A tile mesh is a pure function of the generator identity
//! ([`LANDSCAPE_GENERATOR_VERSION`], [`TerrainSource`], seed), the tile's level
//! and key and the normalized [`TileFilter`] the plan asked for. Those five
//! things are the cache key, so a mesh can only ever be reused for the identity
//! that generated it. Generation runs on one bounded worker thread; the main
//! thread only uploads meshes the worker or the cache produced.
//!
//! Hysteresis keeps a dropped tile's mesh for a short, capped window, so a
//! camera that hovers on a ring boundary does not regenerate the same meshes
//! every few frames. The hold is a pin on the CPU cache, not deferred GPU
//! residency: a tile the plan drops leaves the renderer at once, because a held
//! tile can sit under the new plan's coarser ring and draw the same ground
//! twice. Both the pinned set and the cache bytes are bounded, so a flight that
//! never stops cannot grow either one.
use crate::landscape::normalized_filter;
use matterweave_core::{
    landscape::{self, RingTile, TileFilter, LANDSCAPE_GENERATOR_VERSION},
    Mesh, TerrainSource,
};
use matterweave_render::{Renderer, TerrainTileKey, MAX_TERRAIN_TILES};
use std::{
    collections::{BTreeMap, BTreeSet},
    mem::size_of,
    sync::{
        mpsc::{sync_channel, Receiver, SyncSender, TrySendError},
        Arc, Mutex, PoisonError,
    },
    thread::JoinHandle,
    time::Instant,
};

/// Tile meshes built and uploaded in one frame.
pub const MAX_TILE_UPLOADS: usize = 8;
/// Main-thread milliseconds one frame may spend collecting, requesting and
/// uploading tiles. Checked before each tile, so one tile can overrun it; the
/// next cannot start. Tile generation itself no longer runs here.
pub const MAX_TILE_MS: f64 = 2.0;
/// Mesh jobs the background worker may hold at once.
const MAX_TILE_JOBS: usize = 48;
/// Completed meshes waiting to be collected. Count and bytes are both capped,
/// so a slow main thread cannot let the worker buffer grow.
const MAX_TILE_RESULTS: usize = 8;
const MAX_TILE_RESULT_BYTES: usize = 2 * 1024 * 1024;
/// Tile mesh jobs handed to the worker per frame.
const MAX_TILE_REQUESTS: usize = 8;
/// Frames a tile the plan dropped stays pinned in the CPU mesh cache.
pub const TILE_HYSTERESIS_FRAMES: u64 = 30;
/// Most dropped tiles pinned at once, bounding what the hysteresis can add to
/// the cache's live set.
pub const TILE_HYSTERESIS_HELD: usize = 24;
/// Byte budget of the CPU-side tile mesh cache. Resident meshes already own
/// their GPU copies; this only holds meshes evicted from the plan so a return
/// is an upload, not a regeneration.
pub const TILE_CACHE_BYTES: usize = 8 * 1024 * 1024;

/// Per-frame limits for the tile plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TileBudget {
    /// Tiles that may be built and uploaded this frame.
    pub uploads: usize,
    /// Hard bound on resident tiles, including the ones built this frame.
    pub resident: usize,
}

impl Default for TileBudget {
    fn default() -> Self {
        Self {
            uploads: MAX_TILE_UPLOADS,
            resident: MAX_TERRAIN_TILES,
        }
    }
}

/// How long a dropped mesh stays pinned in the cache, and how many may be
/// pinned at once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hysteresis {
    pub frames: u64,
    pub max_held: usize,
}

impl Hysteresis {
    /// Keep nothing: a dropped tile is evicted the same frame. Used by tests
    /// that assert the unbuffered plan behaviour.
    #[cfg(test)]
    pub const NONE: Self = Self {
        frames: 0,
        max_held: 0,
    };
}

/// What one frame should do to the tile cache.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TilePlanWork {
    /// Keys the renderer must keep resident. Exactly the plan: a tile the plan
    /// drops is never held on the GPU, because a held tile can sit under the
    /// new plan's coarser ring and draw the same ground twice.
    pub declared: Vec<TerrainTileKey>,
    /// Resident tiles to drop now. Every tile the plan no longer wants goes;
    /// hysteresis is a CPU-cache pin, not deferred residency.
    pub evict: Vec<TerrainTileKey>,
    /// Tiles to build and upload, in plan order: nearest ring first.
    pub build: Vec<RingTile>,
    /// Tiles the plan still needs after this frame's budget is spent.
    pub outstanding: usize,
    /// Recently dropped tiles, with the filter they were built with. Their
    /// meshes are pinned in the CPU cache for a bounded window.
    pub pinned: Vec<(TerrainTileKey, TileFilter)>,
}

impl TilePlanWork {
    fn clear(&mut self) {
        self.declared.clear();
        self.evict.clear();
        self.build.clear();
        self.outstanding = 0;
        self.pinned.clear();
    }
}

/// Identity of one generated tile mesh. Part of the cache key: a mesh is only
/// ever served for the exact generator identity, tile and filter that produced
/// it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct TileMeshKey {
    pub generator_version: u32,
    pub source: TerrainSource,
    pub seed: u64,
    pub level: u32,
    pub key: [i32; 2],
    pub filter: TileFilter,
}

impl TileMeshKey {
    pub fn new(
        source: TerrainSource,
        seed: u64,
        level: u32,
        key: [i32; 2],
        filter: TileFilter,
    ) -> Self {
        Self {
            generator_version: LANDSCAPE_GENERATOR_VERSION,
            source,
            seed,
            level,
            key,
            filter,
        }
    }
}

fn mesh_bytes(mesh: &Mesh) -> usize {
    mesh.vertices.capacity() * size_of::<matterweave_core::Vertex>()
        + mesh.indices.capacity() * size_of::<u32>()
}

struct CacheEntry {
    mesh: Arc<Mesh>,
    bytes: usize,
    last_use: u64,
}

/// Byte-budgeted CPU cache of generated tile meshes. Eviction is least
/// recently used, with the key as a deterministic tie-break, so the same
/// sequence of requests and hits always evicts the same entry. A recently
/// dropped tile can be pinned for a bounded number of frames: its mesh then
/// survives the trim, so a plan that comes back to it is an upload, not a
/// regeneration.
pub struct TileMeshCache {
    entries: BTreeMap<TileMeshKey, CacheEntry>,
    bytes: usize,
    budget: usize,
    clock: u64,
    /// Frame through which a key may not be evicted while unpinned entries
    /// remain. Bounded by the caller to the hysteresis cap.
    pinned_until: BTreeMap<TileMeshKey, u64>,
}

impl TileMeshCache {
    pub fn new(budget: usize) -> Self {
        Self {
            entries: BTreeMap::new(),
            bytes: 0,
            budget,
            clock: 0,
            pinned_until: BTreeMap::new(),
        }
    }

    /// Drop pins that have expired.
    pub fn begin_frame(&mut self, frame: u64) {
        self.pinned_until.retain(|_, until| *until > frame);
    }

    /// Keep `key` in the cache through `until` (exclusive) even under byte
    /// pressure, while unpinned entries remain.
    pub fn pin(&mut self, key: TileMeshKey, until: u64) {
        self.pinned_until.insert(key, until);
    }

    /// The cached mesh for `key`, marking it as most recently used.
    pub fn get(&mut self, key: &TileMeshKey) -> Option<Arc<Mesh>> {
        let clock = self.clock;
        self.clock += 1;
        let entry = self.entries.get_mut(key)?;
        entry.last_use = clock;
        Some(Arc::clone(&entry.mesh))
    }

    pub fn contains(&self, key: &TileMeshKey) -> bool {
        self.entries.contains_key(key)
    }

    /// Insert a mesh and return the stored handle. The handle stays valid even
    /// if the entry is trimmed immediately, so the caller can still upload it.
    pub fn insert(&mut self, key: TileMeshKey, mut mesh: Mesh) -> Arc<Mesh> {
        // Measure the payload the cache actually holds, not the growth slack.
        mesh.vertices.shrink_to_fit();
        mesh.indices.shrink_to_fit();
        let mesh = Arc::new(mesh);
        let bytes = mesh_bytes(&mesh);
        if let Some(old) = self.entries.insert(
            key,
            CacheEntry {
                mesh: Arc::clone(&mesh),
                bytes,
                last_use: self.clock,
            },
        ) {
            self.bytes -= old.bytes;
        }
        self.clock += 1;
        self.bytes += bytes;
        self.trim();
        mesh
    }

    /// Trim to the budget. Unpinned entries go first, least recently used;
    /// only when every entry is pinned does a pin (the earliest expiring) get
    /// evicted, so the cache can always return under its byte budget.
    fn trim(&mut self) {
        while self.bytes > self.budget {
            let unpinned = self
                .entries
                .iter()
                .filter(|(key, _)| !self.pinned_until.contains_key(key))
                .min_by_key(|(key, entry)| (entry.last_use, **key))
                .map(|(key, _)| *key);
            let victim = unpinned.or_else(|| {
                self.pinned_until
                    .iter()
                    .filter(|(key, _)| self.entries.contains_key(key))
                    .min_by_key(|(key, until)| (**until, **key))
                    .map(|(key, _)| *key)
            });
            let Some(key) = victim else {
                break;
            };
            self.pinned_until.remove(&key);
            if let Some(entry) = self.entries.remove(&key) {
                self.bytes -= entry.bytes;
            }
        }
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }

    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }
}

#[derive(Default)]
struct WorkerState {
    /// Keys queued, generating or complete but uncollected. Deduplicates
    /// requests so one tile is never generated twice concurrently.
    active: BTreeSet<TileMeshKey>,
    /// Bytes of completed meshes waiting to be collected.
    result_bytes: usize,
    shutdown: bool,
}

/// One bounded background worker that generates tile meshes off the main
/// thread. If the thread cannot be started every request is refused and
/// [`available`](Self::available) stays false, which makes the caller keep the
/// synchronous path instead of accumulating work nobody executes.
pub struct TileMeshWorker {
    jobs: Option<SyncSender<TileMeshKey>>,
    results: Option<Receiver<(TileMeshKey, Mesh)>>,
    state: Arc<Mutex<WorkerState>>,
    worker: Option<JoinHandle<()>>,
    seed: u64,
}

impl TileMeshWorker {
    /// Start the worker for the landscape generator at `seed`.
    pub fn landscape(seed: u64) -> Self {
        let (job_tx, job_rx) = sync_channel(MAX_TILE_JOBS);
        let (result_tx, result_rx) = sync_channel(MAX_TILE_RESULTS);
        let state = Arc::new(Mutex::new(WorkerState::default()));
        let worker = std::thread::Builder::new()
            .name("matterweave-tiles".into())
            .spawn({
                let state = Arc::clone(&state);
                move || run_worker(job_rx, result_tx, state)
            })
            .ok();
        if worker.is_none() {
            state
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .shutdown = true;
        }
        Self {
            jobs: Some(job_tx),
            results: Some(result_rx),
            state,
            worker,
            seed,
        }
    }

    /// False after worker startup failure or an unexpected worker exit.
    pub fn available(&self) -> bool {
        !self
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .shutdown
            && self
                .worker
                .as_ref()
                .is_some_and(|worker| !worker.is_finished())
    }

    /// Keys queued or generating, for the log.
    pub fn pending(&self) -> usize {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .active
            .len()
    }

    /// Queue one tile. Refused for a key this worker does not generate, a
    /// duplicate, a saturated queue or a dead worker; the caller may ask again
    /// on a later frame.
    pub fn request(&self, key: TileMeshKey) -> bool {
        if key.source != TerrainSource::Landscape
            || key.seed != self.seed
            || key.generator_version != LANDSCAPE_GENERATOR_VERSION
        {
            return false;
        }
        let Some(jobs) = &self.jobs else {
            return false;
        };
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.shutdown || state.active.contains(&key) {
            return false;
        }
        match jobs.try_send(key) {
            Ok(()) => {
                state.active.insert(key);
                true
            }
            Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => false,
        }
    }

    /// One completed mesh, oldest first. Stale or superseded results were
    /// already dropped by the worker's bounds; the key always names the mesh.
    pub fn poll(&mut self) -> Option<(TileMeshKey, Mesh)> {
        let (key, mesh) = self.results.as_ref()?.try_recv().ok()?;
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.active.remove(&key);
        state.result_bytes = state.result_bytes.saturating_sub(mesh_bytes(&mesh));
        Some((key, mesh))
    }
}

impl Drop for TileMeshWorker {
    fn drop(&mut self) {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .shutdown = true;
        // Closing the job channel wakes an idle worker; a generating worker
        // observes the flag after its one bounded job.
        self.jobs.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn run_worker(
    jobs: Receiver<TileMeshKey>,
    results: SyncSender<(TileMeshKey, Mesh)>,
    state: Arc<Mutex<WorkerState>>,
) {
    while let Ok(key) = jobs.recv() {
        let mesh = landscape::lod_tile_mesh(key.seed, key.level, key.key, key.filter);
        let bytes = mesh_bytes(&mesh);
        let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.shutdown {
            state.active.remove(&key);
            return;
        }
        if state.result_bytes + bytes > MAX_TILE_RESULT_BYTES {
            // The collector is behind; drop this result and let the caller ask
            // again rather than buffering without bound.
            state.active.remove(&key);
            continue;
        }
        match results.try_send((key, mesh)) {
            Ok(()) => state.result_bytes += bytes,
            Err(_) => {
                state.active.remove(&key);
            }
        }
    }
}

/// Resident tiles and the frame history that decides which dropped meshes stay
/// pinned in the CPU cache. Pure bookkeeping: no GPU or thread state, so the
/// plan and hysteresis are unit-testable without a renderer.
#[derive(Default)]
pub struct TileResidency {
    resident: BTreeMap<TerrainTileKey, TileFilter>,
    /// Last frame each key was in the plan.
    last_plan: BTreeMap<TerrainTileKey, u64>,
    wanted: BTreeSet<TerrainTileKey>,
    evicting: BTreeSet<TerrainTileKey>,
    pinned: Vec<(u64, TerrainTileKey, TileFilter)>,
}

impl TileResidency {
    #[cfg(test)]
    pub fn resident_len(&self) -> usize {
        self.resident.len()
    }

    pub fn filter(&self, key: &TerrainTileKey) -> Option<&TileFilter> {
        self.resident.get(key)
    }

    /// Drop bookkeeping when the renderer goes away. The GPU tiles die with
    /// it; the CPU cache and worker are renderer-independent and survive.
    pub fn forgot_renderer(&mut self) {
        self.resident.clear();
        self.last_plan.clear();
    }

    pub fn mark_uploaded(&mut self, key: TerrainTileKey, filter: TileFilter) {
        self.resident.insert(key, filter);
    }

    pub fn remove(&mut self, key: TerrainTileKey) {
        self.resident.remove(&key);
    }

    /// Diff the plan against residency, applying the frame's build budget and
    /// the hysteresis window. Writes into `out`, whose buffers keep their
    /// capacity, so a steady state allocates nothing.
    pub fn plan_into(
        &mut self,
        plan: &[RingTile],
        frame: u64,
        budget: TileBudget,
        hysteresis: Hysteresis,
        out: &mut TilePlanWork,
    ) {
        out.clear();
        for tile in plan {
            self.last_plan.insert((tile.level, tile.key), frame);
        }
        self.wanted.clear();
        self.wanted
            .extend(plan.iter().map(|tile| (tile.level, tile.key)));

        // Resident tiles the plan no longer wants go now; the recently wanted
        // ones additionally pin their cached mesh for the hysteresis window,
        // so a return is an upload from the CPU cache, not a regeneration. A
        // dropped tile is never held on the GPU: it could overlap the new
        // plan's coarser ring and draw the same ground twice.
        self.pinned.clear();
        for (key, filter) in &self.resident {
            if self.wanted.contains(key) {
                continue;
            }
            out.evict.push(*key);
            if let Some(stamp) = self.last_plan.get(key).copied() {
                if frame.saturating_sub(stamp) <= hysteresis.frames {
                    self.pinned.push((stamp, *key, *filter));
                }
            }
        }
        if self.pinned.len() > hysteresis.max_held {
            // Keep the most recently wanted; the oldest dropped are not pinned.
            self.pinned
                .sort_unstable_by_key(|(stamp, key, _)| (*stamp, *key));
            let excess = self.pinned.len() - hysteresis.max_held;
            self.pinned.drain(..excess);
        }
        out.evict.sort_unstable();
        out.evict.dedup();
        out.pinned
            .extend(self.pinned.iter().map(|(_, key, filter)| (*key, *filter)));

        // Stamps only matter while a key is resident or in the plan; anything
        // older than the window is dropped so a long flight cannot grow this.
        self.last_plan.retain(|key, stamp| {
            self.wanted.contains(key)
                || self.resident.contains_key(key)
                || frame.saturating_sub(*stamp) <= hysteresis.frames
        });

        out.declared
            .extend(plan.iter().map(|tile| (tile.level, tile.key)));

        self.evicting.clear();
        self.evicting.extend(out.evict.iter().copied());
        let kept = self.resident.len() - out.evict.len();
        let mut new_keys = 0usize;
        for tile in plan {
            let key = (tile.level, tile.key);
            let filter = normalized_filter(tile);
            if self.resident.get(&key) == Some(&filter) {
                continue;
            }
            // Only growth is bounded: rebuilding a tile that is already
            // resident replaces its buffers and cannot push the cache over
            // its limit.
            let grows = !self.resident.contains_key(&key);
            let blocked = grows && kept + new_keys + 1 > budget.resident;
            if out.build.len() >= budget.uploads || blocked {
                out.outstanding += 1;
                continue;
            }
            new_keys += usize::from(grows);
            out.build.push(*tile);
        }
        debug_assert!(!out
            .build
            .iter()
            .any(|tile| self.evicting.contains(&(tile.level, tile.key))));
    }
}

/// Resident tiles, the CPU cache and the background worker that keep the
/// distance rings filled. Owns no renderer state: [`sync`](Self::sync) takes
/// one for the frame's uploads.
pub struct TileStream {
    source: TerrainSource,
    seed: u64,
    residency: TileResidency,
    cache: TileMeshCache,
    worker: TileMeshWorker,
    work: TilePlanWork,
    counters: TileCounters,
    budget: TileBudget,
    hysteresis: Hysteresis,
}

/// Counters the HUD shows and the smoke line prints. Every one is a count the
/// sample actually performed, not a target.
#[derive(Clone, Copy, Debug, Default)]
pub struct TileCounters {
    pub uploaded: u64,
    pub evicted: u64,
    pub outstanding: usize,
    /// Main-thread time spent collecting, requesting and uploading tiles,
    /// including the frame's planning.
    pub build_ms: f64,
    pub declare_ms: f64,
    /// Main-thread mesh generation, nonzero only on the synchronous fallback.
    pub generate_ms: f64,
    /// The part of `build_ms` spent inside `upload_terrain_tile`, which waits
    /// on the frame fence before it replaces a buffer.
    pub upload_ms: f64,
    /// Total main-thread generation time over the run, and the meshes it
    /// covers. Their ratio is the only synchronous per-tile cost there is.
    pub total_generate_ms: f64,
    /// Meshes the worker produced (or the fallback generated) over the run.
    pub generated: u64,
    /// Jobs handed to the worker this frame.
    pub requested: u32,
    /// Builds served by the CPU cache instead of the generator.
    pub served_from_cache: u64,
    /// Recently dropped meshes pinned in the CPU cache this frame.
    pub pinned: usize,
    /// Worker jobs queued or generating.
    pub pending: usize,
    /// The single worst frame seen so far, by build time.
    pub worst: WorstFrame,
}

/// The most expensive tile frame of a run.
#[derive(Clone, Copy, Debug, Default)]
pub struct WorstFrame {
    pub tiles: usize,
    pub build_ms: f64,
    pub generate_ms: f64,
    pub upload_ms: f64,
    pub declare_ms: f64,
}

impl TileStream {
    /// Build the stream for the landscape generator at `seed`.
    pub fn new(source: TerrainSource, seed: u64) -> Self {
        // Distance tiles are defined only for the landscape generator; a
        // legacy world here would be keyed as landscape and wrong, so refuse
        // the construction instead of generating the wrong mesh.
        assert_eq!(
            source,
            TerrainSource::Landscape,
            "distance tiles require the landscape generator"
        );
        Self {
            source,
            seed,
            residency: TileResidency::default(),
            cache: TileMeshCache::new(TILE_CACHE_BYTES),
            worker: TileMeshWorker::landscape(seed),
            work: TilePlanWork::default(),
            counters: TileCounters::default(),
            budget: TileBudget::default(),
            hysteresis: Hysteresis {
                frames: TILE_HYSTERESIS_FRAMES,
                max_held: TILE_HYSTERESIS_HELD,
            },
        }
    }

    pub fn counters(&self) -> TileCounters {
        self.counters
    }

    pub fn cache_bytes(&self) -> usize {
        self.cache.bytes()
    }

    pub fn cache_entries(&self) -> usize {
        self.cache.entry_count()
    }

    /// Drop the previous renderer's residency. The cache and the worker hold
    /// no GPU objects and stay, so a resume re-uploads instead of regenerating.
    pub fn forgot_renderer(&mut self) {
        self.residency.forgot_renderer();
        self.counters.outstanding = 0;
    }

    fn key(&self, tile: &RingTile) -> TileMeshKey {
        TileMeshKey::new(
            self.source,
            self.seed,
            tile.level,
            tile.key,
            normalized_filter(tile),
        )
    }

    /// Apply one frame's plan: declare residency, evict, collect generated
    /// meshes, request what is missing and upload within the budget.
    pub fn sync(
        &mut self,
        renderer: &mut Renderer,
        plan: &[RingTile],
        frame: u64,
    ) -> Result<(), String> {
        let declare_begin = Instant::now();
        self.cache.begin_frame(frame);
        self.residency
            .plan_into(plan, frame, self.budget, self.hysteresis, &mut self.work);
        // Declare first: the renderer drops every tile the plan no longer wants
        // before this frame adds anything, so GPU residency never exceeds the
        // plan plus this frame's budget. A dropped tile stays in the CPU cache,
        // pinned for the hysteresis window, so a return is an upload.
        renderer.retain_terrain_tiles(&self.work.declared)?;
        let declare_ms = declare_begin.elapsed().as_secs_f64() * 1000.;
        for key in &self.work.evict {
            self.residency.remove(*key);
        }
        self.counters.evicted += self.work.evict.len() as u64;
        for (key, filter) in &self.work.pinned {
            let mesh_key = TileMeshKey::new(self.source, self.seed, key.0, key.1, *filter);
            self.cache.pin(mesh_key, frame + self.hysteresis.frames + 1);
        }
        self.counters.pinned = self.work.pinned.len();

        let build_begin = Instant::now();
        // Collect finished meshes first, so one that completed this frame can
        // also be uploaded this frame.
        while let Some((key, mesh)) = self.worker.poll() {
            self.cache.insert(key, mesh);
            self.counters.generated += 1;
        }
        let mut requested = 0u32;
        for tile in &self.work.build {
            let key = self.key(tile);
            if self.cache.contains(&key) {
                continue;
            }
            if requested as usize >= MAX_TILE_REQUESTS {
                break;
            }
            if self.worker.request(key) {
                requested += 1;
            }
        }

        let mut uploaded = 0u64;
        let mut built = 0usize;
        let mut generate_ms = 0.0;
        let mut upload_ms = 0.0;
        let mut served = 0u64;
        for tile in &self.work.build {
            if build_begin.elapsed().as_secs_f64() * 1000. >= MAX_TILE_MS {
                break;
            }
            let key = self.key(tile);
            let mesh = match self.cache.get(&key) {
                Some(mesh) => {
                    served += 1;
                    mesh
                }
                None if !self.worker.available() => {
                    // Background preparation is unavailable: generate on the
                    // main thread rather than leaving a hole in the rings.
                    let generate_begin = Instant::now();
                    let mesh =
                        landscape::lod_tile_mesh(self.seed, tile.level, tile.key, key.filter);
                    let elapsed = generate_begin.elapsed().as_secs_f64() * 1000.;
                    generate_ms += elapsed;
                    self.counters.total_generate_ms += elapsed;
                    self.counters.generated += 1;
                    self.cache.insert(key, mesh)
                }
                // Still being generated; a later frame uploads it.
                None => continue,
            };
            built += 1;
            // A tile entirely inside the hole meshes to nothing. It stays a
            // planned, resident-as-empty tile so it is not rebuilt every frame.
            if !mesh.indices.is_empty() {
                let upload_begin = Instant::now();
                renderer.upload_terrain_tile(tile.level, tile.key, &mesh)?;
                upload_ms += upload_begin.elapsed().as_secs_f64() * 1000.;
                uploaded += 1;
            }
            self.residency
                .mark_uploaded((tile.level, tile.key), key.filter);
        }
        let build_ms = build_begin.elapsed().as_secs_f64() * 1000.;

        self.counters.declare_ms = declare_ms;
        self.counters.build_ms = build_ms;
        self.counters.generate_ms = generate_ms;
        self.counters.upload_ms = upload_ms;
        self.counters.uploaded += uploaded;
        self.counters.served_from_cache += served;
        self.counters.requested = requested;
        self.counters.pending = self.worker.pending();
        // Outstanding is what the plan still wants after this frame: a tile
        // whose generated mesh was not ready yet counts, because the next
        // frame must not consider the rings complete.
        self.counters.outstanding = plan
            .iter()
            .filter(|tile| {
                self.residency.filter(&(tile.level, tile.key)) != Some(&normalized_filter(tile))
            })
            .count();
        if build_ms > self.counters.worst.build_ms {
            self.counters.worst = WorstFrame {
                tiles: built,
                build_ms,
                generate_ms,
                upload_ms,
                declare_ms,
            };
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::landscape::SEED;
    use matterweave_core::landscape::{fine_clip, ring_plan, LANDSCAPE_RINGS};
    use matterweave_core::Vertex;
    use std::time::Duration;

    fn plan_at(eye: [f32; 3]) -> Vec<RingTile> {
        ring_plan(eye, fine_clip(eye), &LANDSCAPE_RINGS)
    }

    fn apply(residency: &mut TileResidency, work: &TilePlanWork) {
        for key in &work.evict {
            residency.remove(*key);
        }
        for tile in &work.build {
            residency.mark_uploaded((tile.level, tile.key), normalized_filter(tile));
        }
    }

    #[test]
    fn a_full_plan_is_built_over_several_frames_within_the_budget() {
        let plan = plan_at([0.0, 40.0, 0.0]);
        let mut residency = TileResidency::default();
        let budget = TileBudget::default();
        let mut work = TilePlanWork::default();
        let mut frames = 0;
        loop {
            residency.plan_into(&plan, frames, budget, Hysteresis::NONE, &mut work);
            if work.build.is_empty() {
                assert_eq!(work.outstanding, 0, "nothing buildable but work remaining");
                break;
            }
            assert!(work.build.len() <= budget.uploads);
            assert!(work.evict.is_empty(), "a stable plan evicts nothing");
            apply(&mut residency, &work);
            frames += 1;
            assert!(frames < 1000, "tile work never converged");
        }
        assert_eq!(residency.resident_len(), plan.len());
        assert_eq!(frames, plan.len().div_ceil(budget.uploads) as u64);
        // A fully resident plan asks for nothing at all.
        residency.plan_into(&plan, frames, budget, Hysteresis::NONE, &mut work);
        assert_eq!(work.build.len(), 0);
        assert_eq!(work.evict.len(), 0);
        assert_eq!(work.outstanding, 0);
        assert_eq!(work.pinned.len(), 0);
    }

    #[test]
    fn moving_evicts_dropped_tiles_and_rebuilds_only_what_changed() {
        let here = plan_at([0.0, 40.0, 0.0]);
        let mut residency = TileResidency::default();
        let mut work = TilePlanWork::default();
        // The whole plan resident, as it is after the fill frames.
        for tile in &here {
            residency.mark_uploaded((tile.level, tile.key), normalized_filter(tile));
        }

        // One chunk of movement keeps every tile key, but moves the streaming
        // window, so only the tiles the window cuts may be rebuilt.
        let nudged = plan_at([16.0, 40.0, 0.0]);
        assert_eq!(
            nudged.iter().map(|t| (t.level, t.key)).collect::<Vec<_>>(),
            here.iter().map(|t| (t.level, t.key)).collect::<Vec<_>>()
        );
        residency.plan_into(
            &nudged,
            1,
            TileBudget::default(),
            Hysteresis::NONE,
            &mut work,
        );
        assert!(work.evict.is_empty(), "the tile set did not change");
        let rebuilt = work.build.len() + work.outstanding;
        assert!(
            (1..=16).contains(&rebuilt),
            "one chunk of movement rebuilt {rebuilt} tiles"
        );
        assert!(work
            .build
            .iter()
            .all(|tile| tile.level == LANDSCAPE_RINGS[0].level));

        // A long jump drops most tiles from the plan; with no hysteresis they
        // all go in the same frame and the new plan starts from the budget.
        let far = plan_at([4096.0, 40.0, 4096.0]);
        residency.plan_into(&far, 2, TileBudget::default(), Hysteresis::NONE, &mut work);
        assert!(
            work.evict.len() > here.len() / 2,
            "a jump of two rings should drop most of the cache"
        );
        assert_eq!(work.build.len(), MAX_TILE_UPLOADS);
        // A dropped tile leaves the GPU at once; nothing is held resident.
        let far_keys: BTreeSet<TerrainTileKey> =
            far.iter().map(|tile| (tile.level, tile.key)).collect();
        let shared = here
            .iter()
            .filter(|tile| far_keys.contains(&(tile.level, tile.key)))
            .count();
        assert_eq!(work.evict.len(), here.len() - shared);
        // Everything the new plan needs is either built now or still owed.
        let needed = far
            .iter()
            .filter(|tile| {
                residency.filter(&(tile.level, tile.key)) != Some(&normalized_filter(tile))
            })
            .count();
        assert_eq!(work.build.len() + work.outstanding, needed);
    }

    #[test]
    fn the_residency_bound_refuses_growth_instead_of_exceeding_it() {
        let plan = plan_at([0.0, 40.0, 0.0]);
        let mut residency = TileResidency::default();
        let mut work = TilePlanWork::default();
        let budget = TileBudget {
            uploads: MAX_TILE_UPLOADS,
            resident: 5,
        };
        residency.plan_into(&plan, 0, budget, Hysteresis::NONE, &mut work);
        assert_eq!(work.build.len(), 5);
        assert_eq!(work.build.len() + work.outstanding, plan.len());
        // Rebuilding a resident tile does not grow the cache, so it is allowed
        // even with the bound already reached.
        for tile in &plan {
            residency.mark_uploaded((tile.level, tile.key), normalized_filter(tile));
        }
        let moved = plan_at([16.0, 40.0, 0.0]);
        residency.plan_into(
            &moved,
            1,
            TileBudget {
                uploads: 8,
                resident: 1,
            },
            Hysteresis::NONE,
            &mut work,
        );
        assert!(
            !work.build.is_empty(),
            "a filter change on a resident tile must still be rebuilt"
        );
    }

    #[test]
    fn hysteresis_pins_recently_dropped_tiles_and_is_capped() {
        let here = plan_at([0.0, 40.0, 0.0]);
        let far = plan_at([4096.0, 40.0, 4096.0]);
        let far_keys: BTreeSet<TerrainTileKey> =
            far.iter().map(|tile| (tile.level, tile.key)).collect();
        let window = Hysteresis {
            frames: 30,
            max_held: TILE_HYSTERESIS_HELD,
        };
        let mut residency = TileResidency::default();
        let mut work = TilePlanWork::default();
        // Fill the same plan over its budgeted frames first.
        let mut frame = 0;
        loop {
            residency.plan_into(&here, frame, TileBudget::default(), window, &mut work);
            if work.build.is_empty() {
                break;
            }
            apply(&mut residency, &work);
            frame += 1;
            assert!(frame < 1000, "tile work never converged");
        }
        assert!(residency.resident_len() > TILE_HYSTERESIS_HELD);

        // The frame after the jump every dropped tile is evicted from the
        // plan, and only the capped, most recently wanted ones are pinned.
        let frame = frame + 1;
        let dropped = residency
            .resident
            .keys()
            .filter(|key| !far_keys.contains(key))
            .count();
        residency.plan_into(&far, frame, TileBudget::default(), window, &mut work);
        assert_eq!(work.evict.len(), dropped, "the GPU holds the new plan only");
        assert_eq!(work.pinned.len(), TILE_HYSTERESIS_HELD.min(dropped));
        assert!(work.pinned.iter().all(|(key, _)| !far_keys.contains(key)));
        assert_eq!(
            work.declared.len(),
            far.len(),
            "the declared set is exactly the plan"
        );

        // Past the window nothing is pinned, even with no new plan.
        residency.plan_into(
            &far,
            frame + window.frames + 1,
            TileBudget::default(),
            window,
            &mut work,
        );
        assert!(work.pinned.is_empty());
        assert_eq!(work.evict.len(), dropped);
    }

    #[test]
    fn hysteresis_is_bounded_and_does_not_grow_over_a_long_flight() {
        let budget = TileBudget::default();
        let window = Hysteresis {
            frames: TILE_HYSTERESIS_FRAMES,
            max_held: TILE_HYSTERESIS_HELD,
        };
        let mut residency = TileResidency::default();
        let mut work = TilePlanWork::default();
        let mut max_resident = 0usize;
        let mut max_stamps = 0usize;
        let mut max_pinned = 0usize;
        // A flight that never stops: 10 000 frames of 4 m east and 3 m north.
        for frame in 0..10_000u64 {
            let eye = [frame as f32 * 4.0, 40.0, frame as f32 * 3.0];
            let plan = plan_at(eye);
            residency.plan_into(&plan, frame, budget, window, &mut work);
            apply(&mut residency, &work);
            max_resident = max_resident.max(residency.resident_len());
            max_stamps = max_stamps.max(residency.last_plan.len());
            max_pinned = max_pinned.max(work.pinned.len());
            assert!(
                residency.resident_len() <= plan.len(),
                "frame {frame}: {} resident exceeds the plan",
                residency.resident_len()
            );
            assert!(work.pinned.len() <= window.max_held);
        }
        assert!(max_resident <= plan_at([40_000.0, 40.0, 30_000.0]).len());
        assert!(max_stamps <= max_resident + plan_at([0.0; 3]).len());
        assert!(max_pinned <= window.max_held);
    }

    /// Whether one tile's mesh includes the aligned cell whose centre is
    /// `centre`: the centre must be inside the tile's own square and pass its
    /// normalized filter.
    fn includes_cell(tile: &RingTile, centre: [i32; 2]) -> bool {
        let cell = landscape::lod_cell_m(tile.level);
        let span = landscape::LOD_TILE_CELLS * cell;
        let min = [tile.key[0] * span, tile.key[1] * span];
        let filter = normalized_filter(tile);
        centre[0] >= min[0]
            && centre[0] < min[0] + span
            && centre[1] >= min[1]
            && centre[1] < min[1] + span
            && filter.hole.is_none_or(|hole| !hole.contains_centre(centre))
            && filter
                .bound
                .is_none_or(|bound| bound.contains_centre(centre))
    }

    #[test]
    fn a_dropped_tile_overlaps_the_new_plan_so_it_must_leave_the_gpu() {
        // Hysteresis deliberately evicts the GPU copy immediately. Holding it
        // would be wrong: when the ring centre steps, a dropped tile of the
        // inner ring can sit under the new plan's coarser ring, so both meshes
        // would draw the same ground. This test shows that overlap exists for
        // the actual ring set, which is why `declared` is exactly the plan.
        for shift in [64.0f32, 128.0, 256.0, 512.0] {
            let here = plan_at([0.0, 40.0, 0.0]);
            let there = plan_at([shift, 40.0, 0.0]);
            let there_keys: BTreeSet<TerrainTileKey> =
                there.iter().map(|tile| (tile.level, tile.key)).collect();
            let overlapping = here
                .iter()
                .filter(|tile| !there_keys.contains(&(tile.level, tile.key)))
                .any(|old| {
                    let cell = landscape::lod_cell_m(old.level);
                    (0..landscape::LOD_TILE_CELLS).any(|cz| {
                        (0..landscape::LOD_TILE_CELLS).any(|cx| {
                            let centre = [
                                (old.key[0] * landscape::LOD_TILE_CELLS + cx) * cell + cell / 2,
                                (old.key[1] * landscape::LOD_TILE_CELLS + cz) * cell + cell / 2,
                            ];
                            includes_cell(old, centre)
                                && there.iter().any(|new| {
                                    (new.level, new.key) != (old.level, old.key)
                                        && includes_cell(new, centre)
                                })
                        })
                    })
                });
            assert!(
                overlapping,
                "a {shift} m ring step should drop a tile the new plan covers"
            );
        }
    }

    fn test_mesh(vertices: usize) -> Mesh {
        Mesh {
            vertices: vec![
                Vertex {
                    position: [0.0; 3],
                    normal: [0.0, 1.0, 0.0],
                    color: [1.0; 3],
                };
                vertices
            ],
            indices: vec![0; vertices],
            revision: 0,
        }
    }

    fn cache_key(level: u32, key: [i32; 2]) -> TileMeshKey {
        TileMeshKey::new(
            TerrainSource::Landscape,
            SEED,
            level,
            key,
            TileFilter::default(),
        )
    }

    #[test]
    fn the_cache_serves_a_hit_without_regenerating_and_evicts_lru() {
        let one = mesh_bytes(&test_mesh(100));
        assert!(one > 0);
        let mut cache = TileMeshCache::new(one * 2);
        let (a, b, c) = (
            cache_key(1, [0, 0]),
            cache_key(1, [1, 0]),
            cache_key(1, [2, 0]),
        );
        let first = cache.insert(a, test_mesh(100));
        cache.insert(b, test_mesh(100));
        // Touch `a` so `b` is the least recently used entry.
        let hit = cache.get(&a).expect("a is cached");
        assert!(
            Arc::ptr_eq(&first, &hit),
            "a hit must reuse the stored mesh, not rebuild it"
        );
        let third = cache.insert(c, test_mesh(100));
        assert!(cache.bytes() <= one * 2, "the byte budget bounds the cache");
        assert_eq!(cache.bytes(), one * 2, "the budget bounds the payload");
        assert!(cache.contains(&a) && cache.contains(&c));
        assert!(
            !cache.contains(&b),
            "the least recently used entry was evicted"
        );
        assert!(Arc::ptr_eq(&third, &cache.get(&c).unwrap()));
    }

    #[test]
    fn a_pinned_mesh_survives_trim_that_evicts_the_rest() {
        let one = mesh_bytes(&test_mesh(100));
        let mut cache = TileMeshCache::new(one * 2);
        let (a, b, c) = (
            cache_key(1, [0, 0]),
            cache_key(1, [1, 0]),
            cache_key(1, [2, 0]),
        );
        let pinned = cache.insert(a, test_mesh(100));
        cache.insert(b, test_mesh(100));
        // Pin `a` for 30 frames, then force a trim with a third mesh.
        cache.pin(a, 30);
        cache.insert(c, test_mesh(100));
        assert!(cache.bytes() <= one * 2, "the byte budget still bounds it");
        assert!(cache.contains(&a), "the pinned mesh survived the trim");
        assert!(Arc::ptr_eq(&pinned, &cache.get(&a).unwrap()));
        assert!(
            !cache.contains(&b),
            "the unpinned least recently used entry was evicted first"
        );
        // The pin expires; once nothing is pinned the ordinary LRU trim can
        // evict it again.
        cache.begin_frame(31);
        cache.get(&c).expect("c is cached");
        cache.insert(cache_key(1, [3, 0]), test_mesh(100));
        assert!(
            !cache.contains(&a),
            "an expired pin must not block eviction"
        );
    }

    #[test]
    fn a_mesh_is_never_served_for_a_different_generator_identity() {
        let base = TileMeshKey::new(
            TerrainSource::Landscape,
            SEED,
            2,
            [3, -4],
            TileFilter::default(),
        );
        let mut cache = TileMeshCache::new(usize::MAX);
        cache.insert(base, test_mesh(16));
        // A different filter may make a different mesh for the same tile.
        let filtered = TileMeshKey {
            filter: TileFilter {
                hole: Some(matterweave_core::landscape::Clip {
                    min: [0, 0],
                    max: [32, 32],
                }),
                bound: None,
            },
            ..base
        };
        assert!(!cache.contains(&filtered));
        // A different generator version or source is a different mesh too.
        assert!(!cache.contains(&TileMeshKey {
            generator_version: base.generator_version + 1,
            ..base
        }));
        assert!(!cache.contains(&TileMeshKey {
            source: TerrainSource::LegacyIsland,
            ..base
        }));
        assert!(!cache.contains(&TileMeshKey {
            seed: SEED + 1,
            ..base
        }));
    }

    #[test]
    fn the_worker_generates_one_requested_tile_and_reports_availability() {
        let key = cache_key(1, [0, 0]);
        let mut worker = TileMeshWorker::landscape(SEED);
        assert!(worker.available(), "the worker thread must start");
        assert!(worker.request(key));
        assert!(
            !worker.request(key),
            "a duplicate request must not queue twice"
        );
        // A key for another generator or seed is refused outright.
        assert!(!worker.request(TileMeshKey {
            seed: SEED + 1,
            ..key
        }));
        assert!(!worker.request(TileMeshKey {
            source: TerrainSource::LegacyIsland,
            ..key
        }));
        let deadline = Instant::now() + Duration::from_secs(10);
        let (got, mesh) = loop {
            if let Some(result) = worker.poll() {
                break result;
            }
            assert!(Instant::now() < deadline, "worker produced no tile");
            std::thread::sleep(Duration::from_millis(2));
        };
        assert_eq!(got, key);
        let direct = landscape::lod_tile_mesh(SEED, 1, [0, 0], TileFilter::default());
        assert_eq!(
            mesh.indices, direct.indices,
            "the worker must generate the same mesh as the direct call"
        );
        assert_eq!(mesh.vertices.len(), direct.vertices.len());
        // Collected work frees the key for a later request.
        assert!(worker.request(key));
    }
}
