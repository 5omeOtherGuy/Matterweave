//! Reduced-resolution world target and the present pass that upscales it.
//!
//! The world, water, sky dome and cloud composite are rasterised into this
//! target at `scale` times the swapchain extent; a separate colour-only present
//! pass samples it back to full resolution and then draws the HUD, so text and
//! every geometry decision stay at their existing resolution. At scale 1.0 the
//! renderer skips this module entirely and draws straight into the swapchain:
//! the native path allocates nothing and changes no pixel.
//!
//! The target owns its own render pass and world/water pipelines because a
//! pipeline is created against a render pass, and this pass has a different
//! attachment set (colour stored for sampling, depth optionally stored) than
//! the swapchain's. Its depth range covers the swapchain's own selection, so a
//! driver that can sample one can sample the other.
//!
//! The upscale is sharp bilinear rather than plain bilinear: a coordinate warp
//! keeps a one-pixel target step a step instead of blending its contrast across
//! two output pixels. `UPSCALE_SNAP` carries the measured value and the trade
//! against sub-pixel stability.
use super::{err, pipeline, Depth, Device, PipelineKind, Result};
use crate::clouds::{full_screen_pipeline, FullScreenPipeline};
use ash::vk;
use std::sync::Arc;

/// Lowest scale the renderer accepts. This is a bandwidth/quality floor, not a
/// technical limit: below it the world target stops looking like a smaller
/// render of the frame and starts looking like a smaller image.
pub(crate) const MIN_RENDER_SCALE: f32 = 0.4;
/// The scale that selects the untouched native path.
pub(crate) const MAX_RENDER_SCALE: f32 = 1.0;

/// Plateau fraction of the sharp-bilinear upscale: the share of each target
/// texel's half-width the sample takes whole. The remaining `0.5 * (1 - value)`
/// of a target pixel is the transition band around the texel boundary.
///
/// The value is measured on the device-style host crop (`400,600,3168,720` of a
/// 3168x1440 present frame with clouds low, the phone's own distant band).
/// Distant-band K1 at scale 0.8 against a native 0.0268 and a gate of 0.020:
/// plain bilinear 0.0162, 0.5 plateau 0.0196, 0.75 0.0222, nearest 0.0258.
/// The worst per-pixel change for a 1/16-target-texel camera motion over the
/// same candidates is 0.06, 0.13, 0.25 and 1.0 of the local step contrast.
/// 0.75 is the snappiest single-sample setting that both clears the gate and
/// holds the sub-pixel change under a quarter step; nearest clears it best but
/// snaps an edge by a whole present pixel. See the PR for the table.
pub(crate) const UPSCALE_SNAP: f32 = 0.75;

/// The world extent a requested swapchain extent renders at for `scale`.
/// Rounded to the nearest pixel, never zero, so a degenerate surface and a
/// very small scale both still produce a real target.
pub(crate) fn scaled_extent(full: vk::Extent2D, scale: f32) -> vk::Extent2D {
    let scaled = |value: u32| ((value as f64 * scale as f64).round() as u32).max(1);
    vk::Extent2D {
        width: scaled(full.width),
        height: scaled(full.height),
    }
}

/// The render pass, pipelines and colour image of one reduced-resolution world
/// target, plus the upscale pipeline that presents it. Rebuilt when the
/// swapchain extent, the requested scale, the swapchain format or the
/// depth-store need changes; all of those are inputs to its pass or pipelines.
pub(crate) struct ScaledTarget {
    device: Arc<Device>,
    pub extent: vk::Extent2D,
    pub format: vk::Format,
    /// Whether this target's depth range is stored for the cloud march. False is
    /// the bandwidth choice: with no cloud target nothing samples it after the
    /// pass, so the depth never has to leave the tile.
    pub store_depth: bool,
    image: vk::Image,
    memory: vk::DeviceMemory,
    view: vk::ImageView,
    pub depth: Option<Depth>,
    pub pass: vk::RenderPass,
    pub frame: vk::Framebuffer,
    pub world: vk::Pipeline,
    pub water: vk::Pipeline,
    sampler: vk::Sampler,
    upscale_set_layout: vk::DescriptorSetLayout,
    upscale_pool: vk::DescriptorPool,
    upscale_set: vk::DescriptorSet,
    pub upscale_layout: vk::PipelineLayout,
    pub upscale: vk::Pipeline,
}

impl Drop for ScaledTarget {
    fn drop(&mut self) {
        // SAFETY: the renderer waited the frame fence before replacing this;
        // pipelines and framebuffer are destroyed before the image they name.
        unsafe {
            let d = &self.device.raw;
            d.destroy_pipeline(self.upscale, None);
            d.destroy_pipeline_layout(self.upscale_layout, None);
            d.destroy_descriptor_pool(self.upscale_pool, None);
            d.destroy_descriptor_set_layout(self.upscale_set_layout, None);
            d.destroy_sampler(self.sampler, None);
            d.destroy_pipeline(self.water, None);
            d.destroy_pipeline(self.world, None);
            d.destroy_framebuffer(self.frame, None);
            d.destroy_render_pass(self.pass, None);
            self.depth.take();
            d.destroy_image_view(self.view, None);
            d.destroy_image(self.image, None);
            d.free_memory(self.memory, None);
        }
    }
}

impl ScaledTarget {
    /// `pass` is the swapchain's colour-only present pass the upscale pipeline
    /// is created against; `layout` is the swapchain pipeline layout the world
    /// and water pipelines share with the native path.
    pub(crate) fn new(
        device: Arc<Device>,
        layout: vk::PipelineLayout,
        present_pass: vk::RenderPass,
        extent: vk::Extent2D,
        format: vk::Format,
        store_depth: bool,
    ) -> Result<Self> {
        let mut out = Self {
            device,
            extent,
            format,
            store_depth,
            image: vk::Image::null(),
            memory: vk::DeviceMemory::null(),
            view: vk::ImageView::null(),
            depth: None,
            pass: vk::RenderPass::null(),
            frame: vk::Framebuffer::null(),
            world: vk::Pipeline::null(),
            water: vk::Pipeline::null(),
            sampler: vk::Sampler::null(),
            upscale_set_layout: vk::DescriptorSetLayout::null(),
            upscale_pool: vk::DescriptorPool::null(),
            upscale_set: vk::DescriptorSet::null(),
            upscale_layout: vk::PipelineLayout::null(),
            upscale: vk::Pipeline::null(),
        };
        // SAFETY: every handle is stored in this partial-construction guard as
        // it is created, so an error destroys exactly what was made. Format
        // support is queried; descriptors reference the image this struct owns.
        unsafe {
            let d = &out.device;
            let required = vk::FormatFeatureFlags::COLOR_ATTACHMENT
                | vk::FormatFeatureFlags::SAMPLED_IMAGE
                | vk::FormatFeatureFlags::SAMPLED_IMAGE_FILTER_LINEAR;
            if !d
                .instance
                .raw
                .get_physical_device_format_properties(d.physical, format)
                .optimal_tiling_features
                .contains(required)
            {
                return Err(format!(
                    "Swapchain format {format:?} cannot be a filterable world render target"
                ));
            }
            out.image = d
                .raw
                .create_image(
                    &vk::ImageCreateInfo::default()
                        .image_type(vk::ImageType::TYPE_2D)
                        .format(format)
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
                        .format(format)
                        .subresource_range(
                            vk::ImageSubresourceRange::default()
                                .aspect_mask(vk::ImageAspectFlags::COLOR)
                                .level_count(1)
                                .layer_count(1),
                        ),
                    None,
                )
                .map_err(err)?;
            // Same depth-format rule as the swapchain: readable by the cloud
            // march as well as writable by every world draw.
            let depth_format = [vk::Format::D32_SFLOAT, vk::Format::D16_UNORM]
                .into_iter()
                .find(|f| {
                    d.instance
                        .raw
                        .get_physical_device_format_properties(d.physical, *f)
                        .optimal_tiling_features
                        .contains(
                            vk::FormatFeatureFlags::DEPTH_STENCIL_ATTACHMENT
                                | vk::FormatFeatureFlags::SAMPLED_IMAGE,
                        )
                })
                .ok_or("No sampleable depth attachment format for the world target")?;
            out.depth = Some(Depth::new(d.clone(), extent, depth_format, true)?);
            // Colour is sampled by the upscale in a later pass, so it leaves
            // this pass in shader-read layout. Depth only has to be readable
            // when a cloud target exists to march against it; otherwise the
            // contents are dead after the pass and the store is skipped.
            let attachments = [
                vk::AttachmentDescription::default()
                    .format(format)
                    .samples(vk::SampleCountFlags::TYPE_1)
                    .load_op(vk::AttachmentLoadOp::CLEAR)
                    .store_op(vk::AttachmentStoreOp::STORE)
                    .initial_layout(vk::ImageLayout::UNDEFINED)
                    .final_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL),
                vk::AttachmentDescription::default()
                    .format(depth_format)
                    .samples(vk::SampleCountFlags::TYPE_1)
                    .load_op(vk::AttachmentLoadOp::CLEAR)
                    .store_op(if store_depth {
                        vk::AttachmentStoreOp::STORE
                    } else {
                        vk::AttachmentStoreOp::DONT_CARE
                    })
                    .initial_layout(vk::ImageLayout::UNDEFINED)
                    .final_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL),
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
            let color_stages = vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT;
            let depth_stages = vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS
                | vk::PipelineStageFlags::LATE_FRAGMENT_TESTS;
            // External reads are the previous frame's upscale (colour) and the
            // cloud march (depth); both were submitted before this command
            // buffer, so the dependency is from the prior submission rather
            // than another subpass.
            let dependencies = [
                vk::SubpassDependency::default()
                    .src_subpass(vk::SUBPASS_EXTERNAL)
                    .dst_subpass(0)
                    .src_stage_mask(vk::PipelineStageFlags::FRAGMENT_SHADER)
                    .dst_stage_mask(color_stages | depth_stages)
                    .src_access_mask(vk::AccessFlags::SHADER_READ)
                    .dst_access_mask(
                        vk::AccessFlags::COLOR_ATTACHMENT_WRITE
                            | vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE,
                    ),
                vk::SubpassDependency::default()
                    .src_subpass(0)
                    .dst_subpass(vk::SUBPASS_EXTERNAL)
                    .src_stage_mask(color_stages | depth_stages)
                    .dst_stage_mask(vk::PipelineStageFlags::FRAGMENT_SHADER)
                    .src_access_mask(
                        vk::AccessFlags::COLOR_ATTACHMENT_WRITE
                            | vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE,
                    )
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
                        .attachments(&[out.view, out.depth.as_ref().expect("scaled depth").view])
                        .width(extent.width)
                        .height(extent.height)
                        .layers(1),
                    None,
                )
                .map_err(err)?;
            out.world = pipeline(d, layout, out.pass, PipelineKind::World)?;
            out.water = pipeline(d, layout, out.pass, PipelineKind::Water)?;
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
            out.upscale_set_layout = d
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
            out.upscale_pool = d
                .raw
                .create_descriptor_pool(
                    &vk::DescriptorPoolCreateInfo::default()
                        .max_sets(1)
                        .pool_sizes(&sizes),
                    None,
                )
                .map_err(err)?;
            out.upscale_set = d
                .raw
                .allocate_descriptor_sets(
                    &vk::DescriptorSetAllocateInfo::default()
                        .descriptor_pool(out.upscale_pool)
                        .set_layouts(&[out.upscale_set_layout]),
                )
                .map_err(err)?[0];
            let images = [vk::DescriptorImageInfo::default()
                .image_view(out.view)
                .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
            let samplers = [vk::DescriptorImageInfo::default().sampler(out.sampler)];
            d.raw.update_descriptor_sets(
                &[
                    vk::WriteDescriptorSet::default()
                        .dst_set(out.upscale_set)
                        .dst_binding(0)
                        .descriptor_type(types[0])
                        .image_info(&images),
                    vk::WriteDescriptorSet::default()
                        .dst_set(out.upscale_set)
                        .dst_binding(1)
                        .descriptor_type(types[1])
                        .image_info(&samplers),
                ],
                &[],
            );
            out.upscale_layout = d
                .raw
                .create_pipeline_layout(
                    &vk::PipelineLayoutCreateInfo::default()
                        .set_layouts(&[out.upscale_set_layout])
                        .push_constant_ranges(&[vk::PushConstantRange::default()
                            .stage_flags(vk::ShaderStageFlags::FRAGMENT)
                            .offset(0)
                            .size(24)]),
                    None,
                )
                .map_err(err)?;
            out.upscale = full_screen_pipeline(
                d,
                out.upscale_layout,
                present_pass,
                &FullScreenPipeline {
                    vertex_spirv: include_bytes!(concat!(env!("OUT_DIR"), "/sky.vs_main.spv")),
                    fragment_spirv: include_bytes!(concat!(
                        env!("OUT_DIR"),
                        "/upscale.fs_main.spv"
                    )),
                    fragment_entry: c"fs_main",
                    premultiplied_blend: false,
                    depth_equal: false,
                },
            )?;
        }
        Ok(out)
    }

    /// Record the fullscreen upscale into the swapchain present pass. The
    /// caller has already set that pass's viewport and scissor to the present
    /// extent, which the shader needs as the inverse of that extent: the
    /// fragment position is in present pixels, and the whole range has to map
    /// onto the target's 0..1. The target's own inverse extent lets the shader
    /// measure the sharp-bilinear phase in target pixels rather than in the
    /// rounded ratio of the two extents.
    ///
    /// # Safety
    ///
    /// Must be recorded inside `present_pass`, whose colour attachment is the
    /// swapchain image this target's upscale pipeline was built against.
    pub(crate) unsafe fn record_upscale(&self, cmd: vk::CommandBuffer, present: vk::Extent2D) {
        let push = [
            1.0 / present.width.max(1) as f32,
            1.0 / present.height.max(1) as f32,
            1.0 / self.extent.width.max(1) as f32,
            1.0 / self.extent.height.max(1) as f32,
            UPSCALE_SNAP,
            0.0,
        ];
        let d = &self.device.raw;
        d.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, self.upscale);
        d.cmd_bind_descriptor_sets(
            cmd,
            vk::PipelineBindPoint::GRAPHICS,
            self.upscale_layout,
            0,
            &[self.upscale_set],
            &[],
        );
        d.cmd_push_constants(
            cmd,
            self.upscale_layout,
            vk::ShaderStageFlags::FRAGMENT,
            0,
            bytemuck::cast_slice(&push),
        );
        d.cmd_draw(cmd, 3, 1, 0, 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaled_extents_round_to_nearest_and_never_vanish() {
        let full = vk::Extent2D {
            width: 3168,
            height: 1440,
        };
        // The default the sample runs at: 2534x1152 is 2,919,168 fragments,
        // 36.0% fewer than 4,561,920.
        let default = scaled_extent(full, 0.8);
        assert_eq!((default.width, default.height), (2534, 1152));
        let native = scaled_extent(full, 1.0);
        assert_eq!((native.width, native.height), (3168, 1440));
        // Rounding must not strip a 5 px-high window to nothing at the floor.
        let tiny = scaled_extent(
            vk::Extent2D {
                width: 5,
                height: 3,
            },
            MIN_RENDER_SCALE,
        );
        assert_eq!((tiny.width, tiny.height), (2, 1));
    }

    /// The upscale's sample-position warp, mirrored from `upscale.wgsl` so the
    /// shipped arithmetic can be checked without a GPU. Keep the two in step:
    /// this is the function that decides whether a one-pixel target step stays
    /// a step in the present image.
    fn warped_sample(target_px: f32, snap: f32) -> f32 {
        let base = target_px.floor() + 0.5;
        let d = target_px - base;
        let band = snap * 0.5;
        let t = ((d.abs() - band) / (0.5 - band)).clamp(0.0, 1.0);
        base + d.signum() * t * 0.5
    }

    /// One row of the sharp-bilinear upscale of a 64-texel target row, dark for
    /// the first half and bright for the second, at `scale`. A `nearest` sample
    /// mirrors the exact-texel candidate this filter was measured against.
    /// `phase` is the sub-texel camera position in target pixels.
    fn upscale_row(scale: f32, phase: f32, snap: f32, nearest: bool) -> Vec<f32> {
        const TEXELS: f32 = 64.0;
        const DARK: f32 = 0.1;
        const BRIGHT: f32 = 0.7;
        let texel = |i: f32| if i < TEXELS * 0.5 { DARK } else { BRIGHT };
        let present = (TEXELS * scale).round() as usize;
        (0..present)
            .map(|x| {
                let target_px = (x as f32 + 0.5) * scale + phase;
                if nearest {
                    return texel(target_px.floor().clamp(0.0, TEXELS - 1.0));
                }
                let pos = warped_sample(target_px, snap);
                let left = (pos - 0.5).floor();
                let low = texel(left.clamp(0.0, TEXELS - 1.0));
                let high = texel((left + 1.0).clamp(0.0, TEXELS - 1.0));
                low + (high - low) * (pos - 0.5 - left)
            })
            .collect()
    }

    /// The distant voxel steps are one to two target pixels wide, so the
    /// upscale has to preserve a one-texel step's contrast at scale 0.8; and it
    /// has to do so without snapping the edge between texels, which is what a
    /// sub-pixel camera motion would show as shimmer. Both properties are
    /// measured here on the same synthetic step for every candidate the
    /// comparison ran, so the shipped `UPSCALE_SNAP` is the value that passes
    /// the first without losing the second.
    #[test]
    fn the_upscale_keeps_a_one_pixel_step_and_does_not_snap_a_sub_pixel_shift() {
        const CONTRAST: f32 = 0.6; // The row's bright minus dark value.
        const STEP: f32 = 1.0 / 16.0; // One sixteenth of a target texel.
        let step_delta = |row: &[f32]| {
            row.windows(2)
                .map(|pair| (pair[1] - pair[0]).abs())
                .fold(0.0, f32::max)
        };
        let mean_step = |snap: f32| {
            (0..16)
                .map(|k| step_delta(&upscale_row(0.8, k as f32 * STEP, snap, false)))
                .sum::<f32>()
                / 16.0
        };
        let worst_jump = |snap: f32, nearest: bool| {
            (0..16)
                .map(|k| {
                    let before = upscale_row(0.8, k as f32 * STEP, snap, nearest);
                    let after = upscale_row(0.8, (k + 1) as f32 * STEP, snap, nearest);
                    before
                        .iter()
                        .zip(&after)
                        .map(|(a, b)| (a - b).abs())
                        .fold(0.0, f32::max)
                })
                .fold(0.0, f32::max)
        };

        // Plain bilinear loses the step: this is the K1 collapse the sharp warp
        // was added to fix.
        assert!(mean_step(0.0) < 0.80 * CONTRAST);
        // The shipped filter keeps almost all of it.
        assert!(mean_step(UPSCALE_SNAP) >= 0.90 * CONTRAST);
        // A sub-texel move changes the shipped filter by at most a quarter of
        // the step and plain bilinear by less; nearest moves a whole step.
        assert!(worst_jump(UPSCALE_SNAP, false) <= 0.30 * CONTRAST);
        assert!(worst_jump(0.0, false) <= 0.10 * CONTRAST);
        assert!(worst_jump(0.0, true) >= 0.90 * CONTRAST);
    }

    /// At native scale every sample lands on a texel centre and the warp is
    /// inactive, so a scaled path promoted back to 1.0 would still be a copy
    /// rather than a filtered image.
    #[test]
    fn the_upscale_is_a_copy_when_the_present_extent_matches_the_target() {
        let row = upscale_row(1.0, 0.0, UPSCALE_SNAP, false);
        assert_eq!(row.len(), 64);
        assert_eq!(row[..32], [0.1; 32]);
        assert_eq!(row[32..], [0.7; 32]);
    }
}
