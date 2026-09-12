//! D2.2 destruction, mass/inertia, constraints and persistence regressions.
//!
//! Fracture is deterministic (no RNG): the same seeded scene and the same ray
//! always produce the same pieces in the same order. These tests exercise the
//! exact 64-body boundary, mass and per-piece inertia, persistent fixed welds
//! under gravity load, save/load topology round-trips and lifecycle counts.
use matterweave_core::World;
use matterweave_physics::{
    BodySnapshot, ConstraintSnapshot, Physics, PhysicsSave, PhysicsSnapshot, FIXED_DT, MAX_BODIES,
    MAX_CONSTRAINTS, MAX_FRACTURE_PIECES,
};

fn floor() -> World {
    let mut world = World::new(5);
    for x in -32..32 {
        for z in -32..32 {
            world.set([x, 0, z], 3);
        }
    }
    world
}

fn body(position: [f32; 3], dimensions: [u8; 3]) -> BodySnapshot {
    BodySnapshot {
        position,
        rotation: [0.0, 0.0, 0.0, 1.0],
        velocity: [0.0; 3],
        angular_velocity: [0.0; 3],
        dimensions,
        material: 8,
    }
}

fn snapshot(bodies: Vec<BodySnapshot>) -> PhysicsSnapshot {
    PhysicsSnapshot {
        version: 1,
        eye: [0.0, 4.0, 8.0],
        bodies,
    }
}

fn voxels(dimensions: [u8; 3]) -> usize {
    dimensions.iter().map(|&n| n as usize).product()
}

/// Break the body nearest the target along +Z. All destruction fixtures put the
/// rows in the air with the ray approaching from -Z, so no other body is in the way.
fn break_at(physics: &mut Physics, world: &World, target: [f32; 3]) -> bool {
    physics.break_body(
        world,
        [target[0], target[1], target[2] - 3.0],
        [0.0, 0.0, 1.0],
        8.0,
    )
}

/// Deterministic 64-bit LCG. This is the seeded input for the fracture scenes:
/// it does not depend on platform RNG, threads or sampling order.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) as u32
    }
}

/// One seeded scene: generated terrain plus a row of 6 bodies totalling exactly
/// 64 voxels, suspended out of terrain reach so the first break is posed exactly.
fn seeded_scene(seed: u64) -> (World, PhysicsSnapshot) {
    let world = World::generate(seed);
    let mut rng = Lcg(seed ^ 0xD2_0002);
    let mut bodies = Vec::new();
    for (index, dimensions) in [
        [6u8, 2, 2],
        [2, 2, 2],
        [2, 2, 2],
        [2, 2, 2],
        [2, 2, 2],
        [2, 2, 2],
    ]
    .into_iter()
    .enumerate()
    {
        // Jitter keeps the input genuinely seeded while preserving the row order
        // and the air gap the side rays rely on.
        let jitter = 0.05 * rng.next() as f32 / u32::MAX as f32;
        bodies.push(body(
            [-10.0 + index as f32 * 4.0, 30.0 + jitter, 0.5 + jitter],
            dimensions,
        ));
    }
    assert_eq!(
        bodies
            .iter()
            .map(|body| voxels(body.dimensions))
            .sum::<usize>(),
        MAX_BODIES
    );
    (world, snapshot(bodies))
}

#[test]
fn seeded_fracture_piece_count_is_exact_and_reproducible() {
    let (world, seeded) = seeded_scene(20260913);
    let fracture_all = |physics: &mut Physics| {
        let positions: Vec<[f32; 3]> = seeded.bodies.iter().map(|body| body.position).collect();
        for position in positions {
            break_at(physics, &world, position);
        }
    };
    let mut first = Physics::new(&world);
    first.restore(&seeded).unwrap();
    // The [6, 2, 2] beam first: exactly 24 unit pieces from 24 voxels.
    let beam = first
        .snapshot()
        .bodies
        .iter()
        .find(|body| body.dimensions == [6, 2, 2])
        .unwrap()
        .position;
    assert!(break_at(&mut first, &world, beam));
    assert_eq!(first.body_count(), 5 + 24);
    assert_eq!(
        first
            .snapshot()
            .bodies
            .iter()
            .filter(|body| body.dimensions == [1; 3])
            .count(),
        24,
        "the beam produced one unit piece per voxel"
    );
    // Every remaining body is a 2x2x2 cube; the scene ends exactly at the budget.
    fracture_all(&mut first);
    assert_eq!(first.body_count(), MAX_BODIES);
    assert_eq!(first.snapshot().bodies.len(), MAX_BODIES);
    assert!(first
        .snapshot()
        .bodies
        .iter()
        .all(|body| body.dimensions == [1; 3]));

    // A second independent run from the same seeded input is bit-identical.
    let mut second = Physics::new(&world);
    second.restore(&seeded).unwrap();
    fracture_all(&mut second);
    assert_eq!(first.snapshot(), second.snapshot());
    assert_eq!(first.body_count(), second.body_count());
}

#[test]
fn fracture_cap_allows_exactly_sixty_four_and_refuses_sixty_five() {
    // A [2, 1, 1] body holds two voxels, so splitting it adds one body.
    let scene = |extra_body: bool| {
        let mut bodies = vec![body([0.5, 30.0, 0.5], [2, 1, 1])];
        let singles = if extra_body {
            MAX_BODIES - 1
        } else {
            MAX_BODIES - 2
        };
        for index in 0..singles {
            bodies.push(body([4.0 + index as f32 * 1.5, 30.0, 0.5], [1, 1, 1]));
        }
        bodies
    };

    let world = floor();
    let allowed = snapshot(scene(false));
    assert_eq!(allowed.bodies.len(), MAX_BODIES - 1);
    let mut physics = Physics::new(&world);
    physics.restore(&allowed).unwrap();
    assert!(break_at(&mut physics, &world, [0.5, 30.0, 0.5]));
    assert_eq!(
        physics.body_count(),
        MAX_BODIES,
        "63 bodies plus a 2-voxel split land on exactly the cap"
    );
    assert!(
        physics.body_count() - allowed.bodies.len() < MAX_FRACTURE_PIECES,
        "one call adds at most MAX_FRACTURE_PIECES - 1 bodies"
    );

    let refused = snapshot(scene(true));
    assert_eq!(refused.bodies.len(), MAX_BODIES);
    let mut physics = Physics::new(&world);
    physics.restore(&refused).unwrap();
    let before = physics.snapshot();
    assert!(!break_at(&mut physics, &world, [0.5, 30.0, 0.5]));
    assert_eq!(
        physics.snapshot(),
        before,
        "a refused split must not mutate any body"
    );
    // A single voxel can never split further.
    assert!(!break_at(&mut physics, &world, refused.bodies[1].position));
}

#[test]
fn fracture_conserves_mass_and_rebuilds_per_piece_inertia() {
    let world = floor();
    let mut physics = Physics::new(&world);
    physics
        .restore(&snapshot(vec![body([0.5, 30.0, 0.5], [6, 2, 2])]))
        .unwrap();
    // Density 3 kg/m^3: 24 half-metre voxels = 3 m^3 = 9 kg.
    let parent_mass = physics.body_mass(0).unwrap();
    assert!((parent_mass - 9.0).abs() < 1e-4, "{parent_mass}");
    // Cuboid extents 3 x 1 x 1 m; I_x = m/12 (1^2 + 1^2), I_y = I_z = m/12 (3^2 + 1^2).
    let expected_parent = [9.0 / 12.0 * 2.0, 9.0 / 12.0 * 10.0, 9.0 / 12.0 * 10.0];
    let parent_inertia = physics.body_inertia(0).unwrap();
    for (got, want) in parent_inertia.iter().zip(expected_parent) {
        assert!(
            (got - want).abs() < 1e-3,
            "{parent_inertia:?} vs {expected_parent:?}"
        );
    }

    assert!(break_at(&mut physics, &world, [0.5, 30.0, 0.5]));
    assert_eq!(physics.body_count(), 24);
    let total: f32 = (0..physics.body_count())
        .map(|index| physics.body_mass(index).unwrap())
        .sum();
    assert!(
        (total - parent_mass).abs() < 1e-4,
        "mass {total} != parent {parent_mass}"
    );
    // Each piece is its own 0.5 m cube: 0.125 m^3 * 3 = 0.375 kg and
    // I = m/12 * (0.5^2 + 0.5^2) on every principal axis.
    let piece_mass = 0.375;
    let piece_inertia = piece_mass / 12.0 * (0.5f32 * 0.5 + 0.5 * 0.5);
    for index in 0..physics.body_count() {
        let mass = physics.body_mass(index).unwrap();
        assert!((mass - piece_mass).abs() < 1e-5, "piece {index}: {mass}");
        for axis in physics.body_inertia(index).unwrap() {
            assert!(
                (axis - piece_inertia).abs() < 1e-5,
                "piece {index}: {axis} vs {piece_inertia}"
            );
        }
    }
    assert!(physics.body_mass(physics.body_count()).is_none());
    assert!(physics.body_inertia(physics.body_count()).is_none());
}

fn tower() -> (World, Vec<BodySnapshot>) {
    let world = floor();
    (
        world,
        vec![
            body([0.5, 1.51, 0.5], [2, 2, 2]),
            body([0.5, 2.53, 0.5], [2, 2, 2]),
            // 1.5 m of the 3 m beam overlaps the tower; 1.5 m cantilevers.
            body([1.5, 3.55, 0.5], [6, 2, 2]),
        ],
    )
}

fn advance(physics: &mut Physics, frames: usize) {
    for _ in 0..frames {
        physics.step_objects(FIXED_DT);
    }
}

/// Angle between two orientations, independent of quaternion sign.
fn angle_between(a: [f32; 4], b: [f32; 4]) -> f32 {
    let dot = (a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3])
        .abs()
        .clamp(0.0, 1.0);
    2.0 * dot.acos()
}

#[test]
fn constrained_tower_holds_under_gravity_load() {
    let (world, bodies) = tower();
    let mut tested = Physics::new(&world);
    tested.restore(&snapshot(bodies.clone())).unwrap();
    assert!(tested.constrain(0, 1));
    assert!(tested.constrain(1, 2));
    assert!(!tested.constrain(2, 1), "the same pair is already welded");
    assert!(!tested.constrain(1, 1), "a body cannot be welded to itself");
    assert!(!tested.constrain(3, 0), "out-of-range indices are refused");
    assert_eq!(tested.constraint_count(), 2);
    let start: Vec<BodySnapshot> = tested.snapshot().bodies.clone();
    advance(&mut tested, 600);
    assert_eq!(tested.constraint_count(), 2);
    assert_eq!(tested.body_count(), 3);
    let after = tested.snapshot();
    for (index, pose) in after.bodies.iter().enumerate() {
        assert!(
            pose.position.iter().all(|value| value.is_finite()),
            "{pose:?}"
        );
        let speed = pose.velocity.iter().map(|v| v * v).sum::<f32>().sqrt();
        let angular = pose
            .angular_velocity
            .iter()
            .map(|v| v * v)
            .sum::<f32>()
            .sqrt();
        assert!(speed < 20.0, "body {index} exploded: {speed}");
        assert!(angular < 20.0, "body {index} spun up: {angular}");
    }
    let distance = |a: [f32; 3], b: [f32; 3]| {
        ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
    };
    for (index, (a, b)) in [(0, 1), (1, 2)].into_iter().enumerate() {
        let drift = distance(after.bodies[a].position, after.bodies[b].position)
            - distance(start[a].position, start[b].position);
        assert!(drift.abs() < 0.05, "weld {index} drifted {drift} m");
        // A fixed weld locks rotation as well as translation.
        let turned = angle_between(after.bodies[a].rotation, after.bodies[b].rotation)
            - angle_between(start[a].rotation, start[b].rotation);
        assert!(turned.abs() < 0.05, "weld {index} rotated {turned} rad");
    }

    // Control: the same load without welds separates far beyond that tolerance,
    // so the test discriminates a held assembly from a loose one.
    let mut loose = Physics::new(&world);
    loose.restore(&snapshot(bodies)).unwrap();
    advance(&mut loose, 600);
    let loose_poses = loose.snapshot();
    let separation = distance(
        loose_poses.bodies[1].position,
        loose_poses.bodies[2].position,
    );
    let welded = distance(after.bodies[1].position, after.bodies[2].position);
    assert!(
        separation - welded > 0.3,
        "unwelded bodies must separate ({separation} vs {welded})"
    );
}

#[test]
fn fracturing_a_constrained_body_retires_only_its_welds() {
    let world = floor();
    let mut physics = Physics::new(&world);
    physics
        .restore(&snapshot(vec![
            body([0.5, 30.0, 0.5], [2, 2, 2]),
            body([2.5, 30.0, 0.5], [6, 2, 2]),
            body([5.5, 30.0, 0.5], [2, 2, 2]),
        ]))
        .unwrap();
    assert!(physics.constrain(0, 1));
    assert!(physics.constrain(1, 2));
    assert!(physics.constrain(0, 2));
    assert!(break_at(&mut physics, &world, [2.5, 30.0, 0.5]));
    assert_eq!(physics.body_count(), 2 + 24);
    assert_eq!(
        physics.constraint_count(),
        1,
        "welds touching the fractured body retire with it; the unrelated pair survives"
    );
    let save = physics.save();
    assert_eq!(
        save.constraints,
        vec![ConstraintSnapshot {
            first: 0,
            second: 1
        }]
    );
    let mut reloaded = Physics::new(&world);
    reloaded.load(&save).unwrap();
    assert_eq!(reloaded.constraint_count(), 1);
    advance(&mut reloaded, 120);
    assert!(reloaded
        .snapshot()
        .bodies
        .iter()
        .all(|body| body.position.iter().all(|value| value.is_finite())));
}

#[test]
fn save_and_load_round_trip_bodies_velocities_and_topology() {
    let (world, mut bodies) = tower();
    bodies[2].velocity = [0.5, -0.25, 0.125];
    bodies[2].angular_velocity = [0.1, -0.2, 0.3];
    let mut physics = Physics::new(&world);
    physics.restore(&snapshot(bodies)).unwrap();
    assert!(physics.constrain(0, 1));
    assert!(physics.constrain(1, 2));
    let save = physics.save();
    assert_eq!(save.version, 1);
    assert_eq!(save.constraints.len(), 2);

    // JSON is the on-disk representation; the decoded save equals the live one.
    let json = serde_json::to_string(&save).unwrap();
    let decoded: PhysicsSave = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, save);

    let mut loaded = Physics::new(&world);
    loaded.load(&decoded).unwrap();
    let reloaded = loaded.save();
    assert_eq!(reloaded.constraints, save.constraints);
    assert_eq!(reloaded.snapshot.bodies.len(), save.snapshot.bodies.len());
    for (before, after) in reloaded
        .snapshot
        .bodies
        .iter()
        .zip(save.snapshot.bodies.iter())
    {
        for axis in 0..3 {
            assert!((before.position[axis] - after.position[axis]).abs() < 1e-5);
            assert!((before.velocity[axis] - after.velocity[axis]).abs() < 1e-5);
            assert!((before.angular_velocity[axis] - after.angular_velocity[axis]).abs() < 1e-5);
        }
        for axis in 0..4 {
            assert!((before.rotation[axis] - after.rotation[axis]).abs() < 1e-5);
        }
        assert_eq!(before.dimensions, after.dimensions);
        assert_eq!(before.material, after.material);
    }
    // The restored topology is live, not just recorded: stepping moves both the same way.
    let mut original = Physics::new(&world);
    original.load(&save).unwrap();
    advance(&mut loaded, 120);
    advance(&mut original, 120);
    let after_loaded = loaded.save();
    let after_original = original.save();
    assert_eq!(after_loaded.constraints, after_original.constraints);
    for (a, b) in after_loaded
        .snapshot
        .bodies
        .iter()
        .zip(after_original.snapshot.bodies.iter())
    {
        for axis in 0..3 {
            assert!(
                (a.position[axis] - b.position[axis]).abs() < 1e-4,
                "simulation diverged"
            );
        }
    }
}

#[test]
fn invalid_saves_are_rejected_without_mutating_live_state() {
    let (world, bodies) = tower();
    let mut physics = Physics::new(&world);
    physics.restore(&snapshot(bodies)).unwrap();
    assert!(physics.constrain(0, 1));
    let live = physics.save();
    let mut rejected = |save: &PhysicsSave| {
        assert!(physics.load(save).is_err());
        assert_eq!(physics.save(), live, "rejected load mutated live state");
    };
    let mut save = live.clone();
    save.version = 0;
    rejected(&save);
    save = live.clone();
    save.snapshot.version = 2;
    rejected(&save);
    save = live.clone();
    save.constraints.push(ConstraintSnapshot {
        first: 0,
        second: 9,
    });
    rejected(&save);
    save = live.clone();
    save.constraints.push(ConstraintSnapshot {
        first: 1,
        second: 1,
    });
    rejected(&save);
    save = live.clone();
    save.constraints.push(ConstraintSnapshot {
        first: 1,
        second: 0,
    });
    rejected(&save);
    save = live.clone();
    save.constraints = (0..MAX_CONSTRAINTS + 1)
        .map(|index| ConstraintSnapshot {
            first: (index % 3) as u32,
            second: ((index / 3) % 3) as u32,
        })
        .collect();
    rejected(&save);
    // The original is still loadable after all refusals.
    let mut loaded = Physics::new(&world);
    loaded.load(&live).unwrap();
    assert_eq!(loaded.constraint_count(), 1);
}

#[test]
fn constraint_cap_refuses_the_next_weld() {
    let world = floor();
    let mut physics = Physics::new(&world);
    let bodies: Vec<BodySnapshot> = (0..MAX_BODIES)
        .map(|index| {
            body(
                [(index % 8) as f32 * 2.0, 30.0, (index / 8) as f32 * 2.0],
                [1, 1, 1],
            )
        })
        .collect();
    physics.restore(&snapshot(bodies)).unwrap();
    let mut welded = 0;
    'outer: for first in 0..MAX_BODIES {
        for second in first + 1..MAX_BODIES {
            if welded == MAX_CONSTRAINTS {
                break 'outer;
            }
            assert!(physics.constrain(first, second));
            welded += 1;
        }
    }
    assert_eq!(physics.constraint_count(), MAX_CONSTRAINTS);
    assert!(
        !physics.constrain(3, 4),
        "the cap refuses an otherwise valid unwelded pair"
    );
    assert_eq!(physics.save().constraints.len(), MAX_CONSTRAINTS);
}

#[test]
fn twenty_lifecycle_reset_load_cycles_do_not_leak() {
    let (world, bodies) = tower();
    let mut physics = Physics::new(&world);
    physics.restore(&snapshot(bodies)).unwrap();
    assert!(physics.constrain(0, 1));
    assert!(physics.constrain(1, 2));
    let save = physics.save();
    let baseline = (
        physics.body_count(),
        physics.collider_count(),
        physics.constraint_count(),
    );
    assert_eq!(baseline.0, 3);
    // One character collider, 16 terrain chunks, three voxel bodies.
    assert_eq!(baseline.1, 1 + 16 + 3);
    assert_eq!(baseline.2, 2);
    for cycle in 0..20 {
        // Fracture first, while the restored poses are still exact, then simulate
        // the pieces and reset through a save/load boundary.
        assert!(break_at(&mut physics, &world, [0.5, 1.51, 0.5]));
        assert_eq!(
            physics.body_count(),
            2 + 8,
            "cycle {cycle}: the fracture did not add bodies"
        );
        assert_eq!(
            physics.constraint_count(),
            1,
            "cycle {cycle}: one weld retired"
        );
        advance(&mut physics, 30);
        assert!(physics.set_flying_eye([3.0 + cycle as f32, 6.0, 3.0]));
        physics.load(&save).unwrap();
        assert_eq!(
            (
                physics.body_count(),
                physics.collider_count(),
                physics.constraint_count()
            ),
            baseline,
            "cycle {cycle}: counts must return to the baseline"
        );
        let restored = physics.save();
        assert_eq!(restored.constraints, save.constraints);
        for (before, after) in restored
            .snapshot
            .bodies
            .iter()
            .zip(save.snapshot.bodies.iter())
        {
            for axis in 0..3 {
                assert!(
                    (before.position[axis] - after.position[axis]).abs() < 1e-5,
                    "cycle {cycle}: pose drifted"
                );
            }
        }
    }
    // The world is still usable after the cycles, not just countable.
    advance(&mut physics, 120);
    assert_eq!(physics.constraint_count(), 2);
    assert!(physics
        .snapshot()
        .bodies
        .iter()
        .all(|body| body.position.iter().all(|value| value.is_finite())));
}
