//! Load/edit-time static collision derived from a [`DetailScene`].
//!
//! Policy
//! - The *authoritative finest* source cells of a detail prototype are the only
//!   collision input. Derived meshes and LOD level never participate, so a coarse
//!   visual level can never delete a physical wall.
//! - Only [`MaterialPolicy::Collision`] cells become walls. Liquid and decorative
//!   materials (water, fronds, reeds, rosettes, loose lumen dots) are never walls
//!   at any level of detail, so the 4,000-plant showcase does not attach a
//!   collider to every decorative plant.
//! - Every overlapping instance is considered independently, so a decorative or
//!   liquid instance in front of a solid one cannot mask the solid one.
//! - One [`SharedShape`] is built per *prototype* and shared by every static
//!   instance of that prototype; instances cost a transform and a collider handle,
//!   not new geometry. Shapes are rebuilt for each call, so a scene restored to an
//!   earlier revision can never reuse a stale shape from a previous revision.
//! - The whole update is transactional: any rejection returns `Err` before a
//!   single existing collider is touched, and never silently skips walls.
//! - Nothing here runs per frame: [`Physics::replace_detail_scene`] is a load-time
//!   or edit-time call.
//!
//! Backend choice, measured against the pinned rapier3d 0.32.0 / parry3d 0.26.1
//! (see docs/performance/p03/collision/opus-execution.md):
//! - `SharedShape::voxels` (parry `Voxels`) was implemented and rejected. Its
//!   storage is attractively sparse (8^3 chunks of one state byte), but
//!   `contact_manifolds_voxels_shape` intersects the *unloosened* voxel AABB
//!   with the other shape's AABB, so it emits no manifold across a small
//!   prediction gap. The kinematic character then rested on the surface with
//!   `grounded == false` and could never jump. Measured with this pin: capsule
//!   vs voxels at a 5 mm gap produced 0 manifolds; capsule vs a compound cuboid
//!   in the same pose produced 1 manifold with `local_n1 = -Y`.
//! - Greedy merged cuboids in a `SharedShape::compound` are therefore used. That
//!   is the same merging idea as the existing `solid_boxes` terrain path, but
//!   that function is not reusable as written: it is hard-wired to a dense 16^3
//!   `matterweave_core::World` chunk, 1 m cells and "material != 0 is solid",
//!   while detail prototypes are sparse, carry their own cell scale
//!   (6.25 cm / 25 cm) and must filter by material policy. The merge here runs
//!   over an ordered *set* of collision cells, so a sparse prototype whose
//!   occupied cells are far apart never allocates the empty space between them.
//!
//! Cell geometry matches the detail convention exactly: cell `c` spans
//! `[c * scale, (c + 1) * scale]` in prototype-local metres.

use crate::Physics;
use matterweave_detail::{
    material_policy, DetailScene, DetailVolume, MaterialPolicy, SceneVersion, Yaw,
};
use rapier3d::prelude::*;
use std::collections::BTreeMap;

/// Largest number of static detail colliders accepted. This counts *collidable*
/// instances only: instances of liquid/decorative prototypes produce no collider
/// and are not charged against it, so an arbitrarily lush decorative scene (up to
/// the detail crate's own 200,000-instance budget) is admissible.
pub const MAX_DETAIL_COLLIDERS: usize = 16_384;
/// Largest number of *filtered collision* cells admitted across all prototypes,
/// counted once per prototype (shared shapes are never counted per instance).
///
/// 16 Mi cells is an admission cap chosen so the ~8.8 M-cell full-map prototype
/// source is representable with headroom, rather than the earlier 4 Mi cap which
/// could not represent it at all. Cells are not stored: they are counted, then
/// merged one prototype at a time, so the resident cost is the merged boxes
/// below, not this number. The transient scratch cost is bounded separately by
/// [`MAX_DETAIL_SCRATCH_CHUNKS`].
pub const MAX_DETAIL_SOURCE_COLLISION_CELLS: usize = 16 << 20;
/// Largest number of merged cuboids accepted across all built shapes, counted
/// once per prototype. Merged boxes, not raw cells, are the resident collider
/// cost: each entry is a pose (32 B) plus a shared cuboid allocation (~32 B)
/// plus its compound BVH leaf (~32-64 B), i.e. roughly 100-130 B per box, so
/// this cap is an estimated 26-34 MiB of resident shape memory. It is not raised
/// to fit a whole map in one shape; see the aggregate note in
/// docs/performance/p03/collision/opus-execution.md.
pub const MAX_DETAIL_BOXES: usize = 262_144;
/// Largest number of 16^3 scratch chunks held while merging *one* prototype.
/// Each chunk is a 4,096-bit occupancy mask (512 B), freed as soon as that
/// prototype's shape is built, so the scratch bit payload is bounded at
/// 32 MiB (tree and allocator metadata are additional) even for a fully sparse prototype with one cell per chunk.
pub const MAX_DETAIL_SCRATCH_CHUNKS: usize = 65_536;
/// Edge of a scratch occupancy chunk, in cells.
const SCRATCH_EDGE: i32 = 16;
/// `u64` words per scratch chunk mask (16^3 bits).
const SCRATCH_WORDS: usize = 64;
/// Extra metres added to the changed region when waking bodies.
const WAKE_MARGIN_M: f32 = 0.5;
const DETAIL_FRICTION: f32 = 0.8;

/// Result of one accepted [`Physics::replace_detail_scene`] call.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DetailCollisionStats {
    /// Prototypes present in the scene.
    pub prototypes: usize,
    /// Prototypes with at least one collision cell; one shared shape each.
    pub collidable_prototypes: usize,
    /// Instances present in the scene.
    pub instances: usize,
    /// Static colliders now live: one per instance of a collidable prototype.
    pub static_colliders: usize,
    /// Distinct `SharedShape`s built and shared across the colliders above.
    pub shared_shapes: usize,
    /// Collision cells counted once per prototype (unique stored, not expanded).
    pub source_collision_cells: usize,
    /// Collision cells after expanding every instance. Never conflate with
    /// `source_collision_cells`; nothing is duplicated in memory.
    pub expanded_collision_cells: usize,
    /// Greedy merged cuboids across the built shapes, counted once per
    /// prototype. This is the shape cost that instances share.
    pub merged_boxes: usize,
    /// Dynamic voxel bodies woken because they overlap the changed region.
    pub woken_bodies: usize,
}

/// A prototype's shared collision shape plus the counters proving its cost.
struct PreparedShape {
    shape: SharedShape,
    cells: usize,
    boxes: usize,
}

/// Exact quarter-turn rotation. Built from exact quaternion components rather
/// than `sin`/`cos` of an angle so a quarter turn stays exact.
fn yaw_rotation(yaw: Yaw) -> Rotation {
    const H: f32 = std::f32::consts::FRAC_1_SQRT_2;
    match yaw {
        Yaw::Deg0 => Rotation::from_xyzw(0.0, 0.0, 0.0, 1.0),
        Yaw::Deg90 => Rotation::from_xyzw(0.0, H, 0.0, H),
        Yaw::Deg180 => Rotation::from_xyzw(0.0, 1.0, 0.0, 0.0),
        Yaw::Deg270 => Rotation::from_xyzw(0.0, -H, 0.0, H),
    }
}

/// Occupancy mask of one 16^3 scratch chunk: 4,096 bits in 64 words.
struct ScratchChunk([u64; SCRATCH_WORDS]);

impl ScratchChunk {
    fn index(local: [i32; 3]) -> usize {
        local[0] as usize + 16 * local[1] as usize + 256 * local[2] as usize
    }
    fn set(&mut self, local: [i32; 3]) {
        let index = Self::index(local);
        self.0[index / 64] |= 1 << (index % 64);
    }
    fn get(&self, local: [i32; 3]) -> bool {
        let index = Self::index(local);
        self.0[index / 64] >> (index % 64) & 1 == 1
    }
    fn clear(&mut self, local: [i32; 3]) {
        let index = Self::index(local);
        self.0[index / 64] &= !(1 << (index % 64));
    }
}

fn split_cell(cell: [i32; 3]) -> ([i32; 3], [i32; 3]) {
    (
        cell.map(|v| v.div_euclid(SCRATCH_EDGE)),
        cell.map(|v| v.rem_euclid(SCRATCH_EDGE)),
    )
}

/// Builds the shared collision shape of one prototype, or `Ok(None)` when the
/// prototype has no collision cells at all.
///
/// Every conservative limit is checked before the allocation it protects:
/// 1. collision cells are counted by streaming `iter_cells` and filtering by
///    material policy first, allocating nothing, and are charged against the
///    aggregate budget still remaining for this call, so a huge liquid or
///    decorative prototype costs nothing and a solid one cannot overrun the
///    aggregate cap before its scratch memory is taken;
/// 2. the scratch occupancy masks are bounded by [`MAX_DETAIL_SCRATCH_CHUNKS`]
///    while they are being allocated;
/// 3. merged boxes are charged against the boxes still remaining for this call
///    as they are produced, so an over-budget prototype stops merging instead of
///    building an oversized shape and rejecting it afterwards.
fn prepare_prototype(
    volume: &DetailVolume,
    remaining_cells: usize,
    remaining_boxes: usize,
) -> Result<Option<PreparedShape>, String> {
    let id = volume.id();
    let scale = volume.scale().metres();
    if !scale.is_finite() || scale <= 0.0 {
        return Err(format!("detail prototype {id} has an invalid cell scale"));
    }
    // 1. Count the filtered collision cells. No allocation happens here.
    let cells = volume
        .iter_cells()
        .filter(|(_, material)| material_policy(*material) == MaterialPolicy::Collision)
        .count();
    if cells == 0 {
        return Ok(None);
    }
    if cells > remaining_cells {
        return Err(format!(
            "detail prototype {id} needs {cells} collision cells but only {remaining_cells} of the \
             {MAX_DETAIL_SOURCE_COLLISION_CELLS} aggregate collision-cell budget remain"
        ));
    }
    // 2. Sparse scratch occupancy, bounded while it is allocated.
    let mut chunks: BTreeMap<[i32; 3], ScratchChunk> = BTreeMap::new();
    for (cell, material) in volume.iter_cells() {
        if material_policy(material) != MaterialPolicy::Collision {
            continue;
        }
        let (key, local) = split_cell(cell);
        if !chunks.contains_key(&key) && chunks.len() >= MAX_DETAIL_SCRATCH_CHUNKS {
            return Err(format!(
                "detail prototype {id} needs more than {MAX_DETAIL_SCRATCH_CHUNKS} scratch chunks"
            ));
        }
        chunks
            .entry(key)
            .or_insert_with(|| ScratchChunk([0; SCRATCH_WORDS]))
            .set(local);
    }
    // 3. Merge, charging boxes against the remaining aggregate budget.
    let mut boxes = Vec::new();
    for (key, mut chunk) in chunks {
        greedy_boxes(key, &mut chunk, scale, remaining_boxes, &mut boxes).map_err(|needed| {
            format!(
                "detail prototype {id} needs more than {needed} merged boxes but only \
                 {remaining_boxes} of the {MAX_DETAIL_BOXES} aggregate box budget remain"
            )
        })?;
    }
    Ok(Some(PreparedShape {
        cells,
        boxes: boxes.len(),
        shape: SharedShape::compound(boxes),
    }))
}

/// Merges one scratch chunk's occupied cells into axis-aligned cuboids,
/// extending along x, then z, then y, exactly like the terrain `solid_boxes`
/// merge. Boxes never cross a chunk boundary, which keeps the working set at one
/// 512-byte mask instead of the prototype's whole bounding box; the resulting
/// geometry is identical, only the box split differs.
///
/// Appends to `boxes` and fails with the count reached as soon as `budget` boxes
/// would be exceeded, so an over-budget prototype stops early.
fn greedy_boxes(
    key: [i32; 3],
    chunk: &mut ScratchChunk,
    scale: f32,
    budget: usize,
    boxes: &mut Vec<(Pose, SharedShape)>,
) -> Result<(), usize> {
    let base = key.map(|v| v * SCRATCH_EDGE);
    for z in 0..SCRATCH_EDGE {
        for y in 0..SCRATCH_EDGE {
            for x in 0..SCRATCH_EDGE {
                if !chunk.get([x, y, z]) {
                    continue;
                }
                let mut end_x = x + 1;
                while end_x < SCRATCH_EDGE && chunk.get([end_x, y, z]) {
                    end_x += 1;
                }
                let mut end_z = z + 1;
                while end_z < SCRATCH_EDGE && (x..end_x).all(|xx| chunk.get([xx, y, end_z])) {
                    end_z += 1;
                }
                let mut end_y = y + 1;
                while end_y < SCRATCH_EDGE
                    && (z..end_z).all(|zz| (x..end_x).all(|xx| chunk.get([xx, end_y, zz])))
                {
                    end_y += 1;
                }
                for zz in z..end_z {
                    for yy in y..end_y {
                        for xx in x..end_x {
                            chunk.clear([xx, yy, zz]);
                        }
                    }
                }
                if boxes.len() >= budget {
                    return Err(budget);
                }
                let half = Vector::new((end_x - x) as f32, (end_y - y) as f32, (end_z - z) as f32)
                    * 0.5
                    * scale;
                let centre = Vector::new(
                    (base[0] + x) as f32,
                    (base[1] + y) as f32,
                    (base[2] + z) as f32,
                ) * scale
                    + half;
                boxes.push((
                    Pose::from_translation(centre),
                    SharedShape::cuboid(half.x, half.y, half.z),
                ));
            }
        }
    }
    Ok(())
}

/// Fully prepared collision derived from one immutable authoritative scene state.
/// Backend shapes stay private. This value can be built on a worker thread and
/// moved to the simulation owner; preparation never mutates a live Physics world.
/// Existing collider/cell/box/scratch limits apply unchanged.
pub struct PreparedDetailCollision {
    version: SceneVersion,
    pending: Vec<(Pose, SharedShape)>,
    stats: DetailCollisionStats,
}
impl PreparedDetailCollision {
    pub fn stats(&self) -> DetailCollisionStats {
        self.stats
    }

    /// Whether the authoritative state is still the one used for preparation.
    /// Publication checks again, so edits between polling and committing are safe.
    pub fn is_current(&self, scene: &DetailScene) -> bool {
        self.version == scene.source_version()
    }

    /// World-space AABB of every collider this preparation would install.
    /// Used only for the conservative structural gate (unknown scene change):
    /// publication defers while any dynamic body overlaps any of these, which
    /// is over-conservative but can never miss new material the way a
    /// whole-collider "unchanged" comparison would.
    pub fn collider_aabbs(&self) -> Vec<Aabb> {
        self.pending
            .iter()
            .map(|(pose, shape)| shape.compute_local_aabb().transform_by(pose))
            .collect()
    }

    pub fn build(scene: &DetailScene) -> Result<Self, String> {
        let counts = scene.counts();
        // Instance count alone is never a rejection: liquid and decorative
        // instances produce no collider, so only collidable instances are
        // charged against MAX_DETAIL_COLLIDERS below.
        // 1. Build one shared shape per collidable prototype. Nothing is mutated yet.
        let mut prepared: BTreeMap<String, PreparedShape> = BTreeMap::new();
        let mut source_collision_cells = 0usize;
        let mut merged_boxes = 0usize;
        for id in scene.prototype_ids() {
            let volume = scene
                .prototype(&id)
                .ok_or_else(|| format!("detail prototype {id} disappeared during preparation"))?;
            // Budgets are consumed one prototype at a time, so the remaining
            // aggregate allowance bounds each prototype before it allocates.
            let Some(shape) = prepare_prototype(
                volume,
                MAX_DETAIL_SOURCE_COLLISION_CELLS - source_collision_cells,
                MAX_DETAIL_BOXES - merged_boxes,
            )?
            else {
                continue;
            };
            source_collision_cells += shape.cells;
            merged_boxes += shape.boxes;
            prepared.insert(id, shape);
        }
        // 2. Pose one static collider per collidable instance, reusing the shape.
        let mut pending: Vec<(Pose, SharedShape)> = Vec::new();
        let mut expanded_collision_cells = 0usize;
        for draw in scene.draws() {
            let Some(shape) = prepared.get(&draw.prototype) else {
                continue;
            };
            if !draw.transform.is_valid() {
                return Err(format!(
                    "detail instance {} has an invalid transform",
                    draw.instance
                ));
            }
            if pending.len() >= MAX_DETAIL_COLLIDERS {
                return Err(format!(
                    "detail scene needs more than {MAX_DETAIL_COLLIDERS} static colliders"
                ));
            }
            expanded_collision_cells += shape.cells;
            pending.push((
                Pose::from_parts(
                    Vector::from_array(draw.transform.translation_m),
                    yaw_rotation(draw.transform.yaw),
                ),
                shape.shape.clone(),
            ));
        }
        let stats = DetailCollisionStats {
            prototypes: counts.prototypes,
            collidable_prototypes: prepared.len(),
            instances: counts.instances,
            static_colliders: pending.len(),
            shared_shapes: prepared.len(),
            source_collision_cells,
            expanded_collision_cells,
            merged_boxes,
            woken_bodies: 0,
        };
        Ok(Self {
            version: scene.source_version(),
            pending,
            stats,
        })
    }
}

impl Physics {
    /// Synchronous convenience path. Preparation is isolated and publication is
    /// version checked, exactly as for a worker-built PreparedDetailCollision.
    pub fn replace_detail_scene(
        &mut self,
        scene: &DetailScene,
    ) -> Result<DetailCollisionStats, String> {
        let prepared = PreparedDetailCollision::build(scene)?;
        self.publish_detail_scene(scene, prepared)
    }

    /// Publish prepared shapes only if their source is still current. Rejection
    /// preserves live colliders and bodies. The simulation owner must serialize
    /// this call with physics stepping. Shape construction is already complete;
    /// insertion/removal and waking overlapping bodies still run on this thread.
    pub fn publish_detail_scene(
        &mut self,
        scene: &DetailScene,
        prepared: PreparedDetailCollision,
    ) -> Result<DetailCollisionStats, String> {
        if !prepared.is_current(scene) {
            return Err("prepared detail collision source changed before publication".into());
        }
        let PreparedDetailCollision {
            pending, mut stats, ..
        } = prepared;
        // 3. Commit. From here nothing can fail.
        let mut changed: Option<Aabb> = None;
        let merge = |aabb: Aabb, changed: &mut Option<Aabb>| {
            *changed = Some(match changed.take() {
                Some(previous) => previous.merged(&aabb),
                None => aabb,
            });
        };
        for handle in std::mem::take(&mut self.detail) {
            if let Some(collider) = self.colliders.get(handle) {
                merge(collider.compute_aabb(), &mut changed);
            }
            self.colliders
                .remove(handle, &mut self.islands, &mut self.bodies, true);
        }
        for (pose, shape) in pending {
            let handle = self.colliders.insert(
                ColliderBuilder::new(shape)
                    .position(pose)
                    .friction(DETAIL_FRICTION),
            );
            merge(self.colliders[handle].compute_aabb(), &mut changed);
            self.detail.push(handle);
        }
        let woken_bodies = self.wake_bodies_in(changed);
        stats.woken_bodies = woken_bodies;
        self.detail_stats = stats;
        Ok(stats)
    }

    /// World-space AABBs of every dynamic body: the character capsule plus one
    /// entry per collider of every dynamic voxel object.
    pub fn dynamic_body_aabbs(&self) -> Vec<Aabb> {
        let mut aabbs = Vec::with_capacity(1 + self.objects.len() * 2);
        aabbs.push(self.colliders[self.character_collider].compute_aabb());
        for object in &self.objects {
            for collider in self.bodies[object.handle].colliders() {
                if let Some(collider) = self.colliders.get(*collider) {
                    aabbs.push(collider.compute_aabb());
                }
            }
        }
        aabbs
    }

    /// Whether any dynamic body AABB intersects any region in `added`.
    ///
    /// `added` must cover every world-space region where the pending source
    /// added solid collision material relative to the last accepted
    /// publication; removals are never included because removing collision
    /// cannot trap a body. The [`DetailCollisionCadence`] accumulates this set
    /// from the owner's edit journal, so no collider-geometry comparison is
    /// needed here: coarse whole-collider AABBs can never establish unchanged
    /// shape (filling an interior hole leaves the outer AABB identical), and
    /// are therefore used only for the conservative structural fallback below.
    pub fn detail_added_blocked(&self, added: &[Aabb]) -> bool {
        if added.is_empty() {
            return false;
        }
        self.dynamic_body_aabbs()
            .iter()
            .any(|body| added.iter().any(|region| region.intersects(body)))
    }

    /// Counters of the last accepted detail replacement. A rejected replacement
    /// leaves these unchanged, matching the preserved colliders.
    pub fn detail_collision_stats(&self) -> DetailCollisionStats {
        self.detail_stats
    }

    /// Number of live static detail colliders.
    pub fn detail_collider_count(&self) -> usize {
        self.detail.len()
    }

    /// Wakes every enabled voxel body overlapping the changed region.
    fn wake_bodies_in(&mut self, changed: Option<Aabb>) -> usize {
        let Some(region) = changed.map(|aabb| aabb.loosened(WAKE_MARGIN_M)) else {
            return 0;
        };
        let handles: Vec<RigidBodyHandle> = self
            .objects
            .iter()
            .map(|object| object.handle)
            .filter(|handle| {
                self.bodies[*handle].colliders().iter().any(|collider| {
                    self.colliders
                        .get(*collider)
                        .is_some_and(|collider| collider.compute_aabb().intersects(&region))
                })
            })
            .collect();
        let mut woken = 0;
        for handle in handles {
            let body = &mut self.bodies[handle];
            if body.is_enabled() {
                body.wake_up(true);
                woken += 1;
            }
        }
        woken
    }
}
