//! Prototype/instance scene with a bounded derived-mesh cache.

use crate::select::{choose_lod, Camera, ErrorMetrics, LodConfig};
use crate::{
    material_policy, Bounds, DetailError, DetailVolume, InstanceLod, Lod, MaterialPolicy,
    MeshBatch, PreparedFrame, Result, Transform,
};
use matterweave_core::{Mesh, Vertex};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

pub const MAX_PROTOTYPES: usize = 4_096;
pub const MAX_INSTANCES: usize = 200_000;
/// Instances must stay inside a plausible world extent, in metres.
pub const MAX_SCENE_TRANSLATION_M: f32 = 65_536.0;
/// Aggregate authoritative source payload across prototypes: 32 MiB.
pub const MAX_SCENE_SOURCE_BYTES: usize = 32 * 1024 * 1024;
/// Aggregate derived mesh cache: 64 MiB. Exceeding it is an error, never a
/// silent eviction or truncation.
pub const MAX_SCENE_CACHE_BYTES: usize = 64 * 1024 * 1024;

/// Opaque identity of an authoritative scene state, suitable for asynchronous
/// publication checks. Unrelated scenes never compare equal, even if their local
/// revision counters match. Cloning a source snapshot retains the identity until
/// either copy changes. Derived cache work does not change this identity.
#[derive(Clone, Debug, Default)]
pub struct SceneVersion(Arc<()>);
impl PartialEq for SceneVersion {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for SceneVersion {}

#[derive(Clone, Debug, PartialEq)]
pub struct InstanceDraw {
    pub instance: String,
    pub prototype: String,
    pub transform: Transform,
    /// Cells of this instance's prototype, counted once per instance.
    pub occupied_cells: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SceneCounts {
    pub prototypes: usize,
    pub instances: usize,
    /// Cells actually stored once, across prototypes.
    pub unique_stored_cells: usize,
    /// Cells represented after expanding every instance. Never conflate with
    /// `unique_stored_cells`; nothing is duplicated in memory.
    pub expanded_occupied_cells: usize,
    /// Collision-policy subset of `expanded_occupied_cells`.
    pub expanded_collision_cells: usize,
    pub expanded_liquid_cells: usize,
    /// Authoritative prototype payload bytes only. Excludes derived meshes,
    /// LOD copies, instance records and allocator overhead.
    pub source_bytes: usize,
    /// Vertex/index allocated capacities held in the cache; excludes allocator metadata.
    pub cached_mesh_bytes: usize,
    /// Derived meshes actually built since construction, for cache-reuse proof.
    pub mesh_builds: u64,
}

#[derive(Clone)]
struct Instance {
    prototype: String,
    transform: Transform,
}

struct CachedMesh {
    revision: u64,
    bytes: usize,
    mesh: Mesh,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct MeshKey(String, Lod);

/// Per-prototype, per-revision derived-loss and bounds summary. Computed once per
/// source revision so selection never re-censuses occupied cells per frame.
struct Digest {
    revision: u64,
    occupied: usize,
    bounds_local: Option<Bounds>,
    metrics: [Option<ErrorMetrics>; 3],
}

fn coarse_metric(
    scale_m: f32,
    factor: i32,
    coarse_cells: usize,
    occupied: usize,
) -> Option<ErrorMetrics> {
    // A derived scale outside the supported range is not a selectable level.
    if crate::Scale::new(scale_m * factor as f32).is_err() {
        return None;
    }
    let error_estimate_m = scale_m * factor as f32;
    if occupied == 0 || coarse_cells == 0 {
        return Some(ErrorMetrics {
            error_estimate_m,
            dilation_fraction: 0.0,
        });
    }
    let filled = coarse_cells as f64 * (factor as f64).powi(3);
    let dilation = (1.0 - occupied as f64 / filled).max(0.0) as f32;
    Some(ErrorMetrics {
        error_estimate_m,
        dilation_fraction: dilation,
    })
}

fn compute_digest(volume: &DetailVolume) -> Digest {
    let scale = volume.scale().metres();
    let mut min = [i32::MAX; 3];
    let mut max = [i32::MIN; 3];
    let mut half: BTreeSet<[i32; 3]> = BTreeSet::new();
    let mut quarter: BTreeSet<[i32; 3]> = BTreeSet::new();
    for (cell, _) in volume.iter_cells() {
        for axis in 0..3 {
            min[axis] = min[axis].min(cell[axis]);
            max[axis] = max[axis].max(cell[axis]);
        }
        half.insert(cell.map(|v| v.div_euclid(2)));
        quarter.insert(cell.map(|v| v.div_euclid(4)));
    }
    let occupied = volume.occupied_cells();
    let bounds_local = (occupied > 0).then(|| Bounds {
        min: min.map(|v| v as f32 * scale),
        max: max.map(|v| (v + 1) as f32 * scale),
    });
    let metrics = [
        Some(ErrorMetrics::SOURCE),
        coarse_metric(scale, 2, half.len(), occupied),
        coarse_metric(scale, 4, quarter.len(), occupied),
    ];
    Digest {
        revision: volume.revision(),
        occupied,
        bounds_local,
        metrics,
    }
}

fn mesh_bytes(mesh: &Mesh) -> usize {
    mesh.vertices.capacity() * std::mem::size_of::<Vertex>() + mesh.indices.capacity() * 4
}

fn validate_scene_point(point: [f32; 3]) -> Result<()> {
    let bound = MAX_SCENE_TRANSLATION_M + (crate::MAX_CELL_COORD + 1) as f32 * crate::MAX_SCALE_M;
    if point.iter().all(|v| v.is_finite() && v.abs() <= bound) {
        Ok(())
    } else {
        Err(DetailError::InvalidPoint)
    }
}

// A valid scene point outside this particular prototype's coordinate range is
// a miss, not a malformed scene query that masks a later, nearby instance.
fn sample_instance(volume: &DetailVolume, transform: &Transform, point: [f32; 3]) -> Result<u8> {
    match volume.sample_world_metres(transform, point) {
        Err(DetailError::InvalidPoint) => Ok(crate::material::AIR),
        result => result,
    }
}

/// Prototypes plus instances. Meshes are derived per prototype and LOD, never
/// per instance, so repeated placements cost a transform, not new geometry.
#[derive(Default)]
pub struct DetailScene {
    source_version: SceneVersion,
    prototypes: BTreeMap<String, DetailVolume>,
    instances: BTreeMap<String, Instance>,
    cache: BTreeMap<MeshKey, CachedMesh>,
    cache_bytes: usize,
    mesh_builds: u64,
    /// Per-prototype-revision digest cache. Pure derived data; never authoritative.
    digests: BTreeMap<String, Digest>,
    /// Last selected level per instance id, for hysteresis across prepare calls.
    selection: BTreeMap<String, Lod>,
}

impl DetailScene {
    pub fn new() -> Self {
        Self::default()
    }

    /// Constant-time token for validating work prepared from a source snapshot.
    pub fn source_version(&self) -> SceneVersion {
        self.source_version.clone()
    }

    pub fn add_prototype(&mut self, volume: DetailVolume) -> Result<()> {
        if self.prototypes.contains_key(volume.id()) {
            return Err(DetailError::DuplicatePrototype(volume.id().to_string()));
        }
        if self.prototypes.len() >= MAX_PROTOTYPES {
            return Err(DetailError::SceneFull);
        }
        if self.source_bytes() + volume.source_bytes() > MAX_SCENE_SOURCE_BYTES {
            return Err(DetailError::BudgetExceeded("scene source payload budget"));
        }
        self.prototypes.insert(volume.id().to_string(), volume);
        self.source_version = SceneVersion::default();
        Ok(())
    }

    pub fn prototype(&self, id: &str) -> Option<&DetailVolume> {
        self.prototypes.get(id)
    }

    /// Authoritative-data-only clone for save-candidate validation: every
    /// prototype payload and instance record is copied, while derived meshes
    /// stay behind (an empty cache rebuilds lazily from source revisions).
    /// A rejected candidate's fork — including private prototypes minted by
    /// [`Self::edit_instance`] — is simply dropped, so cell-by-cell undo of
    /// the live scene is never needed and later candidates see pristine
    /// source accounting. One transient fork is live at a time; it holds at
    /// most the bounded source payload (`MAX_SCENE_SOURCE_BYTES`).
    pub fn fork_source(&self) -> Self {
        Self {
            source_version: self.source_version.clone(),
            prototypes: self.prototypes.clone(),
            instances: self.instances.clone(),
            cache: BTreeMap::new(),
            cache_bytes: 0,
            mesh_builds: self.mesh_builds,
            digests: BTreeMap::new(),
            selection: BTreeMap::new(),
        }
    }

    /// Edit one placed object without changing other instances of a shared
    /// prototype. The first real edit privately copies its bounded source.
    /// Deterministic instance IDs give deterministic private IDs across replay.
    pub fn edit_instance(
        &mut self,
        instance_id: &str,
        cell: [i32; 3],
        material: u8,
    ) -> Result<bool> {
        DetailVolume::check_cell(cell)?;
        let instance = self
            .instances
            .get(instance_id)
            .ok_or_else(|| DetailError::UnknownPrototype(instance_id.to_string()))?;
        let source_id = instance.prototype.clone();
        let source = self
            .prototypes
            .get(&source_id)
            .expect("placed prototype exists");
        if source.get(cell) == material {
            return Ok(false);
        }
        let shared = self
            .instances
            .values()
            .filter(|i| i.prototype == source_id)
            .take(2)
            .count()
            > 1;
        if !shared {
            return self.edit_prototype(&source_id, cell, material);
        }
        if self.prototypes.len() >= MAX_PROTOTYPES {
            return Err(DetailError::SceneFull);
        }
        let private_id = format!("__instance_edit:{instance_id}");
        if self.prototypes.contains_key(&private_id) {
            return Err(DetailError::DuplicatePrototype(private_id));
        }
        // Copy the bounded sparse payload directly, without a temporary snapshot
        // run list. The distinct prototype ID separates cache identities.
        if self.source_bytes() + source.source_bytes() > MAX_SCENE_SOURCE_BYTES {
            return Err(DetailError::BudgetExceeded("instance edit source copy"));
        }
        let mut copy = source.clone();
        copy.id = private_id.clone();
        copy.set(cell, material)?;
        self.add_prototype(copy)?;
        self.instances
            .get_mut(instance_id)
            .expect("validated instance")
            .prototype = private_id;
        Ok(true)
    }

    /// The only way to mutate a prototype through a scene: one cell at a time,
    /// preserving the prototype identity and dropping every
    /// cached derived mesh only when content changes. There is deliberately no `&mut DetailVolume`
    /// accessor, which would allow whole-volume replacement (different content
    /// or identity) while a stale cache entry still matched the old revision.
    pub fn edit_prototype(&mut self, id: &str, cell: [i32; 3], material: u8) -> Result<bool> {
        let volume = self
            .prototypes
            .get(id)
            .ok_or_else(|| DetailError::UnknownPrototype(id.to_string()))?;
        DetailVolume::check_cell(cell)?;
        let key = cell.map(|v| v.div_euclid(matterweave_core::CHUNK_EDGE));
        let adds_chunk =
            material != crate::material::AIR && volume.world.chunk_revision(key).is_none();
        if adds_chunk
            && self.source_bytes() + matterweave_core::CHUNK_VOLUME > MAX_SCENE_SOURCE_BYTES
        {
            return Err(DetailError::BudgetExceeded("scene source payload budget"));
        }
        let changed = self
            .prototypes
            .get_mut(id)
            .expect("validated prototype")
            .set(cell, material)?;
        if changed {
            self.source_version = SceneVersion::default();
            self.invalidate(id);
        }
        Ok(changed)
    }

    /// Drops all cached derived meshes for one prototype.
    pub fn invalidate(&mut self, id: &str) {
        let keys: Vec<MeshKey> = self
            .cache
            .keys()
            .filter(|key| key.0 == id)
            .cloned()
            .collect();
        for key in keys {
            if let Some(entry) = self.cache.remove(&key) {
                self.cache_bytes -= entry.bytes;
            }
        }
    }

    pub fn prototype_ids(&self) -> Vec<String> {
        self.prototypes.keys().cloned().collect()
    }
    pub fn instance_ids(&self) -> Vec<String> {
        self.instances.keys().cloned().collect()
    }

    fn source_bytes(&self) -> usize {
        self.prototypes.values().map(|v| v.source_bytes()).sum()
    }

    pub fn place(
        &mut self,
        instance_id: impl Into<String>,
        prototype_id: &str,
        transform: Transform,
    ) -> Result<()> {
        let instance_id = instance_id.into();
        if !self.prototypes.contains_key(prototype_id) {
            return Err(DetailError::UnknownPrototype(prototype_id.to_string()));
        }
        if self.instances.contains_key(&instance_id) {
            return Err(DetailError::DuplicateInstance(instance_id));
        }
        if self.instances.len() >= MAX_INSTANCES {
            return Err(DetailError::SceneFull);
        }
        // Revalidate: a Transform built from public fields carries no guarantee.
        transform.validate()?;
        self.instances.insert(
            instance_id,
            Instance {
                prototype: prototype_id.to_string(),
                transform,
            },
        );
        self.source_version = SceneVersion::default();
        Ok(())
    }

    /// Derived mesh for a prototype at a LOD, in prototype-local metres.
    /// Cached until the source revision changes.
    ///
    /// `Mesh::revision` is always the *authoritative source* revision of the
    /// prototype, including for coarse levels, so an adapter can validate an
    /// uploaded LOD mesh against the prototype it came from.
    pub fn prototype_mesh(&mut self, prototype_id: &str, lod: Lod) -> Result<&Mesh> {
        let volume = self
            .prototypes
            .get(prototype_id)
            .ok_or_else(|| DetailError::UnknownPrototype(prototype_id.to_string()))?;
        let revision = volume.revision();
        let key = MeshKey(prototype_id.to_string(), lod);
        let stale = self
            .cache
            .get(&key)
            .is_none_or(|cached| cached.revision != revision);
        if stale {
            // Include the still-live cache while reserving space for the new output.
            // Conservative rejection is preferable to allocating a mesh we cannot retain.
            let upper = volume.mesh_upper_bound_bytes()?;
            if self.cache_bytes + upper > MAX_SCENE_CACHE_BYTES {
                return Err(DetailError::BudgetExceeded("scene mesh cache budget"));
            }
            let mut mesh = match lod {
                Lod::Source => volume.mesh_local()?,
                other => volume.coarsen(other)?.mesh_local()?,
            };
            // A coarse copy has its own set-count revision; report the source.
            mesh.revision = revision;
            let bytes = mesh_bytes(&mesh);
            let previous = self.cache.get(&key).map_or(0, |entry| entry.bytes);
            if self.cache_bytes - previous + bytes > MAX_SCENE_CACHE_BYTES {
                return Err(DetailError::BudgetExceeded("scene mesh cache budget"));
            }
            if let Some(entry) = self.cache.remove(&key) {
                self.cache_bytes -= entry.bytes;
            }
            self.mesh_builds += 1;
            self.cache_bytes += bytes;
            self.cache.insert(
                MeshKey(prototype_id.to_string(), lod),
                CachedMesh {
                    revision,
                    bytes,
                    mesh,
                },
            );
        }
        Ok(&self.cache[&key].mesh)
    }

    /// Instances in stable order with their prototype and transform. Adapters
    /// pair this with one cached prototype mesh per prototype/LOD.
    pub fn draws(&self) -> Vec<InstanceDraw> {
        self.instances
            .iter()
            .map(|(id, instance)| InstanceDraw {
                instance: id.clone(),
                prototype: instance.prototype.clone(),
                transform: instance.transform,
                occupied_cells: self.prototypes[&instance.prototype].occupied_cells(),
            })
            .collect()
    }

    /// First instance hit for a world-metre point, in stable instance-id order.
    /// This documented first-hit policy is for generic inspection; collision uses
    /// [`DetailScene::is_collidable_world_metres`], which considers every overlap.
    /// Invalid points return `Err`, never a hit.
    pub fn sample_world_metres(&self, point: [f32; 3]) -> Result<Option<(String, u8)>> {
        validate_scene_point(point)?;
        for (id, instance) in &self.instances {
            let volume = &self.prototypes[&instance.prototype];
            let material = sample_instance(volume, &instance.transform, point)?;
            if material != crate::material::AIR {
                return Ok(Some((id.clone(), material)));
            }
        }
        Ok(None)
    }

    /// Every instance overlapping a world-metre point, in stable instance order.
    pub fn sample_all_world_metres(&self, point: [f32; 3]) -> Result<Vec<(String, u8)>> {
        validate_scene_point(point)?;
        let mut hits = Vec::new();
        for (id, instance) in &self.instances {
            let volume = &self.prototypes[&instance.prototype];
            let material = sample_instance(volume, &instance.transform, point)?;
            if material != crate::material::AIR {
                hits.push((id.clone(), material));
            }
        }
        Ok(hits)
    }

    /// Solid-for-collision query. Considers *all* overlapping instances, so a
    /// liquid or decorative volume in front of a solid one cannot mask it.
    pub fn is_collidable_world_metres(&self, point: [f32; 3]) -> Result<bool> {
        validate_scene_point(point)?;
        for instance in self.instances.values() {
            let volume = &self.prototypes[&instance.prototype];
            let material = sample_instance(volume, &instance.transform, point)?;
            if material_policy(material) == MaterialPolicy::Collision {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Allocated vertex/index capacity currently held by the derived mesh cache.
    /// Unlike `counts`, this does not scan authoritative materials or instances.
    pub fn cached_mesh_bytes(&self) -> usize {
        self.cache_bytes
    }

    /// Immutable access to an already-built derived mesh, for adapters that ran
    /// [`Self::prepare_batches`] and now upload the referenced geometry. Returns
    /// `None` when the mesh is not resident (never triggers a build).
    pub fn cached_prototype_mesh(&self, prototype_id: &str, lod: Lod) -> Option<&Mesh> {
        self.cache
            .get(&MeshKey(prototype_id.to_string(), lod))
            .map(|entry| &entry.mesh)
    }

    /// Refreshes and returns the derived-loss/bounds digest for one prototype,
    /// recomputing only when the source revision changed. Panics if the id is
    /// unknown; callers pass ids read from live instances.
    fn digest(&mut self, prototype_id: &str) -> &Digest {
        let revision = self.prototypes[prototype_id].revision();
        let stale = self
            .digests
            .get(prototype_id)
            .is_none_or(|d| d.revision != revision);
        if stale {
            let digest = compute_digest(&self.prototypes[prototype_id]);
            self.digests.insert(prototype_id.to_string(), digest);
        }
        &self.digests[prototype_id]
    }

    /// Selects a view-dependent LOD per instance and records it for hysteresis.
    ///
    /// Pure selection: no meshes are built and authoritative source data is never
    /// touched. Instances are returned in stable instance-id order. Fails only on
    /// an invalid camera or config; a placed instance always resolves to a level,
    /// with `Source` as the retained fallback.
    pub fn select_lods(&mut self, camera: &Camera, config: &LodConfig) -> Result<Vec<InstanceLod>> {
        camera.validate()?;
        config.validate()?;
        let ids: Vec<String> = self.instances.keys().cloned().collect();
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            let instance = &self.instances[&id];
            let prototype = instance.prototype.clone();
            let transform = instance.transform;
            let previous = self.selection.get(&id).copied().unwrap_or(Lod::Source);
            let (metrics, bounds_local, occupied) = {
                let digest = self.digest(&prototype);
                (digest.metrics, digest.bounds_local, digest.occupied)
            };
            // Nearest AABB depth along the camera forward axis (clamped to near),
            // the meaningful screen-space depth; not Euclidean eye distance, which
            // overstates depth for off-axis instances and coarsens them too soon.
            let depth_m = match bounds_local {
                Some(bounds) => camera.nearest_depth(&bounds.transformed(&transform)),
                None => camera.near_m,
            };
            let (lod, error_estimate_m, projected_error_estimate_px) = if occupied == 0 {
                (Lod::Source, 0.0, 0.0)
            } else {
                choose_lod(previous, camera, config, &metrics, depth_m)
            };
            self.selection.insert(id.clone(), lod);
            out.push(InstanceLod {
                instance: id,
                prototype,
                transform,
                lod,
                error_estimate_m,
                projected_error_estimate_px,
                depth_m,
                occupied_cells: occupied,
                fallback: false,
            });
        }
        Ok(out)
    }

    /// Ensures a derived mesh is resident, retreating to the authoritative source
    /// when the coarse build is capped or the cache budget cannot hold it. Returns
    /// the level actually made resident.
    fn ensure_mesh(
        &mut self,
        prototype_id: &str,
        lod: Lod,
        config: &LodConfig,
        coarse_builds: &mut usize,
    ) -> Result<Lod> {
        if lod == Lod::Source {
            self.prototype_mesh(prototype_id, Lod::Source)?;
            return Ok(Lod::Source);
        }
        let revision = self.prototypes[prototype_id].revision();
        let key = MeshKey(prototype_id.to_string(), lod);
        let fresh = self
            .cache
            .get(&key)
            .is_some_and(|entry| entry.revision == revision);
        if fresh {
            return Ok(lod);
        }
        if let Some(cap) = config.max_coarse_builds {
            if *coarse_builds >= cap {
                self.prototype_mesh(prototype_id, Lod::Source)?;
                return Ok(Lod::Source);
            }
        }
        match self.prototype_mesh(prototype_id, lod) {
            Ok(_) => {
                *coarse_builds += 1;
                Ok(lod)
            }
            Err(DetailError::BudgetExceeded(_)) | Err(DetailError::InvalidScale) => {
                self.prototype_mesh(prototype_id, Lod::Source)?;
                Ok(Lod::Source)
            }
            Err(other) => Err(other),
        }
    }

    /// Selects LODs and synchronously, lazily and within budget realizes the
    /// derived meshes for the selected levels, grouped into per-`(prototype, lod)`
    /// static batches an adapter can upload directly. Coarse builds honor
    /// `config.max_coarse_builds`; any instance whose desired mesh is unavailable
    /// falls back to the authoritative `Source` mesh and is flagged.
    pub fn prepare_batches(
        &mut self,
        camera: &Camera,
        config: &LodConfig,
    ) -> Result<PreparedFrame> {
        let builds_before = self.mesh_builds;
        let desired = self.select_lods(camera, config)?;
        let mut resolved: BTreeMap<(String, Lod), Lod> = BTreeMap::new();
        let mut coarse_builds = 0usize;
        for item in &desired {
            let key = (item.prototype.clone(), item.lod);
            if resolved.contains_key(&key) {
                continue;
            }
            let actual = self.ensure_mesh(&item.prototype, item.lod, config, &mut coarse_builds)?;
            resolved.insert(key, actual);
        }
        let mut selected = Vec::with_capacity(desired.len());
        for mut item in desired {
            let actual = resolved[&(item.prototype.clone(), item.lod)];
            if actual != item.lod {
                let metric = self.digest(&item.prototype).metrics[actual.index()]
                    .unwrap_or(ErrorMetrics::SOURCE);
                item.error_estimate_m = metric.error_estimate_m;
                item.projected_error_estimate_px =
                    camera.projected_error_px(metric.error_estimate_m, item.depth_m);
                item.lod = actual;
                item.fallback = true;
            }
            selected.push(item);
        }
        let mut groups: BTreeMap<(String, Lod), Vec<InstanceLod>> = BTreeMap::new();
        for item in &selected {
            groups
                .entry((item.prototype.clone(), item.lod))
                .or_default()
                .push(item.clone());
        }
        let batches = groups
            .into_iter()
            .map(|((prototype, lod), instances)| {
                let mesh_bytes = self
                    .cache
                    .get(&MeshKey(prototype.clone(), lod))
                    .map_or(0, |entry| entry.bytes);
                MeshBatch {
                    prototype,
                    lod,
                    mesh_bytes,
                    instances,
                }
            })
            .collect();
        Ok(PreparedFrame {
            batches,
            selected,
            source_version: self.source_version.clone(),
            mesh_builds_this_call: self.mesh_builds - builds_before,
            cached_mesh_bytes: self.cache_bytes,
        })
    }

    pub fn counts(&self) -> SceneCounts {
        let mut unique_stored_cells = 0;
        let mut source_bytes = 0;
        let mut per_prototype: BTreeMap<&str, (usize, usize, usize)> = BTreeMap::new();
        for (id, volume) in &self.prototypes {
            unique_stored_cells += volume.occupied_cells();
            source_bytes += volume.source_bytes();
            let mut collision = 0;
            let mut liquid = 0;
            for (_, material) in volume.iter_cells() {
                match material_policy(material) {
                    MaterialPolicy::Collision => collision += 1,
                    MaterialPolicy::Liquid => liquid += 1,
                    MaterialPolicy::Decorative => {}
                }
            }
            per_prototype.insert(id.as_str(), (volume.occupied_cells(), collision, liquid));
        }
        let mut expanded_occupied_cells = 0;
        let mut expanded_collision_cells = 0;
        let mut expanded_liquid_cells = 0;
        for instance in self.instances.values() {
            let (occupied, collision, liquid) = per_prototype[instance.prototype.as_str()];
            expanded_occupied_cells += occupied;
            expanded_collision_cells += collision;
            expanded_liquid_cells += liquid;
        }
        SceneCounts {
            prototypes: self.prototypes.len(),
            instances: self.instances.len(),
            unique_stored_cells,
            expanded_occupied_cells,
            expanded_collision_cells,
            expanded_liquid_cells,
            source_bytes,
            cached_mesh_bytes: self.cache_bytes,
            mesh_builds: self.mesh_builds,
        }
    }
}
