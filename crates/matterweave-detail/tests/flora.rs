use matterweave_detail::*;
use std::collections::BTreeSet;

/// Six-connected component count over occupied cells. Used to assert that a
/// fungal body is one connected solid rather than parts sharing a bounding box.
fn components(volume: &DetailVolume) -> usize {
    let cells: BTreeSet<[i32; 3]> = volume.iter_cells().map(|(c, _)| c).collect();
    let mut seen: BTreeSet<[i32; 3]> = BTreeSet::new();
    let mut count = 0;
    for start in &cells {
        if seen.contains(start) {
            continue;
        }
        count += 1;
        let mut stack = vec![*start];
        seen.insert(*start);
        while let Some(c) = stack.pop() {
            for step in [
                [1, 0, 0],
                [-1, 0, 0],
                [0, 1, 0],
                [0, -1, 0],
                [0, 0, 1],
                [0, 0, -1],
            ] {
                let n = [c[0] + step[0], c[1] + step[1], c[2] + step[2]];
                if cells.contains(&n) && seen.insert(n) {
                    stack.push(n);
                }
            }
        }
    }
    count
}

#[test]
fn every_flora_body_connects_to_its_root() {
    for id in FLORA_SPECIES {
        let volume = flora_prototype(id).unwrap();
        assert_eq!(volume.cell_bounds().unwrap().0[1], 0, "{id}");
        assert_eq!(components(&volume), 1, "disconnected anatomy in {id}");
    }
}

#[test]
fn support_honours_fractional_translation_and_rejects_invalid_transforms() {
    let mut tile = DetailVolume::new("support", Scale::new(SCALE_TILE_M).unwrap());
    tile.set([0, 1, 0], material::MOSS_TURF).unwrap();
    tile.set([1, 1, 0], material::WATER).unwrap();
    let mut plant = DetailVolume::new("root", Scale::new(SCALE_FINE_M).unwrap());
    plant.set([1, 0, 0], material::FLORA_FUNNEL_STIPE).unwrap();
    // Both origins floor to terrain column0; only the second actual foot is in water.
    let centred = Transform::new([0.125, 0.5, 0.125], Yaw::Deg0).unwrap();
    let shifted = Transform::new([0.2, 0.5, 0.125], Yaw::Deg0).unwrap();
    assert_eq!(
        instance_support(&tile, &plant, "root", &centred),
        Some((1, "moss_turf", 0.0))
    );
    assert_eq!(instance_support(&tile, &plant, "root", &shifted), None);
    for value in [f32::NAN, f32::INFINITY, -f32::INFINITY] {
        for axis in 0..3 {
            let mut bad = centred;
            bad.translation_m[axis] = value;
            assert_eq!(instance_support(&tile, &plant, "root", &bad), None);
        }
    }
}

#[test]
fn root_contact_does_not_allow_body_to_intersect_a_higher_bank() {
    let mut tile = DetailVolume::new("bank", Scale::new(SCALE_TILE_M).unwrap());
    tile.set([0, 1, 0], material::MOSS_TURF).unwrap();
    let mut plant = DetailVolume::new("branch", Scale::new(SCALE_FINE_M).unwrap());
    plant.set([0, 0, 0], material::FLORA_FUNNEL_STIPE).unwrap();
    for x in 0..=3 {
        plant.set([x, 1, 0], material::FLORA_FUNNEL_STIPE).unwrap();
    }
    let transform = Transform::new([0.125, 0.5, 0.125], Yaw::Deg0).unwrap();
    assert!(instance_support(&tile, &plant, "branch", &transform).is_some());
    tile.set([1, 2, 0], material::BANK_STONE).unwrap();
    assert_eq!(instance_support(&tile, &plant, "branch", &transform), None);
    // Higher foliage may pass above the bank; we do not require a flat canopy footprint.
    tile.set([1, 2, 0], material::AIR).unwrap();
    tile.set([1, 1, 0], material::BANK_STONE).unwrap();
    assert!(instance_support(&tile, &plant, "branch", &transform).is_some());
}

type Builder = fn(&str) -> Result<DetailVolume>;

fn builders() -> Vec<(&'static str, Builder)> {
    vec![
        ("funnel_mushroom", funnel_mushroom),
        ("clustered_mushroom", clustered_mushroom),
        ("fan_frond", fan_frond),
        ("reed_cluster", reed_cluster),
        ("rosette_groundcover", rosette_groundcover),
    ]
}

#[test]
fn palette_is_additive_with_flora_policies() {
    // Existing meanings are preserved exactly.
    assert_eq!(material_name(material::MUSHROOM_CAP), "mushroom_cap");
    assert_eq!(material_policy(material::WATER), MaterialPolicy::Liquid);
    assert_eq!(
        material_policy(material::MOSS_TURF),
        MaterialPolicy::Collision
    );
    assert_eq!(material_policy(material::AIR), MaterialPolicy::Decorative);
    // New fungus materials collide; leafy flora is decorative.
    for id in [30, 31, 32, 33, 34, 35, 36, 37] {
        assert_eq!(
            material_policy(id),
            MaterialPolicy::Collision,
            "material {id}"
        );
        assert_ne!(material_name(id), "unknown", "material {id}");
        assert!(material_color(id).iter().all(|v| v.is_finite()));
    }
    for id in [38, 39, 40, 41, 42, 43, 44, 45, 46, 47] {
        assert_eq!(
            material_policy(id),
            MaterialPolicy::Decorative,
            "material {id}"
        );
        assert_ne!(material_name(id), "unknown", "material {id}");
    }
    // Explicit policy: a luminous accent inside fungal flesh is part of a
    // substantive body and collides. It has its own ID rather than reusing the
    // decorative leaf accent, so colour never opens a hole in a gameplay solid.
    assert_eq!(
        material_policy(material::FLORA_FUNGUS_LUMEN),
        MaterialPolicy::Collision
    );
    assert_eq!(material_name(48), "flora_fungus_lumen");
    assert_ne!(material::FLORA_FUNGUS_LUMEN, material::FLORA_LUMEN_DOT);
}

#[test]
fn prototypes_are_deterministic_with_explicit_scales_and_budgets() {
    for (id, build) in builders() {
        let first = build(id).unwrap();
        let second = build("same-content").unwrap();
        assert_eq!(first.snapshot().runs, second.snapshot().runs, "{id}");
        assert_eq!(first.occupied_cells(), second.occupied_cells(), "{id}");
        assert_eq!(first.cell_bounds(), second.cell_bounds(), "{id}");
        assert!(
            first.occupied_cells() < 33_288,
            "{id} fits the mesh preflight cap"
        );
        assert!(first.chunk_count() <= MAX_VOLUME_CHUNKS, "{id}");
        let (min, _) = first.cell_bounds().unwrap();
        assert_eq!(min[1], 0, "{id} is rooted at local y = 0");
        let bounds = first.bounds_local().unwrap();
        assert!((bounds.min[1] - 0.0).abs() < 1e-6, "{id}");
        assert!(bounds.min.iter().zip(bounds.max.iter()).all(|(a, b)| a < b));
    }
    assert_eq!(funnel_mushroom("f").unwrap().scale().metres(), 0.0625);
    assert_eq!(clustered_mushroom("c").unwrap().scale().metres(), 0.0625);
    assert_eq!(fan_frond("f").unwrap().scale().metres(), 0.125);
    assert_eq!(reed_cluster("r").unwrap().scale().metres(), 0.125);
    assert_eq!(rosette_groundcover("g").unwrap().scale().metres(), 0.125);
    assert_eq!(FLORA_FUNGUS_SCALE_M, 0.0625);
    assert_eq!(FLORA_LEAF_SCALE_M, 0.125);
}

#[test]
fn funnel_has_hollow_pit_connected_stipe_and_gills_under_flesh() {
    let funnel = funnel_mushroom("funnel").unwrap();
    // Stipe runs continuously into the collar; the pit above is an actual hole.
    assert_eq!(funnel.get([0, 11, 0]), material::FLORA_FUNNEL_STIPE);
    assert_eq!(funnel.get([0, 12, 0]), material::FLORA_FUNNEL_STIPE);
    for y in 13..=18 {
        assert_eq!(
            funnel.get([0, y, 0]),
            material::AIR,
            "hollow pit at y = {y}"
        );
    }
    let count = |m: u8| funnel.iter_cells().filter(|(_, mat)| *mat == m).count();
    assert!(
        count(material::FLORA_FUNNEL_STIPE) > 100,
        "substantive stipe"
    );
    assert!(count(material::FLORA_FUNNEL_CAP) > 400, "funnel flesh");
    assert!(count(material::FLORA_FUNNEL_RIM) > 30, "rolled ribbed rim");
    assert!(count(material::FLORA_FUNNEL_GILL) >= 24, "radial gills");
    assert!(
        count(material::FLORA_FUNGUS_LUMEN) > 0,
        "restrained pit accent, collidable fungal flesh"
    );
    assert_eq!(
        count(material::FLORA_LUMEN_DOT),
        0,
        "no decorative holes in a collidable fungus"
    );
    assert_eq!(components(&funnel), 1, "funnel is one connected body");
    // Every gill sits directly under cap flesh.
    for (cell, _) in funnel
        .iter_cells()
        .filter(|(_, m)| *m == material::FLORA_FUNNEL_GILL)
    {
        assert_eq!(cell[1], 12, "gills form one underside layer");
        assert_ne!(
            funnel.get([cell[0], 13, cell[2]]),
            material::AIR,
            "flesh above {cell:?}"
        );
    }
    assert!(assert_policy(&funnel, MaterialPolicy::Collision));
}

#[test]
fn clustered_has_four_distinct_caps_connected_roots_and_gills() {
    let clump = clustered_mushroom("clump").unwrap();
    let umbos = [([-5, 16, -4]), ([5, 19, -5]), ([-4, 14, 5]), ([4, 17, 4])];
    for umbo in umbos {
        assert_eq!(
            clump.get(umbo),
            material::FLORA_CLUSTER_CAP,
            "umbo at {umbo:?}"
        );
    }
    for (i, a) in umbos.iter().enumerate() {
        for b in &umbos[i + 1..] {
            let gap = (a[0] - b[0]).abs() + (a[2] - b[2]).abs();
            assert!(gap >= 6, "distinct cap centres: {a:?} vs {b:?}");
        }
    }
    // One compact root disc at y = 0 carries the whole clump, and every stem is
    // actually joined to it: assert real connectivity, not sampled cells.
    assert_eq!(components(&clump), 1, "clump is one connected body");
    let feet: Vec<[i32; 3]> = clump
        .iter_cells()
        .filter(|(c, _)| c[1] == 0)
        .map(|(c, _)| c)
        .collect();
    assert!(!feet.is_empty(), "clump is rooted");
    assert!(
        feet.iter().all(|c| c[0].abs() <= 2 && c[2].abs() <= 2) && feet.len() >= 13,
        "root disc is compact and substantial: {feet:?}"
    );
    // Gills under one cap with flesh above.
    assert_eq!(clump.get([8, 15, -5]), material::FLORA_CLUSTER_GILL);
    assert_eq!(clump.get([8, 16, -5]), material::FLORA_CLUSTER_CAP);
    let count = |m: u8| clump.iter_cells().filter(|(_, mat)| *mat == m).count();
    assert!(count(material::FLORA_CLUSTER_RIM) > 20, "rim rings");
    assert!(
        count(material::FLORA_CLUSTER_GILL) >= 32,
        "gills under caps"
    );
    assert!(assert_policy(&clump, MaterialPolicy::Collision));
}

#[test]
fn frond_has_continuous_ribs_flanking_blades_and_decorative_policy() {
    let frond = fan_frond("frond").unwrap();
    let dirs = [[8, 0], [6, 6], [0, 8], [-6, 6], [-8, 0], [-6, -6], [6, -6]];
    let lengths = [12, 10, 13, 11, 12, 10, 11];
    for (dir, length) in dirs.into_iter().zip(lengths) {
        let mut rib_cells = 0;
        for t in 0..=length {
            let cell = [dir[0] * t / 8, 1 + t - t * t / length, dir[1] * t / 8];
            assert_ne!(frond.get(cell), material::AIR, "rib continuity at {cell:?}");
            rib_cells += 1;
        }
        assert!(rib_cells >= 10, "readable frond length");
    }
    let count = |m: u8| frond.iter_cells().filter(|(_, mat)| *mat == m).count();
    // Shape proportion: this must read as a fan of blades, not a bundle of
    // sticks, so blade cells dominate rib/vein cells by a real margin.
    assert!(
        count(material::FLORA_FROND_BLADE) >= 2 * count(material::FLORA_FROND_RIB),
        "blades {} vs ribs {}",
        count(material::FLORA_FROND_BLADE),
        count(material::FLORA_FROND_RIB)
    );
    // The blade widens across the middle of a rib and tapers to the tip.
    let span = |t: i32| {
        let y = 1 + t - t * t / 12;
        (-4..=4)
            .filter(|dz| frond.get([t, y, *dz]) != material::AIR)
            .count()
    };
    assert!(span(6) >= 5, "broad blade mid-rib: {}", span(6));
    assert!(span(11) < span(6), "blade tapers to the tip");
    assert!(count(material::FLORA_FROND_STEM) > 0, "rhizome foot");
    assert!(frond.iter_cells().all(|(cell, _)| cell[1] >= 0));
    assert!(assert_policy(&frond, MaterialPolicy::Decorative));
}

#[test]
fn reed_and_rosette_have_readable_supporting_structure() {
    let reed = reed_cluster("reed").unwrap();
    // Nine culms rising from one compact rhizome pad, joined to it by runners.
    for [ox, oz] in [
        [-2, -2],
        [0, -2],
        [2, -2],
        [-2, 0],
        [0, 0],
        [2, 0],
        [-2, 2],
        [0, 2],
        [2, 2],
    ] {
        assert_eq!(reed.get([ox, 1, oz]), material::FLORA_REED_STEM);
    }
    assert_eq!(components(&reed), 1, "reed cluster is one rooted body");
    assert_eq!(
        reed.iter_cells().filter(|(c, _)| c[1] == 0).count(),
        9,
        "compact ground contact pad"
    );
    assert_eq!(reed.get([0, 19, 0]), material::FLORA_LUMEN_DOT);
    let count = |m: u8| reed.iter_cells().filter(|(_, mat)| *mat == m).count();
    assert!(count(material::FLORA_REED_LEAF) >= 40, "angled leaves");
    assert!(count(material::FLORA_REED_PLUME) >= 60, "plume heads");
    assert_eq!(
        count(material::FLORA_LUMEN_DOT),
        9,
        "one lumen tip per culm"
    );
    assert!(assert_policy(&reed, MaterialPolicy::Decorative));

    let rosette = rosette_groundcover("rosette").unwrap();
    assert_eq!(rosette.get([0, 4, 0]), material::FLORA_ROSETTE_HEART);
    let spots = rosette
        .iter_cells()
        .filter(|(_, m)| *m == material::FLORA_ROSETTE_SPOT)
        .count();
    assert_eq!(spots, 4, "accent-spotted even leaf tips");
    assert_eq!(
        rosette
            .iter_cells()
            .filter(|(cell, _)| cell[1] == 0)
            .count(),
        9,
        "compact ground contact pad"
    );
    assert!(
        rosette
            .iter_cells()
            .filter(|(cell, _)| cell[1] == 1)
            .count()
            > 20,
        "leaf whorl lies low over the pad"
    );
    assert!(assert_policy(&rosette, MaterialPolicy::Decorative));
}

#[test]
fn flora_prototypes_round_trip_and_survive_lod_derivation() {
    for (id, build) in builders() {
        let volume = build(id).unwrap();
        let snapshot = volume.snapshot();
        let restored = DetailVolume::from_snapshot(&snapshot).unwrap();
        assert_eq!(restored.occupied_cells(), volume.occupied_cells(), "{id}");
        assert_eq!(restored.cell_bounds(), volume.cell_bounds(), "{id}");
        assert_eq!(restored.snapshot().runs, snapshot.runs, "{id}");

        let before = volume.snapshot();
        let half = volume.coarsen(Lod::Half).unwrap();
        let quarter = volume.coarsen(Lod::Quarter).unwrap();
        assert_eq!(volume.snapshot(), before, "{id} source intact across LOD");
        assert!(
            half.occupied_cells() > 0 && quarter.occupied_cells() > 0,
            "{id}"
        );
        assert!(half.occupied_cells() <= volume.occupied_cells(), "{id}");

        // `coarsen` yields a SEPARATE volume with its own revision; only the
        // scene re-stamps the authoritative source revision on a derived mesh.
        // Assert the real contract on both sides rather than expecting a
        // standalone coarse volume to inherit the source revision.
        let mut scene = DetailScene::new();
        scene.add_prototype(volume.clone()).unwrap();
        for lod in [Lod::Source, Lod::Half, Lod::Quarter] {
            let standalone = match lod {
                Lod::Source => volume.mesh_local().unwrap(),
                other => volume.coarsen(other).unwrap().mesh_local().unwrap(),
            };
            assert!(
                !standalone.indices.is_empty(),
                "{id} {lod:?} meshes geometry"
            );
            let scene_mesh = scene.prototype_mesh(id, lod).unwrap();
            assert_eq!(
                scene_mesh.indices.len(),
                standalone.indices.len(),
                "{id} {lod:?}"
            );
            assert_eq!(
                scene_mesh.revision,
                volume.revision(),
                "{id} {lod:?} carries the authoritative source revision"
            );
        }
        assert_eq!(
            volume.snapshot(),
            before,
            "{id} source intact after meshing"
        );
    }
}

#[test]
fn bracket_species_is_reserved_and_scales_reject_garbage() {
    assert_eq!(
        flora_prototype("bracket_shelf").err(),
        Some(DetailError::UnknownPrototype("bracket_shelf".into()))
    );
    assert_eq!(Scale::new(0.125).unwrap().metres(), 0.125);
    assert_eq!(Scale::new(2.0), Err(DetailError::InvalidScale));
}

#[test]
fn dense_tile_meets_thresholds_deterministically() {
    let first = dense_tile(FLORA_CANONICAL_SEED).unwrap();
    let second = dense_tile(FLORA_CANONICAL_SEED).unwrap();
    assert_eq!(first.draws(), second.draws());
    for id in first.prototype_ids() {
        assert_eq!(
            first.prototype(&id).unwrap().snapshot(),
            second.prototype(&id).unwrap().snapshot(),
            "{id}"
        );
    }
    let counts = first.counts();
    let tile_cells = first
        .prototype("terrain_tile_16m")
        .unwrap()
        .occupied_cells();
    let flora_cells = counts.expanded_occupied_cells - tile_cells;
    let vegetation = counts.instances - 1;
    let mut types = BTreeSet::new();
    for draw in first.draws() {
        if draw.prototype != "terrain_tile_16m" {
            types.insert(draw.prototype);
        }
    }
    assert!(
        vegetation >= DENSE_VEGETATION_MIN,
        "vegetation instances: {vegetation}"
    );
    assert!(
        flora_cells >= DENSE_FLORA_CELLS_MIN,
        "flora cells: {flora_cells}"
    );
    assert!(types.len() >= DENSE_TYPES_MIN, "distinct types: {types:?}");
    assert_eq!(FLORA_SPECIES.len(), 6);
    // Every planned species is present.
    for species in FLORA_SPECIES {
        assert!(types.contains(species), "missing {species}");
    }
    // Varied yaw across the planting.
    let mut yaws = BTreeSet::new();
    for draw in first.draws() {
        yaws.insert(format!("{:?}", draw.transform.yaw));
    }
    assert!(yaws.len() >= 3, "varied yaw: {yaws:?}");
}

#[test]
fn dense_tile_feet_are_supported_corridor_open_and_water_readable() {
    let scene = dense_tile(FLORA_CANONICAL_SEED).unwrap();
    let tile = scene.prototype("terrain_tile_16m").unwrap();
    assert!(
        tile.iter_cells().any(|(_, m)| m == material::WATER),
        "creek water stored"
    );
    for draw in scene.draws() {
        if draw.prototype == "terrain_tile_16m" {
            continue;
        }
        let prototype = scene.prototype(&draw.prototype).unwrap();
        let support = instance_support(tile, prototype, &draw.prototype, &draw.transform)
            .expect("complete foot footprint is supported");
        assert_eq!(support.1, "moss_turf", "{} rooted on moss", draw.instance);
        assert!(support.2.abs() < 1e-4, "{} has zero gap", draw.instance);
        // Corridor stays walkable: no planting inside x in [-1.0, 1.0).
        let x = draw.transform.translation_m[0];
        assert!(
            x < CORRIDOR_TILE_X.0 as f32 * SCALE_TILE_M
                || x >= CORRIDOR_TILE_X.1 as f32 * SCALE_TILE_M,
            "{} respects the corridor",
            draw.instance
        );
        // Independent contact recheck: every y = 0 cell lands on moss at support height.
        let fine = prototype.scale().metres();
        for (cell, _) in prototype.iter_cells() {
            let world = draw
                .transform
                .point_to_world(cell.map(|c| (c as f32 + 0.5) * fine));
            let material = tile.sample_local_metres(world).unwrap();
            assert!(
                material == material::AIR || material == material::WATER,
                "{} source cell {cell:?} intersects terrain",
                draw.instance
            );
        }
        for (cell, _) in prototype.iter_cells().filter(|(c, _)| c[1] == 0) {
            let local = [
                (cell[0] as f32 + 0.5) * fine,
                0.0,
                (cell[2] as f32 + 0.5) * fine,
            ];
            let rotated = match draw.transform.yaw {
                Yaw::Deg0 => [local[0], local[1], local[2]],
                Yaw::Deg90 => [local[2], local[1], -local[0]],
                Yaw::Deg180 => [-local[0], local[1], -local[2]],
                Yaw::Deg270 => [-local[2], local[1], local[0]],
            };
            let wx = draw.transform.translation_m[0] + rotated[0];
            let wz = draw.transform.translation_m[2] + rotated[2];
            let column = [
                (wx / SCALE_TILE_M).floor() as i32,
                (wz / SCALE_TILE_M).floor() as i32,
            ];
            assert_eq!(
                tile.get([column[0], support.0, column[1]]),
                material::MOSS_TURF,
                "{} contact {cell:?}",
                draw.instance
            );
        }
    }
}

#[test]
fn negative_rotated_placements_query_and_foot_policy() {
    let mut scene = DetailScene::new();
    scene
        .add_prototype(funnel_mushroom("funnel").unwrap())
        .unwrap();
    let transform = Transform::new([-3.5, 1.0, -2.25], Yaw::Deg270).unwrap();
    scene.place("neg-funnel", "funnel", transform).unwrap();
    // A known stipe cell through the rotated transform.
    let s = SCALE_FINE_M;
    let local = [(0.5) * s, (5.5) * s, (0.5) * s];
    let world = transform.point_to_world(local);
    assert_eq!(
        scene.sample_world_metres(world).unwrap(),
        Some(("neg-funnel".into(), material::FLORA_FUNNEL_STIPE))
    );
    assert!(scene.is_collidable_world_metres(world).unwrap());
    // Decorative leaf queries are visible but never walls.
    let mut leaf_scene = DetailScene::new();
    leaf_scene
        .add_prototype(fan_frond("frond").unwrap())
        .unwrap();
    leaf_scene
        .place("frond-0", "frond", Transform::identity())
        .unwrap();
    // Blade cell [6, 4, 1] of the +x frond, verified against the built source.
    let frond = fan_frond("probe").unwrap();
    assert_eq!(frond.get([6, 4, 1]), material::FLORA_FROND_BLADE);
    let blade_world = [6.5 * 0.125, 4.5 * 0.125, 1.5 * 0.125];
    let hit = leaf_scene.sample_world_metres(blade_world).unwrap();
    assert!(hit.is_some(), "frond blade is queryable source geometry");
    assert!(!leaf_scene.is_collidable_world_metres(blade_world).unwrap());
    // Error paths on scene identity.
    assert_eq!(
        scene.place("neg-funnel", "funnel", Transform::identity()),
        Err(DetailError::DuplicateInstance("neg-funnel".into()))
    );
    assert_eq!(
        scene.place("x", "missing", Transform::identity()),
        Err(DetailError::UnknownPrototype("missing".into()))
    );
    assert_eq!(
        scene.place(
            "bad",
            "funnel",
            Transform {
                translation_m: [f32::NAN, 0.0, 0.0],
                yaw: Yaw::Deg0
            }
        ),
        Err(DetailError::InvalidTransform)
    );
}

#[test]
fn dense_tile_meshes_fit_budgets_with_honest_cache_accounting() {
    let mut scene = dense_tile(FLORA_CANONICAL_SEED).unwrap();
    let ids = scene.prototype_ids();
    for id in &ids {
        for lod in [Lod::Source, Lod::Half, Lod::Quarter] {
            let revision = scene.prototype(id).unwrap().revision();
            let mesh = scene.prototype_mesh(id, lod).unwrap();
            assert!(!mesh.indices.is_empty(), "{id} {lod:?}");
            assert_eq!(mesh.revision, revision);
        }
    }
    assert_eq!(scene.counts().mesh_builds, (ids.len() * 3) as u64);
    let counts = scene.counts();
    assert!(counts.source_bytes <= MAX_SCENE_SOURCE_BYTES);
    assert!(counts.cached_mesh_bytes <= MAX_SCENE_CACHE_BYTES);
    // Retained capacities are reported honestly: recompute from the meshes.
    let mut retained = 0usize;
    for id in &ids {
        for lod in [Lod::Source, Lod::Half, Lod::Quarter] {
            let mesh = scene.prototype_mesh(id, lod).unwrap();
            retained += mesh.vertices.capacity() * std::mem::size_of::<matterweave_core::Vertex>()
                + mesh.indices.capacity() * std::mem::size_of::<u32>();
        }
    }
    assert_eq!(retained, counts.cached_mesh_bytes, "no hidden cache bytes");
}
