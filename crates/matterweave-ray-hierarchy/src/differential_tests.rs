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
//! unclassified disagreement to fail loudly. A disagreement is classified only when both
//! hits are at the same distance, in the same material, in cells one step apart on every
//! differing axis, with the reported hit point lying on the plane each of those axes
//! shares — an exact face, edge or corner tie — and with unit axis normals consistent with
//! the two walks' tie rules. Anything else — a material change, a multi-cell jump, a point
//! off a shared plane, a non-axis or inconsistent normal — is a real defect and fails. The
//! count is reported, never hidden.
//!
//! A hit/miss difference has disclosed sources of its own at the crop boundary: the
//! inherited zero-length overlap rule (the crop path rejects a point contact the CPU DDA
//! answers at distance 0) and an exact tie on the crop's exit face (the accumulated oracle
//! steps into an in-crop corner cell the recomputed walk exits past). Those cases are
//! classified and counted separately, and each requires the hit point to lie exactly at
//! the crop's exit parameter on its cell boundary, so they cannot excuse other mismatches.

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
/// Exact number of tied-plane resolutions the seeded corpus is allowed, each enumerated
/// in `docs/performance/logs/ray-hierarchy-experiment.md`.
const SEEDED_TIE_ARTIFACTS: usize = 3;
/// Exact number of exact crop-boundary ties and point contacts in the seeded corpus.
const SEEDED_CROP_BOUNDARY_DEVIATIONS: usize = 4;

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
    /// Hit/miss differences explained by an exact tie or point contact on the crop
    /// boundary, where the oracle's accumulated distances take an in-crop corner cell the
    /// recomputed walk exits past.
    crop_boundary_deviations: usize,
    examples: Vec<String>,
    crop_boundary_examples: Vec<String>,
}

impl Comparison {
    fn report(&self, label: &str) {
        println!(
            "{label}: {} hits, {} misses, max distance delta {:e}, {} tied-plane \
             resolutions, {} crop-boundary ties",
            self.hits,
            self.misses,
            self.max_delta,
            self.tie_artifacts,
            self.crop_boundary_deviations
        );
        for example in &self.examples {
            println!("  tie example: {example}");
        }
        for example in &self.crop_boundary_examples {
            println!("  crop-boundary example: {example}");
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

/// Whether a hit's normal names a face of its own cell that contains `point`.
///
/// A traversal reports `normal = -step` for the face it crossed to enter the cell, so a
/// negative component names the cell's lower face on that axis and a positive one its
/// upper face. A zero normal is only the inside-solid start: distance 0, no crossed face,
/// and the point inside the cell. Anything else is inconsistent geometry.
fn normal_names_face(hit: &RayHit, point: [f64; 3], tolerance: f64) -> bool {
    let mut axes = 0;
    for (axis, &value) in hit.normal.iter().enumerate() {
        if value == 0 {
            continue;
        }
        if value.abs() != 1 {
            return false;
        }
        axes += 1;
        let cell_lo = f64::from(hit.cell[axis]);
        let face = if value > 0 { cell_lo + 1.0 } else { cell_lo };
        if (point[axis] - face).abs() > tolerance {
            return false;
        }
    }
    if axes > 0 {
        return true;
    }
    if f64::from(hit.distance) != 0.0 {
        return false;
    }
    (0..3).all(|axis| {
        let cell_lo = f64::from(hit.cell[axis]);
        point[axis] >= cell_lo - tolerance && point[axis] <= cell_lo + 1.0 + tolerance
    })
}

/// Whether two hits differ only by how the two walks resolved the same exact tie.
///
/// The oracle accumulates plane distances; this crate recomputes every crossing from
/// integer planes and steps every axis whose recomputed crossing is exactly the minimum.
/// At a tie the two styles can reach the same point with different cells or a different
/// entry face. Such a disagreement is accepted only when it is fully explained by the
/// geometry:
///
/// - the material and the distance (and therefore the hit point) agree;
/// - each hit's normal names a face of its own cell containing that point;
/// - a differing cell is one step away on each differing axis, and the point lies on the
///   plane the two cells share there — an extra step without a crossing cannot satisfy it.
///
/// Everything else — a material change, a multi-cell jump, a point off a shared plane, a
/// non-axis normal, a normal naming a face the point is not on — is unexplained.
fn is_tied_plane_artifact(
    candidate: &RayHit,
    expected: &RayHit,
    origin: [f64; 3],
    direction: [f64; 3],
) -> bool {
    if candidate.material != expected.material {
        return false;
    }
    let distance = f64::from(candidate.distance);
    let tolerance = TIE_DISTANCE_TOLERANCE * f64::max(distance, 1.0);
    let delta = (f64::from(candidate.distance) - f64::from(expected.distance)).abs();
    if delta > tolerance {
        return false;
    }
    let point = [
        origin[0] + direction[0] * distance,
        origin[1] + direction[1] * distance,
        origin[2] + direction[2] * distance,
    ];
    if !normal_names_face(candidate, point, tolerance)
        || !normal_names_face(expected, point, tolerance)
    {
        return false;
    }
    if candidate.cell == expected.cell {
        // Same cell and point: only the named entry face can be ambiguous.
        return true;
    }
    for (axis, (&candidate_cell, &expected_cell)) in
        candidate.cell.iter().zip(expected.cell.iter()).enumerate()
    {
        let offset = candidate_cell - expected_cell;
        if offset == 0 {
            continue;
        }
        if offset.abs() != 1 {
            return false;
        }
        // The hit point must lie on the plane the two cells share: the step was a real
        // plane crossing at exactly the reported distance, not a skipped test.
        let face = f64::from(candidate_cell.max(expected_cell));
        if (point[axis] - face).abs() > tolerance {
            return false;
        }
    }
    true
}

/// Whether a hit/miss difference is an exact tie or point contact on the crop boundary,
/// where the recomputed walk and the oracle legitimately disagree.
///
/// Two shapes occur. A point contact at the crop's lower face: the origin lies exactly on
/// the face, the ray leaves the crop, and the crop path rejects the zero-length overlap
/// while the DDA reports the origin cell at distance 0. And an exact tie on the crop's
/// exit face: the oracle's accumulated distances step into an in-crop corner cell that the
/// recomputed simultaneous-tie walk (and the reference shader) exits past.
///
/// Both require the oracle's hit point to lie exactly at the parameter where the ray
/// leaves the crop *and* on the boundary of its hit cell, on the exiting axis: a
/// point-only contact the half-open crop does not take. A hit point reached inside the
/// crop and inside its cell can never be excused this way.
fn is_crop_boundary_deviation(
    volume: &HierarchyVolume,
    expected: &RayHit,
    origin: [f64; 3],
    direction: [f64; 3],
) -> bool {
    let distance = f64::from(expected.distance);
    let tolerance = TIE_DISTANCE_TOLERANCE * f64::max(distance, 1.0);
    let lower = volume.origin();
    let mut exit = f64::INFINITY;
    let mut on_cell_boundary = false;
    for axis in 0..3 {
        let lo = f64::from(lower[axis]);
        let hi = lo + f64::from(volume.dimensions()[axis]);
        if direction[axis] > 0.0 {
            exit = exit.min((hi - origin[axis]) / direction[axis]);
        } else if direction[axis] < 0.0 {
            exit = exit.min((lo - origin[axis]) / direction[axis]);
        }
    }
    if (distance - exit).abs() > tolerance {
        return false;
    }
    let point = [
        origin[0] + direction[0] * distance,
        origin[1] + direction[1] * distance,
        origin[2] + direction[2] * distance,
    ];
    for (axis, &coordinate) in point.iter().enumerate() {
        let cell_lo = f64::from(expected.cell[axis]);
        if (coordinate - cell_lo).abs() <= tolerance
            || (coordinate - cell_lo - 1.0).abs() <= tolerance
        {
            on_cell_boundary = true;
        }
    }
    on_cell_boundary
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
    let direction = fixtures::oracle_direction(raw_direction);
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
                is_tied_plane_artifact(&candidate, &expected, origin, direction),
                "cell/material/normal disagreement is not a tied-plane resolution: \
                 candidate {candidate:?} oracle {expected:?} at {context}"
            );
            comparison.tie_artifacts += 1;
            comparison.examples.push(format!(
                "candidate {candidate:?} oracle {expected:?} at {context}"
            ));
        }
        (None, Some(expected)) => {
            assert!(
                is_crop_boundary_deviation(volume, &expected, origin, direction),
                "crop path missed where the oracle hit, and not by an exact crop-boundary \
                 tie or point contact: oracle {expected:?} at {context}"
            );
            comparison.crop_boundary_deviations += 1;
            if comparison.crop_boundary_examples.len() < 5 {
                comparison
                    .crop_boundary_examples
                    .push(format!("oracle {expected:?} at {context}"));
            }
        }
        (Some(candidate), None) => {
            panic!("crop path hit where the oracle missed: candidate {candidate:?} at {context}")
        }
    }
}

#[test]
fn tied_plane_classifier_rejects_unexplained_disagreements() {
    // Geometry for the positive controls: a +x ray from x = -1 whose distance 2 lands
    // exactly on the plane x = 1 shared by the cells [0, ..] and [1, ..].
    let face_origin = [-1.0, 0.5, 0.25];
    let face_direction = [1.0, 0.0, 0.0];
    let expected = RayHit {
        cell: [0, 0, 0],
        normal: [1, 0, 0],
        distance: 2.0,
        material: 7,
    };
    let stepped = RayHit {
        cell: [1, 0, 0],
        normal: [-1, 0, 0],
        distance: 2.0,
        material: 7,
    };
    // The candidate stepped one cell along x into a cell whose lower x face contains the
    // point; the oracle reports the previous cell's upper x face. Same point, same
    // material, same distance.
    assert!(is_tied_plane_artifact(
        &stepped,
        &expected,
        face_origin,
        face_direction
    ));
    // Geometry for the edge and corner controls: a diagonal +x +y ray from (-1, -1, ..)
    // whose 2 * sqrt(2) distance lands on the corner (1, 1, 0.25).
    let diagonal_distance = 8.0f64.sqrt();
    let diagonal_direction = [2.0 / diagonal_distance, 2.0 / diagonal_distance, 0.0];
    let diagonal_origin = [-1.0, -1.0, 0.25];
    let corner = RayHit {
        cell: [0, 0, 0],
        normal: [1, 0, 0],
        distance: diagonal_distance as f32,
        material: 7,
    };
    // Same cell and same point: both the x = 1 and the y = 1 face contain it, so either
    // entry face is a legitimate answer.
    let corner_other_face = RayHit {
        normal: [0, 1, 0],
        ..corner
    };
    assert!(is_tied_plane_artifact(
        &corner_other_face,
        &corner,
        diagonal_origin,
        diagonal_direction
    ));
    // A two-axis cell difference is accepted only because the point is on both shared
    // planes: this is the exact corner tie the seeded corpus produces.
    let corner_neighbour = RayHit {
        cell: [1, 1, 0],
        normal: [-1, 0, 0],
        distance: diagonal_distance as f32,
        material: 7,
    };
    assert!(is_tied_plane_artifact(
        &corner_neighbour,
        &corner,
        diagonal_origin,
        diagonal_direction
    ));
    // Negative controls. A different material is never a tie artifact.
    let different_material = RayHit {
        material: 8,
        ..stepped
    };
    assert!(!is_tied_plane_artifact(
        &different_material,
        &expected,
        face_origin,
        face_direction
    ));
    // A diagonal neighbour with the same material and a valid normal, but the hit point
    // is inside both cells rather than on the y plane they share: the extra y step had no
    // crossing at the reported distance.
    let diagonal_neighbour = RayHit {
        cell: [1, 1, 0],
        normal: [-1, 0, 0],
        distance: 2.0,
        material: 7,
    };
    assert!(!is_tied_plane_artifact(
        &diagonal_neighbour,
        &expected,
        face_origin,
        face_direction
    ));
    // Two cells along one axis: no tie can step that far.
    let double_step = RayHit {
        cell: [2, 0, 0],
        normal: [-1, 0, 0],
        distance: 2.0,
        material: 7,
    };
    assert!(!is_tied_plane_artifact(
        &double_step,
        &expected,
        face_origin,
        face_direction
    ));
    // Same cells and material, but the hit distance is not the face crossing: the named
    // normal does not contain the point.
    let off_face = RayHit {
        distance: 1.5,
        ..stepped
    };
    assert!(!is_tied_plane_artifact(
        &off_face,
        &expected,
        face_origin,
        face_direction
    ));
    // A normal that is not one unit axis vector.
    let skewed_normal = RayHit {
        normal: [-1, -1, 0],
        ..stepped
    };
    assert!(!is_tied_plane_artifact(
        &skewed_normal,
        &expected,
        face_origin,
        face_direction
    ));
    // A normal that points at the wrong face of its own cell.
    let inconsistent_sign = RayHit {
        cell: [1, 0, 0],
        normal: [1, 0, 0],
        distance: 2.0,
        material: 7,
    };
    assert!(!is_tied_plane_artifact(
        &inconsistent_sign,
        &expected,
        face_origin,
        face_direction
    ));
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
            // All four start classes, including origins outside the crop: entry clipping,
            // slab-plane snapping and the tied crossing at entry face the oracle here too,
            // not only in the reference-free mode comparison.
            let start = match index % 4 {
                0 => StartKind::Center,
                1 => StartKind::Integer,
                2 => StartKind::Quarter,
                _ => StartKind::Outside,
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
    comparison.report("seeded oracle corpus");
    // Both classes are pinned exactly and enumerated in the log, so a new disagreement
    // cannot hide behind a percentage budget.
    assert_eq!(
        comparison.tie_artifacts, SEEDED_TIE_ARTIFACTS,
        "tied-plane resolutions (see the log's enumeration)"
    );
    assert_eq!(
        comparison.crop_boundary_deviations, SEEDED_CROP_BOUNDARY_DEVIATIONS,
        "crop-boundary ties (see the log's enumeration)"
    );
}

#[test]
fn max_distance_domain_matches_the_world_oracle() {
    let mut world = World::new(31);
    assert!(world.set([0, 0, 0], 3));
    assert!(world.set([2, 0, 0], 4));
    let volume = fixtures::snapshot(&world, BlockShape::TALL_4_4_8, [0, 0, 0], [4, 4, 4]);
    let mut comparison = Comparison::default();
    // Zero-length queries and ranges above the cap: both sides must answer the same.
    for max in [0.0, 1.0e7, 4096.0] {
        for (origin, direction) in [
            ([0.25, 0.25, 0.75], [1.0, 0.0, 0.0]),
            ([1.5, 0.5, 0.5], [1.0, 0.0, 0.0]),
        ] {
            for mode in MODES {
                compare_with_oracle(
                    &world,
                    &volume,
                    mode,
                    origin,
                    direction,
                    max,
                    &mut comparison,
                );
            }
        }
    }
    comparison.report("max-distance domain");
    assert!(comparison.hits >= 9, "hits {}", comparison.hits);
    assert_eq!(comparison.tie_artifacts, 0);
    assert_eq!(comparison.crop_boundary_deviations, 0);
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
