//! Packed occupancy layout, invariants and bounded update accounting.

use crate::fixtures::{sparse_fixture, Rng};
use crate::occupancy::MAX_BLOCK_BITS;
use crate::{cells_in, BlockShape, HierarchyError, HierarchyVolume, OccupancyGrid, SourceKey};

fn index_of(dimensions: [u32; 3], cell: [u32; 3]) -> usize {
    let [dx, dy, _] = dimensions;
    (cell[0] + dx * (cell[1] + dy * cell[2])) as usize
}

#[test]
fn named_shapes_pack_bits_in_x_fastest_order() {
    let cube4 = BlockShape::CUBE4;
    assert_eq!(cube4.dims(), [4, 4, 4]);
    assert_eq!(cube4.bits(), 64);
    assert_eq!(cube4.words_per_block(), 2);
    assert_eq!(cube4.bit_index([1, 2, 3]), 1 + 4 * (2 + 4 * 3));
    // The cited reference's 4x4x8 block is 128 bits and needs four words.
    let tall = BlockShape::TALL_4_4_8;
    assert_eq!(tall.bits(), 128);
    assert_eq!(tall.words_per_block(), 4);
    assert_eq!(tall.bit_index([3, 3, 7]), 127);
    assert_eq!(BlockShape::CUBE8.words_per_block(), 16);

    // A bit at the top of one block must not leak into the next block's words.
    let dimensions = [8, 4, 8];
    let mut materials = vec![0u32; cells_in(dimensions).unwrap()];
    materials[index_of(dimensions, [3, 3, 7])] = 5;
    let grid = OccupancyGrid::build(dimensions, tall, &materials).unwrap();
    assert_eq!(grid.block_dims(), [2, 1, 1]);
    assert_eq!(grid.blocks(), 2);
    assert!(grid.cell_occupied(0, [3, 3, 7]));
    assert!(!grid.cell_occupied(0, [0, 0, 7]));
    assert_eq!(grid.bits_set(), 1);
    assert!(!grid.block_occupied(1));
    // Word 3 of block 0 carries cell [3, 3, 7]: bit 31, and only that bit.
    assert_eq!(grid.words()[3], 1 << 31);
    assert_eq!(&grid.words()[4..], &[0, 0, 0, 0]);
    assert_eq!(grid.block_words(0).map(<[u32]>::len), Some(4));
    assert_eq!(grid.block_words(2), None);
    assert!(!grid.block_occupied(9));
    assert!(!grid.cell_occupied(0, [4, 3, 7]));
}

#[test]
fn shape_validation_bounds_axes_and_bits() {
    assert_eq!(BlockShape::new(0, 4, 4), None);
    assert_eq!(BlockShape::new(4, 0, 4), None);
    assert_eq!(BlockShape::new(4, 4, 0), None);
    assert_eq!(BlockShape::new(129, 1, 1), None);
    assert_eq!(BlockShape::new(8, 8, 9), None, "576 bits exceeds the bound");
    assert_eq!(MAX_BLOCK_BITS, 512);
    assert_eq!(BlockShape::new(16, 16, 2).map(BlockShape::bits), Some(512));
    assert_eq!(
        BlockShape::new(1, 1, 1).map(BlockShape::words_per_block),
        Some(1)
    );
    assert_eq!(BlockShape::new(128, 1, 1).map(BlockShape::bits), Some(128));
}

#[test]
fn occupancy_matches_materials_and_keeps_them_separate() {
    let fixture = sparse_fixture(11);
    let volume = fixture.snapshot(BlockShape::TALL_4_4_8);
    let grid = volume.occupancy();
    assert_eq!(grid.dimensions(), fixture.dimensions);
    assert_eq!(grid.shape(), BlockShape::TALL_4_4_8);
    let [dx, dy, dz] = fixture.dimensions;
    assert_eq!(
        grid.block_dims(),
        [dx.div_ceil(4), dy.div_ceil(4), dz.div_ceil(8)]
    );
    let cells = cells_in(fixture.dimensions).unwrap();
    assert_eq!(grid.words().len(), grid.blocks() * 4);
    assert_eq!(
        volume.materials().len(),
        cells,
        "materials stay dense and separate"
    );
    assert_eq!(grid.bytes(), grid.words().len() * 4);
    assert_eq!(
        grid.bits_set(),
        volume.materials().iter().filter(|&&m| m != 0).count(),
        "one bit per solid cell"
    );
    for z in 0..dz {
        for y in 0..dy {
            for x in 0..dx {
                let cell = [x, y, z];
                let block = grid.block_index([x / 4, y / 4, z / 8]).unwrap();
                let local = [x % 4, y % 4, z % 8];
                let solid = volume.materials()[index_of(fixture.dimensions, cell)] != 0;
                assert_eq!(grid.cell_occupied(block, local), solid, "cell {cell:?}");
            }
        }
    }
    let memory = volume.memory_stats();
    assert_eq!(memory.cells, cells);
    assert_eq!(memory.material_bytes, cells * 4);
    assert_eq!(memory.occupancy_words, grid.words().len());
    assert_eq!(
        memory.total_bytes,
        memory.material_bytes + memory.occupancy_bytes
    );
    let report = volume.build_report();
    assert_eq!(report.cells, cells);
    assert_eq!(report.material_words_copied, cells);
    assert_eq!(report.occupancy_blocks, grid.blocks());
    assert_eq!(report.occupancy_words_allocated, grid.words().len());
    assert_eq!(report.occupancy_bits_set, grid.bits_set());
}

#[test]
fn partial_edge_blocks_cover_only_crop_cells() {
    // 10x6x5 with 4x4x4 blocks: 3x2x2 = 12 blocks, the far blocks partly masked.
    let mut world = matterweave_core::World::new(1);
    assert!(world.set([9, 5, 4], 3), "corner cell of the crop");
    let volume = crate::fixtures::snapshot(&world, BlockShape::CUBE4, [0, 0, 0], [10, 6, 5]);
    let grid = volume.occupancy();
    assert_eq!(grid.block_dims(), [3, 2, 2]);
    assert_eq!(grid.blocks(), 12);
    let corner = grid.block_index([2, 1, 1]).unwrap();
    assert!(grid.cell_occupied(corner, [1, 1, 0]));
    assert!(
        !grid.cell_occupied(corner, [2, 1, 0]),
        "local 2 is outside the 10-cell axis"
    );
    assert!(!grid.cell_occupied(corner, [3, 1, 0]));
    // Storage for the masked block is still allocated and reported.
    assert_eq!(grid.words().len(), 12 * 2);
    let materials = vec![0u32; 10 * 6 * 5];
    let mut grid = OccupancyGrid::build([10, 6, 5], BlockShape::CUBE4, &materials).unwrap();
    assert!(
        grid.set_cell([10, 0, 0], true).is_none(),
        "outside the crop"
    );
    assert!(grid.set_cell([9, 5, 4], true).is_some());
}

#[test]
fn invalid_crops_and_materials_are_rejected_before_use() {
    let materials = [0u32; 8];
    assert_eq!(
        OccupancyGrid::build([0, 2, 2], BlockShape::CUBE4, &materials),
        Err(HierarchyError::VolumeDimensions {
            dimensions: [0, 2, 2]
        })
    );
    assert_eq!(
        OccupancyGrid::build([129, 1, 1], BlockShape::CUBE4, &materials),
        Err(HierarchyError::VolumeDimensions {
            dimensions: [129, 1, 1]
        })
    );
    assert_eq!(
        OccupancyGrid::build([2, 2, 2], BlockShape::CUBE4, &materials[..7]),
        Err(HierarchyError::MaterialCount {
            expected: 8,
            actual: 7
        })
    );
    assert_eq!(cells_in([64, 64, 64]).unwrap(), 64 * 64 * 64);
    assert_eq!(cells_in([128, 64, 32]).map(|n| n / 1024), Ok(256));
    assert_eq!(cells_in([128, 128, 128]), Err(HierarchyError::VolumeCells));
    assert_eq!(cells_in([64, 64, 65]), Err(HierarchyError::VolumeCells));
    // Material ids are validated by the snapshot constructor, not by the bit packer.
    let mut materials = vec![0u32; 8];
    materials[5] = 256;
    assert!(OccupancyGrid::build([2, 2, 2], BlockShape::CUBE4, &materials).is_ok());
    let key = SourceKey {
        epoch: 0,
        revision: 0,
        seed: 0,
    };
    assert_eq!(
        HierarchyVolume::build([0, 0, 0], [2, 2, 2], &materials, key, BlockShape::CUBE4).err(),
        Some(HierarchyError::MaterialOutOfRange {
            index: 5,
            material: 256
        })
    );
}

#[test]
fn patch_writes_at_most_one_word_and_reports_block_transitions() {
    // Two blocks along x: eight cells with a 4x4x4 shape.
    let mut volume = crate::fixtures::snapshot(
        &matterweave_core::World::new(0),
        BlockShape::CUBE4,
        [0, 0, 0],
        [8, 4, 4],
    );
    assert_eq!(volume.occupancy().blocks(), 2);
    let report = volume.patch_cell([1, 1, 1], 9).unwrap();
    assert!(report.material_changed);
    assert!(report.occupancy_bit_changed);
    assert_eq!(report.occupancy_words_written, 1);
    assert!(report.block_became_occupied);
    assert!(!report.block_became_empty);
    assert_eq!(volume.material_at([1, 1, 1]), Some(9));
    assert!(volume.locally_edited());
    // Rewriting the same material changes nothing and writes nothing.
    let report = volume.patch_cell([1, 1, 1], 9).unwrap();
    assert!(!report.material_changed);
    assert!(!report.occupancy_bit_changed);
    assert_eq!(report.occupancy_words_written, 0);
    assert!(!report.block_became_occupied);
    // Another material in the same block keeps the bit and the word count.
    let report = volume.patch_cell([1, 1, 1], 7).unwrap();
    assert!(report.material_changed);
    assert!(!report.occupancy_bit_changed);
    assert_eq!(report.occupancy_words_written, 0);
    // Clearing the only solid cell empties its block.
    let report = volume.patch_cell([1, 1, 1], 0).unwrap();
    assert!(report.occupancy_bit_changed);
    assert!(report.block_became_empty);
    assert_eq!(volume.occupancy().bits_set(), 0);
    assert!(!volume.occupancy().block_occupied(0));
    assert!(!volume.occupancy().block_occupied(1));
    assert_eq!(volume.memory_stats().material_bytes, 128 * 4);
    assert_eq!(
        volume.patch_cell([8, 0, 0], 1),
        Err(HierarchyError::CellOutsideVolume { cell: [8, 0, 0] })
    );
    assert_eq!(
        volume.patch_cell([-1, 0, 0], 1),
        Err(HierarchyError::CellOutsideVolume { cell: [-1, 0, 0] })
    );
}

#[test]
fn occupancy_bits_stay_in_alignment_with_random_patched_cells() {
    let mut rng = Rng::new(0xA7);
    let fixture = sparse_fixture(5);
    let mut volume = fixture.snapshot(BlockShape::CUBE4);
    let mut expected = volume.materials().to_vec();
    for _ in 0..400 {
        let cell = [
            rng.below(fixture.dimensions[0]) as i32,
            rng.below(fixture.dimensions[1]) as i32,
            rng.below(fixture.dimensions[2]) as i32,
        ];
        let local = [cell[0] as u32, cell[1] as u32, cell[2] as u32];
        let material = if rng.below(3) == 0 {
            0
        } else {
            rng.range(1, 256) as u8
        };
        let world_cell = [
            fixture.origin[0] + cell[0],
            fixture.origin[1] + cell[1],
            fixture.origin[2] + cell[2],
        ];
        let before = expected[index_of(fixture.dimensions, local)];
        let report = volume.patch_cell(world_cell, material).unwrap();
        assert_eq!(report.material_changed, before != u32::from(material));
        expected[index_of(fixture.dimensions, local)] = u32::from(material);
    }
    assert_eq!(volume.materials(), expected.as_slice());
    let grid = volume.occupancy();
    for z in 0..fixture.dimensions[2] {
        for y in 0..fixture.dimensions[1] {
            for x in 0..fixture.dimensions[0] {
                let cell = [x, y, z];
                let block = grid.block_index([x / 4, y / 4, z / 4]).unwrap();
                let solid = expected[index_of(fixture.dimensions, cell)] != 0;
                assert_eq!(grid.cell_occupied(block, [x % 4, y % 4, z % 4]), solid);
            }
        }
    }
    assert_eq!(
        grid.bits_set(),
        expected.iter().filter(|&&m| m != 0).count(),
        "bit population stays exact after patches"
    );
}
