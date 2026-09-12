//! Reference-lineage semantics, targeted boundaries and structural counters.

use crate::fixtures::{self, solid_fixture, sparse_fixture, Rng, StartKind};
use crate::{BlockShape, HierarchyError, HierarchyVolume, RayHit, TraversalMode, TraversalStats};

const MODES: [TraversalMode; 3] = [
    TraversalMode::Reference,
    TraversalMode::BlockMask,
    TraversalMode::BlockStep,
];

/// Builds a crop around explicit cells. The crop is `origin` plus `dimensions`.
fn world_with(cells: &[([i32; 3], u8)]) -> matterweave_core::World {
    let mut world = matterweave_core::World::new(4);
    for &(cell, material) in cells {
        assert!(world.set(cell, material), "fixture write {cell:?}");
    }
    world
}

fn trace(
    volume: &HierarchyVolume,
    mode: TraversalMode,
    origin: [f64; 3],
    raw_direction: [f64; 3],
    max_distance: f64,
) -> (Option<RayHit>, TraversalStats) {
    let direction = fixtures::oracle_direction(raw_direction);
    volume
        .trace_stats(mode, origin, direction, max_distance)
        .expect("valid fixture ray")
}

/// Asserts every mode agrees with the reference mode bit for bit.
fn assert_modes_agree(
    volume: &HierarchyVolume,
    origin: [f64; 3],
    raw: [f64; 3],
    max: f64,
) -> Option<RayHit> {
    let (reference, _) = trace(volume, TraversalMode::Reference, origin, raw, max);
    for mode in [TraversalMode::BlockMask, TraversalMode::BlockStep] {
        let (hit, _) = trace(volume, mode, origin, raw, max);
        fixtures::assert_same_hit(
            reference,
            hit,
            &format!("mode {mode:?} at origin {origin:?} direction {raw:?}"),
        );
    }
    reference
}

#[test]
fn external_entry_on_a_face_hits_the_first_cell_with_that_face_normal() {
    let world = world_with(&[([0, 0, 0], 5)]);
    let volume = fixtures::snapshot(&world, BlockShape::CUBE4, [0, 0, 0], [4, 4, 4]);
    let hit = assert_modes_agree(&volume, [-1.0, 0.5, 0.5], [1.0, 0.0, 0.0], 8.0).unwrap();
    assert_eq!(hit.cell, [0, 0, 0]);
    assert_eq!(hit.normal, [-1, 0, 0]);
    assert_eq!(hit.material, 5);
    // Distance is measured from the caller's origin, like the reference segment.
    assert_eq!(hit.distance, 1.0);
    // The oracle walks the same cells from the same origin.
    assert_eq!(
        fixtures::oracle(&world, [-1.0, 0.5, 0.5], [1.0, 0.0, 0.0], 8.0),
        Some(hit)
    );
}

#[test]
fn zero_length_overlap_is_rejected_where_the_oracle_reports_a_t0_hit() {
    // Disclosed deviation from `World::raycast`, inherited from the reference shader:
    // an exact lower-face origin moving outward has `entry == exit`, so the crop path
    // rejects before inspecting any cell, while the CPU DDA reports the initial cell.
    let world = world_with(&[([0, 0, 0], 3)]);
    let volume = fixtures::snapshot(&world, BlockShape::CUBE4, [0, 0, 0], [4, 4, 4]);
    for mode in MODES {
        assert_eq!(
            trace(&volume, mode, [0.0, 0.5, 0.5], [-1.0, 0.0, 0.0], 4.0).0,
            None
        );
    }
    let oracle = fixtures::oracle(&world, [0.0, 0.5, 0.5], [-1.0, 0.0, 0.0], 4.0).unwrap();
    assert_eq!(oracle.cell, [0, 0, 0]);
    assert_eq!(oracle.distance, 0.0);
    assert_eq!(oracle.normal, [0, 0, 0]);
}

#[test]
fn parallel_rays_follow_the_half_open_slab_rule() {
    let world = world_with(&[([1, 2, 1], 6), ([1, 0, 1], 7), ([1, 4, 1], 8)]);
    let volume = fixtures::snapshot(&world, BlockShape::CUBE4, [0, 0, 0], [4, 4, 4]);
    // A parallel ray inside the slab travels its own row and hits that row's cell.
    let inside = assert_modes_agree(&volume, [0.5, 2.5, 1.5], [1.0, 0.0, 0.0], 8.0).unwrap();
    assert_eq!(inside.cell, [1, 2, 1]);
    assert_eq!(inside.material, 6);
    assert_eq!(inside.distance, 0.5);
    // A parallel ray on the lower face is inside and hits the row-zero cell.
    let lower = assert_modes_agree(&volume, [0.5, 0.0, 1.5], [1.0, 0.0, 0.0], 8.0).unwrap();
    assert_eq!(lower.cell, [1, 0, 1]);
    assert_eq!(lower.material, 7);
    // A parallel ray on the upper face is outside the half-open box, so the crop path
    // misses. The cell it would have crossed at y = 4 is outside the crop, so the
    // whole-world oracle reports it: the disclosed crop-exclusion difference.
    for mode in MODES {
        assert_eq!(
            trace(&volume, mode, [0.5, 4.0, 1.5], [1.0, 0.0, 0.0], 8.0).0,
            None
        );
    }
    assert_eq!(
        fixtures::oracle(&world, [0.5, 4.0, 1.5], [1.0, 0.0, 0.0], 8.0)
            .map(|hit| (hit.cell, hit.material)),
        Some(([1, 4, 1], 8)),
        "excluded content outside the crop stays visible to the oracle"
    );
    // Outside the slab on either side of the travelled axis misses at the slab test.
    for origin in [[0.5, -0.5, 1.5], [0.5, 4.5, 1.5]] {
        for mode in MODES {
            let (hit, stats) = trace(&volume, mode, origin, [1.0, 0.0, 0.0], 8.0);
            assert_eq!(hit, None, "origin {origin:?}");
            assert_eq!(stats, TraversalStats::default(), "clip miss does no work");
        }
    }
}

#[test]
fn tied_axes_step_together_and_the_lowest_axis_supplies_the_normal() {
    let world = world_with(&[([-2, -2, -2], 2)]);
    let volume = fixtures::snapshot(&world, BlockShape::CUBE4, [-2, -2, -2], [4, 4, 4]);
    let hit = assert_modes_agree(&volume, [-2.5, -2.5, -2.5], [1.0, 1.0, 1.0], 9.0).unwrap();
    assert_eq!(hit.cell, [-2, -2, -2]);
    assert_eq!(
        hit.normal,
        [-1, 0, 0],
        "lowest crossed axis wins a corner tie"
    );
    let oracle = fixtures::oracle(&world, [-2.5, -2.5, -2.5], [1.0, 1.0, 1.0], 9.0).unwrap();
    assert_eq!(oracle, hit, "corner entry matches the oracle exactly");
}

#[test]
fn inside_solid_start_reports_zero_distance_and_a_zero_normal() {
    let world = world_with(&[([0, 0, 0], 9)]);
    let volume = fixtures::snapshot(&world, BlockShape::CUBE4, [0, 0, 0], [4, 4, 4]);
    let hit = assert_modes_agree(&volume, [0.25, 0.25, 0.75], [1.0, 0.0, 0.0], 6.0).unwrap();
    assert_eq!(hit.cell, [0, 0, 0]);
    assert_eq!(hit.normal, [0, 0, 0]);
    assert_eq!(hit.distance, 0.0);
    assert_eq!(hit.material, 9);
    assert_eq!(
        fixtures::oracle(&world, [0.25, 0.25, 0.75], [1.0, 0.0, 0.0], 6.0),
        Some(hit)
    );
}

#[test]
fn negative_coordinates_use_floor_not_truncation() {
    // The ray starts in the air cell between two solid cells on negative coordinates, so
    // both travel directions hit and both must use floor, not truncation.
    let world = world_with(&[([-18, -1, -1], 1), ([-16, -1, -1], 255)]);
    let volume = fixtures::snapshot(&world, BlockShape::CUBE4, [-18, -1, -1], [3, 1, 1]);
    let forward = assert_modes_agree(&volume, [-16.5, -0.5, -0.5], [1.0, 0.0, 0.0], 6.0).unwrap();
    assert_eq!(forward.cell, [-16, -1, -1]);
    assert_eq!(forward.material, 255);
    assert_eq!(forward.normal, [-1, 0, 0]);
    assert_eq!(forward.distance, 0.5);
    assert_eq!(
        fixtures::oracle(&world, [-16.5, -0.5, -0.5], [1.0, 0.0, 0.0], 6.0),
        Some(forward)
    );
    let back = assert_modes_agree(&volume, [-16.5, -0.5, -0.5], [-1.0, 0.0, 0.0], 6.0).unwrap();
    assert_eq!(back.cell, [-18, -1, -1]);
    assert_eq!(back.material, 1);
    assert_eq!(back.normal, [1, 0, 0]);
    assert_eq!(
        fixtures::oracle(&world, [-16.5, -0.5, -0.5], [-1.0, 0.0, 0.0], 6.0),
        Some(back)
    );
}

#[test]
fn thin_wall_and_single_opening_block_only_where_solid() {
    // A one-cell wall at x = 3, which is the last cell of the first 4x4x4 block, with a
    // one-cell opening at [3, 1, 1]. A cell behind the opening separates pass from block.
    let mut cells = Vec::new();
    for y in 0..4 {
        for z in 0..4 {
            if (y, z) != (1, 1) {
                cells.push(([3, y, z], 11));
            }
        }
    }
    cells.push(([5, 1, 1], 12));
    let world = world_with(&cells);
    let volume = fixtures::snapshot(&world, BlockShape::CUBE4, [0, 0, 0], [8, 4, 4]);
    let blocked = assert_modes_agree(&volume, [-1.5, 0.5, 0.5], [1.0, 0.0, 0.0], 12.0).unwrap();
    assert_eq!(blocked.cell, [3, 0, 0]);
    assert_eq!(blocked.material, 11);
    assert_eq!(blocked.distance, 4.5);
    assert_eq!(
        fixtures::oracle(&world, [-1.5, 0.5, 0.5], [1.0, 0.0, 0.0], 12.0),
        Some(blocked)
    );
    let through = assert_modes_agree(&volume, [-1.5, 1.5, 1.5], [1.0, 0.0, 0.0], 12.0).unwrap();
    assert_eq!(through.cell, [5, 1, 1], "the one-cell opening still passes");
    assert_eq!(through.material, 12);
    assert_eq!(through.distance, 6.5);
    assert_eq!(
        fixtures::oracle(&world, [-1.5, 1.5, 1.5], [1.0, 0.0, 0.0], 12.0),
        Some(through)
    );
}

#[test]
fn invalid_ray_inputs_are_rejected_before_traversal() {
    let world = world_with(&[([0, 0, 0], 1)]);
    let volume = fixtures::snapshot(&world, BlockShape::CUBE4, [0, 0, 0], [4, 4, 4]);
    let unit = fixtures::oracle_direction([1.0, 2.0, 3.0]);
    for bad in [[0.0, 0.0, 0.0], [1.0, 1.0, 1.0], [2.0, 0.0, 0.0]] {
        assert_eq!(
            volume.trace(TraversalMode::BlockMask, [0.5; 3], bad, 4.0),
            Err(HierarchyError::InvalidDirection { direction: bad })
        );
    }
    // NaN never compares equal, so the variant is checked rather than the payload.
    assert!(matches!(
        volume.trace(
            TraversalMode::BlockMask,
            [0.5; 3],
            [f64::NAN, 0.0, 0.0],
            4.0
        ),
        Err(HierarchyError::InvalidDirection { .. })
    ));
    assert!(matches!(
        volume.trace(TraversalMode::BlockMask, [f64::NAN, 0.5, 0.5], unit, 4.0),
        Err(HierarchyError::InvalidRay { .. })
    ));
    for bad in [0.0, -1.0, 4096.5] {
        assert_eq!(
            volume.trace(TraversalMode::BlockMask, [0.5; 3], unit, bad),
            Err(HierarchyError::InvalidRay { max_distance: bad })
        );
    }
    for bad in [f64::NAN, f64::INFINITY] {
        assert!(matches!(
            volume.trace(TraversalMode::BlockMask, [0.5; 3], unit, bad),
            Err(HierarchyError::InvalidRay { .. })
        ));
    }
    assert_eq!(
        volume.trace(TraversalMode::Reference, [f64::NAN, 0.5, 0.5], unit, 4.0),
        Err(HierarchyError::InvalidRay { max_distance: 4.0 })
    );
    assert!(volume
        .trace(TraversalMode::Reference, [0.5; 3], unit, 4096.0)
        .is_ok());
}

#[test]
fn iteration_cap_is_the_reference_bound_and_is_never_exhausted() {
    let mut rng = Rng::new(0x51);
    for seed in 0..24u64 {
        let fixture = sparse_fixture(seed);
        let volume = fixture.snapshot(BlockShape::TALL_4_4_8);
        assert_eq!(
            volume.max_iterations(),
            fixture
                .dimensions
                .iter()
                .map(|&d| u64::from(d))
                .sum::<u64>()
                + 1
        );
        for _ in 0..12 {
            let start = rng.pick(&[
                StartKind::Center,
                StartKind::Integer,
                StartKind::Quarter,
                StartKind::Outside,
            ]);
            let (origin, raw, max) = fixtures::random_ray(&mut rng, &fixture, start);
            for mode in MODES {
                let (_, stats) = trace(&volume, mode, origin, raw, max);
                assert!(
                    !stats.exhausted,
                    "mode {mode:?} exhausted the cap at {origin:?} {raw:?}"
                );
                assert!(stats.iterations <= volume.max_iterations());
                // Every iteration either advances one cell or one empty block, except a
                // final iteration that resolves the ray.
                let advanced = stats.fine_steps + stats.block_steps;
                assert!(
                    stats.iterations == advanced || stats.iterations == advanced + 1,
                    "iterations {} vs steps {advanced}",
                    stats.iterations
                );
                if mode != TraversalMode::BlockStep {
                    assert_eq!(stats.block_steps, 0, "only the coarse mode skips blocks");
                }
                // A miss decided by the crop clip never enters the loop and reports
                // zeroed counters, so the occupancy counters are checked only after entry.
                if stats.iterations > 0 {
                    assert_eq!(stats.blocks_checked == 0, mode == TraversalMode::Reference);
                    assert_eq!(
                        stats.occupancy_word_loads == 0,
                        mode == TraversalMode::Reference
                    );
                }
            }
        }
    }
}

#[test]
fn counts_separate_material_traffic_from_occupancy_traffic() {
    let fixture = sparse_fixture(3);
    let volume = fixture.snapshot(BlockShape::TALL_4_4_8);
    let mut rng = Rng::new(0xC0);
    let mut saved_materials = 0u64;
    for _ in 0..64 {
        let (origin, raw, max) = fixtures::random_ray(&mut rng, &fixture, StartKind::Center);
        let (reference_hit, reference) = trace(&volume, TraversalMode::Reference, origin, raw, max);
        let (mask_hit, mask) = trace(&volume, TraversalMode::BlockMask, origin, raw, max);
        assert!(!reference.exhausted && !mask.exhausted);
        fixtures::assert_same_hit(reference_hit, mask_hit, "block mask");
        assert!(
            mask.material_reads <= mask.cells_examined,
            "a material read implies an examined cell"
        );
        assert!(
            mask.cells_examined <= reference.material_reads,
            "the candidate examines a subset of the reference's cells"
        );
        assert!(mask.occupancy_word_loads <= mask.blocks_checked);
        assert_eq!(reference.cells_examined, reference.material_reads);
        saved_materials += reference.material_reads - mask.material_reads;
    }
    assert!(
        saved_materials > 0,
        "sparse content must skip material words somewhere"
    );
    println!("sparse fixture: {saved_materials} material reads saved over 64 rays");

    // Fully solid content: every examined cell is solid, so the counts converge.
    let solid = solid_fixture([8, 8, 8]);
    let solid_volume = solid.snapshot(BlockShape::TALL_4_4_8);
    let (origin, raw, max) = fixtures::random_ray(&mut rng, &solid, StartKind::Center);
    let (reference_hit, reference) =
        trace(&solid_volume, TraversalMode::Reference, origin, raw, max);
    let (mask_hit, mask) = trace(&solid_volume, TraversalMode::BlockMask, origin, raw, max);
    fixtures::assert_same_hit(reference_hit, mask_hit, "solid content");
    assert_eq!(mask.material_reads, reference.material_reads);
    assert_eq!(mask.cells_examined, reference.material_reads);
    assert_eq!(reference_hit.map(|hit| hit.material), Some(7));
}

#[test]
fn cached_block_avoids_repeated_occupancy_word_loads() {
    // One block only: the ray crosses four cells of the same block before the hit.
    let world = world_with(&[([3, 3, 3], 21)]);
    let volume = fixtures::snapshot(&world, BlockShape::CUBE4, [0, 0, 0], [4, 4, 4]);
    assert_eq!(volume.occupancy().blocks(), 1);
    let (hit, stats) = trace(
        &volume,
        TraversalMode::BlockMask,
        [0.5, 3.5, 3.5],
        [1.0, 0.0, 0.0],
        8.0,
    );
    assert_eq!(hit.map(|hit| hit.cell), Some([3, 3, 3]));
    assert_eq!(stats.iterations, 4);
    assert_eq!(
        stats.cells_examined, 4,
        "each cell of the occupied block is tested"
    );
    assert_eq!(
        stats.occupancy_word_loads, 1,
        "the block word is fetched once"
    );
    assert_eq!(
        stats.material_reads, 1,
        "only the solid cell reads a material word"
    );
}

#[test]
fn block_step_trades_outer_iterations_for_catch_up_planes() {
    let world = world_with(&[([15, 15, 15], 4)]);
    let volume = fixtures::snapshot(&world, BlockShape::CUBE4, [0, 0, 0], [16, 16, 16]);
    let max = 40.0;
    let (reference_hit, reference) = trace(
        &volume,
        TraversalMode::Reference,
        [0.5; 3],
        [1.0, 1.0, 1.0],
        max,
    );
    let (mask_hit, mask) = trace(
        &volume,
        TraversalMode::BlockMask,
        [0.5; 3],
        [1.0, 1.0, 1.0],
        max,
    );
    let (step_hit, step) = trace(
        &volume,
        TraversalMode::BlockStep,
        [0.5; 3],
        [1.0, 1.0, 1.0],
        max,
    );
    fixtures::assert_same_hit(reference_hit, mask_hit, "block mask diagonal");
    fixtures::assert_same_hit(reference_hit, step_hit, "block step diagonal");
    assert_eq!(reference_hit.map(|hit| hit.cell), Some([15, 15, 15]));
    assert_eq!(reference_hit.map(|hit| hit.normal), Some([-1, 0, 0]));
    assert_eq!(mask.iterations, reference.iterations);
    assert!(step.block_steps > 0);
    assert!(step.catch_up_planes > 0);
    assert!(
        step.iterations < mask.iterations,
        "coarse steps must remove outer iterations: {} vs {}",
        step.iterations,
        mask.iterations
    );
    assert_eq!(step.material_reads, mask.material_reads);
}

#[test]
fn block_shape_scales_occupancy_cost_not_hits() {
    let mut rng = Rng::new(0x3B);
    let shapes = [BlockShape::CUBE4, BlockShape::TALL_4_4_8, BlockShape::CUBE8];
    for seed in 0..16u64 {
        let fixture = sparse_fixture(seed);
        let volumes: Vec<HierarchyVolume> = shapes
            .iter()
            .map(|&shape| fixture.snapshot(shape))
            .collect();
        // Exact accounting per shape: whole words per block, including masked edge
        // blocks, so a coarser shape is not automatically smaller in total bytes.
        for (volume, shape) in volumes.iter().zip(shapes) {
            let memory = volume.memory_stats();
            let blocks = memory.occupancy_words / shape.words_per_block();
            assert_eq!(memory.occupancy_words, blocks * shape.words_per_block());
            assert_eq!(memory.occupancy_bytes, memory.occupancy_words * 4);
            assert_eq!(
                memory.occupancy_words,
                volume.build_report().occupancy_words_allocated
            );
        }
        println!(
            "shape bytes: {:?}",
            volumes
                .iter()
                .map(|v| v.memory_stats().occupancy_bytes)
                .collect::<Vec<_>>()
        );
        let mut loads = [0u64; 3];
        for _ in 0..24 {
            let start = rng.pick(&[StartKind::Center, StartKind::Integer, StartKind::Outside]);
            let (origin, raw, max) = fixtures::random_ray(&mut rng, &fixture, start);
            let mut hits = Vec::new();
            for (index, volume) in volumes.iter().enumerate() {
                let (hit, stats) = trace(volume, TraversalMode::BlockStep, origin, raw, max);
                loads[index] += stats.occupancy_word_loads;
                hits.push(hit);
            }
            fixtures::assert_same_hit(hits[0], hits[1], "cube4 vs 4x4x8");
            fixtures::assert_same_hit(hits[0], hits[2], "cube4 vs cube8");
        }
        // Block grids are nested, so coarser blocks cannot be entered more often.
        assert!(
            loads[2] <= loads[1] && loads[1] <= loads[0],
            "block word loads {loads:?}"
        );
    }
}
