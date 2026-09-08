//! Cache equivalence/work tests, not mobile performance measurements.
use matterweave_core::{Mesh, World};
use matterweave_physics::{BodySnapshot, DynamicMeshCache, Physics, PhysicsSnapshot, FIXED_DT, MAX_BODIES};

fn body(position: [f32; 3]) -> BodySnapshot {
    BodySnapshot {
        position,
        rotation: [0., 0., 0., 1.],
        velocity: [0.; 3],
        angular_velocity: [0.; 3],
        dimensions: [2; 3],
        material: 8,
    }
}
fn restore(physics: &mut Physics, bodies: Vec<BodySnapshot>) {
    physics.restore(&PhysicsSnapshot { version: 1, eye: [0., 3., 6.], bodies }).unwrap();
}
fn floor() -> World {
    let mut world = World::new(5);
    for x in -8..8 {
        for z in -8..8 { world.set([x, 0, z], 3); }
    }
    world
}
fn equivalent(actual: &Mesh, reference: &Mesh) {
    assert_eq!(actual.indices, reference.indices);
    assert_eq!(actual.vertices.len(), reference.vertices.len());
    for (a, b) in actual.vertices.iter().zip(&reference.vertices) {
        for (x, y) in a.position.iter().chain(&a.normal).chain(&a.color)
            .zip(b.position.iter().chain(&b.normal).chain(&b.color)) {
            // Stable identical poses may bypass a numerically noisy slerp.
            assert!((x-y).abs() <= 0.0001, "{x} != {y}");
        }
    }
}

#[test]
fn unchanged_snapshot_reuses_geometry_without_mutating_physics() {
    let mut physics = Physics::new(&World::new(5));
    restore(&mut physics, vec![body([-2., 5., -3.])]);
    let snapshot = physics.snapshot();
    let mut cache = DynamicMeshCache::default();
    assert!(cache.update(&physics));
    let vertices = cache.mesh().vertices.as_ptr();
    let indices = cache.mesh().indices.as_ptr();
    for _ in 0..50 {
        assert!(!cache.update(&physics));
        assert_eq!(cache.mesh().vertices.as_ptr(), vertices);
        assert_eq!(cache.mesh().indices.as_ptr(), indices);
    }
    equivalent(cache.mesh(), &physics.dynamic_mesh());
    assert_eq!(physics.snapshot(), snapshot);
}

#[test]
fn fractional_interpolation_changes_without_fixed_step_revision_change() {
    let mut physics = Physics::new(&World::new(5));
    restore(&mut physics, vec![body([0., 5., 0.])]);
    let mut cache = DynamicMeshCache::default();
    cache.update(&physics);
    assert_eq!(physics.step_objects(FIXED_DT), 1);
    assert_eq!(physics.step_objects(FIXED_DT * 0.5), 0);
    assert!(cache.update(&physics));
    equivalent(cache.mesh(), &physics.dynamic_mesh());
    let current_y = physics.snapshot().bodies[0].position[1];
    let expected_min_y = 5. + (current_y - 5.) * 0.5 - 0.5;
    let min_y = cache.mesh().vertices.iter().map(|v| v.position[1]).fold(f32::INFINITY, f32::min);
    assert!((min_y - expected_min_y).abs() < 0.0001);
    let revision = physics.dynamic_mesh().revision;
    assert_eq!(physics.step_objects(FIXED_DT * 0.25), 0);
    assert_eq!(physics.dynamic_mesh().revision, revision);
    assert!(cache.update(&physics), "fixed-step revision is not a render key");
    equivalent(cache.mesh(), &physics.dynamic_mesh());
}

#[test]
fn settled_rotated_body_stays_cached_across_fractional_steps() {
    let world = floor();
    let mut physics = Physics::new(&world);
    let mut b = body([0., 5., 0.]);
    b.rotation = [0., 0.25881904, 0., 0.9659258];
    restore(&mut physics, vec![b]);
    for _ in 0..600 { physics.step_objects(FIXED_DT); }
    assert_eq!(physics.body_activity().sleeping, 1);
    let mut cache = DynamicMeshCache::default();
    assert!(cache.update(&physics));
    for fraction in [0.1, 0.25, 0.3, 0.75, 1., 0.6, 0.125].repeat(5) {
        physics.step_objects(FIXED_DT * fraction);
        assert!(!cache.update(&physics), "settled geometry rebuilt");
        equivalent(cache.mesh(), &physics.dynamic_mesh());
    }
}

#[test]
fn removal_of_support_wakes_and_updates_visible_geometry() {
    let mut world = floor();
    let mut physics = Physics::new(&world);
    restore(&mut physics, vec![body([0., 5., 0.])]);
    for _ in 0..600 { physics.step_objects(FIXED_DT); }
    assert_eq!(physics.body_activity().sleeping, 1);
    let mut cache = DynamicMeshCache::default();
    cache.update(&physics);
    for x in -2..=2 { for z in -2..=2 { world.set([x, 0, z], 0); } }
    physics.sync_world(&world);
    physics.step_objects(FIXED_DT * 1.5);
    assert!(cache.update(&physics));
    equivalent(cache.mesh(), &physics.dynamic_mesh());
}

#[test]
fn restores_invalidate_size_material_pose_and_empty_geometry() {
    let mut physics = Physics::new(&World::new(5));
    let mut cache = DynamicMeshCache::default();
    let mut b = body([0., 5., 0.]);
    restore(&mut physics, vec![b.clone()]);
    assert!(cache.update(&physics));
    b.dimensions = [4, 2, 2];
    restore(&mut physics, vec![b.clone()]);
    assert!(cache.update(&physics));
    equivalent(cache.mesh(), &physics.dynamic_mesh());
    b.material = 7;
    restore(&mut physics, vec![b.clone()]);
    assert!(cache.update(&physics));
    equivalent(cache.mesh(), &physics.dynamic_mesh());
    b.rotation = [0., std::f32::consts::FRAC_1_SQRT_2, 0., std::f32::consts::FRAC_1_SQRT_2];
    restore(&mut physics, vec![b]);
    assert!(cache.update(&physics));
    let min_x = cache.mesh().vertices.iter().map(|v| v.position[0]).fold(f32::INFINITY, f32::min);
    assert!((min_x + 0.5).abs() < 0.0001, "quarter turn swaps unequal box extents");
    equivalent(cache.mesh(), &physics.dynamic_mesh());
    restore(&mut physics, vec![]);
    assert!(cache.update(&physics));
    assert!(cache.mesh().vertices.is_empty() && cache.mesh().indices.is_empty());
    assert!(!cache.update(&physics));
}

#[test]
fn fracture_rebuilds_then_equivalent_snapshot_can_reuse() {
    let world = World::new(5);
    let mut physics = Physics::new(&world);
    restore(&mut physics, vec![body([0., 5., 0.])]);
    let mut cache = DynamicMeshCache::default();
    cache.update(&physics);
    assert!(physics.break_body(&world, [0., 5., 4.], [0., 0., -1.], 8.));
    assert_eq!(physics.body_count(), 8);
    assert!(cache.update(&physics));
    equivalent(cache.mesh(), &physics.dynamic_mesh());
    let snapshot = physics.snapshot();
    physics.restore(&snapshot).unwrap();
    assert!(!cache.update(&physics), "new backend handles alone don't change geometry");
}

#[test]
fn preserved_disabled_bodies_are_not_removed_from_mesh() {
    let mut physics = Physics::new(&World::new(5));
    restore(&mut physics, vec![body([100., 5., 0.])]);
    physics.step_objects(FIXED_DT);
    assert_eq!(physics.body_activity().not_simulated, 1);
    let mut cache = DynamicMeshCache::default();
    assert!(cache.update(&physics));
    assert_eq!(cache.mesh().indices.len(), 36);
    for _ in 0..10 {
        physics.step_objects(FIXED_DT);
        assert!(!cache.update(&physics));
    }
}

#[test]
fn retained_payload_is_bounded_across_full_budget_and_clears() {
    let mut physics = Physics::new(&World::new(5));
    let mut cache = DynamicMeshCache::default();
    for _ in 0..20 {
        restore(&mut physics, (0..MAX_BODIES).map(|i| body([i as f32, 5., 0.])).collect());
        assert!(cache.update(&physics));
        assert!(!cache.update(&physics));
        assert!(cache.retained_bytes() <= 256 * 1024);
        restore(&mut physics, vec![]);
        assert!(cache.update(&physics));
        assert!(cache.retained_bytes() <= 256 * 1024);
    }
}
