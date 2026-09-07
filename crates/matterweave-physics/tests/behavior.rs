use matterweave_core::World;
use matterweave_physics::{BodySnapshot, Physics, PhysicsSnapshot, FIXED_DT, MAX_BODIES};

fn floor() -> World {
    // Covers the full generated-world extent so the playground arch site near
    // the home camera rests on terrain in tests as it does in the real world.
    let mut world = World::new(5);
    for x in -32..32 {
        for z in -32..32 {
            world.set([x, 0, z], 3);
        }
    }
    world
}
fn advance(physics: &mut Physics, frames: usize, velocity: [f32; 3]) {
    for _ in 0..frames {
        physics.step(FIXED_DT, velocity, false);
    }
}
fn body(position: [f32; 3]) -> BodySnapshot {
    BodySnapshot {
        position,
        rotation: [0.0, 0.0, 0.0, 1.0],
        velocity: [0.0; 3],
        angular_velocity: [0.0; 3],
        dimensions: [2; 3],
        material: 8,
    }
}
#[test]
fn walks_jumps_and_respects_wall_then_edit() {
    let mut world = floor();
    for y in 1..6 {
        for z in -3..4 {
            world.set([4, y, z], 3);
        }
    }
    let mut physics = Physics::new(&world);
    assert!(physics.teleport([0.5, 4.0, 0.5]));
    advance(&mut physics, 120, [0.0; 3]);
    assert!(physics.grounded());
    let standing = physics.character_eye()[1];
    assert!((standing - 2.5).abs() < 0.08, "{standing}");
    physics.step(FIXED_DT, [0.0; 3], true);
    advance(&mut physics, 15, [0.0; 3]);
    assert!(physics.character_eye()[1] > standing + 0.8);
    advance(&mut physics, 100, [0.0; 3]);
    advance(&mut physics, 120, [6.0, 0.0, 0.0]);
    assert!(physics.character_eye()[0] < 3.75);
    for y in 1..6 {
        for z in -3..4 {
            world.set([4, y, z], 0);
        }
    }
    physics.sync_world(&world);
    advance(&mut physics, 40, [6.0, 0.0, 0.0]);
    assert!(physics.character_eye()[0] > 5.0);
}
#[test]
fn removing_support_and_chunk_collision_takes_effect() {
    let mut world = floor();
    let mut physics = Physics::new(&world);
    assert!(physics.teleport([-16.0, 4.0, 0.5]));
    advance(&mut physics, 120, [0.0; 3]);
    assert!(physics.grounded());
    for x in -18..-13 {
        for z in -2..3 {
            world.set([x, 0, z], 0);
        }
    }
    physics.sync_world(&world);
    advance(&mut physics, 90, [0.0; 3]);
    assert!(physics.character_eye()[1] < -2.0);
}
#[test]
fn fixed_time_is_bounded_and_invalid_values_do_not_corrupt_state() {
    let world = floor();
    let mut physics = Physics::new(&world);
    assert_eq!(physics.step(100.0, [0.0; 3], false), 6);
    let before = physics.snapshot();
    assert_eq!(physics.step(f32::NAN, [0.0; 3], false), 0);
    assert_eq!(physics.step(-1.0, [0.0; 3], false), 0);
    assert!(!physics.teleport([f32::NAN, 0.0, 0.0]));
    assert!(!physics.teleport([0.5, 1.0, 0.5]));
    assert_eq!(before, physics.snapshot());
}
#[test]
fn grab_throw_fracture_and_terrain_occlusion() {
    let mut world = floor();
    let mut physics = Physics::new(&world);
    physics
        .restore(&PhysicsSnapshot {
            version: 1,
            eye: [0.5, 3.0, 4.0],
            bodies: vec![body([0.5, 1.6, 0.5])],
        })
        .unwrap();
    let origin = [0.5, 1.6, 4.0];
    let direction = [0.0, 0.0, -1.0];
    world.set([0, 1, 2], 3);
    assert!(!physics.grab(&world, origin, direction, 8.0));
    assert!(!physics.break_body(&world, origin, direction, 8.0));
    world.set([0, 1, 2], 0);
    assert!(physics.grab(&world, origin, direction, 8.0));
    assert!(physics.held());
    assert!(physics.teleport([5.0, 4.0, 5.0]));
    assert!(!physics.held());
    assert!(physics.grab(&world, origin, direction, 8.0));
    physics.update_grab(origin, direction);
    assert!(physics.throw(direction));
    assert!(!physics.held());
    assert!(physics.snapshot().bodies[0].velocity[2] < -12.0);
    assert!(physics.break_body(&world, origin, direction, 8.0));
    let snapshot = physics.snapshot();
    assert_eq!(snapshot.bodies.len(), 8);
    assert!(snapshot.bodies.iter().all(|b| b.dimensions == [1; 3]));
    assert_eq!(physics.dynamic_mesh().indices.len(), 8 * 36);
}
#[test]
fn snapshots_validate_atomically_and_preserve_physical_volumes() {
    let world = floor();
    let mut physics = Physics::new(&world);
    let initial = PhysicsSnapshot {
        version: 1,
        eye: [0.5, 4.0, 4.0],
        bodies: vec![body([0.5, 3.0, 0.5])],
    };
    physics.restore(&initial).unwrap();
    let json = serde_json::to_string(&physics.snapshot()).unwrap();
    let decoded = serde_json::from_str(&json).unwrap();
    assert_eq!(initial, decoded);
    let mut invalid = initial.clone();
    invalid.bodies[0].rotation = [0.0; 4];
    assert!(physics.restore(&invalid).is_err());
    assert_eq!(physics.snapshot(), initial);
    invalid = initial.clone();
    invalid.bodies = vec![body([0.5, 2.0, 0.5]); MAX_BODIES + 1];
    assert!(physics.restore(&invalid).is_err());
    assert_eq!(physics.snapshot(), initial);
}
#[test]
fn distant_bodies_survive_terrain_eviction_and_resume() {
    let world = floor();
    let mut physics = Physics::new(&world);
    physics
        .restore(&PhysicsSnapshot {
            version: 1,
            eye: [0.5, 4.0, 4.0],
            bodies: vec![body([0.5, 3.0, 0.5])],
        })
        .unwrap();
    assert!(physics.teleport([100.0, 4.0, 100.0]));
    physics.sync_world(&World::new(5));
    advance(&mut physics, 180, [0.0; 3]);
    assert_eq!(physics.snapshot().bodies[0].position, [0.5, 3.0, 0.5]);
    physics.sync_world(&world);
    assert!(physics.teleport([0.5, 4.0, 4.0]));
    advance(&mut physics, 180, [0.0; 3]);
    assert!((physics.snapshot().bodies[0].position[1] - 1.5).abs() < 0.1);
}

#[test]
fn jump_edge_survives_short_frame_and_fracture_cap_is_atomic() {
    let world = floor();
    let mut physics = Physics::new(&world);
    assert!(physics.teleport([0.5, 4.0, 4.0]));
    advance(&mut physics, 120, [0.0; 3]);
    let standing = physics.character_eye()[1];
    assert_eq!(physics.step(FIXED_DT * 0.25, [0.0; 3], true), 0);
    advance(&mut physics, 10, [0.0; 3]);
    assert!(physics.character_eye()[1] > standing + 0.7);
    let mut bodies = vec![body([10.0, 5.0, 10.0]); MAX_BODIES];
    bodies[0] = body([0.5, 1.6, 0.5]);
    physics
        .restore(&PhysicsSnapshot {
            version: 1,
            eye: [0.5, 4.0, 4.0],
            bodies,
        })
        .unwrap();
    let before = physics.snapshot();
    assert!(!physics.break_body(&world, [0.5, 1.6, 4.0], [0.0, 0.0, -1.0], 8.0));
    assert_eq!(before, physics.snapshot());
}
#[test]
fn fly_steps_leave_character_stationary_and_fallen_bodies_bounded() {
    let world = World::new(5);
    let mut physics = Physics::new(&world);
    physics
        .restore(&PhysicsSnapshot {
            version: 1,
            eye: [0.0, 4.0, 0.0],
            bodies: vec![body([0.5, 1.5, 0.5])],
        })
        .unwrap();
    for _ in 0..600 {
        physics.step_objects(FIXED_DT);
    }
    let snapshot = physics.snapshot();
    assert_eq!(snapshot.eye, [0.0, 4.0, 0.0]);
    assert_eq!(snapshot.bodies[0].position[1], -32.0);
    assert_eq!(snapshot.bodies[0].velocity, [0.0; 3]);
    physics.restore(&snapshot).unwrap();
}

#[test]
fn moving_fly_camera_at_120hz_preserves_simulation_accumulator() {
    let world = World::new(5);
    let mut physics = Physics::new(&world);
    physics
        .restore(&PhysicsSnapshot {
            version: 1,
            eye: [0.0, 4.0, 0.0],
            bodies: vec![body([0.5, 5.0, 0.5])],
        })
        .unwrap();
    let mut steps = 0;
    for frame in 0..120 {
        assert!(physics.set_flying_eye([frame as f32 * 0.01, 4.0, 0.0]));
        steps += physics.step_objects(FIXED_DT * 0.5);
    }
    assert_eq!(steps, 60);
    assert!(physics.snapshot().bodies[0].position[1] < 0.0);
}

#[test]
fn large_impulses_remain_restorable() {
    let world = World::new(5);
    let mut physics = Physics::new(&world);
    let mut fast = body([0.5, 1.6, 0.5]);
    fast.velocity = [128.0; 3];
    fast.angular_velocity = [128.0; 3];
    physics
        .restore(&PhysicsSnapshot {
            version: 1,
            eye: [0.5, 4.0, 4.0],
            bodies: vec![fast],
        })
        .unwrap();
    assert!(physics.grab(&world, [0.5, 1.6, 4.0], [0.0, 0.0, -1.0], 8.0));
    assert!(physics.throw([1.0, 0.0, 0.0]));
    physics.restore(&physics.snapshot()).unwrap();
    assert!(physics.break_body(&world, [0.5, 1.6, 4.0], [0.0, 0.0, -1.0], 8.0));
    physics.restore(&physics.snapshot()).unwrap();
    for _ in 0..120 {
        physics.step_objects(FIXED_DT);
    }
    physics.restore(&physics.snapshot()).unwrap();
}

#[test]
fn full_body_budget_stack_remains_finite_and_supported() {
    let world = floor();
    let mut physics = Physics::new(&world);
    let mut bodies = Vec::new();
    for x in 0..4 {
        for y in 0..4 {
            for z in 0..4 {
                bodies.push(body([
                    x as f32 * 1.01,
                    1.6 + y as f32 * 1.01,
                    z as f32 * 1.01,
                ]));
            }
        }
    }
    physics
        .restore(&PhysicsSnapshot {
            version: 1,
            eye: [12.0, 4.0, 12.0],
            bodies,
        })
        .unwrap();
    for _ in 0..600 {
        physics.step_objects(FIXED_DT);
    }
    let snapshot = physics.snapshot();
    assert_eq!(snapshot.bodies.len(), MAX_BODIES);
    assert!(snapshot.bodies.iter().all(|body| body.position[1] > 1.0));
    physics.restore(&snapshot).unwrap();
}

fn voxels(dimensions: [u8; 3]) -> usize {
    dimensions.iter().map(|&n| n as usize).product()
}

#[test]
fn playground_spawns_arch_once_and_preserves_demo() {
    let world = floor();
    let mut physics = Physics::new(&world);
    assert!(physics.spawn_playground(&world));
    assert_eq!(physics.body_count(), 6);
    let snapshot = physics.snapshot();
    let mut cubes = 0;
    let mut beams = 0;
    let mut accent = 0;
    let mut total = 0;
    for body in &snapshot.bodies {
        total += voxels(body.dimensions);
        match (body.dimensions, body.material) {
            ([2, 2, 2], 8) => cubes += 1,
            ([6, 2, 2], 8) => beams += 1,
            ([2, 2, 2], 7) => accent += 1,
            unexpected => panic!("unexpected playground body {unexpected:?}"),
        }
    }
    assert_eq!((cubes, beams, accent), (4, 1, 1));
    assert_eq!(total, MAX_BODIES);
    // Spawns only when empty; the demo stays backward compatible.
    assert!(!physics.spawn_playground(&world));
    assert_eq!(physics.snapshot(), snapshot);
    physics.spawn_demo(&world);
    assert_eq!(physics.body_count(), 6);
    let mut fresh = Physics::new(&world);
    fresh.spawn_demo(&world);
    assert!(fresh.body_count() > 0);
    assert!(!fresh.spawn_playground(&world));
}

#[test]
fn playground_arch_settles_supported_on_flat_floor() {
    let world = floor();
    let mut physics = Physics::new(&world);
    assert!(physics.spawn_playground(&world));
    for _ in 0..600 {
        physics.step_objects(FIXED_DT);
    }
    let snapshot = physics.snapshot();
    assert_eq!(snapshot.bodies.len(), 6);
    let beam = snapshot
        .bodies
        .iter()
        .find(|body| body.dimensions == [6, 2, 2])
        .unwrap();
    // Flat floor top is y=1: pillars rest near 1.52/2.54, the beam near 3.56,
    // centered at the scene anchor (12.0, 19.5) in front of the home camera.
    assert!((beam.position[0] - 12.0).abs() < 0.35, "{beam:?}");
    assert!((beam.position[2] - 19.5).abs() < 0.35, "{beam:?}");
    assert!(beam.position[1] > 3.1 && beam.position[1] < 4.0, "{beam:?}");
    for body in &snapshot.bodies {
        assert!(body.position[1] > 1.1, "{body:?}");
    }
}

#[test]
fn playground_fully_fractures_to_body_budget() {
    let world = floor();
    let mut physics = Physics::new(&world);
    assert!(physics.spawn_playground(&world));
    // Largest first via the public ray API. Horizontal rays at each target's
    // own height keep the aimed body nearest without mirroring internals:
    // outer-side origins avoid starting inside a neighboring body.
    let mut breaks = 0;
    for _ in 0..16 {
        let snapshot = physics.snapshot();
        if snapshot.bodies.iter().all(|body| body.dimensions == [1; 3]) {
            break;
        }
        let target = snapshot
            .bodies
            .iter()
            .filter(|body| body.dimensions != [1; 3])
            .max_by_key(|body| voxels(body.dimensions))
            .unwrap()
            .clone();
        let side = if target.position[0] < 12.0 { -3.0 } else { 3.0 };
        let origin = [
            target.position[0] + side,
            target.position[1],
            target.position[2],
        ];
        let direction = if side < 0.0 {
            [1.0, 0.0, 0.0]
        } else {
            [-1.0, 0.0, 0.0]
        };
        assert!(
            physics.break_body(&world, origin, direction, 8.0),
            "valid split refused at {:?}",
            target.position
        );
        breaks += 1;
    }
    assert_eq!(breaks, 6);
    let snapshot = physics.snapshot();
    assert_eq!(snapshot.bodies.len(), MAX_BODIES);
    assert!(snapshot.bodies.iter().all(|body| body.dimensions == [1; 3]));
}

#[test]
fn playground_partial_fracture_roundtrips_through_serde() {
    let world = floor();
    let mut physics = Physics::new(&world);
    assert!(physics.spawn_playground(&world));
    let beam = physics
        .snapshot()
        .bodies
        .iter()
        .find(|body| body.dimensions == [6, 2, 2])
        .unwrap()
        .clone();
    let origin = [beam.position[0] - 3.0, beam.position[1], beam.position[2]];
    assert!(physics.break_body(&world, origin, [1.0, 0.0, 0.0], 8.0));
    let fractured = physics.snapshot();
    assert_eq!(fractured.bodies.len(), 6 - 1 + 24);
    assert_eq!(
        fractured
            .bodies
            .iter()
            .map(|body| voxels(body.dimensions))
            .sum::<usize>(),
        MAX_BODIES
    );
    assert_eq!(
        fractured
            .bodies
            .iter()
            .filter(|body| body.dimensions == [1; 3] && body.material == 8)
            .count(),
        24
    );
    let json = serde_json::to_string(&fractured).unwrap();
    let decoded: PhysicsSnapshot = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, fractured);
    physics.restore(&decoded).unwrap();
    assert_eq!(physics.snapshot(), fractured);
}

#[test]
fn playground_dimension_limits_reject_atomically() {
    let world = floor();
    let mut physics = Physics::new(&world);
    // One breakable cube plus singles: splitting would need 65 bodies.
    let mut bodies = vec![body([0.5, 1.6, 0.5])];
    for index in 0..MAX_BODIES - 1 {
        bodies.push(body([30.0 + index as f32 * 1.01, 1.6, 30.0]));
    }
    physics
        .restore(&PhysicsSnapshot {
            version: 1,
            eye: [0.5, 4.0, 4.0],
            bodies,
        })
        .unwrap();
    let before = physics.snapshot();
    assert!(!physics.break_body(&world, [0.5, 1.6, 3.5], [0.0, 0.0, -1.0], 8.0));
    assert_eq!(physics.snapshot(), before);
    for dimensions in [[7, 1, 1], [7, 2, 2], [6, 6, 6], [6, 3, 3], [0, 2, 2]] {
        let mut invalid = before.clone();
        invalid.bodies[0].dimensions = dimensions;
        assert!(physics.restore(&invalid).is_err(), "{dimensions:?}");
        assert_eq!(physics.snapshot(), before);
    }
    let mut beamed = before.clone();
    beamed.bodies[0].dimensions = [6, 2, 2];
    physics.restore(&beamed).unwrap();
    assert_eq!(physics.snapshot().bodies[0].dimensions, [6, 2, 2]);
}

#[test]
fn nearby_preserved_body_waits_for_its_collision_window() {
    let mut world = World::generate(20260907);
    world.enable_streaming();
    world.stream_around([0., 4., 0.]);
    let mut physics = Physics::new(&world);
    let snapshot = PhysicsSnapshot {
        version: 1,
        eye: [31., 10., 0.],
        bodies: vec![body([70.5, 12., 0.5])],
    };
    physics.restore(&snapshot).unwrap();
    physics.set_flying_eye(snapshot.eye);
    // Within the old 40m distance gate, but outside the published x<64 window.
    for _ in 0..120 {
        physics.step_objects(FIXED_DT);
    }
    assert_eq!(physics.snapshot().bodies, snapshot.bodies);
    world.stream_around(snapshot.eye);
    physics.sync_world(&world);
    for _ in 0..120 {
        physics.step_objects(FIXED_DT);
    }
    let resumed = physics.snapshot();
    assert!(resumed.bodies[0].position[1] < 12.);
    assert!(resumed.bodies[0].position[1] > -8.);
}
