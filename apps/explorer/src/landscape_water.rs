//! Derived water surfaces for the landscape sample: an identity-keyed CPU cache
//! and window residency with hysteresis.
//!
//! A water mesh is a pure function of the generator identity (generator
//! version, terrain source and seed), the sea level and the chunk: the landscape
//! sample never edits authoritative terrain, so those are the whole cache key.
//! The key is the same generator identity
//! [`crate::landscape_tiles::TileMeshKey`] records for a tile mesh. Before this
//! cache existed, `sync_water` stamped every surface with the *world* revision,
//! which `stream_around` advances on every window move: every resident surface
//! was re-derived and re-uploaded on every chunk crossing, which saturated the
//! per-frame upload budget on a steady flight (2266 uploads over 300 frames for
//! 25 resident surfaces).
//!
//! The declaration rule is the tile stream's: the renderer holds exactly the
//! window's surfaces, because a surface left resident outside the window would
//! draw the same sea the innermost distance ring already draws. Hysteresis is
//! therefore a pin on the CPU cache, not deferred GPU residency: a surface the
//! window drops keeps its derived mesh for a bounded number of frames, so a
//! camera on a chunk edge that steps out and back is an upload, not a
//! re-derivation. Both the pinned set and the cache bytes are bounded.

use crate::mesh_cache::MeshCache;
use matterweave_core::{
    landscape::LANDSCAPE_GENERATOR_VERSION, water::sea_level_chunk_y, Mesh, TerrainSource, World,
};
use matterweave_render::Renderer;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
    time::Instant,
};

/// Water meshes derived and uploaded in one frame.
pub const MAX_WATER_UPLOADS: usize = 8;
/// Main-thread milliseconds one frame may spend deriving and uploading water.
pub const MAX_WATER_MS: f64 = 2.0;
/// Byte budget of the CPU-side derived-surface cache.
pub const WATER_CACHE_BYTES: usize = 2 * 1024 * 1024;
/// Frames a surface the window dropped stays pinned in the CPU cache.
pub const WATER_HYSTERESIS_FRAMES: u64 = 30;
/// Most dropped surfaces pinned at once, bounding what the hysteresis can add
/// to the cache's live set.
pub const WATER_HYSTERESIS_HELD: usize = 24;

/// Identity of one derived water surface.
///
/// Generator version, source and seed are the generator identity; sea level and
/// the chunk locate the surface. The landscape sample makes no authoritative
/// edits, so this is the whole identity. If the sample ever gains an edit path,
/// the terrain revision of the chunk column must join the key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct WaterMeshKey {
    pub generator_version: u32,
    pub source: TerrainSource,
    pub seed: u64,
    pub sea_level: i32,
    pub chunk: [i32; 3],
}

/// Identity of every surface this stream can serve. Fixed at construction: the
/// generator does not change under a running sample.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct WaterIdentity {
    generator_version: u32,
    source: TerrainSource,
    seed: u64,
    sea_level: i32,
}

impl WaterIdentity {
    fn mesh_key(self, chunk: [i32; 3]) -> WaterMeshKey {
        WaterMeshKey {
            generator_version: self.generator_version,
            source: self.source,
            seed: self.seed,
            sea_level: self.sea_level,
            chunk,
        }
    }
}

/// Per-frame limits for the water plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WaterBudget {
    /// Surfaces that may be derived and uploaded this frame.
    pub uploads: usize,
}

impl Default for WaterBudget {
    fn default() -> Self {
        Self {
            uploads: MAX_WATER_UPLOADS,
        }
    }
}

/// How long a dropped surface stays pinned in the CPU cache, and how many may
/// be pinned at once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hysteresis {
    pub frames: u64,
    pub max_held: usize,
}

impl Hysteresis {
    /// Keep nothing: a dropped surface is evicted the same frame. Used by tests
    /// that assert the unbuffered behaviour.
    #[cfg(test)]
    pub const NONE: Self = Self {
        frames: 0,
        max_held: 0,
    };
}

/// What one frame should do to the water cache and the renderer's surfaces.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WaterPlanWork {
    /// Sea-level chunks the renderer must keep resident. Exactly the window.
    pub declared: Vec<[i32; 3]>,
    /// Resident surfaces to drop now.
    pub evict: Vec<[i32; 3]>,
    /// Surfaces to derive or serve from the cache, in key order.
    pub build: Vec<[i32; 3]>,
    /// Surfaces the window still wants after this frame's budget is spent.
    pub outstanding: usize,
    /// Recently dropped surfaces whose derived mesh is pinned in the CPU cache.
    pub pinned: Vec<[i32; 3]>,
}

impl WaterPlanWork {
    fn clear(&mut self) {
        self.declared.clear();
        self.evict.clear();
        self.build.clear();
        self.outstanding = 0;
        self.pinned.clear();
    }
}

/// Resident water surfaces and the frame history that decides which dropped
/// meshes stay pinned. Pure bookkeeping: no GPU or cache state, so the plan and
/// hysteresis are unit-testable without a renderer.
#[derive(Default)]
pub struct WaterResidency {
    resident: BTreeSet<[i32; 3]>,
    /// Last frame each key was in the plan.
    last_plan: BTreeMap<[i32; 3], u64>,
    wanted: BTreeSet<[i32; 3]>,
}

impl WaterResidency {
    #[cfg(test)]
    pub fn resident_len(&self) -> usize {
        self.resident.len()
    }

    /// Drop bookkeeping when the renderer goes away. The GPU surfaces die with
    /// it; the CPU cache is renderer-independent and survives.
    pub fn forgot_renderer(&mut self) {
        self.resident.clear();
        self.last_plan.clear();
    }

    pub fn mark_uploaded(&mut self, key: [i32; 3]) {
        self.resident.insert(key);
    }

    pub fn remove(&mut self, key: [i32; 3]) {
        self.resident.remove(&key);
    }

    /// Diff the sea-level layer of a resident window against residency,
    /// applying the frame's build budget and the hysteresis window. Writes into
    /// `out`, whose buffers keep their capacity, so a steady state allocates
    /// nothing.
    ///
    /// Every other chunk layer of `keys` is ignored: only
    /// [`sea_level_chunk_y`] carries a water surface.
    pub fn plan_into(
        &mut self,
        keys: &[[i32; 3]],
        frame: u64,
        budget: WaterBudget,
        hysteresis: Hysteresis,
        out: &mut WaterPlanWork,
    ) {
        out.clear();
        let sea_level = sea_level_chunk_y();
        self.wanted.clear();
        for &key in keys {
            if key[1] == sea_level {
                self.wanted.insert(key);
                self.last_plan.insert(key, frame);
            }
        }

        // Surfaces the window no longer wants go now; the recently wanted ones
        // additionally pin their cached mesh for the hysteresis window, so a
        // return is an upload from the CPU cache, not a regeneration. A dropped
        // surface is never held on the GPU: it would overlap the innermost ring
        // that covers the same sea.
        let mut pinned: Vec<(u64, [i32; 3])> = Vec::new();
        for key in &self.resident {
            if self.wanted.contains(key) {
                continue;
            }
            out.evict.push(*key);
            if let Some(stamp) = self.last_plan.get(key).copied() {
                if frame.saturating_sub(stamp) <= hysteresis.frames {
                    pinned.push((stamp, *key));
                }
            }
        }
        if pinned.len() > hysteresis.max_held {
            // Keep the most recently wanted; the oldest dropped are not pinned.
            pinned.sort_unstable();
            let excess = pinned.len() - hysteresis.max_held;
            pinned.drain(..excess);
        }
        out.evict.sort_unstable();
        out.evict.dedup();
        out.pinned.extend(pinned.into_iter().map(|(_, key)| key));

        // Stamps only matter while a key is wanted, resident or inside the
        // hysteresis window; anything older is dropped so a long flight cannot
        // grow this.
        self.last_plan.retain(|key, stamp| {
            self.wanted.contains(key)
                || self.resident.contains(key)
                || frame.saturating_sub(*stamp) <= hysteresis.frames
        });

        out.declared.extend(self.wanted.iter().copied());

        for &key in &self.wanted {
            if self.resident.contains(&key) {
                continue;
            }
            if out.build.len() >= budget.uploads {
                out.outstanding += 1;
                continue;
            }
            out.build.push(key);
        }
    }
}

/// Counters the HUD shows and the smoke line prints. Every one is a count the
/// sample actually performed, not a target.
#[derive(Clone, Copy, Debug, Default)]
pub struct WaterCounters {
    /// Surfaces uploaded to the renderer over the run.
    pub uploaded: u64,
    /// Surfaces derived by the generator over the run.
    pub generated: u64,
    /// Builds served by the CPU cache instead of the generator.
    pub served_from_cache: u64,
    pub evicted: u64,
    pub pinned: usize,
    pub outstanding: usize,
    pub window: usize,
    pub candidates: usize,
    pub generate_ms: f64,
    pub upload_ms: f64,
    pub wall_ms: f64,
}

/// Resident surfaces, the CPU cache and the window plan that keep the sea
/// drawn. Owns no renderer state: [`sync`](Self::sync) takes one for the
/// frame's uploads.
pub struct WaterStream {
    identity: WaterIdentity,
    cache: MeshCache<WaterMeshKey>,
    residency: WaterResidency,
    work: WaterPlanWork,
    counters: WaterCounters,
    budget: WaterBudget,
    hysteresis: Hysteresis,
}

impl WaterStream {
    /// Build the stream for the landscape generator at `seed`.
    pub fn new(source: TerrainSource, seed: u64) -> Self {
        // Water surfaces are defined only for the landscape generator; a legacy
        // world here would be keyed as landscape and wrong, so refuse the
        // construction instead of deriving the wrong surface.
        assert_eq!(
            source,
            TerrainSource::Landscape,
            "water surfaces require the landscape generator"
        );
        Self {
            identity: WaterIdentity {
                generator_version: LANDSCAPE_GENERATOR_VERSION,
                source,
                seed,
                sea_level: sea_level_chunk_y(),
            },
            cache: MeshCache::new(WATER_CACHE_BYTES),
            residency: WaterResidency::default(),
            work: WaterPlanWork::default(),
            counters: WaterCounters::default(),
            budget: WaterBudget::default(),
            hysteresis: Hysteresis {
                frames: WATER_HYSTERESIS_FRAMES,
                max_held: WATER_HYSTERESIS_HELD,
            },
        }
    }

    pub fn counters(&self) -> WaterCounters {
        self.counters
    }

    pub fn cache_bytes(&self) -> usize {
        self.cache.bytes()
    }

    pub fn cache_entries(&self) -> usize {
        self.cache.entry_count()
    }

    /// Drop the previous renderer's residency. The cache holds no GPU objects
    /// and stays, so a resume re-uploads instead of re-deriving.
    pub fn forgot_renderer(&mut self) {
        self.residency.forgot_renderer();
        self.counters.outstanding = 0;
    }

    fn mesh_key(&self, chunk: [i32; 3]) -> WaterMeshKey {
        self.identity.mesh_key(chunk)
    }

    /// Derive or serve one surface, memoised by identity. Returns the mesh and
    /// whether it was generated (`true`) or served from the cache.
    pub fn mesh_for(&mut self, world: &World, key: [i32; 3]) -> (Arc<Mesh>, bool) {
        let mesh_key = self.mesh_key(key);
        if let Some(mesh) = self.cache.get(&mesh_key) {
            return (mesh, false);
        }
        let mesh = self.cache.insert(mesh_key, world.water_mesh_chunk(key));
        (mesh, true)
    }

    /// Apply one frame's window: declare residency, evict, then derive or serve
    /// within the upload budget and upload. Returns this frame's counters.
    pub fn sync(
        &mut self,
        renderer: &mut Renderer,
        world: &World,
        keys: &[[i32; 3]],
        frame: u64,
    ) -> Result<WaterCounters, String> {
        let begin = Instant::now();
        self.cache.begin_frame(frame);
        self.residency
            .plan_into(keys, frame, self.budget, self.hysteresis, &mut self.work);
        // Declare first: the renderer drops every surface the window no longer
        // wants before this frame adds anything, so GPU residency never exceeds
        // the window plus this frame's budget.
        renderer.retain_water_chunks(&self.work.declared)?;
        for key in &self.work.evict {
            self.residency.remove(*key);
        }
        self.counters.evicted += self.work.evict.len() as u64;
        for key in &self.work.pinned {
            self.cache
                .pin(self.mesh_key(*key), frame + self.hysteresis.frames + 1);
        }
        self.counters.pinned = self.work.pinned.len();

        let mut uploaded = 0u64;
        let mut generated = 0u64;
        let mut served = 0u64;
        let mut generate_ms = 0.0;
        let mut upload_ms = 0.0;
        // Indexing copies the key, so the mutable borrow `mesh_for` needs is
        // not held across the loop by an iterator over the work list.
        for index in 0..self.work.build.len() {
            let key = self.work.build[index];
            if begin.elapsed().as_secs_f64() * 1000. >= MAX_WATER_MS {
                break;
            }
            let derive_begin = Instant::now();
            let (mesh, was_generated) = self.mesh_for(world, key);
            let derive_ms = derive_begin.elapsed().as_secs_f64() * 1000.;
            if was_generated {
                generate_ms += derive_ms;
                generated += 1;
            } else {
                served += 1;
            }
            if mesh.indices.is_empty() {
                // A dry column has no surface to draw, so there is nothing to
                // upload; the empty mesh stays in the CPU cache and the key is
                // settled so the window does not re-offer it every frame. The
                // landscape sample has no edit path, so a settled dry column
                // cannot gain water under a running cache; see `WaterMeshKey`.
                self.residency.mark_uploaded(key);
                continue;
            }
            let upload_begin = Instant::now();
            renderer.upload_water_chunk(key, &mesh)?;
            upload_ms += upload_begin.elapsed().as_secs_f64() * 1000.;
            self.residency.mark_uploaded(key);
            uploaded += 1;
        }

        self.counters.uploaded += uploaded;
        self.counters.generated += generated;
        self.counters.served_from_cache += served;
        self.counters.generate_ms = generate_ms;
        self.counters.upload_ms = upload_ms;
        self.counters.wall_ms = begin.elapsed().as_secs_f64() * 1000.;
        self.counters.window = keys.len();
        self.counters.candidates = self.work.declared.len();
        // Outstanding is what the window still wants after this frame: a
        // surface whose upload did not fit counts, because the next frame must
        // not consider the sea complete.
        self.counters.outstanding = self
            .work
            .declared
            .iter()
            .filter(|key| !self.residency.resident.contains(*key))
            .count();
        Ok(self.counters)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::landscape::SEED;
    use matterweave_core::{water::water_mesh_bytes, Vertex};

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

    fn cache_key(chunk: [i32; 3]) -> WaterMeshKey {
        WaterMeshKey {
            generator_version: LANDSCAPE_GENERATOR_VERSION,
            source: TerrainSource::Landscape,
            seed: SEED,
            sea_level: sea_level_chunk_y(),
            chunk,
        }
    }

    /// A landscape window around the sample spawn plus a sea-level key whose
    /// surface really holds quads, so tests exercise the generator, not an
    /// empty mesh.
    fn streamed_world() -> (World, [i32; 3]) {
        let mut world = World::landscape(SEED);
        world.stream_around([128.0, 40.0, -64.0]);
        let keys = world.stream_resident_chunks().expect("streaming window");
        let key = keys
            .iter()
            .copied()
            .find(|key| {
                key[1] == sea_level_chunk_y() && !world.water_mesh_chunk(*key).indices.is_empty()
            })
            .expect("the window must contain a flooded sea-level chunk");
        (world, key)
    }

    #[test]
    fn a_surface_dropped_and_wanted_again_is_a_cache_hit() {
        let (world, key) = streamed_world();
        let mut stream = WaterStream::new(world.terrain_source(), SEED);
        let mut residency = WaterResidency::default();
        let mut work = WaterPlanWork::default();
        let hysteresis = Hysteresis {
            frames: WATER_HYSTERESIS_FRAMES,
            max_held: WATER_HYSTERESIS_HELD,
        };
        let mut generated = 0u64;
        let mut uploads = 0u64;
        // Forty frames of a camera hovering on the window edge: the surface is
        // wanted, dropped, and wanted again, which is exactly the oscillation
        // the hysteresis exists for.
        for frame in 0..40u64 {
            let wanted: Vec<[i32; 3]> = if frame.is_multiple_of(2) {
                vec![key]
            } else {
                Vec::new()
            };
            residency.plan_into(
                &wanted,
                frame,
                WaterBudget::default(),
                hysteresis,
                &mut work,
            );
            for evicted in &work.evict {
                residency.remove(*evicted);
            }
            for pinned in &work.pinned {
                stream
                    .cache
                    .pin(stream.mesh_key(*pinned), frame + hysteresis.frames + 1);
            }
            for build in &work.build {
                let (_, was_generated) = stream.mesh_for(&world, *build);
                generated += u64::from(was_generated);
                uploads += 1;
                residency.mark_uploaded(*build);
            }
            stream.cache.begin_frame(frame);
        }
        assert_eq!(
            generated, 1,
            "the surface must be derived once across 40 edge crossings"
        );
        assert_eq!(uploads, 20, "every re-entry is still an upload");
    }

    #[test]
    fn a_mesh_is_never_served_for_a_different_generator_identity() {
        let mut cache = MeshCache::new(usize::MAX);
        let base = cache_key([3, 0, -4]);
        cache.insert(base, test_mesh(16));
        assert!(cache.contains(&base));
        for changed in [
            WaterMeshKey {
                generator_version: base.generator_version + 1,
                ..base
            },
            WaterMeshKey {
                source: TerrainSource::LegacyIsland,
                ..base
            },
            WaterMeshKey {
                seed: base.seed + 1,
                ..base
            },
            WaterMeshKey {
                sea_level: base.sea_level + 1,
                ..base
            },
            WaterMeshKey {
                chunk: [3, 0, -3],
                ..base
            },
        ] {
            assert!(
                !cache.contains(&changed),
                "a different identity must not be served: {changed:?}"
            );
        }
    }

    #[test]
    fn the_cache_stays_under_its_byte_budget_and_evicts_least_recently_used() {
        let one = water_mesh_bytes(&test_mesh(100));
        assert!(one > 0);
        let mut cache = MeshCache::new(one * 2);
        let (a, b, c) = (
            cache_key([0, 0, 0]),
            cache_key([1, 0, 0]),
            cache_key([2, 0, 0]),
        );
        cache.insert(a, test_mesh(100));
        cache.insert(b, test_mesh(100));
        cache.get(&a).expect("a is cached");
        cache.insert(c, test_mesh(100));
        assert!(cache.bytes() <= one * 2, "the byte budget bounds the cache");
        assert!(cache.contains(&a) && cache.contains(&c));
        assert!(!cache.contains(&b), "the LRU entry was evicted");
    }

    #[test]
    fn the_declared_set_is_exactly_the_sea_level_window() {
        let (world, _) = streamed_world();
        let keys = world.stream_resident_chunks().unwrap();
        let mut residency = WaterResidency::default();
        let mut work = WaterPlanWork::default();
        residency.plan_into(
            &keys,
            0,
            WaterBudget::default(),
            Hysteresis::NONE,
            &mut work,
        );
        let mut expected: Vec<[i32; 3]> = keys
            .iter()
            .copied()
            .filter(|key| key[1] == sea_level_chunk_y())
            .collect();
        expected.sort_unstable();
        assert_eq!(work.declared, expected);
        // Every other layer is ignored, and the build list is bounded.
        assert!(work.build.iter().all(|key| key[1] == sea_level_chunk_y()));
        assert!(work.build.len() <= MAX_WATER_UPLOADS);
        assert_eq!(
            work.build.len() + work.outstanding,
            expected.len(),
            "every wanted surface is either built now or owed"
        );
    }

    #[test]
    fn moving_evicts_dropped_surfaces_and_keeps_the_rest() {
        let (world, _) = streamed_world();
        let keys = world.stream_resident_chunks().unwrap();
        let sea: Vec<[i32; 3]> = keys
            .iter()
            .copied()
            .filter(|key| key[1] == sea_level_chunk_y())
            .collect();
        assert!(sea.len() > 2);
        let dropped = sea[0];
        let moved: Vec<[i32; 3]> = keys.iter().copied().filter(|key| *key != dropped).collect();

        let mut residency = WaterResidency::default();
        for key in &sea {
            residency.mark_uploaded(*key);
        }
        let mut work = WaterPlanWork::default();
        residency.plan_into(
            &moved,
            1,
            WaterBudget::default(),
            Hysteresis::NONE,
            &mut work,
        );
        assert_eq!(work.evict, vec![dropped]);
        for key in &work.evict {
            residency.remove(*key);
        }
        assert!(
            work.build.is_empty(),
            "everything wanted is already resident"
        );
        assert_eq!(work.outstanding, 0);
        assert_eq!(residency.resident_len(), sea.len() - 1);
    }

    #[test]
    fn the_build_budget_is_a_cap_and_reports_what_is_owed() {
        let (world, _) = streamed_world();
        let keys = world.stream_resident_chunks().unwrap();
        let sea_count = keys
            .iter()
            .filter(|key| key[1] == sea_level_chunk_y())
            .count();
        assert!(sea_count > 4);
        let mut residency = WaterResidency::default();
        let mut work = WaterPlanWork::default();
        residency.plan_into(
            &keys,
            0,
            WaterBudget { uploads: 4 },
            Hysteresis::NONE,
            &mut work,
        );
        assert_eq!(work.build.len(), 4);
        assert_eq!(work.build.len() + work.outstanding, sea_count);
        for key in &work.build {
            residency.mark_uploaded(*key);
        }
        residency.plan_into(
            &keys,
            1,
            WaterBudget { uploads: 4 },
            Hysteresis::NONE,
            &mut work,
        );
        assert!(work.evict.is_empty());
        assert_eq!(work.build.len(), 4);
        assert_eq!(work.outstanding, sea_count - 8);
    }

    #[test]
    fn the_memo_serves_the_real_generator_output_unchanged() {
        let (world, key) = streamed_world();
        let mut stream = WaterStream::new(world.terrain_source(), SEED);
        let direct = world.water_mesh_chunk(key);
        let (mesh, generated) = stream.mesh_for(&world, key);
        assert!(generated);
        assert_eq!(mesh.vertices.len(), direct.vertices.len());
        assert_eq!(mesh.indices, direct.indices);
        assert!(stream.cache_bytes() > 0);
        let (again, generated) = stream.mesh_for(&world, key);
        assert!(!generated, "a repeat must be served from the cache");
        assert!(Arc::ptr_eq(&mesh, &again));
    }
}
