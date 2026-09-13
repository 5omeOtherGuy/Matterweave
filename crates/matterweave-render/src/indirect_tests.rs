use crate::indirect::{IndirectVolume, UpdateBudget, MAX_FACE_SLOTS};
use crate::Sun;
use matterweave_core::World;

fn sun() -> Sun {
    Sun {
        direction_to_sun: [0., 1., 0.],
        intensity: 1.,
    }
}
fn volume() -> IndirectVolume {
    let mut palette = [[0.5; 3]; 256];
    palette[1] = [0.9, 0.05, 0.02];
    IndirectVolume::new([-3, -1, -3], [7, 5, 7], 64, 16., palette).unwrap()
}
fn room(closed: bool) -> World {
    let mut w = World::new(7);
    for x in -3..=3 {
        for z in -3..=3 {
            w.set([x, -1, z], 1);
            if closed {
                w.set([x, 3, z], 2);
            }
            for y in 0..3 {
                if x.abs() == 3 || z.abs() == 3 {
                    w.set([x, y, z], 2);
                }
            }
        }
    }
    w
}
fn finish(v: &mut IndirectVolume, w: &World, epoch: u64, light: Sun) {
    for _ in 0..1000 {
        let s = v
            .update(
                w,
                epoch,
                light,
                UpdateBudget {
                    rays: 4096,
                    work: 4096,
                },
            )
            .unwrap();
        if s.complete {
            return;
        }
    }
    panic!("bounded fixture never completed");
}
#[test]
fn enclosure_opening_and_closing_controls_colored_bounce() {
    let mut w = room(true);
    let mut v = volume();
    finish(&mut v, &w, 0, sun());
    assert_eq!(
        v.sample([-3, 1, 0], 0),
        [0.; 3],
        "sealed room has no illuminated bounce source"
    );
    for x in -3..=3 {
        for z in -3..=3 {
            w.set([x, 3, z], 0);
        }
    }
    finish(&mut v, &w, 0, sun());
    let rgb = v.sample([-3, 1, 0], 0);
    assert!(
        rgb[0] > 0.03,
        "sunlit red floor must illuminate inward wall: {rgb:?}"
    );
    assert!(
        rgb[0] > rgb[1] * 2.,
        "colored energy must transfer: {rgb:?}"
    );
    for x in -3..=3 {
        for z in -3..=3 {
            w.set([x, 3, z], 2);
        }
    }
    v.update(&w, 0, sun(), UpdateBudget { rays: 0, work: 0 })
        .unwrap();
    assert_eq!(
        v.sample([-3, 1, 0], 0),
        [0.; 3],
        "old open-room cache must disappear immediately"
    );
    finish(&mut v, &w, 0, sun());
    assert_eq!(v.sample([-3, 1, 0], 0), [0.; 3]);
}
#[test]
fn budget_and_light_or_source_invalidation_are_explicit() {
    let w = room(false);
    let mut v = volume();
    let s = v
        .update(&w, 0, sun(), UpdateBudget { rays: 3, work: 2 })
        .unwrap();
    assert!(s.rays <= 3 && s.work <= 2 && !s.complete);
    finish(&mut v, &w, 0, sun());
    assert!(v.sample([-3, 1, 0], 0)[0] > 0.);
    let dark = Sun {
        intensity: 0.,
        ..sun()
    };
    v.update(&w, 0, dark, UpdateBudget { rays: 0, work: 0 })
        .unwrap();
    assert_eq!(v.sample([-3, 1, 0], 0), [0.; 3]);
    assert!(!v.valid_for(&w, 0, sun()));
    finish(&mut v, &w, 0, dark);
    assert_eq!(v.sample([-3, 1, 0], 0), [0.; 3]);
    finish(&mut v, &w, 0, sun());
    assert!(
        !v.valid_for(&w, 1, sun()),
        "replacement source epoch rejects same revision"
    );
    let down = Sun {
        direction_to_sun: [0., -1., 0.],
        ..sun()
    };
    finish(&mut v, &w, 1, down);
    assert_eq!(v.sample([-3, 1, 0], 0), [0.; 3]);
}
#[test]
fn thin_wall_separates_lit_and_sealed_spaces() {
    let mut w = room(false);
    // One-voxel divider, roof only on the right: no interpolation across it.
    for z in -3..=3 {
        for y in 0..=3 {
            w.set([0, y, z], 2);
        }
        for x in 1..=3 {
            w.set([x, 3, z], 2);
        }
    }
    let mut v = volume();
    finish(&mut v, &w, 0, sun());
    assert!(v.sample([0, 1, 0], 1)[0] > 0.02);
    assert_eq!(v.sample([0, 1, 0], 0), [0.; 3]);
}
#[test]
fn caps_and_invalid_inputs_fail_before_allocation() {
    let p = [[0.5; 3]; 256];
    for dims in [[u32::MAX; 3], [0, 1, 1], [MAX_FACE_SLOTS as u32, 1, 1]] {
        assert!(IndirectVolume::new([0; 3], dims, 32, 16., p).is_err());
    }
    assert!(IndirectVolume::new([i32::MAX; 3], [1; 3], 32, 16., p).is_err());
    for range in [0., f32::NAN, f32::INFINITY, 4097.] {
        assert!(IndirectVolume::new([0; 3], [1; 3], 32, range, p).is_err());
    }
    let mut v = volume();
    let w = room(false);
    assert!(v
        .update(
            &w,
            0,
            Sun {
                direction_to_sun: [0.; 3],
                intensity: 1.
            },
            UpdateBudget { rays: 1, work: 1 }
        )
        .is_err());
}
#[test]
fn scheduling_is_deterministic_and_does_not_edit_authority() {
    let w = room(false);
    let revision = w.revision();
    let stats = w.stats();
    let mut a = volume();
    let mut b = volume();
    finish(&mut a, &w, 0, sun());
    for _ in 0..100000 {
        if b.update(&w, 0, sun(), UpdateBudget { rays: 18, work: 11 })
            .unwrap()
            .complete
        {
            break;
        }
    }
    assert_eq!(a.sample([-3, 1, 0], 0), b.sample([-3, 1, 0], 0));
    assert_eq!(w.revision(), revision);
    assert_eq!(w.stats(), stats);
}

// ===========================================================================
// D3.1 mesh-only participation probe.
//
// Declared criterion, asserted here rather than eyeballed. The fixture is one
// unit-cube prototype (the engine's own mesher output for one voxel, i.e. what a
// detail prototype pool holds) placed on a sunlit red floor by a
// `StaticInstance` record, in a volume whose footprint also contains a unit-voxel
// receiver and a second, identical mesh-only object used as a static control.
//
// C1 presence and direction: the object's exposed side faces report exactly zero
//    with no mesh geometry attached (air cells are never sampled) and a strictly
//    positive radiance afterwards that is dominated by the floor's albedo channel
//    (red), because those faces gather the sunlit floor.
// C2 trace participation: a unit-voxel receiver face whose gather reached the
//    sunlit floor loses red energy (the object now occludes part of that floor)
//    and gains the object's own green albedo, so the mesh geometry is a trace
//    target, not only a sampled surface.
// C3 movement and control: lifting the object strictly lowers every side-face
//    response, the vacated cell returns to exactly zero, and an identical static
//    object's samples stay bit-identical.
// ===========================================================================
use crate::indirect::{MeshGeometry, MeshProxy};
use crate::static_scene::StaticInstance;
use matterweave_core::{Mesh, Vertex};

const PROBE_FLOOR: u8 = 1;
const PROBE_WALL: u8 = 2;
const PROBE_OBJECT: u8 = 3;

fn probe_palette() -> [[f32; 3]; 256] {
    let mut palette = [[0.5; 3]; 256];
    palette[0] = [0.; 3];
    palette[PROBE_FLOOR as usize] = [0.9, 0.05, 0.02];
    palette[PROBE_WALL as usize] = [0.5; 3];
    palette[PROBE_OBJECT as usize] = [0.05, 0.8, 0.1];
    palette
}

/// One unit cube at `[0,1]^3`: exactly the geometry the engine's own voxel
/// mesher produces for a single cell, so this is a detail prototype, not a
/// synthetic triangle soup.
fn probe_prototype() -> Mesh {
    let mut voxel = World::new(0);
    voxel.set([0, 0, 0], 1);
    voxel.mesh()
}

fn probe_placement(cell: [i32; 3]) -> StaticInstance {
    StaticInstance {
        prototype: 0,
        translation: cell.map(|v| v as f32),
        yaw_quarters: 0,
    }
}

fn probe_volume() -> IndirectVolume {
    IndirectVolume::new([-5, -2, -5], [11, 6, 11], 64, 16., probe_palette()).unwrap()
}

/// Sunlit floor plus one unit-voxel receiver column at `x = -4`, `y in 0..=2`.
fn probe_world() -> World {
    let mut world = World::new(0);
    for x in -5..5 {
        for z in -5..5 {
            world.set([x, -1, z], PROBE_FLOOR);
        }
    }
    for y in 0..=2 {
        world.set([-4, y, 0], PROBE_WALL);
    }
    world
}

fn probe_proxy(placements: &[StaticInstance]) -> MeshProxy {
    let meshes = [probe_prototype()];
    let materials = [PROBE_OBJECT];
    MeshProxy::build(
        &MeshGeometry {
            meshes: &meshes,
            instances: placements,
            materials: &materials,
        },
        [-5, -2, -5],
        [11, 6, 11],
    )
    .unwrap()
}

const OBJECT_CELL: [i32; 3] = [0, 0, 0];
const LIFTED_CELL: [i32; 3] = [0, 2, 0];
const CONTROL_CELL: [i32; 3] = [-3, 0, -3];
const RECEIVER_CELL: [i32; 3] = [-4, 1, 0];
const RECEIVER_FACE: usize = 0;
/// Faces whose hemispheres contain the sunlit floor for a cube standing on it.
const SIDE_FACES: [usize; 4] = [0, 1, 4, 5];

#[test]
fn c1_mesh_only_geometry_receives_indirect_light() {
    let world = probe_world();
    let sun = sun();
    let mut before = probe_volume();
    finish(&mut before, &world, 0, sun);
    assert!(
        !before.has_mesh_proxy() && before.mesh_digest().is_none(),
        "a fresh volume represents unit voxels only"
    );
    for face in 0..6 {
        assert_eq!(
            before.sample(OBJECT_CELL, face),
            [0.; 3],
            "an air cell has no sampled face: face {face}"
        );
    }
    let mut after = probe_volume();
    after.set_mesh_proxy(Some(probe_proxy(&[probe_placement(OBJECT_CELL)])));
    assert!(after.has_mesh_proxy() && after.mesh_digest().is_some());
    finish(&mut after, &world, 0, sun);
    for face in SIDE_FACES {
        let (r, g, b) = {
            let v = after.sample(OBJECT_CELL, face);
            (v[0], v[1], v[2])
        };
        let before_face = before.sample(OBJECT_CELL, face);
        assert!(
            r > 0.05 && r > 10. * g && r > 10. * b,
            "after {face}: the object must gather the sunlit red floor, got {r} {g} {b}; \
             before {before_face:?}"
        );
    }
    // Faces pointing away from the floor stay dark: one bounce with black misses
    // cannot light the top of an object standing on the only bounce source.
    assert_eq!(after.sample(OBJECT_CELL, 2), [0.; 3], "+Y gathers upward");
    assert_eq!(
        after.sample(OBJECT_CELL, 3),
        [0.; 3],
        "-Y touches the floor"
    );
}

#[test]
fn c2_mesh_only_geometry_participates_in_the_gather() {
    let world = probe_world();
    let sun = sun();
    let mut before = probe_volume();
    before.set_mesh_proxy(Some(probe_proxy(&[])));
    finish(&mut before, &world, 0, sun);
    let mut after = probe_volume();
    after.set_mesh_proxy(Some(probe_proxy(&[probe_placement(OBJECT_CELL)])));
    finish(&mut after, &world, 0, sun);
    // Only the receiver's outward face can see the object: -X of the column at
    // `x = -4` gathers toward +X, and the object stands at `x in [0,1)`. Its -Z
    // face gathers away from the object and is deliberately not asserted.
    let face = RECEIVER_FACE;
    let (r0, g0) = {
        let v = before.sample(RECEIVER_CELL, face);
        (v[0], v[1])
    };
    let (r1, g1) = {
        let v = after.sample(RECEIVER_CELL, face);
        (v[0], v[1])
    };
    assert!(
        r1 < r0,
        "face {face}: the object occludes sunlit floor, so red must fall, {r1} vs {r0}"
    );
    assert!(
        g1 > g0 * 1.5,
        "face {face}: the object's albedo must enter the gather, {g1} vs {g0}"
    );
}

#[test]
fn c3_movement_changes_the_response_and_a_static_object_does_not_drift() {
    let world = probe_world();
    let sun = sun();
    let control = probe_placement(CONTROL_CELL);
    let mut rest = probe_volume();
    rest.set_mesh_proxy(Some(probe_proxy(&[probe_placement(OBJECT_CELL), control])));
    finish(&mut rest, &world, 0, sun);
    let mut lifted = probe_volume();
    lifted.set_mesh_proxy(Some(probe_proxy(&[probe_placement(LIFTED_CELL), control])));
    finish(&mut lifted, &world, 0, sun);
    assert_ne!(
        rest.mesh_digest(),
        lifted.mesh_digest(),
        "a moved object must change the representation identity"
    );
    for face in SIDE_FACES {
        let at_rest = rest.sample(OBJECT_CELL, face)[0];
        let raised = lifted.sample(LIFTED_CELL, face)[0];
        assert!(at_rest > 0.05, "face {face}: rest response {at_rest}");
        assert!(
            raised < at_rest,
            "face {face}: lifting away from the floor must lower the response, {raised} vs {at_rest}"
        );
    }
    for face in 0..6 {
        assert_eq!(
            lifted.sample(OBJECT_CELL, face),
            [0.; 3],
            "the vacated cell is air again: face {face}"
        );
    }
    // The control's -X face gathers toward -X while the moved object lies at
    // `x >= 0`, and the sun is vertical, so no ray of this face can reach the
    // object: its samples must hold exactly. Faces that do contain the object
    // legitimately change (its +X face moves 0.3937 -> 0.4078), which is
    // participation, not drift.
    let face = 1;
    assert_eq!(
        lifted.sample(CONTROL_CELL, face),
        rest.sample(CONTROL_CELL, face),
        "the static control object must not drift: face {face}"
    );
    assert_ne!(
        lifted.sample(CONTROL_CELL, 0),
        rest.sample(CONTROL_CELL, 0),
        "a control face whose hemisphere holds the object does see the move"
    );
}

#[test]
fn c4_proxy_identity_reparents_the_cache_and_empty_proxies_are_neutral() {
    let world = probe_world();
    let sun = sun();
    let mut volume = probe_volume();
    volume.set_mesh_proxy(Some(probe_proxy(&[probe_placement(OBJECT_CELL)])));
    finish(&mut volume, &world, 0, sun);
    assert!(volume.valid_for(&world, 0, sun));
    // Attaching a different representation clears the published samples at once.
    volume.set_mesh_proxy(Some(probe_proxy(&[probe_placement(LIFTED_CELL)])));
    assert!(!volume.valid_for(&world, 0, sun));
    assert_eq!(volume.sample(OBJECT_CELL, 1), [0.; 3]);
    // A proxy that marks no cell keeps unit-voxel output bit-identical.
    let mut plain = probe_volume();
    finish(&mut plain, &world, 0, sun);
    let mut empty = probe_volume();
    empty.set_mesh_proxy(Some(probe_proxy(&[])));
    finish(&mut empty, &world, 0, sun);
    assert!(empty.has_mesh_proxy() && empty.mesh_digest().is_some());
    assert!(empty.resident_bytes() > plain.resident_bytes());
    for cell in [OBJECT_CELL, RECEIVER_CELL, [-3, 1, 0], [-5, 0, 4]] {
        for face in 0..6 {
            assert_eq!(empty.sample(cell, face), plain.sample(cell, face));
        }
    }
}

#[test]
fn c5_proxy_rasterizes_voxel_faces_exactly_and_clips_to_its_box() {
    // A two-voxel stack meshes into a 1x2x1 block: exactly those two cells, not
    // an inflated AABB around them.
    let mut voxels = World::new(0);
    voxels.set([0, 0, 0], 1);
    voxels.set([0, 1, 0], 1);
    let meshes = [probe_prototype(), voxels.mesh()];
    let materials = [PROBE_OBJECT, PROBE_WALL];
    let placements = [
        probe_placement([1, 0, 1]),
        StaticInstance {
            prototype: 1,
            translation: [3., 0., 0.],
            yaw_quarters: 0,
        },
    ];
    let proxy = MeshProxy::build(
        &MeshGeometry {
            meshes: &meshes,
            instances: &placements,
            materials: &materials,
        },
        [-2, -2, -2],
        [6, 6, 6],
    )
    .unwrap();
    assert_eq!(proxy.occupied_cells(), 3);
    assert_eq!(proxy.material_at([1, 0, 1]), PROBE_OBJECT);
    assert_eq!(proxy.material_at([3, 0, 0]), PROBE_WALL);
    assert_eq!(proxy.material_at([3, 1, 0]), PROBE_WALL);
    assert_eq!(proxy.material_at([3, 2, 0]), 0);
    assert_eq!(proxy.material_at([3, 1, 1]), 0);
    let cells: Vec<_> = proxy.cells().collect();
    assert_eq!(
        cells,
        vec![
            ([3, 0, 0], PROBE_WALL),
            ([3, 1, 0], PROBE_WALL),
            ([1, 0, 1], PROBE_OBJECT),
        ],
        "cells are emitted in ascending local-index order"
    );
    // A ray outside the coverage box never enters it; inside it hits the proxy.
    assert_eq!(proxy.raycast([10., 10., 10.], [1., 0., 0.], 4.), None);
    assert_eq!(
        proxy
            .raycast([3.5, 0.5, -3.], [0., 0., 1.], 8.)
            .unwrap()
            .cell,
        [3, 0, 0]
    );
    // Clipping: a placement outside the box contributes nothing.
    let block_only = |translation: [f32; 3], dims: [u32; 3]| {
        MeshProxy::build(
            &MeshGeometry {
                meshes: &meshes[1..2],
                instances: &[StaticInstance {
                    prototype: 0,
                    translation,
                    yaw_quarters: 0,
                }],
                materials: &materials[1..2],
            },
            [-2, -2, -2],
            dims,
        )
        .unwrap()
    };
    let clipped = MeshProxy::build(
        &MeshGeometry {
            meshes: &meshes,
            instances: &[placements[0], probe_placement([100, 0, 0])],
            materials: &materials,
        },
        [-2, -2, -2],
        [4, 4, 4],
    )
    .unwrap();
    assert_eq!(clipped.occupied_cells(), 1);
    assert_eq!(clipped.material_at([1, 0, 1]), PROBE_OBJECT);
    // Identity follows content: an equal placement and box agree, a moved
    // placement does not.
    let placed = block_only([1., 0., 1.], [6, 6, 6]);
    let same = block_only([1., 0., 1.], [6, 6, 6]);
    let moved = block_only([1., 1., 1.], [6, 6, 6]);
    assert_eq!(placed.digest(), same.digest());
    assert_ne!(placed.digest(), moved.digest());
}

#[test]
fn c6_proxy_validates_inputs_and_keeps_the_world_authoritative() {
    let meshes = [probe_prototype()];
    let materials = [PROBE_OBJECT];
    let build = |instances: &[StaticInstance], materials: &[u8], dims: [u32; 3]| {
        MeshProxy::build(
            &MeshGeometry {
                meshes: &meshes,
                instances,
                materials,
            },
            [0, 0, 0],
            dims,
        )
    };
    let one = [probe_placement([0, 0, 0])];
    assert!(
        build(&one, &[], [4; 3]).is_err(),
        "one material per prototype"
    );
    assert!(build(&one, &[0], [4; 3]).is_err(), "material zero is air");
    assert!(build(&one, &materials, [0, 4, 4]).is_err(), "nonzero dims");
    assert!(
        build(&one, &materials, [129, 1, 1]).is_err(),
        "per-axis cap"
    );
    assert!(build(&one, &materials, [64, 64, 65]).is_err(), "cell cap");
    assert!(
        build(&one, &materials, [8192, 1, 1]).is_err(),
        "precise coordinate range"
    );
    assert!(
        build(
            &[StaticInstance {
                prototype: 1,
                translation: [0.; 3],
                yaw_quarters: 0
            }],
            &materials,
            [4; 3]
        )
        .is_err(),
        "prototype index"
    );
    assert!(
        build(
            &[StaticInstance {
                prototype: 0,
                translation: [f32::NAN, 0., 0.],
                yaw_quarters: 0
            }],
            &materials,
            [4; 3]
        )
        .is_err(),
        "finite translation"
    );
    assert!(
        build(
            &[StaticInstance {
                prototype: 0,
                translation: [0.; 3],
                yaw_quarters: 4
            }],
            &materials,
            [4; 3]
        )
        .is_err(),
        "yaw range"
    );
    let mut broken = probe_prototype();
    broken.indices.pop();
    assert!(
        MeshProxy::build(
            &MeshGeometry {
                meshes: &[broken],
                instances: &one,
                materials: &materials,
            },
            [0; 3],
            [4; 3]
        )
        .is_err(),
        "complete triangles"
    );
    let mut out_of_range = probe_prototype();
    out_of_range.indices[0] = 9_999;
    assert!(
        MeshProxy::build(
            &MeshGeometry {
                meshes: &[out_of_range],
                instances: &one,
                materials: &materials,
            },
            [0; 3],
            [4; 3]
        )
        .is_err(),
        "index range"
    );
    // World data wins wherever it is solid: merging touches only air cells.
    let mut world = World::new(0);
    world.set([0, 0, 0], PROBE_WALL);
    let proxy = build(&one, &materials, [4; 3]).unwrap();
    assert_eq!(proxy.material_at([0, 0, 0]), PROBE_OBJECT);
    let mut merged = world.clone();
    assert_eq!(proxy.merge_into(&mut merged).unwrap(), 0);
    assert_eq!(merged.get([0, 0, 0]), PROBE_WALL);
    assert_eq!(proxy.merge_into(&mut World::new(0)).unwrap(), 1);
}

// ---------------------------------------------------------------------------
// Slanted-geometry rasterization. A proxy must mark a cell only when the
// triangle actually intersects it: filling the triangle's whole AABB turns a
// 2D surface into a solid volume, which is false occlusion and false
// reflection hits on exactly the slanted flora/branch geometry this engine
// renders.
// ---------------------------------------------------------------------------

fn triangle_mesh(triangles: &[[[f32; 3]; 3]]) -> Mesh {
    let mut mesh = Mesh::default();
    for triangle in triangles {
        let base = mesh.vertices.len() as u32;
        for position in triangle {
            mesh.vertices.push(Vertex {
                position: *position,
                normal: [0.; 3],
                color: [1.; 3],
            });
        }
        mesh.indices.extend_from_slice(&[base, base + 1, base + 2]);
    }
    mesh
}

fn mesh_proxy_of(mesh: Mesh, origin: [i32; 3], dimensions: [u32; 3]) -> Result<MeshProxy, String> {
    let meshes = [mesh];
    let materials = [PROBE_OBJECT];
    MeshProxy::build(
        &MeshGeometry {
            meshes: &meshes,
            instances: &[StaticInstance {
                prototype: 0,
                translation: [0.; 3],
                yaw_quarters: 0,
            }],
            materials: &materials,
        },
        origin,
        dimensions,
    )
}

fn triangle_proxy(
    triangles: &[[[f32; 3]; 3]],
    origin: [i32; 3],
    dimensions: [u32; 3],
) -> MeshProxy {
    mesh_proxy_of(triangle_mesh(triangles), origin, dimensions).unwrap()
}

#[test]
fn c9_slanted_triangles_mark_only_cells_they_intersect() {
    // A sheet in the plane `z = y` spanning the whole 4x4x4 box. Its AABB is the
    // box, but the surface only passes near the diagonal plane.
    let sheet = [[[0.1, 0.1, 0.1], [3.9, 0.1, 0.1], [3.9, 3.9, 3.9]]];
    let proxy = triangle_proxy(&sheet, [0; 3], [4; 3]);
    for cell in [[0, 3, 0], [0, 0, 3], [3, 0, 3], [2, 3, 0]] {
        assert_eq!(
            proxy.material_at(cell),
            0,
            "cell {cell:?} is more than one cell from the z = y plane"
        );
    }
    for cell in [[0, 0, 0], [1, 1, 1], [3, 3, 3]] {
        assert_eq!(
            proxy.material_at(cell),
            PROBE_OBJECT,
            "cell {cell:?} lies on the surface"
        );
    }
    assert!(
        proxy.occupied_cells() < 64,
        "a slanted sheet must not fill its bounding box: {} of 64",
        proxy.occupied_cells()
    );
    assert!(proxy.occupied_cells() >= 10, "the surface is still marked");
    // A thin ribbon along the main diagonal: AABB is still the whole box.
    let ribbon = [[[0.1, 0.1, 0.1], [3.9, 3.9, 3.9], [0.1, 0.4, 0.1]]];
    let proxy = triangle_proxy(&ribbon, [0; 3], [4; 3]);
    for cell in [[3, 3, 0], [0, 3, 0], [0, 0, 3], [3, 0, 3]] {
        assert_eq!(
            proxy.material_at(cell),
            0,
            "cell {cell:?} is off the ribbon"
        );
    }
    for cell in [[0, 0, 0], [2, 2, 2], [3, 3, 3]] {
        assert_eq!(
            proxy.material_at(cell),
            PROBE_OBJECT,
            "cell {cell:?} lies on the ribbon"
        );
    }
    assert!(
        proxy.occupied_cells() < 32,
        "a thin diagonal ribbon must stay thin: {} of 64 (the AABB fill marks all 64; \
         the remainder count here includes cells the closed test touches at a room \
         corner, which are marked on purpose so no ray can slip through)",
        proxy.occupied_cells()
    );
}

#[test]
fn c10_proxy_triangle_budget_is_enforced() {
    // Each of these triangles spans the whole 64^3 coverage box, so candidates
    // alone exceed MAX_MESH_PROXY_TESTS before the box could be filled.
    let wide = [[0.0, 0.0, 0.0], [64.0, 0.0, 0.0], [0.0, 64.0, 64.0]];
    let error = match mesh_proxy_of(triangle_mesh(&vec![wide; 17]), [0; 3], [64; 3]) {
        Ok(_) => panic!("the triangle/cell test budget must bound the build"),
        Err(error) => error,
    };
    assert!(error.contains("budget"), "unexpected error: {error}");
    // Cost, not memory, is the unbounded input: the same box with one triangle
    // is accepted and stays box-bounded.
    let proxy = triangle_proxy(&[wide], [0; 3], [64; 3]);
    assert!(proxy.occupied_cells() > 0 && proxy.occupied_cells() <= 64 * 64 * 64);
}

#[test]
fn c11_slant_rasterization_covers_sampled_surface_points() {
    // Independent check in the other direction: sample the surface itself and
    // require every sampled point's cell to be marked, so a ray crossing the
    // proxy surface cannot slip through a gap the intersection test created.
    let sheet = [[[0.1, 0.1, 0.1], [3.9, 0.1, 0.1], [3.9, 3.9, 3.9]]];
    let proxy = triangle_proxy(&sheet, [0; 3], [4; 3]);
    let [a, b, c] = sheet[0];
    for i in 0..=16u32 {
        for j in 0..=(16 - i) {
            let u = i as f32 / 16.;
            let v = j as f32 / 16.;
            let w = 1. - u - v;
            let point: [f32; 3] =
                std::array::from_fn(|axis| a[axis] * u + b[axis] * v + c[axis] * w);
            let cell = point.map(|value| value.floor() as i32);
            assert_eq!(
                proxy.material_at(cell),
                PROBE_OBJECT,
                "surface point {point:?} in cell {cell:?} is unmarked"
            );
        }
    }
}

#[test]
fn c12_degenerate_and_planar_triangles_stay_bounded() {
    // Zero area away from a boundary: exactly its own cell.
    let dot = [[[1.5, 1.5, 1.5]; 3]];
    let proxy = triangle_proxy(&dot, [0; 3], [4; 3]);
    assert_eq!(proxy.occupied_cells(), 1);
    assert_eq!(proxy.material_at([1, 1, 1]), PROBE_OBJECT);
    // Zero area exactly on a cell corner has no orientation, so it keeps both
    // sides of each integer plane: bounded by eight cells, never the whole box.
    let corner = [[[1.0, 1.0, 1.0]; 3]];
    let proxy = triangle_proxy(&corner, [0; 3], [4; 3]);
    assert!(
        (1..=8).contains(&proxy.occupied_cells()),
        "degenerate corner marked {} cells",
        proxy.occupied_cells()
    );
    // A quad lying exactly in the integer plane y = 2 and facing +Y: the mirror
    // lookup for such a fragment reads the cell below the plane, and nothing
    // above it may be marked.
    let quad = [[[0.1, 2.0, 0.1], [1.9, 2.0, 1.9], [1.9, 2.0, 0.1]]];
    let proxy = triangle_proxy(&quad, [0; 3], [4; 3]);
    assert_eq!(proxy.material_at([0, 1, 0]), PROBE_OBJECT);
    assert_eq!(proxy.material_at([1, 1, 1]), PROBE_OBJECT);
    assert_eq!(proxy.material_at([0, 2, 0]), 0, "nothing above a +Y face");
    assert_eq!(proxy.material_at([1, 2, 1]), 0, "nothing above a +Y face");
    // The same quad wound the other way faces -Y and flips to the cell above.
    let flipped = [[[0.1, 2.0, 0.1], [1.9, 2.0, 0.1], [1.9, 2.0, 1.9]]];
    let proxy = triangle_proxy(&flipped, [0; 3], [4; 3]);
    assert_eq!(proxy.material_at([0, 2, 0]), PROBE_OBJECT);
    assert_eq!(proxy.material_at([0, 1, 0]), 0, "nothing below a -Y face");
}

// ===========================================================================
// D3.2 dynamic-response probes. Declared criteria, asserted rather than
// eyeballed:
//
// L1 moving sun: a divider wall's two faces gather opposite floor halves; a low
//    sun puts the down-sun half in the wall's own shadow, so the up-sun-facing
//    face must brighten and the down-sun-facing face must go exactly dark, and
//    the reverse sun flips the pair. The wall top cannot see the floor under any
//    sun and is the static control: exactly zero throughout.
// L2 enclosure: a sealed unit-voxel room leaks nothing on any interior face
//    (`SEALED_ENCLOSURE_LEAKAGE_MAX`), a partial opening admits strictly more
//    than that bound but no more than a full opening, and closing the roof
//    removes the response after the invalidating update.
// L3 voxel edit: one authoritative `World::set` updates the indirect response
//    (red floor replaced by a sunlit green voxel), leaves a face whose hemisphere
//    cannot see the edit bit-identical, and keeps every untouched footprint and
//    the attached mesh-proxy identity unchanged.
// L5 colour bleeding: the two faces of a column standing on a half-red,
//    half-green floor report the hue of the half each one gathers, and swapping
//    the two albedos swaps the responses.
// L6 latency: a key change clears the old radiance in the first `update` call
//    that observes it, and the declared work bound bounds the calls to complete.
// L7 stale publication: a superseding light clears the previous result in the
//    same call, an incomplete volume is never valid or publishable, and a fresh
//    computation of the same key is bit-identical (no mixed partial state).
// ===========================================================================
use crate::indirect::{footprint_digest, light_key};

fn tilted_sun(direction: [f32; 3]) -> Sun {
    Sun {
        direction_to_sun: direction,
        intensity: 1.,
    }
}

/// Symmetric red floor split by a one-cell-thick, three-cell-tall wall at
/// `x = 0`. With the sun low toward +X (`[2, 1, 0]`, slope 0.5) the wall's
/// shadow reaches 6 cells, covering the whole five-cell -X half, while the +X
/// half stays lit; the reverse sun mirrors that exactly.
fn divider_world() -> World {
    let mut world = World::new(0);
    for x in -5..5 {
        for z in -5..5 {
            world.set([x, -1, z], PROBE_FLOOR);
        }
    }
    for z in -5..5 {
        for y in 0..3 {
            world.set([0, y, z], PROBE_WALL);
        }
    }
    world
}

#[test]
fn moving_sun_redirects_indirect_response_and_static_control_holds() {
    let world = divider_world();
    let mut vertical = probe_volume();
    finish(&mut vertical, &world, 0, sun());
    let mut toward_pos_x = probe_volume();
    finish(&mut toward_pos_x, &world, 0, tilted_sun([2., 1., 0.]));
    let mut toward_neg_x = probe_volume();
    finish(&mut toward_neg_x, &world, 0, tilted_sun([-2., 1., 0.]));
    let face = |v: &IndirectVolume, f: usize| v.sample([0, 1, 0], f)[0];
    // A vertical sun lights both halves: both wall faces gather red floor.
    assert!(
        face(&vertical, 0) > 0.05 && face(&vertical, 1) > 0.05,
        "vertical sun: {} {}",
        face(&vertical, 0),
        face(&vertical, 1)
    );
    // Low sun toward +X: the -X half is entirely inside the wall's shadow, so
    // the +X face gathers lit floor and the -X face gathers nothing.
    assert!(face(&toward_pos_x, 0) > 0.05, "sun side stays lit");
    assert_eq!(face(&toward_pos_x, 1), 0.0, "down-sun half is shadowed");
    // The reverse sun flips exactly that pair.
    assert!(face(&toward_neg_x, 1) > 0.05, "sun side stays lit");
    assert_eq!(face(&toward_neg_x, 0), 0.0, "down-sun half is shadowed");
    // The wall top cannot see the floor under any sun: the static control never
    // drifts.
    for volume in [&vertical, &toward_pos_x, &toward_neg_x] {
        assert_eq!(
            volume.sample([0, 2, 0], 2),
            [0.; 3],
            "+Y gathers upward only"
        );
    }
    // The sun is part of the source identity: a volume for one sun is not valid
    // for another, and a scaled direction is the same physical light.
    assert!(vertical.valid_for(&world, 0, sun()));
    assert!(!vertical.valid_for(&world, 0, tilted_sun([2., 1., 0.])));
    assert_eq!(
        light_key(sun()).unwrap(),
        light_key(tilted_sun([0., 2., 0.])).unwrap(),
        "a scaled sun direction is the same light"
    );
    assert_ne!(
        light_key(sun()).unwrap(),
        light_key(tilted_sun([2., 1., 0.])).unwrap()
    );
    // Recomputing the same key reproduces the same radiance bit for bit.
    let mut repeat = probe_volume();
    finish(&mut repeat, &world, 0, sun());
    for cell in [[0, 1, 0], [0, 2, 0], [-3, 1, 0], [3, 1, 3]] {
        for f in 0..6 {
            assert_eq!(repeat.sample(cell, f), vertical.sample(cell, f));
        }
    }
}

/// Declared leakage bound for a sealed unit-voxel enclosure. The sampler is an
/// exact DDA over the authoritative grid and a miss is black, so a closed
/// enclosure reports exactly zero on every interior face; the constant is the
/// contract this test asserts, not an eyeballed tolerance.
const SEALED_ENCLOSURE_LEAKAGE_MAX: f32 = 0.0;

#[test]
fn enclosure_response_follows_the_opening_and_sealed_leakage_is_bounded() {
    let face = 0; // +X face of the wall at [-3, 1, 0], pointing into the room.
    let sealed = {
        let world = room(true);
        let mut volume = volume();
        finish(&mut volume, &world, 0, sun());
        volume
    };
    for f in 0..6 {
        assert!(
            sealed.sample([-3, 1, 0], f)[0] <= SEALED_ENCLOSURE_LEAKAGE_MAX,
            "sealed room face {f} leaks above the declared bound: {:?}",
            sealed.sample([-3, 1, 0], f)
        );
    }
    // A 3x3 skylight admits indirect light: the same interior face now sees a
    // sunlit floor patch through the opening.
    let mut world = room(true);
    for x in -1..=1 {
        for z in -1..=1 {
            world.set([x, 3, z], 0);
        }
    }
    let mut skylight = volume();
    finish(&mut skylight, &world, 0, sun());
    let admitted = skylight.sample([-3, 1, 0], face)[0];
    assert!(
        admitted > SEALED_ENCLOSURE_LEAKAGE_MAX,
        "a skylight must admit light: {admitted}"
    );
    assert!(
        admitted > skylight.sample([-3, 1, 0], 1)[0],
        "the admission is directional: it enters on the opening's side"
    );
    // Opening the whole roof lights a superset of the skylight's floor, so it
    // cannot admit less.
    for x in -3..=3 {
        for z in -3..=3 {
            world.set([x, 3, z], 0);
        }
    }
    let mut open = volume();
    finish(&mut open, &world, 0, sun());
    let fully_open = open.sample([-3, 1, 0], face)[0];
    assert!(
        fully_open >= admitted,
        "full opening {fully_open} vs skylight {admitted}"
    );
    assert!(fully_open > 0.03, "the open room must admit light");
    // Closing the roof removes the response: the first update after the edit
    // clears the old open-room radiance, so no stale value can be observed, and
    // the recomputed sealed room is back under the declared bound.
    for x in -3..=3 {
        for z in -3..=3 {
            world.set([x, 3, z], 2);
        }
    }
    open.update(&world, 0, sun(), UpdateBudget { rays: 0, work: 0 })
        .unwrap();
    assert!(!open.complete(), "the roof edit invalidates the cache");
    for f in 0..6 {
        assert!(
            open.sample([-3, 1, 0], f)[0] <= SEALED_ENCLOSURE_LEAKAGE_MAX,
            "a closed roof must remove the response on face {f}"
        );
    }
    finish(&mut open, &world, 0, sun());
    assert!(open.sample([-3, 1, 0], face)[0] <= SEALED_ENCLOSURE_LEAKAGE_MAX);
}

#[test]
fn voxel_edit_updates_indirect_and_leaves_untouched_identity_unchanged() {
    let before = probe_world();
    let edit_cell = [-3, 1, 0];
    let face = 0; // +X face of the receiver's top cell, looking at the edit.
    let control_face = 1; // -X face, whose hemisphere cannot contain the edit.
    let sample_cell = [-4, 2, 0];
    let untouched_proxy = probe_placement([3, 0, 3]);
    let mut volume = probe_volume();
    volume.set_mesh_proxy(Some(probe_proxy(&[untouched_proxy])));
    finish(&mut volume, &before, 0, sun());
    let (red_before, green_before) = {
        let v = volume.sample(sample_cell, face);
        (v[0], v[1])
    };
    assert!(
        red_before > 0.05 && red_before > 10. * green_before,
        "before: the red floor dominates the gather: {red_before} {green_before}"
    );
    let control_before = volume.sample(sample_cell, control_face);
    let proxy_digest = volume.mesh_digest();
    assert!(proxy_digest.is_some());

    let mut after = before.clone();
    assert!(after.set(edit_cell, PROBE_OBJECT));

    // The edit is recognized in the first update: the old radiance is cleared
    // and nothing is valid for publication until the new revision completes.
    let stats = volume
        .update(&after, 0, sun(), UpdateBudget { rays: 0, work: 0 })
        .unwrap();
    assert!(!stats.complete && !volume.complete());
    assert!(!volume.valid_for(&after, 0, sun()));
    assert_eq!(volume.sample(sample_cell, face), [0.; 3]);

    finish(&mut volume, &after, 0, sun());
    let (red_after, green_after) = {
        let v = volume.sample(sample_cell, face);
        (v[0], v[1])
    };
    assert!(
        red_after < red_before,
        "the added voxel occludes red floor: {red_after} vs {red_before}"
    );
    assert!(
        green_after > green_before * 2.,
        "the voxel's green albedo enters the gather: {green_after} vs {green_before}"
    );
    // A face whose hemisphere points away from the edit keeps its prior value
    // exactly: the edit is local, not a whole-scene re-render.
    assert_eq!(volume.sample(sample_cell, control_face), control_before);
    // The edit changed no mesh geometry: the proxy identity is untouched, so
    // the proxy does not need a rebuild.
    assert_eq!(volume.mesh_digest(), proxy_digest);
    let rebuilt = probe_proxy(&[untouched_proxy]);
    assert_eq!(rebuilt.digest(), proxy_digest.unwrap());
    // Untouched world footprint keeps its digest; an edited footprint changes.
    let untouched_origin = [-5, -2, -5];
    let untouched_dims = [2, 6, 11]; // x = -5, -4: the edit at x = -3 is outside.
    assert_eq!(
        footprint_digest(&before, untouched_origin, untouched_dims),
        footprint_digest(&after, untouched_origin, untouched_dims)
    );
    let edited_dims = [3, 6, 11]; // includes x = -3.
    assert_ne!(
        footprint_digest(&before, untouched_origin, edited_dims),
        footprint_digest(&after, untouched_origin, edited_dims)
    );
    // The updated result is exactly the result of computing the edited scene
    // from scratch: no value of the pre-edit scene survives the invalidation.
    let mut fresh = probe_volume();
    fresh.set_mesh_proxy(Some(probe_proxy(&[untouched_proxy])));
    finish(&mut fresh, &after, 0, sun());
    for cell in [sample_cell, [0, 1, 0], [-3, 1, 0], [2, 1, 2]] {
        for f in 0..6 {
            assert_eq!(volume.sample(cell, f), fresh.sample(cell, f));
        }
    }
}

#[test]
fn colour_bleeding_carries_the_source_hue_to_the_neighbouring_surface() {
    let split = |negative_half: u8, positive_half: u8| {
        let mut world = World::new(0);
        for x in -5..5 {
            for z in -5..5 {
                world.set(
                    [x, -1, z],
                    if x < 0 { negative_half } else { positive_half },
                );
            }
        }
        for y in 0..3 {
            world.set([0, y, 0], PROBE_WALL);
        }
        world
    };
    let mut world = split(PROBE_FLOOR, PROBE_OBJECT);
    let mut volume = probe_volume();
    finish(&mut volume, &world, 0, sun());
    let negative = volume.sample([0, 1, 0], 1); // gathers the -X floor half
    let positive = volume.sample([0, 1, 0], 0); // gathers the +X floor half
    assert!(
        negative[0] > 2. * negative[1] && negative[0] > 0.05,
        "the -X face must gather the red half: {negative:?}"
    );
    assert!(
        positive[1] > 2. * positive[0] && positive[1] > 0.05,
        "the +X face must gather the green half: {positive:?}"
    );
    // Swapping the two albedos swaps the two responses, so the hue follows the
    // surface colour and not an asymmetry in the sampling pattern.
    world = split(PROBE_OBJECT, PROBE_FLOOR);
    let mut swapped = probe_volume();
    finish(&mut swapped, &world, 0, sun());
    let negative = swapped.sample([0, 1, 0], 1);
    let positive = swapped.sample([0, 1, 0], 0);
    assert!(
        negative[1] > 2. * negative[0] && negative[1] > 0.05,
        "after the swap the -X face must gather the green half: {negative:?}"
    );
    assert!(
        positive[0] > 2. * positive[1] && positive[0] > 0.05,
        "after the swap the +X face must gather the red half: {positive:?}"
    );
}

/// Fixture work budget for the latency probe. The volume has 48 face slots and
/// 8 samples, so a changed key starts with at most 384 work units pending; a
/// 32-unit slice therefore completes it in at most 12 calls, the declared bound.
const LATENCY_WORK_BUDGET: usize = 32;
const LATENCY_CALL_BOUND: usize = 12;

#[test]
fn lighting_latency_is_bounded_and_declared() {
    let mut world = World::new(0);
    world.set([0, 0, 0], PROBE_FLOOR);
    let mut volume = IndirectVolume::new([0, 0, 0], [2, 2, 2], 8, 16., probe_palette()).unwrap();
    assert!(!volume.complete(), "a fresh volume has no current key");
    // The first call with the key spends no work but makes the change fully
    // observable: the volume is cleared and the remaining work is known.
    volume
        .update(&world, 0, sun(), UpdateBudget { rays: 0, work: 0 })
        .unwrap();
    assert!(!volume.complete());
    let pending = volume.pending_work();
    assert!(
        pending <= 48 * 8,
        "pending_work must bound the fixture: {pending}"
    );
    assert!(
        pending.div_ceil(LATENCY_WORK_BUDGET) <= LATENCY_CALL_BOUND,
        "the declared bound must cover the fixture"
    );
    let mut calls = 0;
    let mut previous = pending;
    while !volume.complete() {
        let stats = volume
            .update(
                &world,
                0,
                sun(),
                UpdateBudget {
                    rays: 4096,
                    work: LATENCY_WORK_BUDGET,
                },
            )
            .unwrap();
        calls += 1;
        assert!(stats.rays <= 4096 && stats.work <= LATENCY_WORK_BUDGET);
        assert!(
            calls <= LATENCY_CALL_BOUND,
            "completion took {calls} calls, above the declared bound {LATENCY_CALL_BOUND}"
        );
        let remaining = volume.pending_work();
        assert!(remaining < previous, "work must advance every call");
        assert_eq!(volume.complete(), stats.complete);
        previous = remaining;
    }
    assert_eq!(volume.pending_work(), 0);
    assert!(volume.valid_for(&world, 0, sun()));
    // The sliced result equals one full-budget call: scheduling changes when the
    // value appears, never what it converges to.
    let mut whole = IndirectVolume::new([0, 0, 0], [2, 2, 2], 8, 16., probe_palette()).unwrap();
    whole
        .update(
            &world,
            0,
            sun(),
            UpdateBudget {
                rays: usize::MAX,
                work: usize::MAX,
            },
        )
        .unwrap();
    assert!(whole.complete());
    for cell in [[0, 0, 0], [1, 1, 1]] {
        for f in 0..6 {
            assert_eq!(volume.sample(cell, f), whole.sample(cell, f));
        }
    }
}

#[test]
fn superseded_lighting_never_becomes_visible() {
    let world = probe_world();
    let sun_a = sun();
    let sun_b = tilted_sun([2., 1., 0.]);
    let mut volume = probe_volume();
    finish(&mut volume, &world, 0, sun_a);
    assert!(volume.complete());
    assert!(volume.valid_for(&world, 0, sun_a));
    assert!(volume.source_valid(&world, 0));
    assert!(volume.sample(RECEIVER_CELL, RECEIVER_FACE)[0] > 0.05);
    // The newer light is recognized with a zero-work call: the old radiance is
    // cleared in that same call, so it can never be observed after the change,
    // and neither the old nor the new key validates while incomplete.
    let stats = volume
        .update(&world, 0, sun_b, UpdateBudget { rays: 0, work: 0 })
        .unwrap();
    assert!(!stats.complete && !volume.complete());
    for cell in [RECEIVER_CELL, [0, 1, 0], [-3, 1, 0], [0, 0, 0]] {
        for f in 0..6 {
            assert_eq!(
                volume.sample(cell, f),
                [0.; 3],
                "superseded radiance must be gone from cell {cell:?} face {f}"
            );
        }
    }
    assert!(!volume.valid_for(&world, 0, sun_a));
    assert!(!volume.valid_for(&world, 0, sun_b));
    assert!(
        !volume.source_valid(&world, 0),
        "incomplete is not publishable"
    );
    // Completing the newer light reproduces a fresh volume bit for bit: no
    // sample accumulated for the old light can survive into the new result.
    let mut fresh_b = probe_volume();
    finish(&mut fresh_b, &world, 0, sun_b);
    finish(&mut volume, &world, 0, sun_b);
    for cell in [RECEIVER_CELL, [0, 1, 0], [-3, 1, 0], [0, 0, 0]] {
        for f in 0..6 {
            assert_eq!(volume.sample(cell, f), fresh_b.sample(cell, f));
        }
    }
    assert!(volume.valid_for(&world, 0, sun_b) && volume.source_valid(&world, 0));
    assert!(!volume.valid_for(&world, 0, sun_a));
    // Re-requesting the superseded light reuses no mixed partial state: it
    // clears again and equals a fresh old-light volume.
    let mut fresh_a = probe_volume();
    finish(&mut fresh_a, &world, 0, sun_a);
    finish(&mut volume, &world, 0, sun_a);
    for cell in [RECEIVER_CELL, [0, 1, 0], [-3, 1, 0], [0, 0, 0]] {
        for f in 0..6 {
            assert_eq!(volume.sample(cell, f), fresh_a.sample(cell, f));
        }
    }
    assert!(volume.valid_for(&world, 0, sun_a));
}
