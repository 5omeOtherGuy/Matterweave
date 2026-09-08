use matterweave_detail::*;

fn fine() -> Scale {
    Scale::new(SCALE_FINE_M).unwrap()
}
fn tile() -> Scale {
    Scale::new(SCALE_TILE_M).unwrap()
}

#[test]
fn scale_validation_rejects_nonfinite_zero_and_oversized() {
    for bad in [0.0, -0.25, f32::NAN, f32::INFINITY, 2.0] {
        assert_eq!(Scale::new(bad), Err(DetailError::InvalidScale), "{bad}");
    }
    assert_eq!(fine().metres(), 0.0625);
    assert_eq!(tile().metres(), 0.25);
}

#[test]
fn negative_cells_round_trip_through_edit_and_query() {
    let mut volume = DetailVolume::new("neg", fine());
    assert!(volume.set([-5, -3, -17], material::MOSS_TURF).unwrap());
    assert_eq!(volume.get([-5, -3, -17]), material::MOSS_TURF);
    assert_eq!(volume.get([-5, -3, -16]), material::AIR);
    assert_eq!(volume.occupied_cells(), 1);
    let bounds = volume.bounds_local().unwrap();
    assert_eq!(bounds.min, [-5.0 * 0.0625, -3.0 * 0.0625, -17.0 * 0.0625]);
    assert_eq!(bounds.max, [-4.0 * 0.0625, -2.0 * 0.0625, -16.0 * 0.0625]);
    assert_eq!(
        volume.set([MAX_CELL_COORD + 1, 0, 0], 1),
        Err(DetailError::CellOutOfRange)
    );
}

#[test]
fn transformed_boundary_queries_are_exact_at_two_scales() {
    for (scale, cell) in [(fine(), [-1, 0, -1]), (tile(), [-1, 0, -1])] {
        let s = scale.metres();
        let mut volume = DetailVolume::new("t", scale);
        volume.set(cell, material::BANK_STONE).unwrap();
        let transform = Transform::new([10.0, 0.0, -4.0], Yaw::Deg90).unwrap();
        // Cell [-1,0,-1] spans local [-s,0) on x and z; yaw 90 maps (x,z)->(z,-x).
        let inside = transform.point_to_world([-s * 0.5, s * 0.5, -s * 0.5]);
        assert_eq!(
            volume.sample_world_metres(&transform, inside).unwrap(),
            material::BANK_STONE
        );
        // Just outside the +x local face is air.
        let outside = transform.point_to_world([s * 0.5, s * 0.5, -s * 0.5]);
        assert_eq!(
            volume.sample_world_metres(&transform, outside).unwrap(),
            material::AIR
        );
        let world = volume.bounds_world(&transform).unwrap().unwrap();
        assert!((world.min[0] - (10.0 - s)).abs() < 1e-6, "{world:?}");
        assert!((world.max[1] - s).abs() < 1e-6);
    }
}

#[test]
fn transform_round_trips_and_rejects_invalid_translation() {
    let transform = Transform::new([3.0, -2.0, 7.5], Yaw::Deg270).unwrap();
    let point = [0.25, 1.5, -0.75];
    let back = transform.point_to_local(transform.point_to_world(point));
    for axis in 0..3 {
        assert!((back[axis] - point[axis]).abs() < 1e-6);
    }
    assert_eq!(
        Transform::new([f32::NAN, 0.0, 0.0], Yaw::Deg0),
        Err(DetailError::InvalidTransform)
    );
    assert_eq!(
        Transform::new([MAX_SCENE_TRANSLATION_M * 2.0, 0.0, 0.0], Yaw::Deg0),
        Err(DetailError::InvalidTransform)
    );
}

#[test]
fn greedy_mesh_removes_shared_faces_across_chunk_seams_with_outward_winding() {
    // A 2x1x1-cell slab straddling the chunk boundary at x = 16.
    let mut volume = DetailVolume::new("seam", tile());
    volume.set([15, 0, 0], material::BANK_STONE).unwrap();
    volume.set([16, 0, 0], material::BANK_STONE).unwrap();
    let mesh = volume.mesh_local().unwrap();
    // Ten exposed unit faces, never twelve: the shared seam face is removed.
    assert_eq!(mesh.indices.len() / 6, 10);
    assert_eq!(mesh.vertices.len(), 40);
    let s = SCALE_TILE_M;
    assert!(mesh
        .vertices
        .iter()
        .all(|v| (v.position[0] - 16.0 * s).abs() > 1e-6 || v.normal[0] == 0.0));
    for triangle in mesh.indices.chunks(3) {
        let p: Vec<[f32; 3]> = triangle
            .iter()
            .map(|&i| mesh.vertices[i as usize].position)
            .collect();
        let e1: [f32; 3] = std::array::from_fn(|a| p[1][a] - p[0][a]);
        let e2: [f32; 3] = std::array::from_fn(|a| p[2][a] - p[0][a]);
        let cross = [
            e1[1] * e2[2] - e1[2] * e2[1],
            e1[2] * e2[0] - e1[0] * e2[2],
            e1[0] * e2[1] - e1[1] * e2[0],
        ];
        let normal = mesh.vertices[triangle[0] as usize].normal;
        let dot: f32 = (0..3).map(|a| cross[a] * normal[a]).sum();
        assert!(dot > 0.0, "counter-clockwise winding faces outwards");
    }
}

#[test]
fn mesh_world_preserves_winding_and_uses_crate_palette() {
    let mut volume = DetailVolume::new("w", fine());
    volume.set([-2, 0, 3], material::MUSHROOM_CAP).unwrap();
    let transform = Transform::new([1.0, 2.0, 3.0], Yaw::Deg180).unwrap();
    let local = volume.mesh_local().unwrap();
    let world = volume.mesh_world(&transform).unwrap();
    assert_eq!(local.indices, world.indices);
    assert!(local
        .vertices
        .iter()
        .all(|v| v.color == material_color(material::MUSHROOM_CAP)));
    for (l, w) in local.vertices.iter().zip(&world.vertices) {
        let expected = transform.point_to_world(l.position);
        for (axis, value) in expected.iter().enumerate() {
            assert!((w.position[axis] - value).abs() < 1e-6);
        }
    }
}

#[test]
fn edits_change_revision_and_invalidate_derived_cache() {
    let mut scene = DetailScene::new();
    scene
        .add_prototype(parasol_mushroom("cap").unwrap())
        .unwrap();
    let before = scene.prototype("cap").unwrap().revision();
    let vertices = scene
        .prototype_mesh("cap", Lod::Source)
        .unwrap()
        .vertices
        .len();
    assert_eq!(scene.counts().mesh_builds, 1);
    scene.prototype_mesh("cap", Lod::Source).unwrap();
    assert_eq!(scene.counts().mesh_builds, 1, "cached mesh reused");

    assert!(scene
        .edit_prototype("cap", [0, 30, 0], material::MUSHROOM_CAP)
        .unwrap());
    assert!(scene.prototype("cap").unwrap().revision() > before);
    let after = scene
        .prototype_mesh("cap", Lod::Source)
        .unwrap()
        .vertices
        .len();
    assert_eq!(scene.counts().mesh_builds, 2, "edit invalidated the cache");
    assert!(after > vertices);
}

#[test]
fn snapshot_round_trip_is_deterministic_and_bounded() {
    let volume = terrain_detail_tile("tile", 42).unwrap();
    let snapshot = volume.snapshot();
    assert!(snapshot.runs.len() <= MAX_SNAPSHOT_RUNS);
    assert!(
        snapshot.runs.len() < snapshot.occupied_cells,
        "runs compress"
    );
    let json = serde_json::to_vec(&snapshot).unwrap();
    let decoded: VolumeSnapshot = serde_json::from_slice(&json).unwrap();
    assert_eq!(decoded, snapshot);
    let restored = DetailVolume::from_snapshot(&decoded).unwrap();
    assert_eq!(restored.occupied_cells(), volume.occupied_cells());
    assert_eq!(restored.cell_bounds(), volume.cell_bounds());
    assert_eq!(restored.snapshot().runs, snapshot.runs);
    let (a, b) = (volume.mesh_local().unwrap(), restored.mesh_local().unwrap());
    assert_eq!(
        bytemuck::cast_slice::<_, u8>(&a.vertices),
        bytemuck::cast_slice::<_, u8>(&b.vertices)
    );
    assert_eq!(a.indices, b.indices);
}

#[test]
fn malformed_snapshots_are_rejected() {
    let mut volume = DetailVolume::new("m", tile());
    volume.set([0, 0, 0], material::WATER).unwrap();
    let good = volume.snapshot();

    let mut bad = good.clone();
    bad.version += 1;
    assert!(DetailVolume::from_snapshot(&bad).is_err());

    let mut bad = good.clone();
    bad.scale_m = 0.0;
    assert_eq!(
        DetailVolume::from_snapshot(&bad).err(),
        Some(DetailError::InvalidScale)
    );

    let mut bad = good.clone();
    bad.runs[0][3] = 0;
    assert!(DetailVolume::from_snapshot(&bad).is_err());

    let mut bad = good.clone();
    bad.runs[0][4] = 999;
    assert!(DetailVolume::from_snapshot(&bad).is_err());

    let mut bad = good.clone();
    bad.runs[0][0] = i32::MAX;
    assert!(DetailVolume::from_snapshot(&bad).is_err());

    let mut bad = good.clone();
    bad.occupied_cells = 7;
    assert!(DetailVolume::from_snapshot(&bad).is_err());

    let mut bad = good;
    bad.runs.push([0, 0, 0, 1, 13]); // overlapping duplicate run
    assert!(DetailVolume::from_snapshot(&bad).is_err());
}

#[test]
fn derived_lod_preserves_the_source_volume() {
    let source = parasol_mushroom("shroom").unwrap();
    let before_cells = source.occupied_cells();
    let before_revision = source.revision();
    let before_bounds = source.cell_bounds();
    let before_snapshot = source.snapshot();

    let half = source.coarsen(Lod::Half).unwrap();
    let quarter = source.coarsen(Lod::Quarter).unwrap();

    assert_eq!(source.occupied_cells(), before_cells);
    assert_eq!(source.revision(), before_revision);
    assert_eq!(source.cell_bounds(), before_bounds);
    assert_eq!(source.snapshot(), before_snapshot);

    assert_eq!(half.scale().metres(), SCALE_FINE_M * 2.0);
    assert_eq!(quarter.scale().metres(), SCALE_FINE_M * 4.0);
    assert!(half.occupied_cells() < before_cells);
    assert!(quarter.occupied_cells() < half.occupied_cells());
    // Coarse levels still cover roughly the same metre extent.
    let (sb, qb) = (
        source.bounds_local().unwrap(),
        quarter.bounds_local().unwrap(),
    );
    for axis in 0..3 {
        assert!((qb.max[axis] - sb.max[axis]).abs() <= SCALE_FINE_M * 4.0);
    }
    assert_eq!(
        source.coarsen(Lod::Half).unwrap().snapshot(),
        half.snapshot()
    );
}

#[test]
fn scene_rejects_duplicate_unknown_and_invalid_identities() {
    let mut scene = DetailScene::new();
    scene
        .add_prototype(parasol_mushroom("shroom").unwrap())
        .unwrap();
    assert_eq!(
        scene.add_prototype(parasol_mushroom("shroom").unwrap()),
        Err(DetailError::DuplicatePrototype("shroom".into()))
    );
    scene.place("a", "shroom", Transform::identity()).unwrap();
    assert_eq!(
        scene.place("a", "shroom", Transform::identity()),
        Err(DetailError::DuplicateInstance("a".into()))
    );
    assert_eq!(
        scene.place("b", "missing", Transform::identity()),
        Err(DetailError::UnknownPrototype("missing".into()))
    );
    let invalid = Transform {
        translation_m: [f32::INFINITY, 0.0, 0.0],
        yaw: Yaw::Deg0,
    };
    assert_eq!(
        scene.place("c", "shroom", invalid),
        Err(DetailError::InvalidTransform)
    );
    assert_eq!(scene.counts().instances, 1);
}

#[test]
fn instances_report_distinct_unique_and_expanded_counts_without_extra_meshes() {
    let mut scene = DetailScene::new();
    scene
        .add_prototype(parasol_mushroom("shroom").unwrap())
        .unwrap();
    let unique = scene.prototype("shroom").unwrap().occupied_cells();
    for (i, yaw) in [Yaw::Deg0, Yaw::Deg90, Yaw::Deg180, Yaw::Deg270]
        .into_iter()
        .enumerate()
    {
        let transform = Transform::new([i as f32 * 2.0, 0.0, -3.0], yaw).unwrap();
        scene
            .place(format!("shroom-{i}"), "shroom", transform)
            .unwrap();
    }
    scene.prototype_mesh("shroom", Lod::Source).unwrap();
    for draw in scene.draws() {
        assert_eq!(draw.prototype, "shroom");
        assert_eq!(draw.occupied_cells, unique);
    }
    let counts = scene.counts();
    assert_eq!(counts.unique_stored_cells, unique);
    assert_eq!(counts.expanded_occupied_cells, unique * 4);
    assert_eq!(counts.mesh_builds, 1, "no per-instance mesh generation");
}

#[test]
fn water_is_liquid_policy_and_terrain_is_collidable() {
    assert_eq!(material_policy(material::WATER), MaterialPolicy::Liquid);
    assert_eq!(
        material_policy(material::MOSS_TURF),
        MaterialPolicy::Collision
    );
    assert_eq!(material_name(material::WATER), "water");

    let tile = terrain_detail_tile("tile", 7).unwrap();
    let mut water = None;
    let mut solid = None;
    for (cell, m) in tile.iter_cells() {
        if m == material::WATER && water.is_none() {
            water = Some(cell);
        }
        if m == material::BANK_STONE && solid.is_none() {
            solid = Some(cell);
        }
    }
    let (water, solid) = (
        water.expect("tile has water"),
        solid.expect("tile has stone"),
    );
    let mut scene = DetailScene::new();
    scene.add_prototype(tile).unwrap();
    scene.place("tile", "tile", Transform::identity()).unwrap();
    let centre = |cell: [i32; 3]| cell.map(|v| (v as f32 + 0.5) * SCALE_TILE_M);
    assert_eq!(
        scene
            .sample_world_metres(centre(water))
            .unwrap()
            .map(|hit| hit.1),
        Some(material::WATER)
    );
    assert!(!scene.is_collidable_world_metres(centre(water)).unwrap());
    assert!(scene.is_collidable_world_metres(centre(solid)).unwrap());
}

#[test]
fn tile_fixture_is_deterministic_with_negative_coords_relief_and_water() {
    let a = terrain_detail_tile("tile", 2026).unwrap();
    let b = terrain_detail_tile("tile", 2026).unwrap();
    assert_eq!(a.snapshot(), b.snapshot());
    assert_ne!(
        a.snapshot().runs,
        terrain_detail_tile("tile", 7).unwrap().snapshot().runs
    );
    let (min, max) = a.cell_bounds().unwrap();
    assert_eq!(min[0], -32);
    assert_eq!(max[0], 31);
    assert!(min[2] < 0 && min[1] < 0, "negative coordinates present");
    // 16 m of tile at 25 cm cells.
    let bounds = a.bounds_local().unwrap();
    assert!((bounds.max[0] - bounds.min[0] - 16.0).abs() < 1e-4);
    let heights: std::collections::BTreeSet<i32> = a
        .iter_cells()
        .filter(|(_, m)| *m == material::MOSS_TURF)
        .map(|(cell, _)| cell[1])
        .collect();
    assert!(heights.len() >= 4, "uneven relief: {heights:?}");
    assert!(a.iter_cells().any(|(_, m)| m == material::WATER));
    assert!(a
        .iter_cells()
        .any(|(cell, m)| m == material::WATER && cell[0] < 0 && cell[2] < 0));
}

#[test]
fn mushroom_prototype_has_stipe_crown_rim_and_connected_gills() {
    let shroom = parasol_mushroom("shroom").unwrap();
    let (min, max) = shroom.cell_bounds().unwrap();
    assert_eq!(min[1], 0);
    assert_eq!(max[1], 21);
    // Cap is far wider than the stipe: a parasol silhouette, not a cylinder.
    assert_eq!((min[0], max[0]), (-9, 9));
    let stipe_width = shroom
        .iter_cells()
        .filter(|(cell, m)| cell[1] == 8 && *m == material::MUSHROOM_STIPE)
        .count();
    assert!(stipe_width <= 9, "slim stipe waist: {stipe_width}");

    let count = |material: u8| shroom.iter_cells().filter(|(_, m)| *m == material).count();
    assert!(count(material::MUSHROOM_STIPE) > 20);
    assert!(count(material::MUSHROOM_CAP) > 100);
    assert!(count(material::MUSHROOM_RIM) > 20);
    assert!(count(material::MUSHROOM_GILL) >= 16, "radial gills present");

    // Gills sit under the cap and above the rim layer's outer ring.
    assert!(shroom
        .iter_cells()
        .filter(|(_, m)| *m == material::MUSHROOM_GILL)
        .all(|(cell, _)| cell[1] == 16 && shroom.get([cell[0], 17, cell[2]]) != material::AIR));
    // The body is connected: collar cells join stipe top to cap underside.
    assert_eq!(shroom.get([0, 15, 0]), material::MUSHROOM_STIPE);
    assert_eq!(shroom.get([0, 16, 0]), material::MUSHROOM_STIPE);
    assert_eq!(shroom.get([0, 17, 0]), material::MUSHROOM_CAP);
    assert!(shroom.mesh_local().unwrap().indices.len() > 600);
}

#[test]
fn volume_capacity_and_scale_bounds_are_enforced_in_practice() {
    const { assert!(MAX_VOLUME_CELLS > 0) };
    let mut volume = DetailVolume::new("cap", fine());
    assert!(volume.set([0, 0, 0], material::MOSS_TURF).unwrap());
    assert!(!volume.set([0, 0, 0], material::MOSS_TURF).unwrap());
    assert!(volume.set([0, 0, 0], material::AIR).unwrap());
    assert_eq!(volume.occupied_cells(), 0);
    assert_eq!(volume.cell_bounds(), None);
    assert_eq!(volume.bounds_local(), None);
    assert_eq!(volume.get([MAX_CELL_COORD * 2, 0, 0]), material::AIR);
}

// ---------------------------------------------------------------------------
// A2 repair regressions
// ---------------------------------------------------------------------------

#[test]
fn extreme_cell_coordinates_are_rejected_without_abs_overflow() {
    let mut volume = DetailVolume::new("edge", tile());
    for cell in [
        [i32::MIN, 0, 0],
        [0, i32::MIN, 0],
        [0, 0, i32::MIN],
        [i32::MAX, 0, 0],
        [-MAX_CELL_COORD - 1, 0, 0],
    ] {
        assert_eq!(
            volume.set(cell, material::MOSS_TURF),
            Err(DetailError::CellOutOfRange)
        );
        assert_eq!(volume.get(cell), material::AIR);
    }
    // The inclusive boundary itself is accepted.
    assert!(volume
        .set([-MAX_CELL_COORD, MAX_CELL_COORD, 0], material::MOSS_TURF)
        .unwrap());
}

#[test]
fn non_finite_and_out_of_range_points_are_rejected_not_treated_as_origin_hits() {
    let mut volume = DetailVolume::new("nan", tile());
    volume.set([0, 0, 0], material::BANK_STONE).unwrap();
    let identity = Transform::identity();
    for bad in [
        [f32::NAN, 0.0, 0.0],
        [0.0, f32::INFINITY, 0.0],
        [0.0, 0.0, f32::NEG_INFINITY],
        [1e30, 0.0, 0.0],
    ] {
        assert_eq!(
            volume.sample_world_metres(&identity, bad),
            Err(DetailError::InvalidPoint),
            "{bad:?}"
        );
        assert_eq!(
            volume.cell_at_local_metres(bad),
            Err(DetailError::InvalidPoint)
        );
    }
    assert_eq!(
        volume.sample_local_metres([0.1, 0.1, 0.1]),
        Ok(material::BANK_STONE)
    );
}

#[test]
fn directly_constructed_invalid_transforms_are_rejected_by_every_consumer() {
    let mut volume = DetailVolume::new("bad-transform", tile());
    volume.set([0, 0, 0], material::BANK_STONE).unwrap();
    let invalid = Transform {
        translation_m: [f32::NAN, 0.0, 0.0],
        yaw: Yaw::Deg90,
    };
    assert!(!invalid.is_valid());
    assert_eq!(
        volume.sample_world_metres(&invalid, [0.0, 0.0, 0.0]),
        Err(DetailError::InvalidTransform)
    );
    assert_eq!(
        volume.mesh_world(&invalid).err(),
        Some(DetailError::InvalidTransform)
    );
    assert_eq!(
        volume.bounds_world(&invalid).err(),
        Some(DetailError::InvalidTransform)
    );
    let mut scene = DetailScene::new();
    scene.add_prototype(volume).unwrap();
    assert_eq!(
        scene.place("i", "bad-transform", invalid),
        Err(DetailError::InvalidTransform)
    );
}

#[test]
fn fractional_metre_translations_are_supported_and_queryable() {
    let mut volume = DetailVolume::new("frac", tile());
    volume.set([0, 0, 0], material::MOSS_TURF).unwrap();
    let transform = Transform::new([0.37, -1.13, 2.06], Yaw::Deg270).unwrap();
    let inside = transform.point_to_world([0.125, 0.125, 0.125]);
    assert_eq!(
        volume.sample_world_metres(&transform, inside),
        Ok(material::MOSS_TURF)
    );
}

#[test]
fn snapshot_loader_rejects_overlaps_regardless_of_declared_count() {
    let mut volume = DetailVolume::new("overlap", tile());
    volume.set([0, 0, 0], material::MOSS_TURF).unwrap();
    volume.set([1, 0, 0], material::MOSS_TURF).unwrap();
    let base = volume.snapshot();

    // Contradictory overlap whose lengths still sum to the declared count.
    let mut bad = base.clone();
    bad.runs = vec![[0, 0, 0, 1, 11], [0, 0, 0, 1, 12]];
    bad.occupied_cells = 2;
    assert_eq!(
        DetailVolume::from_snapshot(&bad).err(),
        Some(DetailError::MalformedSnapshot("overlapping runs"))
    );

    // Identical duplicate run, same total.
    let mut bad = base.clone();
    bad.runs = vec![[0, 0, 0, 2, 11], [1, 0, 0, 2, 11]];
    bad.occupied_cells = 4;
    assert_eq!(
        DetailVolume::from_snapshot(&bad).err(),
        Some(DetailError::MalformedSnapshot("overlapping runs"))
    );

    // Out-of-range extremes are refused before any write.
    for run in [[i32::MIN, 0, 0, 1, 11], [MAX_CELL_COORD, 0, 0, 8, 11]] {
        let mut bad = base.clone();
        bad.runs = vec![run];
        bad.occupied_cells = run[3] as usize;
        assert!(matches!(
            DetailVolume::from_snapshot(&bad),
            Err(DetailError::MalformedSnapshot(_))
        ));
    }
}

#[test]
fn chunk_major_snapshot_runs_are_accepted_and_json_loader_is_bounded() {
    // Two chunks along x means runs are emitted chunk-major, not sorted globally.
    let mut volume = DetailVolume::new("chunk-major", tile());
    volume.set([20, 0, 0], material::MOSS_TURF).unwrap();
    volume.set([0, 0, 0], material::MOSS_TURF).unwrap();
    let snapshot = volume.snapshot();
    assert!(snapshot.runs[0][0] < snapshot.runs[1][0]);
    let restored = DetailVolume::from_snapshot(&snapshot).unwrap();
    assert_eq!(restored.occupied_cells(), 2);

    let json = serde_json::to_vec(&snapshot).unwrap();
    assert!(json.len() < MAX_SNAPSHOT_JSON_BYTES);
    assert_eq!(
        DetailVolume::from_json_bytes(&json).unwrap().snapshot(),
        snapshot
    );
    let oversized = vec![b' '; MAX_SNAPSHOT_JSON_BYTES + 1];
    assert_eq!(
        DetailVolume::from_json_bytes(&oversized).err(),
        Some(DetailError::MalformedSnapshot("json byte cap exceeded"))
    );
    assert!(DetailVolume::from_json_bytes(b"{not json").is_err());
}

#[test]
fn coarsen_refuses_unsupported_derived_scales_instead_of_clamping() {
    let mut half_metre = DetailVolume::new("coarse", Scale::new(0.5).unwrap());
    half_metre.set([0, 0, 0], material::BANK_STONE).unwrap();
    assert_eq!(
        half_metre.coarsen(Lod::Half).map(|v| v.scale()),
        Ok(Scale::new(1.0).unwrap())
    );
    assert_eq!(
        half_metre.coarsen(Lod::Quarter).err(),
        Some(DetailError::InvalidScale)
    );
    let mut scene = DetailScene::new();
    scene.add_prototype(half_metre).unwrap();
    assert_eq!(
        scene.prototype_mesh("coarse", Lod::Quarter).err(),
        Some(DetailError::InvalidScale)
    );
}

#[test]
fn lod_mesh_revision_identifies_the_authoritative_prototype_revision() {
    let mut scene = DetailScene::new();
    scene
        .add_prototype(parasol_mushroom("shroom").unwrap())
        .unwrap();
    let revision = scene.prototype("shroom").unwrap().revision();
    for lod in [Lod::Source, Lod::Half, Lod::Quarter] {
        assert_eq!(
            scene.prototype_mesh("shroom", lod).unwrap().revision,
            revision
        );
    }
    scene
        .edit_prototype("shroom", [0, 25, 0], material::MUSHROOM_CAP)
        .unwrap();
    let updated = scene.prototype("shroom").unwrap().revision();
    assert!(updated > revision);
    assert_eq!(
        scene
            .prototype_mesh("shroom", Lod::Quarter)
            .unwrap()
            .revision,
        updated
    );
}

#[test]
fn scene_edits_preserve_identity_and_invalidate_cache_even_at_equal_edit_counts() {
    let mut scene = DetailScene::new();
    scene
        .add_prototype(parasol_mushroom("shroom").unwrap())
        .unwrap();
    let first = scene
        .prototype_mesh("shroom", Lod::Source)
        .unwrap()
        .vertices
        .len();
    let revision = scene.prototype("shroom").unwrap().revision();

    // Two edits that leave the occupied-cell count unchanged: the old failure
    // mode was a same-revision whole-volume swap serving a stale cached mesh.
    scene
        .edit_prototype("shroom", [0, 30, 0], material::MUSHROOM_CAP)
        .unwrap();
    scene
        .edit_prototype("shroom", [0, 21, 0], material::AIR)
        .unwrap();
    assert_eq!(scene.prototype("shroom").unwrap().id(), "shroom");
    assert!(scene.prototype("shroom").unwrap().revision() > revision);
    let second = scene
        .prototype_mesh("shroom", Lod::Source)
        .unwrap()
        .vertices
        .len();
    assert_ne!(first, second, "cache was rebuilt after the edits");
    assert_eq!(scene.counts().mesh_builds, 2);
    assert_eq!(
        scene.edit_prototype("missing", [0, 0, 0], 1).err(),
        Some(DetailError::UnknownPrototype("missing".into()))
    );
}

#[test]
fn collision_query_sees_solids_behind_liquid_instances_in_either_order() {
    let mut water = DetailVolume::new("water", tile());
    water.set([0, 0, 0], material::WATER).unwrap();
    let mut stone = DetailVolume::new("stone", tile());
    stone.set([0, 0, 0], material::BANK_STONE).unwrap();
    let point = [0.125, 0.125, 0.125];

    for (first, second) in [("a-water", "b-stone"), ("a-stone", "b-water")] {
        let mut scene = DetailScene::new();
        scene.add_prototype(water.clone()).unwrap();
        scene.add_prototype(stone.clone()).unwrap();
        let (water_id, stone_id) = if first.contains("water") {
            (first, second)
        } else {
            (second, first)
        };
        scene
            .place(water_id, "water", Transform::identity())
            .unwrap();
        scene
            .place(stone_id, "stone", Transform::identity())
            .unwrap();
        assert!(
            scene.is_collidable_world_metres(point).unwrap(),
            "solid behind liquid must still collide ({first} first)"
        );
        let hits = scene.sample_all_world_metres(point).unwrap();
        assert_eq!(hits.len(), 2);
        // Generic sample keeps its documented first-hit-by-instance-id policy.
        let first_hit = scene.sample_world_metres(point).unwrap().unwrap();
        assert_eq!(first_hit.0, first);
    }
}

#[test]
fn volume_chunk_budget_is_enforced_before_allocation() {
    let mut volume = DetailVolume::new("budget", tile());
    // One cell per chunk: cheap and sparse, never gigabytes.
    let mut placed = 0;
    for i in 0..MAX_VOLUME_CHUNKS as i32 {
        volume.set([i * 16, 0, 0], material::BANK_STONE).unwrap();
        placed += 1;
    }
    assert_eq!(placed, MAX_VOLUME_CHUNKS);
    assert_eq!(volume.chunk_count(), MAX_VOLUME_CHUNKS);
    assert_eq!(volume.source_bytes(), MAX_VOLUME_SOURCE_BYTES);
    let overflow = [MAX_VOLUME_CHUNKS as i32 * 16, 0, 0];
    assert_eq!(
        volume.set(overflow, material::BANK_STONE),
        Err(DetailError::BudgetExceeded("volume chunk budget"))
    );
    // Prior data survives, and edits inside existing chunks still work.
    assert_eq!(volume.get([0, 0, 0]), material::BANK_STONE);
    assert_eq!(volume.occupied_cells(), MAX_VOLUME_CHUNKS);
    assert!(volume.set([1, 0, 0], material::MOSS_TURF).unwrap());
    // Freeing a chunk makes room again.
    assert!(volume.set([1, 0, 0], material::AIR).unwrap());
    assert!(volume.set([0, 0, 0], material::AIR).unwrap());
    assert_eq!(volume.chunk_count(), MAX_VOLUME_CHUNKS - 1);
    assert!(volume.set(overflow, material::BANK_STONE).unwrap());
}

#[test]
fn occupied_and_chunk_bookkeeping_matches_snapshot_after_edits_and_removals() {
    let mut volume = DetailVolume::new("book", fine());
    assert!(volume.set([0, 0, 0], material::MOSS_TURF).unwrap());
    assert!(
        !volume.set([0, 0, 0], material::MOSS_TURF).unwrap(),
        "no-op"
    );
    assert!(
        volume.set([0, 0, 0], material::BANK_STONE).unwrap(),
        "replace"
    );
    assert_eq!(volume.occupied_cells(), 1);
    assert!(volume.set([40, -3, 5], material::MOSS_TURF).unwrap());
    assert_eq!(volume.occupied_cells(), 2);
    assert_eq!(volume.chunk_count(), 2);
    assert!(volume.set([40, -3, 5], material::AIR).unwrap());
    assert_eq!(volume.occupied_cells(), 1);
    assert_eq!(volume.chunk_count(), 1);
    assert_eq!(volume.snapshot().occupied_cells, 1);
    assert_eq!(volume.snapshot().runs.len(), 1);
    let tile = terrain_detail_tile("tile", 5).unwrap();
    assert_eq!(
        tile.occupied_cells(),
        tile.iter_cells().count(),
        "O(1) count matches a full scan"
    );
    assert_eq!(tile.source_bytes(), tile.chunk_count() * 4096);
}

#[test]
fn gallery_scene_instances_rest_on_dry_ground_across_the_foot_footprint() {
    let scene = gallery_scene(2026).unwrap();
    let tile = scene.prototype("terrain_tile_16m").unwrap();
    let cell = SCALE_TILE_M;
    let mushrooms: Vec<_> = scene
        .draws()
        .into_iter()
        .filter(|draw| draw.prototype == "parasol_mushroom")
        .collect();
    assert!(mushrooms.len() >= 4, "sparse but non-trivial gallery");
    for draw in mushrooms {
        let [tx, ty, tz] = draw.transform.translation_m;
        let foot_m = MUSHROOM_FOOT_RADIUS_CELLS as f32 * SCALE_FINE_M;
        for (dx, dz) in [
            (-foot_m, -foot_m),
            (foot_m, -foot_m),
            (-foot_m, foot_m),
            (foot_m, foot_m),
            (0.0, 0.0),
        ] {
            let column_x = ((tx + dx) / cell).floor() as i32;
            let column_z = ((tz + dz) / cell).floor() as i32;
            let (top, material) =
                column_top(tile, column_x, column_z).expect("supporting column exists");
            // Zero gap and zero burial: the support surface is exactly the foot plane.
            assert!(
                ((top + 1) as f32 * cell - ty).abs() < 1e-4,
                "gap under foot at ({column_x},{column_z}): top {top}, y {ty}"
            );
            assert_ne!(material, material::WATER, "no placement on water");
            assert_eq!(
                tile.get([column_x, top + 1, column_z]),
                material::AIR,
                "foot cell is free"
            );
        }
        assert!(ty > TILE_WATER_LEVEL_CELLS as f32 * cell);
    }
}

#[test]
fn gallery_scene_content_is_reproducible() {
    let a = gallery_scene(2026).unwrap();
    let b = gallery_scene(2026).unwrap();
    assert_eq!(a.draws(), b.draws());
    for id in a.prototype_ids() {
        assert_eq!(
            a.prototype(&id).unwrap().snapshot(),
            b.prototype(&id).unwrap().snapshot()
        );
        assert_eq!(
            a.prototype(&id)
                .unwrap()
                .mesh_local()
                .unwrap()
                .vertices
                .len(),
            b.prototype(&id)
                .unwrap()
                .mesh_local()
                .unwrap()
                .vertices
                .len()
        );
    }
}

#[test]
fn scene_source_budget_is_enforced_and_leaves_prior_prototypes_intact() {
    // Sparse one-cell-per-chunk volumes: 32 MiB of accounted payload, allocated
    // as 8 x 1024 chunks, not gigabytes of dense data.
    let volumes_at_cap = MAX_SCENE_SOURCE_BYTES / MAX_VOLUME_SOURCE_BYTES;
    let build = |id: String, chunks: usize| {
        let mut volume = DetailVolume::new(id, tile());
        for i in 0..chunks as i32 {
            volume.set([i * 16, 0, 0], material::BANK_STONE).unwrap();
        }
        volume
    };
    let mut scene = DetailScene::new();
    for index in 0..volumes_at_cap {
        scene
            .add_prototype(build(format!("full-{index}"), MAX_VOLUME_CHUNKS))
            .unwrap();
    }
    assert_eq!(scene.counts().source_bytes, MAX_SCENE_SOURCE_BYTES);
    assert_eq!(
        scene.add_prototype(build("one-more".into(), 1)),
        Err(DetailError::BudgetExceeded("scene source payload budget"))
    );
    assert_eq!(scene.counts().prototypes, volumes_at_cap);
    assert!(scene.prototype("full-0").is_some());
    scene
        .add_prototype(DetailVolume::new("empty", tile()))
        .unwrap();
    let revision = scene.prototype("empty").unwrap().revision();
    assert_eq!(
        scene.edit_prototype("empty", [0, 0, 0], material::BANK_STONE),
        Err(DetailError::BudgetExceeded("scene source payload budget"))
    );
    assert_eq!(scene.prototype("empty").unwrap().revision(), revision);
    assert_eq!(scene.prototype("empty").unwrap().occupied_cells(), 0);
    assert_eq!(scene.counts().source_bytes, MAX_SCENE_SOURCE_BYTES);
}

#[test]
fn unchanged_scene_edit_preserves_cached_geometry() {
    let mut scene = DetailScene::new();
    let mut volume = DetailVolume::new("unchanged", fine());
    volume.set([0, 0, 0], material::MOSS_TURF).unwrap();
    scene.add_prototype(volume).unwrap();
    let mesh = scene.prototype_mesh("unchanged", Lod::Source).unwrap();
    let allocated = mesh.vertices.capacity() * std::mem::size_of::<matterweave_core::Vertex>()
        + mesh.indices.capacity() * std::mem::size_of::<u32>();
    assert_eq!(scene.counts().cached_mesh_bytes, allocated);
    assert!(!scene
        .edit_prototype("unchanged", [0, 0, 0], material::MOSS_TURF)
        .unwrap());
    scene.prototype_mesh("unchanged", Lod::Source).unwrap();
    assert_eq!(scene.counts().mesh_builds, 1);
}

#[test]
fn empty_scene_rejects_invalid_queries_and_distant_instances_do_not_mask_hits() {
    let mut scene = DetailScene::new();
    for value in [f32::NAN, f32::INFINITY, -f32::INFINITY, 1e20] {
        assert_eq!(
            scene.sample_world_metres([value, 0., 0.]),
            Err(DetailError::InvalidPoint)
        );
        assert_eq!(
            scene.sample_all_world_metres([value, 0., 0.]),
            Err(DetailError::InvalidPoint)
        );
        assert_eq!(
            scene.is_collidable_world_metres([value, 0., 0.]),
            Err(DetailError::InvalidPoint)
        );
    }
    let mut volume = DetailVolume::new("cell", fine());
    volume.set([0, 0, 0], material::BANK_STONE).unwrap();
    scene.add_prototype(volume).unwrap();
    scene.place("a-far", "cell", Transform::identity()).unwrap();
    scene
        .place(
            "z-near",
            "cell",
            Transform::new([7000., 0., 0.], Yaw::Deg0).unwrap(),
        )
        .unwrap();
    let half_cell = SCALE_FINE_M * 0.5;
    let point = [7000.0 + half_cell, half_cell, half_cell];
    assert_eq!(
        scene.sample_world_metres(point).unwrap(),
        Some(("z-near".into(), material::BANK_STONE))
    );
    assert_eq!(scene.sample_all_world_metres(point).unwrap().len(), 1);
    assert!(scene.is_collidable_world_metres(point).unwrap());
}

#[test]
fn mesh_budget_includes_indices_and_bounds_coarsening_scratch() {
    // Vertex-only preflight accepted this; total vertex/index bound must reject
    // before constructing a potentially fragmented mesh or its scratch map.
    let mut volume = DetailVolume::new("index-budget", fine());
    for i in 0..34_000 {
        volume
            .set([i % 64, i / 4096, (i / 64) % 64], material::BANK_STONE)
            .unwrap();
    }
    assert!(matches!(
        volume.mesh_local(),
        Err(DetailError::BudgetExceeded(_))
    ));
    assert!(matches!(
        volume.coarsen(Lod::Half),
        Err(DetailError::BudgetExceeded(_))
    ));
    assert_eq!(volume.occupied_cells(), 34_000);
}
