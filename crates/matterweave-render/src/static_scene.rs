//! Bounded GPU prototype instancing for load/edit-time static scenery.
//!
//! Unique prototype geometry is pooled into shared vertex/index buffers and a
//! single packed per-scene instance buffer, avoiding one allocation pair per
//! plant. Replacement is a transaction: validation and planning happen on the
//! host, buffer construction completes before the live scene is swapped, and
//! any failure retains the previous scene. This module owns no backend types in
//! its public interface.

use super::{validate_mesh, Buffer, Frustum, Result};
use crate::lighting::{PlayerPush, Wind};
use ash::vk;
use bytemuck::{Pod, Zeroable};
use matterweave_core::{Mesh, Vertex};
use std::{collections::BTreeMap, sync::Arc};

/// Host-side allocation ceiling for pooled static geometry (vertices+indices).
pub const STATIC_MESH_BUDGET_BYTES: usize = 128 * 1024 * 1024;
/// Host-side allocation ceiling for the packed per-scene instance buffer.
pub const STATIC_INSTANCE_BUDGET_BYTES: usize = 16 * 1024 * 1024;

/// Hard ceiling on one wind-capable flora scene's instances. A dense field is
/// meant to be bounded: a caller over this budget is told so and keeps the
/// scene it already has, rather than having its far tier silently truncated.
pub const MAX_FLORA_INSTANCES: usize = 16_384;
/// Per-buffer byte ceiling implied by [`MAX_FLORA_INSTANCES`]. A flora scene
/// carries two per-instance buffers of the same length - the packed transform
/// record and the wind record - so this bounds each of them.
pub const MAX_FLORA_BYTES: usize = MAX_FLORA_INSTANCES * INSTANCE_RECORD_SIZE;
/// Tallest flora prototype the wind path accepts, in metres. Displacement
/// scales with the height a vertex sits at, so an absurd height would turn a
/// gentle breeze into a catapult.
pub const MAX_FLORA_HEIGHT_M: f32 = 64.0;
/// Largest per-instance uniform scale the flora path accepts. The scale travels
/// in the wind record's fourth slot, where `0` marks a record that is not flora
/// (the zero record every non-flora draw binds), and it multiplies both the
/// prototype's local vertices and the wind displacement.
pub const MAX_FLORA_SCALE: f32 = 4.0;

/// One instanced placement of a prototype mesh. `yaw_quarters` rotates the
/// prototype about the Y axis in quarter turns (0..=3); translation is world
/// space. This is a CPU description only; the packed GPU record is private.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StaticInstance {
    pub prototype: usize,
    pub translation: [f32; 3],
    pub yaw_quarters: u8,
}

/// One instanced placement of a wind-capable flora prototype.
///
/// Everything [`StaticInstance`] carries, plus the numbers `world.wgsl` needs to
/// size and displace it: `phase` decorrelates neighbouring plants, `bend` is how
/// far this plant gives in a unit wind, `height_m` is the prototype's own height
/// in metres so the shader can weight displacement by height above the ground
/// contact, and `scale` is how large this one instance is drawn. [`StaticInstance`]
/// deliberately keeps its shape: existing scenes pack and draw exactly as before.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FloraInstance {
    pub prototype: usize,
    pub translation: [f32; 3],
    pub yaw_quarters: u8,
    /// Sway phase in `0..=1`, one turn of the primary sine.
    pub phase: f32,
    /// Compliance in `0..=1`: 0 is rigid, 1 bends the full wind amount.
    pub bend: f32,
    /// Prototype height in metres, in `(0, MAX_FLORA_HEIGHT_M]` before `scale`.
    pub height_m: f32,
    /// Uniform scale in `(0, MAX_FLORA_SCALE]`. Every instance of a prototype
    /// can then be its own size without a prototype per size, which is what a
    /// dense field needs: a mat of one silhouette repeated is still a mat of one
    /// silhouette.
    pub scale: f32,
}

/// Packed GPU instance record: vertex input location 3 as vec4<f32> holds
/// translation xyz plus quarter-turn yaw in w. Must match world.wgsl/shadow.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct InstanceRecord {
    data: [f32; 4],
}

/// Packed GPU wind record: vertex input location 4 as vec4<f32> holds
/// `(phase, bend, height_m, scale)`. `scale == 0` is the early-out every
/// non-flora draw binds, which is why the zero record is the identity: it
/// disables both the displacement and the scale.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub(crate) struct WindRecord {
    pub data: [f32; 4],
}

const INSTANCE_RECORD_SIZE: usize = std::mem::size_of::<InstanceRecord>();
pub(crate) const WIND_RECORD_SIZE: usize = std::mem::size_of::<WindRecord>();
/// The record a non-flora draw reads: a zero scale disables both the wind
/// displacement and the per-instance scale.
pub(crate) const ZERO_WIND: WindRecord = WindRecord { data: [0.0; 4] };
// Both per-instance bindings are declared with the same stride in the pipeline
// vertex input; keeping the records the same size is what makes that true.
const _: () = assert!(WIND_RECORD_SIZE == INSTANCE_RECORD_SIZE);

/// A placement the packer can group, validate and pack. Implemented by
/// [`StaticInstance`] (no wind) and [`FloraInstance`] (wind), so prototype
/// pooling, per-prototype batching and bounds culling are written once.
pub(crate) trait Placement: Copy {
    /// Whether this placement kind can be displaced by the shader. Every scene
    /// carries a wind buffer parallel to its transform buffer - a batch draw
    /// selects its slice with `firstInstance`, so a shorter buffer would be
    /// read out of bounds - but a scene of plain [`StaticInstance`]s fills it
    /// with [`ZERO_WIND`], which the shader early-outs on.
    const WIND: bool;
    fn base(&self) -> StaticInstance;
    fn wind(&self) -> WindRecord;
    /// Uniform scale this placement draws its prototype at. One for anything
    /// that is not scaled, so the bounds maths below can be written once.
    fn scale(&self) -> f32;
    /// Host validation of the fields this kind adds beyond the base instance.
    fn validate_wind(&self) -> Result<()>;
}

impl Placement for StaticInstance {
    const WIND: bool = false;
    fn base(&self) -> StaticInstance {
        *self
    }
    fn wind(&self) -> WindRecord {
        ZERO_WIND
    }
    fn scale(&self) -> f32 {
        1.0
    }
    fn validate_wind(&self) -> Result<()> {
        Ok(())
    }
}

impl Placement for FloraInstance {
    const WIND: bool = true;
    fn base(&self) -> StaticInstance {
        StaticInstance {
            prototype: self.prototype,
            translation: self.translation,
            yaw_quarters: self.yaw_quarters,
        }
    }
    fn wind(&self) -> WindRecord {
        WindRecord {
            data: [self.phase, self.bend, self.height_m, self.scale],
        }
    }
    fn scale(&self) -> f32 {
        self.scale
    }
    fn validate_wind(&self) -> Result<()> {
        if !self.phase.is_finite() || !(0.0..=1.0).contains(&self.phase) {
            return Err(format!(
                "Flora instance phase {} is not in 0..=1",
                self.phase
            ));
        }
        if !self.bend.is_finite() || !(0.0..=1.0).contains(&self.bend) {
            return Err(format!("Flora instance bend {} is not in 0..=1", self.bend));
        }
        if !self.scale.is_finite() || self.scale <= 0.0 || self.scale > MAX_FLORA_SCALE {
            return Err(format!(
                "Flora instance scale {} is not in (0, {MAX_FLORA_SCALE}]",
                self.scale
            ));
        }
        // The scaled height is what the shader sees, so that is what the cap
        // applies to: an instance cannot escape the height bound by scaling up.
        let height_m = self.height_m * self.scale;
        if !self.height_m.is_finite() || height_m <= 0.0 || height_m > MAX_FLORA_HEIGHT_M {
            return Err(format!(
                "Flora instance height {} m at scale {} is {} m, not in (0, {MAX_FLORA_HEIGHT_M}]",
                self.height_m, self.scale, height_m
            ));
        }
        Ok(())
    }
}

/// Honest accounting of the last accepted static scene. Source bytes are the
/// input mesh bytes; allocated bytes are the real GPU buffer capacities.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StaticSceneStats {
    pub prototypes: usize,
    pub instances: usize,
    pub vertices: usize,
    pub indices: usize,
    pub source_bytes: usize,
    pub allocated_bytes: usize,
    /// Per-prototype draw batches carrying all of that prototype's instances.
    pub batches: usize,
    /// Allocated bytes of the two per-instance buffers: the packed transform
    /// records plus the wind records when the scene carries them.
    pub instance_bytes: usize,
    /// Whether this scene supplies per-instance wind records.
    pub wind: bool,
}

#[derive(Debug)]
pub(crate) struct PrototypeRange {
    pub first_index: u32,
    pub index_count: u32,
    /// Vulkan vertexOffset is i32; the 128 MiB geometry budget keeps merged
    /// offsets far inside that range, but the conversion stays checked.
    pub vertex_offset: i32,
    pub instance_offset: u32,
    pub instance_count: u32,
    /// Union of this prototype's instance bounds; used for conservative
    /// main-pass batch culling. Shadow passes never cull with it.
    pub bounds: [[f32; 3]; 2],
}

#[derive(Debug)]
pub(crate) struct PrototypeGeometry {
    first_index: u32,
    index_count: u32,
    vertex_offset: i32,
    bounds: [[f32; 3]; 2],
    has_vertices: bool,
}

pub(crate) struct InstanceUpdate {
    instance_bytes: Vec<u8>,
    /// Parallel to `instance_bytes`, record for record; `firstInstance`
    /// indexes both buffers identically.
    wind_bytes: Vec<u8>,
    prototypes: Vec<PrototypeRange>,
    bounds: [[f32; 3]; 2],
    instance_count: u32,
}

#[derive(Debug)]
pub(crate) struct StaticScenePlan {
    geometry: Vec<PrototypeGeometry>,
    pub vertex_bytes: Vec<u8>,
    pub index_bytes: Vec<u8>,
    pub instance_bytes: Vec<u8>,
    /// Parallel to `instance_bytes`, record for record.
    pub wind_bytes: Vec<u8>,
    pub prototypes: Vec<PrototypeRange>,
    pub bounds: [[f32; 3]; 2],
    pub instance_count: u32,
    pub has_geometry: bool,
    pub source_bytes: usize,
    /// Whether this plan's placements can carry nonzero wind records.
    pub wind_capable: bool,
}

impl StaticScenePlan {
    fn empty() -> Self {
        Self {
            geometry: Vec::new(),
            vertex_bytes: Vec::new(),
            index_bytes: Vec::new(),
            instance_bytes: Vec::new(),
            wind_bytes: Vec::new(),
            prototypes: Vec::new(),
            bounds: [[0.; 3], [0.; 3]],
            instance_count: 0,
            has_geometry: false,
            source_bytes: 0,
            wind_capable: false,
        }
    }
}

fn rotate_xz(p: [f32; 3], yaw_quarters: u8) -> [f32; 3] {
    // Must match the quarter LUT in world.wgsl/shadow.wgsl:
    // x' = c*x + s*z, z' = -s*x + c*z with (c,s) from the quarter table.
    match yaw_quarters {
        0 => p,
        1 => [p[2], p[1], -p[0]],
        2 => [-p[0], p[1], -p[2]],
        3 => [-p[2], p[1], p[0]],
        _ => unreachable!("yaw_quarters validated to 0..=3 before rotation"),
    }
}

fn rotate_bounds(bounds: [[f32; 3]; 2], yaw_quarters: u8) -> [[f32; 3]; 2] {
    let a = rotate_xz(bounds[0], yaw_quarters);
    let b = rotate_xz(bounds[1], yaw_quarters);
    [
        [a[0].min(b[0]), a[1].min(b[1]), a[2].min(b[2])],
        [a[0].max(b[0]), a[1].max(b[1]), a[2].max(b[2])],
    ]
}

fn translate_bounds(bounds: [[f32; 3]; 2], translation: [f32; 3]) -> [[f32; 3]; 2] {
    [
        [
            bounds[0][0] + translation[0],
            bounds[0][1] + translation[1],
            bounds[0][2] + translation[2],
        ],
        [
            bounds[1][0] + translation[0],
            bounds[1][1] + translation[1],
            bounds[1][2] + translation[2],
        ],
    ]
}

/// Largest horizontal sway `world.wgsl` applies to a flora vertex at scale 1.
///
/// The shader's two decorrelated sines span ±1.5, the per-instance `bend` is in
/// `0..=1` and the wind strength is capped by [`Wind::MAX_STRENGTH_M`]:
/// `1.5 * 1 * 1.5 = 2.25` m. A per-instance scale multiplies this term.
pub const MAX_WIND_SWAY_M: f32 = 1.5 * Wind::MAX_STRENGTH_M;

/// Largest walk-through push `world.wgsl` applies to a flora vertex.
///
/// The shader's `(1 - d/r)^2 * r * 0.5 * h` peaks at `r / 2` with `h = 1`, and
/// the radius is capped by [`PlayerPush::MAX_RADIUS_M`]: `8 * 0.5 = 4` m. The
/// push is *not* multiplied by the instance scale - it parts the field around
/// the player, not around the plant.
pub const MAX_PLAYER_PUSH_M: f32 = 0.5 * PlayerPush::MAX_RADIUS_M;

/// Largest horizontal displacement `world.wgsl` can apply to a flora vertex at
/// scale 1 in total.
///
/// Sway and push are independent terms and can point the same way, so the bound
/// is their sum, `6.25` m - not the wind the landscape sample happens to blow.
/// The terms are derived from the two *validated* maxima above, so raising
/// [`Wind::MAX_STRENGTH_M`] or [`PlayerPush::MAX_RADIUS_M`] moves this bound with
/// them instead of silently invalidating it: flora batch bounds grown by this
/// margin are what keeps whole-batch frustum culling from clipping a plant that
/// swayed into view.
pub const MAX_WIND_DISPLACEMENT_M: f32 = MAX_WIND_SWAY_M + MAX_PLAYER_PUSH_M;

/// Grow bounds horizontally by the wind margin, for wind-capable placements only:
/// the sway term scales with the instance and the push does not, and both are
/// bounded by the validated settings maxima rather than by the sample's weather.
fn wind_margin<P: Placement>(bounds: [[f32; 3]; 2], scale: f32) -> [[f32; 3]; 2] {
    if !P::WIND {
        return bounds;
    }
    let m = MAX_WIND_SWAY_M * scale + MAX_PLAYER_PUSH_M;
    [
        [bounds[0][0] - m, bounds[0][1], bounds[0][2] - m],
        [bounds[1][0] + m, bounds[1][1], bounds[1][2] + m],
    ]
}

/// Scale bounds about the prototype's own origin, which is where the ground
/// contact is, so a scaled instance stands on the same cell it always did.
fn scale_bounds(bounds: [[f32; 3]; 2], scale: f32) -> [[f32; 3]; 2] {
    if scale == 1.0 {
        return bounds;
    }
    [
        [
            bounds[0][0] * scale,
            bounds[0][1] * scale,
            bounds[0][2] * scale,
        ],
        [
            bounds[1][0] * scale,
            bounds[1][1] * scale,
            bounds[1][2] * scale,
        ],
    ]
}

fn union_bounds(target: &mut Option<[[f32; 3]; 2]>, bounds: [[f32; 3]; 2]) {
    match target {
        Some(existing) => {
            for axis in 0..3 {
                existing[0][axis] = existing[0][axis].min(bounds[0][axis]);
                existing[1][axis] = existing[1][axis].max(bounds[1][axis]);
            }
        }
        None => *target = Some(bounds),
    }
}

fn instance_record(instance: &StaticInstance) -> InstanceRecord {
    InstanceRecord {
        data: [
            instance.translation[0],
            instance.translation[1],
            instance.translation[2],
            instance.yaw_quarters as f32,
        ],
    }
}

fn instance_bytes(records: &[u8]) -> u32 {
    // Caller sized this from validated counts; division documents the invariant.
    (records.len() / INSTANCE_RECORD_SIZE) as u32
}

/// Validates and packs a whole static scene on the host. Any error means the
/// caller keeps its previous scene; nothing was constructed or written.
pub(crate) fn plan_static_scene(
    meshes: &[Mesh],
    instances: &[StaticInstance],
) -> Result<StaticScenePlan> {
    plan_static_scene_with_budgets(
        meshes,
        instances,
        STATIC_MESH_BUDGET_BYTES,
        STATIC_INSTANCE_BUDGET_BYTES,
    )
}

/// [`plan_static_scene`] for wind-capable flora, under the tighter
/// [`MAX_FLORA_BYTES`] instance budget.
pub(crate) fn plan_flora_scene(
    meshes: &[Mesh],
    instances: &[FloraInstance],
) -> Result<StaticScenePlan> {
    plan_static_scene_with_budgets(meshes, instances, STATIC_MESH_BUDGET_BYTES, MAX_FLORA_BYTES)
}

pub(crate) fn plan_static_scene_with_budgets<P: Placement>(
    meshes: &[Mesh],
    instances: &[P],
    mesh_budget: usize,
    instance_budget: usize,
) -> Result<StaticScenePlan> {
    // An empty instance list is an explicit clear of the previous scene.
    if instances.is_empty() {
        return Ok(StaticScenePlan::empty());
    }
    let instance_capacity = instances
        .len()
        .checked_mul(INSTANCE_RECORD_SIZE)
        .ok_or("Static instance count overflows host arithmetic")?;
    if instance_capacity > instance_budget {
        return Err(format!(
            "Static scene instances need {instance_capacity} bytes, over the {instance_budget}-byte instance budget"
        ));
    }
    let mut grouped: BTreeMap<usize, Vec<P>> = BTreeMap::new();
    for placement in instances {
        let instance = placement.base();
        if instance.prototype >= meshes.len() {
            return Err(format!(
                "Static instance references prototype {} but only {} prototypes were supplied",
                instance.prototype,
                meshes.len()
            ));
        }
        if instance.yaw_quarters > 3 {
            return Err(format!(
                "Static instance yaw_quarters {} exceeds quarter turns 0..=3",
                instance.yaw_quarters
            ));
        }
        if !instance.translation.iter().all(|v| v.is_finite()) {
            return Err("Static instance translation contains non-finite values".into());
        }
        placement.validate_wind()?;
        grouped
            .entry(instance.prototype)
            .or_default()
            .push(*placement);
    }
    let mut plan = StaticScenePlan::empty();
    plan.wind_capable = P::WIND;
    let mut vertex_offset: u64 = 0;
    let mut first_index: u64 = 0;
    let mut union: Option<[[f32; 3]; 2]> = None;
    for (prototype, mesh) in meshes.iter().enumerate() {
        let (index_count, bounds) = validate_mesh(mesh)?;
        let vertex_bytes_len = mesh
            .vertices
            .len()
            .checked_mul(std::mem::size_of::<Vertex>())
            .ok_or("Static prototype vertex bytes overflow host arithmetic")?;
        let index_bytes_len = mesh
            .indices
            .len()
            .checked_mul(std::mem::size_of::<u32>())
            .ok_or("Static prototype index bytes overflow host arithmetic")?;
        plan.source_bytes = plan
            .source_bytes
            .checked_add(vertex_bytes_len)
            .and_then(|s| s.checked_add(index_bytes_len))
            .ok_or("Static scene source bytes overflow host arithmetic")?;
        let vertex_count = u32::try_from(mesh.vertices.len())
            .map_err(|_| "Static prototype exceeds u32 vertex count")?;
        let next_vertex_offset = vertex_offset + u64::from(vertex_count);
        let next_first_index = first_index + u64::from(index_count);
        if next_vertex_offset > u32::MAX as u64 {
            return Err("Merged static vertices exceed the u32 vertex offset range".into());
        }
        let vertex_offset_i32 = i32::try_from(vertex_offset)
            .map_err(|_| "Merged static vertices exceed the i32 vertex offset range".to_string())?;
        if next_first_index > u32::MAX as u64 {
            return Err("Merged static indices exceed the u32 first-index range".into());
        }
        let geometry = vertex_bytes_len
            .checked_add(index_bytes_len)
            .and_then(|mesh_bytes| mesh_bytes.checked_add(plan.vertex_bytes.len()))
            .and_then(|mesh_bytes| mesh_bytes.checked_add(plan.index_bytes.len()))
            .ok_or("Static scene geometry bytes overflow host arithmetic")?;
        if geometry > mesh_budget {
            return Err(format!(
                "Static scene geometry needs {geometry} bytes, over the {mesh_budget}-byte static mesh budget"
            ));
        }
        plan.vertex_bytes
            .extend_from_slice(bytemuck::cast_slice(&mesh.vertices));
        plan.index_bytes
            .extend_from_slice(bytemuck::cast_slice(&mesh.indices));
        plan.geometry.push(PrototypeGeometry {
            first_index: first_index as u32,
            index_count,
            vertex_offset: vertex_offset_i32,
            bounds,
            has_vertices: !mesh.vertices.is_empty(),
        });
        let list = grouped.get(&prototype);
        let (instance_offset, instance_count, batch_bounds) = match list {
            Some(list) => {
                if mesh.vertices.is_empty() {
                    return Err(format!(
                        "Instances reference prototype {prototype} which has no vertices"
                    ));
                }
                let offset = instance_bytes(&plan.instance_bytes);
                let mut batch: Option<[[f32; 3]; 2]> = None;
                for placement in list {
                    let instance = &placement.base();
                    plan.instance_bytes
                        .extend_from_slice(bytemuck::bytes_of(&instance_record(instance)));
                    plan.wind_bytes
                        .extend_from_slice(bytemuck::bytes_of(&placement.wind()));
                    let placed = wind_margin::<P>(
                        translate_bounds(
                            rotate_bounds(
                                scale_bounds(bounds, placement.scale()),
                                instance.yaw_quarters,
                            ),
                            instance.translation,
                        ),
                        placement.scale(),
                    );
                    if !placed.iter().flatten().all(|v| v.is_finite()) {
                        return Err(
                            "Static instance translation produces non-finite scene bounds".into(),
                        );
                    }
                    union_bounds(&mut batch, placed);
                    union_bounds(&mut union, placed);
                }
                (offset, list.len() as u32, batch)
            }
            None => (0, 0, None),
        };
        if index_count > 0 && instance_count > 0 {
            let bounds = batch_bounds.expect("instances produce bounds");
            plan.prototypes.push(PrototypeRange {
                first_index: first_index as u32,
                index_count,
                vertex_offset: vertex_offset_i32,
                instance_offset,
                instance_count,
                bounds,
            });
        }
        vertex_offset = next_vertex_offset;
        first_index = next_first_index;
    }
    plan.instance_count = instance_bytes(&plan.instance_bytes);
    plan.has_geometry = !plan.prototypes.is_empty();
    if let Some(bounds) = union {
        plan.bounds = bounds;
    }
    Ok(plan)
}

/// Instance-only planning never reads or copies vertex/index payloads.
fn plan_instance_update<P: Placement>(
    geometry: &[PrototypeGeometry],
    instances: &[P],
    budget: usize,
) -> Result<InstanceUpdate> {
    let size = instances
        .len()
        .checked_mul(INSTANCE_RECORD_SIZE)
        .filter(|&n| n <= budget)
        .ok_or("Static instance update exceeds budget")?;
    let mut grouped: BTreeMap<usize, Vec<P>> = BTreeMap::new();
    for placement in instances {
        let instance = placement.base();
        let mesh = geometry
            .get(instance.prototype)
            .ok_or("Unknown static prototype")?;
        if !mesh.has_vertices
            || instance.yaw_quarters > 3
            || !instance.translation.iter().all(|v| v.is_finite())
        {
            return Err("Invalid static instance or empty prototype".into());
        }
        placement.validate_wind()?;
        grouped
            .entry(instance.prototype)
            .or_default()
            .push(*placement);
    }
    let mut output = InstanceUpdate {
        instance_bytes: Vec::with_capacity(size),
        wind_bytes: Vec::with_capacity(size),
        prototypes: Vec::new(),
        bounds: [[0.; 3]; 2],
        instance_count: instances.len() as u32,
    };
    let mut scene_bounds = None;
    for (index, placements) in grouped {
        let mesh = &geometry[index];
        let offset = instance_bytes(&output.instance_bytes);
        let mut bounds = None;
        for placement in &placements {
            let instance = &placement.base();
            let placed = wind_margin::<P>(
                translate_bounds(
                    rotate_bounds(
                        scale_bounds(mesh.bounds, placement.scale()),
                        instance.yaw_quarters,
                    ),
                    instance.translation,
                ),
                placement.scale(),
            );
            if !placed.iter().flatten().all(|v| v.is_finite()) {
                return Err("Static instance produces nonfinite bounds".into());
            }
            union_bounds(&mut bounds, placed);
            union_bounds(&mut scene_bounds, placed);
            output
                .instance_bytes
                .extend_from_slice(bytemuck::bytes_of(&instance_record(instance)));
            output
                .wind_bytes
                .extend_from_slice(bytemuck::bytes_of(&placement.wind()));
        }
        if mesh.index_count > 0 {
            output.prototypes.push(PrototypeRange {
                first_index: mesh.first_index,
                index_count: mesh.index_count,
                vertex_offset: mesh.vertex_offset,
                instance_offset: offset,
                instance_count: placements.len() as u32,
                bounds: bounds.unwrap(),
            });
        }
    }
    output.bounds = scene_bounds.unwrap_or([[0.; 3]; 2]);
    Ok(output)
}

/// GPU-side static scene. All buffers are created before construction returns,
/// so a failed replacement never leaves a partially built scene behind.
pub(crate) struct StaticScene {
    geometry: Vec<PrototypeGeometry>,
    pub(crate) vertices: Option<Buffer>,
    pub(crate) indices: Option<Buffer>,
    pub(crate) instances: Buffer,
    /// Per-instance wind records, parallel to `instances`. Always present, and
    /// zero-filled for a scene of plain static instances.
    pub(crate) wind: Buffer,
    /// Whether this scene's wind records can be nonzero.
    wind_capable: bool,
    pub(crate) prototypes: Vec<PrototypeRange>,
    pub(crate) instance_count: u32,
    pub(crate) has_geometry: bool,
    pub(crate) bounds: [[f32; 3]; 2],
    pub(crate) allocated_bytes: usize,
    source_bytes: usize,
    prototype_count: usize,
}

impl StaticScene {
    pub(crate) fn new(device: Arc<super::Device>, plan: StaticScenePlan) -> Result<Self> {
        let usage = vk::BufferUsageFlags::VERTEX_BUFFER;
        let vertices = (!plan.vertex_bytes.is_empty())
            .then(|| Buffer::new(device.clone(), &plan.vertex_bytes, usage))
            .transpose()?;
        let indices = (!plan.index_bytes.is_empty())
            .then(|| {
                Buffer::new(
                    device.clone(),
                    &plan.index_bytes,
                    vk::BufferUsageFlags::INDEX_BUFFER,
                )
            })
            .transpose()?;
        // An accepted nonempty instance list always produces instance records.
        let instances = Buffer::new(device.clone(), &plan.instance_bytes, usage)?;
        let wind = Buffer::new(device.clone(), &plan.wind_bytes, usage)?;
        let allocated_bytes = vertices.as_ref().map_or(0, |b| b.size)
            + indices.as_ref().map_or(0, |b| b.size)
            + instances.size
            + wind.size;
        let prototype_count = plan.prototypes.len();
        Ok(Self {
            geometry: plan.geometry,
            allocated_bytes,
            prototype_count,
            source_bytes: plan.source_bytes,
            vertices,
            indices,
            instances,
            wind,
            wind_capable: plan.wind_capable,
            prototypes: plan.prototypes,
            instance_count: plan.instance_count,
            has_geometry: plan.has_geometry,
            bounds: plan.bounds,
        })
    }

    pub(crate) fn plan_instances(&self, instances: &[StaticInstance]) -> Result<InstanceUpdate> {
        plan_instance_update(&self.geometry, instances, STATIC_INSTANCE_BUDGET_BYTES)
    }

    pub(crate) fn plan_flora_instances(
        &self,
        instances: &[FloraInstance],
    ) -> Result<InstanceUpdate> {
        plan_instance_update(&self.geometry, instances, MAX_FLORA_BYTES)
    }

    /// Caller has waited the sole frame fence. Geometry allocations stay untouched.
    pub(crate) fn update_instances(&mut self, plan: InstanceUpdate) -> Result<()> {
        // The wind buffer is grown or written first; it and the transform
        // buffer always hold the same record count, so a failure here leaves
        // the previous, still-consistent pair published.
        let device = self.instances.device.clone();
        if plan.wind_bytes.len() > self.wind.size {
            let buffer = Buffer::new(
                device.clone(),
                &plan.wind_bytes,
                vk::BufferUsageFlags::VERTEX_BUFFER,
            )?;
            self.allocated_bytes = self.allocated_bytes - self.wind.size + buffer.size;
            self.wind = buffer;
        } else {
            self.wind.write(&plan.wind_bytes)?;
        }
        if plan.instance_bytes.len() > self.instances.size {
            let buffer = Buffer::new(
                device.clone(),
                &plan.instance_bytes,
                vk::BufferUsageFlags::VERTEX_BUFFER,
            )?;
            self.allocated_bytes = self.allocated_bytes - self.instances.size + buffer.size;
            self.instances = buffer;
        } else {
            // Mapping can fail before writes; after the copy there are no fallible steps.
            self.instances.write(&plan.instance_bytes)?;
        }
        self.prototypes = plan.prototypes;
        self.prototype_count = self.prototypes.len();
        self.has_geometry = !self.prototypes.is_empty();
        self.bounds = plan.bounds;
        self.instance_count = plan.instance_count;
        Ok(())
    }

    pub(crate) fn stats(&self) -> StaticSceneStats {
        StaticSceneStats {
            prototypes: self.prototype_count,
            instances: self.instance_count as usize,
            vertices: self
                .vertices
                .as_ref()
                .map_or(0, |b| b.size / std::mem::size_of::<Vertex>()),
            indices: self
                .indices
                .as_ref()
                .map_or(0, |b| b.size / std::mem::size_of::<u32>()),
            source_bytes: self.source_bytes,
            allocated_bytes: self.allocated_bytes,
            batches: self
                .prototypes
                .iter()
                .filter(|p| p.index_count > 0 && p.instance_count > 0)
                .count(),
            instance_bytes: self.instances.size + self.wind.size,
            wind: self.wind_capable,
        }
    }

    /// Binds the pooled static buffers once and records one draw per prototype
    /// batch. `frustum` culls whole batches only, which can never hide
    /// geometry inside the frustum; shadow passes pass `None` so offscreen
    /// casters are retained.
    pub(crate) fn record_batches(
        &self,
        device: &ash::Device,
        cmd: vk::CommandBuffer,
        frustum: Option<&Frustum>,
    ) -> usize {
        if !self.has_geometry {
            return 0;
        }
        let vertices = self.vertices.as_ref().expect("geometry implies vertices");
        let indices = self.indices.as_ref().expect("geometry implies indices");
        // SAFETY: caller records into an idle command buffer; these buffers stay
        // alive until the submit fence, and the ranges are validated at replace time.
        unsafe {
            device.cmd_bind_vertex_buffers(cmd, 0, &[vertices.raw], &[0]);
            device.cmd_bind_vertex_buffers(cmd, 1, &[self.instances.raw], &[0]);
            // Binding 2 is the wind attribute, parallel to binding 1 so a
            // batch's `firstInstance` selects the same slice of both. A scene
            // of plain static instances holds zero records here, whose `w = 0`
            // early-outs the shader, so its geometry is unchanged.
            device.cmd_bind_vertex_buffers(cmd, 2, &[self.wind.raw], &[0]);
            device.cmd_bind_index_buffer(cmd, indices.raw, 0, vk::IndexType::UINT32);
        }
        let mut batches = 0;
        for prototype in &self.prototypes {
            if prototype.index_count == 0 || prototype.instance_count == 0 {
                continue;
            }
            if frustum.is_some_and(|f| !f.intersects(prototype.bounds)) {
                continue;
            }
            // SAFETY: same idle command buffer; firstInstance selects this
            // batch's contiguous slice of the packed instance buffer.
            unsafe {
                device.cmd_draw_indexed(
                    cmd,
                    prototype.index_count,
                    prototype.instance_count,
                    prototype.first_index,
                    prototype.vertex_offset,
                    prototype.instance_offset,
                );
            }
            batches += 1;
        }
        batches
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mesh(vertices: &[[f32; 3]], indices: &[u32], revision: u64) -> Mesh {
        Mesh {
            vertices: vertices
                .iter()
                .map(|&position| Vertex {
                    position,
                    normal: [0., 1., 0.],
                    color: [0.5; 3],
                })
                .collect(),
            indices: indices.to_vec(),
            revision,
        }
    }

    fn triangle(revision: u64) -> Mesh {
        mesh(
            &[[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]],
            &[0, 1, 2],
            revision,
        )
    }

    fn instance(prototype: usize, translation: [f32; 3], yaw_quarters: u8) -> StaticInstance {
        StaticInstance {
            prototype,
            translation,
            yaw_quarters,
        }
    }

    #[test]
    fn instance_record_matches_shader_location3_vec4() {
        // One packed vec4<f32>: translation xyz then quarter yaw in w.
        assert_eq!(INSTANCE_RECORD_SIZE, 16);
        let record = instance_record(&instance(0, [1.5, -2.0, 3.25], 2));
        assert_eq!(
            record.data,
            [1.5, -2.0, 3.25, 2.0],
            "translation xyz and yaw quarters must land in declared order"
        );
        // The shaders read exactly this attribute and apply the same rotation.
        for source in [include_str!("world.wgsl"), include_str!("shadow.wgsl")] {
            assert!(
                source.contains("@location(3) instance: vec4<f32>"),
                "shader must declare the packed instance attribute at location 3"
            );
            assert!(
                source.contains("quarter_cos") && source.contains("quarter_sin"),
                "shader must rotate through the exact quarter LUT"
            );
        }
        assert!(
            !include_str!("hud.wgsl").contains("@location(3)"),
            "HUD pipeline has no instance binding and must not declare one"
        );
    }

    #[test]
    fn empty_instances_clear_the_scene() {
        let plan = plan_static_scene(&[triangle(1)], &[]).unwrap();
        assert_eq!(plan.instance_count, 0);
        assert!(plan.vertex_bytes.is_empty());
        assert!(plan.index_bytes.is_empty());
        assert!(plan.prototypes.is_empty());
        assert!(!plan.has_geometry);
    }

    #[test]
    fn instance_updates_can_select_previously_unused_geometry() {
        let plan =
            plan_static_scene(&[triangle(1), triangle(2)], &[instance(0, [0.; 3], 0)]).unwrap();
        let update =
            plan_instance_update(&plan.geometry, &[instance(1, [-3., 2., 4.], 1)], 16).unwrap();
        assert_eq!(update.prototypes.len(), 1);
        let batch = &update.prototypes[0];
        assert_eq!(
            (batch.first_index, batch.vertex_offset, batch.index_count),
            (3, 3, 3)
        );
        assert_eq!(batch.bounds, [[-3., 2., 3.], [-3., 3., 4.]]);
        assert_eq!(update.instance_count, 1);
        assert_eq!(update.instance_bytes.len(), 16);
    }

    #[test]
    fn instance_update_rejects_invalid_inputs_before_publication() {
        let plan =
            plan_static_scene(&[triangle(1), Mesh::default()], &[instance(0, [0.; 3], 0)]).unwrap();
        for bad in [
            instance(2, [0.; 3], 0),
            instance(1, [0.; 3], 0),
            instance(0, [0.; 3], 4),
            instance(0, [f32::NAN, 0., 0.], 0),
        ] {
            assert!(plan_instance_update(&plan.geometry, &[bad], 16).is_err());
        }
        assert!(plan_instance_update(&plan.geometry, &[instance(0, [0.; 3], 0)], 15).is_err());
        assert_eq!(plan.prototypes[0].instance_count, 1);
    }

    #[test]
    fn clearing_instances_retains_reusable_geometry_metadata() {
        let plan = plan_static_scene(&[triangle(1)], &[instance(0, [0.; 3], 0)]).unwrap();
        let cleared = plan_instance_update::<StaticInstance>(&plan.geometry, &[], 0).unwrap();
        assert_eq!(cleared.instance_count, 0);
        assert!(cleared.prototypes.is_empty());
        let again = plan_instance_update(&plan.geometry, &[instance(0, [1.; 3], 0)], 16).unwrap();
        assert_eq!(again.prototypes[0].bounds, [[1.; 3], [2., 2., 1.]]);
    }

    #[test]
    fn prototypes_share_packed_buffers_with_per_prototype_ranges() {
        let quad = mesh(
            &[[0., 0., 0.], [1., 0., 0.], [1., 1., 0.], [0., 1., 0.]],
            &[0, 1, 2, 0, 2, 3],
            3,
        );
        let plan = plan_static_scene(
            &[triangle(1), quad],
            &[
                instance(0, [0., 0., 0.], 0),
                instance(1, [5., 0., 0.], 1),
                instance(1, [-3., 2., 1.], 3),
                instance(0, [0., -4., 2.], 2),
            ],
        )
        .unwrap();
        assert_eq!(plan.instance_count, 4);
        assert_eq!(plan.vertex_bytes.len() / std::mem::size_of::<Vertex>(), 7);
        assert_eq!(plan.index_bytes.len() / std::mem::size_of::<u32>(), 9);
        assert_eq!(plan.source_bytes, 7 * 36 + 9 * 4);
        assert_eq!(plan.prototypes.len(), 2);
        let tri = &plan.prototypes[0];
        assert_eq!(
            (tri.first_index, tri.index_count, tri.vertex_offset),
            (0, 3, 0)
        );
        assert_eq!((tri.instance_offset, tri.instance_count), (0, 2));
        let quad_range = &plan.prototypes[1];
        assert_eq!(
            (
                quad_range.first_index,
                quad_range.index_count,
                quad_range.vertex_offset
            ),
            (3, 6, 3)
        );
        assert_eq!(
            (quad_range.instance_offset, quad_range.instance_count),
            (2, 2)
        );
        // Records are grouped per prototype in input order within each group.
        let records: Vec<[f32; 4]> = plan
            .instance_bytes
            .chunks_exact(INSTANCE_RECORD_SIZE)
            .map(|c| *bytemuck::from_bytes(c))
            .collect();
        assert_eq!(records[0], [0., 0., 0., 0.]);
        assert_eq!(records[1], [0., -4., 2., 2.]);
        assert_eq!(records[2], [5., 0., 0., 1.]);
        assert_eq!(records[3], [-3., 2., 1., 3.]);
        assert_eq!(plan.bounds[0], [-3., -4., -1.]);
        assert_eq!(plan.bounds[1], [5., 3., 2.]);
    }

    #[test]
    fn quarter_yaw_rotates_instance_bounds_exactly() {
        // Prototype spans x 0..1, z 0..2.
        let plan = plan_static_scene(
            &[mesh(
                &[[0., 0., 0.], [1., 0., 0.], [0., 0., 2.]],
                &[0, 1, 2],
                1,
            )],
            &[instance(0, [-10., 0., 0.], 1)],
        )
        .unwrap();
        // q=1 maps (x,z) -> (z,-x): x 0..2, z -1..0, plus translation x -10.
        let batch = &plan.prototypes[0];
        assert_eq!(batch.bounds[0], [-10., 0., -1.]);
        assert_eq!(batch.bounds[1], [-8., 0., 0.]);
        for (yaw, min, max) in [
            (0u8, [0., 0., 0.], [1., 1., 0.]),
            // The flat z=0 triangle rotates into negative z for q=1.
            (1, [0., 0., -1.], [0., 1., 0.]),
            (2, [-1., 0., 0.], [0., 1., 0.]),
            (3, [0., 0., 0.], [0., 1., 1.]),
        ] {
            let plan =
                plan_static_scene(&[triangle(1)], &[instance(0, [0., 0., 0.], yaw)]).unwrap();
            let batch = &plan.prototypes[0];
            assert_eq!(batch.bounds[0], min, "yaw {yaw} minimum");
            assert_eq!(batch.bounds[1], max, "yaw {yaw} maximum");
        }
    }

    #[test]
    fn invalid_updates_are_rejected_without_a_plan() {
        let meshes = [triangle(1)];
        let cases: Vec<(&str, Vec<StaticInstance>)> = vec![
            ("prototype out of range", vec![instance(1, [0.; 3], 0)]),
            ("yaw beyond quarters", vec![instance(0, [0.; 3], 4)]),
            (
                "non-finite translation",
                vec![instance(0, [0., f32::NAN, 0.], 0)],
            ),
        ];
        for (name, instances) in cases {
            assert!(plan_static_scene(&meshes, &instances).is_err(), "{name}");
        }
        let mut broken = triangle(1);
        broken.indices.push(99);
        assert!(plan_static_scene(&[broken], &[instance(0, [0.; 3], 0)]).is_err());
    }

    #[test]
    fn budgets_reject_whole_updates_before_allocation() {
        let meshes = [triangle(1)];
        // 2 instances * 16 bytes > 17-byte instance budget.
        assert!(plan_static_scene_with_budgets(
            &meshes,
            &[instance(0, [0.; 3], 0), instance(0, [1.; 3], 1)],
            STATIC_MESH_BUDGET_BYTES,
            17,
        )
        .is_err());
        // Geometry budget covers merged vertices plus indices.
        assert!(plan_static_scene_with_budgets(
            &meshes,
            &[instance(0, [0.; 3], 0)],
            100,
            usize::MAX
        )
        .is_err());
        // Just at the budget is accepted.
        assert!(plan_static_scene_with_budgets(
            &meshes,
            &[instance(0, [0.; 3], 0)],
            usize::MAX,
            16
        )
        .is_ok());
    }

    #[test]
    fn instances_on_empty_prototypes_are_rejected() {
        let empty = Mesh::default();
        // An instance pointing at a vertexless prototype has no drawable
        // geometry and no meaningful bounds; the whole update is rejected.
        assert!(plan_static_scene(&[triangle(1), empty], &[instance(1, [2., 0., 0.], 0)]).is_err());
        // Instances on real prototypes without indices draw nothing but stay valid.
        let no_indices = Mesh {
            vertices: triangle(1).vertices,
            indices: Vec::new(),
            revision: 2,
        };
        let plan = plan_static_scene(&[no_indices], &[instance(0, [2., 0., 0.], 1)]).unwrap();
        assert_eq!(plan.instance_count, 1);
        assert!(plan.prototypes.is_empty());
        assert!(!plan.has_geometry);
    }

    #[test]
    fn camera_push_constant_layout_is_frozen_at_80_bytes() {
        // The instance path must not have grown the shared push constant.
        assert_eq!(
            std::mem::size_of::<super::super::Camera>(),
            80,
            "world push constant must stay 80 bytes"
        );
    }

    #[test]
    fn full_showcase_scale_fits_the_pinned_budgets() {
        // The planned wetland scene: 880 prototype meshes totaling 44.5 MiB and
        // 7470 instances must plan successfully inside 128 MiB + 16 MiB.
        let vertices_per_prototype = 1472; // 1472 * 36 B + 3 * 4 B ~= 51.8 KiB
        let meshes: Vec<Mesh> = (0..880)
            .map(|p| {
                let indices = vec![0u32, 1, 2];
                Mesh {
                    vertices: vec![
                        Vertex {
                            position: [0., 0., 0.],
                            normal: [0., 1., 0.],
                            color: [0.5; 3],
                        };
                        vertices_per_prototype
                    ],
                    indices,
                    revision: p as u64,
                }
            })
            .collect();
        let instances: Vec<StaticInstance> = (0..7470)
            .map(|i| StaticInstance {
                prototype: i % 880,
                translation: [i as f32 * 0.1 - 100., 0., -50. + (i % 64) as f32],
                yaw_quarters: (i % 4) as u8,
            })
            .collect();
        let plan = plan_static_scene(&meshes, &instances).unwrap();
        assert_eq!(plan.instance_count, 7470);
        assert_eq!(plan.instance_bytes.len(), 7470 * 16);
        assert!(plan.source_bytes > 44 * 1024 * 1024);
        assert!(plan.source_bytes < STATIC_MESH_BUDGET_BYTES);
        assert!(plan.vertex_bytes.len() + plan.index_bytes.len() < STATIC_MESH_BUDGET_BYTES);
        assert!(plan.has_geometry);
    }
}
