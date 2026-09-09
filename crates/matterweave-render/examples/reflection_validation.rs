//! Headless native Vulkan validation of the reflection path in the **shipping**
//! `world.wgsl` fragment shader.
//!
//! The harness builds the renderer's exact group-0 descriptor layout from
//! [`matterweave_render::shader_contract`] and compiles the same Naga SPIR-V the
//! Renderer uses, so a probe exercises the production shader rather than a copy of
//! its algorithm. Each probe renders one pixel of a known analytic mirror surface
//! and is compared against [`matterweave_render::reflection::reflect_sample`],
//! which slab-clips to the source volume and then uses the authoritative
//! `World::raycast` DDA.
//!
//! The offscreen target is linear `R8G8B8A8_UNORM`, so the comparison happens in
//! the shading space; the swapchain's sRGB encode is a separate monotonic step.
//! PPM artefacts are sRGB-encoded for viewing only.
//!
//! Exit codes: 0 all checks passed, 1 at least one check failed,
//! 2 no usable Vulkan 1.1 graphics device (checks NOT RUN).
use ash::vk;
use bytemuck::Zeroable;
use glam::{Mat4, Vec3};
use matterweave_core::{Mesh, Vertex, World};
use matterweave_render::reflection::{
    footprint_digest, reflect_sample, shade_sample, MaterialTable, ReflectionSample,
    ReflectionVolume, BACKGROUND, DEFAULT_TRACE_STEPS, FOG_DENSITY, MAX_REFLECTION_AXIS,
    MAX_REFLECTION_CELLS, SURFACE_OFFSET,
};
use matterweave_render::shader_contract::{
    LightingUniform, BINDING_INDIRECT_FACES, BINDING_LIGHTING, BINDING_REFLECTION_MATERIALS,
    BINDING_REFLECTION_PALETTE, BINDING_SHADOW_MAP, BINDING_SHADOW_SAMPLER, LIGHTING_UNIFORM_BYTES,
    WORLD_FRAGMENT_SPIRV, WORLD_VERTEX_SPIRV,
};
use matterweave_render::Sun;
use std::path::{Path, PathBuf};

/// Ideal-mirror probes must agree within 3/255 per channel.
const PROBE_TOLERANCE: f32 = 3.0 / 255.0;
/// The nonreflective baseline must be preserved within 1/255 per channel.
const BASELINE_TOLERANCE: f32 = 1.0 / 255.0;
/// A probe is "non-edge" only if consecutive DDA crossings differ by this much in
/// parametric distance and the hit point is this far from any cell edge. The
/// filter is applied before any comparison and to predetermined probes as well.
const MIN_CROSSING_GAP: f64 = 1e-3;
const MIN_HIT_CLEARANCE: f64 = 0.05;
const MIRROR: u8 = 7;
const STONE: u8 = 2;
const TARGET: u8 = 1;
const SOIL: u8 = 3;
const IMAGE_W: u32 = 192;
const IMAGE_H: u32 = 144;

fn err(e: vk::Result) -> String {
    format!("{e:?}")
}

// ---------------------------------------------------------------------------
// CPU models. They mirror the documented shader contract and are written from the
// contract, not from the WGSL source, so agreement is meaningful.
// ---------------------------------------------------------------------------

fn normalize_sun(sun: Sun) -> [f32; 4] {
    let d = Vec3::from_array(sun.direction_to_sun).normalize();
    [d.x, d.y, d.z, sun.intensity]
}

/// Nonreflective `world.wgsl` model: albedo * (ambient + sun * visibility) + fog.
fn shade_primary(albedo: [f32; 3], normal: Vec3, sun: [f32; 4], view_distance: f32) -> [f32; 3] {
    let n = normal.normalize();
    let sunlight = Vec3::new(sun[0], sun[1], sun[2]).dot(n).max(0.) * sun[3];
    let ambient = 0.28 + 0.12 * n.y.max(0.);
    let lit = Vec3::from_array(albedo) * (ambient + sunlight);
    fog(lit.to_array(), view_distance)
}

fn fog(color: [f32; 3], path_length: f32) -> [f32; 3] {
    let f = 1. - (-path_length.max(0.) * FOG_DENSITY).exp();
    Vec3::from_array(color)
        .lerp(Vec3::from_array(BACKGROUND), f)
        .to_array()
}

/// Full expected pixel for a fragment at `point` with normal `n`, seen from `eye`.
fn expected_pixel(
    volume: &ReflectionVolume,
    world: &World,
    table: &MaterialTable,
    point: [f32; 3],
    normal: [f32; 3],
    eye: [f32; 3],
    sun: Sun,
) -> (ReflectionSample, [f32; 3]) {
    let cell = [
        (point[0] - normal[0] * SURFACE_OFFSET).floor() as i32,
        (point[1] - normal[1] * SURFACE_OFFSET).floor() as i32,
        (point[2] - normal[2] * SURFACE_OFFSET).floor() as i32,
    ];
    let material = volume.material_at(cell);
    let mirror = table.mirror(material);
    let n = Vec3::from_array(normal).normalize();
    let light = normalize_sun(sun);
    let view_distance = (Vec3::from_array(point) - Vec3::from_array(eye)).length();
    let mut lit = shade_primary(table.color(material), n, light, view_distance);
    let sample = if mirror > 0. {
        let s = reflect_sample(volume, world, point, normal, eye);
        // Undo the primary-surface fog: the shader fogs the combined path once.
        let unfogged = unfog(lit, view_distance);
        let reflected = shade_sample(volume, &s, sun).unwrap();
        let mixed = Vec3::from_array(unfogged)
            .lerp(Vec3::from_array(reflected), mirror)
            .to_array();
        lit = fog(mixed, view_distance + s.distance);
        s
    } else {
        ReflectionSample::miss(0.)
    };
    (sample, lit)
}

fn unfog(color: [f32; 3], path_length: f32) -> [f32; 3] {
    let f = 1. - (-path_length.max(0.) * FOG_DENSITY).exp();
    let c = Vec3::from_array(color);
    ((c - Vec3::from_array(BACKGROUND) * f) / (1. - f)).to_array()
}

/// Geometric clearance of one ray: the smallest gap between consecutive DDA
/// crossings and the distance from the hit entry point to the nearest cell edge.
/// Computed in f64 from integer grid planes before any comparison.
fn clearance(
    volume: &ReflectionVolume,
    world: &World,
    point: [f32; 3],
    normal: [f32; 3],
    eye: [f32; 3],
    sample: &ReflectionSample,
) -> (f64, f64) {
    let n = Vec3::from_array(normal);
    let p = Vec3::from_array(point).as_dvec3();
    let e = Vec3::from_array(eye).as_dvec3();
    let n = n.as_dvec3().normalize();
    let d = (p - e).normalize();
    let r = d - 2. * d.dot(n) * n;
    let origin = p + n * f64::from(SURFACE_OFFSET);
    let mut gap = f64::INFINITY;
    let mut previous = 0.0f64;
    let lower = volume.origin();
    let upper: [i32; 3] = std::array::from_fn(|a| lower[a] + volume.dimensions()[a] as i32);
    let mut cell = [
        origin.x.floor() as i32,
        origin.y.floor() as i32,
        origin.z.floor() as i32,
    ];
    for _ in 0..512 {
        if (0..3).any(|a| cell[a] < lower[a] || cell[a] >= upper[a]) {
            return (gap, 1.);
        }
        let mut next = [f64::INFINITY; 3];
        for axis in 0..3 {
            if r[axis] == 0. {
                continue;
            }
            let step = if r[axis] > 0. { 1 } else { 0 };
            let boundary = f64::from(cell[axis]) + f64::from(step);
            next[axis] = (boundary - origin[axis]) / r[axis];
        }
        let mut sorted = next;
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        if sorted[1].is_finite() {
            gap = gap.min(sorted[1] - sorted[0]);
        }
        if sample.hit && cell == sample.cell {
            // Distance from the entry point to the nearest grid plane on the two
            // axes that are not the entry face.
            let entry = origin + r * previous;
            let mut nearest = f64::INFINITY;
            for axis in 0..3 {
                if sample.normal[axis] != 0 {
                    continue;
                }
                let f = entry[axis] - entry[axis].floor();
                nearest = nearest.min(f.min(1. - f));
            }
            return (gap, if nearest.is_finite() { nearest } else { 1. });
        }
        previous = sorted[0];
        let mut stepped = false;
        for axis in 0..3 {
            if next[axis] == sorted[0] && r[axis] != 0. {
                cell[axis] += if r[axis] > 0. { 1 } else { -1 };
                stepped = true;
            }
        }
        if !stepped || world.get(cell) != 0 {
            return (gap, 1.);
        }
    }
    (gap, 1.)
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn table() -> MaterialTable {
    let mut color = [[0.5; 3]; 256];
    color[0] = [0.; 3];
    color[MIRROR as usize] = [0.30, 0.34, 0.38];
    color[STONE as usize] = [0.62, 0.63, 0.60];
    color[TARGET as usize] = [0.90, 0.10, 0.05];
    color[SOIL as usize] = [0.35, 0.24, 0.17];
    let mut t = MaterialTable::new(color).unwrap();
    t.set_mirror(MIRROR, 1.0).unwrap();
    t
}

struct Fixture {
    name: &'static str,
    world: World,
    origin: [i32; 3],
    dimensions: [u32; 3],
}

fn fill(world: &mut World, lo: [i32; 3], hi: [i32; 3], material: u8) {
    for x in lo[0]..hi[0] {
        for y in lo[1]..hi[1] {
            for z in lo[2]..hi[2] {
                world.set([x, y, z], material);
            }
        }
    }
}

fn fixtures() -> Vec<Fixture> {
    // A: mirror floor, a red target block and a stone slab wall.
    let mut a = World::new(101);
    fill(&mut a, [-8, -1, -8], [8, 0, 8], MIRROR);
    fill(&mut a, [3, 0, -1], [5, 3, 1], TARGET);
    fill(&mut a, [7, 0, -8], [8, 6, 8], STONE);
    // B: the same layout at negative coordinates.
    let mut b = World::new(102);
    fill(&mut b, [-16, -5, -16], [0, -4, 0], MIRROR);
    fill(&mut b, [-13, -4, -9], [-11, -1, -7], TARGET);
    fill(&mut b, [-9, -4, -16], [-8, 2, 0], STONE);
    // C: a mirror floor over otherwise empty space: every probe misses.
    let mut c = World::new(103);
    fill(&mut c, [-4, -1, -4], [4, 0, 4], MIRROR);
    vec![
        Fixture {
            name: "corridor",
            world: a,
            origin: [-8, -4, -8],
            dimensions: [16, 12, 16],
        },
        Fixture {
            name: "negative",
            world: b,
            origin: [-16, -8, -16],
            dimensions: [16, 12, 16],
        },
        Fixture {
            name: "empty",
            world: c,
            origin: [-4, -4, -4],
            dimensions: [8, 8, 8],
        },
    ]
}

struct Probe {
    name: &'static str,
    fixture: usize,
    point: [f32; 3],
    normal: [f32; 3],
    /// Eye position; the view direction is `normalize(point - eye)`.
    eye: [f32; 3],
}

/// 28 predetermined probes. Their eye offsets were chosen so that consecutive
/// DDA crossings are separated by a wide margin (verified by `clearance`, which
/// rejects any probe below `MIN_CROSSING_GAP`); none was removed after a failure.
fn probes() -> Vec<Probe> {
    let mut out = Vec::new();
    for (name, point, eye) in [
        ("centre to target block", [0.5, 0.0, 0.5], [-9.2, 2.2, 1.9]),
        ("centre to stone wall", [0.5, 0.0, 0.5], [-5.8, 0.7, -1.1]),
        (
            "centre away from geometry",
            [0.5, 0.0, 0.5],
            [3.4, 0.7, -5.3],
        ),
        (
            "centre, ray parallel to z",
            [0.5, 0.0, 0.5],
            [-9.2, 0.7, 0.5],
        ),
        ("-XZ to target block", [-2.5, 0.0, -2.5], [-8.8, 0.7, -4.1]),
        ("-XZ to stone wall", [-2.5, 0.0, -2.5], [-5.3, 0.7, -1.1]),
        (
            "-XZ away from geometry",
            [-2.5, 0.0, -2.5],
            [6.1, 1.3, -6.8],
        ),
        ("+XZ to target block", [3.5, 0.0, 2.5], [4.8, 2.2, 9.3]),
        (
            "near wall to stone wall",
            [6.5, 0.0, -5.5],
            [-3.2, 2.2, -4.1],
        ),
        ("-X +Z open sky", [-6.5, 0.0, 4.5], [2.1, 2.2, 5.9]),
        ("to stone wall at -Z", [1.5, 0.0, -6.5], [-8.2, 1.3, -8.1]),
    ] {
        out.push(Probe {
            name,
            fixture: 0,
            point,
            normal: [0., 1., 0.],
            eye,
        });
    }
    for (name, point, eye) in [
        (
            "negative to stone slab",
            [-12.5, -4.0, -12.5],
            [-15.3, -3.3, -11.1],
        ),
        (
            "negative open sky",
            [-12.5, -4.0, -12.5],
            [-6.7, -2.7, -15.4],
        ),
        (
            "negative to target block",
            [-12.5, -4.0, -12.5],
            [-14.2, -2.7, -19.9],
        ),
        (
            "negative shallow to target",
            [-12.5, -4.0, -8.5],
            [-22.2, -3.3, -15.9],
        ),
        (
            "negative +X to stone slab",
            [-9.5, -4.0, -8.5],
            [-19.2, -1.8, -7.1],
        ),
        (
            "negative +X to target",
            [-9.5, -4.0, -8.5],
            [-0.9, -1.8, -7.1],
        ),
        (
            "negative -X to target",
            [-14.5, -4.0, -3.5],
            [-17.9, -2.7, 3.3],
        ),
        (
            "negative +X to stone slab 2",
            [-6.5, -4.0, -13.5],
            [2.1, -1.8, -12.1],
        ),
        (
            "negative open +Z",
            [-13.5, -4.0, -14.5],
            [-12.2, -1.8, -7.7],
        ),
    ] {
        out.push(Probe {
            name,
            fixture: 1,
            point,
            normal: [0., 1., 0.],
            eye,
        });
    }
    for (name, point, eye) in [
        ("empty oblique", [0.5, 0.0, 0.5], [3.4, 1.3, -5.3]),
        ("empty ray parallel to z", [0.5, 0.0, 0.5], [-9.2, 0.7, 0.5]),
        ("empty away +X", [-2.5, 0.0, -2.5], [6.1, 2.2, -1.1]),
        ("empty away -X", [2.5, 0.0, 1.5], [-7.2, 2.2, 2.9]),
        ("empty away +X high", [-3.5, 0.0, 2.5], [5.1, 2.2, 3.9]),
        ("empty away +Z", [1.5, 0.0, -3.5], [2.8, 2.2, 3.3]),
        ("empty steep", [-1.5, 0.0, -1.5], [7.1, 8.3, -0.1]),
        ("empty shallow +X", [0.5, 0.0, 0.5], [9.1, 0.7, 0.5]),
    ] {
        out.push(Probe {
            name,
            fixture: 2,
            point,
            normal: [0., 1., 0.],
            eye,
        });
    }
    out
}

/// One quad covering `point`, with the given constant normal and vertex colour.
fn quad(point: [f32; 3], normal: [f32; 3], color: [f32; 3], half: f32) -> Mesh {
    let n = Vec3::from_array(normal).normalize();
    let u = if n.y.abs() > 0.9 {
        Vec3::X
    } else {
        Vec3::Y.cross(n).normalize()
    };
    let v = n.cross(u);
    let p = Vec3::from_array(point);
    let corners = [
        p - u * half - v * half,
        p + u * half - v * half,
        p + u * half + v * half,
        p - u * half + v * half,
    ];
    Mesh {
        vertices: corners
            .iter()
            .map(|c| Vertex {
                position: c.to_array(),
                normal: n.to_array(),
                color,
            })
            .collect(),
        indices: vec![0, 1, 2, 0, 2, 3],
        revision: 0,
    }
}

/// Reuse the authoritative mesher, then recolour from the material table so the
/// GPU colours and the CPU model use one palette.
fn scene_mesh(world: &World, table: &MaterialTable) -> Mesh {
    let mut mesh = world.mesh();
    for vertex in &mut mesh.vertices {
        let cell = [
            (vertex.position[0] - vertex.normal[0] * SURFACE_OFFSET).floor() as i32,
            (vertex.position[1] - vertex.normal[1] * SURFACE_OFFSET).floor() as i32,
            (vertex.position[2] - vertex.normal[2] * SURFACE_OFFSET).floor() as i32,
        ];
        vertex.color = table.color(world.get(cell));
    }
    mesh
}

// ---------------------------------------------------------------------------
// Headless Vulkan harness
// ---------------------------------------------------------------------------

struct Gpu {
    _entry: ash::Entry,
    instance: ash::Instance,
    device: ash::Device,
    queue: vk::Queue,
    memory: vk::PhysicalDeviceMemoryProperties,
    pool: vk::CommandPool,
    name: String,
}

impl Gpu {
    fn new() -> Result<Self, String> {
        // SAFETY: the loader outlives every handle created from it.
        let entry = unsafe { ash::Entry::load() }.map_err(|e| format!("Vulkan loader: {e}"))?;
        let app = vk::ApplicationInfo::default()
            .application_name(c"matterweave-reflection-validation")
            .api_version(vk::API_VERSION_1_1);
        // SAFETY: static application name outlives the instance.
        let instance = unsafe {
            entry.create_instance(
                &vk::InstanceCreateInfo::default().application_info(&app),
                None,
            )
        }
        .map_err(err)?;
        // SAFETY: queried handles belong to the live instance.
        let (physical, family, name) = unsafe {
            let mut selected = None;
            for physical in instance
                .enumerate_physical_devices()
                .map_err(|e| format!("{e:?}"))?
            {
                let props = instance.get_physical_device_properties(physical);
                if props.api_version < vk::API_VERSION_1_1 {
                    continue;
                }
                for (family, q) in instance
                    .get_physical_device_queue_family_properties(physical)
                    .iter()
                    .enumerate()
                {
                    if q.queue_flags.contains(vk::QueueFlags::GRAPHICS) {
                        selected = Some((
                            physical,
                            family as u32,
                            std::ffi::CStr::from_ptr(props.device_name.as_ptr())
                                .to_string_lossy()
                                .into_owned(),
                        ));
                        break;
                    }
                }
                if selected.is_some() {
                    break;
                }
            }
            selected
        }
        .ok_or("No Vulkan 1.1 graphics device")?;
        let priorities = [1.0];
        let queues = [vk::DeviceQueueCreateInfo::default()
            .queue_family_index(family)
            .queue_priorities(&priorities)];
        // SAFETY: the selected queue family exists.
        let device = unsafe {
            instance.create_device(
                physical,
                &vk::DeviceCreateInfo::default().queue_create_infos(&queues),
                None,
            )
        }
        .map_err(err)?;
        // SAFETY: queue zero was requested above.
        let (queue, memory) = unsafe {
            (
                device.get_device_queue(family, 0),
                instance.get_physical_device_memory_properties(physical),
            )
        };
        let pool = unsafe {
            device.create_command_pool(
                &vk::CommandPoolCreateInfo::default()
                    .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER)
                    .queue_family_index(family),
                None,
            )
        }
        .map_err(err)?;
        Ok(Self {
            _entry: entry,
            instance,
            device,
            queue,
            memory,
            pool,
            name,
        })
    }
    fn memory_type(&self, bits: u32, flags: vk::MemoryPropertyFlags) -> Result<u32, String> {
        (0..self.memory.memory_type_count)
            .find(|&i| {
                bits & (1 << i) != 0
                    && self.memory.memory_types[i as usize]
                        .property_flags
                        .contains(flags)
            })
            .ok_or_else(|| format!("No memory type for {flags:?}"))
    }
}

struct Allocation {
    handle: vk::Buffer,
    memory: vk::DeviceMemory,
    size: usize,
}

struct Image {
    handle: vk::Image,
    memory: vk::DeviceMemory,
    view: vk::ImageView,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Camera {
    view_proj: [[f32; 4]; 4],
    eye: [f32; 4],
}

struct Harness {
    gpu: Gpu,
    width: u32,
    height: u32,
    color: Image,
    depth: Image,
    readback: Allocation,
    uniform: Allocation,
    shadow: Image,
    sampler: vk::Sampler,
    indirect: Allocation,
    materials: Allocation,
    palette: Allocation,
    vertices: Allocation,
    indices: Allocation,
    identity: Allocation,
    set_layout: vk::DescriptorSetLayout,
    pool: vk::DescriptorPool,
    set: vk::DescriptorSet,
    pass: vk::RenderPass,
    framebuffer: vk::Framebuffer,
    layout: vk::PipelineLayout,
    pipeline: vk::Pipeline,
    buffer: vk::CommandBuffer,
}

impl Harness {
    fn new(gpu: Gpu, width: u32, height: u32) -> Result<Self, String> {
        let d = &gpu.device;
        // SAFETY: every handle is stored immediately; resources are destroyed in Drop.
        unsafe {
            let alloc = |size: usize, usage: vk::BufferUsageFlags, initial: &[u8]| {
                allocation(&gpu, size, usage, initial)
            };
            let uniform = alloc(
                LIGHTING_UNIFORM_BYTES,
                vk::BufferUsageFlags::UNIFORM_BUFFER,
                bytemuck::bytes_of(&LightingUniform::zeroed()),
            )?;
            let indirect = alloc(
                16,
                vk::BufferUsageFlags::STORAGE_BUFFER,
                bytemuck::cast_slice(&[[0.0f32; 4]]),
            )?;
            let materials = alloc(
                4 * MAX_REFLECTION_CELLS,
                vk::BufferUsageFlags::STORAGE_BUFFER,
                &[0u8; 16],
            )?;
            let palette = alloc(
                matterweave_render::reflection::REFLECTION_PALETTE_BYTES,
                vk::BufferUsageFlags::STORAGE_BUFFER,
                bytemuck::cast_slice(&[[0.0f32; 4]; 256]),
            )?;
            let vertices = alloc(4 << 20, vk::BufferUsageFlags::VERTEX_BUFFER, &[0u8; 16])?;
            let indices = alloc(1 << 20, vk::BufferUsageFlags::INDEX_BUFFER, &[0u8; 16])?;
            let identity = alloc(
                16,
                vk::BufferUsageFlags::VERTEX_BUFFER,
                bytemuck::cast_slice(&[0.0f32; 4]),
            )?;
            let pixels = (width * height * 4) as usize;
            let readback = alloc(
                pixels,
                vk::BufferUsageFlags::TRANSFER_DST,
                &vec![0u8; pixels],
            )?;
            let color = attachment(
                &gpu,
                width,
                height,
                vk::Format::R8G8B8A8_UNORM,
                vk::ImageAspectFlags::COLOR,
            )?;
            let depth = attachment(
                &gpu,
                width,
                height,
                vk::Format::D32_SFLOAT,
                vk::ImageAspectFlags::DEPTH,
            )?;
            let shadow = attachment(
                &gpu,
                1,
                1,
                vk::Format::D32_SFLOAT,
                vk::ImageAspectFlags::DEPTH,
            )?;
            let sampler = d
                .create_sampler(
                    &vk::SamplerCreateInfo::default()
                        .mag_filter(vk::Filter::NEAREST)
                        .min_filter(vk::Filter::NEAREST)
                        .compare_enable(true)
                        .compare_op(vk::CompareOp::LESS_OR_EQUAL)
                        .border_color(vk::BorderColor::FLOAT_OPAQUE_WHITE),
                    None,
                )
                .map_err(err)?;
            let types = [
                vk::DescriptorType::UNIFORM_BUFFER,
                vk::DescriptorType::SAMPLED_IMAGE,
                vk::DescriptorType::SAMPLER,
                vk::DescriptorType::STORAGE_BUFFER,
                vk::DescriptorType::STORAGE_BUFFER,
                vk::DescriptorType::STORAGE_BUFFER,
            ];
            let bindings: Vec<_> = types
                .iter()
                .enumerate()
                .map(|(i, &ty)| {
                    vk::DescriptorSetLayoutBinding::default()
                        .binding(i as u32)
                        .descriptor_type(ty)
                        .descriptor_count(1)
                        .stage_flags(vk::ShaderStageFlags::FRAGMENT)
                })
                .collect();
            let set_layout = d
                .create_descriptor_set_layout(
                    &vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings),
                    None,
                )
                .map_err(err)?;
            let mut sizes: Vec<vk::DescriptorPoolSize> = Vec::new();
            for &ty in types.iter() {
                match sizes.iter_mut().find(|s| s.ty == ty) {
                    Some(size) => size.descriptor_count += 1,
                    None => sizes.push(vk::DescriptorPoolSize {
                        ty,
                        descriptor_count: 1,
                    }),
                }
            }
            let pool = d
                .create_descriptor_pool(
                    &vk::DescriptorPoolCreateInfo::default()
                        .max_sets(1)
                        .pool_sizes(&sizes),
                    None,
                )
                .map_err(err)?;
            let set = d
                .allocate_descriptor_sets(
                    &vk::DescriptorSetAllocateInfo::default()
                        .descriptor_pool(pool)
                        .set_layouts(&[set_layout]),
                )
                .map_err(err)?[0];
            let uniform_info = [vk::DescriptorBufferInfo::default()
                .buffer(uniform.handle)
                .range(LIGHTING_UNIFORM_BYTES as u64)];
            let indirect_info = [vk::DescriptorBufferInfo::default()
                .buffer(indirect.handle)
                .range(16)];
            let materials_info = [vk::DescriptorBufferInfo::default()
                .buffer(materials.handle)
                .range(vk::WHOLE_SIZE)];
            let palette_info = [vk::DescriptorBufferInfo::default()
                .buffer(palette.handle)
                .range(vk::WHOLE_SIZE)];
            let image_info = [vk::DescriptorImageInfo::default()
                .image_view(shadow.view)
                .image_layout(vk::ImageLayout::DEPTH_STENCIL_READ_ONLY_OPTIMAL)];
            let sampler_info = [vk::DescriptorImageInfo::default().sampler(sampler)];
            d.update_descriptor_sets(
                &[
                    vk::WriteDescriptorSet::default()
                        .dst_set(set)
                        .dst_binding(BINDING_LIGHTING)
                        .descriptor_type(types[0])
                        .buffer_info(&uniform_info),
                    vk::WriteDescriptorSet::default()
                        .dst_set(set)
                        .dst_binding(BINDING_SHADOW_MAP)
                        .descriptor_type(types[1])
                        .image_info(&image_info),
                    vk::WriteDescriptorSet::default()
                        .dst_set(set)
                        .dst_binding(BINDING_SHADOW_SAMPLER)
                        .descriptor_type(types[2])
                        .image_info(&sampler_info),
                    vk::WriteDescriptorSet::default()
                        .dst_set(set)
                        .dst_binding(BINDING_INDIRECT_FACES)
                        .descriptor_type(types[3])
                        .buffer_info(&indirect_info),
                    vk::WriteDescriptorSet::default()
                        .dst_set(set)
                        .dst_binding(BINDING_REFLECTION_MATERIALS)
                        .descriptor_type(types[4])
                        .buffer_info(&materials_info),
                    vk::WriteDescriptorSet::default()
                        .dst_set(set)
                        .dst_binding(BINDING_REFLECTION_PALETTE)
                        .descriptor_type(types[5])
                        .buffer_info(&palette_info),
                ],
                &[],
            );
            let attachments = [
                vk::AttachmentDescription::default()
                    .format(vk::Format::R8G8B8A8_UNORM)
                    .samples(vk::SampleCountFlags::TYPE_1)
                    .load_op(vk::AttachmentLoadOp::CLEAR)
                    .store_op(vk::AttachmentStoreOp::STORE)
                    .initial_layout(vk::ImageLayout::UNDEFINED)
                    .final_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL),
                vk::AttachmentDescription::default()
                    .format(vk::Format::D32_SFLOAT)
                    .samples(vk::SampleCountFlags::TYPE_1)
                    .load_op(vk::AttachmentLoadOp::CLEAR)
                    .store_op(vk::AttachmentStoreOp::DONT_CARE)
                    .initial_layout(vk::ImageLayout::UNDEFINED)
                    .final_layout(vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL),
            ];
            let color_ref = [vk::AttachmentReference {
                attachment: 0,
                layout: vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
            }];
            let depth_ref = vk::AttachmentReference {
                attachment: 1,
                layout: vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL,
            };
            let subpass = [vk::SubpassDescription::default()
                .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
                .color_attachments(&color_ref)
                .depth_stencil_attachment(&depth_ref)];
            let pass = d
                .create_render_pass(
                    &vk::RenderPassCreateInfo::default()
                        .attachments(&attachments)
                        .subpasses(&subpass),
                    None,
                )
                .map_err(err)?;
            let views = [color.view, depth.view];
            let framebuffer = d
                .create_framebuffer(
                    &vk::FramebufferCreateInfo::default()
                        .render_pass(pass)
                        .attachments(&views)
                        .width(width)
                        .height(height)
                        .layers(1),
                    None,
                )
                .map_err(err)?;
            let push = [vk::PushConstantRange::default()
                .stage_flags(vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT)
                .offset(0)
                .size(80)];
            let layout = d
                .create_pipeline_layout(
                    &vk::PipelineLayoutCreateInfo::default()
                        .push_constant_ranges(&push)
                        .set_layouts(&[set_layout]),
                    None,
                )
                .map_err(err)?;
            let pipeline = graphics_pipeline(&gpu, layout, pass)?;
            let buffer = d
                .allocate_command_buffers(
                    &vk::CommandBufferAllocateInfo::default()
                        .command_pool(gpu.pool)
                        .level(vk::CommandBufferLevel::PRIMARY)
                        .command_buffer_count(1),
                )
                .map_err(err)?[0];
            Ok(Self {
                gpu,
                width,
                height,
                color,
                depth,
                readback,
                uniform,
                shadow,
                sampler,
                indirect,
                materials,
                palette,
                vertices,
                indices,
                identity,
                set_layout,
                pool,
                set,
                pass,
                framebuffer,
                layout,
                pipeline,
                buffer,
            })
        }
    }

    /// Publishes a reflection volume into the harness buffers, mirroring
    /// `Renderer::upload_reflection` without the frame-fence contract.
    fn publish(&mut self, volume: &ReflectionVolume) -> Result<usize, String> {
        let materials = bytemuck::cast_slice::<u32, u8>(volume.materials());
        let palette = bytemuck::cast_slice::<[f32; 4], u8>(volume.palette());
        write(&self.gpu, &self.materials, materials)?;
        write(&self.gpu, &self.palette, palette)?;
        Ok(materials.len() + palette.len())
    }

    /// Renders one image and reads it back as RGBA8 linear pixels.
    fn render(
        &mut self,
        mesh: &Mesh,
        camera: &Camera,
        lighting: &LightingUniform,
        reflection_enabled: bool,
    ) -> Result<Vec<[u8; 4]>, String> {
        let d = &self.gpu.device;
        let mut lighting = *lighting;
        if !reflection_enabled {
            lighting.reflection_dimensions[3] = 0;
        }
        write(&self.gpu, &self.uniform, bytemuck::bytes_of(&lighting))?;
        let vertices = bytemuck::cast_slice::<Vertex, u8>(&mesh.vertices);
        let indices = bytemuck::cast_slice::<u32, u8>(&mesh.indices);
        write(&self.gpu, &self.vertices, vertices)?;
        write(&self.gpu, &self.indices, indices)?;
        // SAFETY: one command buffer, owned resources, device idle before readback.
        unsafe {
            d.reset_command_buffer(self.buffer, vk::CommandBufferResetFlags::empty())
                .map_err(err)?;
            d.begin_command_buffer(
                self.buffer,
                &vk::CommandBufferBeginInfo::default()
                    .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            )
            .map_err(err)?;
            let clears = [
                vk::ClearValue {
                    color: vk::ClearColorValue {
                        float32: [0.16, 0.24, 0.29, 1.0],
                    },
                },
                vk::ClearValue {
                    depth_stencil: vk::ClearDepthStencilValue {
                        depth: 1.0,
                        stencil: 0,
                    },
                },
            ];
            let area = vk::Rect2D {
                offset: vk::Offset2D::default(),
                extent: vk::Extent2D {
                    width: self.width,
                    height: self.height,
                },
            };
            d.cmd_begin_render_pass(
                self.buffer,
                &vk::RenderPassBeginInfo::default()
                    .render_pass(self.pass)
                    .framebuffer(self.framebuffer)
                    .render_area(area)
                    .clear_values(&clears),
                vk::SubpassContents::INLINE,
            );
            d.cmd_set_viewport(
                self.buffer,
                0,
                &[vk::Viewport {
                    x: 0.,
                    y: 0.,
                    width: self.width as f32,
                    height: self.height as f32,
                    min_depth: 0.,
                    max_depth: 1.,
                }],
            );
            d.cmd_set_scissor(self.buffer, 0, &[area]);
            d.cmd_bind_pipeline(self.buffer, vk::PipelineBindPoint::GRAPHICS, self.pipeline);
            d.cmd_bind_descriptor_sets(
                self.buffer,
                vk::PipelineBindPoint::GRAPHICS,
                self.layout,
                0,
                &[self.set],
                &[],
            );
            d.cmd_push_constants(
                self.buffer,
                self.layout,
                vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT,
                0,
                bytemuck::bytes_of(camera),
            );
            d.cmd_bind_vertex_buffers(self.buffer, 0, &[self.vertices.handle], &[0]);
            d.cmd_bind_vertex_buffers(self.buffer, 1, &[self.identity.handle], &[0]);
            d.cmd_bind_index_buffer(self.buffer, self.indices.handle, 0, vk::IndexType::UINT32);
            d.cmd_draw_indexed(self.buffer, mesh.indices.len() as u32, 1, 0, 0, 0);
            d.cmd_end_render_pass(self.buffer);
            let extent = vk::Extent3D {
                width: self.width,
                height: self.height,
                depth: 1,
            };
            let copy = [vk::BufferImageCopy::default()
                .image_subresource(
                    vk::ImageSubresourceLayers::default()
                        .aspect_mask(vk::ImageAspectFlags::COLOR)
                        .layer_count(1),
                )
                .image_extent(extent)];
            d.cmd_copy_image_to_buffer(
                self.buffer,
                self.color.handle,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                self.readback.handle,
                &copy,
            );
            d.cmd_pipeline_barrier(
                self.buffer,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::HOST,
                vk::DependencyFlags::empty(),
                &[vk::MemoryBarrier::default()
                    .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                    .dst_access_mask(vk::AccessFlags::HOST_READ)],
                &[],
                &[],
            );
            d.end_command_buffer(self.buffer).map_err(err)?;
            d.queue_submit(
                self.gpu.queue,
                &[vk::SubmitInfo::default().command_buffers(&[self.buffer])],
                vk::Fence::null(),
            )
            .map_err(err)?;
            d.device_wait_idle().map_err(err)?;
            let pixels = (self.width * self.height) as usize;
            let pointer = d
                .map_memory(
                    self.readback.memory,
                    0,
                    (pixels * 4) as u64,
                    vk::MemoryMapFlags::empty(),
                )
                .map_err(err)?
                .cast::<[u8; 4]>();
            let image = std::slice::from_raw_parts(pointer, pixels).to_vec();
            d.unmap_memory(self.readback.memory);
            Ok(image)
        }
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        // SAFETY: device is idle; children destroyed before their pools/layouts.
        unsafe {
            let d = &self.gpu.device;
            d.destroy_pipeline(self.pipeline, None);
            d.destroy_pipeline_layout(self.layout, None);
            d.destroy_framebuffer(self.framebuffer, None);
            d.destroy_render_pass(self.pass, None);
            d.destroy_descriptor_pool(self.pool, None);
            d.destroy_descriptor_set_layout(self.set_layout, None);
            d.destroy_sampler(self.sampler, None);
            for image in [&self.color, &self.depth, &self.shadow] {
                d.destroy_image_view(image.view, None);
                d.destroy_image(image.handle, None);
                d.free_memory(image.memory, None);
            }
            for buffer in [
                &self.readback,
                &self.uniform,
                &self.indirect,
                &self.materials,
                &self.palette,
                &self.vertices,
                &self.indices,
                &self.identity,
            ] {
                d.destroy_buffer(buffer.handle, None);
                d.free_memory(buffer.memory, None);
            }
            d.destroy_command_pool(self.gpu.pool, None);
            d.destroy_device(None);
            self.gpu.instance.destroy_instance(None);
        }
    }
}

fn allocation(
    gpu: &Gpu,
    size: usize,
    usage: vk::BufferUsageFlags,
    initial: &[u8],
) -> Result<Allocation, String> {
    // SAFETY: owned buffer; requirements are queried before binding.
    unsafe {
        let handle = gpu
            .device
            .create_buffer(
                &vk::BufferCreateInfo::default()
                    .size(size.max(4) as u64)
                    .usage(usage)
                    .sharing_mode(vk::SharingMode::EXCLUSIVE),
                None,
            )
            .map_err(err)?;
        let requirements = gpu.device.get_buffer_memory_requirements(handle);
        let index = gpu.memory_type(
            requirements.memory_type_bits,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        )?;
        let memory = gpu
            .device
            .allocate_memory(
                &vk::MemoryAllocateInfo::default()
                    .allocation_size(requirements.size)
                    .memory_type_index(index),
                None,
            )
            .map_err(err)?;
        gpu.device
            .bind_buffer_memory(handle, memory, 0)
            .map_err(err)?;
        let out = Allocation {
            handle,
            memory,
            size: size.max(4),
        };
        write(gpu, &out, initial)?;
        Ok(out)
    }
}

fn write(gpu: &Gpu, target: &Allocation, bytes: &[u8]) -> Result<(), String> {
    if bytes.is_empty() {
        return Ok(());
    }
    assert!(
        bytes.len() <= target.size,
        "harness buffer too small: {} > {}",
        bytes.len(),
        target.size
    );
    // SAFETY: exclusive, coherent, host-visible allocation; device is idle.
    unsafe {
        let pointer = gpu
            .device
            .map_memory(
                target.memory,
                0,
                bytes.len() as u64,
                vk::MemoryMapFlags::empty(),
            )
            .map_err(err)?;
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), pointer.cast(), bytes.len());
        gpu.device.unmap_memory(target.memory);
    }
    Ok(())
}

fn attachment(
    gpu: &Gpu,
    width: u32,
    height: u32,
    format: vk::Format,
    aspect: vk::ImageAspectFlags,
) -> Result<Image, String> {
    // SAFETY: owned image; requirements queried before binding.
    unsafe {
        let handle = gpu
            .device
            .create_image(
                &vk::ImageCreateInfo::default()
                    .image_type(vk::ImageType::TYPE_2D)
                    .format(format)
                    .extent(vk::Extent3D {
                        width,
                        height,
                        depth: 1,
                    })
                    .mip_levels(1)
                    .array_layers(1)
                    .samples(vk::SampleCountFlags::TYPE_1)
                    .tiling(vk::ImageTiling::OPTIMAL)
                    .usage(
                        if aspect == vk::ImageAspectFlags::COLOR {
                            vk::ImageUsageFlags::COLOR_ATTACHMENT
                        } else {
                            vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT
                        } | vk::ImageUsageFlags::TRANSFER_SRC
                            | vk::ImageUsageFlags::SAMPLED,
                    )
                    .sharing_mode(vk::SharingMode::EXCLUSIVE),
                None,
            )
            .map_err(err)?;
        let requirements = gpu.device.get_image_memory_requirements(handle);
        let index = gpu.memory_type(
            requirements.memory_type_bits,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
        )?;
        let memory = gpu
            .device
            .allocate_memory(
                &vk::MemoryAllocateInfo::default()
                    .allocation_size(requirements.size)
                    .memory_type_index(index),
                None,
            )
            .map_err(err)?;
        gpu.device
            .bind_image_memory(handle, memory, 0)
            .map_err(err)?;
        let view = gpu
            .device
            .create_image_view(
                &vk::ImageViewCreateInfo::default()
                    .image(handle)
                    .view_type(vk::ImageViewType::TYPE_2D)
                    .format(format)
                    .subresource_range(
                        vk::ImageSubresourceRange::default()
                            .aspect_mask(aspect)
                            .level_count(1)
                            .layer_count(1),
                    ),
                None,
            )
            .map_err(err)?;
        Ok(Image {
            handle,
            memory,
            view,
        })
    }
}

fn graphics_pipeline(
    gpu: &Gpu,
    layout: vk::PipelineLayout,
    pass: vk::RenderPass,
) -> Result<vk::Pipeline, String> {
    // SAFETY: SPIR-V produced by Naga at build time; layouts are owned.
    unsafe {
        let module = |words: &[u8]| -> Result<vk::ShaderModule, String> {
            let words: Vec<u32> = words
                .chunks_exact(4)
                .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                .collect();
            gpu.device
                .create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&words), None)
                .map_err(err)
        };
        let vs = module(WORLD_VERTEX_SPIRV)?;
        let fs = module(WORLD_FRAGMENT_SPIRV)?;
        let stages = [
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::VERTEX)
                .module(vs)
                .name(c"vs_main"),
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::FRAGMENT)
                .module(fs)
                .name(c"fs_main"),
        ];
        let bindings = [
            vk::VertexInputBindingDescription::default()
                .binding(0)
                .stride(size_of::<Vertex>() as u32)
                .input_rate(vk::VertexInputRate::VERTEX),
            vk::VertexInputBindingDescription::default()
                .binding(1)
                .stride(16)
                .input_rate(vk::VertexInputRate::VERTEX),
        ];
        let attributes = [
            vk::VertexInputAttributeDescription::default()
                .binding(0)
                .location(0)
                .format(vk::Format::R32G32B32_SFLOAT)
                .offset(0),
            vk::VertexInputAttributeDescription::default()
                .binding(0)
                .location(1)
                .format(vk::Format::R32G32B32_SFLOAT)
                .offset(12),
            vk::VertexInputAttributeDescription::default()
                .binding(0)
                .location(2)
                .format(vk::Format::R32G32B32_SFLOAT)
                .offset(24),
            vk::VertexInputAttributeDescription::default()
                .binding(1)
                .location(3)
                .format(vk::Format::R32G32B32A32_SFLOAT)
                .offset(0),
        ];
        let vertex = vk::PipelineVertexInputStateCreateInfo::default()
            .vertex_binding_descriptions(&bindings)
            .vertex_attribute_descriptions(&attributes);
        let assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
            .topology(vk::PrimitiveTopology::TRIANGLE_LIST);
        let viewport = vk::PipelineViewportStateCreateInfo::default()
            .viewport_count(1)
            .scissor_count(1);
        let raster = vk::PipelineRasterizationStateCreateInfo::default()
            .polygon_mode(vk::PolygonMode::FILL)
            .cull_mode(vk::CullModeFlags::NONE)
            .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
            .line_width(1.0);
        let multisample = vk::PipelineMultisampleStateCreateInfo::default()
            .rasterization_samples(vk::SampleCountFlags::TYPE_1);
        let depth = vk::PipelineDepthStencilStateCreateInfo::default()
            .depth_test_enable(true)
            .depth_write_enable(true)
            .depth_compare_op(vk::CompareOp::LESS);
        let blend = [vk::PipelineColorBlendAttachmentState::default()
            .color_write_mask(vk::ColorComponentFlags::RGBA)];
        let color = vk::PipelineColorBlendStateCreateInfo::default().attachments(&blend);
        let dynamic = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
        let dynamic_state = vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic);
        let pipeline = gpu
            .device
            .create_graphics_pipelines(
                vk::PipelineCache::null(),
                &[vk::GraphicsPipelineCreateInfo::default()
                    .stages(&stages)
                    .vertex_input_state(&vertex)
                    .input_assembly_state(&assembly)
                    .viewport_state(&viewport)
                    .rasterization_state(&raster)
                    .multisample_state(&multisample)
                    .depth_stencil_state(&depth)
                    .color_blend_state(&color)
                    .dynamic_state(&dynamic_state)
                    .layout(layout)
                    .render_pass(pass)
                    .subpass(0)],
                None,
            )
            .map_err(|e| format!("{e:?}"))?[0];
        gpu.device.destroy_shader_module(vs, None);
        gpu.device.destroy_shader_module(fs, None);
        Ok(pipeline)
    }
}

// ---------------------------------------------------------------------------
// Output artefacts
// ---------------------------------------------------------------------------

fn write_ppm(path: &Path, width: u32, height: u32, pixels: &[[u8; 4]]) -> Result<(), String> {
    let mut bytes = format!("P6\n{width} {height}\n255\n").into_bytes();
    for pixel in pixels {
        for channel in &pixel[..3] {
            let linear = f32::from(*channel) / 255.;
            let srgb = if linear <= 0.0031308 {
                linear * 12.92
            } else {
                1.055 * linear.powf(1. / 2.4) - 0.055
            };
            bytes.push((srgb.clamp(0., 1.) * 255. + 0.5) as u8);
        }
    }
    std::fs::write(path, bytes).map_err(|e| e.to_string())
}

struct Case {
    id: String,
    status: &'static str,
    detail: String,
}

fn main() {
    let out = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("reflection-validation"));
    if let Err(e) = std::fs::create_dir_all(&out) {
        eprintln!(
            "reflection_validation: cannot create {}: {e}",
            out.display()
        );
        std::process::exit(1);
    }
    let gpu = match Gpu::new() {
        Ok(gpu) => gpu,
        Err(e) => {
            eprintln!("reflection_validation: NOT RUN, no Vulkan device: {e}");
            std::process::exit(2);
        }
    };
    println!("reflection_validation: device {}", gpu.name);
    let mut cases: Vec<Case> = Vec::new();
    let mut failures = 0usize;
    let started = std::time::Instant::now();

    let mut probe_harness = match Harness::new(gpu, 1, 1) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("reflection_validation: harness failed: {e}");
            std::process::exit(1);
        }
    };

    let sun = Sun {
        direction_to_sun: [0.4, 0.85, 0.3],
        intensity: 0.8,
    };
    let light = normalize_sun(sun);

    // --- Case 1: nonreflective baseline preserved within 1/255. -------------
    {
        let fixtures = fixtures();
        let flat = MaterialTable::new(table().colors()).unwrap(); // all mirrors zero
        let mut worst = 0.0f32;
        let mut compared = 0usize;
        for fixture in &fixtures {
            let volume = ReflectionVolume::pack(
                &fixture.world,
                0,
                fixture.origin,
                fixture.dimensions,
                &flat,
                DEFAULT_TRACE_STEPS,
            )
            .unwrap();
            probe_harness.publish(&volume).unwrap();
            for probe in probes()
                .iter()
                .filter(|p| p.fixture == fixtures_index(&fixtures, fixture.name))
            {
                let mesh = quad(probe.point, probe.normal, [0.5, 0.5, 0.5], 0.4);
                let (camera, eye) = camera_for(probe);
                let lighting = LightingUniform {
                    view_proj: Mat4::IDENTITY.to_cols_array_2d(),
                    sun: light,
                    params: [0., 0., 1., 0.],
                    indirect_origin: [0; 4],
                    indirect_dimensions: [0; 4],
                    reflection_origin: [
                        volume.origin()[0],
                        volume.origin()[1],
                        volume.origin()[2],
                        0,
                    ],
                    reflection_dimensions: [
                        volume.dimensions()[0],
                        volume.dimensions()[1],
                        volume.dimensions()[2],
                        1,
                    ],
                    reflection_params: [volume.trace_bound() as f32, SURFACE_OFFSET, 0., 0.],
                };
                let on = probe_harness
                    .render(&mesh, &camera, &lighting, true)
                    .unwrap()[0];
                let off = probe_harness
                    .render(&mesh, &camera, &lighting, false)
                    .unwrap()[0];
                let expected = shade_primary(
                    [0.5, 0.5, 0.5],
                    Vec3::from_array(probe.normal),
                    light,
                    (Vec3::from_array(probe.point) - Vec3::from_array(eye)).length(),
                );
                for channel in 0..3 {
                    worst = worst
                        .max((f32::from(on[channel]) / 255. - expected[channel]).abs())
                        .max((f32::from(off[channel]) / 255. - expected[channel]).abs())
                        .max((f32::from(on[channel]) - f32::from(off[channel])).abs() / 255.);
                }
                compared += 1;
            }
        }
        let status = if worst <= BASELINE_TOLERANCE {
            "PASS"
        } else {
            failures += 1;
            "FAIL"
        };
        println!("baseline: {compared} pixels, worst error {worst:.6} ({status})");
        cases.push(Case {
            id: "1-nonreflective-baseline-1-255".into(),
            status,
            detail: format!(
                "{compared} pixels, worst {:.6} <= {:.6}",
                worst, BASELINE_TOLERANCE
            ),
        });
    }

    // --- Case 2: predetermined non-edge mirror probes vs the CPU oracle. ----
    let mut report = Vec::new();
    {
        let fixtures = fixtures();
        let t = table();
        let mut worst = 0.0f32;
        let mut hits = 0usize;
        let mut misses = 0usize;
        let mut rejected = Vec::new();
        for (index, probe) in probes().iter().enumerate() {
            let fixture = &fixtures[probe.fixture];
            let volume = ReflectionVolume::pack(
                &fixture.world,
                0,
                fixture.origin,
                fixture.dimensions,
                &t,
                DEFAULT_TRACE_STEPS,
            )
            .unwrap();
            probe_harness.publish(&volume).unwrap();
            let (sample, expected) = expected_pixel(
                &volume,
                &fixture.world,
                &t,
                probe.point,
                probe.normal,
                probe.eye,
                sun,
            );
            let (gap, edge) = clearance(
                &volume,
                &fixture.world,
                probe.point,
                probe.normal,
                probe.eye,
                &sample,
            );
            if gap < MIN_CROSSING_GAP || edge < MIN_HIT_CLEARANCE {
                rejected.push(format!("{}: gap {gap:.6} clearance {edge:.6}", probe.name));
                failures += 1;
                continue;
            }
            let mesh = quad(probe.point, probe.normal, t.color(MIRROR), 0.4);
            let (camera, _eye) = camera_for(probe);
            let lighting = LightingUniform {
                view_proj: Mat4::IDENTITY.to_cols_array_2d(),
                sun: light,
                params: [0., 0., 1., 0.],
                indirect_origin: [0; 4],
                indirect_dimensions: [0; 4],
                reflection_origin: [
                    volume.origin()[0],
                    volume.origin()[1],
                    volume.origin()[2],
                    0,
                ],
                reflection_dimensions: [
                    volume.dimensions()[0],
                    volume.dimensions()[1],
                    volume.dimensions()[2],
                    1,
                ],
                reflection_params: [volume.trace_bound() as f32, SURFACE_OFFSET, 0., 0.],
            };
            let pixel = probe_harness
                .render(&mesh, &camera, &lighting, true)
                .unwrap()[0];
            let error = (0..3)
                .map(|c| (f32::from(pixel[c]) / 255. - expected[c]).abs())
                .fold(0f32, f32::max);
            worst = worst.max(error);
            if sample.hit {
                hits += 1;
            } else {
                misses += 1;
            }
            report.push(format!(
                "probe {:02} {:<44} hit={:<5} material={:<3} cell={:?} error={:.5} gap={:.4} clearance={:.3}",
                index,
                probe.name,
                sample.hit,
                sample.material,
                sample.cell,
                error,
                gap,
                edge
            ));
        }
        let status = if rejected.is_empty() && worst <= PROBE_TOLERANCE && hits + misses >= 24 {
            "PASS"
        } else {
            failures += 1;
            "FAIL"
        };
        println!(
            "probes: {} ({hits} hits, {misses} misses), worst error {worst:.6} ({status})",
            report.len()
        );
        for line in &report {
            println!("  {line}");
        }
        for line in &rejected {
            println!("  REJECTED (edge) {line}");
        }
        cases.push(Case {
            id: "2-predetermined-probes-vs-oracle".into(),
            status,
            detail: format!(
                "{} probes, {hits} hits, {misses} misses, worst {:.6} <= {:.6}, rejected {}",
                report.len(),
                worst,
                PROBE_TOLERANCE,
                rejected.len()
            ),
        });
    }

    // --- Case 3: seeded randomized scenes. ---------------------------------
    {
        let t = table();
        let mut state = 0x5eed_1234_5678_9abcu64;
        let mut next = || {
            state = state
                .wrapping_mul(0x5851_f42d_4c95_7f2d)
                .wrapping_add(0x1405_7b7e_f767_814f);
            (state >> 33) as u32
        };
        let mut checked = 0usize;
        let mut worst = 0.0f32;
        let mut skipped = 0usize;
        let mut worlds = 0usize;
        for _ in 0..64 {
            let mut world = World::new(200 + u64::from(next() % 8));
            for _ in 0..18 {
                let x = next() % 12;
                let y = next() % 8;
                let z = next() % 12;
                let material = [TARGET, STONE, SOIL][(next() % 3) as usize];
                fill(
                    &mut world,
                    [x as i32, y as i32, z as i32],
                    [
                        x as i32 + 1 + (next() % 3) as i32,
                        y as i32 + 1 + (next() % 3) as i32,
                        z as i32 + 1 + (next() % 3) as i32,
                    ],
                    material,
                );
            }
            for x in 0..14 {
                for z in 0..14 {
                    world.set([x, -1, z], MIRROR);
                }
            }
            worlds += 1;
            let volume = ReflectionVolume::pack(
                &world,
                0,
                [-4, -4, -4],
                [20, 16, 20],
                &t,
                DEFAULT_TRACE_STEPS,
            )
            .unwrap();
            probe_harness.publish(&volume).unwrap();
            for _ in 0..8 {
                let point = [(next() % 14) as f32 + 0.5, 0.0, (next() % 14) as f32 + 0.5];
                let eye = [
                    (next() % 20) as f32 - 10.0,
                    (next() % 8) as f32 + 0.6,
                    (next() % 20) as f32 - 10.0,
                ];
                let sample = reflect_sample(&volume, &world, point, [0., 1., 0.], eye);
                let (gap, edge) = clearance(&volume, &world, point, [0., 1., 0.], eye, &sample);
                if gap < MIN_CROSSING_GAP || edge < MIN_HIT_CLEARANCE {
                    skipped += 1;
                    continue;
                }
                let (_, expected) =
                    expected_pixel(&volume, &world, &t, point, [0., 1., 0.], eye, sun);
                let mesh = quad(point, [0., 1., 0.], t.color(MIRROR), 0.4);
                let (camera, _eye) = camera_for(&Probe {
                    name: "random",
                    fixture: 0,
                    point,
                    normal: [0., 1., 0.],
                    eye,
                });
                let lighting = LightingUniform {
                    view_proj: Mat4::IDENTITY.to_cols_array_2d(),
                    sun: light,
                    params: [0., 0., 1., 0.],
                    indirect_origin: [0; 4],
                    indirect_dimensions: [0; 4],
                    reflection_origin: [
                        volume.origin()[0],
                        volume.origin()[1],
                        volume.origin()[2],
                        0,
                    ],
                    reflection_dimensions: [
                        volume.dimensions()[0],
                        volume.dimensions()[1],
                        volume.dimensions()[2],
                        1,
                    ],
                    reflection_params: [volume.trace_bound() as f32, SURFACE_OFFSET, 0., 0.],
                };
                let pixel = probe_harness
                    .render(&mesh, &camera, &lighting, true)
                    .unwrap()[0];
                worst = worst.max(
                    (0..3)
                        .map(|c| (f32::from(pixel[c]) / 255. - expected[c]).abs())
                        .fold(0f32, f32::max),
                );
                checked += 1;
            }
        }
        let status = if checked >= 64 && worst <= PROBE_TOLERANCE {
            "PASS"
        } else {
            failures += 1;
            "FAIL"
        };
        println!("random: {worlds} scenes, {checked} probes, worst error {worst:.6} ({status})");
        cases.push(Case {
            id: "3-seeded-randomized-scenes".into(),
            status,
            detail: format!(
                "{worlds} worlds, {checked} probes, {skipped} skipped by the pre-registered edge filter, worst {worst:.6}"
            ),
        });
    }

    // --- Case 4: recorded images for the seven required responses. ---------
    let image_gpu = match Gpu::new() {
        Ok(gpu) => gpu,
        Err(e) => {
            eprintln!("reflection_validation: second device failed: {e}");
            std::process::exit(1);
        }
    };
    {
        let mut harness = match Harness::new(image_gpu, IMAGE_W, IMAGE_H) {
            Ok(h) => h,
            Err(e) => {
                eprintln!("reflection_validation: image harness failed: {e}");
                std::process::exit(1);
            }
        };
        let t = table();
        let world = wall_scene();
        let origin = [-4, -4, -4];
        let dimensions = [24, 20, 24];
        let mut epoch = 0u64;
        let volume =
            ReflectionVolume::pack(&world, epoch, origin, dimensions, &t, DEFAULT_TRACE_STEPS)
                .unwrap();
        harness.publish(&volume).unwrap();
        let eye = [11.0, 3.0, 0.0];
        let target = [-1.0, 3.0, 0.0];
        let view = Mat4::look_at_rh(Vec3::from_array(eye), Vec3::from_array(target), Vec3::Y);
        let projection = Mat4::perspective_rh(
            35f32.to_radians(),
            IMAGE_W as f32 / IMAGE_H as f32,
            0.1,
            100.0,
        );
        let camera = |view_proj: Mat4, eye: [f32; 3]| Camera {
            view_proj: view_proj.to_cols_array_2d(),
            eye: [eye[0], eye[1], eye[2], 1.0],
        };
        let lighting = |volume: &ReflectionVolume| LightingUniform {
            view_proj: Mat4::IDENTITY.to_cols_array_2d(),
            sun: light,
            params: [0., 0., 1., 0.],
            indirect_origin: [0; 4],
            indirect_dimensions: [0; 4],
            reflection_origin: [
                volume.origin()[0],
                volume.origin()[1],
                volume.origin()[2],
                0,
            ],
            reflection_dimensions: [
                volume.dimensions()[0],
                volume.dimensions()[1],
                volume.dimensions()[2],
                1,
            ],
            reflection_params: [volume.trace_bound() as f32, SURFACE_OFFSET, 0., 0.],
        };
        let mesh = scene_mesh(&world, &t);
        let camera_on = camera(projection * view, eye);
        let light_on = lighting(&volume);
        let off = harness.render(&mesh, &camera_on, &light_on, false).unwrap();
        let on = harness.render(&mesh, &camera_on, &light_on, true).unwrap();
        write_ppm(&out.join("01-reflection-off.ppm"), IMAGE_W, IMAGE_H, &off).unwrap();
        write_ppm(&out.join("02-reflection-on.ppm"), IMAGE_W, IMAGE_H, &on).unwrap();
        let changed = on
            .iter()
            .zip(&off)
            .filter(|(a, b)| a[..3] != b[..3])
            .count();
        println!("image: reflection on/off changes {changed} pixels");

        // Off-screen proof: with reflection disabled, deleting the object must not
        // change one pixel, so any change with reflection enabled is reflected only.
        let mut without = world.clone();
        fill(&mut without, [6, 2, 5], [8, 6, 8], 0);
        epoch += 1;
        let volume_without =
            ReflectionVolume::pack(&without, epoch, origin, dimensions, &t, DEFAULT_TRACE_STEPS)
                .unwrap();
        harness.publish(&volume_without).unwrap();
        let mesh_without = scene_mesh(&without, &t);
        let off_without = harness
            .render(&mesh_without, &camera_on, &lighting(&volume_without), false)
            .unwrap();
        let on_without = harness
            .render(&mesh_without, &camera_on, &lighting(&volume_without), true)
            .unwrap();
        write_ppm(
            &out.join("03-object-removed-off.ppm"),
            IMAGE_W,
            IMAGE_H,
            &off_without,
        )
        .unwrap();
        write_ppm(
            &out.join("04-object-removed-on.ppm"),
            IMAGE_W,
            IMAGE_H,
            &on_without,
        )
        .unwrap();
        let direct_change = off_without
            .iter()
            .zip(&off)
            .filter(|(a, b)| a[..3] != b[..3])
            .count();
        let reflected_change = on_without
            .iter()
            .zip(&on)
            .filter(|(a, b)| a[..3] != b[..3])
            .count();
        println!("image: object off-screen (direct change {direct_change}), reflected change {reflected_change}");

        // Camera movement.
        harness.publish(&volume).unwrap();
        let moved = Mat4::look_at_rh(
            Vec3::from_array([11.0, 6.0, 5.0]),
            Vec3::from_array([-1.0, 2.0, 0.0]),
            Vec3::Y,
        );
        let moved_pixels = harness
            .render(
                &mesh,
                &camera(projection * moved, [11.0, 6.0, 5.0]),
                &lighting(&volume),
                true,
            )
            .unwrap();
        let camera_changed = moved_pixels
            .iter()
            .zip(&on)
            .filter(|(a, b)| a[..3] != b[..3])
            .count();
        write_ppm(
            &out.join("03-camera-moved.ppm"),
            IMAGE_W,
            IMAGE_H,
            &moved_pixels,
        )
        .unwrap();

        // Material/colour change: republish with a recoloured target material.
        let mut recolored = table();
        let mut colors = recolored.colors();
        colors[TARGET as usize] = [0.05, 0.85, 0.30];
        recolored = MaterialTable::new(colors).unwrap();
        recolored.set_mirror(MIRROR, 1.0).unwrap();
        let volume_colored = ReflectionVolume::pack(
            &world,
            epoch,
            origin,
            dimensions,
            &recolored,
            DEFAULT_TRACE_STEPS,
        )
        .unwrap();
        harness.publish(&volume_colored).unwrap();
        let mesh_colored = scene_mesh(&world, &recolored);
        let colored = harness
            .render(
                &mesh_colored,
                &camera(projection * view, eye),
                &lighting(&volume_colored),
                true,
            )
            .unwrap();
        let material_changed = colored
            .iter()
            .zip(&on)
            .filter(|(a, b)| a[..3] != b[..3])
            .count();
        write_ppm(
            &out.join("04-material-change.ppm"),
            IMAGE_W,
            IMAGE_H,
            &colored,
        )
        .unwrap();

        // Occluder added between the mirror and the reflected object.
        let mut occluded = world.clone();
        fill(&mut occluded, [4, 1, 4], [6, 5, 8], STONE);
        epoch += 1;
        let volume_occluded = ReflectionVolume::pack(
            &occluded,
            epoch,
            origin,
            dimensions,
            &t,
            DEFAULT_TRACE_STEPS,
        )
        .unwrap();
        harness.publish(&volume_occluded).unwrap();
        let mesh_occluded = scene_mesh(&occluded, &t);
        let occluder_pixels = harness
            .render(
                &mesh_occluded,
                &camera(projection * view, eye),
                &lighting(&volume_occluded),
                true,
            )
            .unwrap();
        let occluder_changed = occluder_pixels
            .iter()
            .zip(&on)
            .filter(|(a, b)| a[..3] != b[..3])
            .count();
        write_ppm(
            &out.join("05-occluder-added.ppm"),
            IMAGE_W,
            IMAGE_H,
            &occluder_pixels,
        )
        .unwrap();

        // Sun-direction change: the reflection uses the live uniform.
        let sun_moved = Sun {
            direction_to_sun: [-0.6, 0.35, 0.7],
            intensity: 0.8,
        };
        harness.publish(&volume).unwrap();
        let mut lighting_sun = lighting(&volume);
        lighting_sun.sun = normalize_sun(sun_moved);
        let sun_pixels = harness
            .render(&mesh, &camera(projection * view, eye), &lighting_sun, true)
            .unwrap();
        let sun_changed = sun_pixels
            .iter()
            .zip(&on)
            .filter(|(a, b)| a[..3] != b[..3])
            .count();
        write_ppm(&out.join("07-sun-moved.ppm"), IMAGE_W, IMAGE_H, &sun_pixels).unwrap();

        // Scene replacement at an equal revision, then harness recreation.
        let mut replacement = World::new(101);
        fill(&mut replacement, [0, -1, -4], [8, 0, 8], MIRROR);
        fill(&mut replacement, [3, 0, 1], [5, 4, 3], SOIL);
        let equal_revision = replacement.revision() == world.revision();
        let stale_rejected = !volume.valid_for(&replacement, epoch);
        epoch += 1;
        let volume_replaced = ReflectionVolume::pack(
            &replacement,
            epoch,
            origin,
            dimensions,
            &t,
            DEFAULT_TRACE_STEPS,
        )
        .unwrap();
        harness.publish(&volume_replaced).unwrap();
        let mesh_replaced = scene_mesh(&replacement, &t);
        let replaced_pixels = harness
            .render(
                &mesh_replaced,
                &camera(projection * view, eye),
                &lighting(&volume_replaced),
                true,
            )
            .unwrap();
        let replaced_changed = replaced_pixels
            .iter()
            .zip(&on)
            .filter(|(a, b)| a[..3] != b[..3])
            .count();
        write_ppm(
            &out.join("08-scene-replaced.ppm"),
            IMAGE_W,
            IMAGE_H,
            &replaced_pixels,
        )
        .unwrap();

        // Recreate the whole device/harness and re-render the replacement scene.
        let recreated_gpu = Gpu::new().unwrap();
        let mut recreated = Harness::new(recreated_gpu, IMAGE_W, IMAGE_H).unwrap();
        recreated.publish(&volume_replaced).unwrap();
        let recreated_pixels = recreated
            .render(
                &mesh_replaced,
                &camera(projection * view, eye),
                &lighting(&volume_replaced),
                true,
            )
            .unwrap();
        let recreation_drift = recreated_pixels
            .iter()
            .zip(&replaced_pixels)
            .map(|(a, b)| {
                (0..3)
                    .map(|c| (f32::from(a[c]) - f32::from(b[c])).abs())
                    .fold(0f32, f32::max)
            })
            .fold(0f32, f32::max);
        write_ppm(
            &out.join("09-renderer-recreated.ppm"),
            IMAGE_W,
            IMAGE_H,
            &recreated_pixels,
        )
        .unwrap();
        drop(recreated);

        // Oracle-backed response check at one fixed mirror point: the reflected
        // material, hit cell and shaded colour must respond to each change. This
        // isolates the reflection from global relighting effects.
        let probe_point = [1.0f32, 3.0, 4.0];
        let probe_normal = [1.0f32, 0.0, 0.0];
        let base = reflect_sample(&volume, &world, probe_point, probe_normal, eye);
        let base_color = shade_sample(&volume, &base, sun).unwrap();
        let moved_sample =
            reflect_sample(&volume, &world, probe_point, probe_normal, [11.0, 5.0, 2.0]);
        let occluded_sample =
            reflect_sample(&volume_occluded, &occluded, probe_point, probe_normal, eye);
        let removed_sample =
            reflect_sample(&volume_without, &without, probe_point, probe_normal, eye);
        let sun_color = shade_sample(&volume, &base, sun_moved).unwrap();
        let delta =
            |a: [f32; 3], b: [f32; 3]| (0..3).map(|c| (a[c] - b[c]).abs()).fold(0f32, f32::max);
        let responses = [
            (
                "camera movement",
                (moved_sample.cell != base.cell) as usize,
                delta(
                    shade_sample(&volume, &moved_sample, sun).unwrap(),
                    base_color,
                ),
            ),
            (
                "material/color change",
                1,
                delta(
                    shade_sample(
                        &volume_colored,
                        &reflect_sample(&volume_colored, &world, probe_point, probe_normal, eye),
                        sun,
                    )
                    .unwrap(),
                    base_color,
                ),
            ),
            (
                "occluder added",
                usize::from(occluded_sample.material != base.material),
                delta(
                    shade_sample(&volume_occluded, &occluded_sample, sun).unwrap(),
                    base_color,
                ),
            ),
            (
                "reflected object removed",
                usize::from(
                    removed_sample.material != base.material || removed_sample.cell != base.cell,
                ),
                delta(
                    shade_sample(&volume_without, &removed_sample, sun).unwrap(),
                    base_color,
                ),
            ),
            ("sun direction change", 1, delta(sun_color, base_color)),
        ];
        println!(
            "oracle: base hit={} material={} cell={:?}",
            base.hit, base.material, base.cell
        );
        let mut oracle_ok = base.hit && base.material == TARGET;
        for (label, identity, colour) in responses {
            println!("oracle: {label}: identity change {identity}, colour delta {colour:.5}");
            if identity == 0 || colour <= PROBE_TOLERANCE {
                oracle_ok = false;
            }
        }
        if !oracle_ok {
            failures += 1;
        }
        cases.push(Case {
            id: "4b-oracle-response-per-change".into(),
            status: if oracle_ok { "PASS" } else { "FAIL" },
            detail: format!(
                "base material {} cell {:?}; responses verified for camera/material/occluder/removal/sun",
                base.material, base.cell
            ),
        });

        let thresholds = [
            (
                "off-screen object visible only through reflection",
                changed,
                200usize,
            ),
            ("camera movement", camera_changed, 2000),
            ("material/color change", material_changed, 50),
            ("occluder added", occluder_changed, 50),
            ("reflected object removed", reflected_change, 50),
            ("sun direction change", sun_changed, 500),
            ("scene replacement", replaced_changed, 500),
        ];
        let mut image_ok = stale_rejected && recreation_drift == 0.0 && direct_change == 0;
        for (label, count, minimum) in thresholds {
            println!("image: {label}: {count} pixels changed (minimum {minimum})");
            if count < minimum {
                image_ok = false;
            }
        }
        println!("image: equal revision {equal_revision}, stale pack rejected {stale_rejected}, recreation drift {recreation_drift}");
        if !image_ok {
            failures += 1;
        }
        cases.push(Case {
            id: "4-recorded-images-seven-responses".into(),
            status: if image_ok { "PASS" } else { "FAIL" },
            detail: format!(
                "direct-change {direct_change}, off-screen {changed}, camera {camera_changed}, material {material_changed}, occluder {occluder_changed}, removed {reflected_change}, sun {sun_changed}, replacement {replaced_changed}, recreation drift {recreation_drift}"
            ),
        });
        // The reported replacement equality is informational; the digest rejects
        // a replaced scene at any revision, so it does not gate the case.
        let _ = (
            equal_revision,
            footprint_digest(&world, origin, dimensions),
            volume.source_digest(),
        );
    }

    // --- Case 5: delayed result cannot publish. ----------------------------
    {
        let t = table();
        let mut edited = World::new(301);
        edited.set([0, 0, 0], TARGET);
        let prepared =
            ReflectionVolume::pack(&edited, 5, [-4; 3], [8; 3], &t, DEFAULT_TRACE_STEPS).unwrap();
        let revision = edited.revision();
        edited.set([0, 0, 0], 0); // state B
        edited.set([0, 0, 0], TARGET); // state A again, at a later revision
        assert!(edited.revision() > revision);
        let round_trip = !prepared.valid_for(&edited, 5);
        // A different scene at the *same* revision, seed and epoch: only the
        // footprint digest can reject it.
        let mut replacement = World::new(301);
        replacement.set([3, 3, 3], STONE);
        let equal = replacement.revision() == revision;
        let replacement_rejected = !prepared.valid_for(&replacement, 5);
        let ok = round_trip && replacement_rejected && equal;
        if !ok {
            failures += 1;
        }
        println!(
            "delayed: A->B->A rejected {round_trip}, equal-revision replacement rejected {replacement_rejected}, equal={equal}" 
        );
        cases.push(Case {
            id: "5-delayed-result-cannot-publish".into(),
            status: if ok { "PASS" } else { "FAIL" },
            detail: format!(
                "A->B->A rejected {round_trip}, equal-revision replacement rejected {replacement_rejected}"
            ),
        });
    }

    // --- Case 6: trace-budget and volume bounds are enforced. --------------
    {
        let t = table();
        let world = World::new(401);
        let too_big =
            ReflectionVolume::pack(&world, 0, [0; 3], [MAX_REFLECTION_AXIS + 1, 1, 1], &t, 8);
        let too_many_steps = ReflectionVolume::pack(&world, 0, [0; 3], [4; 3], &t, 513);
        let biggest = ReflectionVolume::pack(&world, 0, [-32; 3], [64; 3], &t, 512).unwrap();
        let ok = too_big.is_err()
            && too_many_steps.is_err()
            && biggest.materials().len() == MAX_REFLECTION_CELLS
            && biggest.trace_bound() == 193;
        if !ok {
            failures += 1;
        }
        println!(
            "bounds: 64^3 cap {} MiB, trace bound {}",
            biggest.memory_stats().material_bytes / (1024 * 1024),
            biggest.trace_bound()
        );
        cases.push(Case {
            id: "6-volume-and-budget-bounds".into(),
            status: if ok { "PASS" } else { "FAIL" },
            detail: format!(
                "rejects axis>{} and steps>512; 64^3 = {} cells, bound {}",
                MAX_REFLECTION_AXIS,
                MAX_REFLECTION_CELLS,
                biggest.trace_bound()
            ),
        });
    }

    let elapsed = started.elapsed();
    let manifest = render_manifest(&cases, elapsed.as_secs_f64(), &probe_harness);
    if let Err(e) = std::fs::write(out.join("manifest.json"), manifest) {
        eprintln!("reflection_validation: manifest write failed: {e}");
        failures += 1;
    }
    if let Err(e) = std::fs::write(out.join("probes.txt"), report.join("\n")) {
        eprintln!("reflection_validation: probe report write failed: {e}");
        failures += 1;
    }
    println!(
        "reflection_validation: {} in {:.1}s",
        if failures == 0 { "PASS" } else { "FAIL" },
        elapsed.as_secs_f64()
    );
    std::process::exit(if failures == 0 { 0 } else { 1 });
}

/// Mirror-wall fixture used for the recorded images: the reflected object sits
/// outside the camera frustum and is reachable only through the reflection.
fn wall_scene() -> World {
    let mut world = World::new(101);
    // Mirror wall at x = 0, exposing its +X face at x = 1.
    fill(&mut world, [0, 0, -4], [1, 8, 8], MIRROR);
    // Stone floor and side walls for context; both are inside the source volume.
    fill(&mut world, [1, -1, -4], [12, 0, 8], STONE);
    fill(&mut world, [1, 0, -4], [12, 6, -3], STONE);
    fill(&mut world, [1, 0, 8], [12, 6, 9], STONE);
    // Reflected object: far to the camera's side, outside a 35-degree frustum
    // aimed at the mirror, but reachable by the mirrored view ray.
    fill(&mut world, [6, 2, 5], [8, 6, 8], TARGET);
    world
}

fn fixtures_index(fixtures: &[Fixture], name: &str) -> usize {
    fixtures.iter().position(|f| f.name == name).unwrap()
}

/// Orthographic camera whose single-pixel centre ray passes exactly through the
/// probe point, derived analytically rather than from an unprojection.
fn camera_for(probe: &Probe) -> (Camera, [f32; 3]) {
    let point = Vec3::from_array(probe.point);
    let eye = Vec3::from_array(probe.eye);
    let forward = (point - eye).normalize();
    let up = if forward.y.abs() > 0.9 {
        Vec3::Z
    } else {
        Vec3::Y
    };
    let view = Mat4::look_at_rh(eye, eye + forward, up);
    let projection = Mat4::orthographic_rh(-0.25, 0.25, -0.25, 0.25, 0.1, 200.0);
    (
        Camera {
            view_proj: (projection * view).to_cols_array_2d(),
            eye: [eye.x, eye.y, eye.z, 1.0],
        },
        probe.eye,
    )
}

fn render_manifest(cases: &[Case], seconds: f64, harness: &Harness) -> String {
    let mut out = String::from("{\n  \"artifacts\": {\n");
    out.push_str("    \"images\": \"PPM, sRGB-encoded for viewing, linear R8G8B8A8 source\",\n");
    out.push_str(&format!(
        "    \"extents\": \"probe 1x1, scenes {}x{}\",\n",
        IMAGE_W, IMAGE_H
    ));
    out.push_str(&format!("    \"elapsed_seconds\": {seconds:.3}\n"));
    out.push_str("  },\n  \"cases\": [\n");
    for (index, case) in cases.iter().enumerate() {
        out.push_str(&format!(
            "    {{\"id\": \"{}\", \"status\": \"{}\", \"detail\": \"{}\"}}{}\n",
            case.id,
            case.status,
            case.detail.replace('"', "'"),
            if index + 1 == cases.len() { "" } else { "," }
        ));
    }
    out.push_str("  ],\n  \"not_run\": [\n");
    // Device gates are owned by the coordinator; recorded here so the manifest
    // covers every acceptance case rather than only the ones that were executed.
    for (index, case) in NOT_RUN.iter().enumerate() {
        out.push_str(&format!(
            "    {{\"id\": \"{}\", \"status\": \"NOT RUN\", \"detail\": \"{}\"}}{}\n",
            case.0,
            case.1,
            if index + 1 == NOT_RUN.len() { "" } else { "," }
        ));
    }
    out.push_str("  ]\n}\n");
    out
}

/// Acceptance cases that require the owner-reserved Android device.
const NOT_RUN: &[(&str, &str)] = &[
    (
        "10-device-frame-cost-off-on",
        "120 warmup + 1000 measured frames per mode at matched resolution",
    ),
    (
        "10-device-gpu-timings",
        "GPU interval timings if the device exposes timestamp queries",
    ),
    ("10-device-thermal", "thermal conditions during measurement"),
    (
        "11-device-captures",
        "edit, camera motion and HOME/resume captures on the reserved device",
    ),
];
