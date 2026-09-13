//! Functional feasibility probe for continuous moving-body GI (issue 47).
//!
//! The rejected slice proposal (`orchestration/opus-moving-lighting-slice`)
//! wanted dependency-bounded invalidation for a moving mesh proxy: diff the old
//! and new proxy, expand the changed cells by `ceil(R)`, mark those face slots
//! dirty, and lower the wetland gather distance to `R` so the dirty set stays
//! small. Its review (`orchestration/opus-moving-lighting-feasibility`) rejected
//! the radius: one sample traces **two** segments of up to `R` each (the gather
//! ray, then the hit-to-sun ray from the hit point), so a change can move a
//! value from up to `2R` away, and an `R`-only expansion silently publishes
//! wrong values under the same digest.
//!
//! This module is the executable functional probe for that claim. It is
//! `#[cfg(test)]`-only: it changes no algorithm, budget, gather distance,
//! shader or app constant, and it adopts no `R`. It measures on the shape the
//! wetland actually uses (20 x 10 x 20 cells, 4000 cells, 24 000 face slots, a
//! one-cell body moving one cell):
//!
//! 1. **Table.** For `R` in `{1, 2, 4, 8, 24}` (24 is the current production
//!    gather distance, 4 the rejected proposal's) with empty, sparse and dense
//!    surroundings at an edge and a mid-box body position: changed
//!    occupancy/material cells, the cells, face slots and exposed face slots
//!    inside the `ceil(R)` bound and inside the conservative `ceil(2R) + 1`
//!    bound, the ray and work-unit ceilings of both, and the whole-volume
//!    restart for comparison. Ray/work counts are engine work units derived from
//!    the published budget (rays 1024, work 8192); they are **not** time, and no
//!    latency claim is made from them.
//! 2. **Counterexample.** A small fixture where a one-cell move changes an
//!    exposed face from farther than `R` (in cell distance and in Euclidean
//!    origin-to-occluder distance), because the occluder blocks the *second*
//!    segment. The evidence is the real [`IndirectVolume::update`]; a
//!    reversed-sun control shows the change is sun visibility and not an
//!    accidental gather hit.
//! 3. **No omissions.** For every scene and `R`, two fresh volumes are computed
//!    to completion for the body before and after the move; every face whose
//!    value actually changed must lie inside the conservative bound. The count
//!    of changed faces outside the `ceil(R)` bound is reported as well, because
//!    that is exactly what the rejected expansion would have missed.
//!
//! All volumes are built through the public/crate API (`MeshProxy::build`,
//! `IndirectVolume::new/set_mesh_proxy/update/sample`); the probe never reaches
//! into the volume's private state, so it measures the shipped code path.
//!
//! What this module deliberately does **not** claim: that any frame count here
//! is a frame budget, that a dirty-set design is implemented, or that a bounded
//! gather distance is acceptable. It also does not assert a universal
//! "frames > 1": the empty-scene numbers show that the conservative bound at
//! small `R` can drain inside one budget slice.

use crate::indirect::{scene_material, IndirectVolume, MeshGeometry, MeshProxy, UpdateBudget};
use crate::static_scene::StaticInstance;
use crate::Sun;
use matterweave_core::{Mesh, World};

/// The wetland-shaped coverage box (20 x 10 x 20 cells), matching
/// `apps/explorer/src/wetland_lighting.rs` `BOX_DIMENSIONS`.
const BOX_ORIGIN: [i32; 3] = [-10, 0, -10];
const BOX_DIMENSIONS: [u32; 3] = [20, 10, 20];
/// Wetland quadrature and per-frame budget, copied here because the app crate is
/// not a dependency of the render crate. The probe only divides by these; it
/// never changes them.
const SAMPLES: u32 = 16;
const UPDATE_BUDGET: UpdateBudget = UpdateBudget {
    rays: 1024,
    work: 8192,
};
/// Gather distances compared: the rejected proposal's `R = 4`, the current
/// production `GATHER_DISTANCE_M = 24`, and intermediate points.
const GATHER_RADII: [f32; 5] = [1.0, 2.0, 4.0, 8.0, 24.0];
/// Probe-local palette indices; only nonzero matters for occupancy.
const FLOOR_MATERIAL: u8 = 11;
const ROCK_MATERIAL: u8 = 12;
const BODY_MATERIAL: u8 = 200;
/// Oblique sun: the hit-to-sun segment must have horizontal reach for the
/// second dependency hop to be exercised at all. Not the wetland's vertical sun.
const PROBE_SUN: Sun = Sun {
    direction_to_sun: [0.0, 1.0, 5.0],
    intensity: 1.0,
};
/// The one-cell body before and after a single-cell move, at two positions in
/// the box: near the edge, and mid-box where the same radius covers a much
/// larger fraction of the 4000 cells. Both cells are air in every fixture.
const BODY_MOVES: [(&str, [i32; 3], [i32; 3]); 2] = [
    ("edge", [-9, 1, -9], [-8, 1, -9]),
    ("center", [1, 1, 1], [2, 1, 1]),
];
/// Sparse debris: single cells resting on the floor.
const SPARSE_ROCKS: [[i32; 3]; 6] = [
    [-6, 1, -3],
    [-2, 1, 4],
    [3, 1, -6],
    [6, 1, 7],
    [-4, 1, 6],
    [7, 1, 1],
];

/// Face order is the documented `+X, -X, +Y, -Y, +Z, -Z` of `IndirectVolume`.
const FACE_NORMALS: [[i32; 3]; 6] = [
    [1, 0, 0],
    [-1, 0, 0],
    [0, 1, 0],
    [0, -1, 0],
    [0, 0, 1],
    [0, 0, -1],
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Surroundings {
    /// Body only: no geometry at all, so every gathered value is zero.
    Empty,
    /// Floor plus six scattered single cells.
    Sparse,
    /// Floor plus a checkerboard of two-cell pillars every other cell: dense
    /// cover, but sunlight still reaches the floor between the pillars.
    Dense,
}

fn cells_of(origin: [i32; 3], dimensions: [u32; 3]) -> impl Iterator<Item = [i32; 3]> {
    (0..dimensions[2]).flat_map(move |z| {
        (0..dimensions[1]).flat_map(move |y| {
            (0..dimensions[0]).map(move |x| {
                [
                    origin[0] + x as i32,
                    origin[1] + y as i32,
                    origin[2] + z as i32,
                ]
            })
        })
    })
}

fn chebyshev(a: [i32; 3], b: [i32; 3]) -> i32 {
    (0..3).map(|axis| (a[axis] - b[axis]).abs()).max().unwrap()
}

/// The rejected slice's expansion: `ceil(R)` cells (its own stated invariant).
fn r_only_radius_cells(r: f32) -> i32 {
    r.ceil() as i32
}

/// Conservative dependency radius in cells. A sample gathers up to `R` metres
/// and then tests sun visibility up to `R` metres from where it stopped, so any
/// cell that can change the sample lies within `2R` of the face's sample origin.
/// The origin sits `0.5 + eps` off the face cell's centre on the cell grid, and
/// the second segment starts `eps` off the hit face; one cell of index slack
/// covers both, so the bound is `ceil(2R) + 1` cells.
fn conservative_radius_cells(r: f32) -> i32 {
    (2.0 * r).ceil() as i32 + 1
}

fn surroundings_world(kind: Surroundings) -> World {
    let mut world = World::new(47);
    if kind != Surroundings::Empty {
        for cell in cells_of(BOX_ORIGIN, BOX_DIMENSIONS) {
            if cell[1] == 0 {
                world.set(cell, FLOOR_MATERIAL);
            }
        }
    }
    match kind {
        Surroundings::Empty => {}
        Surroundings::Sparse => {
            for cell in SPARSE_ROCKS {
                world.set(cell, ROCK_MATERIAL);
            }
        }
        Surroundings::Dense => {
            for cell in cells_of(BOX_ORIGIN, BOX_DIMENSIONS) {
                let even = |axis: usize| (cell[axis] - BOX_ORIGIN[axis]) % 2 == 0;
                if cell[1] > 0 && cell[1] < 3 && even(0) && even(2) {
                    world.set(cell, ROCK_MATERIAL);
                }
            }
        }
    }
    world
}

/// One unit cube: the engine mesher's own output for a single voxel, so the
/// proxy marks exactly the cell its translation names.
fn unit_cube() -> Mesh {
    let mut voxel = World::new(0);
    voxel.set([0, 0, 0], 1);
    voxel.mesh()
}

fn body_proxy(origin: [i32; 3], dimensions: [u32; 3], cell: [i32; 3]) -> MeshProxy {
    let meshes = [unit_cube()];
    let instances = [StaticInstance {
        prototype: 0,
        translation: cell.map(|value| value as f32),
        yaw_quarters: 0,
    }];
    let materials = [BODY_MATERIAL];
    MeshProxy::build(
        &MeshGeometry {
            meshes: &meshes,
            instances: &instances,
            materials: &materials,
        },
        origin,
        dimensions,
    )
    .expect("probe body proxy")
}

fn probe_palette() -> [[f32; 3]; 256] {
    let mut palette = [[0.5; 3]; 256];
    palette[FLOOR_MATERIAL as usize] = [0.9, 0.9, 0.9];
    palette[ROCK_MATERIAL as usize] = [0.4, 0.5, 0.4];
    palette[BODY_MATERIAL as usize] = [0.8, 0.2, 0.1];
    palette
}

/// Cells whose composed scene material differs between the two body placements.
fn changed_cells(
    world: &World,
    before: Option<&MeshProxy>,
    after: Option<&MeshProxy>,
    origin: [i32; 3],
    dimensions: [u32; 3],
) -> Vec<[i32; 3]> {
    cells_of(origin, dimensions)
        .filter(|&cell| scene_material(world, before, cell) != scene_material(world, after, cell))
        .collect()
}

/// `(occupancy changes, material-only changes)` over `changed`.
fn classify_changes(
    world: &World,
    before: Option<&MeshProxy>,
    after: Option<&MeshProxy>,
    changed: &[[i32; 3]],
) -> (usize, usize) {
    let (mut occupancy, mut material_only) = (0, 0);
    for &cell in changed {
        let (a, b) = (
            scene_material(world, before, cell),
            scene_material(world, after, cell),
        );
        if a == 0 || b == 0 {
            occupancy += 1;
        } else {
            material_only += 1;
        }
    }
    (occupancy, material_only)
}

/// Every in-box cell within `radius` Chebyshev cells of any changed cell.
fn bound_cells(
    origin: [i32; 3],
    dimensions: [u32; 3],
    changed: &[[i32; 3]],
    radius: i32,
) -> Vec<[i32; 3]> {
    cells_of(origin, dimensions)
        .filter(|cell| changed.iter().any(|&c| chebyshev(*cell, c) <= radius))
        .collect()
}

fn exposed_slot(world: &World, mesh: Option<&MeshProxy>, cell: [i32; 3], face: usize) -> bool {
    let normal = FACE_NORMALS[face];
    let neighbor = [
        cell[0] + normal[0],
        cell[1] + normal[1],
        cell[2] + normal[2],
    ];
    scene_material(world, mesh, cell) != 0 && scene_material(world, mesh, neighbor) == 0
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct BoundCounts {
    cells: usize,
    slots: usize,
    /// Slots exposed in either placement: every slot whose stored value must be
    /// rewritten, either recomputed or zeroed.
    touched_faces: usize,
    /// Slots exposed in the corrected (after) placement: only these trace rays.
    ray_faces: usize,
    /// Ceiling if every dirty slot were sampled: `slots * SAMPLES * 2` rays.
    rays_all_slots: usize,
    /// Ceiling for the shipped loop, which only traces rays for exposed slots.
    rays_exposed: usize,
    work_all_slots: usize,
    /// The shipped loop's work model: `SAMPLES` per exposed slot, one inspection
    /// for every other slot.
    work_exposed: usize,
    frames_rays_all: usize,
    frames_rays_exposed: usize,
    frames_work_all: usize,
    frames_work_exposed: usize,
}

fn bound_counts(
    world: &World,
    before: Option<&MeshProxy>,
    after: Option<&MeshProxy>,
    cells: &[[i32; 3]],
) -> BoundCounts {
    let mut touched_faces = 0;
    let mut ray_faces = 0;
    for &cell in cells {
        for face in 0..6 {
            if exposed_slot(world, before, cell, face) || exposed_slot(world, after, cell, face) {
                touched_faces += 1;
            }
            if exposed_slot(world, after, cell, face) {
                ray_faces += 1;
            }
        }
    }
    let slots = cells.len() * 6;
    let samples = SAMPLES as usize;
    let rays_all_slots = slots * samples * 2;
    let rays_exposed = ray_faces * samples * 2;
    let work_all_slots = slots * samples;
    let work_exposed = ray_faces * samples + (slots - ray_faces);
    BoundCounts {
        cells: cells.len(),
        slots,
        touched_faces,
        ray_faces,
        rays_all_slots,
        rays_exposed,
        work_all_slots,
        work_exposed,
        frames_rays_all: rays_all_slots.div_ceil(UPDATE_BUDGET.rays),
        frames_rays_exposed: rays_exposed.div_ceil(UPDATE_BUDGET.rays),
        frames_work_all: work_all_slots.div_ceil(UPDATE_BUDGET.work),
        frames_work_exposed: work_exposed.div_ceil(UPDATE_BUDGET.work),
    }
}

struct Fresh {
    volume: IndirectVolume,
    frames: usize,
    rays: usize,
    work: usize,
}

/// Compute one volume from its first face to `complete()`, one wetland budget
/// slice per iteration, counting real slices, rays and work units.
fn fresh_volume(
    origin: [i32; 3],
    dimensions: [u32; 3],
    radius_m: f32,
    sun: Sun,
    world: &World,
    proxy: MeshProxy,
    label: &str,
) -> Fresh {
    let mut volume = IndirectVolume::new(origin, dimensions, SAMPLES, radius_m, probe_palette())
        .unwrap_or_else(|error| panic!("{label} volume: {error}"));
    volume.set_mesh_proxy(Some(proxy));
    let mut fresh = Fresh {
        volume,
        frames: 0,
        rays: 0,
        work: 0,
    };
    loop {
        let stats = fresh
            .volume
            .update(world, 0, sun, UPDATE_BUDGET)
            .unwrap_or_else(|error| panic!("{label} update: {error}"));
        fresh.frames += 1;
        fresh.rays += stats.rays;
        fresh.work += stats.work;
        if stats.complete {
            return fresh;
        }
        assert!(fresh.frames < 100_000, "{label} never completed");
    }
}

/// Face slots whose published value differs between two completed volumes.
fn changed_faces(
    before: &IndirectVolume,
    after: &IndirectVolume,
    origin: [i32; 3],
    dimensions: [u32; 3],
) -> Vec<([i32; 3], usize)> {
    let mut changed = Vec::new();
    for cell in cells_of(origin, dimensions) {
        for face in 0..6 {
            if before.sample(cell, face) != after.sample(cell, face) {
                changed.push((cell, face));
            }
        }
    }
    changed
}

fn outside_bound(bound: &[[i32; 3]], faces: &[([i32; 3], usize)]) -> Vec<([i32; 3], usize)> {
    faces
        .iter()
        .copied()
        .filter(|(cell, _)| !bound.contains(cell))
        .collect()
}

/// Distance from a face's sample origin to a target cell using the documented
/// `cell + 0.5 + n * (0.5 + eps)` origin with `eps` dropped. Dropping `eps`
/// moves the real origin 0.001 along the face normal, so this is accurate to
/// ±0.001 rather than a strict lower bound (it over-estimates for targets in
/// front of the face); the decisive inequality clears that margin by far.
fn face_origin_to_cell_min_distance(cell: [i32; 3], face: usize, target: [i32; 3]) -> f32 {
    let normal = FACE_NORMALS[face];
    let origin: [f32; 3] =
        std::array::from_fn(|axis| cell[axis] as f32 + 0.5 + normal[axis] as f32 * 0.5);
    (0..3)
        .map(|axis| {
            let low = target[axis] as f32;
            let high = low + 1.0;
            if origin[axis] < low {
                low - origin[axis]
            } else if origin[axis] > high {
                origin[axis] - high
            } else {
                0.0
            }
        })
        .map(|gap| gap * gap)
        .sum::<f32>()
        .sqrt()
}

fn report_bounds(tag: &str, counts: &BoundCounts) -> String {
    format!(
        "{tag} cells={} slots={} touched={} rays_for={} rays_all={} frames_rays_all={} \
         rays_exposed={} frames_rays_exposed={} work_all={} frames_work_all={} work_exposed={} \
         frames_work_exposed={}",
        counts.cells,
        counts.slots,
        counts.touched_faces,
        counts.ray_faces,
        counts.rays_all_slots,
        counts.frames_rays_all,
        counts.rays_exposed,
        counts.frames_rays_exposed,
        counts.work_all_slots,
        counts.frames_work_all,
        counts.work_exposed,
        counts.frames_work_exposed,
    )
}

/// The whole scene/R sweep: geometry counts and the actual fresh-volume
/// difference with the same-radius bound check, at both body positions.
#[test]
fn feasibility_table_and_actual_differences_are_conservative() {
    let mut any_outside_r_only = 0;
    for scene in [
        Surroundings::Empty,
        Surroundings::Sparse,
        Surroundings::Dense,
    ] {
        let world = surroundings_world(scene);
        let whole_cells: Vec<[i32; 3]> = cells_of(BOX_ORIGIN, BOX_DIMENSIONS).collect();
        for (place, before_cell, after_cell) in BODY_MOVES {
            let before_proxy = body_proxy(BOX_ORIGIN, BOX_DIMENSIONS, before_cell);
            let after_proxy = body_proxy(BOX_ORIGIN, BOX_DIMENSIONS, after_cell);
            let changed = changed_cells(
                &world,
                Some(&before_proxy),
                Some(&after_proxy),
                BOX_ORIGIN,
                BOX_DIMENSIONS,
            );
            let (occupancy, material_only) =
                classify_changes(&world, Some(&before_proxy), Some(&after_proxy), &changed);
            // A one-cell body moving one cell changes exactly its vacated and
            // occupied cells; the fixtures keep the destination air.
            assert_eq!(changed.len(), 2, "{scene:?} {place}: changed cells");
            assert_eq!(
                (occupancy, material_only),
                (2, 0),
                "{scene:?} {place}: change kinds"
            );

            let whole = bound_counts(
                &world,
                Some(&before_proxy),
                Some(&after_proxy),
                &whole_cells,
            );
            assert_eq!(whole.cells, 4000, "the wetland box is 20 x 10 x 20");
            assert_eq!(whole.slots, 24_000, "six face slots per cell");
            println!(
                "[probe] === scene={scene:?} place={place} changed={} occupancy={occupancy} \
                 material_only={material_only} ===\n\
                 [probe] {scene:?} {place} whole-volume restart: {}",
                changed.len(),
                report_bounds("whole", &whole),
            );

            let mut previous: Option<BoundCounts> = None;
            for radius_m in GATHER_RADII {
                let r_only_cells = bound_cells(
                    BOX_ORIGIN,
                    BOX_DIMENSIONS,
                    &changed,
                    r_only_radius_cells(radius_m),
                );
                let conservative_cells = bound_cells(
                    BOX_ORIGIN,
                    BOX_DIMENSIONS,
                    &changed,
                    conservative_radius_cells(radius_m),
                );
                assert!(
                    r_only_cells
                        .iter()
                        .all(|cell| conservative_cells.contains(cell)),
                    "R={radius_m}: the conservative bound must contain the R-only bound"
                );
                let r_only = bound_counts(
                    &world,
                    Some(&before_proxy),
                    Some(&after_proxy),
                    &r_only_cells,
                );
                let conservative = bound_counts(
                    &world,
                    Some(&before_proxy),
                    Some(&after_proxy),
                    &conservative_cells,
                );
                assert_eq!(conservative.slots, conservative.cells * 6);
                assert_eq!(
                    conservative.rays_all_slots,
                    conservative.slots * SAMPLES as usize * 2
                );
                assert_eq!(
                    conservative.rays_exposed,
                    conservative.ray_faces * SAMPLES as usize * 2
                );
                assert_eq!(
                    conservative.work_exposed,
                    conservative.ray_faces * SAMPLES as usize
                        + (conservative.slots - conservative.ray_faces)
                );
                assert_eq!(
                    conservative.frames_rays_exposed,
                    conservative.rays_exposed.div_ceil(UPDATE_BUDGET.rays)
                );
                assert_eq!(
                    conservative.frames_work_exposed,
                    conservative.work_exposed.div_ceil(UPDATE_BUDGET.work)
                );
                assert!(
                    conservative.touched_faces >= r_only.touched_faces
                        && conservative.ray_faces >= r_only.ray_faces
                        && conservative.cells >= r_only.cells,
                    "R={radius_m}: the two-segment bound must be at least the R-only bound"
                );
                if let Some(previous) = previous {
                    assert!(
                        conservative.cells >= previous.cells
                            && conservative.touched_faces >= previous.touched_faces
                            && conservative.ray_faces >= previous.ray_faces,
                        "the conservative bound must not shrink as R grows"
                    );
                }
                previous = Some(conservative);
                if radius_m == 24.0 {
                    // 2 * 24 + 1 exceeds every box axis, so the whole box is dirty.
                    assert_eq!(
                        conservative.cells, whole.cells,
                        "R=24: the conservative bound covers the whole box"
                    );
                    assert_eq!(conservative.touched_faces, whole.touched_faces);
                    assert_eq!(conservative.ray_faces, whole.ray_faces);
                }
                println!(
                    "[probe] {scene:?} {place} R={radius_m} dR={} d2R={} | {} | {}",
                    r_only_radius_cells(radius_m),
                    conservative_radius_cells(radius_m),
                    report_bounds("r-only", &r_only),
                    report_bounds("conservative", &conservative),
                );

                // Actual fresh volumes at this radius, compared with the bound.
                let before = fresh_volume(
                    BOX_ORIGIN,
                    BOX_DIMENSIONS,
                    radius_m,
                    PROBE_SUN,
                    &world,
                    body_proxy(BOX_ORIGIN, BOX_DIMENSIONS, before_cell),
                    "before",
                );
                let after = fresh_volume(
                    BOX_ORIGIN,
                    BOX_DIMENSIONS,
                    radius_m,
                    PROBE_SUN,
                    &world,
                    body_proxy(BOX_ORIGIN, BOX_DIMENSIONS, after_cell),
                    "after",
                );
                let actual =
                    changed_faces(&before.volume, &after.volume, BOX_ORIGIN, BOX_DIMENSIONS);
                let omissions = outside_bound(&conservative_cells, &actual);
                assert!(
                    omissions.is_empty(),
                    "{scene:?} {place} R={radius_m}: the conservative bound missed changed \
                     faces {omissions:?}"
                );
                let missed = outside_bound(&r_only_cells, &actual);
                any_outside_r_only += missed.len();
                for &(cell, face) in &actual {
                    assert!(
                        exposed_slot(&world, Some(&before_proxy), cell, face)
                            || exposed_slot(&world, Some(&after_proxy), cell, face),
                        "{scene:?} {place} R={radius_m}: {cell:?} face {face} changed while not \
                         exposed in either placement"
                    );
                }
                if scene == Surroundings::Empty {
                    assert!(
                        actual.is_empty(),
                        "{scene:?}: a body alone in a void carries no radiance to change"
                    );
                }
                println!(
                    "[probe] {scene:?} {place} R={radius_m} actual changed_faces={} \
                     outside_conservative={} outside_r_only={} | volume before frames={} rays={} \
                     work={} | volume after frames={} rays={} work={}",
                    actual.len(),
                    omissions.len(),
                    missed.len(),
                    before.frames,
                    before.rays,
                    before.work,
                    after.frames,
                    after.rays,
                    after.work,
                );
            }
        }
    }
    assert!(
        any_outside_r_only > 0,
        "the fixtures must exercise at least one changed face outside the R-only bound"
    );
}

/// Counterexample box: floor at `y = 0`, a one-cell receiver at
/// `[4, 1, 8]`, and the moving one-cell body.
const CX_ORIGIN: [i32; 3] = [0, 0, 0];
const CX_DIMENSIONS: [u32; 3] = [10, 5, 20];
const CX_RADIUS_M: f32 = 4.0;
const CX_RECEIVER_CELL: [i32; 3] = [4, 1, 8];
/// `+Z`, the receiver face that looks down the sun's horizontal direction.
const CX_RECEIVER_FACE: usize = 4;
const CX_BODY_BEFORE: [i32; 3] = [3, 1, 14];
const CX_BODY_AFTER: [i32; 3] = [3, 1, 13];
const CX_SUN: Sun = Sun {
    direction_to_sun: [0.0, 1.0, 5.0],
    intensity: 1.0,
};
/// Same geometry, sun horizontal component reversed: the hit-to-sun segment now
/// leaves the occluder behind, so the body move must not change the receiver.
const CX_SUN_REVERSED: Sun = Sun {
    direction_to_sun: [0.0, 1.0, -5.0],
    intensity: 1.0,
};

fn counterexample_world() -> World {
    let mut world = World::new(47);
    for cell in cells_of(CX_ORIGIN, CX_DIMENSIONS) {
        if cell[1] == 0 {
            world.set(cell, FLOOR_MATERIAL);
        }
    }
    world.set(CX_RECEIVER_CELL, ROCK_MATERIAL);
    world
}

/// The rejected `R`-only expansion misses a value its owner would publish as
/// current. The receiver's gather ray stops on the floor about 1.6 m away; the
/// body cell that blocks the sun ray from that hit point is within `R` of the
/// hit but more than `R` away from the receiver in cell distance and in
/// Euclidean distance from the receiver's sample origin, so only the second
/// traced segment can see it.
#[test]
fn r_only_expansion_misses_a_sun_occluder_that_the_two_r_bound_covers() {
    let world = counterexample_world();
    let before_proxy = body_proxy(CX_ORIGIN, CX_DIMENSIONS, CX_BODY_BEFORE);
    let after_proxy = body_proxy(CX_ORIGIN, CX_DIMENSIONS, CX_BODY_AFTER);
    let changed = changed_cells(
        &world,
        Some(&before_proxy),
        Some(&after_proxy),
        CX_ORIGIN,
        CX_DIMENSIONS,
    );
    assert_eq!(changed.len(), 2, "a one-cell move changes two cells");
    let r_only_cells = bound_cells(
        CX_ORIGIN,
        CX_DIMENSIONS,
        &changed,
        r_only_radius_cells(CX_RADIUS_M),
    );
    let conservative_cells = bound_cells(
        CX_ORIGIN,
        CX_DIMENSIONS,
        &changed,
        conservative_radius_cells(CX_RADIUS_M),
    );
    assert!(
        !r_only_cells.contains(&CX_RECEIVER_CELL),
        "fixture: the receiver must be outside the R-only dirty set"
    );
    assert!(
        conservative_cells.contains(&CX_RECEIVER_CELL),
        "fixture: the receiver must be inside the conservative dirty set"
    );
    for body in [CX_BODY_BEFORE, CX_BODY_AFTER] {
        let distance = face_origin_to_cell_min_distance(CX_RECEIVER_CELL, CX_RECEIVER_FACE, body);
        assert!(
            distance > CX_RADIUS_M,
            "fixture: no gather ray of R={CX_RADIUS_M} can reach {body:?} from the receiver \
             (lower-bound distance {distance})"
        );
    }

    let before = fresh_volume(
        CX_ORIGIN,
        CX_DIMENSIONS,
        CX_RADIUS_M,
        CX_SUN,
        &world,
        before_proxy,
        "counterexample before",
    );
    let after = fresh_volume(
        CX_ORIGIN,
        CX_DIMENSIONS,
        CX_RADIUS_M,
        CX_SUN,
        &world,
        after_proxy,
        "counterexample after",
    );
    let lit = before.volume.sample(CX_RECEIVER_CELL, CX_RECEIVER_FACE);
    let occluded = after.volume.sample(CX_RECEIVER_CELL, CX_RECEIVER_FACE);
    assert!(
        lit[0] > occluded[0],
        "the body one cell closer must block the hit-to-sun ray: before {lit:?} after {occluded:?}"
    );

    let actual = changed_faces(&before.volume, &after.volume, CX_ORIGIN, CX_DIMENSIONS);
    let omissions = outside_bound(&conservative_cells, &actual);
    let missed = outside_bound(&r_only_cells, &actual);
    assert!(
        omissions.is_empty(),
        "the conservative bound missed changed faces {omissions:?}"
    );
    assert!(
        !missed.is_empty(),
        "fixture: the R-only bound must miss at least one changed face"
    );

    // Control: reverse the sun's horizontal component. The gather geometry is
    // unchanged and no gather ray reaches either body cell, so the only
    // possible difference is the sun segment; leaving the occluder behind must
    // leave the receiver unchanged.
    let reversed_before = fresh_volume(
        CX_ORIGIN,
        CX_DIMENSIONS,
        CX_RADIUS_M,
        CX_SUN_REVERSED,
        &world,
        body_proxy(CX_ORIGIN, CX_DIMENSIONS, CX_BODY_BEFORE),
        "counterexample reversed before",
    );
    let reversed_after = fresh_volume(
        CX_ORIGIN,
        CX_DIMENSIONS,
        CX_RADIUS_M,
        CX_SUN_REVERSED,
        &world,
        body_proxy(CX_ORIGIN, CX_DIMENSIONS, CX_BODY_AFTER),
        "counterexample reversed after",
    );
    let reversed_lit = reversed_before
        .volume
        .sample(CX_RECEIVER_CELL, CX_RECEIVER_FACE);
    let reversed_occluded = reversed_after
        .volume
        .sample(CX_RECEIVER_CELL, CX_RECEIVER_FACE);
    assert_eq!(
        reversed_lit, reversed_occluded,
        "reversing the sun must remove the change, so the change is sun visibility"
    );

    println!(
        "[probe] counterexample R={CX_RADIUS_M} receiver={CX_RECEIVER_CELL:?} face=+Z \
         body_before={CX_BODY_BEFORE:?} body_after={CX_BODY_AFTER:?}\n\
         [probe] counterexample r_only_radius={} conservative_radius={} receiver_chebyshev_before={} \
         receiver_chebyshev_after={} origin_distance_before={:.3} origin_distance_after={:.3}\n\
         [probe] counterexample value before={lit:?} after={occluded:?} \
         delta={:.6} reversed before={reversed_lit:?} reversed after={reversed_occluded:?}\n\
         [probe] counterexample changed_faces={} outside_conservative={} outside_r_only={:?} \
         (count {})",
        r_only_radius_cells(CX_RADIUS_M),
        conservative_radius_cells(CX_RADIUS_M),
        chebyshev(CX_RECEIVER_CELL, CX_BODY_BEFORE),
        chebyshev(CX_RECEIVER_CELL, CX_BODY_AFTER),
        face_origin_to_cell_min_distance(CX_RECEIVER_CELL, CX_RECEIVER_FACE, CX_BODY_BEFORE),
        face_origin_to_cell_min_distance(CX_RECEIVER_CELL, CX_RECEIVER_FACE, CX_BODY_AFTER),
        lit[0] - occluded[0],
        actual.len(),
        omissions.len(),
        missed,
        missed.len(),
    );
}

// ---------------------------------------------------------------------------
// Continuous-motion fixture (issue 47)
// ---------------------------------------------------------------------------

/// Production-shaped continuous-motion fixture. The wetland's authored clearing
/// sits at `[0.125, 0.125, 0.0]` m, so `box_origin` rounds it to this origin
/// (`apps/explorer/src/wetland.rs`) and the production coverage box is
/// 20 x 10 x 20 cells with the ground plane at cell level `y = 0` and five
/// metres of air below it.
const MOTION_ORIGIN: [i32; 3] = [-10, -5, -10];
const MOTION_DIMENSIONS: [u32; 3] = [20, 10, 20];
const MOTION_GROUND_Y: i32 = 0;
const MOTION_BODY_Y: i32 = 1;
const MOTION_Z: i32 = -6;
/// The body path bounces along +X between these two cells, so every frame of a
/// sustained run moves a body cell while staying inside the coverage box.
const MOTION_PATH_START_X: i32 = -8;
const MOTION_PATH_SPAN: i32 = 15;
/// The three suns the production wetland cycles through (`Action::Sun` in
/// `apps/explorer/src/lib.rs`), intensity included: the low-angle sun is the one
/// whose shadow segment reaches farthest, and the overhead sun is the one whose
/// shadow segment reaches least.
const MOTION_SUNS: [(&str, Sun); 3] = [
    (
        "afternoon",
        Sun {
            direction_to_sun: [0.4, 0.85, 0.3],
            intensity: 0.8,
        },
    ),
    (
        "low",
        Sun {
            direction_to_sun: [-0.8, 0.35, 0.3],
            intensity: 0.8,
        },
    ),
    (
        "overhead",
        Sun {
            direction_to_sun: [0.2, 1.0, -0.5],
            intensity: 0.8,
        },
    ),
];
/// The wetland app's own dependency cap.
const MOTION_RETENTION_BYTES: usize = 6 * 1024 * 1024;
const GROUND_MATERIAL: u8 = 13;

/// Authored props on the ground plate: single cells and a two-cell pillar, the
/// debris the clearing carries. None of them is on the body path.
const MOTION_PROPS: [[i32; 3]; 9] = [
    [-7, 1, -2],
    [-3, 1, -9],
    [0, 1, -4],
    [4, 1, -8],
    [6, 1, -3],
    [-5, 1, 3],
    [2, 1, 5],
    [-1, 1, -7],
    [-1, 2, -7],
];

/// One proxy with one prototype per material.
fn proxy_of(cells: &[([i32; 3], u8)], origin: [i32; 3], dimensions: [u32; 3]) -> MeshProxy {
    let mut meshes = Vec::new();
    let mut materials = Vec::new();
    let mut instances = Vec::new();
    for &(cell, material) in cells {
        let prototype = match materials.iter().position(|&known| known == material) {
            Some(index) => index,
            None => {
                meshes.push(unit_cube());
                materials.push(material);
                meshes.len() - 1
            }
        };
        instances.push(StaticInstance {
            prototype,
            translation: cell.map(|value| value as f32),
            yaw_quarters: 0,
        });
    }
    MeshProxy::build(
        &MeshGeometry {
            meshes: &meshes,
            instances: &instances,
            materials: &materials,
        },
        origin,
        dimensions,
    )
    .expect("motion proxy build")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MotionScene {
    /// The clearing: ground plate plus the authored props above.
    Clearing,
    /// The same ground with a two-cell pillar field every third cell, so most
    /// gather and shadow segments end on geometry instead of on the sky.
    Debris,
}

fn motion_scene_cells(scene: MotionScene) -> Vec<([i32; 3], u8)> {
    let mut cells: Vec<([i32; 3], u8)> = cells_of(MOTION_ORIGIN, MOTION_DIMENSIONS)
        .filter(|cell| cell[1] == MOTION_GROUND_Y)
        .map(|cell| (cell, GROUND_MATERIAL))
        .collect();
    match scene {
        MotionScene::Clearing => {
            for cell in MOTION_PROPS {
                cells.push((cell, ROCK_MATERIAL));
            }
        }
        MotionScene::Debris => {
            for x in (MOTION_ORIGIN[0]..MOTION_ORIGIN[0] + MOTION_DIMENSIONS[0] as i32).step_by(3) {
                for z in
                    (MOTION_ORIGIN[2]..MOTION_ORIGIN[2] + MOTION_DIMENSIONS[2] as i32).step_by(3)
                {
                    // Keep the body path's own row clear.
                    if z == MOTION_Z || (x - MOTION_ORIGIN[0]) % 3 == 2 {
                        continue;
                    }
                    cells.push(([x, MOTION_BODY_Y, z], ROCK_MATERIAL));
                    cells.push(([x, MOTION_BODY_Y + 1, z], ROCK_MATERIAL));
                }
            }
        }
    }
    cells
}

/// How many cells one physics body occupies. A body resting on the clearing's
/// ground spans one cell in `y`; the question is how many cells its footprint
/// grazes horizontally, because a body straddling cell boundaries changes twice
/// as many cells per step as an aligned one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MotionBody {
    /// One aligned cell (the app's half-metre pebble at a cell-centred
    /// translation): one step changes the vacated and the occupied cell.
    Aligned,
    /// A 2 x 2 footprint (the same pebble at a boundary-straddling translation):
    /// one step changes four cells, the largest footprint a body resting on the
    /// ground can have.
    Straddle,
}

impl MotionBody {
    fn offsets(self) -> &'static [[i32; 3]] {
        match self {
            Self::Aligned => &[[0, 0, 0]],
            Self::Straddle => &[[0, 0, 0], [1, 0, 0], [0, 0, 1], [1, 0, 1]],
        }
    }

    fn cells(self, body: [i32; 3]) -> impl Iterator<Item = [i32; 3]> {
        self.offsets().iter().map(move |offset| {
            [
                body[0] + offset[0],
                body[1] + offset[1],
                body[2] + offset[2],
            ]
        })
    }
}

fn motion_proxy(scene: MotionScene, body: MotionBody, at: [i32; 3]) -> MeshProxy {
    let mut cells = motion_scene_cells(scene);
    for cell in body.cells(at) {
        cells.push((cell, BODY_MATERIAL));
    }
    proxy_of(&cells, MOTION_ORIGIN, MOTION_DIMENSIONS)
}

/// Triangular path: `span` cells out, `span` cells back, inside the box.
fn motion_body(step: i32) -> [i32; 3] {
    let period = 2 * MOTION_PATH_SPAN;
    let phase = step.rem_euclid(period);
    let x = if phase <= MOTION_PATH_SPAN {
        MOTION_PATH_START_X + phase
    } else {
        MOTION_PATH_START_X + period - phase
    };
    [x, MOTION_BODY_Y, MOTION_Z]
}

/// One frame of a continuous-motion path, in engine work units.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct MotionFrame {
    step: i32,
    /// Whether the volume is complete and current for this frame's proxy, i.e.
    /// whether the production scheduler could publish it in this frame.
    live: bool,
    retained: usize,
    invalidated: usize,
    dirty: usize,
    pending: usize,
    rays: usize,
    work: usize,
    /// Slots that differed from a fresh recomputation for this frame's proxy,
    /// checked only on checkpoint frames.
    stale_slots: usize,
}

/// One continuous-motion run: the per-frame frames, the tracker's own cost, and
/// what the fresh-reference checkpoints found.
struct MotionReport {
    frames: Vec<MotionFrame>,
    /// Frames at which a fresh volume was recomputed and compared.
    checked_frames: usize,
    /// Face slots that differed from the fresh reference, summed over the
    /// checkpoints. Nonzero means a stale or partial publication was reachable.
    stale_slots: usize,
    tracked_faces: usize,
    untracked_faces: usize,
    dependency_bytes: usize,
}

impl MotionReport {
    fn live_fraction(&self) -> f64 {
        self.frames.iter().filter(|frame| frame.live).count() as f64 / self.frames.len() as f64
    }

    fn longest_dark_run(&self) -> usize {
        let mut longest = 0;
        let mut dark = 0;
        for frame in &self.frames {
            dark = if frame.live { 0 } else { dark + 1 };
            longest = longest.max(dark);
        }
        longest
    }

    fn max_rays(&self) -> usize {
        self.frames
            .iter()
            .map(|frame| frame.rays)
            .max()
            .unwrap_or(0)
    }

    fn max_work(&self) -> usize {
        self.frames
            .iter()
            .map(|frame| frame.work)
            .max()
            .unwrap_or(0)
    }

    fn max_invalidated(&self) -> usize {
        self.frames
            .iter()
            .map(|frame| frame.invalidated)
            .max()
            .unwrap_or(0)
    }

    fn sum(&self, pick: fn(&MotionFrame) -> usize) -> usize {
        self.frames.iter().map(pick).sum()
    }
}

/// Drive a body `speed` cells per frame along the motion path for `moves` body
/// cells, one production budget slice per frame, through the shipped retention
/// path: `replace_mesh_proxy` for the new footprint, then `update`.
///
/// `check_every > 0` compares the live volume bit-for-bit against a freshly
/// recomputed volume for the current proxy every that many frames.
fn run_motion_path(
    scene: MotionScene,
    body: MotionBody,
    sun: Sun,
    speed: i32,
    moves: i32,
    check_every: i32,
) -> MotionReport {
    let world = World::new(47);
    let mut volume = IndirectVolume::new(
        MOTION_ORIGIN,
        MOTION_DIMENSIONS,
        SAMPLES,
        GATHER_RADII[4],
        probe_palette(),
    )
    .expect("motion volume");
    volume
        .enable_proxy_retention(MOTION_RETENTION_BYTES)
        .expect("motion retention");
    volume.set_mesh_proxy(Some(motion_proxy(scene, body, motion_body(0))));
    loop {
        let stats = volume
            .update(&world, 0, sun, UPDATE_BUDGET)
            .expect("motion warmup");
        if stats.complete {
            break;
        }
    }
    let mut report = MotionReport {
        frames: Vec::new(),
        checked_frames: 0,
        stale_slots: 0,
        tracked_faces: 0,
        untracked_faces: 0,
        dependency_bytes: 0,
    };
    let mut step = 0;
    let mut index = 0;
    while step < moves {
        step += speed;
        let frame_index = index;
        index += 1;
        let at = motion_body(step);
        let edit = volume.replace_mesh_proxy(Some(motion_proxy(scene, body, at)));
        let stats = volume
            .update(&world, 0, sun, UPDATE_BUDGET)
            .expect("motion update");
        let live = volume.valid_for(&world, 0, sun);
        let mut frame = MotionFrame {
            step,
            live,
            retained: edit.retained_faces,
            invalidated: edit.invalidated_faces,
            dirty: volume.dirty_faces(),
            pending: volume.pending_work(),
            rays: stats.rays,
            work: stats.work,
            stale_slots: 0,
        };
        if check_every > 0 && frame_index % check_every == 0 && live {
            let fresh = fresh_volume(
                MOTION_ORIGIN,
                MOTION_DIMENSIONS,
                GATHER_RADII[4],
                sun,
                &world,
                motion_proxy(scene, body, at),
                "motion reference",
            );
            report.checked_frames += 1;
            for cell in cells_of(MOTION_ORIGIN, MOTION_DIMENSIONS) {
                for face in 0..6 {
                    if volume.sample(cell, face) != fresh.volume.sample(cell, face) {
                        frame.stale_slots += 1;
                    }
                }
            }
            report.stale_slots += frame.stale_slots;
        }
        report.frames.push(frame);
    }
    if let Some(status) = volume.retention_status() {
        report.tracked_faces = status.tracked_faces;
        report.untracked_faces = status.untracked_faces;
        report.dependency_bytes = status.resident_bytes;
    }
    report
}

fn report_motion(label: &str, report: &MotionReport, verbose: bool) {
    println!(
        "[probe] motion {label}: frames={} live={} live_fraction={:.3} longest_dark_run={} \
         retained={} invalidated={} max_invalidated={} dirty_end={} rays={} work={} max_rays={} \
         max_work={} checkpoints={} stale_slots={} tracked={} untracked={} dependency_kib={}",
        report.frames.len(),
        report.frames.iter().filter(|frame| frame.live).count(),
        report.live_fraction(),
        report.longest_dark_run(),
        report.sum(|frame| frame.retained),
        report.sum(|frame| frame.invalidated),
        report.max_invalidated(),
        report.frames.last().map_or(0, |frame| frame.dirty),
        report.sum(|frame| frame.rays),
        report.sum(|frame| frame.work),
        report.max_rays(),
        report.max_work(),
        report.checked_frames,
        report.stale_slots,
        report.tracked_faces,
        report.untracked_faces,
        report.dependency_bytes / 1024,
    );
    if verbose {
        for frame in &report.frames {
            println!(
                "[probe] motion {label} step={} live={} retained={} invalidated={} dirty={} \
                 pending={} rays={} work={} stale_slots={}",
                frame.step,
                frame.live,
                frame.retained,
                frame.invalidated,
                frame.dirty,
                frame.pending,
                frame.rays,
                frame.work,
                frame.stale_slots,
            );
        }
    }
}

/// Continuous motion at several rates, over both scene shapes and all three
/// production suns. Every frame runs exactly one production budget slice, and
/// every live checkpoint volume is compared bit-for-bit against a fresh recompute
/// for its own proxy: a stale or partial publication would show up as a nonzero
/// `stale_slots`.
#[test]
fn continuous_motion_live_fraction_is_measured() {
    // One cell per frame is the sustained body motion the open requirement names
    // (a walking or rolling body changes its footprint every frame or two). The
    // higher rates are probes: they give the body fewer frames per cell, so they
    // change the same number of cells per step and are not a larger invalidated
    // set - the boundary probe for a larger set is the boundary-straddling
    // footprint, which changes four cells per step instead of two.
    let mut summary = Vec::new();
    // Every configuration is measured and printed before anything is asserted,
    // so a run against a build whose retention did not hold still reports the
    // whole table rather than only the first failing line.
    let mut reports = Vec::new();
    let mut worst_rays = 0usize;
    let mut worst_label = String::new();
    for (sun_label, sun) in MOTION_SUNS {
        for scene in [MotionScene::Clearing, MotionScene::Debris] {
            for (body, rates) in [
                (MotionBody::Aligned, &[1, 2, 4, 8, 16][..]),
                (MotionBody::Straddle, &[1, 2, 4][..]),
            ] {
                for speed in rates {
                    // The two healing configurations the pre-change code never
                    // kept live (a low sun over a pillar field) are checked on
                    // every single frame; the rest on frames spread over the
                    // path. The moving volume is live on every frame, so a
                    // checkpoint is skipped only for a frame that stayed dark.
                    let dense = sun_label == "low"
                        && scene == MotionScene::Debris
                        && body == MotionBody::Aligned;
                    let frames_total = 30 / *speed;
                    let check_every = if dense { 1 } else { (frames_total / 2).max(1) };
                    let report = run_motion_path(scene, body, sun, *speed, 30, check_every);
                    let label = format!("{sun_label}/{scene:?}/{body:?}/{speed}cells");
                    report_motion(&label, &report, false);
                    if report.max_rays() > worst_rays {
                        worst_rays = report.max_rays();
                        worst_label = label.clone();
                    }
                    summary.push((
                        label,
                        report.live_fraction(),
                        report.max_rays(),
                        report.max_work(),
                        report.max_invalidated(),
                    ));
                    reports.push(report);
                }
            }
        }
    }
    for (label, fraction, max_rays, max_work, max_invalidated) in &summary {
        println!(
            "[probe] motion summary {label} live_fraction={fraction:.3} max_rays={max_rays} \
             max_work={max_work} max_invalidated={max_invalidated}"
        );
    }
    println!(
        "[probe] motion worst case: {worst_label} max_rays={worst_rays} of {} budget rays",
        UPDATE_BUDGET.rays
    );
    for (index, (label, _, _, _, _)) in summary.iter().enumerate() {
        let report = &reports[index];
        assert!(
            report.stale_slots == 0,
            "{label}: a published volume differed from the fresh reference at {} of {} \
             checkpoints",
            report.stale_slots,
            report.checked_frames
        );
        assert!(
            report.checked_frames > 0,
            "{label}: no live checkpoint was checked"
        );
        assert_eq!(
            report.longest_dark_run(),
            0,
            "{label}: indirect lighting went dark for {} frames",
            report.longest_dark_run()
        );
        assert_eq!(
            report.live_fraction(),
            1.0,
            "{label}: {} of {} frames stayed live",
            report.frames.iter().filter(|frame| frame.live).count(),
            report.frames.len()
        );
    }
    assert!(
        worst_rays <= UPDATE_BUDGET.rays,
        "the worst measured frame exceeded the production ray budget"
    );
}
