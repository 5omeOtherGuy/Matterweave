//! Headless native Vulkan functional comparison of the optional packed-occupancy
//! (BlockMask) traversal candidate for issue 42.
//!
//! Renders one pixel per probe through the real Naga-compiled
//! `ray_hierarchy_gpu.wgsl` SPIR-V (five set-0 bindings: camera, materials, palette,
//! occupancy words, hierarchy parameters), reads color and depth back and compares them
//! against the CPU reference lineage on the same geometry.
//!
//! The CPU side of the comparison is the `matterweave-ray-hierarchy` crate:
//! `TraversalMode::Reference` and `TraversalMode::BlockMask` are asserted equal on every
//! probe, and `Reference` is the retained lineage of `ray_reference.wgsl` /
//! `World::raycast`. The geometry fed to the CPU walk is the shader's own unprojection,
//! mirrored in `f32` ([`shader_ray`]), so a difference between the two walks is a
//! traversal-arithmetic difference (`f64` host, `f32` kernel), not a camera difference.
//! A separate check bounds the mirrored ray against the analytic probe ray.
//!
//! Not a performance measurement, not a renderer path, no Android execution. See
//! [the log](../../../docs/performance/logs/ray-hierarchy-gpu.md).
//!
//! ```sh
//! VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json \
//!   cargo run -p matterweave-ray-hierarchy --example ray_hierarchy_gpu
//! ```
//!
//! Exit codes: 0 all checks passed, 1 at least one check failed, 2 no usable Vulkan 1.1
//! graphics device (checks NOT RUN).

use ash::vk;
use glam::{Mat4, Vec3, Vec4};
use matterweave_core::{RayHit, World};
use matterweave_ray_hierarchy::{clip_depth, fixtures, BlockShape, HierarchyVolume, TraversalMode};
use matterweave_render::ray_hierarchy_gpu::{HierarchyUpload, FRAGMENT_SPIRV, VERTEX_SPIRV};
use matterweave_render::ray_reference::RayVolume;
use matterweave_render::Sun;
use std::io::Cursor;

const NEAR: f32 = 0.25;
const FAR: f32 = 40.0;
const EXTENT: u32 = 1;
/// Source pack epoch; the staleness checks move this deliberately.
const EPOCH: u64 = 1;
/// Per-channel palette-space bound for a material match: about five 8-bit steps.
/// Comparing in palette space divides out the lighting and fog attenuation that would
/// otherwise let a wrong material hide inside a color tolerance in dim light.
const PALETTE_TOLERANCE: f32 = 0.02;
/// Below this palette-to-pixel scale a material cannot be verified from the readback;
/// the check fails closed instead of accepting an unverifiable match.
const MIN_PALETTE_SCALE: f32 = 0.05;
/// Depth `z/w` is computed on both sides with the same `f32` formula from the same
/// mirrored ray, so it must agree to arithmetic noise. At the corpus's farthest hits a
/// one-cell distance error is at least `2e-4` in depth, far outside this bound; the
/// observed maximum is printed.
const DEPTH_TOLERANCE: f32 = 5e-5;
/// The mirrored `f32` unprojection must stay this close to the analytic probe ray.
const RAY_TOLERANCE: f64 = 1e-4;
/// Distance bound for a hit point to lie on a cell plane; `f32`-scale, not a hit budget.
const PLANE_TOLERANCE: f64 = 1e-5;
/// Segment length bound for the mirrored near/far ray. The far point sits `FAR` world
/// units out and an `f32` inverse projection moves it by ~2e-3 there (2.5e-4 relative).
const SEGMENT_TOLERANCE: f64 = 1e-2;
/// Seeded corpus size, matching the CPU experiment's generator: 48 fixtures x 8 rays.
const SEEDED_FIXTURES: u64 = 48;
const SEEDED_RAYS: usize = 8;

fn err(e: vk::Result) -> String {
    format!("{e:?}")
}

/// Documented golden lighting model from `world.wgsl`, identical to the retained
/// reference example's mirror.
fn shade(material_rgb: [f32; 3], normal: Vec3, view_distance: f32, sun: [f32; 4]) -> [f32; 3] {
    let sunlight = normal.dot(Vec3::new(sun[0], sun[1], sun[2])).max(0.0);
    let ambient = 0.28 + 0.12 * normal.y.max(0.0);
    let fog = 1.0 - (-view_distance * 0.013).exp();
    let lit = Vec3::from_array(material_rgb) * (ambient + sunlight * sun[3]);
    lit.lerp(Vec3::new(0.16, 0.24, 0.29), fog).to_array()
}

/// Camera whose centre pixel ray is exactly (`origin`, `direction`): the eye sits one
/// `NEAR` behind the probe origin.
fn camera(origin: Vec3, direction: Vec3, orthographic: bool) -> (Mat4, Vec3) {
    let forward = direction.normalize();
    let eye = origin - forward * NEAR;
    let up = if forward.y.abs() > 0.9 {
        Vec3::Z
    } else {
        Vec3::Y
    };
    let view = Mat4::look_at_rh(eye, eye + forward, up);
    let projection = if orthographic {
        Mat4::orthographic_rh(-0.5, 0.5, -0.5, 0.5, NEAR, FAR)
    } else {
        Mat4::perspective_rh(1.0, 1.0, NEAR, FAR)
    };
    (projection * view, eye)
}

/// Mirror of the shader's center-pixel unprojection, including the grid-plane
/// stabilization loop, in `f32`.
///
/// Returns the world-space near point, the `f32` unit direction and the clipped segment
/// length, all widened to `f64` for the CPU walk. `None` when the shader would discard
/// (`w == 0` or a zero-length segment).
fn shader_ray(inverse_view_projection: &Mat4) -> Option<([f64; 3], [f64; 3], f64)> {
    let near_h = *inverse_view_projection * Vec4::new(0.0, 0.0, 0.0, 1.0);
    let far_h = *inverse_view_projection * Vec4::new(0.0, 0.0, 1.0, 1.0);
    if near_h.w == 0.0 || far_h.w == 0.0 {
        return None;
    }
    let mut origin = near_h.truncate() / near_h.w;
    let mut far_point = far_h.truncate() / far_h.w;
    for axis in 0..3 {
        let plane = origin[axis].round();
        // 2^-20, the shader's exact eight-relative-ULP stabilization tolerance.
        let tolerance = 2.0f32.powi(-20) * plane.abs().max(1.0);
        if (origin[axis] - plane).abs() <= tolerance && (far_point[axis] - plane).abs() <= tolerance
        {
            origin[axis] = plane;
            far_point[axis] = plane;
        }
    }
    let segment = far_point - origin;
    let length = segment.length();
    if length == 0.0 {
        return None;
    }
    let direction = segment / length;
    Some((
        origin.to_array().map(f64::from),
        direction.to_array().map(f64::from),
        f64::from(length),
    ))
}

/// The shader's `world = ray_origin + direction * distance` in `f32`.
fn mirrored_point(origin: [f64; 3], direction: [f64; 3], distance: f64) -> Vec3 {
    let origin = Vec3::from_array(origin.map(|value| value as f32));
    let direction = Vec3::from_array(direction.map(|value| value as f32));
    origin + direction * distance as f32
}

struct Probe {
    name: String,
    origin: [f64; 3],
    /// Raw direction; normalized once, like the retained example.
    direction: [f64; 3],
}

fn probe(name: &str, origin: [f64; 3], direction: [f64; 3]) -> Probe {
    Probe {
        name: name.into(),
        origin,
        direction,
    }
}

/// A fixture, its block shapes and its probes.
struct Case {
    name: &'static str,
    world: World,
    origin: [i32; 3],
    dimensions: [u32; 3],
    shapes: Vec<BlockShape>,
    probes: Vec<Probe>,
    /// Render both projection modes for every probe.
    both_projections: bool,
}

fn solid(world: &mut World, cells: &[[i32; 3]], material: u8) {
    for cell in cells {
        assert!(world.set(*cell, material), "fixture write {cell:?}");
    }
}

/// Targeted cases: the CPU experiment's pinned geometry classes plus the crop, edit and
/// zero-length disclosures this native comparison adds.
fn targeted_cases() -> Vec<Case> {
    let wall: Vec<([i32; 3], u8)> = (0..4)
        .flat_map(|y| (0..4).filter_map(move |z| ((y, z) != (1, 1)).then_some(([3, y, z], 11))))
        .chain(std::iter::once(([5, 1, 1], 12)))
        .collect();
    let mut tied = World::new(17);
    solid(&mut tied, &[[-2, -2, -2]], 2);
    solid(&mut tied, &[[-2, -1, -2]], 3);
    let mut axes = World::new(18);
    for (cell, material) in [([0, 1, 1], 5), ([1, 1, 1], 6), ([2, 1, 1], 7)] {
        assert!(axes.set(cell, material));
    }
    let mut inside = World::new(19);
    solid(&mut inside, &[[0, 0, 0]], 8);
    let mut walled = World::new(20);
    for &(cell, material) in &wall {
        assert!(walled.set(cell, material));
    }
    let mut negative = World::new(21);
    solid(
        &mut negative,
        &[[-17, -1, -1], [-16, -1, -1], [-17, 0, -1], [-16, 0, 0]],
        1,
    );
    let mut crop = World::new(22);
    solid(&mut crop, &[[2, 0, 0]], 5);
    solid(&mut crop, &[[-2, 0, 0]], 77);
    vec![
        Case {
            name: "tied-diagonal-corners",
            world: tied,
            origin: [-2, -2, -2],
            dimensions: [4, 4, 4],
            shapes: vec![BlockShape::CUBE4, BlockShape::TALL_4_4_8],
            probes: vec![
                probe(
                    "diagonal into corner cell",
                    [-2.5, -2.5, -2.5],
                    [1.0, 1.0, 1.0],
                ),
                probe("interior diagonal", [-1.5, -1.5, -1.5], [1.0, 1.0, 1.0]),
                probe("edge tie x-z", [-1.5, -2.5, -1.5], [1.0, 1.0, 1.0]),
                probe("negative diagonal", [-2.5, -1.5, -1.5], [-1.0, 1.0, 1.0]),
            ],
            both_projections: true,
        },
        Case {
            name: "zero-direction-components",
            world: axes,
            origin: [-2, -2, -2],
            dimensions: [6, 6, 6],
            shapes: vec![BlockShape::TALL_4_4_8],
            probes: vec![
                probe("+x row", [-1.5, 1.5, 1.5], [1.0, 0.0, 0.0]),
                probe("-x row", [-1.5, 1.5, 1.5], [-1.0, 0.0, 0.0]),
                probe("+y row", [-1.5, 1.5, 1.5], [0.0, 1.0, 0.0]),
                probe("+z row", [-1.5, 1.5, 1.5], [0.0, 0.0, 1.0]),
                probe("x with 0.25 z", [-1.5, 1.5, 1.5], [1.0, 0.0, 0.25]),
                probe("x with 0.5 y", [-1.5, 1.5, 1.5], [1.0, 0.5, 0.0]),
            ],
            both_projections: false,
        },
        Case {
            name: "inside-solid-starts",
            world: inside,
            origin: [0, 0, 0],
            dimensions: [4, 4, 4],
            shapes: vec![BlockShape::TALL_4_4_8],
            probes: vec![
                probe("inside solid +x", [0.25, 0.25, 0.75], [1.0, 0.0, 0.0]),
                probe("inside solid diagonal", [0.5, 0.5, 0.5], [1.0, 1.0, 1.0]),
                probe("inside solid -x", [3.5, 0.5, 0.5], [-1.0, 0.0, 0.0]),
            ],
            both_projections: true,
        },
        Case {
            name: "thin-wall-opening",
            world: walled,
            origin: [0, 0, 0],
            dimensions: [8, 4, 4],
            shapes: vec![BlockShape::TALL_4_4_8, BlockShape::CUBE8],
            probes: vec![
                probe("into wall", [0.5, 0.5, 0.5], [1.0, 0.0, 0.0]),
                probe("through opening", [0.5, 1.5, 1.5], [1.0, 0.0, 0.0]),
                probe("through opening slanted", [0.5, 1.5, 1.5], [1.0, 0.25, 0.0]),
                probe("wall diagonal", [0.5, 2.5, 0.5], [1.0, 0.0, 0.5]),
            ],
            both_projections: false,
        },
        Case {
            name: "negative-crop-edges",
            world: negative,
            origin: [-17, -1, -1],
            dimensions: [2, 2, 2],
            shapes: vec![BlockShape::CUBE4],
            probes: vec![
                probe("negative +x", [-16.5, -0.5, -0.5], [1.0, 0.0, 0.0]),
                probe("negative -x", [-16.5, -0.5, -0.5], [-1.0, 0.0, 0.0]),
                probe("negative +y", [-16.5, -0.5, -0.5], [0.0, 1.0, 0.0]),
                probe("negative +z", [-16.5, -0.5, -0.5], [0.0, 0.0, 1.0]),
                probe("negative diagonal", [-16.5, -0.5, -0.5], [1.0, 1.0, 1.0]),
            ],
            both_projections: true,
        },
        Case {
            name: "crop-exclusion",
            world: crop,
            origin: [0, 0, 0],
            dimensions: [4, 4, 4],
            shapes: vec![BlockShape::TALL_4_4_8],
            probes: vec![
                // The out-of-crop occluder at [-2, 0, 0] is hit by `World::raycast`; the
                // crop path and the kernel must both ignore it and hit [2, 0, 0].
                probe("foreground excluded", [-4.5, 0.5, 0.5], [1.0, 0.0, 0.0]),
            ],
            both_projections: false,
        },
    ]
}

/// Rendered readback of one probe.
struct Rendered {
    rgba: [f32; 4],
    depth: f32,
}

impl Rendered {
    #[cfg(test)]
    fn background() -> Self {
        Self {
            rgba: [0.0, 0.0, 0.0, 0.0],
            depth: 1.0,
        }
    }
    fn is_background(&self) -> bool {
        self.rgba[3] <= 0.5
    }
}

/// What the classifier proved about one readback.
#[derive(Debug)]
enum Verdict {
    /// Color and depth match the CPU expectation.
    Exact,
    /// The readback is a tied-plane resolution of the CPU hit, proven geometrically.
    Tie(String),
    /// Both sides miss.
    Miss,
}

/// The CPU expectation for one probe, on the mirrored shader ray.
struct Expectation {
    hit: Option<RayHit>,
    origin: [f64; 3],
    direction: [f64; 3],
    view_projection: Mat4,
    eye: Vec3,
    sun: [f32; 4],
}

/// Palette-space distance between a readback and one candidate material and face.
///
/// `shade` maps a palette color to the observed pixel through one scalar light factor
/// and one fog blend; dividing the per-channel difference by `(1 - fog) * light` removes
/// the attenuation that would otherwise let a wrong material hide inside a color
/// tolerance in dim light. `None` when that scale is too small to verify a material, so
/// the candidate fails closed instead of being accepted.
fn palette_error(
    observed: &Rendered,
    material: u8,
    normal: [i32; 3],
    world: Vec3,
    eye: Vec3,
    sun: [f32; 4],
) -> Option<f32> {
    let normal = Vec3::new(normal[0] as f32, normal[1] as f32, normal[2] as f32);
    let sunlight = normal.dot(Vec3::new(sun[0], sun[1], sun[2])).max(0.0);
    let light = 0.28 + 0.12 * normal.y.max(0.0) + sunlight * sun[3];
    let distance = (world - eye).length();
    let fog = 1.0 - (-distance * 0.013).exp();
    let scale = light * (1.0 - fog);
    if !scale.is_finite() || scale <= MIN_PALETTE_SCALE {
        return None;
    }
    let expected = shade(
        fixtures::palette()[usize::from(material)],
        normal,
        distance,
        sun,
    );
    Some(
        (0..3)
            .map(|channel| (observed.rgba[channel] - expected[channel]).abs())
            .fold(0.0f32, f32::max)
            / scale,
    )
}

/// Strict classification of one rendered probe against the CPU reference hit.
///
/// An `Exact` verdict requires the projected depth (the shared `f32` formula) to agree
/// and the material to be verifiable in palette space (`PALETTE_TOLERANCE`). Anything
/// else is accepted only as a proven tie resolution: the depth must still match, and
/// there must exist a candidate cell and entry face such that
///
/// - the candidate cell is inside the crop with a nonzero material;
/// - every axis where it differs from the CPU cell differs by exactly one step and the
///   hit point lies on the plane the two cells share (`PLANE_TOLERANCE`, `f32`-scale);
/// - the candidate's entry normal names a face of the candidate cell that contains the
///   hit point;
/// - the candidate's palette-space error is within `PALETTE_TOLERANCE`.
///
/// No other difference is accepted, and there is no numeric mismatch budget.
fn classify(
    volume: &HierarchyVolume,
    expected: &Expectation,
    observed: &Rendered,
) -> Result<Verdict, String> {
    let Some(hit) = expected.hit else {
        return if observed.is_background() {
            Ok(Verdict::Miss)
        } else {
            Err(format!(
                "kernel hit where the CPU reference missed: rgba {:?} depth {}",
                observed.rgba, observed.depth
            ))
        };
    };
    let point = [
        expected.origin[0] + expected.direction[0] * f64::from(hit.distance),
        expected.origin[1] + expected.direction[1] * f64::from(hit.distance),
        expected.origin[2] + expected.direction[2] * f64::from(hit.distance),
    ];
    let world = mirrored_point(expected.origin, expected.direction, f64::from(hit.distance));
    let expected_depth = clip_depth(
        &expected.view_projection.to_cols_array_2d(),
        expected.origin,
        expected.direction,
        f64::from(hit.distance),
    )
    .ok_or("CPU hit has no finite projected depth")?;
    let depth_delta = (observed.depth - expected_depth).abs();
    if observed.is_background() {
        return Err(format!(
            "kernel missed where the CPU reference hit {hit:?} (expected depth {expected_depth})"
        ));
    }
    let expected_error = palette_error(
        observed,
        hit.material,
        hit.normal,
        world,
        expected.eye,
        expected.sun,
    )
    .ok_or_else(|| {
        format!("CPU hit {hit:?} has a light scale below {MIN_PALETTE_SCALE}, so its material cannot be verified")
    })?;
    if depth_delta <= DEPTH_TOLERANCE && expected_error <= PALETTE_TOLERANCE {
        return Ok(Verdict::Exact);
    }
    if depth_delta > DEPTH_TOLERANCE {
        return Err(format!(
            "depth {} differs from the CPU hit {hit:?} at {expected_depth} by {depth_delta} \
             (tolerance {DEPTH_TOLERANCE})",
            observed.depth
        ));
    }
    let tie_tolerance = PLANE_TOLERANCE * f64::max(f64::from(hit.distance), 1.0);
    for dx in -1i32..=1 {
        for dy in -1i32..=1 {
            for dz in -1i32..=1 {
                let candidate = [hit.cell[0] + dx, hit.cell[1] + dy, hit.cell[2] + dz];
                let offsets = [dx, dy, dz];
                let Some(material) = volume.material_at(candidate) else {
                    continue;
                };
                if material == 0 {
                    continue;
                }
                let Ok(material) = u8::try_from(material) else {
                    continue;
                };
                let mut proved = true;
                for axis in 0..3 {
                    if offsets[axis] == 0 {
                        continue;
                    }
                    let face = f64::from(candidate[axis].max(hit.cell[axis]));
                    if (point[axis] - face).abs() > tie_tolerance {
                        proved = false;
                    }
                }
                if !proved {
                    continue;
                }
                // The candidate's entry face must contain the hit point: lower face for
                // a negative normal, upper face for a positive one.
                for axis in 0..3 {
                    for sign in [-1.0f32, 1.0] {
                        let cell_lo = f64::from(candidate[axis]);
                        let face = if sign < 0.0 { cell_lo } else { cell_lo + 1.0 };
                        if (point[axis] - face).abs() > tie_tolerance {
                            continue;
                        }
                        let mut normal = [0i32; 3];
                        normal[axis] = sign as i32;
                        let Some(candidate_error) = palette_error(
                            observed,
                            material,
                            normal,
                            world,
                            expected.eye,
                            expected.sun,
                        ) else {
                            continue;
                        };
                        if candidate_error <= PALETTE_TOLERANCE {
                            return Ok(Verdict::Tie(format!(
                                "CPU {hit:?} -> kernel cell {candidate:?} material {material} \
                                 normal {normal:?} at the same point (offsets {offsets:?}, \
                                 palette error {candidate_error})"
                            )));
                        }
                    }
                }
            }
        }
    }
    Err(format!(
        "readback rgba {:?} depth {} matches neither the CPU hit {hit:?} (palette error \
         {expected_error}) nor a proven tie resolution",
        observed.rgba, observed.depth
    ))
}

/// Bounded counters and failure list for the whole run.
#[derive(Default)]
struct Report {
    checks: usize,
    hits: usize,
    misses: usize,
    exact: usize,
    ties: Vec<String>,
    failures: Vec<String>,
    max_palette_error: f32,
    max_observed_depth_delta: f32,
    upload_bytes: usize,
}

impl Report {
    fn fail(&mut self, message: String) {
        self.failures.push(message);
    }
}

struct Gpu {
    _entry: ash::Entry,
    device_name: String,
    instance: ash::Instance,
    device: ash::Device,
    queue: vk::Queue,
    memory: vk::PhysicalDeviceMemoryProperties,
    pool: vk::CommandPool,
}

impl Gpu {
    fn new() -> Result<Self, String> {
        let entry = unsafe { ash::Entry::load() }.map_err(|e| format!("Vulkan loader: {e}"))?;
        let app = vk::ApplicationInfo::default()
            .application_name(c"matterweave-ray-hierarchy-gpu")
            .api_version(vk::API_VERSION_1_1);
        // SAFETY: the static application name outlives the instance.
        let instance = unsafe {
            entry.create_instance(
                &vk::InstanceCreateInfo::default().application_info(&app),
                None,
            )
        }
        .map_err(|e| format!("no Vulkan 1.1 instance: {}", err(e)))?;
        // SAFETY: queried handles belong to the live instance.
        let (physical, family) = unsafe {
            let mut chosen = None;
            for device in instance.enumerate_physical_devices().map_err(err)? {
                if instance.get_physical_device_properties(device).api_version < vk::API_VERSION_1_1
                {
                    continue;
                }
                for (index, queue) in instance
                    .get_physical_device_queue_family_properties(device)
                    .iter()
                    .enumerate()
                {
                    if queue.queue_flags.contains(vk::QueueFlags::GRAPHICS) {
                        chosen = Some((device, index as u32));
                        break;
                    }
                }
                if chosen.is_some() {
                    break;
                }
            }
            chosen.ok_or("no Vulkan 1.1 graphics device".to_string())?
        };
        let priorities = [1.0];
        let queues = [vk::DeviceQueueCreateInfo::default()
            .queue_family_index(family)
            .queue_priorities(&priorities)];
        // SAFETY: the queue family exists; no extensions or features are requested.
        let (device, queue, memory) = unsafe {
            let device = instance
                .create_device(
                    physical,
                    &vk::DeviceCreateInfo::default().queue_create_infos(&queues),
                    None,
                )
                .map_err(err)?;
            let memory = instance.get_physical_device_memory_properties(physical);
            let queue = device.get_device_queue(family, 0);
            (device, queue, memory)
        };
        // SAFETY: the family index was validated above.
        let pool = unsafe {
            device.create_command_pool(
                &vk::CommandPoolCreateInfo::default()
                    .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER)
                    .queue_family_index(family),
                None,
            )
        }
        .map_err(err)?;
        let device_name = {
            let props = unsafe { instance.get_physical_device_properties(physical) };
            let name = props
                .device_name
                .iter()
                .take_while(|&&c| c != 0)
                .map(|&c| c as u8 as char)
                .collect::<String>();
            let major = vk::api_version_major(props.api_version);
            let minor = vk::api_version_minor(props.api_version);
            format!("{name} (Vulkan {major}.{minor})")
        };
        Ok(Self {
            _entry: entry,
            device_name,
            instance,
            device,
            queue,
            memory,
            pool,
        })
    }

    fn memory_type(&self, bits: u32, flags: vk::MemoryPropertyFlags) -> Result<u32, String> {
        self.memory
            .memory_types
            .iter()
            .zip(0u32..)
            .find(|(t, bit)| (bits & (1 << bit)) != 0 && t.property_flags.contains(flags))
            .map(|(_, bit)| bit)
            .ok_or_else(|| format!("no memory type with {flags:?}"))
    }
}

impl Drop for Gpu {
    fn drop(&mut self) {
        // SAFETY: owned handles; the device is destroyed before the instance.
        unsafe {
            self.device.destroy_command_pool(self.pool, None);
            self.device.destroy_device(None);
            self.instance.destroy_instance(None);
        }
    }
}

struct Buffer {
    handle: vk::Buffer,
    memory: vk::DeviceMemory,
    size: u64,
}

struct Attachment {
    image: vk::Image,
    memory: vk::DeviceMemory,
    view: vk::ImageView,
}

/// One pipeline plus its five set-0 descriptor resources for one uploaded crop.
struct Session {
    camera: Buffer,
    materials: Buffer,
    palette: Buffer,
    occupancy: Buffer,
    hierarchy: Buffer,
    color_read: Buffer,
    depth_read: Buffer,
    color: Attachment,
    depth: Attachment,
    layout: vk::PipelineLayout,
    set_layout: vk::DescriptorSetLayout,
    descriptor_pool: vk::DescriptorPool,
    set: vk::DescriptorSet,
    pipeline: vk::Pipeline,
    pass: vk::RenderPass,
    framebuffer: vk::Framebuffer,
    buffer: vk::CommandBuffer,
}

impl Session {
    fn new(gpu: &Gpu, pack: &RayVolume, upload: &HierarchyUpload<'_>) -> Result<Self, String> {
        // SAFETY: every handle is owned here and freed in `destroy`.
        unsafe {
            let device = &gpu.device;
            let camera = Self::buffer(gpu, 192, vk::BufferUsageFlags::UNIFORM_BUFFER, None)?;
            let material_bytes: Vec<u8> = upload
                .materials()
                .iter()
                .flat_map(|word| word.to_le_bytes())
                .collect();
            let materials = Self::buffer(
                gpu,
                material_bytes.len().max(4),
                vk::BufferUsageFlags::STORAGE_BUFFER,
                Some(&material_bytes),
            )?;
            let palette_bytes: Vec<u8> = pack
                .palette()
                .iter()
                .flat_map(|color| color.iter().flat_map(|value| value.to_le_bytes()))
                .collect();
            let palette = Self::buffer(
                gpu,
                palette_bytes.len(),
                vk::BufferUsageFlags::STORAGE_BUFFER,
                Some(&palette_bytes),
            )?;
            let occupancy_bytes: Vec<u8> = upload
                .occupancy()
                .iter()
                .flat_map(|word| word.to_le_bytes())
                .collect();
            let occupancy = Self::buffer(
                gpu,
                occupancy_bytes.len().max(4),
                vk::BufferUsageFlags::STORAGE_BUFFER,
                Some(&occupancy_bytes),
            )?;
            let hierarchy_uniform = upload.uniform();
            let hierarchy = Self::buffer(
                gpu,
                48,
                vk::BufferUsageFlags::UNIFORM_BUFFER,
                Some(bytemuck::bytes_of(&hierarchy_uniform)),
            )?;
            let color_read = Self::buffer(gpu, 4, vk::BufferUsageFlags::TRANSFER_DST, None)?;
            let depth_read = Self::buffer(gpu, 4, vk::BufferUsageFlags::TRANSFER_DST, None)?;

            let bindings = [
                vk::DescriptorSetLayoutBinding::default()
                    .binding(matterweave_render::ray_hierarchy_gpu::CAMERA_BINDING)
                    .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
                    .descriptor_count(1)
                    .stage_flags(vk::ShaderStageFlags::FRAGMENT),
                vk::DescriptorSetLayoutBinding::default()
                    .binding(matterweave_render::ray_hierarchy_gpu::MATERIAL_BINDING)
                    .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                    .descriptor_count(1)
                    .stage_flags(vk::ShaderStageFlags::FRAGMENT),
                vk::DescriptorSetLayoutBinding::default()
                    .binding(matterweave_render::ray_hierarchy_gpu::PALETTE_BINDING)
                    .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                    .descriptor_count(1)
                    .stage_flags(vk::ShaderStageFlags::FRAGMENT),
                vk::DescriptorSetLayoutBinding::default()
                    .binding(matterweave_render::ray_hierarchy_gpu::OCCUPANCY_BINDING)
                    .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                    .descriptor_count(1)
                    .stage_flags(vk::ShaderStageFlags::FRAGMENT),
                vk::DescriptorSetLayoutBinding::default()
                    .binding(matterweave_render::ray_hierarchy_gpu::HIERARCHY_BINDING)
                    .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
                    .descriptor_count(1)
                    .stage_flags(vk::ShaderStageFlags::FRAGMENT),
            ];
            let set_layout = device
                .create_descriptor_set_layout(
                    &vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings),
                    None,
                )
                .map_err(err)?;
            let layouts = [set_layout];
            let layout = device
                .create_pipeline_layout(
                    &vk::PipelineLayoutCreateInfo::default().set_layouts(&layouts),
                    None,
                )
                .map_err(err)?;
            let pool_sizes = [
                vk::DescriptorPoolSize::default()
                    .ty(vk::DescriptorType::UNIFORM_BUFFER)
                    .descriptor_count(2),
                vk::DescriptorPoolSize::default()
                    .ty(vk::DescriptorType::STORAGE_BUFFER)
                    .descriptor_count(3),
            ];
            let descriptor_pool = device
                .create_descriptor_pool(
                    &vk::DescriptorPoolCreateInfo::default()
                        .pool_sizes(&pool_sizes)
                        .max_sets(1),
                    None,
                )
                .map_err(err)?;
            let set = device
                .allocate_descriptor_sets(
                    &vk::DescriptorSetAllocateInfo::default()
                        .descriptor_pool(descriptor_pool)
                        .set_layouts(&layouts),
                )
                .map_err(err)?[0];
            let camera_info = [vk::DescriptorBufferInfo::default()
                .buffer(camera.handle)
                .range(192)];
            let material_info = [vk::DescriptorBufferInfo::default()
                .buffer(materials.handle)
                .range(materials.size)];
            let palette_info = [vk::DescriptorBufferInfo::default()
                .buffer(palette.handle)
                .range(palette.size)];
            let occupancy_info = [vk::DescriptorBufferInfo::default()
                .buffer(occupancy.handle)
                .range(occupancy.size)];
            let hierarchy_info = [vk::DescriptorBufferInfo::default()
                .buffer(hierarchy.handle)
                .range(48)];
            device.update_descriptor_sets(
                &[
                    vk::WriteDescriptorSet::default()
                        .dst_set(set)
                        .dst_binding(matterweave_render::ray_hierarchy_gpu::CAMERA_BINDING)
                        .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
                        .buffer_info(&camera_info),
                    vk::WriteDescriptorSet::default()
                        .dst_set(set)
                        .dst_binding(matterweave_render::ray_hierarchy_gpu::MATERIAL_BINDING)
                        .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                        .buffer_info(&material_info),
                    vk::WriteDescriptorSet::default()
                        .dst_set(set)
                        .dst_binding(matterweave_render::ray_hierarchy_gpu::PALETTE_BINDING)
                        .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                        .buffer_info(&palette_info),
                    vk::WriteDescriptorSet::default()
                        .dst_set(set)
                        .dst_binding(matterweave_render::ray_hierarchy_gpu::OCCUPANCY_BINDING)
                        .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                        .buffer_info(&occupancy_info),
                    vk::WriteDescriptorSet::default()
                        .dst_set(set)
                        .dst_binding(matterweave_render::ray_hierarchy_gpu::HIERARCHY_BINDING)
                        .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
                        .buffer_info(&hierarchy_info),
                ],
                &[],
            );

            let vertex =
                ash::util::read_spv(&mut Cursor::new(VERTEX_SPIRV)).map_err(|e| e.to_string())?;
            let fragment =
                ash::util::read_spv(&mut Cursor::new(FRAGMENT_SPIRV)).map_err(|e| e.to_string())?;
            let vertex_module = device
                .create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&vertex), None)
                .map_err(err)?;
            let fragment_module = device
                .create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&fragment), None)
                .map_err(err)?;
            let stages = [
                vk::PipelineShaderStageCreateInfo::default()
                    .stage(vk::ShaderStageFlags::VERTEX)
                    .module(vertex_module)
                    .name(c"vs_main"),
                vk::PipelineShaderStageCreateInfo::default()
                    .stage(vk::ShaderStageFlags::FRAGMENT)
                    .module(fragment_module)
                    .name(c"fs_main"),
            ];

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
                    .store_op(vk::AttachmentStoreOp::STORE)
                    .initial_layout(vk::ImageLayout::UNDEFINED)
                    .final_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL),
            ];
            let color_refs = [vk::AttachmentReference::default()
                .attachment(0)
                .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)];
            let depth_ref = vk::AttachmentReference::default()
                .attachment(1)
                .layout(vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL);
            let subpasses = [vk::SubpassDescription::default()
                .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
                .color_attachments(&color_refs)
                .depth_stencil_attachment(&depth_ref)];
            // Make attachment writes and final layout transitions visible to the
            // following image-to-buffer copies, for both color and depth.
            let dependencies = [vk::SubpassDependency::default()
                .src_subpass(0)
                .dst_subpass(vk::SUBPASS_EXTERNAL)
                .src_stage_mask(
                    vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT
                        | vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS
                        | vk::PipelineStageFlags::LATE_FRAGMENT_TESTS,
                )
                .src_access_mask(
                    vk::AccessFlags::COLOR_ATTACHMENT_WRITE
                        | vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE,
                )
                .dst_stage_mask(vk::PipelineStageFlags::TRANSFER)
                .dst_access_mask(vk::AccessFlags::TRANSFER_READ)];
            let pass = device
                .create_render_pass(
                    &vk::RenderPassCreateInfo::default()
                        .attachments(&attachments)
                        .subpasses(&subpasses)
                        .dependencies(&dependencies),
                    None,
                )
                .map_err(err)?;
            let color =
                Self::attachment(gpu, vk::Format::R8G8B8A8_UNORM, vk::ImageAspectFlags::COLOR)?;
            let depth = Self::attachment(gpu, vk::Format::D32_SFLOAT, vk::ImageAspectFlags::DEPTH)?;
            let views = [color.view, depth.view];
            let framebuffer = device
                .create_framebuffer(
                    &vk::FramebufferCreateInfo::default()
                        .render_pass(pass)
                        .attachments(&views)
                        .width(EXTENT)
                        .height(EXTENT)
                        .layers(1),
                    None,
                )
                .map_err(err)?;

            let viewports = [vk::Viewport {
                x: 0.0,
                y: 0.0,
                width: EXTENT as f32,
                height: EXTENT as f32,
                min_depth: 0.0,
                max_depth: 1.0,
            }];
            let scissors = [vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: vk::Extent2D {
                    width: EXTENT,
                    height: EXTENT,
                },
            }];
            let blend = [vk::PipelineColorBlendAttachmentState::default()
                .color_write_mask(vk::ColorComponentFlags::RGBA)
                .blend_enable(false)];
            let vertex_input = vk::PipelineVertexInputStateCreateInfo::default();
            let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
                .topology(vk::PrimitiveTopology::TRIANGLE_LIST);
            let viewport_state = vk::PipelineViewportStateCreateInfo::default()
                .viewports(&viewports)
                .scissors(&scissors);
            let rasterization = vk::PipelineRasterizationStateCreateInfo::default()
                .polygon_mode(vk::PolygonMode::FILL)
                .cull_mode(vk::CullModeFlags::NONE)
                .line_width(1.0);
            let multisample = vk::PipelineMultisampleStateCreateInfo::default()
                .rasterization_samples(vk::SampleCountFlags::TYPE_1);
            let depth_stencil = vk::PipelineDepthStencilStateCreateInfo::default()
                .depth_test_enable(true)
                .depth_write_enable(true)
                .depth_compare_op(vk::CompareOp::LESS);
            let color_blend = vk::PipelineColorBlendStateCreateInfo::default().attachments(&blend);
            let info = [vk::GraphicsPipelineCreateInfo::default()
                .stages(&stages)
                .vertex_input_state(&vertex_input)
                .input_assembly_state(&input_assembly)
                .viewport_state(&viewport_state)
                .rasterization_state(&rasterization)
                .multisample_state(&multisample)
                .depth_stencil_state(&depth_stencil)
                .color_blend_state(&color_blend)
                .layout(layout)
                .render_pass(pass)
                .subpass(0)];
            let pipeline = device
                .create_graphics_pipelines(vk::PipelineCache::null(), &info, None)
                .map_err(|(_, e)| err(e))?[0];
            device.destroy_shader_module(vertex_module, None);
            device.destroy_shader_module(fragment_module, None);
            let buffer = device
                .allocate_command_buffers(
                    &vk::CommandBufferAllocateInfo::default()
                        .command_pool(gpu.pool)
                        .level(vk::CommandBufferLevel::PRIMARY)
                        .command_buffer_count(1),
                )
                .map_err(err)?[0];
            Ok(Self {
                camera,
                materials,
                palette,
                occupancy,
                hierarchy,
                color_read,
                depth_read,
                color,
                depth,
                layout,
                set_layout,
                descriptor_pool,
                set,
                pipeline,
                pass,
                framebuffer,
                buffer,
            })
        }
    }

    // SAFETY: the caller guarantees the device is idle when handles are destroyed.
    unsafe fn buffer(
        gpu: &Gpu,
        size: usize,
        usage: vk::BufferUsageFlags,
        initial: Option<&[u8]>,
    ) -> Result<Buffer, String> {
        let size = size as u64;
        let handle = gpu
            .device
            .create_buffer(
                &vk::BufferCreateInfo::default()
                    .size(size)
                    .usage(usage)
                    .sharing_mode(vk::SharingMode::EXCLUSIVE),
                None,
            )
            .map_err(err)?;
        let requirements = gpu.device.get_buffer_memory_requirements(handle);
        let memory = {
            let index = gpu.memory_type(
                requirements.memory_type_bits,
                vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
            )?;
            gpu.device
                .allocate_memory(
                    &vk::MemoryAllocateInfo::default()
                        .allocation_size(requirements.size)
                        .memory_type_index(index),
                    None,
                )
                .map_err(err)?
        };
        gpu.device
            .bind_buffer_memory(handle, memory, 0)
            .map_err(err)?;
        if let Some(bytes) = initial {
            let pointer = gpu
                .device
                .map_memory(memory, 0, size, vk::MemoryMapFlags::empty())
                .map_err(err)?;
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), pointer.cast(), bytes.len());
            gpu.device.unmap_memory(memory);
        }
        Ok(Buffer {
            handle,
            memory,
            size,
        })
    }

    // SAFETY: the caller owns the created handles and frees them in `destroy`.
    unsafe fn attachment(
        gpu: &Gpu,
        format: vk::Format,
        aspect: vk::ImageAspectFlags,
    ) -> Result<Attachment, String> {
        let image = gpu
            .device
            .create_image(
                &vk::ImageCreateInfo::default()
                    .image_type(vk::ImageType::TYPE_2D)
                    .format(format)
                    .extent(vk::Extent3D {
                        width: EXTENT,
                        height: EXTENT,
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
                        } | vk::ImageUsageFlags::TRANSFER_SRC,
                    )
                    .sharing_mode(vk::SharingMode::EXCLUSIVE),
                None,
            )
            .map_err(err)?;
        let requirements = gpu.device.get_image_memory_requirements(image);
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
            .bind_image_memory(image, memory, 0)
            .map_err(err)?;
        let view = gpu
            .device
            .create_image_view(
                &vk::ImageViewCreateInfo::default()
                    .image(image)
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
        Ok(Attachment {
            image,
            memory,
            view,
        })
    }

    /// Renders one pixel with `uniform` and reads back (rgba, depth).
    ///
    /// SAFETY: `self` owns the resources; the queue is waited idle before returning.
    unsafe fn render(&self, gpu: &Gpu, uniform: &[u8; 192]) -> Result<Rendered, String> {
        let device = &gpu.device;
        let pointer = device
            .map_memory(self.camera.memory, 0, 192, vk::MemoryMapFlags::empty())
            .map_err(err)?;
        std::ptr::copy_nonoverlapping(uniform.as_ptr(), pointer.cast(), 192);
        device.unmap_memory(self.camera.memory);
        device
            .reset_command_buffer(self.buffer, vk::CommandBufferResetFlags::empty())
            .map_err(err)?;
        device
            .begin_command_buffer(
                self.buffer,
                &vk::CommandBufferBeginInfo::default()
                    .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            )
            .map_err(err)?;
        let clears = [
            vk::ClearValue {
                color: vk::ClearColorValue {
                    float32: [0.0, 0.0, 0.0, 0.0],
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
            offset: vk::Offset2D { x: 0, y: 0 },
            extent: vk::Extent2D {
                width: EXTENT,
                height: EXTENT,
            },
        };
        device.cmd_begin_render_pass(
            self.buffer,
            &vk::RenderPassBeginInfo::default()
                .render_pass(self.pass)
                .framebuffer(self.framebuffer)
                .render_area(area)
                .clear_values(&clears),
            vk::SubpassContents::INLINE,
        );
        device.cmd_bind_pipeline(self.buffer, vk::PipelineBindPoint::GRAPHICS, self.pipeline);
        device.cmd_bind_descriptor_sets(
            self.buffer,
            vk::PipelineBindPoint::GRAPHICS,
            self.layout,
            0,
            &[self.set],
            &[],
        );
        device.cmd_draw(self.buffer, 3, 1, 0, 0);
        device.cmd_end_render_pass(self.buffer);
        let extent = vk::Extent3D {
            width: EXTENT,
            height: EXTENT,
            depth: 1,
        };
        let color_copy = [vk::BufferImageCopy::default()
            .image_subresource(
                vk::ImageSubresourceLayers::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .layer_count(1),
            )
            .image_extent(extent)];
        device.cmd_copy_image_to_buffer(
            self.buffer,
            self.color.image,
            vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
            self.color_read.handle,
            &color_copy,
        );
        let depth_copy = [vk::BufferImageCopy::default()
            .image_subresource(
                vk::ImageSubresourceLayers::default()
                    .aspect_mask(vk::ImageAspectFlags::DEPTH)
                    .layer_count(1),
            )
            .image_extent(extent)];
        device.cmd_copy_image_to_buffer(
            self.buffer,
            self.depth.image,
            vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
            self.depth_read.handle,
            &depth_copy,
        );
        // Readback becomes host-visible before the queue-idle completion wait.
        let readback = [vk::MemoryBarrier::default()
            .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
            .dst_access_mask(vk::AccessFlags::HOST_READ)];
        device.cmd_pipeline_barrier(
            self.buffer,
            vk::PipelineStageFlags::TRANSFER,
            vk::PipelineStageFlags::HOST,
            vk::DependencyFlags::empty(),
            &readback,
            &[],
            &[],
        );
        device.end_command_buffer(self.buffer).map_err(err)?;
        let buffers = [self.buffer];
        let submits = [vk::SubmitInfo::default().command_buffers(&buffers)];
        device
            .queue_submit(gpu.queue, &submits, vk::Fence::null())
            .map_err(err)?;
        device.device_wait_idle().map_err(err)?;
        let rgba = {
            let p = device
                .map_memory(self.color_read.memory, 0, 4, vk::MemoryMapFlags::empty())
                .map_err(err)?
                .cast::<[u8; 4]>();
            let value = p.read();
            device.unmap_memory(self.color_read.memory);
            [
                value[0] as f32 / 255.0,
                value[1] as f32 / 255.0,
                value[2] as f32 / 255.0,
                value[3] as f32 / 255.0,
            ]
        };
        let depth = {
            let p = device
                .map_memory(self.depth_read.memory, 0, 4, vk::MemoryMapFlags::empty())
                .map_err(err)?
                .cast::<f32>();
            let value = p.read();
            device.unmap_memory(self.depth_read.memory);
            value
        };
        Ok(Rendered { rgba, depth })
    }

    /// SAFETY: the device is idle and no command buffer references these handles.
    unsafe fn destroy(&mut self, gpu: &Gpu) {
        let device = &gpu.device;
        device.free_command_buffers(gpu.pool, &[self.buffer]);
        device.destroy_framebuffer(self.framebuffer, None);
        for buffer in [
            &self.camera,
            &self.materials,
            &self.palette,
            &self.occupancy,
            &self.hierarchy,
            &self.color_read,
            &self.depth_read,
        ] {
            device.destroy_buffer(buffer.handle, None);
            device.free_memory(buffer.memory, None);
        }
        for attachment in [&self.color, &self.depth] {
            device.destroy_image_view(attachment.view, None);
            device.destroy_image(attachment.image, None);
            device.free_memory(attachment.memory, None);
        }
        device.destroy_pipeline(self.pipeline, None);
        device.destroy_render_pass(self.pass, None);
        device.destroy_descriptor_pool(self.descriptor_pool, None);
        device.destroy_pipeline_layout(self.layout, None);
        device.destroy_descriptor_set_layout(self.set_layout, None);
    }
}

/// One live session and its CPU counterpart, ready to check probes.
struct ProbeScene<'a> {
    gpu: &'a Gpu,
    session: &'a Session,
    volume: &'a HierarchyVolume,
    pack: &'a RayVolume,
}

impl ProbeScene<'_> {
    /// Runs one probe through the kernel and classifies the readback.
    fn check(&self, report: &mut Report, label: &str, probe: &Probe, orthographic: bool) {
        let (gpu, session, volume, pack) = (self.gpu, self.session, self.volume, self.pack);
        report.checks += 1;
        let context = format!("{label} [{}]", probe.name);
        let direction = fixtures::oracle_direction(probe.direction);
        let (view_projection, eye) = camera(
            Vec3::from_array(probe.origin.map(|value| value as f32)),
            Vec3::from_array(direction.map(|value| value as f32)),
            orthographic,
        );
        let sun = Sun::default();
        let uniform = match pack.uniform(view_projection, eye, sun) {
            Ok(uniform) => uniform,
            Err(message) => {
                report.fail(format!("{context}: uniform: {message}"));
                return;
            }
        };
        let uniform_bytes: [u8; 192] = match bytemuck::bytes_of(&uniform).try_into() {
            Ok(bytes) => bytes,
            Err(_) => {
                report.fail(format!("{context}: uniform size"));
                return;
            }
        };
        let inverse = Mat4::from_cols_array_2d(&uniform.inverse_view_projection);
        let Some((origin, shader_direction, length)) = shader_ray(&inverse) else {
            report.fail(format!("{context}: shader ray is degenerate"));
            return;
        };
        // Camera contract: the mirrored unprojection must stay close to the analytic probe.
        let origin_delta = (0..3)
            .map(|axis| (origin[axis] - probe.origin[axis]).abs())
            .fold(0.0f64, f64::max);
        let direction_delta = (0..3)
            .map(|axis| (shader_direction[axis] - direction[axis]).abs())
            .fold(0.0f64, f64::max);
        let length_delta = (length - f64::from(FAR - NEAR)).abs();
        if origin_delta > RAY_TOLERANCE
            || direction_delta > RAY_TOLERANCE
            || length_delta > SEGMENT_TOLERANCE
        {
            report.fail(format!(
                "{context}: mirrored ray deviates (origin {origin_delta}, direction \
                 {direction_delta}, length {length_delta})"
            ));
            return;
        }
        // The CPU reference lineage and the CPU BlockMask candidate must be identical on
        // the exact geometry the kernel sees; the comparison is then kernel-vs-reference.
        let reference =
            match volume.trace(TraversalMode::Reference, origin, shader_direction, length) {
                Ok(hit) => hit,
                Err(message) => {
                    report.fail(format!("{context}: CPU reference walk: {message}"));
                    return;
                }
            };
        let candidate =
            match volume.trace(TraversalMode::BlockMask, origin, shader_direction, length) {
                Ok(hit) => hit,
                Err(message) => {
                    report.fail(format!("{context}: CPU BlockMask walk: {message}"));
                    return;
                }
            };
        if let Err(message) = same_hit(reference, candidate) {
            report.fail(format!("{context}: CPU Reference vs BlockMask: {message}"));
            return;
        }
        let rendered = match unsafe { session.render(gpu, &uniform_bytes) } {
            Ok(rendered) => rendered,
            Err(message) => {
                report.fail(format!("{context}: render: {message}"));
                return;
            }
        };
        let sun_key = [
            uniform.sun[0],
            uniform.sun[1],
            uniform.sun[2],
            uniform.sun[3],
        ];
        let expectation = Expectation {
            hit: candidate,
            origin,
            direction: shader_direction,
            view_projection,
            eye,
            sun: sun_key,
        };
        match classify(volume, &expectation, &rendered) {
            Ok(Verdict::Miss) => report.misses += 1,
            Ok(Verdict::Exact) => {
                report.hits += 1;
                report.exact += 1;
            }
            Ok(Verdict::Tie(proof)) => {
                report.hits += 1;
                report.ties.push(format!("{context}: {proof}"));
            }
            Err(message) => report.fail(format!("{context}: {message}")),
        }
        if let Some(hit) = candidate {
            if let Some(expected_depth) = clip_depth(
                &view_projection.to_cols_array_2d(),
                origin,
                shader_direction,
                f64::from(hit.distance),
            ) {
                report.max_observed_depth_delta = report
                    .max_observed_depth_delta
                    .max((rendered.depth - expected_depth).abs());
            }
            let world = mirrored_point(origin, shader_direction, f64::from(hit.distance));
            if let Some(error) =
                palette_error(&rendered, hit.material, hit.normal, world, eye, sun_key)
            {
                report.max_palette_error = report.max_palette_error.max(error);
            }
        }
    }
}

/// Compare two CPU hits bit-for-bit on cell, material, normal and distance.
fn same_hit(left: Option<RayHit>, right: Option<RayHit>) -> Result<(), String> {
    match (left, right) {
        (None, None) => Ok(()),
        (Some(left), Some(right))
            if left.cell == right.cell
                && left.material == right.material
                && left.normal == right.normal
                && left.distance.to_bits() == right.distance.to_bits() =>
        {
            Ok(())
        }
        _ => Err(format!("left {left:?} right {right:?}")),
    }
}

/// Runs one case across its block shapes and probes.
fn run_case(gpu: &Gpu, report: &mut Report, case: &Case) {
    let world = &case.world;
    let pack = match RayVolume::pack(
        world,
        EPOCH,
        case.origin,
        case.dimensions,
        fixtures::palette(),
    ) {
        Ok(pack) => pack,
        Err(message) => {
            report.fail(format!("{}: pack rejected: {message}", case.name));
            return;
        }
    };
    for shape in &case.shapes {
        let volume = match HierarchyVolume::from_reference(&pack, *shape) {
            Ok(volume) => volume,
            Err(message) => {
                report.fail(format!("{}: snapshot rejected: {message}", case.name));
                continue;
            }
        };
        let upload = match HierarchyUpload::from_parts(
            &pack,
            volume.materials(),
            volume.occupancy().words(),
            shape.dims(),
        ) {
            Ok(upload) => upload,
            Err(message) => {
                report.fail(format!("{}: upload rejected: {message}", case.name));
                continue;
            }
        };
        report.upload_bytes = report.upload_bytes.max(upload.memory_stats().upload_bytes);
        if !upload.is_current(&pack)
            || !matterweave_render::ray_hierarchy_gpu::is_fresh(&upload, &pack, world, EPOCH)
        {
            report.fail(format!("{}: fresh upload reported stale", case.name));
            continue;
        }
        let mut session = match Session::new(gpu, &pack, &upload) {
            Ok(session) => session,
            Err(message) => {
                report.fail(format!("{}: pipeline unavailable: {message}", case.name));
                continue;
            }
        };
        let label = format!("{}/{:?}", case.name, shape.dims());
        let scene = ProbeScene {
            gpu,
            session: &session,
            volume: &volume,
            pack: &pack,
        };
        let hits_before = report.hits;
        let misses_before = report.misses;
        for probe in &case.probes {
            let projections: &[bool] = if case.both_projections {
                &[true, false]
            } else {
                &[false]
            };
            for &orthographic in projections {
                scene.check(report, &label, probe, orthographic);
            }
        }
        println!(
            "case {label}: {} hits, {} misses",
            report.hits - hits_before,
            report.misses - misses_before
        );
        // SAFETY: the queue was waited idle after every render.
        unsafe { session.destroy(gpu) };
    }
}

/// The four start classes of the CPU experiment, two rays each per fixture.
fn run_seeded(gpu: &Gpu, report: &mut Report) {
    let hits_before = report.hits;
    let misses_before = report.misses;
    for seed in 0..SEEDED_FIXTURES {
        let fixture = fixtures::sparse_fixture(seed);
        let pack = match RayVolume::pack(
            &fixture.world,
            EPOCH,
            fixture.origin,
            fixture.dimensions,
            fixtures::palette(),
        ) {
            Ok(pack) => pack,
            Err(message) => {
                report.fail(format!("seed {seed}: pack rejected: {message}"));
                continue;
            }
        };
        let shape = BlockShape::TALL_4_4_8;
        let volume = match HierarchyVolume::from_reference(&pack, shape) {
            Ok(volume) => volume,
            Err(message) => {
                report.fail(format!("seed {seed}: snapshot rejected: {message}"));
                continue;
            }
        };
        let upload = match HierarchyUpload::from_parts(
            &pack,
            volume.materials(),
            volume.occupancy().words(),
            shape.dims(),
        ) {
            Ok(upload) => upload,
            Err(message) => {
                report.fail(format!("seed {seed}: upload rejected: {message}"));
                continue;
            }
        };
        let mut session = match Session::new(gpu, &pack, &upload) {
            Ok(session) => session,
            Err(message) => {
                report.fail(format!("seed {seed}: pipeline unavailable: {message}"));
                continue;
            }
        };
        let scene = ProbeScene {
            gpu,
            session: &session,
            volume: &volume,
            pack: &pack,
        };
        let mut rng = fixtures::Rng::new(0xE0 + seed);
        for index in 0..SEEDED_RAYS {
            let start = match index % 4 {
                0 => fixtures::StartKind::Center,
                1 => fixtures::StartKind::Integer,
                2 => fixtures::StartKind::Quarter,
                _ => fixtures::StartKind::Outside,
            };
            let (origin, raw, _) = fixtures::random_ray(&mut rng, &fixture, start);
            let probe = probe(&format!("seed {seed} ray {index} {start:?}"), origin, raw);
            scene.check(report, "seeded", &probe, false);
        }
        // SAFETY: the queue was waited idle after every render.
        unsafe { session.destroy(gpu) };
    }
    println!(
        "seeded corpus: {} hits, {} misses over {SEEDED_FIXTURES} fixtures x {SEEDED_RAYS} rays",
        report.hits - hits_before,
        report.misses - misses_before
    );
}

/// One crop and block shape, so the edit check stays within clippy's argument budget.
struct Crop {
    origin: [i32; 3],
    dimensions: [u32; 3],
    shape: BlockShape,
}

/// Derives a fresh snapshot from `world`, runs one probe and reports it.
///
/// Returns the pack, snapshot and CPU hit so the caller can assert staleness and the
/// edited answer.
fn render_once(
    gpu: &Gpu,
    report: &mut Report,
    world: &World,
    label: &str,
    crop: &Crop,
    ray: &Probe,
) -> Result<(RayVolume, HierarchyVolume, Option<RayHit>), String> {
    let pack = RayVolume::pack(
        world,
        EPOCH,
        crop.origin,
        crop.dimensions,
        fixtures::palette(),
    )?;
    let volume = HierarchyVolume::from_reference(&pack, crop.shape)
        .map_err(|message| message.to_string())?;
    let upload = HierarchyUpload::from_parts(
        &pack,
        volume.materials(),
        volume.occupancy().words(),
        crop.shape.dims(),
    )?;
    if !upload.is_current(&pack)
        || !matterweave_render::ray_hierarchy_gpu::is_fresh(&upload, &pack, world, EPOCH)
    {
        return Err("fresh upload reported stale".into());
    }
    let mut session = Session::new(gpu, &pack, &upload)?;
    let scene = ProbeScene {
        gpu,
        session: &session,
        volume: &volume,
        pack: &pack,
    };
    scene.check(report, label, ray, false);
    // SAFETY: the queue was waited idle after the render.
    unsafe { session.destroy(gpu) };
    let direction = fixtures::oracle_direction(ray.direction);
    let hit = volume
        .trace(
            TraversalMode::BlockMask,
            ray.origin,
            direction,
            f64::from(FAR - NEAR),
        )
        .map_err(|message| message.to_string())?;
    Ok((pack, volume, hit))
}

/// Edits: staleness must be detected, and re-deriving must feed the new bytes.
fn run_edit_check(gpu: &Gpu, report: &mut Report) {
    let mut world = World::new(99);
    solid(&mut world, &[[2, 0, 0]], 41);
    let crop = Crop {
        origin: [0, 0, 0],
        dimensions: [4, 4, 4],
        shape: BlockShape::TALL_4_4_8,
    };
    let ray = probe("edit probe", [0.5, 0.5, 0.5], [1.0, 0.0, 0.0]);
    let (pack, volume, hit) = match render_once(gpu, report, &world, "edits/initial", &crop, &ray) {
        Ok(value) => value,
        Err(message) => {
            report.fail(format!("edits: initial: {message}"));
            return;
        }
    };
    report.checks += 1;
    match hit {
        Some(hit) if hit.material == 41 => report.hits += 1,
        other => report.fail(format!(
            "edits: initial CPU hit {other:?} is not material 41"
        )),
    }
    // Editing the authoritative world invalidates the derived pack and the upload.
    report.checks += 1;
    if !world.set([2, 0, 0], 0) {
        report.fail("edits: clear write was rejected".into());
        return;
    }
    if pack.valid_for(&world, EPOCH) {
        report.fail("edits: pack still valid after the world edit".into());
    }
    match HierarchyUpload::from_parts(
        &pack,
        volume.materials(),
        volume.occupancy().words(),
        crop.shape.dims(),
    ) {
        Ok(upload) => {
            if matterweave_render::ray_hierarchy_gpu::is_fresh(&upload, &pack, &world, EPOCH) {
                report.fail("edits: upload reported fresh on an edited world".into());
            }
        }
        Err(message) => report.fail(format!("edits: stale upload rejected: {message}")),
    }
    match render_once(gpu, report, &world, "edits/cleared", &crop, &ray) {
        Ok((_, _, cleared)) => {
            report.checks += 1;
            if cleared.is_some() {
                report.fail(format!("edits: cleared CPU hit {cleared:?} is not a miss"));
            }
        }
        Err(message) => report.fail(format!("edits: cleared: {message}")),
    }
    if !world.set([2, 0, 0], 42) {
        report.fail("edits: replacement write was rejected".into());
        return;
    }
    match render_once(gpu, report, &world, "edits/replaced", &crop, &ray) {
        Ok((_, _, replaced)) => {
            report.checks += 1;
            match replaced {
                Some(hit) if hit.material == 42 => report.hits += 1,
                other => report.fail(format!("edits: replaced CPU hit {other:?} is not 42")),
            }
        }
        Err(message) => report.fail(format!("edits: replaced: {message}")),
    }
}

/// Crop exclusion, pinned on the native side too: the authoritative CPU oracle hits the
/// foreground occluder outside the crop, while the crop path and the kernel must ignore
/// it and hit the in-crop cell behind it.
fn documented_crop_exclusion() -> Result<(), String> {
    let mut world = World::new(22);
    solid(&mut world, &[[2, 0, 0]], 5);
    solid(&mut world, &[[-2, 0, 0]], 77);
    let pack = RayVolume::pack(&world, EPOCH, [0, 0, 0], [4, 4, 4], fixtures::palette())?;
    let volume = HierarchyVolume::from_reference(&pack, BlockShape::TALL_4_4_8)
        .map_err(|message| message.to_string())?;
    let oracle = world.raycast([-4.5, 0.5, 0.5], [1.0, 0.0, 0.0], 12.0);
    match oracle {
        Some(hit) if hit.cell == [-2, 0, 0] && hit.material == 77 => {}
        other => {
            return Err(format!(
                "oracle did not hit the out-of-crop occluder: {other:?}"
            ))
        }
    }
    let crop = volume
        .trace(
            TraversalMode::Reference,
            [-4.5, 0.5, 0.5],
            [1.0, 0.0, 0.0],
            12.0,
        )
        .map_err(|message| message.to_string())?;
    match crop {
        Some(hit) if hit.cell == [2, 0, 0] && hit.material == 5 => Ok(()),
        other => Err(format!("crop path did not exclude the occluder: {other:?}")),
    }
}

/// The CPU `max_distance == 0` inside-solid query is oracle-defined and has no fragment
/// equivalent: the kernel walks the near/far segment only and the reference discards a
/// zero-length segment. This check pins the CPU side and states the difference.
fn documented_zero_length_query() -> Result<(), String> {
    let mut world = World::new(5);
    solid(&mut world, &[[0, 0, 0]], 9);
    let pack = RayVolume::pack(&world, EPOCH, [0, 0, 0], [2, 2, 2], fixtures::palette())?;
    let volume = HierarchyVolume::from_reference(&pack, BlockShape::TALL_4_4_8)
        .map_err(|message| message.to_string())?;
    let hit = volume
        .trace(
            TraversalMode::BlockMask,
            [0.5, 0.5, 0.5],
            [1.0, 0.0, 0.0],
            0.0,
        )
        .map_err(|message| message.to_string())?;
    match hit {
        Some(hit) if hit.cell == [0, 0, 0] && hit.distance == 0.0 && hit.normal == [0; 3] => Ok(()),
        other => Err(format!("zero-length query answered {other:?}")),
    }
}

fn main() {
    let mut report = Report::default();
    let gpu = match Gpu::new() {
        Ok(gpu) => gpu,
        // SAFETY: no Vulkan device exists; nothing was created.
        Err(message) => {
            println!("NOT RUN: {message}");
            println!(
                "ray_hierarchy_gpu: no native Vulkan 1.1 graphics device on this host; \
                 Android execution is a separate gate"
            );
            std::process::exit(2);
        }
    };
    println!("device: {}", gpu.device_name);
    for case in targeted_cases() {
        run_case(&gpu, &mut report, &case);
    }
    run_seeded(&gpu, &mut report);
    run_edit_check(&gpu, &mut report);
    report.checks += 1;
    if let Err(message) = documented_zero_length_query() {
        report.fail(format!("zero-length query: {message}"));
    } else {
        println!(
            "documented: the CPU max_distance == 0 inside-solid query answers the origin \
             cell at distance 0; the fragment walks the near/far segment only and cannot \
             express it (the retained reference discards zero-length segments)"
        );
    }
    report.checks += 1;
    if let Err(message) = documented_crop_exclusion() {
        report.fail(format!("crop exclusion: {message}"));
    } else {
        println!(
            "documented: the CPU oracle hits the out-of-crop occluder [-2, 0, 0]; the \
             crop path and the kernel ignore it and hit the in-crop cell [2, 0, 0]"
        );
    }
    println!(
        "probes: {} total, {} hits ({} exact, {} proven tie resolutions), {} misses",
        report.checks,
        report.hits,
        report.exact,
        report.ties.len(),
        report.misses
    );
    for tie in &report.ties {
        println!("  tie: {tie}");
    }
    println!(
        "max observed depth delta {:e} (tolerance {DEPTH_TOLERANCE:e}), max exact palette \
         error {:e} (tolerance {PALETTE_TOLERANCE:e}), max upload bytes {}",
        report.max_observed_depth_delta, report.max_palette_error, report.upload_bytes
    );
    for failure in &report.failures {
        println!("FAIL {failure}");
    }
    println!(
        "ray_hierarchy_gpu: {} checks, {} failures",
        report.checks,
        report.failures.len()
    );
    if !report.failures.is_empty() {
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(pack: &RayVolume, shape: BlockShape) -> HierarchyVolume {
        HierarchyVolume::from_reference(pack, shape).expect("snapshot")
    }

    fn expectation(
        hit: RayHit,
        origin: [f64; 3],
        direction: [f64; 3],
        view_projection: Mat4,
        eye: Vec3,
    ) -> Expectation {
        Expectation {
            hit: Some(hit),
            origin,
            direction,
            view_projection,
            eye,
            sun: [0.4, 0.85, 0.3, 0.8],
        }
    }

    fn observed_from(hit: &RayHit, normal: [i32; 3], material: u8, e: &Expectation) -> Rendered {
        let world = mirrored_point(e.origin, e.direction, f64::from(hit.distance));
        let color = shade(
            fixtures::palette()[usize::from(material)],
            Vec3::new(normal[0] as f32, normal[1] as f32, normal[2] as f32),
            (world - e.eye).length(),
            e.sun,
        );
        let depth = clip_depth(
            &e.view_projection.to_cols_array_2d(),
            e.origin,
            e.direction,
            f64::from(hit.distance),
        )
        .unwrap();
        Rendered {
            rgba: [color[0], color[1], color[2], 1.0],
            depth,
        }
    }

    /// A corner tie: the point (1, 1, 0.25) is the shared corner of the cells
    /// [0, 0, 0] (material 7) and [1, 1, 0] (material 21); [1, 1, 1] (material 31)
    /// touches only at a different corner.
    fn corner_case() -> (
        RayVolume,
        HierarchyVolume,
        [f64; 3],
        [f64; 3],
        Mat4,
        Vec3,
        RayHit,
    ) {
        let mut world = World::new(4);
        solid(&mut world, &[[0, 0, 0]], 7);
        solid(&mut world, &[[1, 1, 0]], 21);
        solid(&mut world, &[[1, 1, 1]], 31);
        let pack =
            RayVolume::pack(&world, EPOCH, [0, 0, 0], [2, 2, 2], fixtures::palette()).unwrap();
        let volume = build(&pack, BlockShape::CUBE4);
        let origin = [-1.0, -1.0, 0.25];
        let direction = fixtures::oracle_direction([2.0, 2.0, 0.0]);
        let (view_projection, eye) = camera(
            Vec3::from_array(origin.map(|value| value as f32)),
            Vec3::from_array(direction.map(|value| value as f32)),
            false,
        );
        let distance = 8.0f64.sqrt();
        let hit = RayHit {
            cell: [0, 0, 0],
            normal: [1, 0, 0],
            distance: distance as f32,
            material: 7,
        };
        (pack, volume, origin, direction, view_projection, eye, hit)
    }

    #[test]
    fn classifier_accepts_exact_and_proven_tie_resolutions() {
        let (_pack, volume, origin, direction, view_projection, eye, hit) = corner_case();
        let expectation = expectation(hit, origin, direction, view_projection, eye);
        // Exact: the CPU hit's own shading and depth.
        let exact = observed_from(&hit, hit.normal, hit.material, &expectation);
        assert!(matches!(
            classify(&volume, &expectation, &exact),
            Ok(Verdict::Exact)
        ));
        // Positive control: the kernel enters the same point through the [1, 1, 0]
        // corner cell instead of [0, 0, 0]; the point is on both shared planes.
        let tie = observed_from(&hit, [-1, 0, 0], 21, &expectation);
        match classify(&volume, &expectation, &tie) {
            Ok(Verdict::Tie(_)) => {}
            other => panic!("expected a proven tie, got {other:?}"),
        }
        // Same cell, different entry face at the corner.
        let other_face = observed_from(&hit, [0, 1, 0], 7, &expectation);
        assert!(matches!(
            classify(&volume, &expectation, &other_face),
            Ok(Verdict::Tie(_))
        ));
    }

    #[test]
    fn classifier_rejects_unexplained_disagreements() {
        let (_pack, volume, origin, direction, view_projection, eye, hit) = corner_case();
        let expectation = expectation(hit, origin, direction, view_projection, eye);
        // Negative: a material one step further along z; the point is not on the plane
        // z = 1 the two cells would share.
        let wrong_cell = observed_from(&hit, [-1, 0, 0], 31, &expectation);
        assert!(classify(&volume, &expectation, &wrong_cell).is_err());
        // Negative: a material no cell contains.
        let wrong_material = observed_from(&hit, [1, 0, 0], 200, &expectation);
        assert!(classify(&volume, &expectation, &wrong_material).is_err());
        // Negative: a normal on a face the point does not lie on.
        let wrong_face = observed_from(&hit, [1, 1, 0], 7, &expectation);
        assert!(classify(&volume, &expectation, &wrong_face).is_err());
        // Negative: the depth disagrees by more than the f32 bound.
        let mut shifted = observed_from(&hit, hit.normal, hit.material, &expectation);
        shifted.depth += 1e-3;
        assert!(classify(&volume, &expectation, &shifted).is_err());
        // Negative: a miss where the CPU hit.
        assert!(classify(&volume, &expectation, &Rendered::background()).is_err());
        // Negative: a hit where the CPU missed.
        let miss = Expectation {
            hit: None,
            ..expectation
        };
        assert!(classify(&volume, &miss, &exact_observed()).is_err());
        // Positive: both sides miss.
        assert!(matches!(
            classify(&volume, &miss, &Rendered::background()),
            Ok(Verdict::Miss)
        ));
    }

    fn exact_observed() -> Rendered {
        Rendered {
            rgba: [0.5, 0.25, 0.5, 1.0],
            depth: 0.5,
        }
    }

    #[test]
    fn mirrored_ray_tracks_the_analytic_probe() {
        let mut world = World::new(7);
        solid(&mut world, &[[1, 1, 1]], 3);
        let pack =
            RayVolume::pack(&world, EPOCH, [0, 0, 0], [4, 4, 4], fixtures::palette()).unwrap();
        for (origin, raw, orthographic) in [
            ([0.5, 0.5, 0.5], [1.0, 0.0, 0.0], false),
            ([3.5, 0.5, 0.5], [-1.0, 0.0, 0.0], true),
            ([0.5, 3.5, 0.5], [0.0, -1.0, 0.0], false),
            ([3.5, 3.5, 3.5], [-1.0, -1.0, -1.0], true),
            ([-2.5, 2.5, 2.5], [1.0, -1.0, 1.0], false),
        ] {
            let direction = fixtures::oracle_direction(raw);
            let (view_projection, eye) = camera(
                Vec3::from_array(origin.map(|value| value as f32)),
                Vec3::from_array(direction.map(|value| value as f32)),
                orthographic,
            );
            let uniform = pack.uniform(view_projection, eye, Sun::default()).unwrap();
            let inverse = Mat4::from_cols_array_2d(&uniform.inverse_view_projection);
            let (mirrored, mirrored_direction, length) = shader_ray(&inverse).unwrap();
            for axis in 0..3 {
                assert!((mirrored[axis] - origin[axis]).abs() <= RAY_TOLERANCE);
                assert!(
                    (mirrored_direction[axis] - direction[axis]).abs() <= RAY_TOLERANCE,
                    "origin {origin:?} raw {raw:?}"
                );
            }
            assert!((length - f64::from(FAR - NEAR)).abs() <= SEGMENT_TOLERANCE);
        }
    }
}
