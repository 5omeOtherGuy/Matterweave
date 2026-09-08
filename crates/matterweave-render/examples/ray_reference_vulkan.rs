//! Headless native Vulkan exercise of the `ray_reference` traversal shader.
//!
//! Builds the documented set-0 pipeline (uniform + two storage buffers), renders
//! one pixel per probe through the real Naga-compiled SPIR-V, reads color and
//! depth back, and compares against the CPU `World::raycast` oracle on the same
//! analytic ray. Not a performance measurement and not a path selection: see
//! `docs/performance/ray-reference.md`.
//!
//! Exit codes: 0 all checks passed, 1 at least one check failed,
//! 2 no usable Vulkan 1.1 graphics device (checks NOT RUN).
use ash::vk;
use glam::{Mat4, Vec3, Vec4};
use matterweave_core::World;
use matterweave_render::ray_reference::{RayVolume, FRAGMENT_SPIRV, VERTEX_SPIRV};
use matterweave_render::Sun;
use std::io::Cursor;

const NEAR: f32 = 0.25;
const FAR: f32 = 40.0;
/// One 8-bit UNORM step plus f32 traversal differences.
const COLOR_TOLERANCE: f32 = 0.02;
const DEPTH_TOLERANCE: f32 = 2e-3;
const EXTENT: u32 = 1;

fn err(e: vk::Result) -> String {
    format!("{e:?}")
}

fn palette() -> [[f32; 3]; 256] {
    std::array::from_fn(|i| [i as f32 / 255.0, 0.25, 0.5])
}

struct Fixture {
    name: &'static str,
    world: World,
    origin: [i32; 3],
    dimensions: [u32; 3],
}

fn solid(world: &mut World, cells: &[[i32; 3]], material: u8) {
    for cell in cells {
        world.set(*cell, material);
    }
}

fn fixtures() -> Vec<Fixture> {
    // Boundary/negative/diagonal fixture. Only cells inside the crop are set.
    let mut mixed = World::new(11);
    solid(&mut mixed, &[[0, 0, 0]], 1);
    solid(&mut mixed, &[[-4, -4, -4]], 2);
    solid(&mut mixed, &[[3, 3, 3]], 3);
    solid(&mut mixed, &[[-1, 2, -3]], 4);

    // Thin volume: one cell thick in Y and Z, eight cells along X.
    let mut thin = World::new(12);
    solid(&mut thin, &[[4, 0, 0]], 7);

    // Inside-start fixture: the probe begins inside an occupied cell.
    let mut inside = World::new(13);
    solid(&mut inside, &[[1, 1, 1]], 9);

    vec![
        Fixture {
            name: "mixed-8cube",
            world: mixed,
            origin: [-4; 3],
            dimensions: [8; 3],
        },
        Fixture {
            name: "thin-8x1x1",
            world: thin,
            origin: [0, 0, 0],
            dimensions: [8, 1, 1],
        },
        Fixture {
            name: "inside-solid",
            world: inside,
            origin: [0; 3],
            dimensions: [4; 3],
        },
    ]
}

struct Probe {
    name: &'static str,
    /// Ray origin at the camera near plane; equals `eye + forward * NEAR`.
    origin: [f32; 3],
    direction: [f32; 3],
    /// Documented shader normal where a CPU tie is known to differ (None = oracle).
    normal_override: Option<[i32; 3]>,
}

fn probes(fixture: usize) -> Vec<Probe> {
    match fixture {
        0 => vec![
            Probe {
                name: "-X into center cell",
                origin: [8.5, 0.5, 0.5],
                direction: [-1.0, 0.0, 0.0],
                normal_override: None,
            },
            Probe {
                name: "-Y into center cell",
                origin: [0.5, 8.5, 0.5],
                direction: [0.0, -1.0, 0.0],
                normal_override: None,
            },
            Probe {
                name: "+X into negative min corner",
                origin: [-12.5, -3.5, -3.5],
                direction: [1.0, 0.0, 0.0],
                normal_override: None,
            },
            Probe {
                name: "diagonal tie on three upper faces",
                origin: [8.5, 8.5, 8.5],
                direction: [-1.0, -1.0, -1.0],
                normal_override: Some([1, 0, 0]),
            },
            Probe {
                name: "grazing miss through empty row",
                origin: [8.5, 1.5, 1.5],
                direction: [-1.0, 0.0, 0.0],
                normal_override: None,
            },
            Probe {
                name: "negative-coordinate interior cell",
                origin: [-8.5, 2.5, -2.5],
                direction: [1.0, 0.0, 0.0],
                normal_override: None,
            },
        ],
        1 => vec![
            Probe {
                name: "-Y through thin one-cell axis",
                origin: [4.5, 5.5, 0.5],
                direction: [0.0, -1.0, 0.0],
                normal_override: None,
            },
            Probe {
                name: "parallel on upper face misses",
                origin: [-4.5, 1.0, 0.5],
                direction: [1.0, 0.0, 0.0],
                normal_override: None,
            },
            Probe {
                name: "parallel on lower face is inside",
                origin: [-4.5, 0.0, 0.5],
                direction: [1.0, 0.0, 0.0],
                normal_override: None,
            },
            Probe {
                name: "thin volume from below",
                origin: [4.5, -6.5, 0.5],
                direction: [0.0, 1.0, 0.0],
                normal_override: None,
            },
        ],
        _ => vec![
            Probe {
                name: "origin inside solid returns zero normal",
                origin: [1.5, 1.5, 1.5],
                direction: [1.0, 0.0, 0.0],
                normal_override: Some([0; 3]),
            },
            Probe {
                name: "inside air then solid",
                origin: [0.5, 1.5, 1.5],
                direction: [1.0, 0.0, 0.0],
                normal_override: None,
            },
        ],
    }
}

/// Camera whose centre pixel ray is exactly (`origin`, `direction`): the eye sits
/// one `NEAR` behind the ray origin, so both projection modes unproject the same
/// segment. Derived analytically, not from the shader's unprojection.
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

/// Documented basic lighting from world.wgsl, mirrored as the golden model.
fn shade(material_rgb: [f32; 3], normal: Vec3, view_distance: f32, sun: [f32; 4]) -> [f32; 3] {
    let sunlight = normal.dot(Vec3::new(sun[0], sun[1], sun[2])).max(0.0);
    let ambient = 0.28 + 0.12 * normal.y.max(0.0);
    let fog = 1.0 - (-view_distance * 0.013).exp();
    let lit = Vec3::from_array(material_rgb) * (ambient + sunlight * sun[3]);
    lit.lerp(Vec3::new(0.16, 0.24, 0.29), fog).to_array()
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
            .application_name(c"matterweave-ray-reference")
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

/// One pipeline plus its set-0 descriptor resources for a packed volume.
struct Session {
    uniform: Buffer,
    materials: Buffer,
    palette: Buffer,
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
    fn new(gpu: &Gpu, pack: &RayVolume) -> Result<Self, String> {
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
            let color_read = Self::buffer(gpu, 4, vk::BufferUsageFlags::TRANSFER_DST, None)?;
            let depth_read = Self::buffer(gpu, 4, vk::BufferUsageFlags::TRANSFER_DST, None)?;

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
            let layout = device
                .create_pipeline_layout(
                    &vk::PipelineLayoutCreateInfo::default().set_layouts(&layouts),
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
                .range(materials.size)];
            let palette_info = [vk::DescriptorBufferInfo::default()
                .buffer(palette.handle)
                .range(palette.size)];
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
            let pass = device
                .create_render_pass(
                    &vk::RenderPassCreateInfo::default()
                        .attachments(&attachments)
                        .subpasses(&subpasses),
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
                uniform,
                materials,
                palette,
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
        Ok(Buffer {
            handle,
            memory,
            size,
        })
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
    unsafe fn render(&self, gpu: &Gpu, uniform: &[u8; 192]) -> Result<([f32; 4], f32), String> {
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
        Ok((rgba, depth))
    }

    unsafe fn destroy(&mut self, gpu: &Gpu) {
        let device = &gpu.device;
        for buffer in [
            &self.uniform,
            &self.materials,
            &self.palette,
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
        device.destroy_framebuffer(self.framebuffer, None);
        device.destroy_pipeline(self.pipeline, None);
        device.destroy_render_pass(self.pass, None);
        device.destroy_descriptor_pool(self.descriptor_pool, None);
        device.destroy_pipeline_layout(self.layout, None);
        device.destroy_descriptor_set_layout(self.set_layout, None);
    }
}

fn main() {
    let mut failures = 0usize;
    let mut checks = 0usize;
    let gpu = match Gpu::new() {
        Ok(gpu) => gpu,
        // SAFETY: no Vulkan device exists; nothing was created.
        Err(message) => {
            println!("NOT RUN: {message}");
            println!("ray_reference_vulkan: no native Vulkan 1.1 graphics device on this host");
            std::process::exit(2);
        }
    };
    println!("device: {}", gpu.device_name);
    let sun = Sun::default();
    let sun_direction = Vec3::from_array(sun.direction_to_sun).normalize();
    let sun_key = [
        sun_direction.x,
        sun_direction.y,
        sun_direction.z,
        sun.intensity,
    ];
    for (index, fixture) in fixtures().into_iter().enumerate() {
        let pack = match RayVolume::pack(
            &fixture.world,
            1,
            fixture.origin,
            fixture.dimensions,
            palette(),
        ) {
            Ok(pack) => pack,
            Err(message) => {
                println!("FAIL {}: pack rejected: {message}", fixture.name);
                failures += 1;
                continue;
            }
        };
        let mut session = match Session::new(&gpu, &pack) {
            Ok(session) => session,
            Err(message) => {
                println!("FAIL {}: pipeline unavailable: {message}", fixture.name);
                failures += 1;
                continue;
            }
        };
        println!(
            "fixture {} dims {:?} origin {:?}",
            fixture.name, fixture.dimensions, fixture.origin
        );
        // SAFETY: session handles are live for the whole loop body.
        for probe in probes(index) {
            for orthographic in [true, false] {
                let mode = if orthographic { "ortho" } else { "persp" };
                let origin = Vec3::from_array(probe.origin);
                let direction = Vec3::from_array(probe.direction).normalize();
                let (view_projection, eye) = camera(origin, direction, orthographic);
                let uniform: [u8; 192] = match pack.uniform(view_projection, eye, sun) {
                    Ok(uniform) => match bytemuck::bytes_of(&uniform).try_into() {
                        Ok(bytes) => bytes,
                        Err(_) => {
                            println!("  FAIL {} [{mode}] uniform size", probe.name);
                            failures += 1;
                            continue;
                        }
                    },
                    Err(message) => {
                        println!("  FAIL {} [{mode}] uniform: {message}", probe.name);
                        failures += 1;
                        continue;
                    }
                };
                let rendered = unsafe { session.render(&gpu, &uniform) };
                let (rgba, depth) = match rendered {
                    Ok(value) => value,
                    Err(message) => {
                        println!("  FAIL {} [{mode}] render: {message}", probe.name);
                        failures += 1;
                        continue;
                    }
                };
                let oracle = fixture
                    .world
                    .raycast(probe.origin, direction.to_array(), FAR - NEAR);
                let mut note = String::new();
                let (want_color, want_depth) = match &oracle {
                    Some(hit) => {
                        let normal = probe.normal_override.unwrap_or(hit.normal);
                        if probe.normal_override.is_some() {
                            note = format!(" oracle normal {:?}", hit.normal);
                        }
                        let point = origin + direction * hit.distance;
                        let clip = view_projection * Vec4::new(point.x, point.y, point.z, 1.0);
                        let color = shade(
                            palette()[usize::from(hit.material)],
                            Vec3::new(normal[0] as f32, normal[1] as f32, normal[2] as f32),
                            (point - eye).length(),
                            sun_key,
                        );
                        (color, clip.z / clip.w)
                    }
                    None => ([0.0, 0.0, 0.0], 1.0),
                };
                checks += 1;
                let color_ok = (0..3).all(|c| (rgba[c] - want_color[c]).abs() <= COLOR_TOLERANCE);
                let depth_ok = (depth - want_depth).abs() <= DEPTH_TOLERANCE;
                let material = oracle.as_ref().map(|h| h.material);
                if color_ok && depth_ok {
                    println!(
                        "  PASS {} [{mode}] rgb {:.4?} depth {depth:.5} material {material:?}{note}",
                        probe.name, &rgba[..3]
                    );
                } else {
                    failures += 1;
                    println!(
                        "  FAIL {} [{mode}] got rgb {:.4?} depth {depth:.5}, want rgb {:.4?} depth {want_depth:.5}, material {material:?}{note}",
                        probe.name, &rgba[..3], want_color
                    );
                }
            }
        }
        // SAFETY: the queue was waited idle after the last submit.
        unsafe { session.destroy(&gpu) };
    }
    println!("ray_reference_vulkan: {checks} checks, {failures} failures");
    if failures > 0 {
        std::process::exit(1);
    }
}
