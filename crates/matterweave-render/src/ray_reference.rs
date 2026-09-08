//! Isolated unit-voxel ray reference; no GPU resource ownership or path selection.
//! See `docs/performance/ray-reference.md` for the adapter and traversal contract.
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use matterweave_core::World;

pub const MAX_CELLS: usize = 64 * 64 * 64;
pub const MAX_AXIS: u32 = 128;
pub const PALETTE_BYTES: usize = 256 * 16;
pub const UNIFORM_BYTES: usize = 192;
pub const VERTEX_SPIRV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/ray_reference.vs_main.spv"));
pub const FRAGMENT_SPIRV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/ray_reference.fs_main.spv"));

/// Group 0 binding 0, 192 bytes. Column-major, unflipped RH 0..1-depth matrices.
/// Padded vec4s have no implicit padding; bytemuck verifies the upload contract.
#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct RayUniform {
    pub inverse_view_projection: [[f32; 4]; 4],
    pub view_projection: [[f32; 4]; 4],
    pub eye: [f32; 4],
    pub origin: [i32; 4],
    pub dimensions: [u32; 4],
    /// Normalized direction to sun xyz, intensity w (same as world.wgsl).
    pub sun: [f32; 4],
}

/// Logical payload sizes, not allocator overhead, staging duplication or GPU residency.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RayMemoryStats {
    pub material_bytes: usize,
    pub palette_bytes: usize,
    pub uniform_bytes: usize,
    /// One complete upload of all three descriptors.
    pub upload_bytes: usize,
}

/// Immutable derived snapshot of a half-open integer AABB. Everything outside it
/// is intentionally excluded. Material zero is air. Every other u8 ID is solid.
/// Caller MUST change epoch on replacement/fork/load, even at identical revision;
/// World has no globally unique identity. Epochs must not be reused while a pack lives.
#[derive(Debug)]
pub struct RayVolume {
    origin: [i32; 3],
    dimensions: [u32; 3],
    materials: Vec<u32>,
    palette: [[f32; 4]; 256],
    source_epoch: u64,
    source_revision: u64,
    source_seed: u64,
}

impl RayVolume {
    /// Validates before allocation. Both AABB endpoints must lie within +/-8192.
    /// Palette is caller-owned linear RGB, not sRGB; alpha is packed as one.
    pub fn pack(
        world: &World,
        epoch: u64,
        origin: [i32; 3],
        dimensions: [u32; 3],
        palette: [[f32; 3]; 256],
    ) -> Result<Self, String> {
        if dimensions.iter().any(|&d| d == 0 || d > MAX_AXIS) {
            return Err("Ray dimensions must be in 1..=128".into());
        }
        let cells = dimensions.iter().try_fold(1usize, |n, &d| n.checked_mul(d as usize))
            .filter(|&n| n <= MAX_CELLS).ok_or("Ray volume exceeds 64^3 cells")?;
        for axis in 0..3 {
            let end = i64::from(origin[axis]) + i64::from(dimensions[axis]);
            if origin[axis] < -8192 || end > 8192 {
                return Err("Ray bounds exceed +/-8192".into());
            }
        }
        if palette.iter().flatten().any(|&v| !v.is_finite() || !(0.0..=1.0).contains(&v)) {
            return Err("Ray palette must be finite linear RGB in 0..=1".into());
        }
        let mut materials = Vec::new();
        materials.try_reserve_exact(cells).map_err(|e| format!("Ray allocation: {e}"))?;
        for z in 0..dimensions[2] {
            for y in 0..dimensions[1] {
                for x in 0..dimensions[0] {
                    materials.push(u32::from(world.get([
                        origin[0] + x as i32, origin[1] + y as i32, origin[2] + z as i32,
                    ])));
                }
            }
        }
        Ok(Self {
            origin, dimensions, materials,
            palette: palette.map(|[r,g,b]| [r,g,b,1.0]),
            source_epoch: epoch, source_revision: world.revision(), source_seed: world.seed(),
        })
    }
    pub fn origin(&self) -> [i32; 3] { self.origin }
    pub fn dimensions(&self) -> [u32; 3] { self.dimensions }
    /// Group 0 binding 1: x + dims.x * (y + dims.y * z), u32 stride 4.
    pub fn materials(&self) -> &[u32] { &self.materials }
    /// Group 0 binding 2: exactly 256 vec4s, stride 16 (4096 bytes).
    pub fn palette(&self) -> &[[f32; 4]; 256] { &self.palette }
    pub fn source_epoch(&self) -> u64 { self.source_epoch }
    pub fn source_revision(&self) -> u64 { self.source_revision }
    /// Conservative: even edits outside the crop invalidate this pack.
    pub fn valid_for(&self, world: &World, epoch: u64) -> bool {
        self.source_epoch == epoch && self.source_revision == world.revision()
            && self.source_seed == world.seed()
    }
    pub fn memory_stats(&self) -> RayMemoryStats {
        let material_bytes = self.materials.len() * size_of::<u32>();
        RayMemoryStats { material_bytes, palette_bytes: PALETTE_BYTES,
            uniform_bytes: UNIFORM_BYTES, upload_bytes: material_bytes + PALETTE_BYTES + UNIFORM_BYTES }
    }
    /// Use finite perspective/orthographic RH projection with finite near/far,
    /// positive near, standard (not reversed) 0..1 depth. No manual Vulkan Y flip.
    /// Validate freshness before uploading; constructing a uniform does not do so.
    pub fn uniform(&self, view_projection: Mat4, eye: Vec3, sun: crate::Sun) -> Result<RayUniform, String> {
        if !view_projection.is_finite() || !eye.is_finite() || view_projection.determinant() == 0.0 {
            return Err("Ray camera must be finite and invertible".into());
        }
        let inverse = view_projection.inverse();
        if !inverse.is_finite() { return Err("Ray inverse camera is nonfinite".into()); }
        Ok(RayUniform {
            inverse_view_projection: inverse.to_cols_array_2d(),
            view_projection: view_projection.to_cols_array_2d(),
            eye: [eye.x, eye.y, eye.z, 0.0],
            origin: [self.origin[0], self.origin[1], self.origin[2], 0],
            dimensions: [self.dimensions[0], self.dimensions[1], self.dimensions[2], 0],
            sun: crate::indirect::light_key(sun)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Mat4, Vec3};
    use matterweave_core::World;
    use std::mem::{align_of, offset_of, size_of};

    fn palette() -> [[f32; 3]; 256] {
        std::array::from_fn(|i| [i as f32 / 255.0, 0.25, 0.5])
    }

    #[test]
    fn packs_authoritative_negative_chunk_edges_x_fastest() {
        let mut world = World::new(7);
        for (cell, id) in [([-17,-1,-1], 1), ([-16,-1,-1], 255),
            ([-17,0,-1], 3), ([-16,0,-1], 4), ([-17,-1,0], 5),
            ([-16,-1,0], 6), ([-17,0,0], 7), ([-16,0,0], 8),
            ([-18,-1,-1], 99), ([-15,0,0], 100)] {
            world.set(cell, id);
        }
        let pack = RayVolume::pack(&world, 12, [-17,-1,-1], [2,2,2], palette()).unwrap();
        assert_eq!(pack.materials(), &[1,255,3,4,5,6,7,8]);
        assert_eq!(pack.origin(), [-17,-1,-1]);
        assert_eq!(pack.dimensions(), [2,2,2]);
        assert_eq!(pack.palette()[255], [1.0,0.25,0.5,1.0]);
        assert_eq!(pack.palette()[0], [0.0,0.25,0.5,1.0]);
        let memory = pack.memory_stats();
        assert_eq!(memory.material_bytes, 32);
        assert_eq!(memory.palette_bytes, 4096);
        assert_eq!(memory.uniform_bytes, 192);
        assert_eq!(memory.upload_bytes, 4320);
    }

    #[test]
    fn snapshot_staleness_requires_revision_and_replacement_epoch() {
        let mut world = World::new(7);
        world.set([-1,0,0], 9);
        let pack = RayVolume::pack(&world, 3, [-1,0,0], [1,1,1], palette()).unwrap();
        assert!(pack.valid_for(&world, 3));
        assert!(!pack.valid_for(&world, 4));
        assert_eq!(pack.source_revision(), world.revision());
        assert_eq!(pack.source_epoch(), 3);
        let mut replacement = World::new(7);
        replacement.set([-1,0,0], 22);
        assert_eq!(world.revision(), replacement.revision());
        assert!(!pack.valid_for(&replacement, 4));
        world.set([-1,0,0], 0);
        assert!(!pack.valid_for(&world, 3));
        assert_eq!(pack.materials(), &[9]);
        let new = RayVolume::pack(&world, 3, [-1,0,0], [1,1,1], palette()).unwrap();
        assert_eq!(new.materials(), &[0]);
        world.set([200,0,0], 1);
        assert!(!new.valid_for(&world, 3));
    }

    #[test]
    fn rejects_invalid_bounds_and_palette_before_allocation() {
        let world = World::new(0);
        for (origin, dims) in [([0;3], [0,1,1]), ([0;3], [129,1,1]),
            ([0;3], [128;3]), ([0;3], [u32::MAX;3]),
            ([i32::MAX;3], [1;3]), ([i32::MIN;3], [1;3]),
            ([-8193,0,0], [1;3]), ([8192,0,0], [1;3])] {
            assert!(RayVolume::pack(&world, 0, origin, dims, palette()).is_err());
        }
        for invalid in [f32::NAN, f32::INFINITY, -0.01, 1.01] {
            let mut colors = palette();
            colors[255][2] = invalid;
            assert!(RayVolume::pack(&world, 0, [0;3], [1;3], colors).is_err());
        }
        for origin in [[-8192;3], [8191;3]] {
            assert!(RayVolume::pack(&world, 0, origin, [1;3], palette()).is_ok());
        }
        let pack = RayVolume::pack(&world, 0, [0;3], [128,64,32], palette()).unwrap();
        assert_eq!(pack.materials().len(), 64*64*64);
        assert_eq!(pack.memory_stats().material_bytes, 1_048_576);
    }

    #[test]
    fn uniform_layout_and_camera_round_trip() {
        assert_eq!(size_of::<RayUniform>(), 192);
        assert_eq!(align_of::<RayUniform>(), 16);
        assert_eq!(offset_of!(RayUniform, inverse_view_projection), 0);
        assert_eq!(offset_of!(RayUniform, view_projection), 64);
        assert_eq!(offset_of!(RayUniform, eye), 128);
        assert_eq!(offset_of!(RayUniform, origin), 144);
        assert_eq!(offset_of!(RayUniform, dimensions), 160);
        assert_eq!(offset_of!(RayUniform, sun), 176);
        let pack = RayVolume::pack(&World::new(0), 0, [-4,-3,-2], [8,6,4], palette()).unwrap();
        let eye = Vec3::new(3.0, 4.0, 8.0);
        let view = Mat4::look_at_rh(eye, Vec3::ZERO, Vec3::Y);
        for proj in [Mat4::perspective_rh(1.0, 1.4, 0.1, 100.0),
            Mat4::orthographic_rh(-5.0,5.0,-4.0,4.0,0.1,100.0)] {
            let vp = proj * view;
            let uniform = pack.uniform(vp, eye, crate::Sun::default()).unwrap();
            assert_eq!(bytemuck::bytes_of(&uniform).len(), 192);
            assert_eq!(uniform.origin, [-4,-3,-2,0]);
            assert_eq!(uniform.dimensions, [8,6,4,0]);
            let inverse = Mat4::from_cols_array_2d(&uniform.inverse_view_projection);
            assert!((inverse * vp).abs_diff_eq(Mat4::IDENTITY, 0.0001));
        }
        assert!(pack.uniform(Mat4::ZERO, eye, crate::Sun::default()).is_err());
        assert!(pack.uniform(Mat4::IDENTITY, Vec3::NAN, crate::Sun::default()).is_err());
    }
}
