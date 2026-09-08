//! Prototype/instance scene with a bounded derived-mesh cache.

use crate::{material_policy, DetailError, DetailVolume, Lod, MaterialPolicy, Result, Transform};
use matterweave_core::{Mesh, Vertex};
use std::collections::BTreeMap;

pub const MAX_PROTOTYPES: usize = 4_096;
pub const MAX_INSTANCES: usize = 200_000;
/// Instances must stay inside a plausible world extent, in metres.
pub const MAX_SCENE_TRANSLATION_M: f32 = 65_536.0;
/// Aggregate authoritative source payload across prototypes: 32 MiB.
pub const MAX_SCENE_SOURCE_BYTES: usize = 32 * 1024 * 1024;
/// Aggregate derived mesh cache: 64 MiB. Exceeding it is an error, never a
/// silent eviction or truncation.
pub const MAX_SCENE_CACHE_BYTES: usize = 64 * 1024 * 1024;

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
    prototypes: BTreeMap<String, DetailVolume>,
    instances: BTreeMap<String, Instance>,
    cache: BTreeMap<MeshKey, CachedMesh>,
    cache_bytes: usize,
    mesh_builds: u64,
}

impl DetailScene {
    pub fn new() -> Self {
        Self::default()
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
        Ok(())
    }

    pub fn prototype(&self, id: &str) -> Option<&DetailVolume> {
        self.prototypes.get(id)
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
