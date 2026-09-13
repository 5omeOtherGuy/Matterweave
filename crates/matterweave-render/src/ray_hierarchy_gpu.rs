//! Optional packed-occupancy (BlockMask) traversal candidate for issue 42.
//!
//! This module is the GPU counterpart of the CPU `BlockMask` mode in
//! `matterweave-ray-hierarchy`: it owns the fragment kernel, its SPIR-V and the
//! byte-level binding contract. It is **not** a selected production path, it changes no
//! renderer default, and it makes no performance claim. The retained
//! [`crate::ray_reference`] pipeline and its shader are untouched. See
//! [the engineering log](../../../docs/performance/logs/ray-hierarchy-gpu.md) and
//! [the CPU experiment](../../../docs/performance/logs/ray-hierarchy-experiment.md).
//!
//! # Descriptor contract (set 0)
//!
//! | Binding | Type | Contents |
//! | --- | --- | --- |
//! | [`CAMERA_BINDING`] | uniform | [`crate::ray_reference::RayUniform`], 192 bytes, unchanged from the reference |
//! | [`MATERIAL_BINDING`] | storage read | dense `u32` material words, x fastest, one per crop cell |
//! | [`PALETTE_BINDING`] | storage read | 256 `vec4<f32>`, as in the reference |
//! | [`OCCUPANCY_BINDING`] | storage read | packed occupancy words, [`HierarchyLayout`] shaped |
//! | [`HIERARCHY_BINDING`] | uniform | [`HierarchyUniform`], 48 bytes |
//!
//! The geometry, slab entry, tie stepping, lowest-axis normals, iteration cap, shading
//! and depth output are copied unchanged from `ray_reference.wgsl`; only memory access
//! differs. Packed occupancy never replaces the material words: a bit is set exactly when
//! the material word is nonzero, so an empty block is skipped without a material read and
//! a set bit gates the read of the same word the reference would have read.
//!
//! # Bounded upload and lifetimes
//!
//! [`HierarchyUpload`] borrows the material and occupancy words. It validates, before any
//! buffer is created or updated:
//!
//! - the crop rules of `RayVolume::pack` (`MAX_AXIS`, `MAX_CELLS`);
//! - a block shape of at most [`BLOCK_WORDS_MAX`] words (512 bits), matching the shader's
//!   register cache;
//! - the exact material and occupancy word counts, so both buffers are bounded by the
//!   crop and the block shape, not by caller input;
//! - every material word within `0..=255`, because the shader indexes the 256-entry
//!   palette with it and hits report `u8` materials;
//! - the set occupancy bits and the nonzero material words to have the same count, so a
//!   mismatched (solid, occupancy) pair is rejected before it can skip a solid cell.
//!
//! The upload also carries its source key. [`HierarchyUpload::is_current`] re-checks
//! epoch, revision, seed, crop origin and dimensions against a pack; a stale upload must
//! not be reused, and the caller still gates on `RayVolume::valid_for(world, epoch)`
//! before deriving a new one. Because the words are borrowed, the derived snapshot (the
//! CPU `HierarchyVolume`) must outlive every GPU buffer fed from this upload.
//!
//! # Documented differences from the CPU modes
//!
//! - The kernel walks the clipped near/far segment only. The CPU `max_distance == 0`
//!   inside-solid query (oracle-defined) is not expressible in the fragment path, and the
//!   reference discards a zero-length segment for the same reason.
//! - CPU distances are `f64` recomputed from integer planes; the kernel is `f32`. Tie
//!   resolutions can therefore differ in the last bits. The native example classifies
//!   such differences with a pinned geometric proof; this module claims no bit-identity
//!   between the two walks.

use crate::ray_reference::RayVolume;
use bytemuck::{Pod, Zeroable};
use matterweave_core::World;

/// Fullscreen vertex entry point, compiled by `build.rs` through naga 24.0.0.
pub const VERTEX_SPIRV: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/ray_hierarchy_gpu.vs_main.spv"));
/// Fragment kernel entry point, compiled by `build.rs` through naga 24.0.0.
pub const FRAGMENT_SPIRV: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/ray_hierarchy_gpu.fs_main.spv"));

/// Set-0 binding of the unchanged `RayUniform` camera.
pub const CAMERA_BINDING: u32 = 0;
/// Set-0 binding of the dense material words.
pub const MATERIAL_BINDING: u32 = 1;
/// Set-0 binding of the 256-entry palette.
pub const PALETTE_BINDING: u32 = 2;
/// Set-0 binding of the packed occupancy words.
pub const OCCUPANCY_BINDING: u32 = 3;
/// Set-0 binding of the hierarchy parameters.
pub const HIERARCHY_BINDING: u32 = 4;

/// Largest block the shader caches in registers: 512 bits in 16 `u32` words.
pub const BLOCK_WORDS_MAX: usize = 16;
/// Byte size of [`HierarchyUniform`]; its offsets are all multiples of 16.
pub const HIERARCHY_UNIFORM_BYTES: usize = 48;

/// Group-0 binding 4, 48 bytes: the packed grid the shader indexes.
///
/// Three `vec4<u32>` fields, no padding. `shape` is the block extent in cells,
/// `block_dims` is `ceil(dimensions / shape)` per axis, and `misc` holds
/// `[words_per_block, bits_per_block, 0, 0]`.
#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Pod, Zeroable)]
pub struct HierarchyUniform {
    pub shape: [u32; 4],
    pub block_dims: [u32; 4],
    pub misc: [u32; 4],
}

/// Validated block grid of one bounded crop.
///
/// The grid rules are the CPU `OccupancyGrid` contract: `block_dims` is
/// `ceil(dimensions / shape)`, `bits_per_block` is `shape.x * shape.y * shape.z` (at most
/// 512), and the words of one block are consecutive with x fastest, low bits first.
/// [`Self::occupancy_words`] is exactly the word count a conforming occupancy buffer must
/// have.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HierarchyLayout {
    dimensions: [u32; 3],
    shape: [u32; 3],
    block_dims: [u32; 3],
    bits_per_block: usize,
    words_per_block: usize,
}

impl HierarchyLayout {
    /// Validates a crop and block shape against the shared bounded contract.
    ///
    /// Rejection reasons are strings, matching `RayVolume::pack`'s style.
    pub fn new(dimensions: [u32; 3], shape: [u32; 3]) -> Result<Self, String> {
        const MAX_AXIS: u32 = crate::ray_reference::MAX_AXIS;
        const MAX_CELLS: usize = crate::ray_reference::MAX_CELLS;
        if dimensions.iter().any(|&d| d == 0 || d > MAX_AXIS) {
            return Err("Hierarchy crop dimensions must be in 1..=128".into());
        }
        dimensions
            .iter()
            .try_fold(1usize, |n, &d| n.checked_mul(d as usize))
            .filter(|&n| n <= MAX_CELLS)
            .ok_or("Hierarchy crop exceeds 64^3 cells")?;
        if shape.iter().any(|&s| s == 0 || s > MAX_AXIS) {
            return Err("Hierarchy block shape must be in 1..=128 per axis".into());
        }
        let bits = shape
            .iter()
            .try_fold(1usize, |n, &s| n.checked_mul(s as usize))
            .filter(|&n| n <= BLOCK_WORDS_MAX * 32)
            .ok_or("Hierarchy block exceeds 512 bits")?;
        let block_dims = std::array::from_fn(|axis| dimensions[axis].div_ceil(shape[axis]));
        Ok(Self {
            dimensions,
            shape,
            block_dims,
            bits_per_block: bits,
            words_per_block: bits.div_ceil(32),
        })
    }

    /// Crop extent in cells.
    pub fn dimensions(&self) -> [u32; 3] {
        self.dimensions
    }
    /// Block extent in cells.
    pub fn shape(&self) -> [u32; 3] {
        self.shape
    }
    /// Blocks per axis, `ceil(dimensions / shape)`.
    pub fn block_dims(&self) -> [u32; 3] {
        self.block_dims
    }
    /// `u32` words per block, `ceil(bits_per_block / 32)`.
    pub fn words_per_block(&self) -> usize {
        self.words_per_block
    }
    /// Cells per block, at most 512.
    pub fn bits_per_block(&self) -> usize {
        self.bits_per_block
    }
    /// Number of blocks in the grid.
    pub fn blocks(&self) -> usize {
        self.block_dims
            .iter()
            .map(|&d| d as usize)
            .product::<usize>()
    }
    /// Occupancy words a conforming buffer holds, `blocks * words_per_block`.
    pub fn occupancy_words(&self) -> usize {
        self.blocks() * self.words_per_block
    }
    /// Exact cell count of the crop, the required material word count.
    pub fn materials(&self) -> usize {
        self.dimensions.iter().map(|&d| d as usize).product()
    }
    /// The 48-byte uniform this layout uploads.
    pub fn uniform(&self) -> HierarchyUniform {
        HierarchyUniform {
            shape: [self.shape[0], self.shape[1], self.shape[2], 0],
            block_dims: [
                self.block_dims[0],
                self.block_dims[1],
                self.block_dims[2],
                0,
            ],
            misc: [
                self.words_per_block as u32,
                self.bits_per_block() as u32,
                0,
                0,
            ],
        }
    }
}

/// Provenance key of an upload: the `RayVolume` source it was derived from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceKey {
    pub epoch: u64,
    pub revision: u64,
    pub seed: u64,
}

/// Logical payload sizes of one complete upload, not allocator overhead or residency.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HierarchyMemoryStats {
    pub material_bytes: usize,
    pub occupancy_bytes: usize,
    pub palette_bytes: usize,
    pub camera_uniform_bytes: usize,
    pub hierarchy_uniform_bytes: usize,
    /// One complete upload of all five descriptors.
    pub upload_bytes: usize,
}

/// A validated, borrowed pair of buffers plus the parameters describing them.
///
/// Construction is the whole validation step; after that, [`Self::memory_stats`] bounds
/// the Vulkan buffer sizes and [`Self::is_current`] answers whether the bytes still
/// belong to the pack the caller holds.
#[derive(Debug)]
pub struct HierarchyUpload<'a> {
    materials: &'a [u32],
    occupancy: &'a [u32],
    layout: HierarchyLayout,
    origin: [i32; 3],
    key: SourceKey,
}

impl<'a> HierarchyUpload<'a> {
    /// Validates `materials` and `occupancy` against `pack` and `shape`.
    ///
    /// `materials` is a dense `u32` word per crop cell (normally
    /// `HierarchyVolume::materials()`); `occupancy` is the packed word list of the same
    /// crop (`HierarchyVolume::occupancy().words()`). The shape comes from the caller
    /// because the GPU candidate tests more than the CPU-selected shape.
    pub fn from_parts(
        pack: &RayVolume,
        materials: &'a [u32],
        occupancy: &'a [u32],
        shape: [u32; 3],
    ) -> Result<Self, String> {
        let layout = HierarchyLayout::new(pack.dimensions(), shape)?;
        if materials.len() != layout.materials() {
            return Err(format!(
                "Hierarchy upload has {} material words, crop needs {}",
                materials.len(),
                layout.materials()
            ));
        }
        if let Some((index, &material)) = materials
            .iter()
            .enumerate()
            .find(|(_, &material)| material > u32::from(u8::MAX))
        {
            return Err(format!(
                "Hierarchy material {material} at {index} exceeds u8::MAX"
            ));
        }
        if occupancy.len() != layout.occupancy_words() {
            return Err(format!(
                "Hierarchy upload has {} occupancy words, layout needs {}",
                occupancy.len(),
                layout.occupancy_words()
            ));
        }
        // Layout-independent integrity: every solid cell sets exactly one bit, so the
        // totals must match. Bit *positions* are proven end to end by the native example,
        // which compares the kernel's readback against the CPU walk over the same words.
        let solid = materials.iter().filter(|&&word| word != 0).count();
        let occupied: usize = occupancy
            .iter()
            .map(|word| word.count_ones() as usize)
            .sum();
        if solid != occupied {
            return Err(format!(
                "Hierarchy solid cells {solid} do not match occupancy bits {occupied}"
            ));
        }
        Ok(Self {
            materials,
            occupancy,
            layout,
            origin: pack.origin(),
            key: SourceKey {
                epoch: pack.source_epoch(),
                revision: pack.source_revision(),
                seed: pack.source_seed(),
            },
        })
    }

    /// Dense material words, bounded to `layout().materials()`.
    pub fn materials(&self) -> &'a [u32] {
        self.materials
    }
    /// Packed occupancy words, bounded to `layout().occupancy_words()`.
    pub fn occupancy(&self) -> &'a [u32] {
        self.occupancy
    }
    /// Validated grid the shader indexes.
    pub fn layout(&self) -> HierarchyLayout {
        self.layout
    }
    /// Crop origin of the source pack, part of the freshness check.
    pub fn origin(&self) -> [i32; 3] {
        self.origin
    }
    /// Source key of the upload.
    pub fn source(&self) -> SourceKey {
        self.key
    }
    /// Logical byte sizes of this upload.
    pub fn memory_stats(&self) -> HierarchyMemoryStats {
        let material_bytes = std::mem::size_of_val(self.materials);
        let occupancy_bytes = std::mem::size_of_val(self.occupancy);
        HierarchyMemoryStats {
            material_bytes,
            occupancy_bytes,
            palette_bytes: crate::ray_reference::PALETTE_BYTES,
            camera_uniform_bytes: crate::ray_reference::UNIFORM_BYTES,
            hierarchy_uniform_bytes: HIERARCHY_UNIFORM_BYTES,
            upload_bytes: material_bytes
                + occupancy_bytes
                + crate::ray_reference::PALETTE_BYTES
                + crate::ray_reference::UNIFORM_BYTES
                + HIERARCHY_UNIFORM_BYTES,
        }
    }

    /// Whether `pack` is still the exact source this upload was derived from.
    ///
    /// Requires the epoch, whole-world revision and seed of the source key, plus the crop
    /// origin and dimensions, to match. This is the pack-identity half of the freshness
    /// rule; `RayVolume::valid_for(world, epoch)` remains the authority for whether the
    /// pack itself still describes the world.
    pub fn is_current(&self, pack: &RayVolume) -> bool {
        self.key.epoch == pack.source_epoch()
            && self.key.revision == pack.source_revision()
            && self.key.seed == pack.source_seed()
            && self.origin == pack.origin()
            && self.layout.dimensions() == pack.dimensions()
    }

    /// The 48-byte hierarchy uniform this upload binds.
    pub fn uniform(&self) -> HierarchyUniform {
        self.layout.uniform()
    }
}

/// Convenience: the freshness rule a production caller must satisfy before a GPU draw,
/// combining the upload's source key with the pack's world revision check.
///
/// Kept as a free function so the two halves stay separately testable and the upload does
/// not need to hold a `World`.
pub fn is_fresh(upload: &HierarchyUpload<'_>, pack: &RayVolume, world: &World, epoch: u64) -> bool {
    upload.is_current(pack) && pack.valid_for(world, epoch)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{align_of, offset_of, size_of};

    fn palette() -> [[f32; 3]; 256] {
        std::array::from_fn(|index| [index as f32 / 255.0, 0.25, 0.5])
    }

    fn pack(world: &World, epoch: u64, origin: [i32; 3], dimensions: [u32; 3]) -> RayVolume {
        RayVolume::pack(world, epoch, origin, dimensions, palette()).expect("valid fixture crop")
    }

    #[test]
    fn uniform_layout_and_spirv_entries() {
        assert_eq!(size_of::<HierarchyUniform>(), HIERARCHY_UNIFORM_BYTES);
        assert_eq!(align_of::<HierarchyUniform>(), 16);
        assert_eq!(offset_of!(HierarchyUniform, shape), 0);
        assert_eq!(offset_of!(HierarchyUniform, block_dims), 16);
        assert_eq!(offset_of!(HierarchyUniform, misc), 32);
        assert_eq!(&VERTEX_SPIRV[..4], &0x07230203u32.to_le_bytes());
        assert_eq!(&FRAGMENT_SPIRV[..4], &0x07230203u32.to_le_bytes());
        for (spirv, entry) in [
            (VERTEX_SPIRV, b"vs_main".as_slice()),
            (FRAGMENT_SPIRV, b"fs_main".as_slice()),
        ] {
            assert!(
                spirv.windows(entry.len()).any(|window| window == entry),
                "entry point missing"
            );
        }
    }

    #[test]
    fn layout_word_rules_match_the_bounded_block_contract() {
        // The 13x16x20 crop's byte totals are the ones the CPU experiment records for
        // each shape; whole-word rounding makes the coarser shapes larger here.
        for (shape, block_dims, words_per_block, word_count, bytes) in [
            ([4, 4, 4], [4, 4, 5], 2usize, 160usize, 640usize),
            ([4, 4, 8], [4, 4, 3], 4, 192, 768),
            ([8, 8, 8], [2, 2, 3], 16, 192, 768),
        ] {
            let layout = HierarchyLayout::new([13, 16, 20], shape).unwrap();
            assert_eq!(layout.block_dims(), block_dims);
            assert_eq!(layout.words_per_block(), words_per_block);
            assert_eq!(layout.occupancy_words(), word_count);
            assert_eq!(layout.occupancy_words() * 4, bytes);
        }
        let layout = HierarchyLayout::new([13, 16, 20], [8, 8, 8]).unwrap();
        assert_eq!(layout.uniform().block_dims, [2, 2, 3, 0]);
        assert_eq!(layout.blocks(), 12);
        let single = HierarchyLayout::new([1, 1, 1], [8, 8, 8]).unwrap();
        assert_eq!(single.block_dims(), [1, 1, 1]);
        assert_eq!(single.occupancy_words(), 16);
        assert_eq!(single.uniform().misc, [16, 512, 0, 0]);
        let full = HierarchyLayout::new([64, 64, 64], [4, 4, 8]).unwrap();
        assert_eq!(full.materials(), 64 * 64 * 64);
        assert_eq!(full.uniform().shape, [4, 4, 8, 0]);
    }

    #[test]
    fn layout_rejects_out_of_contract_crops_and_shapes() {
        for dimensions in [[0, 1, 1], [129, 1, 1], [128, 128, 128], [u32::MAX; 3]] {
            assert!(HierarchyLayout::new(dimensions, [4, 4, 8]).is_err());
        }
        for shape in [[0, 1, 1], [129, 1, 1], [8, 8, 9], [16, 16, 3]] {
            assert!(HierarchyLayout::new([4, 4, 4], shape).is_err());
        }
        assert!(HierarchyLayout::new([4, 4, 4], [16, 16, 2]).is_ok());
    }

    #[test]
    fn upload_validates_word_counts_and_material_range() {
        let mut world = World::new(7);
        assert!(world.set([0, 0, 0], 1));
        let pack = pack(&world, 1, [0, 0, 0], [4, 4, 4]);
        let layout = HierarchyLayout::new(pack.dimensions(), [4, 4, 4]).unwrap();
        let materials = vec![0u32; layout.materials()];
        let occupancy = vec![0u32; layout.occupancy_words()];
        let upload =
            HierarchyUpload::from_parts(&pack, &materials, &occupancy, layout.shape()).unwrap();
        assert_eq!(upload.layout(), layout);
        assert_eq!(upload.origin(), [0, 0, 0]);
        assert_eq!(upload.source().epoch, 1);
        let stats = upload.memory_stats();
        assert_eq!(stats.material_bytes, 64 * 4);
        assert_eq!(stats.occupancy_bytes, 2 * 4);
        assert_eq!(stats.camera_uniform_bytes, 192);
        assert_eq!(stats.hierarchy_uniform_bytes, 48);
        assert_eq!(
            stats.upload_bytes,
            stats.material_bytes + stats.occupancy_bytes + 4096 + 192 + 48
        );
        assert!(
            HierarchyUpload::from_parts(&pack, &materials[1..], &occupancy, layout.shape())
                .is_err()
        );
        assert!(
            HierarchyUpload::from_parts(&pack, &materials, &occupancy[1..], layout.shape())
                .is_err()
        );
        let mut over = materials.clone();
        over[3] = 256;
        assert!(HierarchyUpload::from_parts(&pack, &over, &occupancy, layout.shape()).is_err());
        assert!(HierarchyUpload::from_parts(&pack, &materials, &occupancy, [0, 4, 4]).is_err());
        // A material word needs its occupancy bit: the pair is rejected when the counts
        // disagree, and accepted once they match.
        let mut solid = materials.clone();
        solid[5] = 255;
        assert!(HierarchyUpload::from_parts(&pack, &solid, &occupancy, layout.shape()).is_err());
        let mut matching = occupancy.clone();
        matching[0] |= 1 << 5;
        assert!(HierarchyUpload::from_parts(&pack, &solid, &matching, layout.shape()).is_ok());
        let mut orphan = occupancy.clone();
        orphan[0] |= 1 << 2;
        assert!(HierarchyUpload::from_parts(&pack, &materials, &orphan, layout.shape()).is_err());
    }

    #[test]
    fn upload_freshness_tracks_pack_identity_and_world_revision() {
        let mut world = World::new(7);
        assert!(world.set([0, 0, 0], 9));
        let source = pack(&world, 3, [0, 0, 0], [4, 4, 4]);
        let layout = HierarchyLayout::new(source.dimensions(), [4, 4, 8]).unwrap();
        let materials = vec![0u32; layout.materials()];
        let occupancy = vec![0u32; layout.occupancy_words()];
        let upload =
            HierarchyUpload::from_parts(&source, &materials, &occupancy, layout.shape()).unwrap();
        assert!(upload.is_current(&source));
        assert!(is_fresh(&upload, &source, &world, 3));
        assert!(!is_fresh(&upload, &source, &world, 4));
        // An edit invalidates the pack and therefore the upload.
        assert!(world.set([0, 0, 0], 0));
        assert!(!source.valid_for(&world, 3));
        assert!(!is_fresh(&upload, &source, &world, 3));
        let edited = pack(&world, 3, [0, 0, 0], [4, 4, 4]);
        assert!(!upload.is_current(&edited));
        // A replacement at the same revision and seed is still a different source: the
        // repacked bytes describe the same world, but the revision moved here anyway.
        let moved = pack(&world, 4, [0, 0, 0], [4, 4, 4]);
        assert!(!upload.is_current(&moved));
        let elsewhere = pack(&world, 3, [8, 0, 0], [4, 4, 4]);
        assert!(!upload.is_current(&elsewhere));
    }
}
