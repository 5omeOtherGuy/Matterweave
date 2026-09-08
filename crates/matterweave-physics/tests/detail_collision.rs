//! Behavior of load/edit-time detail collision: what the character and bodies
//! can stand on, walk through and be blocked by, and what a rejected update
//! preserves. Assertions are on observable simulation outcomes and on the
//! authoritative scene queries, not on internal collider bookkeeping.

use matterweave_detail::{material, DetailScene, DetailVolume, Lod, Scale, Transform, Yaw};
use matterweave_physics::{BodySnapshot, Physics, PhysicsSnapshot, FIXED_DT, MAX_DETAIL_COLLIDERS};

const TILE: f32 = 0.25;
/// Distance from the character centre to the bottom of its capsule.
const FEET: f32 = 0.85;
const EYE: f32 = 0.65;

fn volume(id: &str, extent: [i32; 3], material: u8) -> DetailVolume {
    let mut volume = DetailVolume::new(id, Scale::new(TILE).expect("valid scale"));
    for x in 0..extent[0] {
        for y in 0..extent[1] {
            for z in 0..extent[2] {
                volume.set([x, y, z], material).expect("in-budget cell");
            }
        }
    }
    volume
}

/// 8 m x 0.25 m x 8 m stone slab whose local origin is its minimum corner.
fn floor_volume() -> DetailVolume {
    volume("floor", [32, 1, 32], material::BANK_STONE)
}

/// 0.25 m x 2 m x 4 m stone wall extending along +z.
fn wall_volume() -> DetailVolume {
    volume("wall", [1, 8, 16], material::BANK_STONE)
}

fn scene_with_floor() -> DetailScene {
    let mut scene = DetailScene::new();
    scene.add_prototype(floor_volume()).expect("prototype");
    scene
        .place("floor.0", "floor", Transform::identity())
        .expect("placement");
    scene
}

fn physics_on(scene: &DetailScene) -> Physics {
    let mut physics = Physics::new(&matterweave_core::World::new(1));
    physics.replace_detail_scene(scene).expect("applied");
    physics
}

/// Walks the character for `steps` fixed steps and returns its final eye.
fn walk(physics: &mut Physics, velocity: [f32; 3], steps: usize) -> [f32; 3] {
    for _ in 0..steps {
        physics.step(FIXED_DT, velocity, false);
    }
    physics.character_eye()
}

fn settle(physics: &mut Physics, steps: usize) {
    for _ in 0..steps {
        physics.step(FIXED_DT, [0.0; 3], false);
    }
}

#[test]
fn character_stands_on_detail_floor_and_falls_without_it() {
    let scene = scene_with_floor();
    let mut physics = physics_on(&scene);
    assert!(
        physics.teleport([2.0, 2.0, 2.0]),
        "free space above the slab"
    );
    settle(&mut physics, 120);
    assert!(physics.grounded(), "supported by the detail slab");
    let eye = physics.character_eye();
    assert!(
        (eye[1] - (TILE + FEET + EYE)).abs() < 0.05,
        "feet rest on the slab top, eye {eye:?}"
    );

    let stats = physics
        .replace_detail_scene(&DetailScene::new())
        .expect("empty scene clears collision");
    assert_eq!(stats.static_colliders, 0);
    assert_eq!(physics.detail_collider_count(), 0);
    settle(&mut physics, 120);
    assert!(!physics.grounded(), "nothing left to stand on");
    assert!(
        physics.character_eye()[1] < 0.0,
        "the character falls through cleared collision"
    );
}

#[test]
fn detail_wall_blocks_the_character_and_a_cap_blocks_a_jump() {
    let mut scene = scene_with_floor();
    scene.add_prototype(wall_volume()).expect("prototype");
    scene
        .place(
            "wall.0",
            "wall",
            Transform::new([4.0, TILE, 0.0], Yaw::Deg0).expect("transform"),
        )
        .expect("placement");
    // Cap slab: 8 m x 0.25 m x 8 m ceiling 2 m above the floor top.
    scene
        .place(
            "cap.0",
            "floor",
            Transform::new([0.0, 2.25, 0.0], Yaw::Deg0).expect("transform"),
        )
        .expect("placement");
    let mut physics = physics_on(&scene);
    assert!(physics.teleport([2.0, 1.8, 2.0]));
    settle(&mut physics, 120);

    let eye = walk(&mut physics, [4.0, 0.0, 0.0], 180);
    assert!(
        eye[0] < 4.0,
        "the wall stops horizontal motion before 4 m: {eye:?}"
    );
    assert!(eye[0] > 3.0, "the character actually walked up to the wall");

    let mut highest = f32::MIN;
    for _ in 0..120 {
        physics.step(FIXED_DT, [0.0; 3], true);
        highest = highest.max(physics.character_eye()[1]);
    }
    assert!(
        highest < 2.25 - FEET + EYE + 0.05,
        "the cap blocks the jump, highest eye {highest}"
    );
}

#[test]
fn rotated_and_negative_placements_match_the_authoritative_scene() {
    for (index, yaw) in [Yaw::Deg0, Yaw::Deg90, Yaw::Deg180, Yaw::Deg270]
        .into_iter()
        .enumerate()
    {
        let mut scene = DetailScene::new();
        scene.add_prototype(wall_volume()).expect("prototype");
        let translation = [-40.0 + index as f32 * 4.0, 3.0, -30.0];
        scene
            .place(
                "wall.0",
                "wall",
                Transform::new(translation, yaw).expect("transform"),
            )
            .expect("placement");
        let mut physics = physics_on(&scene);

        // Centres of wall cells, rotated into world metres by the scene itself.
        for cell in [[0, 1, 1], [0, 4, 8], [0, 6, 14]] {
            let local = cell.map(|v: i32| (v as f32 + 0.5) * TILE);
            let point = Transform::new(translation, yaw)
                .expect("transform")
                .point_to_world(local);
            assert_eq!(
                scene.is_collidable_world_metres(point),
                Ok(true),
                "{yaw:?} cell {cell:?} is authoritative wall"
            );
            assert!(
                !physics.teleport([point[0], point[1] + EYE, point[2]]),
                "{yaw:?}: physics must reject a teleport inside the wall at {point:?}"
            );
        }
        let free = [translation[0] + 8.0, translation[1] + 1.0, translation[2]];
        assert_eq!(scene.is_collidable_world_metres(free), Ok(false));
        assert!(
            physics.teleport([free[0], free[1] + EYE, free[2]]),
            "{yaw:?}: clear space stays free"
        );
    }
}

#[test]
fn liquid_and_decorative_instances_are_never_walls() {
    let mut scene = DetailScene::new();
    scene
        .add_prototype(volume("water", [32, 8, 32], material::WATER))
        .expect("prototype");
    scene
        .add_prototype(volume("frond", [8, 16, 8], material::FLORA_FROND_BLADE))
        .expect("prototype");
    scene
        .place("water.0", "water", Transform::identity())
        .expect("placement");
    for index in 0..64 {
        scene
            .place(
                format!("frond.{index}"),
                "frond",
                Transform::new([index as f32 * 0.5, 0.0, 1.0], Yaw::Deg90).expect("transform"),
            )
            .expect("placement");
    }
    let mut physics = physics_on(&scene);
    let stats = physics.detail_collision_stats();
    assert_eq!(stats.instances, 65, "every instance was considered");
    assert_eq!(
        stats.static_colliders, 0,
        "no decorative plant or liquid cell becomes a collider"
    );
    assert_eq!(stats.collidable_prototypes, 0);
    assert_eq!(stats.source_collision_cells, 0);

    assert!(physics.teleport([1.0, 1.0, 1.0]), "liquid is not a wall");
    settle(&mut physics, 120);
    assert!(
        !physics.grounded(),
        "the character falls through water/flora"
    );
}

#[test]
fn a_decorative_instance_cannot_mask_an_overlapping_solid() {
    let mut scene = scene_with_floor();
    scene
        .add_prototype(volume("frond", [32, 8, 32], material::FLORA_FROND_BLADE))
        .expect("prototype");
    // Placed over the same space, and earlier in instance order than the floor.
    scene
        .place("aaa.frond", "frond", Transform::identity())
        .expect("placement");
    let mut physics = physics_on(&scene);
    let stats = physics.detail_collision_stats();
    assert_eq!(stats.instances, 2);
    assert_eq!(stats.static_colliders, 1, "only the solid slab collides");
    assert_eq!(stats.shared_shapes, 1, "one shape per collidable prototype");
    assert!(physics.teleport([2.0, 2.0, 2.0]));
    settle(&mut physics, 120);
    assert!(
        physics.grounded(),
        "the overlapping decorative volume does not hide the solid slab"
    );
}

#[test]
fn one_shape_is_shared_by_every_instance_of_a_prototype() {
    let mut scene = DetailScene::new();
    scene.add_prototype(floor_volume()).expect("prototype");
    for index in 0..16 {
        scene
            .place(
                format!("floor.{index}"),
                "floor",
                Transform::new([index as f32 * 8.0, 0.0, 0.0], Yaw::Deg0).expect("transform"),
            )
            .expect("placement");
    }
    let physics = physics_on(&scene);
    let stats = physics.detail_collision_stats();
    assert_eq!(stats.static_colliders, 16);
    assert_eq!(stats.shared_shapes, 1);
    assert_eq!(stats.source_collision_cells, 32 * 32);
    assert_eq!(
        stats.expanded_collision_cells,
        16 * 32 * 32,
        "expanded cells are reported without duplicating storage"
    );
    assert_eq!(
        stats.source_collision_cells * 16,
        stats.expanded_collision_cells
    );
}

#[test]
fn edits_and_removals_republish_collision_without_stale_shapes() {
    let mut scene = scene_with_floor();
    scene.add_prototype(wall_volume()).expect("prototype");
    scene
        .place(
            "wall.0",
            "wall",
            Transform::new([4.0, TILE, 0.0], Yaw::Deg0).expect("transform"),
        )
        .expect("placement");
    let mut physics = physics_on(&scene);
    assert!(physics.teleport([2.0, 2.0, 2.0]));
    settle(&mut physics, 60);
    assert!(walk(&mut physics, [4.0, 0.0, 0.0], 180)[0] < 4.0, "blocked");

    // Edit: carve a doorway out of the authoritative prototype.
    for y in 0..8 {
        for z in 4..12 {
            scene
                .edit_prototype("wall", [0, y, z], material::AIR)
                .expect("edit");
        }
    }
    physics.replace_detail_scene(&scene).expect("republished");
    assert!(physics.teleport([2.0, 2.0, 2.0]));
    settle(&mut physics, 60);
    assert!(
        walk(&mut physics, [4.0, 0.0, 0.0], 240)[0] > 4.5,
        "the edited doorway is walkable"
    );

    // Restoring the earlier content must rebuild a wall, not reuse a stale shape.
    for y in 0..8 {
        for z in 4..12 {
            scene
                .edit_prototype("wall", [0, y, z], material::BANK_STONE)
                .expect("edit");
        }
    }
    physics.replace_detail_scene(&scene).expect("republished");
    assert!(physics.teleport([2.0, 2.0, 2.0]));
    settle(&mut physics, 60);
    assert!(
        walk(&mut physics, [4.0, 0.0, 0.0], 240)[0] < 4.0,
        "the restored revision blocks again"
    );

    // Removal: a scene without the wall instance drops exactly that collider.
    let floor_only = scene_with_floor();
    let stats = physics
        .replace_detail_scene(&floor_only)
        .expect("republished");
    assert_eq!(stats.static_colliders, 1);
    assert!(physics.teleport([2.0, 2.0, 2.0]));
    settle(&mut physics, 60);
    assert!(walk(&mut physics, [4.0, 0.0, 0.0], 240)[0] > 4.5);
}

#[test]
fn collision_is_independent_of_derived_lod() {
    let mut scene = scene_with_floor();
    scene.add_prototype(wall_volume()).expect("prototype");
    scene
        .place(
            "wall.0",
            "wall",
            Transform::new([4.0, TILE, 0.0], Yaw::Deg0).expect("transform"),
        )
        .expect("placement");
    let mut physics = physics_on(&scene);
    let before = physics.detail_collision_stats();

    for lod in [Lod::Source, Lod::Half, Lod::Quarter] {
        scene.prototype_mesh("wall", lod).expect("derived mesh");
        scene.prototype_mesh("floor", lod).expect("derived mesh");
    }
    let after = physics.replace_detail_scene(&scene).expect("republished");
    assert_eq!(
        (
            after.static_colliders,
            after.source_collision_cells,
            after.merged_boxes
        ),
        (
            before.static_colliders,
            before.source_collision_cells,
            before.merged_boxes
        ),
        "coarse derived levels never change collision"
    );
    assert!(physics.teleport([2.0, 2.0, 2.0]));
    settle(&mut physics, 60);
    assert!(
        walk(&mut physics, [4.0, 0.0, 0.0], 180)[0] < 4.0,
        "a coarse LOD must not remove the physical wall"
    );
}

#[test]
fn a_dropped_body_rests_on_detail_collision() {
    let scene = scene_with_floor();
    let mut physics = physics_on(&scene);
    physics
        .restore(&PhysicsSnapshot {
            version: 1,
            eye: [2.0, 6.0, 2.0],
            bodies: vec![BodySnapshot {
                position: [4.0, 4.0, 4.0],
                rotation: [0.0, 0.0, 0.0, 1.0],
                velocity: [0.0; 3],
                angular_velocity: [0.0; 3],
                dimensions: [2; 3],
                material: 8,
            }],
        })
        .expect("restored");
    for _ in 0..600 {
        physics.step_objects(FIXED_DT);
    }
    let resting = physics.snapshot().bodies[0].position;
    assert!(
        (resting[1] - (TILE + 0.5)).abs() < 0.1,
        "the 1 m cube rests on the slab top, got {resting:?}"
    );
}

#[test]
fn replacement_wakes_bodies_over_the_changed_region() {
    let mut world = matterweave_core::World::new(3);
    for x in -8..8 {
        for z in -8..8 {
            world.set([x, 0, z], 3);
        }
    }
    let mut physics = Physics::new(&world);
    physics
        .restore(&PhysicsSnapshot {
            version: 1,
            eye: [2.0, 6.0, 2.0],
            bodies: vec![BodySnapshot {
                position: [3.5, 1.5, 3.5],
                rotation: [0.0, 0.0, 0.0, 1.0],
                velocity: [0.0; 3],
                angular_velocity: [0.0; 3],
                dimensions: [2; 3],
                material: 8,
            }],
        })
        .expect("restored");
    for _ in 0..600 {
        physics.step_objects(FIXED_DT);
    }
    assert_eq!(
        physics.body_activity().sleeping,
        1,
        "the body settled and slept"
    );

    let mut scene = DetailScene::new();
    scene.add_prototype(floor_volume()).expect("prototype");
    scene
        .place(
            "floor.0",
            "floor",
            Transform::new([1.0, 1.5, 1.0], Yaw::Deg0).expect("transform"),
        )
        .expect("placement");
    let stats = physics.replace_detail_scene(&scene).expect("applied");
    assert_eq!(stats.woken_bodies, 1, "the overlapped body is woken");
    assert_eq!(physics.body_activity().active, 1);
}

#[test]
fn an_over_budget_update_is_rejected_and_preserves_existing_contacts() {
    let scene = scene_with_floor();
    let mut physics = physics_on(&scene);
    assert!(physics.teleport([2.0, 2.0, 2.0]));
    settle(&mut physics, 120);
    assert!(physics.grounded());
    let before = physics.detail_collision_stats();

    let mut oversized = DetailScene::new();
    oversized
        .add_prototype(volume("pebble", [1, 1, 1], material::BANK_STONE))
        .expect("prototype");
    for index in 0..=MAX_DETAIL_COLLIDERS {
        oversized
            .place(
                format!("pebble.{index}"),
                "pebble",
                Transform::new([index as f32 * 0.001, 60.0, 0.0], Yaw::Deg0).expect("transform"),
            )
            .expect("placement");
    }
    let error = physics
        .replace_detail_scene(&oversized)
        .expect_err("an over-budget scene must be rejected, not partially applied");
    assert!(
        error.contains("static colliders"),
        "explicit limit: {error}"
    );

    assert_eq!(
        physics.detail_collision_stats(),
        before,
        "a rejected update leaves the published collision unchanged"
    );
    assert_eq!(physics.detail_collider_count(), 1);
    settle(&mut physics, 60);
    assert!(
        physics.grounded(),
        "the character keeps its contact with the previous colliders"
    );
    assert!(
        !physics.teleport([2.0, 0.1, 2.0]),
        "the previous slab still occupies its space"
    );
}

#[test]
fn clearing_an_already_empty_scene_repeatedly_is_accepted() {
    let mut physics = physics_on(&scene_with_floor());
    for _ in 0..3 {
        let stats = physics
            .replace_detail_scene(&DetailScene::new())
            .expect("clearing is always valid");
        assert_eq!(stats.static_colliders, 0);
        assert_eq!(stats.shared_shapes, 0);
        assert_eq!(stats.expanded_collision_cells, 0);
        assert_eq!(physics.detail_collider_count(), 0);
    }
}

#[test]
fn detail_collision_preserves_legacy_terrain_and_is_not_rebuilt_per_frame() {
    let mut world = matterweave_core::World::new(7);
    for x in -8..8 {
        for z in -8..8 {
            world.set([x, 0, z], 3);
        }
    }
    let mut physics = Physics::new(&world);
    let mut scene = DetailScene::new();
    scene.add_prototype(floor_volume()).expect("prototype");
    scene
        .place(
            "floor.0",
            "floor",
            Transform::new([0.0, 3.0, 0.0], Yaw::Deg0).expect("transform"),
        )
        .expect("placement");
    let stats = physics.replace_detail_scene(&scene).expect("applied");

    // Legacy terrain still supports the character and still blocks teleports.
    assert!(physics.teleport([-2.0, 3.0, -2.0]));
    settle(&mut physics, 120);
    assert!(physics.grounded(), "legacy 1 m terrain still collides");
    assert!((physics.character_eye()[1] - (1.0 + FEET + EYE)).abs() < 0.05);
    assert!(!physics.teleport([-2.0, 0.5, -2.0]), "legacy solid rejects");

    // Stepping never regenerates or drops detail collision.
    let colliders = physics.detail_collider_count();
    settle(&mut physics, 300);
    assert_eq!(physics.detail_collider_count(), colliders);
    assert_eq!(physics.detail_collision_stats(), stats);

    // Legacy chunk streaming/removal keeps working alongside detail colliders.
    for x in -8..8 {
        for z in -8..8 {
            world.set([x, 0, z], 0);
        }
    }
    physics.sync_world(&world);
    assert_eq!(
        physics.detail_collider_count(),
        colliders,
        "terrain sync leaves detail collision alone"
    );
    assert!(
        physics.teleport([-2.0, 0.5, -2.0]),
        "legacy terrain removed"
    );
}

/// Cost of the existing authored `dense_tile` fixture. The assertions are the
/// engineering budgets; the printed line is the measurement recorded in
/// docs/performance/p03/collision/opus-execution.md.
#[test]
fn dense_tile_collision_cost_stays_bounded() {
    let scene = matterweave_detail::dense_tile(matterweave_detail::FLORA_CANONICAL_SEED)
        .expect("authored dense tile");
    let counts = scene.counts();
    let mut physics = Physics::new(&matterweave_core::World::new(1));
    let started = std::time::Instant::now();
    let stats = physics.replace_detail_scene(&scene).expect("applied");
    let elapsed = started.elapsed();
    println!(
        "dense_tile: instances={} colliders={} shapes={} source_collision_cells={} merged_boxes={} expanded_collision_cells={} build={:?}",
        stats.instances,
        stats.static_colliders,
        stats.shared_shapes,
        stats.source_collision_cells,
        stats.merged_boxes,
        stats.expanded_collision_cells,
        elapsed
    );
    assert_eq!(stats.instances, counts.instances);
    assert!(
        stats.static_colliders < stats.instances,
        "decorative-only plants must not each get a collider: {} of {}",
        stats.static_colliders,
        stats.instances
    );
    assert!(
        stats.merged_boxes < stats.source_collision_cells,
        "greedy merging must reduce the collider cost below one box per cell: {} vs {}",
        stats.merged_boxes,
        stats.source_collision_cells
    );
    assert!(stats.shared_shapes <= counts.prototypes);
    assert_eq!(
        stats.expanded_collision_cells, counts.expanded_collision_cells,
        "reported expanded collision cells match the authoritative scene"
    );
}

#[test]
fn decorative_scene_is_admitted_by_collision_cost_not_visual_counts() {
    let mut scene = DetailScene::new();
    scene
        .add_prototype(volume("leaf", [1, 1, 1], material::FLORA_FROND_BLADE))
        .unwrap();
    for i in 0..=MAX_DETAIL_COLLIDERS {
        scene
            .place(format!("leaf-{i}"), "leaf", Transform::identity())
            .unwrap();
    }
    let stats = physics_on(&scene).detail_collision_stats();
    assert_eq!(stats.instances, MAX_DETAIL_COLLIDERS + 1);
    assert_eq!(stats.static_colliders, 0);
    assert_eq!(stats.source_collision_cells, 0);
    let mut scene = DetailScene::new();
    // Above the previous one-million-occupied-cell precheck. Liquid has no wall.
    scene
        .add_prototype(volume("water", [129, 64, 128], material::WATER))
        .unwrap();
    scene
        .place("water", "water", Transform::identity())
        .unwrap();
    let stats = physics_on(&scene).detail_collision_stats();
    assert_eq!(stats.static_colliders, 0);
    assert_eq!(stats.source_collision_cells, 0);
}

#[test]
fn narrow_clearance_and_partition_seams_preserve_contacts() {
    let mut scene = scene_with_floor();
    scene
        .place(
            "cap",
            "floor",
            Transform::new([0., 2., 0.], Yaw::Deg0).unwrap(),
        )
        .unwrap();
    let mut physics = physics_on(&scene);
    // Capsule top is eye + 0.2 m. Test genuine positive clearance and penetration,
    // not the numerically ambiguous exact-touching teleport case.
    for x in [2., 3.99, 4., 4.01, 6.] {
        assert!(physics.teleport([x, 1.78, 2.]), "clear gap x={x}");
        assert!(
            !physics.teleport([x, 1.82, 2.]),
            "ceiling penetration x={x}"
        );
        settle(&mut physics, 60);
        assert!(physics.grounded(), "partition cannot remove support x={x}");
    }
}
