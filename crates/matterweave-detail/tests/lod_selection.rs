//! Automatic view-dependent LOD selection: approach/retreat/zoom, orthographic,
//! negative coordinates, invalid inputs, hysteresis, edit invalidation, source
//! invariance, thin-feature preservation and memory/work budgets.

use matterweave_detail::{
    material, Camera, DetailScene, DetailVolume, Lod, LodConfig, Projection, Scale, Transform, Yaw,
    SCALE_FINE_M,
};

fn place_at(x: f32, y: f32, z: f32) -> Transform {
    Transform::new([x, y, z], Yaw::Deg0).unwrap()
}

const FOV60: f32 = std::f32::consts::PI / 3.0;

fn solid_block(id: &str, edge: i32, mat: u8) -> DetailVolume {
    let mut v = DetailVolume::new(id, Scale::new(SCALE_FINE_M).unwrap());
    for x in 0..edge {
        for y in 0..edge {
            for z in 0..edge {
                v.set([x, y, z], mat).unwrap();
            }
        }
    }
    v
}

/// A one-cell-thick sheet: coarsening any-occupied fills mostly air, driving the
/// dilation fraction high, so the thin-feature guard holds it at Source.
fn thin_sheet(id: &str, mat: u8) -> DetailVolume {
    let mut v = DetailVolume::new(id, Scale::new(SCALE_FINE_M).unwrap());
    for x in 0..16 {
        for y in 0..32 {
            v.set([x, y, 0], mat).unwrap();
        }
    }
    v
}

/// Perspective camera looking down -Z so a block at the origin sits `distance`
/// metres away along +Z. The block spans 0..0.5 m; eye x/y sit inside that span.
fn persp(distance: f32) -> Camera {
    Camera {
        eye_m: [0.25, 0.25, 0.5 + distance],
        forward_m: [0.0, 0.0, -1.0],
        viewport_height_px: 1080.0,
        near_m: 0.1,
        projection: Projection::Perspective {
            vertical_fov_rad: FOV60,
        },
    }
}

fn ortho(distance: f32, view_height_m: f32) -> Camera {
    Camera {
        eye_m: [0.25, 0.25, 0.5 + distance],
        forward_m: [0.0, 0.0, -1.0],
        viewport_height_px: 1080.0,
        near_m: 0.1,
        projection: Projection::Orthographic { view_height_m },
    }
}

fn scene_with_block() -> DetailScene {
    let mut scene = DetailScene::new();
    scene
        .add_prototype(solid_block("block", 8, material::BANK_STONE))
        .unwrap();
    scene.place("b", "block", Transform::identity()).unwrap();
    scene
}

fn lod_at(scene: &mut DetailScene, camera: &Camera, config: &LodConfig) -> Lod {
    let selected = scene.select_lods(camera, config).unwrap();
    selected[0].lod
}

#[test]
fn perspective_approach_and_retreat_move_between_source_and_coarse_levels() {
    let config = LodConfig::default();
    // Fresh scene per distance isolates the ideal curve from hysteresis history.
    assert_eq!(
        lod_at(&mut scene_with_block(), &persp(1.0), &config),
        Lod::Source
    );
    assert_eq!(
        lod_at(&mut scene_with_block(), &persp(90.0), &config),
        Lod::Half
    );
    assert_eq!(
        lod_at(&mut scene_with_block(), &persp(300.0), &config),
        Lod::Quarter
    );
}

#[test]
fn orthographic_zoom_selects_finer_when_zoomed_in() {
    let config = LodConfig::default();
    // Zoomed out (large world view height): object is small on screen -> coarse.
    assert_eq!(
        lod_at(&mut scene_with_block(), &ortho(10.0, 200.0), &config),
        Lod::Quarter
    );
    // Zoomed in (small view height): object is large -> authoritative source.
    assert_eq!(
        lod_at(&mut scene_with_block(), &ortho(10.0, 5.0), &config),
        Lod::Source
    );
}

#[test]
fn orthographic_selection_is_distance_invariant() {
    let config = LodConfig::default();
    // Projected size is independent of distance under orthographic projection,
    // so moving the eye without changing the view height must not change the LOD.
    let near = lod_at(&mut scene_with_block(), &ortho(5.0, 200.0), &config);
    let far = lod_at(&mut scene_with_block(), &ortho(5000.0, 200.0), &config);
    assert_eq!(near, far);
    assert_eq!(near, Lod::Quarter);
}

#[test]
fn negative_world_placement_selects_by_distance_like_positive() {
    let config = LodConfig::default();
    let mut scene = DetailScene::new();
    scene
        .add_prototype(solid_block("block", 8, material::BANK_STONE))
        .unwrap();
    // Place far into negative world coordinates.
    scene
        .place("n", "block", place_at(-500.0, -500.0, -500.0))
        .unwrap();
    // Eye near the negative instance -> Source; eye far away -> coarse.
    let near_eye = Camera {
        eye_m: [-499.75, -499.75, -499.0],
        ..persp(1.0)
    };
    let far_eye = Camera {
        eye_m: [-499.75, -499.75, -200.0],
        ..persp(1.0)
    };
    assert_eq!(lod_at(&mut scene, &near_eye, &config), Lod::Source);
    assert_eq!(lod_at(&mut scene, &far_eye, &config), Lod::Quarter);
}

#[test]
fn invalid_cameras_and_configs_are_rejected() {
    let mut scene = scene_with_block();
    let config = LodConfig::default();
    let bad_cameras = [
        Camera {
            eye_m: [f32::NAN, 0.0, 0.0],
            ..persp(1.0)
        },
        Camera {
            viewport_height_px: 0.0,
            ..persp(1.0)
        },
        Camera {
            near_m: 0.0,
            ..persp(1.0)
        },
        Camera {
            projection: Projection::Perspective {
                vertical_fov_rad: 0.0,
            },
            ..persp(1.0)
        },
        Camera {
            projection: Projection::Perspective {
                vertical_fov_rad: std::f32::consts::PI,
            },
            ..persp(1.0)
        },
        Camera {
            projection: Projection::Orthographic { view_height_m: 0.0 },
            ..persp(1.0)
        },
    ];
    for camera in bad_cameras {
        assert!(scene.select_lods(&camera, &config).is_err());
    }
    let bad_configs = [
        LodConfig {
            error_budget_px: 0.0,
            ..config
        },
        LodConfig {
            hysteresis: -0.1,
            ..config
        },
        LodConfig {
            max_dilation_fraction: 1.5,
            ..config
        },
    ];
    for config in bad_configs {
        assert!(scene.select_lods(&persp(1.0), &config).is_err());
    }
}

#[test]
fn eye_inside_bounds_resolves_to_source() {
    let config = LodConfig::default();
    let mut scene = scene_with_block();
    let inside = Camera {
        eye_m: [0.25, 0.25, 0.25],
        ..persp(1.0)
    };
    assert_eq!(lod_at(&mut scene, &inside, &config), Lod::Source);
}

#[test]
fn hysteresis_holds_a_level_across_the_boundary_and_prevents_chatter() {
    let config = LodConfig::default();
    let mut scene = scene_with_block();
    // Settle at Half.
    assert_eq!(lod_at(&mut scene, &persp(90.0), &config), Lod::Half);
    // d=70 is inside the dead-band: a scene already at Half keeps Half...
    assert_eq!(lod_at(&mut scene, &persp(70.0), &config), Lod::Half);
    // ...while a fresh scene at the same distance chooses the finer Source.
    assert_eq!(
        lod_at(&mut scene_with_block(), &persp(70.0), &config),
        Lod::Source
    );
    // Rapid reversal across the boundary must not flip the level.
    for _ in 0..8 {
        assert_eq!(lod_at(&mut scene, &persp(90.0), &config), Lod::Half);
        assert_eq!(lod_at(&mut scene, &persp(70.0), &config), Lod::Half);
    }
}

#[test]
fn thin_features_stay_at_source_while_solid_bodies_coarsen() {
    let config = LodConfig::default();
    let mut scene = DetailScene::new();
    scene
        .add_prototype(solid_block("block", 8, material::BANK_STONE))
        .unwrap();
    scene
        .add_prototype(thin_sheet("sheet", material::BANK_STONE))
        .unwrap();
    scene.place("b", "block", Transform::identity()).unwrap();
    scene.place("s", "sheet", place_at(10.0, 0.0, 0.0)).unwrap();
    let camera = persp(300.0);
    let selected = scene.select_lods(&camera, &config).unwrap();
    let block = selected.iter().find(|s| s.instance == "b").unwrap();
    let sheet = selected.iter().find(|s| s.instance == "s").unwrap();
    assert_eq!(block.lod, Lod::Quarter, "solid body coarsens far away");
    assert_eq!(
        sheet.lod,
        Lod::Source,
        "thin sheet held at source silhouette"
    );

    // Control: relaxing the guard lets the same thin sheet coarsen, proving the
    // guard, not distance, held it at Source.
    let relaxed = LodConfig {
        max_dilation_fraction: 0.95,
        ..config
    };
    let selected = scene.select_lods(&camera, &relaxed).unwrap();
    let sheet = selected.iter().find(|s| s.instance == "s").unwrap();
    assert!(sheet.lod > Lod::Source, "relaxed guard allows coarsening");
}

#[test]
fn quality_cap_and_disable_bound_the_selected_level() {
    let mut scene = scene_with_block();
    let camera = persp(300.0);
    // Disabled -> always Source.
    assert_eq!(
        lod_at(&mut scene, &camera, &LodConfig::disabled()),
        Lod::Source
    );
    // Cap at Half -> never Quarter even very far.
    let capped = LodConfig {
        max_lod: Lod::Half,
        ..LodConfig::default()
    };
    assert_eq!(lod_at(&mut scene_with_block(), &camera, &capped), Lod::Half);
}

#[test]
fn selection_reports_error_and_projected_error_evidence() {
    let config = LodConfig::default();
    let mut scene = scene_with_block();
    let selected = scene.select_lods(&persp(300.0), &config).unwrap();
    let item = &selected[0];
    assert_eq!(item.lod, Lod::Quarter);
    // Quarter of a 0.0625 m source is a 0.25 m coarse-cell error estimate.
    assert!((item.error_estimate_m - 0.25).abs() < 1e-4);
    assert!(
        item.projected_error_estimate_px > 0.0
            && item.projected_error_estimate_px <= config.error_budget_px
    );
    assert!(item.depth_m > 299.0 && item.depth_m < 301.0);
    // Source cell count is camera independent identity, always the finest count.
    assert_eq!(item.occupied_cells, 8 * 8 * 8);
    assert!(!item.fallback);
}

#[test]
fn prepare_batches_builds_meshes_once_and_reuses_the_cache() {
    let config = LodConfig::default();
    let mut scene = scene_with_block();
    let camera = persp(300.0);
    let frame = scene.prepare_batches(&camera, &config).unwrap();
    assert_eq!(frame.batches.len(), 1);
    let batch = &frame.batches[0];
    assert_eq!(batch.prototype, "block");
    assert_eq!(batch.lod, Lod::Quarter);
    assert!(batch.mesh_bytes > 0);
    assert_eq!(batch.instances.len(), 1);
    assert!(frame.mesh_builds_this_call >= 1);
    assert!(scene.cached_prototype_mesh("block", Lod::Quarter).is_some());
    assert_eq!(frame.source_version, scene.source_version());

    // Same camera again: cache is reused, no rebuild, no per-frame census.
    let again = scene.prepare_batches(&camera, &config).unwrap();
    assert_eq!(again.mesh_builds_this_call, 0);
}

#[test]
fn edits_invalidate_stale_lod_meshes_and_advance_the_source_version() {
    let config = LodConfig::default();
    let mut scene = scene_with_block();
    let camera = persp(300.0);
    let before = scene.prepare_batches(&camera, &config).unwrap();
    let before_version = before.source_version.clone();
    let before_bytes = before.batches[0].mesh_bytes;

    // Edit source: adds an isolated cell, changing the derived geometry.
    scene
        .edit_prototype("block", [20, 20, 20], material::BANK_STONE)
        .unwrap();
    assert_ne!(scene.source_version(), before_version);

    let after = scene.prepare_batches(&camera, &config).unwrap();
    assert!(
        after.mesh_builds_this_call >= 1,
        "stale LOD mesh rebuilt after edit"
    );
    assert_ne!(after.batches[0].mesh_bytes, before_bytes);
}

#[test]
fn source_collision_and_queries_are_invariant_under_camera_changes() {
    let config = LodConfig::default();
    let mut scene = scene_with_block();
    let point = [0.1, 0.1, 0.1];
    let collide0 = scene.is_collidable_world_metres(point).unwrap();
    let sample0 = scene.sample_world_metres(point).unwrap();
    let counts0 = scene.counts();
    assert!(collide0);

    for distance in [1.0, 45.0, 90.0, 300.0, 5.0, 500.0] {
        scene.prepare_batches(&persp(distance), &config).unwrap();
        assert_eq!(scene.is_collidable_world_metres(point).unwrap(), collide0);
        assert_eq!(scene.sample_world_metres(point).unwrap(), sample0);
        let counts = scene.counts();
        assert_eq!(counts.source_bytes, counts0.source_bytes);
        assert_eq!(counts.unique_stored_cells, counts0.unique_stored_cells);
        assert_eq!(
            counts.expanded_collision_cells,
            counts0.expanded_collision_cells
        );
    }
}

#[test]
fn build_cap_falls_back_to_source_without_extra_coarse_builds() {
    let mut scene = DetailScene::new();
    scene
        .add_prototype(solid_block("a", 8, material::BANK_STONE))
        .unwrap();
    scene
        .add_prototype(solid_block("b", 8, material::BANK_STONE))
        .unwrap();
    scene.place("ia", "a", Transform::identity()).unwrap();
    scene.place("ib", "b", place_at(10.0, 0.0, 0.0)).unwrap();
    let camera = persp(300.0);
    // No coarse builds permitted: every instance retreats to the Source mesh.
    let config = LodConfig {
        max_coarse_builds: Some(0),
        ..LodConfig::default()
    };
    let frame = scene.prepare_batches(&camera, &config).unwrap();
    for item in &frame.selected {
        assert_eq!(item.lod, Lod::Source);
        assert!(item.fallback);
    }
    // Only Source meshes were built; no Quarter mesh is resident.
    assert!(scene.cached_prototype_mesh("a", Lod::Quarter).is_none());
    assert!(scene.cached_prototype_mesh("a", Lod::Source).is_some());
}

#[test]
fn selection_is_deterministic_for_identical_inputs() {
    let config = LodConfig::default();
    let mut a = scene_with_block();
    let mut b = scene_with_block();
    let camera = persp(120.0);
    let sa = a.select_lods(&camera, &config).unwrap();
    let sb = b.select_lods(&camera, &config).unwrap();
    assert_eq!(sa, sb);
}
