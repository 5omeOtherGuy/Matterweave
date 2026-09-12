//! Full-image ray / raster / hybrid comparison on one Vulkan device.
//!
//! Three paths share one camera, palette, sun, ambient, fog, depth range and
//! image size, and none of them uses shadows or global illumination:
//!
//! * `raster`: exposed-surface `World::mesh` built from the *raster* cell set only,
//!   drawn with `comparison_raster.wgsl` (mesh in, matched lighting out).
//! * `ray`: `RayVolume` packed from the *ray* cell set only, drawn with the real
//!   Naga-compiled `ray_reference.wgsl` traversal shader.
//! * `hybrid`: one render pass, two subpasses sharing the same color and depth
//!   attachments. Subpass 0 rasterizes the mesh, subpass 1 runs the ray shader
//!   fullscreen with depth test and depth write against that same depth buffer,
//!   so the nearer of the two surfaces wins per pixel with correct mutual
//!   occlusion. This is a real combination, not a renamed path.
//!
//! Split fixtures use disjoint sets separated by an empty column to test mutual
//! occlusion. Matched fixtures give each path the complete authoritative world
//! and require full-image color/depth agreement within declared tolerances.
//!
//! Pass criteria (declared before measurement, see
//! `docs/performance/renderer-comparison.md`):
//!
//! 1. every path produces an image and raster plus ray both cover pixels;
//! 2. hybrid equals the per-pixel minimum-depth composite of raster and ray
//!    (0 interior and 0 edge violations within tolerance);
//! 3. the ray path agrees with the CPU `World::raycast` oracle on sampled pixels
//!    that are not boundary-tie or inside-solid classified;
//! 4. no Vulkan validation error is reported while the layer is active;
//! 5. an edit invalidates the same world's pack and changes the hybrid image;
//! 6. matched images have zero unexplained mismatches; at most 0.05% of
//!    pixels may differ only at CPU-confirmed face edges within 0.001 pixel.
//!
//! Costs are reported as wall time for packing, mesh build, combined resource/pipeline setup and
//! draw+readback separately. Draw+readback includes a synchronous queue wait, so
//! it is NOT isolated GPU time. No renderer is selected here.
//!
//! Exit codes: 0 all checks passed, 1 at least one check failed,
//! 2 no usable Vulkan 1.1 graphics device (checks NOT RUN).
use ash::vk;
use ash::vk::Handle;
use glam::{Mat4, Vec3, Vec4};
use matterweave_core::World;
use matterweave_render::ray_reference::{RayVolume, FRAGMENT_SPIRV, VERTEX_SPIRV};
use matterweave_render::Sun;
use std::io::Cursor;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Instant;

const WIDTH: u32 = 128;
const HEIGHT: u32 = 128;
const PIXELS: usize = (WIDTH * HEIGHT) as usize;
const NEAR: f32 = 0.25;
const FAR: f32 = 90.0;
/// 8-bit quantization plus float interpolation differences.
const COLOR_TOL: f32 = 3.0 / 255.0;
const DEPTH_TOL: f32 = 1.0e-3;
/// A sampled ray whose hit point is this close to a cell boundary is a tie.
const TIE_EPS: f32 = 1.0e-4;

const RASTER_VERTEX_SPIRV: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/comparison_raster.vs_main.spv"));
const RASTER_FRAGMENT_SPIRV: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/comparison_raster.fs_main.spv"));

static VALIDATION_ERRORS: AtomicUsize = AtomicUsize::new(0);
static VALIDATION_LOG: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn err(e: vk::Result) -> String {
    format!("{e:?}")
}

/// Mirrors `matterweave_core::mesh::color` so both paths share one palette.
fn palette() -> [[f32; 3]; 256] {
    std::array::from_fn(|i| match i {
        1 => [0.29, 0.48, 0.27],
        2 => [0.35, 0.24, 0.17],
        3 => [0.47, 0.51, 0.52],
        4 => [0.72, 0.65, 0.46],
        5 => [0.30, 0.20, 0.13],
        6 => [0.19, 0.38, 0.28],
        7 => [0.42, 0.77, 0.72],
        _ => [0.69, 0.46, 0.33],
    })
}

struct Cam {
    eye: [f32; 3],
    target: [f32; 3],
    orthographic: bool,
    /// Orthographic half-height, or perspective vertical field of view in radians.
    extent: f32,
}

impl Cam {
    fn view_projection(&self) -> Mat4 {
        let eye = Vec3::from_array(self.eye);
        let forward = (Vec3::from_array(self.target) - eye).normalize();
        let up = if forward.y.abs() > 0.95 {
            Vec3::Z
        } else {
            Vec3::Y
        };
        let view = Mat4::look_at_rh(eye, eye + forward, up);
        let projection = if self.orthographic {
            Mat4::orthographic_rh(
                -self.extent,
                self.extent,
                -self.extent,
                self.extent,
                NEAR,
                FAR,
            )
        } else {
            Mat4::perspective_rh(self.extent, WIDTH as f32 / HEIGHT as f32, NEAR, FAR)
        };
        projection * view
    }
}

struct Fixture {
    name: &'static str,
    cells: Vec<([i32; 3], u8)>,
    /// Split mode puts `x < split` in the ray set and `x > split` in the raster
    /// set, with `x == split` empty. Matched mode gives both paths every cell.
    split: i32,
    camera: Cam,
    /// Optional edit applied for a second "-edited" run.
    edit: Option<([i32; 3], u8)>,
    /// Marker cell used to validate the camera and image orientation convention.
    marker: [i32; 3],
    /// Optional inclusive near-detail AABB. When set, the sparse CPU oracle must
    /// sample at least one hit inside it, so thin near geometry cannot pass
    /// merely because the terrain behind it was sampled.
    detail: Option<([i32; 3], [i32; 3])>,
}

fn fixtures() -> Vec<Fixture> {
    // Thin one-cell plate in the ray set, solid block in the raster set.
    let mut thin: Vec<([i32; 3], u8)> = Vec::new();
    for y in 0..5 {
        for z in 0..5 {
            thin.push(([-3, y, z], 7));
        }
    }
    for x in 3..5 {
        for y in 0..2 {
            for z in 0..2 {
                thin.push(([x, y, z], 3));
            }
        }
    }
    // Crosses the chunk boundary at x = -16 with negative Y and Z cells.
    let mut negative: Vec<([i32; 3], u8)> = Vec::new();
    for y in -2..1 {
        for z in -2..1 {
            negative.push(([-17, y, z], 6));
        }
    }
    negative.push(([-16, -1, -1], 5));
    negative.push(([-19, 0, 2], 4));
    for y in 0..3 {
        negative.push(([-14, y, 0], 2));
    }
    // Close perspective view of a one-cell-thin floor plate plus a thin pillar.
    let mut close: Vec<([i32; 3], u8)> = Vec::new();
    for x in -4..1 {
        for z in -3..2 {
            close.push(([x, 0, z], 4));
        }
    }
    close.push(([-2, 1, 0], 1));
    close.push(([-2, 2, 0], 1));
    for y in 0..3 {
        close.push(([3, y, -1], 3));
    }
    // Orthographic diagonal view: thin diagonal wall plus a tall thin column.
    let mut ortho: Vec<([i32; 3], u8)> = Vec::new();
    for step in 0..6 {
        ortho.push(([-3 + step, step, 0], 2));
    }
    for y in 0..6 {
        ortho.push(([4, y, 3], 5));
    }
    vec![
        Fixture {
            name: "thin-plate-vs-block",
            cells: thin.clone(),
            split: 0,
            camera: Cam {
                eye: [12.0, 6.0, 14.0],
                target: [0.0, 1.5, 1.5],
                orthographic: false,
                extent: 0.9,
            },
            edit: Some(([-3, 4, 4], 1)),
            marker: [4, 1, 1],
            detail: None,
        },
        Fixture {
            name: "orthographic-opening-removal",
            cells: thin,
            split: 0,
            camera: Cam {
                eye: [12.0, 6.0, 14.0],
                target: [0.0, 1.5, 1.5],
                orthographic: true,
                extent: 5.0,
            },
            edit: Some(([-3, 2, 2], 0)),
            marker: [4, 1, 1],
            detail: None,
        },
        Fixture {
            name: "negative-chunk-boundary",
            cells: negative,
            split: -15,
            camera: Cam {
                eye: [-8.0, 8.0, 14.0],
                target: [-16.0, -1.0, 0.0],
                orthographic: false,
                extent: 0.8,
            },
            edit: None,
            marker: [-14, 1, 0],
            detail: None,
        },
        Fixture {
            name: "close-perspective-thin-floor",
            cells: close,
            split: 2,
            camera: Cam {
                eye: [6.0, 4.5, 7.0],
                target: [-1.0, 0.5, -0.5],
                orthographic: false,
                extent: 1.1,
            },
            edit: Some(([1, 1, 1], 7)),
            marker: [3, 1, -1],
            detail: None,
        },
        Fixture {
            name: "ortho-diagonal-thin-wall",
            cells: ortho,
            split: 1,
            camera: Cam {
                eye: [12.0, 8.0, 12.0],
                target: [0.0, 2.0, 1.0],
                orthographic: true,
                extent: 6.0,
            },
            edit: None,
            marker: [4, 3, 3],
            detail: None,
        },
    ]
}

/// Opt-in landscape fixtures (`--landscape`). They drive the same real paths,
/// CPU oracle, edit invalidation and image gates as the default suite, but use
/// larger scenes and are therefore excluded from the default 16-run CI gate.
fn landscape_fixtures() -> Vec<Fixture> {
    // (1) One-cell-thick vegetation a few cells in front of a distant stepped
    // ridge at z = 40..42. The ridge top profile is the occluding terrain
    // silhouette; the near stems and fronds are the thin detail the paths must
    // order against it. Both paths see the whole world in matched mode and the
    // x < 0 / x > 0 halves in split mode, so each half needs real geometry.
    let mut vegetation: Vec<([i32; 3], u8)> = Vec::new();
    for x in -24i32..=24 {
        let ridge_top = match x {
            -2..=3 => 6,
            12..=15 => 7,
            _ => 8 + x.rem_euclid(5),
        };
        for z in 40..=42 {
            for y in -20..=ridge_top {
                vegetation.push(([x, y, z], if y == ridge_top { 3 } else { 2 }));
            }
        }
    }
    for (index, x) in [-3, -1, 1, 3].into_iter().enumerate() {
        let height = 3 + (index % 3) as i32;
        for y in 0..height {
            vegetation.push(([x, y, 3], 1));
        }
        vegetation.push(([x - 1, height - 1, 3], 6));
        vegetation.push(([x + 1, height - 1, 3], 6));
        vegetation.push(([x, height - 1, 4], 6));
    }

    // (2) Negative-coordinate rock terrain under an orthographic camera: a
    // ground slab, a front wall with a three-cell-wide doorway, a back wall and
    // loose boulders. The edit blocks the lower middle of the opening.
    let mut terrain: Vec<([i32; 3], u8)> = Vec::new();
    for x in -44..=-16 {
        for z in -48..=-2 {
            for y in -6..=-3 {
                terrain.push(([x, y, z], 2));
            }
        }
    }
    for x in -44..=-16 {
        for z in -20..=-19 {
            for y in -2..=8 {
                if (-31..=-29).contains(&x) && (-2..=3).contains(&y) {
                    continue;
                }
                terrain.push(([x, y, z], 3));
            }
        }
    }
    for x in -44..=-16 {
        for z in -46..=-45 {
            for y in -2..=8 {
                terrain.push(([x, y, z], if y == 8 { 4 } else { 3 }));
            }
        }
    }
    for (cell, material) in [
        ([-34, -2, -16], 3),
        ([-34, -1, -16], 3),
        ([-27, -2, -14], 5),
        ([-24, -2, -12], 5),
        ([-30, -2, -10], 7),
        ([-21, -2, -18], 5),
        ([-21, -1, -18], 5),
        ([-21, 0, -18], 5),
    ] {
        terrain.push((cell, material));
    }

    vec![
        Fixture {
            name: "vegetation-vs-distant-ridge",
            cells: vegetation,
            split: 0,
            camera: Cam {
                eye: [0.0, 4.0, -6.0],
                target: [0.0, 3.0, 30.0],
                orthographic: false,
                extent: 0.9,
            },
            edit: Some(([-3, 1, 3], 0)),
            marker: [6, 9, 40],
            // Keeps the near stems and fronds in the sampled oracle; the ridge
            // alone must not be able to satisfy this fixture's coverage checks.
            detail: Some(([-5, 0, 3], [5, 5, 4])),
        },
        Fixture {
            name: "negative-ortho-opening-edit",
            cells: terrain,
            split: -36,
            camera: Cam {
                eye: [-30.0, 14.0, 6.0],
                target: [-30.0, -3.0, -30.0],
                orthographic: true,
                extent: 16.0,
            },
            edit: Some(([-30, 0, -19], 3)),
            marker: [-24, 8, -20],
            detail: None,
        },
    ]
}

struct Images {
    color: Vec<[u8; 4]>,
    depth: Vec<f32>,
}

impl Images {
    fn covered(&self, index: usize) -> bool {
        self.depth[index] < 1.0 - 1.0e-6
    }
    fn color_f32(&self, index: usize) -> [f32; 3] {
        let c = self.color[index];
        [
            c[0] as f32 / 255.0,
            c[1] as f32 / 255.0,
            c[2] as f32 / 255.0,
        ]
    }
}

#[derive(Default)]
struct Diff {
    mismatched: usize,
    edge: usize,
    interior: usize,
    max_color: f32,
    sum_color: f32,
}

/// Pixels whose 4-neighbourhood spans a coverage change or a depth step of
/// either image are silhouette pixels: rasterization and analytic rays legitimately
/// disagree there. Interior mismatches are reported separately and must be zero.
fn edge_mask(a: &Images, b: &Images) -> Vec<bool> {
    let mut mask = vec![false; PIXELS];
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let index = (y * WIDTH + x) as usize;
            let here = [a.covered(index), b.covered(index)];
            let mut is_edge = here[0] != here[1];

            for (dx, dy) in [(1i32, 0i32), (0, 1), (-1, 0), (0, -1)] {
                let nx = x as i32 + dx;
                let ny = y as i32 + dy;
                if !(0..WIDTH as i32).contains(&nx) || !(0..HEIGHT as i32).contains(&ny) {
                    continue;
                }
                let n = (ny as u32 * WIDTH + nx as u32) as usize;
                if a.covered(n) != here[0] || b.covered(n) != here[1] {
                    is_edge = true;
                }
                if (a.depth[n] - a.depth[index]).abs() > 0.01
                    || (b.depth[n] - b.depth[index]).abs() > 0.01
                {
                    is_edge = true;
                }
            }
            mask[index] = is_edge;
        }
    }
    mask
}

fn compare(a: &Images, b: &Images, edges: &[bool]) -> Diff {
    let mut diff = Diff::default();
    for (index, &is_edge) in edges.iter().enumerate().take(PIXELS) {
        let ca = a.color_f32(index);
        let cb = b.color_f32(index);
        let color_delta = (0..3).map(|c| (ca[c] - cb[c]).abs()).fold(0.0f32, f32::max);
        let depth_delta = (a.depth[index] - b.depth[index]).abs();
        if !a.depth[index].is_finite()
            || !b.depth[index].is_finite()
            || color_delta > COLOR_TOL
            || depth_delta > DEPTH_TOL
        {
            diff.mismatched += 1;
            diff.max_color = diff.max_color.max(color_delta);
            diff.sum_color += color_delta;
            if is_edge {
                diff.edge += 1;
            } else {
                diff.interior += 1;
            }
        }
    }
    diff
}

struct Gpu {
    _entry: ash::Entry,
    device_name: String,
    validation: bool,
    instance: ash::Instance,
    device: ash::Device,
    queue: vk::Queue,
    memory: vk::PhysicalDeviceMemoryProperties,
    pool: vk::CommandPool,
    _debug_api: Option<ash::ext::debug_utils::Instance>,
    _debug: Option<vk::DebugUtilsMessengerEXT>,
}

unsafe extern "system" fn validation_callback(
    severity: vk::DebugUtilsMessageSeverityFlagsEXT,
    _kind: vk::DebugUtilsMessageTypeFlagsEXT,
    data: *const vk::DebugUtilsMessengerCallbackDataEXT<'_>,
    _user: *mut std::ffi::c_void,
) -> vk::Bool32 {
    // SAFETY: Vulkan supplies callback data valid for this call.
    if data.is_null() {
        return vk::FALSE;
    }
    if !severity.contains(vk::DebugUtilsMessageSeverityFlagsEXT::ERROR) {
        return vk::FALSE;
    }
    // SAFETY: Vulkan guarantees the message string for the duration of the call.
    let message = unsafe {
        std::ffi::CStr::from_ptr((*data).p_message)
            .to_string_lossy()
            .to_string()
    };
    VALIDATION_ERRORS.fetch_add(1, Ordering::Relaxed);
    let short: String = message.chars().take(200).collect();
    if let Ok(mut log) = VALIDATION_LOG.lock() {
        if log.len() < 32 {
            log.push(short);
        }
    }
    vk::FALSE
}

impl Gpu {
    fn new() -> Result<Self, String> {
        let entry = unsafe { ash::Entry::load() }.map_err(|e| format!("Vulkan loader: {e}"))?;
        // SAFETY: no instance exists yet; the loader outlives everything here.
        let (extensions, layers) = unsafe {
            let available = entry
                .enumerate_instance_extension_properties(None)
                .map_err(err)?;
            let names = |list: &[vk::ExtensionProperties]| {
                list.iter()
                    .map(|e| {
                        e.extension_name
                            .iter()
                            .take_while(|&&c| c != 0)
                            .map(|&c| c as u8 as char)
                            .collect::<String>()
                    })
                    .collect::<Vec<_>>()
            };
            let extensions = names(&available);
            let layers = entry
                .enumerate_instance_layer_properties()
                .map_err(err)
                .map(|list| {
                    list.iter()
                        .map(|l| {
                            l.layer_name
                                .iter()
                                .take_while(|&&c| c != 0)
                                .map(|&c| c as u8 as char)
                                .collect::<String>()
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            (extensions, layers)
        };
        let want_debug = extensions.iter().any(|n| n == "VK_EXT_debug_utils")
            && layers.iter().any(|n| n == "VK_LAYER_KHRONOS_validation");
        let raw_layers: Vec<std::ffi::CString> = if want_debug {
            vec![std::ffi::CString::new("VK_LAYER_KHRONOS_validation").unwrap()]
        } else {
            Vec::new()
        };
        let raw_extensions: Vec<std::ffi::CString> = if want_debug {
            vec![std::ffi::CString::new("VK_EXT_debug_utils").unwrap()]
        } else {
            Vec::new()
        };
        let layer_ptrs: Vec<*const std::ffi::c_char> =
            raw_layers.iter().map(|c| c.as_ptr()).collect();
        let extension_ptrs: Vec<*const std::ffi::c_char> =
            raw_extensions.iter().map(|c| c.as_ptr()).collect();
        let app = vk::ApplicationInfo::default()
            .application_name(c"matterweave-renderer-comparison")
            .api_version(vk::API_VERSION_1_1);
        let mut info = vk::InstanceCreateInfo::default().application_info(&app);
        if !layer_ptrs.is_empty() {
            info = info.enabled_layer_names(&layer_ptrs);
        }
        if !extension_ptrs.is_empty() {
            info = info.enabled_extension_names(&extension_ptrs);
        }
        // SAFETY: the CStrings outlive the instance creation call.
        let instance =
            unsafe { entry.create_instance(&info, None) }.map_err(|e| format!("{e:?}"))?;
        // SAFETY: queried handles belong to the live instance.
        let (physical, family) = unsafe {
            let mut chosen = None;
            for device in instance.enumerate_physical_devices().map_err(err)? {
                if instance.get_physical_device_properties(device).api_version < vk::API_VERSION_1_1
                {
                    continue;
                }
                let supported_formats = [
                    (
                        vk::Format::R8G8B8A8_UNORM,
                        vk::FormatFeatureFlags::COLOR_ATTACHMENT,
                    ),
                    (
                        vk::Format::D32_SFLOAT,
                        vk::FormatFeatureFlags::DEPTH_STENCIL_ATTACHMENT,
                    ),
                ]
                .into_iter()
                .all(|(format, attachment)| {
                    instance
                        .get_physical_device_format_properties(device, format)
                        .optimal_tiling_features
                        .contains(attachment | vk::FormatFeatureFlags::TRANSFER_SRC)
                });
                if !supported_formats {
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
        // SAFETY: the queue family was validated above.
        let (device, queue, memory, device_name) = unsafe {
            let device = instance
                .create_device(
                    physical,
                    &vk::DeviceCreateInfo::default().queue_create_infos(&queues),
                    None,
                )
                .map_err(err)?;
            let memory = instance.get_physical_device_memory_properties(physical);
            let queue = device.get_device_queue(family, 0);
            let props = instance.get_physical_device_properties(physical);
            let name = props
                .device_name
                .iter()
                .take_while(|&&c| c != 0)
                .map(|&c| c as u8 as char)
                .collect::<String>();
            (
                device,
                queue,
                memory,
                format!(
                    "{name} (Vulkan {}.{})",
                    vk::api_version_major(props.api_version),
                    vk::api_version_minor(props.api_version)
                ),
            )
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
        let (debug_api, debug) = if want_debug {
            let api = ash::ext::debug_utils::Instance::new(&entry, &instance);
            let severity = vk::DebugUtilsMessageSeverityFlagsEXT::ERROR;
            let mut message = vk::DebugUtilsMessengerCreateInfoEXT::default()
                .message_severity(severity)
                .message_type(
                    vk::DebugUtilsMessageTypeFlagsEXT::GENERAL
                        | vk::DebugUtilsMessageTypeFlagsEXT::VALIDATION
                        | vk::DebugUtilsMessageTypeFlagsEXT::PERFORMANCE,
                );
            message.pfn_user_callback = Some(validation_callback);
            // SAFETY: the callback is a plain extern "system" fn with no captures.
            let handle = unsafe { api.create_debug_utils_messenger(&message, None) }
                .map_err(err)
                .unwrap_or_else(|e| {
                    println!("validation messenger unavailable: {e}");
                    vk::DebugUtilsMessengerEXT::null()
                });
            (Some(api), Some(handle))
        } else {
            (None, None)
        };
        Ok(Self {
            _entry: entry,
            device_name,
            validation: debug.is_some_and(|h| !h.is_null()),
            instance,
            device,
            queue,
            memory,
            pool,
            _debug_api: debug_api,
            _debug: debug,
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
            if let (Some(api), Some(handle)) = (&self._debug_api, self._debug) {
                if !handle.is_null() {
                    api.destroy_debug_utils_messenger(handle, None);
                }
            }
            self.device.destroy_command_pool(self.pool, None);
            self.device.destroy_device(None);
            self.instance.destroy_instance(None);
        }
    }
}

struct Buffer {
    handle: vk::Buffer,
    memory: vk::DeviceMemory,
}

struct Attachment {
    image: vk::Image,
    memory: vk::DeviceMemory,
    view: vk::ImageView,
}

struct Session {
    uniform: Buffer,
    materials: Buffer,
    palette: Buffer,
    vertices: Buffer,
    indices: Buffer,
    color_read: Buffer,
    depth_read: Buffer,
    color: Attachment,
    depth: Attachment,
    ray_layout: vk::PipelineLayout,
    raster_layout: vk::PipelineLayout,
    set_layout: vk::DescriptorSetLayout,
    descriptor_pool: vk::DescriptorPool,
    set: vk::DescriptorSet,
    ray_pipeline: vk::Pipeline,
    raster_pipeline: vk::Pipeline,
    pass: vk::RenderPass,
    framebuffer: vk::Framebuffer,
    buffer: vk::CommandBuffer,
}

impl Session {
    fn new(gpu: &Gpu, pack: &RayVolume, vertices: &[u8], indices: &[u8]) -> Result<Self, String> {
        // SAFETY: every handle is owned here and freed in `destroy`.
        unsafe {
            let device = &gpu.device;
            let uniform = Self::buffer(gpu, 256, vk::BufferUsageFlags::UNIFORM_BUFFER, None)?;
            let material_bytes: Vec<u8> = pack
                .materials()
                .iter()
                .flat_map(|m| m.to_le_bytes())
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
                .flat_map(|c| c.iter().flat_map(|v| v.to_le_bytes()))
                .collect();
            let palette = Self::buffer(
                gpu,
                palette_bytes.len(),
                vk::BufferUsageFlags::STORAGE_BUFFER,
                Some(&palette_bytes),
            )?;
            let vertices = Self::buffer(
                gpu,
                vertices.len().max(4),
                vk::BufferUsageFlags::VERTEX_BUFFER,
                Some(vertices),
            )?;
            let indices = Self::buffer(
                gpu,
                indices.len().max(4),
                vk::BufferUsageFlags::INDEX_BUFFER,
                Some(indices),
            )?;
            let color_read =
                Self::buffer(gpu, PIXELS * 4, vk::BufferUsageFlags::TRANSFER_DST, None)?;
            let depth_read =
                Self::buffer(gpu, PIXELS * 4, vk::BufferUsageFlags::TRANSFER_DST, None)?;

            let bindings = [
                vk::DescriptorSetLayoutBinding::default()
                    .binding(0)
                    .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
                    .descriptor_count(1)
                    .stage_flags(vk::ShaderStageFlags::FRAGMENT),
                vk::DescriptorSetLayoutBinding::default()
                    .binding(1)
                    .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                    .descriptor_count(1)
                    .stage_flags(vk::ShaderStageFlags::FRAGMENT),
                vk::DescriptorSetLayoutBinding::default()
                    .binding(2)
                    .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
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
            let ray_layout = device
                .create_pipeline_layout(
                    &vk::PipelineLayoutCreateInfo::default().set_layouts(&layouts),
                    None,
                )
                .map_err(err)?;
            let push_range = [vk::PushConstantRange::default()
                .stage_flags(vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT)
                .offset(0)
                .size(96)];
            let raster_layout = device
                .create_pipeline_layout(
                    &vk::PipelineLayoutCreateInfo::default().push_constant_ranges(&push_range),
                    None,
                )
                .map_err(err)?;
            let pool_sizes = [
                vk::DescriptorPoolSize::default()
                    .ty(vk::DescriptorType::UNIFORM_BUFFER)
                    .descriptor_count(1),
                vk::DescriptorPoolSize::default()
                    .ty(vk::DescriptorType::STORAGE_BUFFER)
                    .descriptor_count(2),
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
            let uniform_info = [vk::DescriptorBufferInfo::default()
                .buffer(uniform.handle)
                .range(192)];
            let material_info = [vk::DescriptorBufferInfo::default()
                .buffer(materials.handle)
                .range(pack.materials().len() as u64 * 4)];
            let palette_info = [vk::DescriptorBufferInfo::default()
                .buffer(palette.handle)
                .range(4096)];
            device.update_descriptor_sets(
                &[
                    vk::WriteDescriptorSet::default()
                        .dst_set(set)
                        .dst_binding(0)
                        .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
                        .buffer_info(&uniform_info),
                    vk::WriteDescriptorSet::default()
                        .dst_set(set)
                        .dst_binding(1)
                        .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                        .buffer_info(&material_info),
                    vk::WriteDescriptorSet::default()
                        .dst_set(set)
                        .dst_binding(2)
                        .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                        .buffer_info(&palette_info),
                ],
                &[],
            );

            let ray_vs =
                ash::util::read_spv(&mut Cursor::new(VERTEX_SPIRV)).map_err(|e| e.to_string())?;
            let ray_fs =
                ash::util::read_spv(&mut Cursor::new(FRAGMENT_SPIRV)).map_err(|e| e.to_string())?;
            let raster_vs = ash::util::read_spv(&mut Cursor::new(RASTER_VERTEX_SPIRV))
                .map_err(|e| e.to_string())?;
            let raster_fs = ash::util::read_spv(&mut Cursor::new(RASTER_FRAGMENT_SPIRV))
                .map_err(|e| e.to_string())?;
            let modules = [ray_vs, ray_fs, raster_vs, raster_fs]
                .iter()
                .map(|code| {
                    device
                        .create_shader_module(
                            &vk::ShaderModuleCreateInfo::default().code(code),
                            None,
                        )
                        .map_err(err)
                })
                .collect::<Result<Vec<_>, _>>()?;
            let ray_stages = [
                vk::PipelineShaderStageCreateInfo::default()
                    .stage(vk::ShaderStageFlags::VERTEX)
                    .module(modules[0])
                    .name(c"vs_main"),
                vk::PipelineShaderStageCreateInfo::default()
                    .stage(vk::ShaderStageFlags::FRAGMENT)
                    .module(modules[1])
                    .name(c"fs_main"),
            ];
            let raster_stages = [
                vk::PipelineShaderStageCreateInfo::default()
                    .stage(vk::ShaderStageFlags::VERTEX)
                    .module(modules[2])
                    .name(c"vs_main"),
                vk::PipelineShaderStageCreateInfo::default()
                    .stage(vk::ShaderStageFlags::FRAGMENT)
                    .module(modules[3])
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
            let subpasses = [
                vk::SubpassDescription::default()
                    .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
                    .color_attachments(&color_refs)
                    .depth_stencil_attachment(&depth_ref),
                vk::SubpassDescription::default()
                    .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
                    .color_attachments(&color_refs)
                    .depth_stencil_attachment(&depth_ref),
            ];
            // Subpass 0 (raster) writes color and depth; subpass 1 (ray) reads and
            // writes both against the same depth buffer. The final dependency makes
            // attachment writes and layout transitions visible to the readback copies.
            let test_stages = vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS
                | vk::PipelineStageFlags::LATE_FRAGMENT_TESTS;
            let dependencies = [
                vk::SubpassDependency::default()
                    .src_subpass(0)
                    .dst_subpass(1)
                    .src_stage_mask(test_stages | vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
                    .src_access_mask(
                        vk::AccessFlags::COLOR_ATTACHMENT_WRITE
                            | vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE,
                    )
                    .dst_stage_mask(test_stages | vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
                    .dst_access_mask(
                        vk::AccessFlags::COLOR_ATTACHMENT_READ
                            | vk::AccessFlags::COLOR_ATTACHMENT_WRITE
                            | vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_READ
                            | vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE,
                    ),
                vk::SubpassDependency::default()
                    .src_subpass(1)
                    .dst_subpass(vk::SUBPASS_EXTERNAL)
                    .src_stage_mask(test_stages | vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
                    .src_access_mask(
                        vk::AccessFlags::COLOR_ATTACHMENT_WRITE
                            | vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE,
                    )
                    .dst_stage_mask(vk::PipelineStageFlags::TRANSFER)
                    .dst_access_mask(vk::AccessFlags::TRANSFER_READ),
            ];
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
                        .width(WIDTH)
                        .height(HEIGHT)
                        .layers(1),
                    None,
                )
                .map_err(err)?;

            let viewports = [vk::Viewport {
                x: 0.0,
                y: 0.0,
                width: WIDTH as f32,
                height: HEIGHT as f32,
                min_depth: 0.0,
                max_depth: 1.0,
            }];
            let scissors = [vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: vk::Extent2D {
                    width: WIDTH,
                    height: HEIGHT,
                },
            }];
            let blend = [vk::PipelineColorBlendAttachmentState::default()
                .color_write_mask(vk::ColorComponentFlags::RGBA)
                .blend_enable(false)];
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
            let no_vertex_input = vk::PipelineVertexInputStateCreateInfo::default();
            let attributes = [
                vk::VertexInputAttributeDescription::default()
                    .location(0)
                    .binding(0)
                    .format(vk::Format::R32G32B32_SFLOAT)
                    .offset(0),
                vk::VertexInputAttributeDescription::default()
                    .location(1)
                    .binding(0)
                    .format(vk::Format::R32G32B32_SFLOAT)
                    .offset(12),
                vk::VertexInputAttributeDescription::default()
                    .location(2)
                    .binding(0)
                    .format(vk::Format::R32G32B32_SFLOAT)
                    .offset(24),
            ];
            let bindings_desc = [vk::VertexInputBindingDescription::default()
                .binding(0)
                .stride(36)
                .input_rate(vk::VertexInputRate::VERTEX)];
            let mesh_vertex_input = vk::PipelineVertexInputStateCreateInfo::default()
                .vertex_binding_descriptions(&bindings_desc)
                .vertex_attribute_descriptions(&attributes);

            let ray_info = [vk::GraphicsPipelineCreateInfo::default()
                .stages(&ray_stages)
                .vertex_input_state(&no_vertex_input)
                .input_assembly_state(&input_assembly)
                .viewport_state(&viewport_state)
                .rasterization_state(&rasterization)
                .multisample_state(&multisample)
                .depth_stencil_state(&depth_stencil)
                .color_blend_state(&color_blend)
                .layout(ray_layout)
                .render_pass(pass)
                .subpass(1)];
            let raster_info = [vk::GraphicsPipelineCreateInfo::default()
                .stages(&raster_stages)
                .vertex_input_state(&mesh_vertex_input)
                .input_assembly_state(&input_assembly)
                .viewport_state(&viewport_state)
                .rasterization_state(&rasterization)
                .multisample_state(&multisample)
                .depth_stencil_state(&depth_stencil)
                .color_blend_state(&color_blend)
                .layout(raster_layout)
                .render_pass(pass)
                .subpass(0)];
            let ray_pipeline = device
                .create_graphics_pipelines(vk::PipelineCache::null(), &ray_info, None)
                .map_err(|(_, e)| err(e))?[0];
            let raster_pipeline = device
                .create_graphics_pipelines(vk::PipelineCache::null(), &raster_info, None)
                .map_err(|(_, e)| err(e))?[0];
            for module in &modules {
                device.destroy_shader_module(*module, None);
            }
            let buffer = device
                .allocate_command_buffers(
                    &vk::CommandBufferAllocateInfo::default()
                        .command_pool(gpu.pool)
                        .level(vk::CommandBufferLevel::PRIMARY)
                        .command_buffer_count(1),
                )
                .map_err(err)?[0];
            Ok(Self {
                uniform,
                materials,
                palette,
                vertices,
                indices,
                color_read,
                depth_read,
                color,
                depth,
                ray_layout,
                raster_layout,
                set_layout,
                descriptor_pool,
                set,
                ray_pipeline,
                raster_pipeline,
                pass,
                framebuffer,
                buffer,
            })
        }
    }

    // SAFETY: caller guarantees the device is idle when handles are destroyed.
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
        Ok(Buffer { handle, memory })
    }

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
                        width: WIDTH,
                        height: HEIGHT,
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

    /// Renders one full image. `draw_mesh` fills subpass 0, `draw_ray` subpass 1.
    /// Wall time includes the synchronous submit and readback, so it is not
    /// isolated GPU time.
    unsafe fn render(
        &self,
        gpu: &Gpu,
        uniform: &[u8; 192],
        push: &[u8; 96],
        draw_mesh: bool,
        draw_ray: bool,
        index_count: u32,
    ) -> Result<Images, String> {
        let device = &gpu.device;
        let pointer = device
            .map_memory(self.uniform.memory, 0, 192, vk::MemoryMapFlags::empty())
            .map_err(err)?;
        std::ptr::copy_nonoverlapping(uniform.as_ptr(), pointer.cast(), 192);
        device.unmap_memory(self.uniform.memory);
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
                width: WIDTH,
                height: HEIGHT,
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
        if draw_mesh && index_count > 0 {
            device.cmd_bind_pipeline(
                self.buffer,
                vk::PipelineBindPoint::GRAPHICS,
                self.raster_pipeline,
            );
            device.cmd_push_constants(
                self.buffer,
                self.raster_layout,
                vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT,
                0,
                push,
            );
            device.cmd_bind_vertex_buffers(self.buffer, 0, &[self.vertices.handle], &[0]);
            device.cmd_bind_index_buffer(
                self.buffer,
                self.indices.handle,
                0,
                vk::IndexType::UINT32,
            );
            device.cmd_draw_indexed(self.buffer, index_count, 1, 0, 0, 0);
        }
        device.cmd_next_subpass(self.buffer, vk::SubpassContents::INLINE);
        if draw_ray {
            device.cmd_bind_pipeline(
                self.buffer,
                vk::PipelineBindPoint::GRAPHICS,
                self.ray_pipeline,
            );
            device.cmd_bind_descriptor_sets(
                self.buffer,
                vk::PipelineBindPoint::GRAPHICS,
                self.ray_layout,
                0,
                &[self.set],
                &[],
            );
            device.cmd_draw(self.buffer, 3, 1, 0, 0);
        }
        device.cmd_end_render_pass(self.buffer);
        let extent = vk::Extent3D {
            width: WIDTH,
            height: HEIGHT,
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
        let color = {
            let p = device
                .map_memory(
                    self.color_read.memory,
                    0,
                    (PIXELS * 4) as u64,
                    vk::MemoryMapFlags::empty(),
                )
                .map_err(err)?
                .cast::<[u8; 4]>();
            let slice = std::slice::from_raw_parts(p, PIXELS);
            let value = slice.to_vec();
            device.unmap_memory(self.color_read.memory);
            value
        };
        let depth = {
            let p = device
                .map_memory(
                    self.depth_read.memory,
                    0,
                    (PIXELS * 4) as u64,
                    vk::MemoryMapFlags::empty(),
                )
                .map_err(err)?
                // SAFETY: offset zero; Vulkan minMemoryMapAlignment is at
                // least 64, sufficient for f32. Transfer->HOST_READ and idle
                // wait precede this coherent mapping.
                .cast::<f32>();
            let slice = std::slice::from_raw_parts(p, PIXELS);
            let value = slice.to_vec();
            device.unmap_memory(self.depth_read.memory);
            value
        };
        Ok(Images { color, depth })
    }

    unsafe fn destroy(&mut self, gpu: &Gpu) {
        let device = &gpu.device;
        device.free_command_buffers(gpu.pool, &[self.buffer]);
        device.destroy_framebuffer(self.framebuffer, None);
        for buffer in [
            &self.uniform,
            &self.materials,
            &self.palette,
            &self.vertices,
            &self.indices,
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
        device.destroy_pipeline(self.ray_pipeline, None);
        device.destroy_pipeline(self.raster_pipeline, None);
        device.destroy_render_pass(self.pass, None);
        device.destroy_descriptor_pool(self.descriptor_pool, None);
        device.destroy_pipeline_layout(self.ray_layout, None);
        device.destroy_pipeline_layout(self.raster_layout, None);
        device.destroy_descriptor_set_layout(self.set_layout, None);
    }
}

/// Pixel centre to clip space. Naga flips clip Y for Vulkan, so the fragment
/// shader's unflipped NDC has +Y at the top row of the framebuffer.
fn pixel_to_ndc(x: u32, y: u32) -> (f32, f32) {
    (
        2.0 * (x as f32 + 0.5) / WIDTH as f32 - 1.0,
        1.0 - 2.0 * (y as f32 + 0.5) / HEIGHT as f32,
    )
}

fn project(view_projection: Mat4, point: Vec3) -> Option<(u32, u32, f32)> {
    let clip = view_projection * Vec4::new(point.x, point.y, point.z, 1.0);
    if clip.w <= 0.0 {
        return None;
    }
    let ndc = clip.truncate() / clip.w;
    let px = ((ndc.x * 0.5 + 0.5) * WIDTH as f32) as i32;
    // Undo the fragment-space flip: row = (1 - ndc.y) / 2 * HEIGHT.
    let py = ((0.5 - ndc.y * 0.5) * HEIGHT as f32) as i32;
    if !(0..WIDTH as i32).contains(&px) || !(0..HEIGHT as i32).contains(&py) {
        return None;
    }
    Some((px as u32, py as u32, ndc.z))
}

fn write_ppm(path: &std::path::Path, image: &Images) -> Result<(), String> {
    let mut bytes = format!("P6\n{WIDTH} {HEIGHT}\n255\n").into_bytes();
    for pixel in &image.color {
        bytes.extend_from_slice(&[pixel[0], pixel[1], pixel[2]]);
    }
    std::fs::write(path, bytes).map_err(|e| e.to_string())
}

fn write_depth_pgm(path: &std::path::Path, image: &Images) -> Result<(), String> {
    let mut bytes = format!("P5\n{WIDTH} {HEIGHT}\n65535\n").into_bytes();
    for &d in &image.depth {
        let v = (d.clamp(0.0, 1.0) * 65535.0) as u16;
        bytes.extend_from_slice(&v.to_be_bytes());
    }
    std::fs::write(path, bytes).map_err(|e| e.to_string())
}

struct RunResult {
    /// World the ray path must reproduce: the ray cell set in split mode, every
    /// cell in matched mode. Used for CPU oracle sampling.
    oracle_world: World,
    raster: Images,
    ray: Images,
    hybrid: Images,
    pack_ms: f64,
    mesh_ms: f64,
    setup_ms: f64,
    draw_ms: [f64; 3],
}

/// Ray volume AABB covering every cell of the fixture with one cell of margin,
/// so the same bounds serve split and matched modes.
fn bounds(cells: &[([i32; 3], u8)]) -> ([i32; 3], [u32; 3]) {
    let mut min = [i32::MAX; 3];
    let mut max = [i32::MIN; 3];
    for (cell, _) in cells {
        for axis in 0..3 {
            min[axis] = min[axis].min(cell[axis]);
            max[axis] = max[axis].max(cell[axis]);
        }
    }
    let origin = min.map(|v| v - 1);
    let dimensions = std::array::from_fn(|axis| (max[axis] - min[axis] + 3).min(128) as u32);
    (origin, dimensions)
}

fn run_fixture(
    gpu: &Gpu,
    fixture: &Fixture,
    epoch: u64,
    edited: bool,
    matched: bool,
    out: &std::path::Path,
) -> Result<RunResult, String> {
    let mut world = World::new(3);
    for (cell, material) in &fixture.cells {
        world.set(*cell, *material);
    }
    if edited {
        if let Some((cell, material)) = fixture.edit {
            world.set(cell, material);
        }
    }
    let mut ray_world = World::new(3);
    let mut mesh_world = World::new(3);
    for (cell, material) in &fixture.cells {
        // The split column is air so the two sets never share a face plane;
        // matched mode keeps every cell in both paths.
        let excluded = !matched && cell[0] == fixture.split;
        if !excluded && (matched || cell[0] < fixture.split) {
            ray_world.set(*cell, *material);
        }
        if !excluded && (matched || cell[0] > fixture.split) {
            mesh_world.set(*cell, *material);
        }
    }
    if edited {
        if let Some((cell, material)) = fixture.edit {
            if matched || cell[0] < fixture.split {
                ray_world.set(cell, material);
            }
            if matched || cell[0] > fixture.split {
                mesh_world.set(cell, material);
            }
        }
    }

    let (ray_origin, ray_dimensions) = bounds(&fixture.cells);
    let pack_start = Instant::now();
    let pack = RayVolume::pack(&ray_world, epoch, ray_origin, ray_dimensions, palette())?;
    let pack_ms = pack_start.elapsed().as_secs_f64() * 1000.0;

    let mesh_start = Instant::now();
    let mesh = mesh_world.mesh();
    let mut vertex_bytes = Vec::with_capacity(mesh.vertices.len() * 36);
    for vertex in &mesh.vertices {
        for component in vertex
            .position
            .iter()
            .chain(vertex.normal.iter())
            .chain(vertex.color.iter())
        {
            vertex_bytes.extend_from_slice(&component.to_le_bytes());
        }
    }
    let mut index_bytes = Vec::with_capacity(mesh.indices.len() * 4);
    for index in &mesh.indices {
        index_bytes.extend_from_slice(&index.to_le_bytes());
    }
    let index_count = mesh.indices.len() as u32;
    let mesh_ms = mesh_start.elapsed().as_secs_f64() * 1000.0;

    let setup_start = Instant::now();
    let mut session = Session::new(gpu, &pack, &vertex_bytes, &index_bytes)?;
    let setup_ms = setup_start.elapsed().as_secs_f64() * 1000.0;

    let view_projection = fixture.camera.view_projection();
    let eye = Vec3::from_array(fixture.camera.eye);
    let sun = Sun::default();
    let uniform = pack.uniform(view_projection, eye, sun)?;
    let uniform_bytes: [u8; 192] = bytemuck::bytes_of(&uniform)
        .try_into()
        .map_err(|_| "ray uniform is not 192 bytes".to_string())?;
    let sun_direction = Vec3::from_array(sun.direction_to_sun).normalize();
    let mut push = [0u8; 96];
    for (slot, column) in view_projection.to_cols_array_2d().iter().enumerate() {
        for (index, value) in column.iter().enumerate() {
            push[slot * 16 + index * 4..slot * 16 + index * 4 + 4]
                .copy_from_slice(&value.to_le_bytes());
        }
    }
    push[64..68].copy_from_slice(&eye.x.to_le_bytes());
    push[68..72].copy_from_slice(&eye.y.to_le_bytes());
    push[72..76].copy_from_slice(&eye.z.to_le_bytes());
    push[76..80].copy_from_slice(&0.0f32.to_le_bytes());
    push[80..84].copy_from_slice(&sun_direction.x.to_le_bytes());
    push[84..88].copy_from_slice(&sun_direction.y.to_le_bytes());
    push[88..92].copy_from_slice(&sun_direction.z.to_le_bytes());
    push[92..96].copy_from_slice(&sun.intensity.to_le_bytes());

    // SAFETY: session handles are live for the whole function body.
    let mut draw_ms = [0.0f64; 3];
    let mut images = Vec::with_capacity(3);
    for (index, (mesh_draw, ray_draw)) in [(true, false), (false, true), (true, true)]
        .into_iter()
        .enumerate()
    {
        let start = Instant::now();
        let image = unsafe {
            session.render(gpu, &uniform_bytes, &push, mesh_draw, ray_draw, index_count)
        }?;
        draw_ms[index] = start.elapsed().as_secs_f64() * 1000.0;
        images.push(image);
    }
    // SAFETY: the queue was waited idle after the last submit.
    unsafe { session.destroy(gpu) };

    let name = format!(
        "{}{}{}",
        fixture.name,
        if matched { "-matched" } else { "-split" },
        if edited { "-edited" } else { "" }
    );
    for (label, image) in [
        ("raster", &images[0]),
        ("ray", &images[1]),
        ("hybrid", &images[2]),
    ] {
        write_ppm(&out.join(format!("{name}.{label}.ppm")), image)?;
        write_depth_pgm(&out.join(format!("{name}.{label}.pgm")), image)?;
    }
    let mut images = images.into_iter();
    let oracle_world = if matched { world } else { ray_world };
    Ok(RunResult {
        oracle_world,
        raster: images.next().ok_or("missing raster image")?,
        ray: images.next().ok_or("missing ray image")?,
        hybrid: images.next().ok_or("missing hybrid image")?,
        pack_ms,
        mesh_ms,
        setup_ms,
        draw_ms,
    })
}

struct OracleReport {
    sampled: usize,
    hits: usize,
    /// Hits whose cell changes under a quarter-pixel perturbation: grazing or
    /// shared-edge cases where a sample has no well-defined correct answer.
    grazing: usize,
    diagnostics: Vec<String>,
    ties: usize,
    inside: usize,
    /// Samples whose first hit lies inside the fixture's declared near-detail
    /// AABB, independent of boundary/grazing classification.
    detail_hits: usize,
    mismatched: usize,
    max_depth_delta: f32,
    max_color_delta: f32,
}

/// CPU oracle sampling: unproject sampled pixel centres, raycast the combined
/// world and compare material and depth with the ray image. Classifies
/// boundary ties and inside-solid starts instead of counting them as failures.
/// A sampled pixel is grazing when a quarter-pixel perturbation of the same ray
/// reaches a different cell. Such samples straddle a shared edge or corner, so
/// they are reported, not counted as path disagreements.
fn is_grazing(world: &World, view_projection: Mat4, x: u32, y: u32, cell: [i32; 3]) -> bool {
    let inverse = view_projection.inverse();
    for (dx, dy) in [(0.25, 0.25), (-0.25, 0.25), (0.25, -0.25), (-0.25, -0.25)] {
        let ndc = (
            2.0 * (x as f32 + 0.5 + dx) / WIDTH as f32 - 1.0,
            1.0 - 2.0 * (y as f32 + 0.5 + dy) / HEIGHT as f32,
        );
        let near = inverse * Vec4::new(ndc.0, ndc.1, 0.0, 1.0);
        let far = inverse * Vec4::new(ndc.0, ndc.1, 1.0, 1.0);
        if near.w <= 0.0 || far.w <= 0.0 {
            return true;
        }
        let origin = near.truncate() / near.w;
        let far_point = far.truncate() / far.w;
        let direction = (far_point - origin).normalize();
        let hit = world.raycast(
            origin.to_array(),
            direction.to_array(),
            (far_point - origin).length(),
        );
        if hit.is_none_or(|h| h.cell != cell) {
            return true;
        }
    }
    false
}

fn oracle_report(
    world: &World,
    ray: &Images,
    view_projection: Mat4,
    eye: Vec3,
    detail: Option<([i32; 3], [i32; 3])>,
) -> OracleReport {
    let mut report = OracleReport {
        sampled: 0,
        hits: 0,
        grazing: 0,
        diagnostics: Vec::new(),
        ties: 0,
        inside: 0,
        detail_hits: 0,
        mismatched: 0,
        max_depth_delta: 0.0,
        max_color_delta: 0.0,
    };
    let inverse = view_projection.inverse();
    for y in (0..HEIGHT).step_by(8) {
        for x in (0..WIDTH).step_by(8) {
            let index = (y * WIDTH + x) as usize;
            let (ndc_x, ndc_y) = pixel_to_ndc(x, y);
            let near = inverse * Vec4::new(ndc_x, ndc_y, 0.0, 1.0);
            let far = inverse * Vec4::new(ndc_x, ndc_y, 1.0, 1.0);
            if near.w <= 0.0 || far.w <= 0.0 {
                continue;
            }
            let origin = near.truncate() / near.w;
            let far_point = far.truncate() / far.w;
            let direction = (far_point - origin).normalize();
            let max_distance = (far_point - origin).length();
            report.sampled += 1;
            let hit = world.raycast(origin.to_array(), direction.to_array(), max_distance);
            let (want_depth, want_color) = match &hit {
                Some(hit) => {
                    report.hits += 1;
                    if let Some((min, max)) = detail {
                        if (0..3).all(|axis| (min[axis]..=max[axis]).contains(&hit.cell[axis])) {
                            report.detail_hits += 1;
                        }
                    }
                    let point = origin + direction * hit.distance;
                    let local =
                        point - Vec3::new(point.x.floor(), point.y.floor(), point.z.floor());
                    // Only the two tangential axes decide edge/corner ties: the
                    // normal axis is always exactly on a face plane.
                    let face_axis = (0..3).find(|&axis| hit.normal[axis] != 0);
                    let on_boundary = (0..3).any(|axis| {
                        Some(axis) != face_axis
                            && (local[axis] < TIE_EPS || local[axis] > 1.0 - TIE_EPS)
                    });
                    if on_boundary {
                        report.ties += 1;
                        continue;
                    }
                    if hit.normal == [0, 0, 0] {
                        report.inside += 1;
                        continue;
                    }
                    if is_grazing(world, view_projection, x, y, hit.cell) {
                        report.grazing += 1;
                        continue;
                    }
                    let clip = view_projection * Vec4::new(point.x, point.y, point.z, 1.0);
                    let normal = Vec3::new(
                        hit.normal[0] as f32,
                        hit.normal[1] as f32,
                        hit.normal[2] as f32,
                    );
                    let sunlight = normal
                        .dot(Vec3::from_array(Sun::default().direction_to_sun).normalize())
                        .max(0.0);
                    let ambient = 0.28 + 0.12 * normal.y.max(0.0);
                    let lit = Vec3::from_array(palette()[usize::from(hit.material)])
                        * (ambient + sunlight * Sun::default().intensity);
                    let fog = 1.0 - (-((point - eye).length()) * 0.013).exp();
                    let color = lit.lerp(Vec3::new(0.16, 0.24, 0.29), fog).to_array();
                    (clip.z / clip.w, color)
                }
                None => (1.0, [0.0, 0.0, 0.0]),
            };
            let depth_delta = (ray.depth[index] - want_depth).abs();
            let color_delta = (0..3)
                .map(|c| (ray.color_f32(index)[c] - want_color[c]).abs())
                .fold(0.0f32, f32::max);
            if depth_delta > DEPTH_TOL || color_delta > COLOR_TOL {
                report.mismatched += 1;
                report.max_depth_delta = report.max_depth_delta.max(depth_delta);
                report.max_color_delta = report.max_color_delta.max(color_delta);
                if report.diagnostics.len() < 6 {
                    let got = ray.color_f32(index);
                    report.diagnostics.push(format!(
                        "    sample ({x},{y}) image depth {:.5} rgb {:.3?} | oracle depth {:.5} rgb {:.3?} material {:?} cell {:?} normal {:?}",
                        ray.depth[index],
                        got,
                        want_depth,
                        want_color,
                        hit.as_ref().map(|h| h.material),
                        hit.as_ref().map(|h| h.cell),
                        hit.as_ref().map(|h| h.normal)
                    ));
                }
            }
        }
    }
    report
}

/// Only excuse color/depth pairs independently reproduced by CPU rays on two
/// different faces within 0.001 pixel of the sample. Never excuse lost coverage.
fn explained_face_edges(world: &World, vp: Mat4, eye: Vec3, a: &Images, b: &Images) -> usize {
    let inverse = vp.inverse();
    let mut explained = 0;
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let i = (y * WIDTH + x) as usize;
            let ac = a.color_f32(i);
            let bc = b.color_f32(i);
            if !a.covered(i) || !b.covered(i) || !a.depth[i].is_finite() || !b.depth[i].is_finite()
            {
                continue;
            }
            if (0..3).all(|c| (ac[c] - bc[c]).abs() <= COLOR_TOL)
                && (a.depth[i] - b.depth[i]).abs() <= DEPTH_TOL
            {
                continue;
            }
            let mut matches = [false; 2];
            let mut first_normal = None;
            let mut multiple_faces = false;
            for (dx, dy) in [
                (0.0, 0.0),
                (-0.001, -0.001),
                (-0.001, 0.001),
                (0.001, -0.001),
                (0.001, 0.001),
            ] {
                let nx = 2.0 * (x as f32 + 0.5 + dx) / WIDTH as f32 - 1.0;
                let ny = 1.0 - 2.0 * (y as f32 + 0.5 + dy) / HEIGHT as f32;
                let near = inverse * Vec4::new(nx, ny, 0.0, 1.0);
                let far = inverse * Vec4::new(nx, ny, 1.0, 1.0);
                let origin = near.truncate() / near.w;
                let endpoint = far.truncate() / far.w;
                let direction = (endpoint - origin).normalize();
                let Some(hit) = world.raycast(
                    origin.to_array(),
                    direction.to_array(),
                    (endpoint - origin).length(),
                ) else {
                    continue;
                };
                if hit.normal == [0; 3] {
                    continue;
                }
                if let Some(normal) = first_normal {
                    multiple_faces |= normal != hit.normal;
                } else {
                    first_normal = Some(hit.normal);
                }
                let point = origin + direction * hit.distance;
                let clip = vp * point.extend(1.0);
                let normal = Vec3::from_array(hit.normal.map(|v| v as f32));
                let sun = Sun::default();
                let light = 0.28
                    + 0.12 * normal.y.max(0.0)
                    + normal
                        .dot(Vec3::from_array(sun.direction_to_sun).normalize())
                        .max(0.0)
                        * sun.intensity;
                let lit = Vec3::from_array(palette()[usize::from(hit.material)]) * light;
                let color = lit
                    .lerp(
                        Vec3::new(0.16, 0.24, 0.29),
                        1.0 - (-(point - eye).length() * 0.013).exp(),
                    )
                    .clamp(Vec3::ZERO, Vec3::ONE)
                    .to_array();
                for (slot, image) in [a, b].iter().enumerate() {
                    matches[slot] |= (image.depth[i] - clip.z / clip.w).abs() <= DEPTH_TOL
                        && (0..3).all(|c| (image.color_f32(i)[c] - color[c]).abs() <= COLOR_TOL);
                }
            }
            if multiple_faces && matches.into_iter().all(|matched| matched) {
                explained += 1;
            }
        }
    }
    explained
}

fn composite(raster: &Images, ray: &Images) -> Images {
    let mut color = vec![[0u8; 4]; PIXELS];
    let mut depth = vec![1.0f32; PIXELS];
    for index in 0..PIXELS {
        let use_ray = ray.depth[index] < raster.depth[index];
        color[index] = if use_ray {
            ray.color[index]
        } else {
            raster.color[index]
        };
        depth[index] = raster.depth[index].min(ray.depth[index]);
    }
    Images { color, depth }
}

fn main() {
    let mut failures = 0usize;
    let mut checks = 0usize;
    let gpu = match Gpu::new() {
        Ok(gpu) => gpu,
        Err(message) => {
            println!("NOT RUN: {message}");
            println!("renderer_comparison: no native Vulkan 1.1 graphics device on this host");
            std::process::exit(2);
        }
    };
    // A one-cell pack exercises the smallest legal storage-buffer range.
    let mut tiny_world = World::new(3);
    tiny_world.set([0, 0, 0], 1);
    let tiny = RayVolume::pack(&tiny_world, 7, [0; 3], [1; 3], palette()).unwrap();
    match Session::new(&gpu, &tiny, &[], &[]) {
        Ok(mut session) => unsafe { session.destroy(&gpu) },
        Err(message) => {
            println!("FAIL one-cell descriptor contract: {message}");
            failures += 1;
        }
    }
    println!("device: {}", gpu.device_name);
    println!(
        "validation layer: {}",
        if gpu.validation {
            "VK_LAYER_KHRONOS_validation active (errors are failures)"
        } else {
            "not available on this host"
        }
    );
    let out = std::env::var("MATTERWEAVE_COMPARISON_OUT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("target/engine-03-comparison"));
    if let Err(message) = std::fs::create_dir_all(&out) {
        println!("FAIL cannot create output directory: {message}");
        std::process::exit(1);
    }
    println!("artifacts: {}", out.display());
    println!(
        "settings: {WIDTH}x{HEIGHT}, near {NEAR}, far {FAR}, color tol {COLOR_TOL:.5}, depth tol {DEPTH_TOL}"
    );

    // `--landscape` adds the larger functional fixtures; the default invocation
    // still runs exactly the original 16-run gate that CI executes.
    let landscape = std::env::args().skip(1).any(|arg| arg == "--landscape");
    println!(
        "landscape fixtures: {}",
        if landscape {
            "on (--landscape)"
        } else {
            "off (pass --landscape to add the landscape runs)"
        }
    );
    let mut suite = fixtures();
    if landscape {
        suite.extend(landscape_fixtures());
    }

    let mut summary = String::new();
    for fixture in suite {
        for (mode, matched) in [("split", false), ("matched", true)] {
            let runs = if fixture.edit.is_some() { 2 } else { 1 };
            let mut previous: Option<Images> = None;
            for run in 0..runs {
                let edited = run == 1;
                let label = format!(
                    "{}-{mode}{}",
                    fixture.name,
                    if edited { "-edited" } else { "" }
                );
                let result = match run_fixture(&gpu, &fixture, 7, edited, matched, &out) {
                    Ok(result) => result,
                    Err(message) => {
                        failures += 1;
                        println!("FAIL {label}: {message}");
                        continue;
                    }
                };
                let coverage = |image: &Images| (0..PIXELS).filter(|&i| image.covered(i)).count();
                let raster_coverage = coverage(&result.raster);
                let ray_coverage = coverage(&result.ray);
                let hybrid_coverage = coverage(&result.hybrid);
                println!(
                "fixture {label}: coverage raster {raster_coverage} ray {ray_coverage} hybrid {hybrid_coverage} of {PIXELS}"
            );
                println!(
                "  costs ms: pack {:.3} mesh {:.3} combined-resource-and-pipeline-setup {:.3} draw+readback raster {:.3} ray {:.3} hybrid {:.3}",
                result.pack_ms,
                result.mesh_ms,
                result.setup_ms,
                result.draw_ms[0],
                result.draw_ms[1],
                result.draw_ms[2]
            );
                checks += 1;
                if raster_coverage == 0 || ray_coverage == 0 {
                    failures += 1;
                    println!("  FAIL {label}: a path produced no geometry");
                    continue;
                }

                // Hybrid must equal the per-pixel minimum-depth composite of both paths.
                let expected = composite(&result.raster, &result.ray);
                let edges = edge_mask(&result.raster, &result.ray);
                let composite_diff = compare(&result.hybrid, &expected, &edges);
                if composite_diff.mismatched == 0 {
                    println!(
                    "  PASS hybrid shares depth: {composite} mismatches (edge {edge} interior {interior})",
                    composite = composite_diff.mismatched,
                    edge = composite_diff.edge,
                    interior = composite_diff.interior
                );
                } else {
                    failures += 1;
                    println!(
                    "  FAIL hybrid composite mismatches {} (edge {} interior {}), max color delta {:.4}",
                    composite_diff.mismatched,
                    composite_diff.edge,
                    composite_diff.interior,
                    composite_diff.max_color
                );
                }

                // Ray versus raster: silhouette-edge differences are expected
                // rasterization conventions and are reported separately.
                let path_diff = compare(&result.ray, &result.raster, &edges);
                println!(
                "  compare ray-vs-raster{note}: mismatched {mismatched} (edge-class {edge}, interior-class {interior}), mean color delta {mean:.5}, max {max:.5}",
                mismatched = path_diff.mismatched,
                edge = path_diff.edge,
                interior = path_diff.interior,
                mean = path_diff.sum_color / path_diff.mismatched.max(1) as f32,
                max = path_diff.max_color,
                // split mode draws disjoint cell sets: differences are scene
                // content, not rasterization convention.
                note = if matched { "" } else { " (disjoint cell sets: content diff)" }
            );
                let face_edges = if matched {
                    explained_face_edges(
                        &result.oracle_world,
                        fixture.camera.view_projection(),
                        Vec3::from_array(fixture.camera.eye),
                        &result.raster,
                        &result.ray,
                    )
                } else {
                    0
                };
                if matched {
                    println!(
                        "  matched face-edge ambiguities {face_edges}; unexplained {}",
                        path_diff.mismatched - face_edges
                    );
                }
                // Working fixture threshold: <=0.05% independently explained
                // face-edge samples; zero unexplained mismatch.
                if matched && (path_diff.mismatched != face_edges || face_edges * 2000 > PIXELS) {
                    failures += 1;
                    println!(
                        "  FAIL matched ray/raster differences exceed the declared face-edge gate"
                    );
                }
                if hybrid_coverage < raster_coverage.max(ray_coverage) {
                    failures += 1;
                    println!("  FAIL hybrid lost coverage present in a single path");
                }

                let view_projection = fixture.camera.view_projection();
                let eye = Vec3::from_array(fixture.camera.eye);
                let oracle = oracle_report(
                    &result.oracle_world,
                    &result.ray,
                    view_projection,
                    eye,
                    fixture.detail,
                );
                if oracle.mismatched == 0
                    && oracle.hits > oracle.ties + oracle.grazing + oracle.inside
                {
                    println!(
                    "  PASS ray matches CPU oracle: {sampled} samples, {hits} hits, {ties} ties, {grazing} grazing, {inside} inside-start, {mismatched} mismatches",
                    sampled = oracle.sampled, hits = oracle.hits, ties = oracle.ties,
                    grazing = oracle.grazing, inside = oracle.inside, mismatched = oracle.mismatched
                );
                } else {
                    failures += 1;
                    for line in &oracle.diagnostics {
                        println!("{line}");
                    }
                    println!(
                    "  FAIL ray vs CPU oracle: {mismatched} of {sampled} samples ({hits} hits, ties {ties}, grazing {grazing}, inside {inside}), max depth delta {depth:.5}, max color delta {color:.5}",
                    hits = oracle.hits,
                    mismatched = oracle.mismatched,
                    sampled = oracle.sampled,
                    ties = oracle.ties,
                    inside = oracle.inside,
                    grazing = oracle.grazing,
                    depth = oracle.max_depth_delta,
                    color = oracle.max_color_delta
                );
                }
                if let Some((min, max)) = fixture.detail {
                    if oracle.detail_hits == 0 {
                        failures += 1;
                        println!(
                            "  FAIL no CPU oracle sample reached near detail {min:?}..={max:?}"
                        );
                    } else {
                        println!(
                            "  PASS near detail sampled: {hits} CPU oracle hits inside {min:?}..={max:?}",
                            hits = oracle.detail_hits
                        );
                    }
                }

                // Camera and image-orientation convention: the marker cell must project
                // onto a covered pixel no farther than the cell centre.
                let marker = Vec3::new(
                    fixture.marker[0] as f32 + 0.5,
                    fixture.marker[1] as f32 + 0.5,
                    fixture.marker[2] as f32 + 0.5,
                );
                match project(view_projection, marker) {
                    Some((px, py, center_depth)) => {
                        let index = (py * WIDTH + px) as usize;
                        if result.hybrid.covered(index)
                            && result.hybrid.depth[index] <= center_depth + 0.05
                        {
                            println!(
                            "  PASS marker {:?} projects to ({px},{py}) depth {:.5} <= centre {:.5}",
                            fixture.marker, result.hybrid.depth[index], center_depth
                        );
                        } else {
                            failures += 1;
                            println!(
                                "  FAIL marker {:?} at ({px},{py}) depth {:.5} vs centre {:.5}",
                                fixture.marker, result.hybrid.depth[index], center_depth
                            );
                        }
                    }
                    None => {
                        failures += 1;
                        println!(
                            "  FAIL marker {:?} does not project into the image",
                            fixture.marker
                        );
                    }
                }

                if let Some(previous) = previous.take() {
                    let diff = compare(&result.hybrid, &previous, &vec![false; PIXELS]);
                    if diff.mismatched > 0 {
                        println!(
                        "  PASS edit changed the hybrid image on {changed} pixels (max color delta {max:.5})",
                        changed = diff.mismatched, max = diff.max_color
                    );
                    } else {
                        failures += 1;
                        println!("  FAIL edit left the hybrid image unchanged");
                    }
                }
                if edited {
                    let mut edited_world = combined_world(&fixture, false);
                    let (ray_origin, ray_dimensions) = bounds(&fixture.cells);
                    let pack =
                        RayVolume::pack(&edited_world, 7, ray_origin, ray_dimensions, palette())
                            .unwrap();
                    if let Some((cell, material)) = fixture.edit {
                        edited_world.set(cell, material);
                    }
                    if pack.valid_for(&edited_world, 7) {
                        failures += 1;
                        println!("  FAIL edited world still validates against the pre-edit pack");
                    } else {
                        println!("  PASS edit invalidates the ray pack");
                    }
                }
                previous = Some(result.hybrid);
                summary.push_str(&format!(
                "{label} [{mode}]: raster {raster_coverage} ray {ray_coverage} hybrid {hybrid_coverage} pixels; ray-vs-raster mismatched {m} (edge {e}, interior {i}); hybrid composite mismatches {h}\n",
                m = path_diff.mismatched,
                e = path_diff.edge,
                i = path_diff.interior,
                h = composite_diff.mismatched
            ));
            }
        }
    }
    let errors = VALIDATION_ERRORS.load(Ordering::Relaxed);
    if !gpu.validation {
        println!("NOT RUN Vulkan validation: layer unavailable");
    } else if errors == 0 {
        println!("PASS Vulkan validation: no error messages");
    } else {
        failures += 1;
        println!("FAIL Vulkan validation: {errors} error messages");
        if let Ok(log) = VALIDATION_LOG.lock() {
            for line in log.iter().take(8) {
                println!("  {line}");
            }
        }
    }
    let summary_path = out.join("summary.txt");
    let mut summary_text = format!(
        "device: {}\nsettings: {WIDTH}x{HEIGHT} near {NEAR} far {FAR} color tol {COLOR_TOL} depth tol {DEPTH_TOL}\n",
        gpu.device_name
    );
    summary_text.push_str(&summary);
    if let Err(message) = std::fs::write(&summary_path, summary_text) {
        println!("FAIL cannot write summary: {message}");
        failures += 1;
    } else {
        println!("summary: {}", summary_path.display());
    }
    println!("renderer_comparison: {checks} fixture runs, {failures} failures");
    if failures > 0 {
        std::process::exit(1);
    }
}

/// The authoritative combined world: both cell sets, exactly what the hybrid
/// image must reproduce.
fn combined_world(fixture: &Fixture, edited: bool) -> World {
    let mut world = World::new(3);
    for (cell, material) in &fixture.cells {
        world.set(*cell, *material);
    }
    if edited {
        if let Some((cell, material)) = fixture.edit {
            world.set(cell, material);
        }
    }
    world
}

#[cfg(test)]
mod tests {
    use super::*;
    use matterweave_render::ray_reference::{MAX_AXIS, MAX_CELLS};

    #[test]
    fn landscape_fixtures_fit_declared_ray_volume_bounds() {
        let landscape = landscape_fixtures();
        assert_eq!(landscape.len(), 2, "expected two landscape fixtures");
        for fixture in &landscape {
            assert!(fixture.edit.is_some(), "{} has no edit", fixture.name);
            let (cell, material) = fixture.edit.unwrap();
            let existing = fixture
                .cells
                .iter()
                .find(|(existing, _)| *existing == cell)
                .map(|(_, existing)| *existing);
            assert!(
                existing.is_none_or(|existing| existing != material),
                "{} edit cell {cell:?} does not change the surface (already {material})",
                fixture.name
            );
            assert_ne!(
                cell[0], fixture.split,
                "{} edit cell {cell:?} sits on the dropped split column",
                fixture.name
            );
            if let Some((min, max)) = fixture.detail {
                let inside = |cell: &[i32; 3]| {
                    (0..3).all(|axis| (min[axis]..=max[axis]).contains(&cell[axis]))
                };
                assert!(
                    fixture
                        .cells
                        .iter()
                        .any(|(cell, _)| inside(cell) && cell[0] < fixture.split),
                    "{} has no near detail on the ray side of the split",
                    fixture.name
                );
                assert!(
                    fixture
                        .cells
                        .iter()
                        .any(|(cell, _)| inside(cell) && cell[0] > fixture.split),
                    "{} has no near detail on the mesh side of the split",
                    fixture.name
                );
            }
            let (origin, dimensions) = bounds(&fixture.cells);
            let cells: usize = dimensions.iter().map(|&d| d as usize).product();
            assert!(
                dimensions.iter().all(|&d| (1..=MAX_AXIS).contains(&d)),
                "{} dimensions {dimensions:?} exceed the RayVolume axis bound",
                fixture.name
            );
            assert!(
                cells <= MAX_CELLS,
                "{} packs {cells} cells, over the {} cell bound",
                fixture.name,
                MAX_CELLS
            );
            // `bounds` silently clamps to 128; no fixture cell may fall outside
            // the unclamped AABB, or the pack would crop real geometry.
            for (cell, _) in &fixture.cells {
                for axis in 0..3 {
                    let relative = cell[axis] - origin[axis];
                    assert!(
                        (0..dimensions[axis] as i32).contains(&relative),
                        "{} cell {cell:?} is outside packed bounds origin {origin:?} dimensions {dimensions:?}",
                        fixture.name
                    );
                }
            }
        }
    }

    #[test]
    fn default_gate_keeps_its_fixture_set() {
        let default: Vec<&str> = fixtures().iter().map(|fixture| fixture.name).collect();
        assert_eq!(
            default,
            [
                "thin-plate-vs-block",
                "orthographic-opening-removal",
                "negative-chunk-boundary",
                "close-perspective-thin-floor",
                "ortho-diagonal-thin-wall",
            ]
        );
        assert!(
            landscape_fixtures()
                .iter()
                .all(|fixture| !default.contains(&fixture.name)),
            "landscape fixture names must not collide with the default gate"
        );
    }

    #[test]
    fn face_edge_exception_requires_both_cpu_colors_and_preserved_coverage() {
        let fixture = fixtures()
            .into_iter()
            .find(|f| f.name == "orthographic-opening-removal")
            .unwrap();
        let world = combined_world(&fixture, false);
        let vp = fixture.camera.view_projection();
        let inverse = vp.inverse();
        let (nx, ny) = pixel_to_ndc(40, 17);
        let near = inverse * Vec4::new(nx, ny, 0.0, 1.0);
        let far = inverse * Vec4::new(nx, ny, 1.0, 1.0);
        let origin = near.truncate() / near.w;
        let endpoint = far.truncate() / far.w;
        let direction = (endpoint - origin).normalize();
        let hit = world
            .raycast(
                origin.to_array(),
                direction.to_array(),
                (endpoint - origin).length(),
            )
            .unwrap();
        let clip = vp * (origin + direction * hit.distance).extend(1.0);
        let mut a = Images {
            color: vec![[0; 4]; PIXELS],
            depth: vec![1.0; PIXELS],
        };
        let mut b = Images {
            color: vec![[0; 4]; PIXELS],
            depth: vec![1.0; PIXELS],
        };
        let i = 17 * WIDTH as usize + 40;
        a.depth[i] = clip.z / clip.w;
        b.depth[i] = a.depth[i];
        // Actual +Y/+X face colors observed at the same geometric edge.
        a.color[i] = [101, 181, 173, 255];
        b.color[i] = [60, 106, 103, 255];
        let eye = Vec3::from_array(fixture.camera.eye);
        assert_eq!(explained_face_edges(&world, vp, eye, &a, &b), 1);
        b.color[i] = [255, 0, 255, 255];
        assert_eq!(explained_face_edges(&world, vp, eye, &a, &b), 0);
        b.color[i] = [60, 106, 103, 255];
        b.depth[i] = 1.0;
        assert_eq!(explained_face_edges(&world, vp, eye, &a, &b), 0);
        b.depth[i] = f32::NAN;
        assert_eq!(explained_face_edges(&world, vp, eye, &a, &b), 0);
    }

    #[test]
    fn nonfinite_depth_cannot_pass_image_comparison() {
        let a = Images {
            color: vec![[0; 4]; PIXELS],
            depth: vec![1.0; PIXELS],
        };
        let mut b = Images {
            color: vec![[0; 4]; PIXELS],
            depth: vec![1.0; PIXELS],
        };
        b.depth[17] = f32::NAN;
        assert_eq!(compare(&a, &b, &vec![false; PIXELS]).mismatched, 1);
    }
}
