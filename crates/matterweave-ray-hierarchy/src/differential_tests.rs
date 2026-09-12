//! Differential evidence: the authoritative CPU oracle, the reference mode and the
//! packed-occupancy modes.
//!
//! # Why the oracle comparison classifies exact-tie disagreements
//!
//! `World::raycast` accumulates plane distances (`next += delta`) in `f64` from the world
//! origin; this crate recomputes every crossing from integer planes. Inputs are dyadic
//! and identical on both sides, so hits agree on cell, material and normal in general.
//! When two crossing planes are mathematically tied, the two evaluation styles can round
//! the tied distances differently in the last bits, so one side may step one more axis at
//! that iteration. The retained reference discloses the same class of difference between
//! accumulated f64 CPU and recomputed f32 WGSL. This module therefore requires an
//! unclassified disagreement to fail loudly, and classifies a disagreement only when the
//! two hits are at the same distance and in face-adjacent cells, which is exactly a tie
//! resolved on either side of one plane. The count is reported, never hidden.

use crate::fixtures::{self, sparse_fixture, Rng, StartKind};
use crate::{
    clip_depth, BlockShape, HierarchyVolume, RayHit, SourceKey, TraversalMode, TraversalStats,
};
use glam::{Mat4, Vec3};
use matterweave_core::World;
use matterweave_render::ray_reference::RayVolume;
use matterweave_render::Sun;

/// Distance agreement bound for the oracle comparison, far below one cell.
const DISTANCE_TOLERANCE: f64 = 1e-4;
/// Relative bound for two hits to count as the same point at a tied plane.
const TIE_DISTANCE_TOLERANCE: f64 = 1e-6;

const MODES: [TraversalMode; 3] = [
    TraversalMode::Reference,
    TraversalMode::BlockMask,
    TraversalMode::BlockStep,
];

fn sum_stats(left: TraversalStats, right: TraversalStats) -> TraversalStats {
    TraversalStats {
        iterations: left.iterations + right.iterations,
        fine_steps: left.fine_steps + right.fine_steps,
        block_steps: left.block_steps + right.block_steps,
        catch_up_planes: left.catch_up_planes + right.catch_up_planes,
        blocks_checked: left.blocks_checked + right.blocks_checked,
        occupancy_word_loads: left.occupancy_word_loads + right.occupancy_word_loads,
        cells_examined: left.cells_examined + right.cells_examined,
        material_reads: left.material_reads + right.material_reads,
        exhausted: left.exhausted || right.exhausted,
    }
}

#[derive(Default)]
struct Comparison {
    hits: usize,
    misses: usize,
    max_delta: f64,
    tie_artifacts: usize,
    examples: Vec<String>,
}

impl Comparison {
    fn report(&self, label: &str) {
        println!(
            "{label}: {} hits, {} misses, max distance delta {:e}, {} tied-plane resolutions",
            self.hits, self.misses, self.max_delta, self.tie_artifacts
        );
        for example in &self.examples {
            println!("  tie example: {example}");
        }
    }
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

/// Whether two hits differ only by which side of one tied plane was taken.
fn is_tied_plane_artifact(candidate: &RayHit, expected: &RayHit) -> bool {
    let delta = (f64::from(candidate.distance) - f64::from(expected.distance)).abs();
    if delta > TIE_DISTANCE_TOLERANCE * f64::from(candidate.distance) {
        return false;
    }
    let mut differing = 0usize;
    for axis in 0..3 {
        let offset = candidate.cell[axis] - expected.cell[axis];
        if offset != 0 {
            if offset.abs() != 1 {
                return false;
            }
            differing += 1;
        }
    }
    differing > 0
}

fn compare_with_oracle(
    world: &World,
    volume: &HierarchyVolume,
    mode: TraversalMode,
    origin: [f64; 3],
    raw_direction: [f64; 3],
    max_distance: f64,
    comparison: &mut Comparison,
) {
    let candidate = trace(volume, mode, origin, raw_direction, max_distance).0;
    let expected = fixtures::oracle(world, origin, raw_direction, max_distance);
    let context =
        format!("{mode:?} origin {origin:?} direction {raw_direction:?} max {max_distance}");
    match (candidate, expected) {
        (None, None) => comparison.misses += 1,
        (Some(candidate), Some(expected)) => {
            let delta = (f64::from(candidate.distance) - f64::from(expected.distance)).abs();
            comparison.max_delta = comparison.max_delta.max(delta);
            assert!(
                delta <= DISTANCE_TOLERANCE,
                "distance {delta} above tolerance: {context}"
            );
            if candidate == expected {
                comparison.hits += 1;
                return;
            }
            assert!(
                is_tied_plane_artifact(&candidate, &expected),
                "cell/material/normal disagreement is not a tied-plane resolution: \
                 candidate {candidate:?} oracle {expected:?} at {context}"
            );
            comparison.tie_artifacts += 1;
            if comparison.examples.len() < 3 {
                comparison.examples.push(format!(
                    "candidate {candidate:?} oracle {expected:?} at {context}"
                ));
            }
        }
        _ => panic!("hit/miss mismatch: candidate {candidate:?} oracle {expected:?} at {context}"),
    }
}

/// One targeted fixture and the rays cast against it. Origins are strictly inside the
/// crop so the crop clip cannot change the answer.
struct Case {
    name: &'static str,
    cells: Vec<([i32; 3], u8)>,
    origin: [i32; 3],
    dimensions: [u32; 3],
    rays: Vec<([f64; 3], [f64; 3])>,
}

fn targeted_cases() -> Vec<Case> {
    let wall: Vec<([i32; 3], u8)> = (0..4)
        .flat_map(|y| (0..4).filter_map(move |z| ((y, z) != (1, 1)).then_some(([3, y, z], 11))))
        .chain(std::iter::once(([5, 1, 1], 12)))
        .collect();
    vec![
        Case {
            name: "tied diagonal into a corner cell",
            cells: vec![([-2, -2, -2], 2), ([-2, -1, -2], 3)],
            origin: [-2, -2, -2],
            dimensions: [4, 4, 4],
            rays: vec![
                ([-2.5, -2.5, -2.5], [1.0, 1.0, 1.0]),
                ([-1.5, -1.5, -1.5], [1.0, 1.0, 1.0]),
                ([-1.5, -2.5, -1.5], [1.0, 1.0, 1.0]),
                ([-2.5, -1.5, -1.5], [-1.0, 1.0, 1.0]),
            ],
        },
        Case {
            name: "axis rays with zero direction components",
            cells: vec![([0, 1, 1], 5), ([1, 1, 1], 6), ([2, 1, 1], 7)],
            origin: [-2, -2, -2],
            dimensions: [6, 6, 6],
            rays: vec![
                ([-1.5, 1.5, 1.5], [1.0, 0.0, 0.0]),
                ([-1.5, 1.5, 1.5], [-1.0, 0.0, 0.0]),
                ([-1.5, 1.5, 1.5], [0.0, 1.0, 0.0]),
                ([-1.5, 1.5, 1.5], [0.0, 0.0, 1.0]),
                ([-1.5, 1.5, 1.5], [1.0, 0.0, 0.25]),
                ([-1.5, 1.5, 1.5], [1.0, 0.5, 0.0]),
            ],
        },
        Case {
            name: "inside solid start",
            cells: vec![([0, 0, 0], 8)],
            origin: [0, 0, 0],
            dimensions: [4, 4, 4],
            rays: vec![
                ([0.25, 0.25, 0.75], [1.0, 0.0, 0.0]),
                ([0.5, 0.5, 0.5], [1.0, 1.0, 1.0]),
                ([3.5, 0.5, 0.5], [-1.0, 0.0, 0.0]),
            ],
        },
        Case {
            name: "thin wall with a one-cell opening",
            cells: wall,
            origin: [0, 0, 0],
            dimensions: [8, 4, 4],
            rays: vec![
                ([0.5, 0.5, 0.5], [1.0, 0.0, 0.0]),
                ([0.5, 1.5, 1.5], [1.0, 0.0, 0.0]),
                ([0.5, 1.5, 1.5], [1.0, 0.25, 0.0]),
                ([0.5, 2.5, 0.5], [1.0, 0.0, 0.5]),
            ],
        },
        Case {
            name: "negative crop edges",
            cells: vec![
                ([-17, -1, -1], 1),
                ([-16, -1, -1], 2),
                ([-17, 0, -1], 3),
                ([-16, 0, 0], 4),
            ],
            origin: [-17, -1, -1],
            dimensions: [2, 2, 2],
            rays: vec![
                ([-16.5, -0.5, -0.5], [1.0, 0.0, 0.0]),
                ([-16.5, -0.5, -0.5], [-1.0, 0.0, 0.0]),
                ([-16.5, -0.5, -0.5], [0.0, 1.0, 0.0]),
                ([-16.5, -0.5, -0.5], [0.0, 0.0, 1.0]),
                ([-16.5, -0.5, -0.5], [1.0, 1.0, 1.0]),
            ],
        },
    ]
}

#[test]
fn targeted_cases_match_the_world_oracle() {
    let mut comparison = Comparison::default();
    for case in targeted_cases() {
        println!("targeted case: {}", case.name);
        let mut world = World::new(17);
        for &(cell, material) in &case.cells {
            assert!(world.set(cell, material), "fixture write {cell:?}");
        }
        for shape in [BlockShape::CUBE4, BlockShape::TALL_4_4_8] {
            let volume = fixtures::snapshot(&world, shape, case.origin, case.dimensions);
            for &(origin, direction) in &case.rays {
                for mode in MODES {
                    compare_with_oracle(
                        &world,
                        &volume,
                        mode,
                        origin,
                        direction,
                        12.0,
                        &mut comparison,
                    );
                }
            }
        }
    }
    comparison.report("targeted oracle cases");
    // The corpus is five fixtures x two shapes x four to six rays x three modes.
    assert!(comparison.hits > 80, "hits {}", comparison.hits);
    assert!(comparison.misses > 20, "misses {}", comparison.misses);
    assert_eq!(
        comparison.tie_artifacts, 0,
        "targeted dyadic rays must agree exactly with the oracle"
    );
}

#[test]
fn seeded_corpus_matches_the_world_oracle() {
    let mut comparison = Comparison::default();
    for seed in 0..48u64 {
        let fixture = sparse_fixture(seed);
        let volume = fixture.snapshot(BlockShape::TALL_4_4_8);
        let mut rng = Rng::new(0xE0 + seed);
        for index in 0..16 {
            // Origins stay strictly inside the crop: entry clipping is covered by the
            // mode comparison, where the crop semantics are the subject.
            let start = match index % 2 {
                0 => StartKind::Center,
                _ => StartKind::Quarter,
            };
            let (origin, raw, max) = fixtures::random_ray(&mut rng, &fixture, start);
            compare_with_oracle(
                &fixture.world,
                &volume,
                TraversalMode::Reference,
                origin,
                raw,
                max,
                &mut comparison,
            );
        }
    }
    assert!(comparison.hits > 40, "hits {}", comparison.hits);
    assert!(comparison.misses > 40, "misses {}", comparison.misses);
    assert!(
        comparison.tie_artifacts * 50 < comparison.hits + comparison.misses,
        "tied-plane resolutions must stay rare: {} of {}",
        comparison.tie_artifacts,
        comparison.hits + comparison.misses
    );
    comparison.report("seeded oracle corpus");
}

#[test]
fn seeded_corpus_modes_agree_bit_for_bit() {
    let shapes = [BlockShape::CUBE4, BlockShape::TALL_4_4_8, BlockShape::CUBE8];
    let mut rng = Rng::new(0xD1);
    let mut hits = 0usize;
    let mut misses = 0usize;
    let mut block_steps = 0u64;
    // Structural counters per mode over the whole corpus. These are buffer-access
    // counts, not timings and not measured bandwidth.
    let mut totals = [TraversalStats::default(); 3];
    for seed in 0..48u64 {
        let fixture = sparse_fixture(seed);
        for shape in shapes {
            let volume = fixture.snapshot(shape);
            for index in 0..16 {
                let start = match index % 4 {
                    0 => StartKind::Center,
                    1 => StartKind::Integer,
                    2 => StartKind::Quarter,
                    _ => StartKind::Outside,
                };
                let (origin, raw, max) = fixtures::random_ray(&mut rng, &fixture, start);
                let (reference_hit, reference) =
                    trace(&volume, TraversalMode::Reference, origin, raw, max);
                assert!(
                    !reference.exhausted,
                    "reference exhausted at {origin:?} {raw:?} shape {shape:?}"
                );
                assert_eq!(
                    reference.fine_steps,
                    reference.iterations.saturating_sub(1),
                    "the reference mode never skips a step"
                );
                totals[0] = sum_stats(totals[0], reference);
                for (index, mode) in [TraversalMode::BlockMask, TraversalMode::BlockStep]
                    .into_iter()
                    .enumerate()
                {
                    let (hit, stats) = trace(&volume, mode, origin, raw, max);
                    totals[index + 1] = sum_stats(totals[index + 1], stats);
                    fixtures::assert_same_hit(
                        reference_hit,
                        hit,
                        &format!(
                            "{mode:?} shape {shape:?} seed {seed} origin {origin:?} direction {raw:?}"
                        ),
                    );
                    assert!(!stats.exhausted);
                    assert!(stats.iterations <= reference.iterations);
                    block_steps += stats.block_steps;
                }
                if reference_hit.is_some() {
                    hits += 1;
                } else {
                    misses += 1;
                }
            }
        }
    }
    assert!(hits > 500, "hits {hits}");
    assert!(misses > 200, "misses {misses}");
    // Bit-gated material reads happen exactly once per solid hit over the whole corpus:
    // no bit ever claims solid for an air cell that the traversal would then read.
    assert_eq!(totals[1].material_reads, hits as u64);
    assert_eq!(totals[2].material_reads, hits as u64);
    assert!(block_steps > 100, "coarse steps exercised: {block_steps}");
    println!("mode corpus: {hits} hits, {misses} misses, {block_steps} coarse steps");
    for (mode, stats) in ["Reference", "BlockMask", "BlockStep"]
        .into_iter()
        .zip(totals)
    {
        println!(
            "  {mode}: iterations {}, fine steps {}, block steps {}, catch-up planes {}, \
             blocks checked {}, block word loads {}, cells examined {}, material reads {}",
            stats.iterations,
            stats.fine_steps,
            stats.block_steps,
            stats.catch_up_planes,
            stats.blocks_checked,
            stats.occupancy_word_loads,
            stats.cells_examined,
            stats.material_reads
        );
    }
}

#[test]
fn block_boundary_cases_agree_bit_for_bit() {
    // Block extents 4, 4x4x8, 8 and 3x5x2 put these cells and planes on, inside and
    // beyond block boundaries, including a masked edge block.
    let shapes = [
        BlockShape::CUBE4,
        BlockShape::TALL_4_4_8,
        BlockShape::CUBE8,
        BlockShape::new(3, 5, 2).unwrap(),
    ];
    let cells: Vec<([i32; 3], u8)> = vec![
        ([0, 0, 0], 1),
        ([3, 3, 7], 2),
        ([4, 0, 0], 3),
        ([4, 4, 8], 4),
        ([7, 7, 15], 5),
        ([8, 8, 16], 6),
        ([11, 15, 19], 7),
        ([12, 3, 4], 8),
    ];
    let dimensions = [13, 16, 20];
    let origin = [-3, -5, -7];
    let mut world = World::new(23);
    for &(cell, material) in &cells {
        assert!(
            world.set(
                [
                    origin[0] + cell[0],
                    origin[1] + cell[1],
                    origin[2] + cell[2]
                ],
                material
            ),
            "fixture write {cell:?}"
        );
    }
    // Rays that travel along block planes, diagonally across block corners and along
    // the masked margin of an edge block.
    let rays: [([f64; 3], [f64; 3]); 14] = [
        ([-2.0, -4.0, -6.0], [1.0, 0.0, 0.0]),
        ([-2.0, -1.0, -6.0], [1.0, 0.0, 0.0]),
        ([1.0, -4.5, -6.5], [1.0, 0.0, 0.0]),
        ([0.5, 4.5, 8.5], [1.0, 0.0, 0.0]),
        ([0.5, 0.5, 0.5], [1.0, 1.0, 1.0]),
        ([-2.5, 0.0, 1.0], [1.0, 0.0, 0.0]),
        ([5.5, 8.5, 12.5], [-1.0, -1.0, -1.0]),
        ([3.0, -4.0, 2.5], [0.0, 1.0, 0.0]),
        // Rays that walk backwards into a chosen cell, one axis at a time.
        ([-2.5, -0.5, 1.5], [-1.0, 0.0, 0.0]),
        ([2.5, -0.5, 1.5], [-1.0, 0.0, 0.0]),
        ([5.5, 2.5, 8.5], [-1.0, 0.0, 0.0]),
        ([7.5, 10.5, 12.5], [1.0, 0.0, 0.0]),
        ([-2.5, -4.5, -6.5], [1.0, 0.0, 0.0]),
        ([-2.5, 4.5, 8.5], [1.0, 0.0, 0.0]),
    ];
    let mut comparison = Comparison::default();
    for shape in shapes {
        let volume = fixtures::snapshot(&world, shape, origin, dimensions);
        for &(ray_origin, direction) in &rays {
            let (reference_hit, _) = trace(
                &volume,
                TraversalMode::Reference,
                ray_origin,
                direction,
                60.0,
            );
            for mode in [TraversalMode::BlockMask, TraversalMode::BlockStep] {
                let (hit, stats) = trace(&volume, mode, ray_origin, direction, 60.0);
                fixtures::assert_same_hit(
                    reference_hit,
                    hit,
                    &format!(
                        "{mode:?} shape {shape:?} origin {ray_origin:?} direction {direction:?}"
                    ),
                );
                assert!(!stats.exhausted, "shape {shape:?} origin {ray_origin:?}");
            }
            if ray_origin.iter().enumerate().all(|(axis, &v)| {
                v >= f64::from(origin[axis])
                    && v < f64::from(origin[axis] + dimensions[axis] as i32)
            }) {
                compare_with_oracle(
                    &world,
                    &volume,
                    TraversalMode::BlockStep,
                    ray_origin,
                    direction,
                    60.0,
                    &mut comparison,
                );
            }
        }
    }
    comparison.report("block boundary oracle cases");
    // Every in-crop ray must produce an oracle comparison with a real answer.
    let expected_hits = rays
        .iter()
        .filter(|(ray_origin, _)| {
            (0..3).all(|axis| {
                ray_origin[axis] >= f64::from(origin[axis])
                    && ray_origin[axis] < f64::from(origin[axis] + dimensions[axis] as i32)
            })
        })
        .count();
    assert!(
        comparison.hits >= expected_hits,
        "hits {} of {} in-crop rays",
        comparison.hits,
        expected_hits
    );
}

#[test]
fn patched_snapshots_match_a_rebuilt_snapshot_and_the_edited_world() {
    let wall: Vec<([i32; 3], u8)> = (0..4)
        .flat_map(|y| (0..4).map(move |z| ([3, y, z], 31)))
        .collect();
    let mut world = World::new(29);
    for &(cell, material) in &wall {
        assert!(world.set(cell, material));
    }
    let origin = [0, 0, 0];
    let dimensions = [8, 4, 4];
    let shape = BlockShape::CUBE4;
    let pack = fixtures::pack(&world, 1, origin, dimensions);
    let rays = [
        ([0.5, 1.5, 1.5], [1.0, 0.0, 0.0]),
        ([0.5, 0.5, 0.5], [1.0, 0.0, 0.0]),
        ([0.5, 3.5, 2.5], [1.0, 0.0, 0.0]),
    ];
    // Punch a one-cell hole exactly on the block boundary plane x = 3.
    let mut patched = HierarchyVolume::from_reference(&pack, shape).unwrap();
    let report = patched.patch_cell([3, 1, 1], 0).unwrap();
    assert!(report.material_changed);
    assert!(report.occupancy_bit_changed);
    assert!(
        !report.block_became_empty,
        "the rest of the wall keeps this block occupied"
    );
    assert!(patched.locally_edited());
    // Authoritative data is untouched by a derived-snapshot patch, and the snapshot is
    // stale for its source world.
    assert_eq!(world.get([3, 1, 1]), 31);
    assert!(!patched.valid_for(&world, 1));
    // The source world's oracle still sees the wall: the patch is a derived divergence.
    assert_eq!(
        fixtures::oracle(&world, [0.5, 1.5, 1.5], [1.0, 0.0, 0.0], 12.0).map(|hit| hit.cell),
        Some([3, 1, 1])
    );
    // The same edit applied to the world and re-derived produces identical hits.
    assert!(world.set([3, 1, 1], 0));
    let rebuilt = fixtures::snapshot(&world, shape, origin, dimensions);
    let mut rebuilt_comparison = Comparison::default();
    for (ray_origin, direction) in rays {
        let (patched_hit, _) = trace(
            &patched,
            TraversalMode::BlockStep,
            ray_origin,
            direction,
            12.0,
        );
        let (patched_reference, _) = trace(
            &patched,
            TraversalMode::Reference,
            ray_origin,
            direction,
            12.0,
        );
        fixtures::assert_same_hit(patched_hit, patched_reference, "patched snapshot modes");
        let (rebuilt_hit, _) = trace(
            &rebuilt,
            TraversalMode::BlockStep,
            ray_origin,
            direction,
            12.0,
        );
        fixtures::assert_same_hit(patched_hit, rebuilt_hit, "patched vs rebuilt");
        compare_with_oracle(
            &world,
            &rebuilt,
            TraversalMode::BlockStep,
            ray_origin,
            direction,
            12.0,
            &mut rebuilt_comparison,
        );
    }
    // Filling the hole again closes the gap, in the patch path and after a rebuild.
    let pack_with_hole = fixtures::pack(&world, 2, origin, dimensions);
    let mut filled = HierarchyVolume::from_reference(&pack_with_hole, shape).unwrap();
    assert!(filled.patch_cell([3, 1, 1], 44).unwrap().material_changed);
    let report = filled.patch_cell([3, 1, 1], 44).unwrap();
    assert!(
        !report.material_changed,
        "repeating a patch changes nothing"
    );
    assert!(world.set([3, 1, 1], 44));
    let refilled = fixtures::snapshot(&world, shape, origin, dimensions);
    for (ray_origin, direction) in rays {
        let (filled_hit, _) = trace(
            &filled,
            TraversalMode::BlockStep,
            ray_origin,
            direction,
            12.0,
        );
        let (refilled_hit, _) = trace(
            &refilled,
            TraversalMode::BlockStep,
            ray_origin,
            direction,
            12.0,
        );
        fixtures::assert_same_hit(filled_hit, refilled_hit, "refilled vs rebuilt");
        assert_eq!(
            filled_hit,
            fixtures::oracle(&world, ray_origin, direction, 12.0),
            "the refilled snapshot matches the edited world"
        );
    }
}

#[test]
fn staleness_matches_the_reference_pack_rule() {
    let mut world = World::new(7);
    assert!(world.set([-1, 0, 0], 9));
    let shape = BlockShape::TALL_4_4_8;
    let pack = fixtures::pack(&world, 3, [-1, 0, 0], [2, 1, 1]);
    let volume = HierarchyVolume::from_reference(&pack, shape).unwrap();
    assert!(volume.valid_for(&world, 3));
    for epoch in [0u64, 2, 3, 4] {
        assert_eq!(
            volume.valid_for(&world, epoch),
            pack.valid_for(&world, epoch),
            "epoch {epoch}"
        );
    }
    // Any edit anywhere invalidates both, including edits outside the crop.
    let mut edited = world.clone();
    assert!(edited.set([200, 0, 0], 1));
    assert_eq!(volume.valid_for(&edited, 3), pack.valid_for(&edited, 3));
    assert!(!volume.valid_for(&edited, 3));
    // A different seed invalidates both.
    let other_seed = World::new(8);
    assert_eq!(
        volume.valid_for(&other_seed, 3),
        pack.valid_for(&other_seed, 3)
    );
    assert!(!volume.valid_for(&other_seed, 3));
    // A local patch is stricter than the pack rule: the snapshot no longer describes one
    // published revision, while the source pack is still valid.
    let mut patched = HierarchyVolume::from_reference(&pack, shape).unwrap();
    patched.patch_cell([-1, 0, 0], 4).unwrap();
    assert!(!patched.valid_for(&world, 3));
    assert!(pack.valid_for(&world, 3));
    assert!(patched.locally_edited());
    assert_eq!(patched.key(), SourceKey::from_reference(&pack));
    assert_eq!(patched.source_epoch(), pack.source_epoch());
}

#[test]
fn crop_exclusion_and_out_of_crop_content_are_intentional() {
    // Content just outside the crop is invisible to the crop traversal while the
    // whole-world oracle reports it. This mirrors the retained reference's disclosed
    // boundary: comparison fixtures must fit inside the box.
    let mut world = World::new(0);
    assert!(world.set([0, 0, 0], 9));
    assert!(world.set([-1, 0, 0], 10));
    let volume = fixtures::snapshot(&world, BlockShape::CUBE4, [-4, 0, 0], [4, 1, 1]);
    assert_eq!(
        fixtures::oracle(&world, [0.5, 0.5, 0.5], [-1.0, 1.0, 0.0], 3.0).map(|hit| (
            hit.cell,
            hit.material,
            hit.distance
        )),
        Some(([0, 0, 0], 9, 0.0)),
        "the oracle starts inside the excluded cell"
    );
    for mode in MODES {
        let (hit, stats) = trace(&volume, mode, [0.5, 0.5, 0.5], [-1.0, 1.0, 0.0], 3.0);
        assert_eq!(hit, None, "{mode:?} excludes content outside the crop");
        assert_eq!(stats, TraversalStats::default());
    }
    // The same ray from inside the crop reports the in-crop cell only.
    let (hit, _) = trace(
        &volume,
        TraversalMode::BlockStep,
        [-0.5, 0.5, 0.5],
        [-1.0, 1.0, 0.0],
        3.0,
    );
    assert_eq!(
        hit.map(|hit| (hit.cell, hit.material)),
        Some(([-1, 0, 0], 10))
    );
}

#[test]
fn depth_matches_across_modes_and_the_projection_convention() {
    let fixture = sparse_fixture(2);
    let volume = fixture.snapshot(BlockShape::TALL_4_4_8);
    let pack = fixtures::pack(&fixture.world, 1, fixture.origin, fixture.dimensions);
    let eye = Vec3::new(
        f64::from(fixture.origin[0]) as f32 - 8.0,
        f64::from(fixture.origin[1]) as f32 + 12.0,
        f64::from(fixture.origin[2]) as f32 - 8.0,
    );
    let target = Vec3::new(
        fixture.right()[0] as f32 * 0.5,
        4.0,
        fixture.right()[2] as f32 * 0.5,
    );
    let view = Mat4::look_at_rh(eye, target, Vec3::Y);
    let mut rng = Rng::new(0xF0);
    let mut compared = 0usize;
    for projection in [
        Mat4::perspective_rh(1.0, 1.0, 0.1, 200.0),
        Mat4::orthographic_rh(-24.0, 24.0, -24.0, 24.0, 0.1, 200.0),
    ] {
        let view_projection = projection * view;
        let uniform = pack
            .uniform(view_projection, eye, Sun::default())
            .expect("finite fixture camera");
        for _ in 0..48 {
            let (origin, raw, max) = fixtures::random_ray(&mut rng, &fixture, StartKind::Center);
            let direction = fixtures::oracle_direction(raw);
            let (hit, _) = volume
                .trace_stats(TraversalMode::BlockStep, origin, direction, max)
                .unwrap();
            let Some(hit) = hit else { continue };
            let depth = clip_depth(
                &uniform.view_projection,
                origin,
                direction,
                f64::from(hit.distance),
            )
            .expect("finite depth");
            // Independent check of the column-major convention: glam's own product.
            let world = Vec3::new(
                origin[0] as f32 + direction[0] as f32 * hit.distance,
                origin[1] as f32 + direction[1] as f32 * hit.distance,
                origin[2] as f32 + direction[2] as f32 * hit.distance,
            );
            let clip = view_projection * world.extend(1.0);
            assert!(
                (depth - clip.z / clip.w).abs() <= 1e-6,
                "depth {depth} vs glam {}",
                clip.z / clip.w
            );
            assert!(
                (-1.0e-4..=1.0 + 1.0e-4).contains(&depth),
                "legal Vulkan depth range: {depth}"
            );
            // Every mode derives the same depth because it reports the same hit.
            for mode in MODES {
                let (other, _) = trace(&volume, mode, origin, raw, max);
                assert_eq!(other, Some(hit), "mode {mode:?}");
            }
            compared += 1;
        }
    }
    assert!(compared > 20, "compared {compared}");
    println!("depth comparison: {compared} hits");
}

#[test]
fn snapshot_construction_accepts_exactly_the_reference_crop_rules() {
    let world = World::new(5);
    let key = SourceKey {
        epoch: 0,
        revision: world.revision(),
        seed: world.seed(),
    };
    for (origin, dimensions) in [
        ([0, 0, 0], [1, 1, 1]),
        ([-8192, -8192, -8192], [1, 1, 1]),
        ([8191, 8191, 8191], [1, 1, 1]),
        ([-17, -1, -1], [3, 4, 5]),
        ([0, 0, 0], [0, 1, 1]),
        ([0, 0, 0], [129, 1, 1]),
        ([0, 0, 0], [64, 64, 65]),
        ([8192, 0, 0], [1, 1, 1]),
        ([-8193, 0, 0], [1, 1, 1]),
        ([-8192, 0, 0], [64, 1, 1]),
    ] {
        let pack = RayVolume::pack(&world, 0, origin, dimensions, fixtures::palette());
        let materials = vec![0u32; crate::cells_in(dimensions).unwrap_or(0)];
        let volume =
            HierarchyVolume::build(origin, dimensions, &materials, key, BlockShape::TALL_4_4_8);
        assert_eq!(
            volume.is_ok(),
            pack.is_ok(),
            "crop acceptance differs at {origin:?} {dimensions:?}"
        );
    }
}
