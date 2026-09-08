//! Direct Vulkan exposed-surface baseline. See README.md for ownership and synchronization.
mod frustum;
mod hud;
pub mod indirect;
#[cfg(test)]
mod indirect_edge_tests;
#[cfg(test)]
mod indirect_tests;
mod lighting;
mod shadow;
mod static_scene;
mod timing;
use ash::{vk, Entry};
use bytemuck::{Pod, Zeroable};
use frustum::Frustum;
pub use hud::Hud;
pub use lighting::{LightingSettings, Sun};
use matterweave_core::Mesh;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use shadow::Shadow;
use static_scene::StaticScene;
pub use static_scene::{StaticInstance, StaticSceneStats};
use std::{collections::BTreeMap, ffi::CStr, sync::Arc, time::Instant};
pub use timing::GpuTimings;
use timing::TimestampQueries;
use winit::window::Window;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Camera {
    view_proj: [[f32; 4]; 4],
    eye: [f32; 4],
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameResult {
    Presented,
    Retry,
    OutOfMemory,
    Fatal(String),
}
type Result<T> = std::result::Result<T, String>;
fn err(e: vk::Result) -> String {
    format!("Vulkan: {e:?}")
}
fn elapsed_ms(begin: Instant) -> f64 {
    begin.elapsed().as_secs_f64() * 1000.
}

struct Instance {
    // Loader and window outlive all Vulkan handles using them.
    _entry: Entry,
    _window: Arc<Window>,
    raw: ash::Instance,
    surface_api: ash::khr::surface::Instance,
    surface: vk::SurfaceKHR,
    debug_api: Option<ash::ext::debug_utils::Instance>,
    debug: vk::DebugUtilsMessengerEXT,
}
impl Drop for Instance {
    fn drop(&mut self) {
        // SAFETY: last Arc owner; all device children are already destroyed.
        unsafe {
            self.surface_api.destroy_surface(self.surface, None);
            if let Some(api) = &self.debug_api {
                api.destroy_debug_utils_messenger(self.debug, None);
            }
            self.raw.destroy_instance(None);
        }
    }
}
unsafe extern "system" fn validation(
    severity: vk::DebugUtilsMessageSeverityFlagsEXT,
    _kind: vk::DebugUtilsMessageTypeFlagsEXT,
    data: *const vk::DebugUtilsMessengerCallbackDataEXT<'_>,
    _user: *mut std::ffi::c_void,
) -> vk::Bool32 {
    // SAFETY: Vulkan supplies callback data and a terminated message valid for this call.
    if !data.is_null() {
        unsafe {
            if !(*data).p_message.is_null() {
                eprintln!(
                    "Vulkan validation {severity:?}: {}",
                    CStr::from_ptr((*data).p_message).to_string_lossy()
                );
            }
        }
    }
    vk::FALSE
}
struct Device {
    instance: Arc<Instance>,
    raw: ash::Device,
    physical: vk::PhysicalDevice,
    queue: vk::Queue,
    family: u32,
    memory: vk::PhysicalDeviceMemoryProperties,
}
impl Drop for Device {
    fn drop(&mut self) {
        // SAFETY: resources retain their own Arc; no children remain here.
        unsafe {
            let _ = self.raw.device_wait_idle();
            self.raw.destroy_device(None);
        }
    }
}
impl Device {
    fn memory_type(&self, bits: u32, flags: vk::MemoryPropertyFlags) -> Result<u32> {
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
struct Buffer {
    device: Arc<Device>,
    raw: vk::Buffer,
    memory: vk::DeviceMemory,
    size: usize,
}
impl Buffer {
    fn new(device: Arc<Device>, bytes: &[u8], usage: vk::BufferUsageFlags) -> Result<Self> {
        Self::with_capacity(device, bytes, bytes.len(), usage)
    }
    fn with_capacity(
        device: Arc<Device>,
        bytes: &[u8],
        capacity: usize,
        usage: vk::BufferUsageFlags,
    ) -> Result<Self> {
        let mut out = Self {
            device,
            raw: vk::Buffer::null(),
            memory: vk::DeviceMemory::null(),
            size: capacity.max(bytes.len()).max(4),
        };
        // SAFETY: exclusive new buffer, requirements checked before binding, RAII cleans partial failures.
        unsafe {
            out.raw = out
                .device
                .raw
                .create_buffer(
                    &vk::BufferCreateInfo::default()
                        .size(out.size as u64)
                        .usage(usage)
                        .sharing_mode(vk::SharingMode::EXCLUSIVE),
                    None,
                )
                .map_err(err)?;
            let req = out.device.raw.get_buffer_memory_requirements(out.raw);
            let ty = out.device.memory_type(
                req.memory_type_bits,
                vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
            )?;
            out.memory = out
                .device
                .raw
                .allocate_memory(
                    &vk::MemoryAllocateInfo::default()
                        .allocation_size(req.size)
                        .memory_type_index(ty),
                    None,
                )
                .map_err(err)?;
            out.device
                .raw
                .bind_buffer_memory(out.raw, out.memory, 0)
                .map_err(err)?;
        }
        out.write(bytes)?;
        Ok(out)
    }
    fn write(&self, bytes: &[u8]) -> Result<()> {
        if bytes.len() > self.size {
            return Err("Buffer write exceeds allocation".into());
        }
        if bytes.is_empty() {
            return Ok(());
        }
        // SAFETY: caller waits frame fence before writes; mapped coherent range covers bytes,
        // source/destination cannot alias and the mapping is released before submission.
        unsafe {
            let dst = self
                .device
                .raw
                .map_memory(
                    self.memory,
                    0,
                    bytes.len() as u64,
                    vk::MemoryMapFlags::empty(),
                )
                .map_err(err)?;
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), dst.cast::<u8>(), bytes.len());
            self.device.raw.unmap_memory(self.memory);
        }
        Ok(())
    }
}

struct GpuMesh {
    vertices: Option<Buffer>,
    indices: Option<Buffer>,
    index_count: u32,
    revision: u64,
    bounds: [[f32; 3]; 2],
}

pub(crate) fn validate_mesh(mesh: &Mesh) -> Result<(u32, [[f32; 3]; 2])> {
    let count = u32::try_from(mesh.indices.len()).map_err(|_| "Mesh has more than u32 indices")?;
    if mesh
        .indices
        .iter()
        .any(|&i| i as usize >= mesh.vertices.len())
    {
        return Err("Mesh index exceeds vertex count".into());
    }
    let mut bounds = [[f32::INFINITY; 3], [f32::NEG_INFINITY; 3]];
    for vertex in &mesh.vertices {
        if !vertex
            .position
            .iter()
            .chain(&vertex.normal)
            .chain(&vertex.color)
            .all(|v| v.is_finite())
        {
            return Err("Mesh contains non-finite vertex data".into());
        }
        for (axis, position) in vertex.position.iter().copied().enumerate() {
            bounds[0][axis] = bounds[0][axis].min(position);
            bounds[1][axis] = bounds[1][axis].max(position);
        }
    }
    Ok((count, bounds))
}

impl GpuMesh {
    fn new(device: Arc<Device>, mesh: &Mesh, spare_capacity: bool) -> Result<Self> {
        let (index_count, bounds) = validate_mesh(mesh)?;
        let capacity = |size: usize| {
            if spare_capacity {
                size.checked_next_power_of_two().unwrap_or(size)
            } else {
                size
            }
        };
        // Allocation and writes complete before this object replaces a live mesh.
        let (vertices, indices) = if index_count == 0 {
            (None, None)
        } else {
            let vertices = bytemuck::cast_slice(&mesh.vertices);
            let indices = bytemuck::cast_slice(&mesh.indices);
            (
                Some(Buffer::with_capacity(
                    device.clone(),
                    vertices,
                    capacity(vertices.len()),
                    vk::BufferUsageFlags::VERTEX_BUFFER,
                )?),
                Some(Buffer::with_capacity(
                    device,
                    indices,
                    capacity(indices.len()),
                    vk::BufferUsageFlags::INDEX_BUFFER,
                )?),
            )
        };
        Ok(Self {
            vertices,
            indices,
            index_count,
            revision: mesh.revision,
            bounds,
        })
    }

    fn allocated_bytes(&self) -> usize {
        self.vertices.as_ref().map_or(0, |b| b.size) + self.indices.as_ref().map_or(0, |b| b.size)
    }

    /// Caller must complete the frame fence first. Both mappings succeed before
    /// either buffer changes, preserving a complete previous mesh on map failure.
    fn rewrite(&mut self, mesh: &Mesh) -> Result<bool> {
        let (count, bounds) = validate_mesh(mesh)?;
        if count == 0 {
            self.index_count = 0;
            self.revision = mesh.revision;
            self.bounds = bounds;
            return Ok(true);
        }
        let (Some(v), Some(i)) = (&self.vertices, &self.indices) else {
            return Ok(false);
        };
        let vertices = bytemuck::cast_slice::<_, u8>(&mesh.vertices);
        let indices = bytemuck::cast_slice::<_, u8>(&mesh.indices);
        if v.size < vertices.len() || i.size < indices.len() {
            return Ok(false);
        }
        // SAFETY: exclusive Renderer access after fence wait, distinct owned
        // coherent allocations, validated ranges, unmap both before submission.
        unsafe {
            let d = &v.device.raw;
            let vp = d
                .map_memory(
                    v.memory,
                    0,
                    vertices.len() as u64,
                    vk::MemoryMapFlags::empty(),
                )
                .map_err(err)?;
            let ip = match d.map_memory(
                i.memory,
                0,
                indices.len() as u64,
                vk::MemoryMapFlags::empty(),
            ) {
                Ok(p) => p,
                Err(e) => {
                    d.unmap_memory(v.memory);
                    return Err(err(e));
                }
            };
            std::ptr::copy_nonoverlapping(vertices.as_ptr(), vp.cast::<u8>(), vertices.len());
            std::ptr::copy_nonoverlapping(indices.as_ptr(), ip.cast::<u8>(), indices.len());
            d.unmap_memory(i.memory);
            d.unmap_memory(v.memory);
        }
        self.index_count = count;
        self.revision = mesh.revision;
        self.bounds = bounds;
        Ok(true)
    }
}
impl Drop for Buffer {
    fn drop(&mut self) {
        // SAFETY: Renderer waits for work before replacement/destruction.
        unsafe {
            self.device.raw.destroy_buffer(self.raw, None);
            self.device.raw.free_memory(self.memory, None);
        }
    }
}
struct Depth {
    device: Arc<Device>,
    image: vk::Image,
    memory: vk::DeviceMemory,
    view: vk::ImageView,
}
impl Depth {
    fn new(
        device: Arc<Device>,
        size: vk::Extent2D,
        format: vk::Format,
        sampled: bool,
    ) -> Result<Self> {
        let mut out = Self {
            device,
            image: vk::Image::null(),
            memory: vk::DeviceMemory::null(),
            view: vk::ImageView::null(),
        };
        // SAFETY: owned image, queried memory requirements, all partially created handles guarded.
        unsafe {
            out.image = out
                .device
                .raw
                .create_image(
                    &vk::ImageCreateInfo::default()
                        .image_type(vk::ImageType::TYPE_2D)
                        .format(format)
                        .extent(vk::Extent3D {
                            width: size.width,
                            height: size.height,
                            depth: 1,
                        })
                        .mip_levels(1)
                        .array_layers(1)
                        .samples(vk::SampleCountFlags::TYPE_1)
                        .tiling(vk::ImageTiling::OPTIMAL)
                        .usage(
                            vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT
                                | if sampled {
                                    vk::ImageUsageFlags::SAMPLED
                                } else {
                                    vk::ImageUsageFlags::empty()
                                },
                        )
                        .sharing_mode(vk::SharingMode::EXCLUSIVE),
                    None,
                )
                .map_err(err)?;
            let req = out.device.raw.get_image_memory_requirements(out.image);
            let ty = out
                .device
                .memory_type(req.memory_type_bits, vk::MemoryPropertyFlags::DEVICE_LOCAL)?;
            out.memory = out
                .device
                .raw
                .allocate_memory(
                    &vk::MemoryAllocateInfo::default()
                        .allocation_size(req.size)
                        .memory_type_index(ty),
                    None,
                )
                .map_err(err)?;
            out.device
                .raw
                .bind_image_memory(out.image, out.memory, 0)
                .map_err(err)?;
            out.view = out
                .device
                .raw
                .create_image_view(
                    &vk::ImageViewCreateInfo::default()
                        .image(out.image)
                        .view_type(vk::ImageViewType::TYPE_2D)
                        .format(format)
                        .subresource_range(
                            vk::ImageSubresourceRange::default()
                                .aspect_mask(vk::ImageAspectFlags::DEPTH)
                                .level_count(1)
                                .layer_count(1),
                        ),
                    None,
                )
                .map_err(err)?;
        }
        Ok(out)
    }
}
impl Drop for Depth {
    fn drop(&mut self) {
        // SAFETY: no pending command uses these handles.
        unsafe {
            self.device.raw.destroy_image_view(self.view, None);
            self.device.raw.destroy_image(self.image, None);
            self.device.raw.free_memory(self.memory, None);
        }
    }
}
struct Swapchain {
    device: Arc<Device>,
    api: ash::khr::swapchain::Device,
    raw: vk::SwapchainKHR,
    size: vk::Extent2D,
    views: Vec<vk::ImageView>,
    frames: Vec<vk::Framebuffer>,
    // A semaphore per acquired image: acquire proves that image's prior present wait completed.
    finished: Vec<vk::Semaphore>,
    depth: Option<Depth>,
    pass: vk::RenderPass,
    layout: vk::PipelineLayout,
    world: vk::Pipeline,
    hud: vk::Pipeline,
}
impl Drop for Swapchain {
    fn drop(&mut self) {
        // SAFETY: Renderer uses the standard unextended WSI idle-before-retirement
        // fallback. See README for the lack of a formal presentation-fence guarantee.
        unsafe {
            for f in self.frames.drain(..) {
                self.device.raw.destroy_framebuffer(f, None);
            }
            self.device.raw.destroy_pipeline(self.world, None);
            self.device.raw.destroy_pipeline(self.hud, None);
            self.device.raw.destroy_pipeline_layout(self.layout, None);
            self.device.raw.destroy_render_pass(self.pass, None);
            self.depth.take();
            for v in self.views.drain(..) {
                self.device.raw.destroy_image_view(v, None);
            }
            for s in self.finished.drain(..) {
                self.device.raw.destroy_semaphore(s, None);
            }
            self.api.destroy_swapchain(self.raw, None);
        }
    }
}
impl Swapchain {
    fn new(
        device: Arc<Device>,
        requested: vk::Extent2D,
        shadow_layout: vk::DescriptorSetLayout,
    ) -> Result<Self> {
        let api = ash::khr::swapchain::Device::new(&device.instance.raw, &device.raw);
        let mut out = Self {
            device,
            api,
            raw: vk::SwapchainKHR::null(),
            size: requested,
            views: vec![],
            frames: vec![],
            finished: vec![],
            depth: None,
            pass: vk::RenderPass::null(),
            layout: vk::PipelineLayout::null(),
            world: vk::Pipeline::null(),
            hud: vk::Pipeline::null(),
        };
        // SAFETY: surface/window and selected present queue are alive. Each created handle is
        // immediately recorded in the guard; attachment formats and extent are queried.
        unsafe {
            let d = &out.device;
            let i = &d.instance;
            let caps = i
                .surface_api
                .get_physical_device_surface_capabilities(d.physical, i.surface)
                .map_err(err)?;
            let formats = i
                .surface_api
                .get_physical_device_surface_formats(d.physical, i.surface)
                .map_err(err)?;
            let mut format = *formats
                .iter()
                .find(|f| {
                    [vk::Format::B8G8R8A8_SRGB, vk::Format::R8G8B8A8_SRGB].contains(&f.format)
                })
                .or(formats.first())
                .ok_or("Surface has no format")?;
            if format.format == vk::Format::UNDEFINED {
                format.format = vk::Format::B8G8R8A8_SRGB;
            }
            out.size = if caps.current_extent.width != u32::MAX {
                caps.current_extent
            } else {
                vk::Extent2D {
                    width: requested
                        .width
                        .clamp(caps.min_image_extent.width, caps.max_image_extent.width),
                    height: requested
                        .height
                        .clamp(caps.min_image_extent.height, caps.max_image_extent.height),
                }
            };
            if out.size.width == 0 || out.size.height == 0 {
                return Err("Surface has zero extent".into());
            }
            if !caps
                .supported_usage_flags
                .contains(vk::ImageUsageFlags::COLOR_ATTACHMENT)
            {
                return Err("Surface lacks color attachment usage".into());
            }
            // The app supplies window-oriented world and HUD coordinates. Ask the
            // compositor to handle display rotation; claiming current_transform
            // would require rotating both shaders and the image extent ourselves.
            if !caps
                .supported_transforms
                .contains(vk::SurfaceTransformFlagsKHR::IDENTITY)
            {
                return Err(
                    "Surface requires application pre-rotation; identity transform unavailable"
                        .into(),
                );
            }
            let alpha = [
                vk::CompositeAlphaFlagsKHR::OPAQUE,
                vk::CompositeAlphaFlagsKHR::PRE_MULTIPLIED,
                vk::CompositeAlphaFlagsKHR::POST_MULTIPLIED,
                vk::CompositeAlphaFlagsKHR::INHERIT,
            ]
            .into_iter()
            .find(|a| caps.supported_composite_alpha.contains(*a))
            .ok_or("No composite alpha mode")?;
            let count = if caps.max_image_count == 0 {
                caps.min_image_count.saturating_add(1)
            } else {
                caps.min_image_count
                    .saturating_add(1)
                    .min(caps.max_image_count)
            };
            out.raw = out
                .api
                .create_swapchain(
                    &vk::SwapchainCreateInfoKHR::default()
                        .surface(i.surface)
                        .min_image_count(count)
                        .image_format(format.format)
                        .image_color_space(format.color_space)
                        .image_extent(out.size)
                        .image_array_layers(1)
                        .image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT)
                        .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
                        .pre_transform(vk::SurfaceTransformFlagsKHR::IDENTITY)
                        .composite_alpha(alpha)
                        .present_mode(vk::PresentModeKHR::FIFO)
                        .clipped(true),
                    None,
                )
                .map_err(err)?;
            let depth_format = [vk::Format::D32_SFLOAT, vk::Format::D16_UNORM]
                .into_iter()
                .find(|f| {
                    i.raw
                        .get_physical_device_format_properties(d.physical, *f)
                        .optimal_tiling_features
                        .contains(vk::FormatFeatureFlags::DEPTH_STENCIL_ATTACHMENT)
                })
                .ok_or("No depth attachment format")?;
            out.depth = Some(Depth::new(d.clone(), out.size, depth_format, false)?);
            let attachments = [
                vk::AttachmentDescription::default()
                    .format(format.format)
                    .samples(vk::SampleCountFlags::TYPE_1)
                    .load_op(vk::AttachmentLoadOp::CLEAR)
                    .store_op(vk::AttachmentStoreOp::STORE)
                    .initial_layout(vk::ImageLayout::UNDEFINED)
                    .final_layout(vk::ImageLayout::PRESENT_SRC_KHR),
                vk::AttachmentDescription::default()
                    .format(depth_format)
                    .samples(vk::SampleCountFlags::TYPE_1)
                    .load_op(vk::AttachmentLoadOp::CLEAR)
                    .store_op(vk::AttachmentStoreOp::DONT_CARE)
                    .initial_layout(vk::ImageLayout::UNDEFINED)
                    .final_layout(vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL),
            ];
            let color = [vk::AttachmentReference {
                attachment: 0,
                layout: vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
            }];
            let depth = vk::AttachmentReference {
                attachment: 1,
                layout: vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL,
            };
            let subpasses = [vk::SubpassDescription::default()
                .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
                .color_attachments(&color)
                .depth_stencil_attachment(&depth)];
            let stages = vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT
                | vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS
                | vk::PipelineStageFlags::LATE_FRAGMENT_TESTS;
            let dependencies = [vk::SubpassDependency::default()
                .src_subpass(vk::SUBPASS_EXTERNAL)
                .dst_subpass(0)
                .src_stage_mask(stages)
                .dst_stage_mask(stages)
                .src_access_mask(vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE)
                .dst_access_mask(
                    vk::AccessFlags::COLOR_ATTACHMENT_WRITE
                        | vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE,
                )];
            out.pass = d
                .raw
                .create_render_pass(
                    &vk::RenderPassCreateInfo::default()
                        .attachments(&attachments)
                        .subpasses(&subpasses)
                        .dependencies(&dependencies),
                    None,
                )
                .map_err(err)?;
            let push = [vk::PushConstantRange::default()
                .stage_flags(vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT)
                .offset(0)
                .size(80)];
            out.layout = d
                .raw
                .create_pipeline_layout(
                    &vk::PipelineLayoutCreateInfo::default()
                        .push_constant_ranges(&push)
                        .set_layouts(&[shadow_layout]),
                    None,
                )
                .map_err(err)?;
            out.world = pipeline(&out.device, out.layout, out.pass, PipelineKind::World)?;
            out.hud = pipeline(&out.device, out.layout, out.pass, PipelineKind::Hud)?;
            for image in out.api.get_swapchain_images(out.raw).map_err(err)? {
                let view = d
                    .raw
                    .create_image_view(
                        &vk::ImageViewCreateInfo::default()
                            .image(image)
                            .view_type(vk::ImageViewType::TYPE_2D)
                            .format(format.format)
                            .subresource_range(
                                vk::ImageSubresourceRange::default()
                                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                                    .level_count(1)
                                    .layer_count(1),
                            ),
                        None,
                    )
                    .map_err(err)?;
                out.views.push(view);
                let views = [view, out.depth.as_ref().expect("depth created").view];
                out.frames.push(
                    d.raw
                        .create_framebuffer(
                            &vk::FramebufferCreateInfo::default()
                                .render_pass(out.pass)
                                .attachments(&views)
                                .width(out.size.width)
                                .height(out.size.height)
                                .layers(1),
                            None,
                        )
                        .map_err(err)?,
                );
                out.finished.push(
                    d.raw
                        .create_semaphore(&Default::default(), None)
                        .map_err(err)?,
                );
            }
        }
        Ok(out)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PipelineKind {
    World,
    Hud,
    Shadow,
}
fn pipeline(
    device: &Device,
    layout: vk::PipelineLayout,
    pass: vk::RenderPass,
    kind: PipelineKind,
) -> Result<vk::Pipeline> {
    let hud = kind == PipelineKind::Hud;
    let shadow = kind == PipelineKind::Shadow;
    let (vs, fs): (&[u8], &[u8]) = if hud {
        (
            include_bytes!(concat!(env!("OUT_DIR"), "/hud.vs_main.spv")),
            include_bytes!(concat!(env!("OUT_DIR"), "/hud.fs_main.spv")),
        )
    } else if shadow {
        (
            include_bytes!(concat!(env!("OUT_DIR"), "/shadow.vs_main.spv")),
            &[],
        )
    } else {
        (
            include_bytes!(concat!(env!("OUT_DIR"), "/world.vs_main.spv")),
            include_bytes!(concat!(env!("OUT_DIR"), "/world.fs_main.spv")),
        )
    };
    let mut modules = Vec::new();
    // SAFETY: build-time Naga validates SPIR-V; modules outlive pipeline creation, then are
    // destroyed on success and error. Struct slices stay in scope during synchronous calls.
    unsafe {
        let result =
            (|| {
                for bytes in [vs, fs].into_iter().filter(|b| !b.is_empty()) {
                    let words = ash::util::read_spv(&mut std::io::Cursor::new(bytes))
                        .map_err(|e| e.to_string())?;
                    modules.push(
                        device
                            .raw
                            .create_shader_module(
                                &vk::ShaderModuleCreateInfo::default().code(&words),
                                None,
                            )
                            .map_err(err)?,
                    );
                }
                let mut stages = vec![vk::PipelineShaderStageCreateInfo::default()
                    .stage(vk::ShaderStageFlags::VERTEX)
                    .module(modules[0])
                    .name(c"vs_main")];
                if !shadow {
                    stages.push(
                        vk::PipelineShaderStageCreateInfo::default()
                            .stage(vk::ShaderStageFlags::FRAGMENT)
                            .module(modules[1])
                            .name(c"fs_main"),
                    );
                }
                let mut bindings = vec![vk::VertexInputBindingDescription {
                    binding: 0,
                    stride: if hud { 24 } else { 36 },
                    input_rate: vk::VertexInputRate::VERTEX,
                }];
                // World and shadow pipelines read one packed instance record
                // (translation xyz, quarter yaw) per instance at location 3.
                // Legacy/chunk/dynamic draws bind a single identity record.
                if !hud {
                    bindings.push(vk::VertexInputBindingDescription {
                        binding: 1,
                        stride: 16,
                        input_rate: vk::VertexInputRate::INSTANCE,
                    });
                }
                let mut attributes: Vec<_> = if hud {
                    vec![
                        vk::VertexInputAttributeDescription {
                            location: 0,
                            binding: 0,
                            format: vk::Format::R32G32_SFLOAT,
                            offset: 0,
                        },
                        vk::VertexInputAttributeDescription {
                            location: 1,
                            binding: 0,
                            format: vk::Format::R32G32B32A32_SFLOAT,
                            offset: 8,
                        },
                    ]
                } else {
                    (0..if shadow { 1 } else { 3 })
                        .map(|location| vk::VertexInputAttributeDescription {
                            location,
                            binding: 0,
                            format: vk::Format::R32G32B32_SFLOAT,
                            offset: location * 12,
                        })
                        .collect()
                };
                if !hud {
                    attributes.push(vk::VertexInputAttributeDescription {
                        location: 3,
                        binding: 1,
                        format: vk::Format::R32G32B32A32_SFLOAT,
                        offset: 0,
                    });
                }
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
                    .cull_mode(if hud || shadow {
                        vk::CullModeFlags::NONE
                    } else {
                        vk::CullModeFlags::BACK
                    })
                    .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
                    .line_width(1.0);
                let multisample = vk::PipelineMultisampleStateCreateInfo::default()
                    .rasterization_samples(vk::SampleCountFlags::TYPE_1);
                let depth = vk::PipelineDepthStencilStateCreateInfo::default()
                    .depth_test_enable(!hud)
                    .depth_write_enable(!hud)
                    .depth_compare_op(vk::CompareOp::LESS);
                let attachments = [vk::PipelineColorBlendAttachmentState::default()
                    .blend_enable(hud)
                    .src_color_blend_factor(vk::BlendFactor::SRC_ALPHA)
                    .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
                    .color_blend_op(vk::BlendOp::ADD)
                    .src_alpha_blend_factor(vk::BlendFactor::ONE)
                    .dst_alpha_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
                    .alpha_blend_op(vk::BlendOp::ADD)
                    .color_write_mask(vk::ColorComponentFlags::RGBA)];
                let blend = vk::PipelineColorBlendStateCreateInfo::default()
                    .attachments(if shadow { &[] } else { &attachments });
                let dynamic_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
                let dynamic =
                    vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic_states);
                let infos = [vk::GraphicsPipelineCreateInfo::default()
                    .stages(&stages)
                    .vertex_input_state(&vertex)
                    .input_assembly_state(&assembly)
                    .viewport_state(&viewport)
                    .rasterization_state(&raster)
                    .multisample_state(&multisample)
                    .depth_stencil_state(&depth)
                    .color_blend_state(&blend)
                    .dynamic_state(&dynamic)
                    .layout(layout)
                    .render_pass(pass)
                    .subpass(0)];
                match device
                    .raw
                    .create_graphics_pipelines(vk::PipelineCache::null(), &infos, None)
                {
                    Ok(p) => Ok(p[0]),
                    Err((partial, e)) => {
                        for p in partial {
                            device.raw.destroy_pipeline(p, None);
                        }
                        Err(err(e))
                    }
                }
            })();
        for module in modules {
            device.raw.destroy_shader_module(module, None);
        }
        result
    }
}
struct Commands {
    device: Arc<Device>,
    pool: vk::CommandPool,
    buffer: vk::CommandBuffer,
    fence: vk::Fence,
    available: vk::Semaphore,
}
impl Commands {
    fn new(device: Arc<Device>) -> Result<Self> {
        let mut out = Self {
            device,
            pool: vk::CommandPool::null(),
            buffer: vk::CommandBuffer::null(),
            fence: vk::Fence::null(),
            available: vk::Semaphore::null(),
        };
        // SAFETY: exclusive command pool and single frame in flight, guard handles partial creation.
        unsafe {
            out.pool = out
                .device
                .raw
                .create_command_pool(
                    &vk::CommandPoolCreateInfo::default()
                        .queue_family_index(out.device.family)
                        .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER),
                    None,
                )
                .map_err(err)?;
            out.buffer = out
                .device
                .raw
                .allocate_command_buffers(
                    &vk::CommandBufferAllocateInfo::default()
                        .command_pool(out.pool)
                        .level(vk::CommandBufferLevel::PRIMARY)
                        .command_buffer_count(1),
                )
                .map_err(err)?[0];
            out.fence = out
                .device
                .raw
                .create_fence(
                    &vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED),
                    None,
                )
                .map_err(err)?;
            out.available = out
                .device
                .raw
                .create_semaphore(&Default::default(), None)
                .map_err(err)?;
        }
        Ok(out)
    }
    fn wait(&self) -> Result<()> {
        // SAFETY: fence belongs to this device, reset only immediately before submission.
        unsafe {
            self.device
                .raw
                .wait_for_fences(&[self.fence], true, u64::MAX)
                .map_err(err)
        }
    }
}
impl Drop for Commands {
    fn drop(&mut self) {
        // SAFETY: Renderer idles work first; pool owns its command buffer.
        unsafe {
            self.device.raw.destroy_command_pool(self.pool, None);
            self.device.raw.destroy_fence(self.fence, None);
            self.device.raw.destroy_semaphore(self.available, None);
        }
    }
}

/// Opt-in per-frame CPU-side diagnostics. These are wall-clock waits at the real
/// API call boundaries, not GPU execution time, and never a presentation
/// (scanout) timestamp. Absent values mean the boundary was not reached or
/// diagnostics are disabled.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DrawDiagnostics {
    /// GPU submission identity produced by this draw attempt, if it submitted.
    /// Matches `GpuTimings::frame_id` for the same submission. A draw can submit
    /// and still report `Retry` when the presentation request is out of date.
    pub submitted_frame_id: Option<u64>,
    /// Summed blocking fence waits inside the upload/retain calls made since
    /// `begin_frame_diagnostics`, at their own call sites. These run before the
    /// draw and are normally where the frame actually blocks.
    pub upload_fence_wait_ms: Option<f64>,
    /// Number of upload/retain fence waits summed above (0 when none ran).
    pub upload_fence_waits: Option<u32>,
    /// Blocking wait on the submission fence inside the draw itself. It is
    /// usually already signalled by the upload waits above, so this is not the
    /// frame's total fence wait.
    pub render_fence_wait_ms: Option<f64>,
    /// `vkAcquireNextImageKHR` call duration.
    pub acquire_ms: Option<f64>,
    /// `vkQueuePresentKHR` call duration: queueing the request, not scanout.
    pub present_ms: Option<f64>,
}

/// Accumulates fence waits at call sites outside the draw. Enabled state is
/// explicit: while disabled it never reads the clock and reports nothing, which
/// keeps "no waits" (`Some(0)`) distinct from "not measured" (`None`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct WaitTally {
    enabled: bool,
    waits: u32,
    total_ms: f64,
}

impl WaitTally {
    fn reset(&mut self, enabled: bool) {
        *self = Self {
            enabled,
            waits: 0,
            total_ms: 0.,
        };
    }
    fn timed_begin(&self) -> Option<Instant> {
        self.enabled.then(Instant::now)
    }
    fn record(&mut self, begin: Option<Instant>) {
        if let Some(begin) = begin {
            self.waits += 1;
            self.total_ms += elapsed_ms(begin);
        }
    }
    fn total(&self) -> Option<f64> {
        self.enabled.then_some(self.total_ms)
    }
    fn count(&self) -> Option<u32> {
        self.enabled.then_some(self.waits)
    }
}

/// Result of the presentation request, independent of the submission that
/// preceded it. An out-of-date swapchain is not a successful presentation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PresentOutcome {
    Presented { recreate: bool },
    OutOfDate,
    Failed(vk::Result),
}

fn classify_present(
    result: std::result::Result<bool, vk::Result>,
    _acquire_suboptimal: bool,
) -> PresentOutcome {
    match result {
        // SUBOPTIMAL is advisory: the swapchain still presents successfully.
        // Android can report it persistently with compositor-managed rotation.
        // Defer recreation to explicit resize or OUT_OF_DATE; rebuilding here
        // recreates pipelines every frame without resolving that advisory.
        Ok(_) => PresentOutcome::Presented { recreate: false },
        Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => PresentOutcome::OutOfDate,
        Err(e) => PresentOutcome::Failed(e),
    }
}

pub struct Renderer {
    material_time: Option<f32>,
    world_visible: bool,
    device: Arc<Device>,
    commands: Commands,
    swapchain: Option<Swapchain>,
    shadow: Shadow,
    shadow_map_updated: bool,
    timestamps: Option<TimestampQueries>,
    diagnostics_enabled: bool,
    diagnostics: DrawDiagnostics,
    upload_waits: WaitTally,
    submissions: u64,
    legacy: Option<GpuMesh>,
    chunks: BTreeMap<[i32; 3], GpuMesh>,
    dynamic: Option<GpuMesh>,
    static_scene: Option<StaticScene>,
    // One zeroed instance record: identity transform for non-instanced draws.
    identity: Buffer,
    hud: Option<Buffer>,
    requested: vk::Extent2D,
    recreate: bool,
    pub mesh_revision: Option<u64>,
    pub capabilities: String,
    /// Allocated vertex/index buffer capacity; excludes HUD, depth and driver overhead.
    pub mesh_bytes: usize,
    pub visible_chunks: usize,
    pub resident_chunks: usize,
}
impl Renderer {
    pub async fn new(window: Arc<Window>) -> Result<Self> {
        let size = window.inner_size();
        let display = window.display_handle().map_err(|e| e.to_string())?.as_raw();
        let handle = window.window_handle().map_err(|e| e.to_string())?.as_raw();
        // SAFETY: load system Vulkan loader, retained by Instance until all Vulkan children die.
        let entry = unsafe { Entry::load() }.map_err(|e| format!("Vulkan loader: {e}"))?;
        let mut extensions = ash_window::enumerate_required_extensions(display)
            .map_err(err)?
            .to_vec();
        // SAFETY: loader enumeration has no object lifetime prerequisites.
        let (layers, instance_extensions) = unsafe {
            (
                entry.enumerate_instance_layer_properties().map_err(err)?,
                entry
                    .enumerate_instance_extension_properties(None)
                    .map_err(err)?,
            )
        };
        let layer_available = layers.iter().any(
            |l| unsafe { CStr::from_ptr(l.layer_name.as_ptr()) } == c"VK_LAYER_KHRONOS_validation",
        );
        let debug_available = instance_extensions.iter().any(
            |e| unsafe { CStr::from_ptr(e.extension_name.as_ptr()) } == ash::ext::debug_utils::NAME,
        );
        let validation_enabled = std::env::var("MATTERWEAVE_VALIDATION")
            .map_or(cfg!(debug_assertions), |v| v != "0")
            && layer_available;
        let layer_names = if validation_enabled {
            vec![c"VK_LAYER_KHRONOS_validation".as_ptr()]
        } else {
            vec![]
        };
        if validation_enabled && debug_available {
            extensions.push(ash::ext::debug_utils::NAME.as_ptr());
        }
        let app = vk::ApplicationInfo::default()
            .application_name(c"Matterweave Explorer")
            .application_version(1)
            .engine_name(c"Matterweave")
            .engine_version(1)
            .api_version(vk::API_VERSION_1_1);
        // SAFETY: pointer arrays and app names remain alive throughout create_instance.
        let raw = unsafe {
            entry.create_instance(
                &vk::InstanceCreateInfo::default()
                    .application_info(&app)
                    .enabled_extension_names(&extensions)
                    .enabled_layer_names(&layer_names),
                None,
            )
        }
        .map_err(err)?;
        let surface_api = ash::khr::surface::Instance::new(&entry, &raw);
        let mut instance = Instance {
            _entry: entry,
            _window: window,
            raw,
            surface_api,
            surface: vk::SurfaceKHR::null(),
            debug_api: None,
            debug: vk::DebugUtilsMessengerEXT::null(),
        };
        // SAFETY: Arc owns native window until Instance drop, valid paired winit handles.
        unsafe {
            instance.surface =
                ash_window::create_surface(&instance._entry, &instance.raw, display, handle, None)
                    .map_err(err)?;
            if validation_enabled && debug_available {
                let api = ash::ext::debug_utils::Instance::new(&instance._entry, &instance.raw);
                instance.debug = api
                    .create_debug_utils_messenger(
                        &vk::DebugUtilsMessengerCreateInfoEXT::default()
                            .message_severity(
                                vk::DebugUtilsMessageSeverityFlagsEXT::WARNING
                                    | vk::DebugUtilsMessageSeverityFlagsEXT::ERROR,
                            )
                            .message_type(
                                vk::DebugUtilsMessageTypeFlagsEXT::GENERAL
                                    | vk::DebugUtilsMessageTypeFlagsEXT::VALIDATION
                                    | vk::DebugUtilsMessageTypeFlagsEXT::PERFORMANCE,
                            )
                            .pfn_user_callback(Some(validation)),
                        None,
                    )
                    .map_err(err)?;
                instance.debug_api = Some(api);
            }
        }
        let instance = Arc::new(instance);
        let mut selected = None;
        // SAFETY: queried handles belong to live instance; feature/extension requirements checked.
        unsafe {
            for physical in instance.raw.enumerate_physical_devices().map_err(err)? {
                let props = instance.raw.get_physical_device_properties(physical);
                if props.api_version < vk::API_VERSION_1_1 {
                    continue;
                }
                let ext = instance
                    .raw
                    .enumerate_device_extension_properties(physical)
                    .map_err(err)?;
                if !ext
                    .iter()
                    .any(|e| CStr::from_ptr(e.extension_name.as_ptr()) == ash::khr::swapchain::NAME)
                {
                    continue;
                }
                for (family, q) in instance
                    .raw
                    .get_physical_device_queue_family_properties(physical)
                    .iter()
                    .enumerate()
                {
                    if q.queue_flags.contains(vk::QueueFlags::GRAPHICS)
                        && instance
                            .surface_api
                            .get_physical_device_surface_support(
                                physical,
                                family as u32,
                                instance.surface,
                            )
                            .map_err(err)?
                    {
                        selected = Some((physical, family as u32, props));
                        break;
                    }
                }
                if selected.is_some() {
                    break;
                }
            }
        }
        let (physical, family, props) =
            selected.ok_or("No Vulkan 1.1 graphics/present device with VK_KHR_swapchain")?;
        let priorities = [1.0];
        let queues = [vk::DeviceQueueCreateInfo::default()
            .queue_family_index(family)
            .queue_priorities(&priorities)];
        let extensions = [ash::khr::swapchain::NAME.as_ptr()];
        // SAFETY: selected queue family and required extension exist. No optional features requested.
        let raw = unsafe {
            instance.raw.create_device(
                physical,
                &vk::DeviceCreateInfo::default()
                    .queue_create_infos(&queues)
                    .enabled_extension_names(&extensions),
                None,
            )
        }
        .map_err(err)?;
        // SAFETY: queue zero requested above; physical device is live.
        let (queue, memory) = unsafe {
            (
                raw.get_device_queue(family, 0),
                instance.raw.get_physical_device_memory_properties(physical),
            )
        };
        let device = Arc::new(Device {
            instance,
            raw,
            physical,
            queue,
            family,
            memory,
        });
        // SAFETY: Vulkan returns a nul-terminated fixed-size device name.
        let name = unsafe { CStr::from_ptr(props.device_name.as_ptr()) }.to_string_lossy();
        let heaps: Vec<String> = memory.memory_heaps[..memory.memory_heap_count as usize]
            .iter()
            .map(|h| format!("{}MiB:{:?}", h.size / (1024 * 1024), h.flags))
            .collect();
        let capabilities=format!("Vulkan {}.{}.{} | {} | driver {} | vendor {:04x} device {:04x} | heaps {} | validation {}",vk::api_version_major(props.api_version),vk::api_version_minor(props.api_version),vk::api_version_patch(props.api_version),name,props.driver_version,props.vendor_id,props.device_id,heaps.join(","),validation_enabled);
        let commands = Commands::new(device.clone())?;
        let shadow = Shadow::new(device.clone(), LightingSettings::default().shadow_map_size)?;
        let timestamps = TimestampQueries::new(device.clone())?;
        // One zeroed packed instance record: identity transform fallback so the
        // legacy/chunk/dynamic paths keep rendering unchanged through the
        // instanced vertex pipeline.
        let identity = Buffer::new(
            device.clone(),
            &[0u8; 16],
            vk::BufferUsageFlags::VERTEX_BUFFER,
        )?;
        Ok(Self {
            material_time: None,
            world_visible: true,
            device,
            commands,
            swapchain: None,
            shadow,
            shadow_map_updated: false,
            timestamps,
            diagnostics_enabled: false,
            diagnostics: DrawDiagnostics::default(),
            upload_waits: WaitTally::default(),
            submissions: 0,
            legacy: None,
            chunks: BTreeMap::new(),
            dynamic: None,
            static_scene: None,
            identity,
            hud: None,
            requested: vk::Extent2D {
                width: size.width,
                height: size.height,
            },
            recreate: true,
            mesh_revision: None,
            capabilities,
            mesh_bytes: 0,
            visible_chunks: 0,
            resident_chunks: 0,
        })
    }
    pub fn resize(&mut self, width: u32, height: u32) {
        self.requested = vk::Extent2D { width, height };
        self.recreate = true;
    }
    /// Compatibility whole-world path. A successful upload replaces cached chunks.
    pub fn upload(&mut self, mesh: &Mesh) -> Result<()> {
        if self.mesh_revision.is_some_and(|r| r > mesh.revision) {
            return Ok(());
        }
        let wait = self.upload_waits.timed_begin();
        self.commands.wait()?;
        self.upload_waits.record(wait);
        let replacement = GpuMesh::new(self.device.clone(), mesh, false)?;
        self.legacy = Some(replacement);
        self.chunks.clear();
        self.mesh_revision = Some(mesh.revision);
        self.update_counters();
        Ok(())
    }

    /// World-space geometry, normally one 16-cubed voxel chunk. Empty meshes
    /// retain their revision so occluded chunks are not rebuilt every frame.
    /// Older revisions are ignored. A successful upload switches off legacy mesh.
    pub fn upload_chunk(&mut self, key: [i32; 3], mesh: &Mesh) -> Result<()> {
        if self
            .chunk_revision(key)
            .is_some_and(|revision| revision > mesh.revision)
        {
            return Ok(());
        }
        let wait = self.upload_waits.timed_begin();
        self.commands.wait()?;
        self.upload_waits.record(wait);
        let replacement = GpuMesh::new(self.device.clone(), mesh, false)?;
        self.chunks.insert(key, replacement);
        self.legacy = None;
        self.mesh_revision = None;
        self.update_counters();
        Ok(())
    }

    pub fn chunk_revision(&self, key: [i32; 3]) -> Option<u64> {
        self.chunks.get(&key).map(|mesh| mesh.revision)
    }

    /// Forget evicted chunks only after all draws referencing them complete.
    /// Callers must discard stale jobs for evicted chunks before uploading them.
    pub fn retain_chunks(&mut self, keys: &[[i32; 3]]) -> Result<()> {
        let keep: std::collections::BTreeSet<_> = keys.iter().copied().collect();
        if self.chunks.keys().any(|key| !keep.contains(key)) {
            let wait = self.upload_waits.timed_begin();
            self.commands.wait()?;
            self.upload_waits.record(wait);
            self.chunks.retain(|key, _| keep.contains(key));
            self.update_counters();
        }
        Ok(())
    }

    /// CPU-transformed world-space objects. Reuses coherent buffer capacity after
    /// the frame fence; grows transactionally when geometry exceeds capacity.
    /// Revision is informational here: changing transforms may retain a revision.
    pub fn upload_dynamic(&mut self, mesh: &Mesh) -> Result<()> {
        let wait = self.upload_waits.timed_begin();
        self.commands.wait()?;
        self.upload_waits.record(wait);
        // In-place writes can change positions while retaining the revision.
        // Invalidate before any write, also covering a partial write failure.
        self.shadow.invalidate();
        if let Some(dynamic) = &mut self.dynamic {
            if dynamic.rewrite(mesh)? {
                return Ok(());
            }
        }
        self.dynamic = Some(GpuMesh::new(self.device.clone(), mesh, true)?);
        self.update_counters();
        Ok(())
    }

    /// Atomically replaces the instanced static scene: unique prototype
    /// geometry is pooled into shared vertex/index buffers plus one packed
    /// instance buffer, and each prototype is drawn as one batch carrying all
    /// of its instances. An empty instance list clears the scene. Validation
    /// and budget checks (128 MiB geometry, 16 MiB instances) run before any
    /// allocation; on any error, including allocation failure mid-build, the
    /// previous scene is retained unchanged. Callers should treat this as a
    /// load/edit-time operation, not a per-frame path.
    pub fn replace_static_scene(
        &mut self,
        meshes: &[Mesh],
        instances: &[StaticInstance],
    ) -> Result<StaticSceneStats> {
        // Host-side planning validates everything before touching the device or
        // the live scene; an error here retains the previous scene untouched.
        let plan = static_scene::plan_static_scene(meshes, instances)?;
        let wait = self.upload_waits.timed_begin();
        self.commands.wait()?;
        self.upload_waits.record(wait);
        if plan.instance_count == 0 {
            self.static_scene = None;
            self.update_counters();
            return Ok(StaticSceneStats::default());
        }
        // Construction is transactional: all buffers exist before the swap, so
        // a failed allocation drops only the partial build. The fence wait above
        // guarantees no submitted frame references the retired scene.
        let scene = StaticScene::new(self.device.clone(), plan)?;
        let stats = scene.stats();
        self.static_scene = Some(scene);
        self.update_counters();
        Ok(stats)
    }

    /// Honest accounting of the currently resident static scene, if any.
    pub fn static_scene_stats(&self) -> Option<StaticSceneStats> {
        self.static_scene.as_ref().map(|scene| scene.stats())
    }

    fn update_counters(&mut self) {
        // Every committed geometry replacement/removal comes through here.
        // Rejected transactional uploads leave both geometry and validity intact.
        self.shadow.invalidate();
        self.resident_chunks = self.chunks.len();
        self.visible_chunks = self.visible_chunks.min(self.resident_chunks);
        self.mesh_bytes = self
            .chunks
            .values()
            .chain(self.legacy.iter())
            .chain(self.dynamic.iter())
            .map(GpuMesh::allocated_bytes)
            .sum::<usize>()
            // Pooled static geometry capacity; per-buffer and instance-buffer
            // capacities are broken out in StaticSceneStats.
            + self
                .static_scene
                .as_ref()
                .map_or(0, |scene| scene.allocated_bytes);
    }

    /// Suspend world draws behind an opaque menu while retaining GPU resources.
    /// HUD/presentation continue; no world or shadow draw is submitted while hidden.
    pub fn set_world_visible(&mut self, visible: bool) {
        self.world_visible = visible;
    }

    /// Opt-in wetland material response. The phase wraps continuously for both
    /// ripple frequencies; legacy rendering keeps its original material response.
    pub fn set_wetland_material_time(&mut self, seconds: Option<f32>) -> Result<()> {
        if seconds.is_some_and(|s| !s.is_finite()) {
            return Err("Material time must be finite".into());
        }
        self.material_time = seconds.map(|s| s.rem_euclid(std::f32::consts::TAU * 10.));
        Ok(())
    }

    pub fn render(&mut self, view_proj: [[f32; 4]; 4], eye: [f32; 3], hud: &Hud) -> FrameResult {
        self.render_with_lighting(
            view_proj,
            eye,
            hud,
            &LightingSettings {
                shadows: false,
                ..Default::default()
            },
        )
    }

    /// Publish a current CPU indirect cache after uploading its matching geometry.
    /// The caller supplies the current authoritative World and replacement epoch,
    /// never a job's old snapshot. Rejected stale data disables previous output.
    /// All geometry uploads and sun changes disable GI until republished; shadow
    /// resource replacement also disables it. Unit World voxels only, opt-in.
    pub fn upload_indirect(
        &mut self,
        volume: &indirect::IndirectVolume,
        world: &matterweave_core::World,
        source_epoch: u64,
    ) -> Result<()> {
        self.shadow.disable_indirect();
        if self.dynamic.as_ref().is_some_and(|m| m.index_count != 0)
            || self.static_scene.as_ref().is_some_and(|s| s.has_geometry)
        {
            return Err("Indirect World cache does not cover mesh-only objects/instances".into());
        }
        if !volume.source_valid(world, source_epoch) {
            return Err("Stale indirect source revision/epoch".into());
        }
        self.commands.wait()?;
        self.shadow.upload_indirect(volume)
    }

    pub fn disable_indirect(&mut self) {
        self.shadow.disable_indirect();
    }

    /// Current validity, not a GPU timing or proof of nonzero pixel contribution.
    pub fn indirect_enabled(&self) -> bool {
        self.shadow.indirect_enabled()
    }

    /// Sunlight and shadow settings apply to this frame. Invalid settings return Fatal.
    pub fn render_with_lighting(
        &mut self,
        view_proj: [[f32; 4]; 4],
        eye: [f32; 3],
        hud: &Hud,
        lighting: &LightingSettings,
    ) -> FrameResult {
        let mut effective = *lighting;
        effective.shadows &= self.world_visible;
        match self.draw(view_proj, eye, hud, &effective) {
            Ok(result) => result,
            Err(e) => {
                if e.contains("ERROR_OUT_OF_HOST_MEMORY")
                    || e.contains("ERROR_OUT_OF_DEVICE_MEMORY")
                {
                    FrameResult::OutOfMemory
                } else {
                    FrameResult::Fatal(e)
                }
            }
        }
    }
    /// Most recently completed submission, normally the preceding presented frame.
    pub fn gpu_timings(&self) -> Option<GpuTimings> {
        self.timestamps.as_ref().and_then(|q| q.completed)
    }

    pub fn gpu_timestamps_supported(&self) -> bool {
        self.timestamps.is_some()
    }

    /// Enables per-frame CPU wait diagnostics. Off by default: normal operation
    /// adds no clock readings beyond the pre-existing ones.
    pub fn set_diagnostics_enabled(&mut self, enabled: bool) {
        self.diagnostics_enabled = enabled;
        if !enabled {
            self.diagnostics = DrawDiagnostics::default();
            self.upload_waits.reset(false);
        }
    }

    /// Starts one frame's diagnostics. Callers must invoke this before the
    /// frame's upload/retain calls so waits from an earlier frame cannot leak
    /// into this row.
    pub fn begin_frame_diagnostics(&mut self) {
        self.diagnostics = DrawDiagnostics::default();
        self.upload_waits.reset(self.diagnostics_enabled);
    }

    /// Diagnostics for the current frame; `None` while disabled.
    pub fn draw_diagnostics(&self) -> Option<DrawDiagnostics> {
        self.diagnostics_enabled.then(|| DrawDiagnostics {
            upload_fence_wait_ms: self.upload_waits.total(),
            upload_fence_waits: self.upload_waits.count(),
            ..self.diagnostics
        })
    }

    /// Whether this draw attempt submitted a depth-map update (including first-use
    /// clear with shadows disabled). False on reuse or before submission.
    pub fn shadow_map_updated(&self) -> bool {
        self.shadow_map_updated
    }

    /// Nonempty mesh draws recorded for this attempt, independent of camera culling.
    /// Zero when the shadow map is reused or shadows are disabled.
    pub fn shadow_caster_meshes(&self) -> usize {
        self.shadow.caster_meshes
    }

    fn draw(
        &mut self,
        view_proj: [[f32; 4]; 4],
        eye: [f32; 3],
        hud: &Hud,
        lighting: &LightingSettings,
    ) -> Result<FrameResult> {
        self.shadow_map_updated = false;
        // Clear this draw's own fields; upload waits recorded since
        // begin_frame_diagnostics belong to the same frame and are preserved.
        self.diagnostics = DrawDiagnostics {
            submitted_frame_id: None,
            render_fence_wait_ms: None,
            acquire_ms: None,
            present_ms: None,
            ..self.diagnostics
        };
        if self.requested.width == 0 || self.requested.height == 0 {
            return Ok(FrameResult::Retry);
        }
        let fence_begin = self.diagnostics_enabled.then(Instant::now);
        self.commands.wait()?;
        self.diagnostics.render_fence_wait_ms = fence_begin.map(elapsed_ms);
        if let Some(timestamps) = &mut self.timestamps {
            timestamps.read_completed()?;
        }
        if lighting.shadow_map_size != self.shadow.size {
            // Identical descriptor layout definitions remain pipeline-compatible.
            // Construct replacement transactionally, then retire the idle old map.
            self.shadow = Shadow::new(self.device.clone(), lighting.shadow_map_size)?;
        }
        let mut bounds: Vec<_> = self
            .chunks
            .values()
            .chain(self.legacy.iter())
            .chain(self.dynamic.iter())
            .filter(|m| self.world_visible && m.index_count > 0)
            .map(|m| m.bounds)
            .collect();
        // Shadow depth fitting must include the instanced scene bounds so
        // offscreen static casters stay inside the map.
        if let Some(scene) = &self.static_scene {
            if self.world_visible && scene.has_geometry {
                bounds.push(scene.bounds);
            }
        }
        self.shadow.update(eye, lighting, &bounds)?;
        if self.recreate {
            // SAFETY: exceptional resize/retirement only. This is the standard
            // unextended WSI idle fallback; its presentation-completion limitation
            // and the deferred maintenance-extension path are documented in README.
            unsafe {
                self.device.raw.device_wait_idle().map_err(err)?;
            }
            self.swapchain.take();
            self.swapchain =
                match Swapchain::new(self.device.clone(), self.requested, self.shadow.set_layout) {
                    Ok(swapchain) => Some(swapchain),
                    Err(e)
                        if e == "Surface has zero extent"
                            || e.contains("ERROR_OUT_OF_DATE_KHR") =>
                    {
                        return Ok(FrameResult::Retry)
                    }
                    Err(e) => return Err(e),
                };
            self.recreate = false;
        }
        let bytes = bytemuck::cast_slice(&hud.vertices);
        if self.hud.as_ref().is_none_or(|b| b.size < bytes.len()) {
            self.hud = Some(Buffer::new(
                self.device.clone(),
                bytes,
                vk::BufferUsageFlags::VERTEX_BUFFER,
            )?);
        } else {
            self.hud.as_ref().expect("HUD allocated").write(bytes)?;
        }
        let s = self.swapchain.as_ref().expect("swapchain created");
        let hud_count =
            u32::try_from(hud.vertices.len()).map_err(|_| "HUD exceeds u32 vertex count")?;
        let d = &self.device.raw;
        let cmd = self.commands.buffer;
        // SAFETY: fence above completed previous command buffer and coherent writes. Acquire's
        // binary semaphore is consumed by exactly one submit. The acquired image uniquely selects
        // its presentation semaphore. All render-pass/pipeline/buffer handles remain alive.
        unsafe {
            let acquire_begin = self.diagnostics_enabled.then(Instant::now);
            let acquired = s.api.acquire_next_image(
                s.raw,
                u64::MAX,
                self.commands.available,
                vk::Fence::null(),
            );
            self.diagnostics.acquire_ms = acquire_begin.map(elapsed_ms);
            let (index, suboptimal) = match acquired {
                Ok(v) => v,
                Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                    self.recreate = true;
                    return Ok(FrameResult::Retry);
                }
                Err(e) => return Err(err(e)),
            };
            d.reset_command_buffer(cmd, vk::CommandBufferResetFlags::empty())
                .map_err(err)?;
            d.begin_command_buffer(
                cmd,
                &vk::CommandBufferBeginInfo::default()
                    .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            )
            .map_err(err)?;
            if let Some(timestamps) = &mut self.timestamps {
                timestamps.begin(cmd);
            }
            let shadow_updated = self.shadow.record(
                cmd,
                lighting.shadows,
                self.chunks
                    .values()
                    .chain(self.legacy.iter())
                    .chain(self.dynamic.iter()),
                self.static_scene.as_ref(),
                self.identity.raw,
            );
            if let Some(timestamps) = &self.timestamps {
                timestamps.mark(cmd, 1);
            }
            let clear = [
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
                extent: s.size,
            };
            d.cmd_begin_render_pass(
                cmd,
                &vk::RenderPassBeginInfo::default()
                    .render_pass(s.pass)
                    .framebuffer(s.frames[index as usize])
                    .render_area(area)
                    .clear_values(&clear),
                vk::SubpassContents::INLINE,
            );
            d.cmd_set_viewport(
                cmd,
                0,
                &[vk::Viewport {
                    x: 0.0,
                    y: 0.0,
                    width: s.size.width as f32,
                    height: s.size.height as f32,
                    min_depth: 0.0,
                    max_depth: 1.0,
                }],
            );
            d.cmd_set_scissor(cmd, 0, &[area]);
            d.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, s.world);
            d.cmd_bind_descriptor_sets(
                cmd,
                vk::PipelineBindPoint::GRAPHICS,
                s.layout,
                0,
                &[self.shadow.set],
                &[],
            );
            // Identity instance record for the non-instanced draws below.
            d.cmd_bind_vertex_buffers(cmd, 1, &[self.identity.raw], &[0]);
            d.cmd_push_constants(
                cmd,
                s.layout,
                vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT,
                0,
                bytemuck::bytes_of(&Camera {
                    view_proj,
                    eye: [
                        eye[0],
                        eye[1],
                        eye[2],
                        self.material_time.map_or(1.0, |t| -1.0 - t),
                    ],
                }),
            );
            let frustum = Frustum::new(view_proj);
            self.visible_chunks = self
                .chunks
                .values()
                .filter(|mesh| {
                    self.world_visible && mesh.index_count > 0 && frustum.intersects(mesh.bounds)
                })
                .count();
            let visible = self.chunks.values().filter(|mesh| {
                self.world_visible && mesh.index_count > 0 && frustum.intersects(mesh.bounds)
            });
            for mesh in self.legacy.iter().chain(visible).chain(self.dynamic.iter()) {
                if !self.world_visible || mesh.index_count == 0 {
                    continue;
                }
                if let (Some(v), Some(i)) = (&mesh.vertices, &mesh.indices) {
                    d.cmd_bind_vertex_buffers(cmd, 0, &[v.raw], &[0]);
                    d.cmd_bind_index_buffer(cmd, i.raw, 0, vk::IndexType::UINT32);
                    d.cmd_draw_indexed(cmd, mesh.index_count, 1, 0, 0, 0);
                }
            }
            // Instanced static scene: one batch per prototype carrying all of
            // its instances; whole-batch frustum culling only, never applied
            // to the shadow pass above.
            if self.world_visible {
                if let Some(scene) = &self.static_scene {
                    scene.record_batches(d, cmd, Some(&frustum));
                }
            }
            if hud_count > 0 {
                d.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, s.hud);
                d.cmd_bind_vertex_buffers(
                    cmd,
                    0,
                    &[self.hud.as_ref().expect("HUD allocated").raw],
                    &[0],
                );
                d.cmd_draw(cmd, hud_count, 1, 0, 0);
            }
            d.cmd_end_render_pass(cmd);
            if let Some(timestamps) = &self.timestamps {
                timestamps.mark(cmd, 2);
            }
            d.end_command_buffer(cmd).map_err(err)?;
            let waits = [self.commands.available];
            let stages = [vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT];
            let buffers = [cmd];
            let signals = [s.finished[index as usize]];
            let submit = [vk::SubmitInfo::default()
                .wait_semaphores(&waits)
                .wait_dst_stage_mask(&stages)
                .command_buffers(&buffers)
                .signal_semaphores(&signals)];
            d.reset_fences(&[self.commands.fence]).map_err(err)?;
            d.queue_submit(self.device.queue, &submit, self.commands.fence)
                .map_err(err)?;
            if shadow_updated {
                self.shadow.submitted(lighting.shadows);
            }
            self.shadow_map_updated = shadow_updated;
            // One identity per accepted submission, shared with the timestamp
            // queries, so a capture can join CPU and GPU records exactly.
            self.submissions += 1;
            self.diagnostics.submitted_frame_id = Some(self.submissions);
            if let Some(timestamps) = &mut self.timestamps {
                timestamps.submitted(lighting.shadows, shadow_updated, lighting.shadow_map_size);
            }
            let chains = [s.raw];
            let indices = [index];
            let present_begin = self.diagnostics_enabled.then(Instant::now);
            let presented = s.api.queue_present(
                self.device.queue,
                &vk::PresentInfoKHR::default()
                    .wait_semaphores(&signals)
                    .swapchains(&chains)
                    .image_indices(&indices),
            );
            self.diagnostics.present_ms = present_begin.map(elapsed_ms);
            match classify_present(presented, suboptimal) {
                PresentOutcome::Presented { recreate } => self.recreate = recreate,
                PresentOutcome::OutOfDate => {
                    // The submission stands and its identity is kept; only the
                    // presentation request failed, so this is not a presented frame.
                    self.recreate = true;
                    return Ok(FrameResult::Retry);
                }
                PresentOutcome::Failed(e) => return Err(err(e)),
            }
        }
        Ok(FrameResult::Presented)
    }
}
impl Drop for Renderer {
    fn drop(&mut self) {
        // SAFETY: exclusive owner; wait before fields release buffers, command pool and swapchain.
        unsafe {
            let _ = self.device.raw.device_wait_idle();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{classify_present, PresentOutcome, WaitTally};
    use ash::vk;
    use std::time::Instant;

    #[test]
    fn disabled_wait_tally_takes_no_clock_and_reports_nothing() {
        let mut tally = WaitTally::default();
        tally.reset(false);
        let begin = tally.timed_begin();
        assert!(begin.is_none(), "a disabled tally must not read the clock");
        tally.record(begin);
        assert_eq!(tally.total(), None);
        assert_eq!(tally.count(), None);
    }

    #[test]
    fn enabled_wait_tally_distinguishes_zero_one_and_many_waits() {
        let mut tally = WaitTally::default();
        tally.reset(true);
        // No wait happened: an explicit zero, not a missing measurement.
        assert_eq!(tally.total(), Some(0.));
        assert_eq!(tally.count(), Some(0));
        tally.record(Some(Instant::now()));
        assert_eq!(tally.count(), Some(1));
        let after_one = tally.total().unwrap();
        tally.record(Some(Instant::now()));
        tally.record(Some(Instant::now()));
        assert_eq!(tally.count(), Some(3), "every wait call site is counted");
        assert!(
            tally.total().unwrap() >= after_one,
            "waits accumulate rather than replace"
        );
    }

    #[test]
    fn resetting_a_tally_drops_waits_from_the_previous_attempt() {
        let mut tally = WaitTally::default();
        tally.reset(true);
        tally.record(Some(Instant::now()));
        tally.reset(true);
        assert_eq!(tally.count(), Some(0), "prior waits must not leak forward");
        assert_eq!(tally.total(), Some(0.));
        tally.record(Some(Instant::now()));
        tally.reset(false);
        assert_eq!(tally.count(), None);
        assert_eq!(tally.total(), None);
    }

    #[test]
    fn present_results_map_to_honest_frame_outcomes() {
        assert_eq!(
            classify_present(Ok(false), false),
            PresentOutcome::Presented { recreate: false }
        );
        // Advisory suboptimal results remain usable. Recreating on every
        // advisory result can rebuild pipelines every frame on Android.
        for (present_suboptimal, acquire_suboptimal) in [(false, true), (true, false), (true, true)]
        {
            assert_eq!(
                classify_present(Ok(present_suboptimal), acquire_suboptimal),
                PresentOutcome::Presented { recreate: false }
            );
        }
        // Out of date is not a successful presentation: the submission happened,
        // the presentation request did not succeed.
        assert_eq!(
            classify_present(Err(vk::Result::ERROR_OUT_OF_DATE_KHR), false),
            PresentOutcome::OutOfDate
        );
        assert_eq!(
            classify_present(Err(vk::Result::ERROR_OUT_OF_DATE_KHR), true),
            PresentOutcome::OutOfDate
        );
        assert_eq!(
            classify_present(Err(vk::Result::ERROR_DEVICE_LOST), false),
            PresentOutcome::Failed(vk::Result::ERROR_DEVICE_LOST)
        );
        assert_eq!(
            classify_present(Err(vk::Result::ERROR_OUT_OF_DEVICE_MEMORY), false),
            PresentOutcome::Failed(vk::Result::ERROR_OUT_OF_DEVICE_MEMORY)
        );
    }
}
