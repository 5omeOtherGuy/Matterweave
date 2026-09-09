//! Single source of truth for the group-0 contract shared by `world.wgsl`, the
//! renderer and host validation harnesses.
//!
//! The renderer owns the descriptor set; a headless validation harness builds an
//! identical layout from these constants, so a probe exercises the shipping
//! fragment shader rather than a copy of its algorithm. Tests assert every offset.
use bytemuck::{Pod, Zeroable};

/// Group-0 binding indices declared by `world.wgsl`.
pub const BINDING_LIGHTING: u32 = 0;
pub const BINDING_SHADOW_MAP: u32 = 1;
pub const BINDING_SHADOW_SAMPLER: u32 = 2;
pub const BINDING_INDIRECT_FACES: u32 = 3;
pub const BINDING_REFLECTION_MATERIALS: u32 = 4;
pub const BINDING_REFLECTION_PALETTE: u32 = 5;

/// Shipping `world.wgsl` SPIR-V. Exposed so validation harnesses compile the
/// exact module the renderer uses; Naga flips clip Y for Vulkan at build time.
pub const WORLD_VERTEX_SPIRV: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/world.vs_main.spv"));
pub const WORLD_FRAGMENT_SPIRV: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/world.fs_main.spv"));

/// Group-0 binding 0. Column-major matrices; `sun` is direction xyz (normalized)
/// plus intensity w. `params` is (shadow enabled, depth bias, inverse map size, 0).
/// `indirect_dimensions.w` and `reflection_dimensions.w` are enable flags: a zero
/// disables the corresponding effect without touching the other bindings.
/// `reflection_params` is (trace step bound, surface offset, 0, 0).
#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct LightingUniform {
    pub view_proj: [[f32; 4]; 4],
    pub sun: [f32; 4],
    pub params: [f32; 4],
    pub indirect_origin: [i32; 4],
    pub indirect_dimensions: [u32; 4],
    pub reflection_origin: [i32; 4],
    pub reflection_dimensions: [u32; 4],
    pub reflection_params: [f32; 4],
}

pub const LIGHTING_UNIFORM_BYTES: usize = 176;

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{align_of, offset_of, size_of};

    #[test]
    fn uniform_matches_world_wgsl_layout() {
        assert_eq!(size_of::<LightingUniform>(), LIGHTING_UNIFORM_BYTES);
        assert_eq!(align_of::<LightingUniform>(), 16);
        assert_eq!(offset_of!(LightingUniform, view_proj), 0);
        assert_eq!(offset_of!(LightingUniform, sun), 64);
        assert_eq!(offset_of!(LightingUniform, params), 80);
        assert_eq!(offset_of!(LightingUniform, indirect_origin), 96);
        assert_eq!(offset_of!(LightingUniform, indirect_dimensions), 112);
        assert_eq!(offset_of!(LightingUniform, reflection_origin), 128);
        assert_eq!(offset_of!(LightingUniform, reflection_dimensions), 144);
        assert_eq!(offset_of!(LightingUniform, reflection_params), 160);
        // Every binding index is distinct and contiguous from zero.
        let bindings = [
            BINDING_LIGHTING,
            BINDING_SHADOW_MAP,
            BINDING_SHADOW_SAMPLER,
            BINDING_INDIRECT_FACES,
            BINDING_REFLECTION_MATERIALS,
            BINDING_REFLECTION_PALETTE,
        ];
        for (index, binding) in bindings.iter().enumerate() {
            assert_eq!(*binding as usize, index);
        }
    }

    #[test]
    fn world_spirv_is_a_valid_module() {
        for module in [WORLD_VERTEX_SPIRV, WORLD_FRAGMENT_SPIRV] {
            assert_eq!(&module[..4], &0x07230203u32.to_le_bytes());
        }
    }
}
