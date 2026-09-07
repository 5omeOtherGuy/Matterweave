//! A single directional depth map. Resource replacement is serialized by Renderer.
use super::{err, pipeline, Buffer, Depth, Device, Frustum, GpuMesh, PipelineKind, Result};
use crate::lighting::{LightingSettings, ShadowCamera};
use ash::vk;
use bytemuck::{Pod, Zeroable};
use std::sync::Arc;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniform {
    view_proj: [[f32; 4]; 4],
    sun: [f32; 4],
    params: [f32; 4],
}

pub(crate) struct Shadow {
    device: Arc<Device>,
    depth: Option<Depth>,
    uniform: Option<Buffer>,
    pass: vk::RenderPass,
    frame: vk::Framebuffer,
    pipeline: vk::Pipeline,
    pipeline_layout: vk::PipelineLayout,
    pub set_layout: vk::DescriptorSetLayout,
    pool: vk::DescriptorPool,
    pub set: vk::DescriptorSet,
    sampler: vk::Sampler,
    pub size: u32,
    initialized: bool,
    pub caster_meshes: usize,
    camera: ShadowCamera,
}

impl Shadow {
    pub fn new(device: Arc<Device>, size: u32) -> Result<Self> {
        let camera = ShadowCamera::new([0.; 3], Default::default(), &[], size)?;
        let mut out = Self {
            device,
            depth: None,
            uniform: None,
            pass: vk::RenderPass::null(),
            frame: vk::Framebuffer::null(),
            pipeline: vk::Pipeline::null(),
            pipeline_layout: vk::PipelineLayout::null(),
            set_layout: vk::DescriptorSetLayout::null(),
            pool: vk::DescriptorPool::null(),
            set: vk::DescriptorSet::null(),
            sampler: vk::Sampler::null(),
            size,
            initialized: false,
            caster_meshes: 0,
            camera,
        };
        // SAFETY: every created handle is immediately stored in this partial-construction
        // guard. Format support is queried; descriptors reference owned resources.
        unsafe {
            let d = &out.device;
            let required = vk::FormatFeatureFlags::DEPTH_STENCIL_ATTACHMENT
                | vk::FormatFeatureFlags::SAMPLED_IMAGE;
            let format = [vk::Format::D32_SFLOAT, vk::Format::D16_UNORM]
                .into_iter()
                .find(|&f| {
                    d.instance
                        .raw
                        .get_physical_device_format_properties(d.physical, f)
                        .optimal_tiling_features
                        .contains(required)
                })
                .ok_or("No sampled depth attachment format for sunlight shadows")?;
            let extent = vk::Extent2D {
                width: size,
                height: size,
            };
            out.depth = Some(Depth::new(d.clone(), extent, format, true)?);
            out.uniform = Some(Buffer::new(
                d.clone(),
                bytemuck::bytes_of(&Uniform::zeroed()),
                vk::BufferUsageFlags::UNIFORM_BUFFER,
            )?);
            // Nearest comparison samples plus explicit 3x3 PCF need no optional
            // linear-filtering capability for the selected depth format.
            out.sampler = d
                .raw
                .create_sampler(
                    &vk::SamplerCreateInfo::default()
                        .mag_filter(vk::Filter::NEAREST)
                        .min_filter(vk::Filter::NEAREST)
                        .mipmap_mode(vk::SamplerMipmapMode::NEAREST)
                        .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_BORDER)
                        .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_BORDER)
                        .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_BORDER)
                        .border_color(vk::BorderColor::FLOAT_OPAQUE_WHITE)
                        .compare_enable(true)
                        .compare_op(vk::CompareOp::LESS_OR_EQUAL)
                        .min_lod(0.)
                        .max_lod(0.),
                    None,
                )
                .map_err(err)?;
            let types = [
                vk::DescriptorType::UNIFORM_BUFFER,
                vk::DescriptorType::SAMPLED_IMAGE,
                vk::DescriptorType::SAMPLER,
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
            out.set_layout = d
                .raw
                .create_descriptor_set_layout(
                    &vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings),
                    None,
                )
                .map_err(err)?;
            let sizes: Vec<_> = types
                .iter()
                .map(|&ty| vk::DescriptorPoolSize {
                    ty,
                    descriptor_count: 1,
                })
                .collect();
            out.pool = d
                .raw
                .create_descriptor_pool(
                    &vk::DescriptorPoolCreateInfo::default()
                        .max_sets(1)
                        .pool_sizes(&sizes),
                    None,
                )
                .map_err(err)?;
            out.set = d
                .raw
                .allocate_descriptor_sets(
                    &vk::DescriptorSetAllocateInfo::default()
                        .descriptor_pool(out.pool)
                        .set_layouts(&[out.set_layout]),
                )
                .map_err(err)?[0];
            let buffers = [vk::DescriptorBufferInfo::default()
                .buffer(out.uniform.as_ref().unwrap().raw)
                .range(std::mem::size_of::<Uniform>() as u64)];
            let images = [vk::DescriptorImageInfo::default()
                .image_view(out.depth.as_ref().unwrap().view)
                .image_layout(vk::ImageLayout::DEPTH_STENCIL_READ_ONLY_OPTIMAL)];
            let samplers = [vk::DescriptorImageInfo::default().sampler(out.sampler)];
            d.raw.update_descriptor_sets(
                &[
                    vk::WriteDescriptorSet::default()
                        .dst_set(out.set)
                        .dst_binding(0)
                        .descriptor_type(types[0])
                        .buffer_info(&buffers),
                    vk::WriteDescriptorSet::default()
                        .dst_set(out.set)
                        .dst_binding(1)
                        .descriptor_type(types[1])
                        .image_info(&images),
                    vk::WriteDescriptorSet::default()
                        .dst_set(out.set)
                        .dst_binding(2)
                        .descriptor_type(types[2])
                        .image_info(&samplers),
                ],
                &[],
            );
            let attachments = [vk::AttachmentDescription::default()
                .format(format)
                .samples(vk::SampleCountFlags::TYPE_1)
                .load_op(vk::AttachmentLoadOp::CLEAR)
                .store_op(vk::AttachmentStoreOp::STORE)
                .initial_layout(vk::ImageLayout::UNDEFINED)
                .final_layout(vk::ImageLayout::DEPTH_STENCIL_READ_ONLY_OPTIMAL)];
            let depth = vk::AttachmentReference {
                attachment: 0,
                layout: vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL,
            };
            let subpasses = [vk::SubpassDescription::default()
                .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
                .depth_stencil_attachment(&depth)];
            let tests = vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS
                | vk::PipelineStageFlags::LATE_FRAGMENT_TESTS;
            let dependencies = [
                vk::SubpassDependency::default()
                    .src_subpass(vk::SUBPASS_EXTERNAL)
                    .dst_subpass(0)
                    .src_stage_mask(vk::PipelineStageFlags::FRAGMENT_SHADER)
                    .dst_stage_mask(tests)
                    .src_access_mask(vk::AccessFlags::SHADER_READ)
                    .dst_access_mask(vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE),
                vk::SubpassDependency::default()
                    .src_subpass(0)
                    .dst_subpass(vk::SUBPASS_EXTERNAL)
                    .src_stage_mask(tests)
                    .dst_stage_mask(vk::PipelineStageFlags::FRAGMENT_SHADER)
                    .src_access_mask(vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE)
                    .dst_access_mask(vk::AccessFlags::SHADER_READ),
            ];
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
            out.frame = d
                .raw
                .create_framebuffer(
                    &vk::FramebufferCreateInfo::default()
                        .render_pass(out.pass)
                        .attachments(&[out.depth.as_ref().unwrap().view])
                        .width(size)
                        .height(size)
                        .layers(1),
                    None,
                )
                .map_err(err)?;
            out.pipeline_layout = d
                .raw
                .create_pipeline_layout(
                    &vk::PipelineLayoutCreateInfo::default().push_constant_ranges(&[
                        vk::PushConstantRange::default()
                            .stage_flags(vk::ShaderStageFlags::VERTEX)
                            .size(64),
                    ]),
                    None,
                )
                .map_err(err)?;
            out.pipeline = pipeline(d, out.pipeline_layout, out.pass, PipelineKind::Shadow)?;
        }
        Ok(out)
    }

    /// Caller has completed the previous frame fence before updating this coherent UBO.
    pub fn update(
        &mut self,
        eye: [f32; 3],
        settings: &LightingSettings,
        bounds: &[[[f32; 3]; 2]],
    ) -> Result<()> {
        self.camera = ShadowCamera::new(eye, settings.sun, bounds, self.size)?;
        let sun = self.camera.direction;
        self.uniform
            .as_ref()
            .unwrap()
            .write(bytemuck::bytes_of(&Uniform {
                view_proj: self.camera.view_proj,
                sun: [sun[0], sun[1], sun[2], settings.sun.intensity],
                // World-space bias preserves its scale when the fitted depth span changes.
                params: [
                    if settings.shadows { 1.0 } else { 0.0 },
                    0.025 / self.camera.depth_span,
                    1.0 / self.size as f32,
                    0.,
                ],
            }))
    }

    /// Record a clear even on the first shadows-off frame: descriptor layout and
    /// depth contents must be valid before the world pipeline can reference them.
    pub fn record<'a>(
        &mut self,
        cmd: vk::CommandBuffer,
        enabled: bool,
        meshes: impl Iterator<Item = &'a GpuMesh>,
    ) {
        self.caster_meshes = 0;
        if !enabled && self.initialized {
            return;
        }
        // SAFETY: caller records into an idle command buffer; all referenced meshes,
        // descriptors and framebuffer resources survive until its submit fence.
        unsafe {
            let d = &self.device.raw;
            let area = vk::Rect2D {
                offset: Default::default(),
                extent: vk::Extent2D {
                    width: self.size,
                    height: self.size,
                },
            };
            let clear = [vk::ClearValue {
                depth_stencil: vk::ClearDepthStencilValue {
                    depth: 1.,
                    stencil: 0,
                },
            }];
            d.cmd_begin_render_pass(
                cmd,
                &vk::RenderPassBeginInfo::default()
                    .render_pass(self.pass)
                    .framebuffer(self.frame)
                    .render_area(area)
                    .clear_values(&clear),
                vk::SubpassContents::INLINE,
            );
            d.cmd_set_viewport(
                cmd,
                0,
                &[vk::Viewport {
                    x: 0.,
                    y: 0.,
                    width: self.size as f32,
                    height: self.size as f32,
                    min_depth: 0.,
                    max_depth: 1.,
                }],
            );
            d.cmd_set_scissor(cmd, 0, &[area]);
            d.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, self.pipeline);
            d.cmd_push_constants(
                cmd,
                self.pipeline_layout,
                vk::ShaderStageFlags::VERTEX,
                0,
                bytemuck::cast_slice(&self.camera.view_proj),
            );
            if enabled {
                let frustum = Frustum::new(self.camera.view_proj);
                for mesh in meshes.filter(|m| m.index_count > 0 && frustum.intersects(m.bounds)) {
                    if let (Some(v), Some(i)) = (&mesh.vertices, &mesh.indices) {
                        d.cmd_bind_vertex_buffers(cmd, 0, &[v.raw], &[0]);
                        d.cmd_bind_index_buffer(cmd, i.raw, 0, vk::IndexType::UINT32);
                        d.cmd_draw_indexed(cmd, mesh.index_count, 1, 0, 0, 0);
                        self.caster_meshes += 1;
                    }
                }
            }
            d.cmd_end_render_pass(cmd);
        }
        self.initialized = true;
    }
}

impl Drop for Shadow {
    fn drop(&mut self) {
        // SAFETY: Renderer waits before replacement/teardown. Pool frees its set;
        // framebuffer/pipeline are destroyed before images, views and their memory.
        unsafe {
            let d = &self.device.raw;
            d.destroy_framebuffer(self.frame, None);
            d.destroy_pipeline(self.pipeline, None);
            d.destroy_pipeline_layout(self.pipeline_layout, None);
            d.destroy_render_pass(self.pass, None);
            d.destroy_descriptor_pool(self.pool, None);
            d.destroy_sampler(self.sampler, None);
            d.destroy_descriptor_set_layout(self.set_layout, None);
        }
    }
}
