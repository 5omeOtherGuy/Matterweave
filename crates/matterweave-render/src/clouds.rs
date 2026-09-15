//! Sky dome and volumetric clouds.
//!
//! Two draws, both full-screen triangles with no vertex buffer:
//!
//!  * the sky dome, drawn into the frame after opaque geometry with a depth
//!    test that passes only where the scene left the far value, so covered
//!    pixels are never shaded;
//!  * a volumetric cloud march into a reduced-resolution offscreen target,
//!    recorded outside the frame's render pass, whose upsample is composited
//!    after the dome with the same depth test.
//!
//! Both read the group-0 lighting uniform the world pass already binds, so the
//! sun and sky colour here are the same ones the terrain is lit and fogged
//! with. The offscreen target exists only while clouds are enabled: disabling
//! them frees it, and every sample that never enables them allocates nothing.
use super::{err, Buffer, Device, Result};
use crate::lighting::Atmosphere;
use ash::vk;
use bytemuck::{Pod, Zeroable};
use std::sync::Arc;

/// Volumetric cloud settings. The default is off: a sample that never sets
/// this renders exactly the frame it rendered before clouds existed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Clouds {
    pub enabled: bool,
    /// 0 = cheapest (fewest steps, quarter resolution), 1 = quality (more
    /// steps, half resolution). Nothing above [`Clouds::MAX_QUALITY`].
    pub quality: u8,
    /// Animation clock in seconds. Callers advance it; the renderer folds it
    /// into [`Clouds::TIME_PERIOD_S`], which is exactly the period the shader's
    /// wrapping noise lattice repeats over, so the fold is not a visible jump.
    pub time_s: f32,
}

impl Default for Clouds {
    fn default() -> Self {
        Self {
            enabled: false,
            quality: 0,
            time_s: 0.0,
        }
    }
}

impl Clouds {
    pub const MAX_QUALITY: u8 = 1;
    /// Drift is 0.5 m/s and the shader's lattice wraps every 32768 m, so the
    /// clock folds where the layer is bit-identical to where it started.
    pub const TIME_PERIOD_S: f32 = 65536.0;

    pub(crate) fn validate(&self) -> Result<()> {
        if self.quality > Self::MAX_QUALITY {
            return Err(format!(
                "Cloud quality must be at most {}",
                Self::MAX_QUALITY
            ));
        }
        if !self.time_s.is_finite() {
            return Err("Cloud time must be finite".into());
        }
        Ok(())
    }

    /// Ray-march steps through the slab and steps toward the sun. Both are
    /// fixed per quality level: the shader loops on these counts, so an
    /// unbounded or non-finite value would be an unbounded fragment.
    pub(crate) fn march_steps(&self) -> u32 {
        if self.quality >= 1 {
            56
        } else {
            28
        }
    }

    pub(crate) fn light_steps(&self) -> u32 {
        if self.quality >= 1 {
            6
        } else {
            4
        }
    }

    /// How many frame pixels one cloud pixel covers on each axis.
    pub(crate) fn divisor(&self) -> u32 {
        if self.quality >= 1 {
            2
        } else {
            4
        }
    }

    pub(crate) fn time(&self) -> f32 {
        self.time_s.rem_euclid(Self::TIME_PERIOD_S)
    }
}

/// The offscreen target a set of settings asks for at a frame size. Equality is
/// the rebuild test: same plan, same resources.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CloudPlan {
    pub width: u32,
    pub height: u32,
    pub quality: u8,
}

impl CloudPlan {
    /// `None` when no target should exist: clouds off, or a frame with no area.
    pub(crate) fn new(frame: (u32, u32), clouds: &Clouds) -> Option<Self> {
        if !clouds.enabled || clouds.quality > Clouds::MAX_QUALITY {
            return None;
        }
        let (width, height) = frame;
        if width == 0 || height == 0 {
            return None;
        }
        let divisor = clouds.divisor();
        Some(Self {
            width: width.div_ceil(divisor).max(1),
            height: height.div_ceil(divisor).max(1),
            quality: clouds.quality,
        })
    }
}

/// Push constants for both sky entries. 112 bytes, inside the 128 every Vulkan
/// implementation guarantees.
#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub(crate) struct SkyPush {
    pub inv_view_proj: [[f32; 4]; 4],
    /// eye xyz, w = cloud animation clock in seconds
    pub eye: [f32; 4],
    /// (march steps, light steps, inverse target width, inverse target height)
    pub params: [f32; 4],
    /// (frame width, frame height, cloud divisor, flags). Flags bit 3 is the
    /// shader cost counters; the low bits are the amortisation phase.
    pub control: [u32; 4],
}

pub(crate) const SKY_PUSH_BYTES: u32 = 112;
/// Flag bit in `SkyPush::control[3]` that turns the shader cost counters on.
pub(crate) const COUNTERS_ENABLED: u32 = 8;
/// Flag bits in `SkyPush::control[3]` that select the amortised cloud sub-grid.
pub(crate) const SUBGRID_MASK: u32 = 0b11;
/// Flag bit that forces a full cloud target update this frame.
pub(crate) const FULL_UPDATE: u32 = 4;
/// Byte size of the counter storage buffer: six `u32` atomics.
pub(crate) const COUNTER_BYTES: usize = 24;

/// Push constants for the cloud composite: frame and cloud-texel sizes.
#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub(crate) struct CompositePush {
    /// (inverse frame width, inverse frame height, cloud texel width, cloud texel height)
    pub texel: [f32; 4],
}

pub(crate) const COMPOSITE_PUSH_BYTES: u32 = 16;

/// How far the camera may move, in target pixels, before the amortised cloud
/// target is recomputed in full instead of one sub-grid at a time. The reused
/// pixels are stale by up to a few frames, so this bounds their screen-space
/// error; 1.5 target pixels is under two full-resolution pixels at the cheap
/// quality level and stays below the cloud field's own detail scale.
pub(crate) const AMORTISATION_BUDGET_PX: f32 = 1.5;

/// Column-major 3x3 homography mapping a UV in the `current` view-projection
/// back to the UV the same far-plane direction had under `previous`.
///
/// Computed in f64: it composes an inverse projection with a projection, and
/// f32 loses too much of the far-plane direction for a distant cloud layer.
/// A cloud layer is not a plane, so this is exact only for rotation; the
/// residual translation parallax between the layer and the far plane is under
/// a pixel per frame at walking speed.
pub(crate) fn cloud_reprojection(
    previous: [[f32; 4]; 4],
    current_inverse: [[f32; 4]; 4],
) -> [[f32; 4]; 3] {
    // Both matrices are column-major, as WGSL and glam use them.
    let b = glam::DMat4::from_cols_array_2d(&current_inverse.map(|c| c.map(f64::from)));
    let a = glam::DMat4::from_cols_array_2d(&previous.map(|c| c.map(f64::from)));
    // A UV (u, v) reaches clip space as (2u - 1, 1 - 2v, 1, 1), which is the
    // linear combination u * (2, 0, 0, 0) + v * (0, -2, 0, 0) + (-1, 1, 1, 1).
    let columns = glam::DMat4::from_cols(
        2.0 * b.x_axis,
        -2.0 * b.y_axis,
        -b.x_axis + b.y_axis + b.z_axis + b.w_axis,
        glam::DVec4::ZERO,
    );
    let r = a * columns;
    // ndc back to UV is (x/2 + 1/2, 1/2 - y/2), applied to the homogeneous x/y
    // before the divide so the whole map stays one 3x3. Each of X, Y and W is a
    // row of `r`: a coefficient vector over (u, v, 1).
    let x = 0.5 * r.row(0) + 0.5 * r.row(3);
    let y = 0.5 * r.row(3) - 0.5 * r.row(1);
    let w = r.row(3);
    // Column-major 3x3: column j holds the (u, v, 1) coefficients of the j-th
    // input component.
    let mut out = [[0.0_f32; 4]; 3];
    out[0] = [x.x as f32, y.x as f32, w.x as f32, 0.0];
    out[1] = [x.y as f32, y.y as f32, w.y as f32, 0.0];
    out[2] = [x.z as f32, y.z as f32, w.z as f32, 0.0];
    out
}

/// Largest screen-space movement, in target pixels, that `homography` (current
/// UV -> reference UV) implies over a grid covering the target. This is the
/// error the amortised reuse would carry, so it decides when to recompute the
/// whole target instead of one sub-grid.
pub(crate) fn reprojection_displacement_px(homography: &[[f32; 4]; 3], target: (u32, u32)) -> f32 {
    // A 5x5 grid: the mapping is projective, so the corners carry most of the
    // motion, and the centre catches a pure zoom.
    let mut worst = 0.0_f32;
    for j in 0..5 {
        for i in 0..5 {
            let u = i as f32 / 4.0;
            let v = j as f32 / 4.0;
            let x = homography[0][0] * u + homography[1][0] * v + homography[2][0];
            let y = homography[0][1] * u + homography[1][1] * v + homography[2][1];
            let w = homography[0][2] * u + homography[1][2] * v + homography[2][2];
            if w.abs() < 1.0e-9 {
                return f32::INFINITY;
            }
            let du = (x / w - u) * target.0 as f32;
            let dv = (y / w - v) * target.1 as f32;
            worst = worst.max(du.hypot(dv));
        }
    }
    worst
}

/// Per-frame counts from the sky dome and the cloud march. Every field is zero
/// unless the counters were enabled for the frame that produced it; the whole
/// value is absent when they were not.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CostCounters {
    /// Cloud-target rays that cleared the slab and horizon early-outs, which is
    /// exactly what the pre-mask shader marched on those pixels.
    pub cloud_candidates: u32,
    /// Candidates the depth mask and the amortised sub-grid let through to the
    /// view march.
    pub cloud_marched_pixels: u32,
    /// View march steps those rays actually executed, including the early exit
    /// once the ray is opaque.
    pub cloud_march_steps: u32,
    /// Candidates skipped because opaque scene geometry covers every
    /// full-resolution texel of their footprint.
    pub cloud_masked_pixels: u32,
    /// Candidates skipped by the interleaved amortisation and kept from the
    /// previous frame's target.
    pub cloud_reused_pixels: u32,
    /// Sky dome fragments that reached the shading path.
    pub sky_shaded_pixels: u32,
}

/// Colour format of the offscreen target. Vulkan requires optimal-tiling
/// R8G8B8A8_UNORM to support colour attachment, sampling and linear filtering,
/// so no fallback is needed; support is still asserted rather than assumed.
const TARGET_FORMAT: vk::Format = vk::Format::R8G8B8A8_UNORM;

/// One full-screen pipeline request: both stages, the fragment entry, and the
/// two state choices that differ between the three full-screen draws.
struct FullScreenPipeline {
    vertex_spirv: &'static [u8],
    fragment_spirv: &'static [u8],
    fragment_entry: &'static std::ffi::CStr,
    /// Premultiplied cloud light (blend on) or an opaque background (off).
    premultiplied_blend: bool,
    /// Depth compare EQUAL against the cleared far value, with the early test.
    depth_equal: bool,
}

fn full_screen_pipeline(
    device: &Device,
    layout: vk::PipelineLayout,
    pass: vk::RenderPass,
    request: &FullScreenPipeline,
) -> Result<vk::Pipeline> {
    let FullScreenPipeline {
        vertex_spirv,
        fragment_spirv,
        fragment_entry,
        premultiplied_blend,
        depth_equal,
    } = *request;
    let mut modules = Vec::new();
    // SAFETY: build-time Naga validates every module; the modules outlive pipeline
    // creation and are destroyed on both paths. Slices stay live across the call.
    unsafe {
        let result = (|| {
            for bytes in [vertex_spirv, fragment_spirv] {
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
            let stages = [
                vk::PipelineShaderStageCreateInfo::default()
                    .stage(vk::ShaderStageFlags::VERTEX)
                    .module(modules[0])
                    .name(c"vs_main"),
                vk::PipelineShaderStageCreateInfo::default()
                    .stage(vk::ShaderStageFlags::FRAGMENT)
                    .module(modules[1])
                    .name(fragment_entry),
            ];
            // No vertex buffers: the vertex stage derives one covering triangle
            // from its index.
            let vertex = vk::PipelineVertexInputStateCreateInfo::default();
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
            // Neither draw writes depth. The dome and the composite do test it,
            // against the far value a cleared depth buffer still holds where no
            // opaque surface was drawn: EQUAL passes exactly on the pixels the
            // scene left empty, and the early fragment test keeps the shader from
            // running on the covered ones. The march has no depth attachment at
            // all, so it tests nothing.
            let depth = vk::PipelineDepthStencilStateCreateInfo::default()
                .depth_test_enable(depth_equal)
                .depth_write_enable(false)
                .depth_compare_op(if depth_equal {
                    vk::CompareOp::EQUAL
                } else {
                    vk::CompareOp::LESS
                });
            // The cloud target holds premultiplied light with 1 - transmittance
            // in alpha, so the composite is ONE / ONE_MINUS_SRC_ALPHA. The dome
            // itself is opaque background and does not blend.
            let attachments = [vk::PipelineColorBlendAttachmentState::default()
                .blend_enable(premultiplied_blend)
                .src_color_blend_factor(vk::BlendFactor::ONE)
                .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
                .color_blend_op(vk::BlendOp::ADD)
                .src_alpha_blend_factor(vk::BlendFactor::ONE)
                .dst_alpha_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
                .alpha_blend_op(vk::BlendOp::ADD)
                .color_write_mask(vk::ColorComponentFlags::RGBA)];
            let blend = vk::PipelineColorBlendStateCreateInfo::default().attachments(&attachments);
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

/// The reduced-resolution cloud target and the two pipelines that write and
/// read it. Owned by [`SkyPass`]; dropped whenever the plan changes.
struct CloudTarget {
    device: Arc<Device>,
    plan: CloudPlan,
    image: vk::Image,
    memory: vk::DeviceMemory,
    view: vk::ImageView,
    sampler: vk::Sampler,
    /// Steady state: load the previous target so an amortised frame can keep the
    /// pixels it did not re-march.
    pass: vk::RenderPass,
    frame: vk::Framebuffer,
    /// First use of a new target: there is nothing to load yet, and the image is
    /// still UNDEFINED, so this pass clears it instead.
    first_pass: vk::RenderPass,
    first_frame: vk::Framebuffer,
    march: vk::Pipeline,
    set_layout: vk::DescriptorSetLayout,
    pool: vk::DescriptorPool,
    set: vk::DescriptorSet,
    composite_layout: vk::PipelineLayout,
    composite: vk::Pipeline,
}

impl Drop for CloudTarget {
    fn drop(&mut self) {
        // SAFETY: the owner waited the frame fence before dropping this, so no
        // submitted command buffer references any of these handles.
        unsafe {
            let d = &self.device.raw;
            d.destroy_pipeline(self.composite, None);
            d.destroy_pipeline_layout(self.composite_layout, None);
            d.destroy_pipeline(self.march, None);
            d.destroy_descriptor_pool(self.pool, None);
            d.destroy_descriptor_set_layout(self.set_layout, None);
            d.destroy_framebuffer(self.frame, None);
            d.destroy_render_pass(self.pass, None);
            d.destroy_framebuffer(self.first_frame, None);
            d.destroy_render_pass(self.first_pass, None);
            d.destroy_sampler(self.sampler, None);
            d.destroy_image_view(self.view, None);
            d.destroy_image(self.image, None);
            d.free_memory(self.memory, None);
        }
    }
}

impl CloudTarget {
    fn new(
        device: Arc<Device>,
        march_layout: vk::PipelineLayout,
        frame_pass: vk::RenderPass,
        plan: CloudPlan,
    ) -> Result<Self> {
        let mut out = Self {
            device,
            plan,
            image: vk::Image::null(),
            memory: vk::DeviceMemory::null(),
            view: vk::ImageView::null(),
            sampler: vk::Sampler::null(),
            pass: vk::RenderPass::null(),
            frame: vk::Framebuffer::null(),
            first_pass: vk::RenderPass::null(),
            first_frame: vk::Framebuffer::null(),
            march: vk::Pipeline::null(),
            set_layout: vk::DescriptorSetLayout::null(),
            pool: vk::DescriptorPool::null(),
            set: vk::DescriptorSet::null(),
            composite_layout: vk::PipelineLayout::null(),
            composite: vk::Pipeline::null(),
        };
        // SAFETY: every handle is stored in this partial-construction guard as it
        // is created, so an error frees exactly what was made. Format support is
        // queried; the descriptor references the image this struct owns.
        unsafe {
            let d = &out.device;
            let required = vk::FormatFeatureFlags::COLOR_ATTACHMENT
                | vk::FormatFeatureFlags::SAMPLED_IMAGE
                | vk::FormatFeatureFlags::SAMPLED_IMAGE_FILTER_LINEAR;
            if !d
                .instance
                .raw
                .get_physical_device_format_properties(d.physical, TARGET_FORMAT)
                .optimal_tiling_features
                .contains(required)
            {
                return Err("No filterable colour attachment format for the cloud target".into());
            }
            let extent = vk::Extent2D {
                width: plan.width,
                height: plan.height,
            };
            out.image = d
                .raw
                .create_image(
                    &vk::ImageCreateInfo::default()
                        .image_type(vk::ImageType::TYPE_2D)
                        .format(TARGET_FORMAT)
                        .extent(vk::Extent3D {
                            width: extent.width,
                            height: extent.height,
                            depth: 1,
                        })
                        .mip_levels(1)
                        .array_layers(1)
                        .samples(vk::SampleCountFlags::TYPE_1)
                        .tiling(vk::ImageTiling::OPTIMAL)
                        .usage(vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::SAMPLED)
                        .sharing_mode(vk::SharingMode::EXCLUSIVE),
                    None,
                )
                .map_err(err)?;
            let req = d.raw.get_image_memory_requirements(out.image);
            let ty = d.memory_type(req.memory_type_bits, vk::MemoryPropertyFlags::DEVICE_LOCAL)?;
            out.memory = d
                .raw
                .allocate_memory(
                    &vk::MemoryAllocateInfo::default()
                        .allocation_size(req.size)
                        .memory_type_index(ty),
                    None,
                )
                .map_err(err)?;
            d.raw
                .bind_image_memory(out.image, out.memory, 0)
                .map_err(err)?;
            out.view = d
                .raw
                .create_image_view(
                    &vk::ImageViewCreateInfo::default()
                        .image(out.image)
                        .view_type(vk::ImageViewType::TYPE_2D)
                        .format(TARGET_FORMAT)
                        .subresource_range(
                            vk::ImageSubresourceRange::default()
                                .aspect_mask(vk::ImageAspectFlags::COLOR)
                                .level_count(1)
                                .layer_count(1),
                        ),
                    None,
                )
                .map_err(err)?;
            out.sampler = d
                .raw
                .create_sampler(
                    &vk::SamplerCreateInfo::default()
                        .mag_filter(vk::Filter::LINEAR)
                        .min_filter(vk::Filter::LINEAR)
                        .mipmap_mode(vk::SamplerMipmapMode::NEAREST)
                        .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                        .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                        .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                        .min_lod(0.)
                        .max_lod(0.),
                    None,
                )
                .map_err(err)?;
            // One colour attachment, left in shader-read layout for the
            // composite in the frame's own pass. Two descriptions: the first use
            // of a new image must start from UNDEFINED and clear (there is
            // nothing valid to load); every later frame loads the pixels it did
            // not re-march.
            let steady = [vk::AttachmentDescription::default()
                .format(TARGET_FORMAT)
                .samples(vk::SampleCountFlags::TYPE_1)
                .load_op(vk::AttachmentLoadOp::LOAD)
                .store_op(vk::AttachmentStoreOp::STORE)
                .initial_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                .final_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
            let first = [vk::AttachmentDescription::default()
                .format(TARGET_FORMAT)
                .samples(vk::SampleCountFlags::TYPE_1)
                .load_op(vk::AttachmentLoadOp::CLEAR)
                .store_op(vk::AttachmentStoreOp::STORE)
                .initial_layout(vk::ImageLayout::UNDEFINED)
                .final_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
            let color = [vk::AttachmentReference {
                attachment: 0,
                layout: vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
            }];
            let subpasses = [vk::SubpassDescription::default()
                .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
                .color_attachments(&color)];
            let dependencies = [
                // The previous frame's composite must finish sampling before
                // this pass overwrites the target.
                vk::SubpassDependency::default()
                    .src_subpass(vk::SUBPASS_EXTERNAL)
                    .dst_subpass(0)
                    .src_stage_mask(vk::PipelineStageFlags::FRAGMENT_SHADER)
                    .dst_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
                    .src_access_mask(vk::AccessFlags::SHADER_READ)
                    .dst_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE),
                // And this pass's writes must be visible to that sampling.
                vk::SubpassDependency::default()
                    .src_subpass(0)
                    .dst_subpass(vk::SUBPASS_EXTERNAL)
                    .src_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
                    .dst_stage_mask(vk::PipelineStageFlags::FRAGMENT_SHADER)
                    .src_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
                    .dst_access_mask(vk::AccessFlags::SHADER_READ),
                // This pass samples the frame depth, which the frame pass wrote
                // earlier in the same command buffer (and which it also clears at
                // the start of the next frame), so make that write visible here
                // and the read complete before the next write.
                vk::SubpassDependency::default()
                    .src_subpass(vk::SUBPASS_EXTERNAL)
                    .dst_subpass(0)
                    .src_stage_mask(vk::PipelineStageFlags::LATE_FRAGMENT_TESTS)
                    .dst_stage_mask(vk::PipelineStageFlags::FRAGMENT_SHADER)
                    .src_access_mask(vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE)
                    .dst_access_mask(vk::AccessFlags::SHADER_READ),
            ];
            for (pass, attachments) in [&mut out.pass, &mut out.first_pass]
                .into_iter()
                .zip([&steady, &first])
            {
                *pass = d
                    .raw
                    .create_render_pass(
                        &vk::RenderPassCreateInfo::default()
                            .attachments(attachments)
                            .subpasses(&subpasses)
                            .dependencies(&dependencies),
                        None,
                    )
                    .map_err(err)?;
            }
            for (framebuffer, pass) in [&mut out.frame, &mut out.first_frame]
                .into_iter()
                .zip([out.pass, out.first_pass])
            {
                *framebuffer = d
                    .raw
                    .create_framebuffer(
                        &vk::FramebufferCreateInfo::default()
                            .render_pass(pass)
                            .attachments(&[out.view])
                            .width(extent.width)
                            .height(extent.height)
                            .layers(1),
                        None,
                    )
                    .map_err(err)?;
            }
            out.march = full_screen_pipeline(
                d,
                march_layout,
                out.pass,
                &FullScreenPipeline {
                    vertex_spirv: include_bytes!(concat!(env!("OUT_DIR"), "/sky.vs_main.spv")),
                    fragment_spirv: include_bytes!(concat!(env!("OUT_DIR"), "/sky.fs_clouds.spv")),
                    fragment_entry: c"fs_clouds",
                    premultiplied_blend: false,
                    depth_equal: false,
                },
            )?;
            // Composite set: the cloud target and its sampler, as separate
            // bindings, matching what Naga emits for `cloud_composite.wgsl`.
            let types = [
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
            let images = [vk::DescriptorImageInfo::default()
                .image_view(out.view)
                .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
            let samplers = [vk::DescriptorImageInfo::default().sampler(out.sampler)];
            d.raw.update_descriptor_sets(
                &[
                    vk::WriteDescriptorSet::default()
                        .dst_set(out.set)
                        .dst_binding(0)
                        .descriptor_type(types[0])
                        .image_info(&images),
                    vk::WriteDescriptorSet::default()
                        .dst_set(out.set)
                        .dst_binding(1)
                        .descriptor_type(types[1])
                        .image_info(&samplers),
                ],
                &[],
            );
            out.composite_layout = d
                .raw
                .create_pipeline_layout(
                    &vk::PipelineLayoutCreateInfo::default()
                        .set_layouts(&[out.set_layout])
                        .push_constant_ranges(&[vk::PushConstantRange::default()
                            .stage_flags(vk::ShaderStageFlags::FRAGMENT)
                            .offset(0)
                            .size(COMPOSITE_PUSH_BYTES)]),
                    None,
                )
                .map_err(err)?;
            out.composite = full_screen_pipeline(
                d,
                out.composite_layout,
                frame_pass,
                &FullScreenPipeline {
                    vertex_spirv: include_bytes!(concat!(
                        env!("OUT_DIR"),
                        "/cloud_composite.vs_main.spv"
                    )),
                    fragment_spirv: include_bytes!(concat!(
                        env!("OUT_DIR"),
                        "/cloud_composite.fs_main.spv"
                    )),
                    fragment_entry: c"fs_main",
                    premultiplied_blend: true,
                    depth_equal: true,
                },
            )?;
        }
        Ok(out)
    }
}

/// Everything about this frame's surface that the sky pass is built from. A
/// change in any of it rebuilds the pass: the pipelines name the render pass,
/// the target follows the extent, and the march samples the depth view.
pub(crate) struct SkyFrame {
    pub device: Arc<Device>,
    pub lighting_set_layout: vk::DescriptorSetLayout,
    pub frame_pass: vk::RenderPass,
    pub extent: vk::Extent2D,
    pub depth_view: vk::ImageView,
}

/// The sky dome pipeline, plus the cloud target while clouds are enabled.
/// Rebuilt when the frame's render pass or extent changes; both are inputs to
/// the pipelines and the target it owns.
pub(crate) struct SkyPass {
    device: Arc<Device>,
    frame_pass: vk::RenderPass,
    frame: vk::Extent2D,
    dome_pipe_layout: vk::PipelineLayout,
    march_pipe_layout: vk::PipelineLayout,
    dome: vk::Pipeline,
    clouds: Option<CloudTarget>,
    counters: Buffer,
    dome_set_layout: vk::DescriptorSetLayout,
    march_set_layout: vk::DescriptorSetLayout,
    pool: vk::DescriptorPool,
    dome_set: vk::DescriptorSet,
    march_set: vk::DescriptorSet,
}

impl Drop for SkyPass {
    fn drop(&mut self) {
        // SAFETY: the owner waited the frame fence before dropping this.
        unsafe {
            self.clouds.take();
            self.device.raw.destroy_pipeline(self.dome, None);
            self.device
                .raw
                .destroy_pipeline_layout(self.dome_pipe_layout, None);
            self.device
                .raw
                .destroy_pipeline_layout(self.march_pipe_layout, None);
            self.device.raw.destroy_descriptor_pool(self.pool, None);
            self.device
                .raw
                .destroy_descriptor_set_layout(self.dome_set_layout, None);
            self.device
                .raw
                .destroy_descriptor_set_layout(self.march_set_layout, None);
        }
    }
}

impl SkyPass {
    /// Bring `slot` in line with this frame's settings, pass and extent. The
    /// caller has already waited the frame fence, so anything replaced here is
    /// idle. Nothing is allocated for a frame that draws neither.
    /// Bring `slot` in line with this frame's settings, pass and extent, and
    /// report whether the pass or its cloud target was (re)built. A rebuilt
    /// target holds nothing: the composite for this frame must be skipped and
    /// the march has to run after the frame pass, because only that pass can
    /// give the freshly built target a depth buffer to mask against.
    pub(crate) fn ensure(
        slot: &mut Option<Self>,
        frame: SkyFrame,
        atmosphere: &Atmosphere,
        clouds: &Clouds,
    ) -> Result<bool> {
        let SkyFrame {
            device,
            lighting_set_layout,
            frame_pass,
            extent,
            depth_view,
        } = frame;
        if !atmosphere.sky_gradient && !clouds.enabled {
            *slot = None;
            return Ok(false);
        }
        let mut rebuilt = false;
        if slot
            .as_ref()
            .is_some_and(|s| s.frame_pass != frame_pass || s.frame != extent)
        {
            *slot = None;
        }
        if slot.is_none() {
            *slot = Some(Self::new(
                device.clone(),
                lighting_set_layout,
                frame_pass,
                extent,
                depth_view,
            )?);
            rebuilt = true;
        }
        let pass = slot.as_mut().expect("sky pass created");
        let plan = CloudPlan::new((extent.width, extent.height), clouds);
        if pass.clouds.as_ref().map(|target| target.plan) != plan {
            // Free first: one target at a time, and the old one is idle.
            pass.clouds = None;
            if let Some(plan) = plan {
                pass.clouds = Some(CloudTarget::new(
                    device,
                    pass.march_pipe_layout,
                    frame_pass,
                    plan,
                )?);
            }
            rebuilt = true;
        }
        Ok(rebuilt)
    }

    fn new(
        device: Arc<Device>,
        lighting_set_layout: vk::DescriptorSetLayout,
        frame_pass: vk::RenderPass,
        frame: vk::Extent2D,
        depth_view: vk::ImageView,
    ) -> Result<Self> {
        let counters = Buffer::new(
            device.clone(),
            &[0_u8; COUNTER_BYTES],
            vk::BufferUsageFlags::STORAGE_BUFFER,
        )?;
        let mut out = Self {
            device,
            frame_pass,
            frame,
            dome_pipe_layout: vk::PipelineLayout::null(),
            march_pipe_layout: vk::PipelineLayout::null(),
            dome: vk::Pipeline::null(),
            clouds: None,
            counters,
            dome_set_layout: vk::DescriptorSetLayout::null(),
            march_set_layout: vk::DescriptorSetLayout::null(),
            pool: vk::DescriptorPool::null(),
            dome_set: vk::DescriptorSet::null(),
            march_set: vk::DescriptorSet::null(),
        };
        // SAFETY: every handle is stored in this partial-construction guard as
        // it is created. The lighting set layout outlives this pass; the counter
        // buffer is owned here, and the depth view belongs to the swapchain the
        // renderer rebuilds this pass with.
        unsafe {
            let d = &out.device.raw;
            let counter_binding = vk::DescriptorSetLayoutBinding::default()
                .binding(0)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT);
            out.dome_set_layout = d
                .create_descriptor_set_layout(
                    &vk::DescriptorSetLayoutCreateInfo::default().bindings(&[counter_binding]),
                    None,
                )
                .map_err(err)?;
            // The march reads the frame depth as a texture at binding 1; the
            // dome must not have that binding in its layout, because in the
            // frame pass it is the depth attachment and cannot be sampled.
            let depth_binding = vk::DescriptorSetLayoutBinding::default()
                .binding(1)
                .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT);
            out.march_set_layout = d
                .create_descriptor_set_layout(
                    &vk::DescriptorSetLayoutCreateInfo::default()
                        .bindings(&[counter_binding, depth_binding]),
                    None,
                )
                .map_err(err)?;
            out.pool = d
                .create_descriptor_pool(
                    &vk::DescriptorPoolCreateInfo::default()
                        .max_sets(2)
                        .pool_sizes(&[
                            vk::DescriptorPoolSize {
                                ty: vk::DescriptorType::STORAGE_BUFFER,
                                descriptor_count: 2,
                            },
                            vk::DescriptorPoolSize {
                                ty: vk::DescriptorType::SAMPLED_IMAGE,
                                descriptor_count: 1,
                            },
                        ]),
                    None,
                )
                .map_err(err)?;
            let sets = d
                .allocate_descriptor_sets(
                    &vk::DescriptorSetAllocateInfo::default()
                        .descriptor_pool(out.pool)
                        .set_layouts(&[out.dome_set_layout, out.march_set_layout]),
                )
                .map_err(err)?;
            out.dome_set = sets[0];
            out.march_set = sets[1];
            let counter_info = [vk::DescriptorBufferInfo::default()
                .buffer(out.counters.raw)
                .range(COUNTER_BYTES as u64)];
            let depth_info = [vk::DescriptorImageInfo::default()
                .image_view(depth_view)
                .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
            d.update_descriptor_sets(
                &[
                    vk::WriteDescriptorSet::default()
                        .dst_set(out.dome_set)
                        .dst_binding(0)
                        .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                        .buffer_info(&counter_info),
                    vk::WriteDescriptorSet::default()
                        .dst_set(out.march_set)
                        .dst_binding(0)
                        .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                        .buffer_info(&counter_info),
                    vk::WriteDescriptorSet::default()
                        .dst_set(out.march_set)
                        .dst_binding(1)
                        .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                        .image_info(&depth_info),
                ],
                &[],
            );
            for (layout, set_layout) in [
                (&mut out.dome_pipe_layout, out.dome_set_layout),
                (&mut out.march_pipe_layout, out.march_set_layout),
            ] {
                *layout = out
                    .device
                    .raw
                    .create_pipeline_layout(
                        &vk::PipelineLayoutCreateInfo::default()
                            .set_layouts(&[lighting_set_layout, set_layout])
                            .push_constant_ranges(&[vk::PushConstantRange::default()
                                .stage_flags(
                                    vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT,
                                )
                                .offset(0)
                                .size(SKY_PUSH_BYTES)]),
                        None,
                    )
                    .map_err(err)?;
            }
        }
        out.dome = full_screen_pipeline(
            &out.device,
            out.dome_pipe_layout,
            frame_pass,
            &FullScreenPipeline {
                vertex_spirv: include_bytes!(concat!(env!("OUT_DIR"), "/sky.vs_main.spv")),
                fragment_spirv: include_bytes!(concat!(env!("OUT_DIR"), "/sky.fs_main.spv")),
                fragment_entry: c"fs_main",
                premultiplied_blend: false,
                depth_equal: true,
            },
        )?;
        Ok(out)
    }

    /// Offscreen extent, or `None` when no cloud target exists.
    pub(crate) fn cloud_extent(&self) -> Option<(u32, u32)> {
        self.clouds.as_ref().map(|t| (t.plan.width, t.plan.height))
    }

    pub(crate) fn cloud_quality(&self) -> Option<u8> {
        self.clouds.as_ref().map(|t| t.plan.quality)
    }

    /// The target plan of the currently allocated target, if any. The renderer
    /// compares it frame to frame to notice a target that has just been built.
    pub(crate) fn cloud_plan(&self) -> Option<CloudPlan> {
        self.clouds.as_ref().map(|t| t.plan)
    }

    /// Zero this frame's counters. Safe from the CPU only while the previous
    /// frame fence is waited, which the renderer does before every recording.
    pub(crate) fn reset_counters(&self) -> Result<()> {
        self.counters.write(&[0_u8; COUNTER_BYTES])
    }

    /// The counters written by the most recently completed submission.
    pub(crate) fn counters_snapshot(&self) -> Result<CostCounters> {
        let mut bytes = [0_u8; COUNTER_BYTES];
        self.counters.read(&mut bytes)?;
        let word = |i: usize| u32::from_ne_bytes(bytes[i..i + 4].try_into().expect("4 bytes"));
        Ok(CostCounters {
            cloud_candidates: word(0),
            cloud_marched_pixels: word(4),
            cloud_march_steps: word(8),
            cloud_masked_pixels: word(12),
            cloud_reused_pixels: word(16),
            sky_shaded_pixels: word(20),
        })
    }

    /// The volumetric march, recorded outside the frame's render pass. Returns
    /// whether a pass was recorded. `push` is completed here with the cloud
    /// target's own inverse size, so the march reconstructs the same rays the
    /// full-resolution dome does. `clear_first` is set on the first march into
    /// a newly built target: that pass clears instead of loading, because the
    /// image has no valid contents yet.
    pub(crate) fn record_clouds(
        &self,
        cmd: vk::CommandBuffer,
        lighting_set: vk::DescriptorSet,
        push: SkyPush,
        clear_first: bool,
    ) -> bool {
        let Some(target) = &self.clouds else {
            return false;
        };
        let extent = vk::Extent2D {
            width: target.plan.width,
            height: target.plan.height,
        };
        let (pass, framebuffer) = if clear_first {
            (target.first_pass, target.first_frame)
        } else {
            (target.pass, target.frame)
        };
        let mut push = push;
        push.params[2] = 1.0 / extent.width as f32;
        push.params[3] = 1.0 / extent.height as f32;
        // SAFETY: the caller is recording an idle command buffer; the target,
        // its framebuffer, pipeline and descriptor set live until its fence.
        unsafe {
            let d = &self.device.raw;
            let area = vk::Rect2D {
                offset: Default::default(),
                extent,
            };
            d.cmd_begin_render_pass(
                cmd,
                &vk::RenderPassBeginInfo::default()
                    .render_pass(pass)
                    .framebuffer(framebuffer)
                    .render_area(area)
                    .clear_values(&[vk::ClearValue {
                        color: vk::ClearColorValue { float32: [0.0; 4] },
                    }]),
                vk::SubpassContents::INLINE,
            );
            d.cmd_set_viewport(
                cmd,
                0,
                &[vk::Viewport {
                    x: 0.,
                    y: 0.,
                    width: extent.width as f32,
                    height: extent.height as f32,
                    min_depth: 0.,
                    max_depth: 1.,
                }],
            );
            d.cmd_set_scissor(cmd, 0, &[area]);
            d.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, target.march);
            d.cmd_bind_descriptor_sets(
                cmd,
                vk::PipelineBindPoint::GRAPHICS,
                self.march_pipe_layout,
                0,
                &[lighting_set],
                &[],
            );
            d.cmd_bind_descriptor_sets(
                cmd,
                vk::PipelineBindPoint::GRAPHICS,
                self.march_pipe_layout,
                1,
                &[self.march_set],
                &[],
            );
            d.cmd_push_constants(
                cmd,
                self.march_pipe_layout,
                vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT,
                0,
                bytemuck::bytes_of(&push),
            );
            d.cmd_draw(cmd, 3, 1, 0, 0);
            d.cmd_end_render_pass(cmd);
        }
        true
    }

    /// The dome, recorded inside the frame's render pass after opaque geometry
    /// and before the cloud composite. `push.params.zw` must already be the
    /// frame's inverse size.
    pub(crate) fn record_dome(
        &self,
        cmd: vk::CommandBuffer,
        lighting_set: vk::DescriptorSet,
        push: SkyPush,
    ) {
        // SAFETY: recorded inside the caller's render pass with the viewport and
        // scissor it already set; every handle outlives the submission.
        unsafe {
            let d = &self.device.raw;
            d.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, self.dome);
            d.cmd_bind_descriptor_sets(
                cmd,
                vk::PipelineBindPoint::GRAPHICS,
                self.dome_pipe_layout,
                0,
                &[lighting_set],
                &[],
            );
            d.cmd_bind_descriptor_sets(
                cmd,
                vk::PipelineBindPoint::GRAPHICS,
                self.dome_pipe_layout,
                1,
                &[self.dome_set],
                &[],
            );
            d.cmd_push_constants(
                cmd,
                self.dome_pipe_layout,
                vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT,
                0,
                bytemuck::bytes_of(&push),
            );
            d.cmd_draw(cmd, 3, 1, 0, 0);
        }
    }

    /// The upsample, recorded immediately after the dome and before the water
    /// and HUD passes. Does nothing when no cloud target exists.
    pub(crate) fn record_composite(&self, cmd: vk::CommandBuffer) {
        let Some(target) = &self.clouds else {
            return;
        };
        let push = CompositePush {
            texel: [
                1.0 / self.frame.width.max(1) as f32,
                1.0 / self.frame.height.max(1) as f32,
                1.0 / target.plan.width as f32,
                1.0 / target.plan.height as f32,
            ],
        };
        // SAFETY: recorded inside the caller's render pass; the descriptor set
        // points at the target this struct owns until its fence completes.
        unsafe {
            let d = &self.device.raw;
            d.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, target.composite);
            d.cmd_bind_descriptor_sets(
                cmd,
                vk::PipelineBindPoint::GRAPHICS,
                target.composite_layout,
                0,
                &[target.set],
                &[],
            );
            d.cmd_push_constants(
                cmd,
                target.composite_layout,
                vk::ShaderStageFlags::FRAGMENT,
                0,
                bytemuck::bytes_of(&push),
            );
            d.cmd_draw(cmd, 3, 1, 0, 0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn defaults_are_off_and_cheapest() {
        let clouds = Clouds::default();
        assert!(!clouds.enabled);
        assert_eq!(clouds.quality, 0);
        assert_eq!(clouds.time_s, 0.0);
        clouds.validate().unwrap();
    }

    #[test]
    fn quality_and_time_are_validated() {
        for quality in 0..=Clouds::MAX_QUALITY {
            Clouds {
                enabled: true,
                quality,
                time_s: 12.0,
            }
            .validate()
            .unwrap();
        }
        assert!(Clouds {
            enabled: true,
            quality: Clouds::MAX_QUALITY + 1,
            time_s: 0.0,
        }
        .validate()
        .is_err());
        for time_s in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert!(Clouds {
                enabled: true,
                quality: 0,
                time_s,
            }
            .validate()
            .is_err());
        }
    }

    #[test]
    fn step_counts_are_bounded_and_rise_with_quality() {
        let cheap = Clouds {
            enabled: true,
            quality: 0,
            time_s: 0.0,
        };
        let quality = Clouds {
            quality: 1,
            ..cheap
        };
        assert!(cheap.march_steps() < quality.march_steps());
        assert!(cheap.light_steps() < quality.light_steps());
        assert!(quality.march_steps() <= 64 && quality.light_steps() <= 6);
        // Neither level renders at full resolution.
        assert!(cheap.divisor() >= 2 && quality.divisor() >= 2);
        assert!(cheap.divisor() > quality.divisor());
    }

    #[test]
    fn the_clock_folds_onto_the_lattice_period() {
        let clouds = Clouds {
            enabled: true,
            quality: 0,
            time_s: Clouds::TIME_PERIOD_S + 5.0,
        };
        assert_eq!(clouds.time(), 5.0);
        let negative = Clouds {
            time_s: -1.0,
            ..clouds
        };
        assert_eq!(negative.time(), Clouds::TIME_PERIOD_S - 1.0);
    }

    #[test]
    fn a_disabled_or_empty_frame_plans_no_target() {
        assert_eq!(CloudPlan::new((1920, 1080), &Clouds::default()), None);
        let on = Clouds {
            enabled: true,
            quality: 0,
            time_s: 0.0,
        };
        assert_eq!(CloudPlan::new((0, 1080), &on), None);
        assert_eq!(CloudPlan::new((1920, 0), &on), None);
    }

    #[test]
    fn the_plan_is_a_fraction_of_the_frame_and_never_empty() {
        let cheap = Clouds {
            enabled: true,
            quality: 0,
            time_s: 0.0,
        };
        let quality = Clouds {
            quality: 1,
            ..cheap
        };
        // The phone's panel: quarter and half resolution, rounded up.
        assert_eq!(
            CloudPlan::new((3168, 1440), &cheap),
            Some(CloudPlan {
                width: 792,
                height: 360,
                quality: 0
            })
        );
        assert_eq!(
            CloudPlan::new((3168, 1440), &quality),
            Some(CloudPlan {
                width: 1584,
                height: 720,
                quality: 1
            })
        );
        // A one-pixel frame still plans a one-pixel target, never a zero extent.
        assert_eq!(
            CloudPlan::new((1, 1), &cheap),
            Some(CloudPlan {
                width: 1,
                height: 1,
                quality: 0
            })
        );
    }

    #[test]
    fn a_plan_changes_exactly_when_the_target_must_be_rebuilt() {
        let cheap = Clouds {
            enabled: true,
            quality: 0,
            time_s: 0.0,
        };
        let moved_clock = Clouds {
            time_s: 900.0,
            ..cheap
        };
        // Animation alone never rebuilds the target.
        assert_eq!(
            CloudPlan::new((1280, 768), &cheap),
            CloudPlan::new((1280, 768), &moved_clock)
        );
        // A resize and a quality change both do.
        assert_ne!(
            CloudPlan::new((1280, 768), &cheap),
            CloudPlan::new((800, 600), &cheap)
        );
        assert_ne!(
            CloudPlan::new((1280, 768), &cheap),
            CloudPlan::new(
                (1280, 768),
                &Clouds {
                    quality: 1,
                    ..cheap
                }
            )
        );
    }

    #[test]
    fn push_constants_fit_the_guaranteed_range() {
        assert_eq!(size_of::<SkyPush>(), SKY_PUSH_BYTES as usize);
        assert_eq!(size_of::<CompositePush>(), COMPOSITE_PUSH_BYTES as usize);
        // 128 bytes is the range every Vulkan implementation guarantees.
        const { assert!(SKY_PUSH_BYTES <= 128 && COMPOSITE_PUSH_BYTES <= 128) };
    }

    fn look(eye: glam::Vec3, target: glam::Vec3) -> glam::Mat4 {
        glam::Mat4::perspective_rh(70_f32.to_radians(), 1.0, 0.2, 20_000.0)
            * glam::Mat4::look_at_rh(eye, target, glam::Vec3::Y)
    }

    /// Apply the stored column-major homography to a UV the way the shader does.
    fn apply(homography: &[[f32; 4]; 3], uv: (f32, f32)) -> (f32, f32) {
        let x = homography[0][0] * uv.0 + homography[1][0] * uv.1 + homography[2][0];
        let y = homography[0][1] * uv.0 + homography[1][1] * uv.1 + homography[2][1];
        let w = homography[0][2] * uv.0 + homography[1][2] * uv.1 + homography[2][2];
        (x / w, y / w)
    }

    #[test]
    fn the_reprojection_homography_matches_an_unprojection_for_every_uv() {
        let current = look(
            glam::Vec3::new(5.0, 60.0, 0.0),
            glam::Vec3::new(5.0, 60.0, -1.0),
        );
        let previous = look(
            glam::Vec3::new(0.0, 60.0, 0.0),
            glam::Vec3::new(4.0, 64.0, -1.0),
        );
        let inverse = current.inverse();
        let homography =
            cloud_reprojection(previous.to_cols_array_2d(), inverse.to_cols_array_2d());
        // The oracle runs in f64 too: the far-plane point is 20 km out, and f32
        // roundoff there is larger than the tolerance being checked.
        let previous =
            glam::DMat4::from_cols_array_2d(&previous.to_cols_array_2d().map(|c| c.map(f64::from)));
        let inverse =
            glam::DMat4::from_cols_array_2d(&inverse.to_cols_array_2d().map(|c| c.map(f64::from)));
        for j in 0..5 {
            for i in 0..5 {
                let uv = (i as f32 / 4.0, j as f32 / 4.0);
                // The shader's ray convention: ndc = (2u - 1, 1 - 2v) at far depth.
                let point = inverse
                    * glam::DVec4::new(
                        f64::from(uv.0) * 2.0 - 1.0,
                        1.0 - f64::from(uv.1) * 2.0,
                        1.0,
                        1.0,
                    );
                let clip = previous * point;
                let expected = ((clip.x / clip.w) * 0.5 + 0.5, 0.5 - (clip.y / clip.w) * 0.5);
                let got = apply(&homography, uv);
                assert!(
                    (f64::from(got.0) - expected.0).abs() < 1.0e-4
                        && (f64::from(got.1) - expected.1).abs() < 1.0e-4,
                    "uv {uv:?}: {got:?} vs {expected:?}"
                );
            }
        }
    }

    #[test]
    fn a_stationary_camera_reprojects_to_itself_and_a_turn_is_visible() {
        let eye = glam::Vec3::new(0.0, 60.0, 0.0);
        let still = look(eye, glam::Vec3::new(0.0, 60.0, -1.0));
        let inverse = still.inverse();
        let identity = cloud_reprojection(still.to_cols_array_2d(), inverse.to_cols_array_2d());
        assert!(reprojection_displacement_px(&identity, (792, 360)) < 0.01);

        let turned = look(eye, glam::Vec3::new(5_f32.to_radians().tan(), 60.0, -1.0));
        let turn = cloud_reprojection(turned.to_cols_array_2d(), inverse.to_cols_array_2d());
        let pixels = reprojection_displacement_px(&turn, (792, 360));
        // Five degrees of a 70 degree view is about 1/14 of the width.
        assert!(
            (20.0..100.0).contains(&pixels),
            "five degree turn measured {pixels} target pixels"
        );
    }
}
