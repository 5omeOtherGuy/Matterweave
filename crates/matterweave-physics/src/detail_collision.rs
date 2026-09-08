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
use matterweave_detail::{material_policy, DetailScene, DetailVolume, MaterialPolicy, Yaw};
use rapier3d::prelude::*;
use std::collections::{BTreeMap, BTreeSet};

/// Largest number of static detail colliders (collidable instances) accepted.
pub const MAX_DETAIL_COLLIDERS: usize = 16_384;
/// Largest number of collision cells accepted in one prototype.
pub const MAX_DETAIL_PROTOTYPE_COLLISION_CELLS: usize = 1 << 20;
/// Largest number of collision cells accepted across all prototypes, counted
/// once per prototype (shared shapes are never counted per instance).
pub const MAX_DETAIL_SOURCE_COLLISION_CELLS: usize = 4 << 20;
/// Largest number of merged cuboids accepted across all built shapes, counted
/// once per prototype. Merged boxes, not raw cells, are the collider cost.
pub const MAX_DETAIL_BOXES: usize = 262_144;
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

/// Builds the shared collision shape of one prototype, or `Ok(None)` when the
/// prototype has no collision cells at all.
///
/// Conservative limits are checked *before* any allocation: the O(1) occupied
/// cell count is an upper bound on collision cells and is rejected first, and
/// the sparse chunk count is enforced while streaming cells, before the shape is
/// constructed.
fn prepare_prototype(volume: &DetailVolume) -> Result<Option<PreparedShape>, String> {
    let id = volume.id();
    let scale = volume.scale().metres();
    if !scale.is_finite() || scale <= 0.0 {
        return Err(format!("detail prototype {id} has an invalid cell scale"));
    }
    if volume.occupied_cells() > MAX_DETAIL_PROTOTYPE_COLLISION_CELLS {
        return Err(format!(
            "detail prototype {id} has {} occupied cells, above the {} collision-cell limit",
            volume.occupied_cells(),
            MAX_DETAIL_PROTOTYPE_COLLISION_CELLS
        ));
    }
    let mut remaining: BTreeSet<[i32; 3]> = volume
        .iter_cells()
        .filter(|(_, material)| material_policy(*material) == MaterialPolicy::Collision)
        .map(|(cell, _)| cell)
        .collect();
    let cells = remaining.len();
    if cells == 0 {
        return Ok(None);
    }
    let boxes = greedy_boxes(&mut remaining, scale);
    if boxes.len() > MAX_DETAIL_BOXES {
        return Err(format!(
            "detail prototype {id} needs {} merged boxes, above the {MAX_DETAIL_BOXES} limit",
            boxes.len()
        ));
    }
    Ok(Some(PreparedShape {
        cells,
        boxes: boxes.len(),
        shape: SharedShape::compound(boxes),
    }))
}

/// Merges an ordered set of occupied cells into axis-aligned cuboids, extending
/// along x, then z, then y, exactly like the terrain `solid_boxes` merge. The
/// set is consumed. Cost follows the number of occupied cells, never the extent
/// of the bounding box, so far apart clusters stay cheap.
fn greedy_boxes(remaining: &mut BTreeSet<[i32; 3]>, scale: f32) -> Vec<(Pose, SharedShape)> {
    let mut boxes = Vec::new();
    while let Some(&[x, y, z]) = remaining.iter().next() {
        let mut end_x = x + 1;
        while remaining.contains(&[end_x, y, z]) {
            end_x += 1;
        }
        let mut end_z = z + 1;
        while (x..end_x).all(|xx| remaining.contains(&[xx, y, end_z])) {
            end_z += 1;
        }
        let mut end_y = y + 1;
        while (z..end_z).all(|zz| (x..end_x).all(|xx| remaining.contains(&[xx, end_y, zz]))) {
            end_y += 1;
        }
        for zz in z..end_z {
            for yy in y..end_y {
                for xx in x..end_x {
                    remaining.remove(&[xx, yy, zz]);
                }
            }
        }
        let half =
            Vector::new((end_x - x) as f32, (end_y - y) as f32, (end_z - z) as f32) * 0.5 * scale;
        let centre = Vector::new(x as f32, y as f32, z as f32) * scale + half;
        boxes.push((
            Pose::from_translation(centre),
            SharedShape::cuboid(half.x, half.y, half.z),
        ));
    }
    boxes
}

impl Physics {
    /// Replaces *all* static detail collision with the colliders derived from
    /// `scene`. Call at load time or after an authoring edit; never per frame.
    ///
    /// An empty scene clears detail collision. On `Err` nothing changed: the
    /// previously published colliders and every live body are preserved, so an
    /// invalid or over-budget update fails loudly instead of quietly dropping
    /// walls. Bodies overlapping the changed region are woken.
    pub fn replace_detail_scene(
        &mut self,
        scene: &DetailScene,
    ) -> Result<DetailCollisionStats, String> {
        let counts = scene.counts();
        if counts.instances > MAX_DETAIL_COLLIDERS {
            return Err(format!(
                "detail scene has {} instances, above the {MAX_DETAIL_COLLIDERS} collider limit",
                counts.instances
            ));
        }
        // 1. Build one shared shape per collidable prototype. Nothing is mutated yet.
        let mut prepared: BTreeMap<String, PreparedShape> = BTreeMap::new();
        let mut source_collision_cells = 0usize;
        let mut merged_boxes = 0usize;
        for id in scene.prototype_ids() {
            let volume = scene
                .prototype(&id)
                .ok_or_else(|| format!("detail prototype {id} disappeared during preparation"))?;
            let Some(shape) = prepare_prototype(volume)? else {
                continue;
            };
            source_collision_cells += shape.cells;
            merged_boxes += shape.boxes;
            if source_collision_cells > MAX_DETAIL_SOURCE_COLLISION_CELLS {
                return Err(format!(
                    "detail scene needs more than {MAX_DETAIL_SOURCE_COLLISION_CELLS} source collision cells"
                ));
            }
            if merged_boxes > MAX_DETAIL_BOXES {
                return Err(format!(
                    "detail scene needs more than {MAX_DETAIL_BOXES} merged boxes"
                ));
            }
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
        let stats = DetailCollisionStats {
            prototypes: counts.prototypes,
            collidable_prototypes: prepared.len(),
            instances: counts.instances,
            static_colliders: self.detail.len(),
            shared_shapes: prepared.len(),
            source_collision_cells,
            expanded_collision_cells,
            merged_boxes,
            woken_bodies,
        };
        self.detail_stats = stats;
        Ok(stats)
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
