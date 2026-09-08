//! Bounded GPU prototype instancing for load/edit-time static scenery.
//!
//! Unique prototype geometry is pooled into shared vertex/index buffers and a
//! single packed per-scene instance buffer, avoiding one allocation pair per
//! plant. Replacement is a transaction: validation and planning happen on the
//! host, buffer construction completes before the live scene is swapped, and
//! any failure retains the previous scene. This module owns no backend types in
//! its public interface.

use super::{validate_mesh, Buffer, Frustum, Result};
use ash::vk;
use bytemuck::{Pod, Zeroable};
use matterweave_core::{Mesh, Vertex};
use std::{collections::BTreeMap, sync::Arc};

/// Host-side allocation ceiling for pooled static geometry (vertices+indices).
pub const STATIC_MESH_BUDGET_BYTES: usize = 128 * 1024 * 1024;
/// Host-side allocation ceiling for the packed per-scene instance buffer.
pub const STATIC_INSTANCE_BUDGET_BYTES: usize = 16 * 1024 * 1024;

/// One instanced placement of a prototype mesh. `yaw_quarters` rotates the
/// prototype about the Y axis in quarter turns (0..=3); translation is world
/// space. This is a CPU description only; the packed GPU record is private.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StaticInstance {
    pub prototype: usize,
    pub translation: [f32; 3],
    pub yaw_quarters: u8,
}

/// Packed GPU instance record: vertex input location 3 as vec4<f32> holds
/// translation xyz plus quarter-turn yaw in w. Must match world.wgsl/shadow.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct InstanceRecord {
    data: [f32; 4],
}

const INSTANCE_RECORD_SIZE: usize = std::mem::size_of::<InstanceRecord>();

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

pub(crate) struct StaticScenePlan {
    pub vertex_bytes: Vec<u8>,
    pub index_bytes: Vec<u8>,
    pub instance_bytes: Vec<u8>,
    pub prototypes: Vec<PrototypeRange>,
    pub bounds: [[f32; 3]; 2],
    pub instance_count: u32,
    pub has_geometry: bool,
    pub source_bytes: usize,
}

impl StaticScenePlan {
    fn empty() -> Self {
        Self {
            vertex_bytes: Vec::new(),
            index_bytes: Vec::new(),
            instance_bytes: Vec::new(),
            prototypes: Vec::new(),
            bounds: [[0.; 3], [0.; 3]],
            instance_count: 0,
            has_geometry: false,
            source_bytes: 0,
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

pub(crate) fn plan_static_scene_with_budgets(
    meshes: &[Mesh],
    instances: &[StaticInstance],
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
    let mut grouped: BTreeMap<usize, Vec<StaticInstance>> = BTreeMap::new();
    for instance in instances {
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
        grouped
            .entry(instance.prototype)
            .or_default()
            .push(*instance);
    }
    let mut plan = StaticScenePlan::empty();
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
                for instance in list {
                    plan.instance_bytes
                        .extend_from_slice(bytemuck::bytes_of(&instance_record(instance)));
                    let placed = translate_bounds(
                        rotate_bounds(bounds, instance.yaw_quarters),
                        instance.translation,
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

/// GPU-side static scene. All buffers are created before construction returns,
/// so a failed replacement never leaves a partially built scene behind.
pub(crate) struct StaticScene {
    pub(crate) vertices: Option<Buffer>,
    pub(crate) indices: Option<Buffer>,
    pub(crate) instances: Buffer,
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
        let allocated_bytes = vertices.as_ref().map_or(0, |b| b.size)
            + indices.as_ref().map_or(0, |b| b.size)
            + instances.size;
        let prototype_count = plan.prototypes.len();
        Ok(Self {
            allocated_bytes,
            prototype_count,
            source_bytes: plan.source_bytes,
            vertices,
            indices,
            instances,
            prototypes: plan.prototypes,
            instance_count: plan.instance_count,
            has_geometry: plan.has_geometry,
            bounds: plan.bounds,
        })
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
