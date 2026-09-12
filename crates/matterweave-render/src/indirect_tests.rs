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
use matterweave_core::Mesh;

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
