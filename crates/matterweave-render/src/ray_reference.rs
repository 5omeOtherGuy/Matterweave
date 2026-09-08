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
