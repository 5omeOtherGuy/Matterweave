//! Acceptance tests for the full 128 m showcase map.
//!
//! The map is generated ONCE per test binary and shared, because a full
//! generation is the expensive part of this suite. Only the determinism test
//! pays for a second generation, which is exactly what it is measuring.

use matterweave_detail::{
    build_showcase, carved, composition_hash, material, material_policy, showcase_class,
    DetailScene, Lod, MaterialPolicy, Showcase, BASE_ROCK_ID, BASE_SOIL_ID, BASIN_WATER_LEVEL_M,
    MAP_EDGE_CELLS, MAP_EDGE_M, MAX_PROTOTYPE_CELLS, MAX_SCENE_CACHE_BYTES, MAX_SCENE_SOURCE_BYTES,
    SHOWCASE_EXPANDED_CELLS_MIN, SHOWCASE_FLORA_CELLS_MIN, SHOWCASE_PLANTS_MIN, SHOWCASE_SEED,
    TERRAIN_CELL_M, TILE_CELLS, WALK_SPEED_M_S,
};
use std::sync::OnceLock;

fn map() -> &'static Showcase {
    static MAP: OnceLock<Showcase> = OnceLock::new();
    MAP.get_or_init(|| build_showcase(SHOWCASE_SEED).expect("full showcase generation"))
}

/// Stable content hash of one prototype's authoritative cells, so determinism is
/// checked on actual voxel layout and not only on aggregate counts.
fn prototype_content_hash(scene: &DetailScene, id: &str) -> u64 {
    let volume = scene.prototype(id).expect("prototype exists");
    let mut acc: u64 = 0xcbf2_9ce4_8422_2325;
    for (cell, material) in volume.iter_cells() {
        for value in [
            cell[0] as i64 as u64,
            cell[1] as i64 as u64,
            cell[2] as i64 as u64,
            u64::from(material),
        ] {
            acc ^= value;
            acc = acc.wrapping_mul(0x1000_0000_01b3);
        }
    }
    acc
}

// ---------------------------------------------------------------------------
// Content: the owner's showcase density criteria, on authoritative data
// ---------------------------------------------------------------------------

#[test]
fn canonical_counts_meet_showcase_targets() {
    let showcase = map();
    let counts = showcase.scene.counts();
    let manifest = &showcase.manifest;

    assert!(
        counts.expanded_occupied_cells >= SHOWCASE_EXPANDED_CELLS_MIN,
        "instance-expanded occupied cells {} < {SHOWCASE_EXPANDED_CELLS_MIN}",
        counts.expanded_occupied_cells
    );
    assert!(
        manifest.flora_expanded_cells >= SHOWCASE_FLORA_CELLS_MIN,
        "above-ground flora cells {} < {SHOWCASE_FLORA_CELLS_MIN}",
        manifest.flora_expanded_cells
    );
    assert!(
        manifest.flora_instances >= SHOWCASE_PLANTS_MIN,
        "placed plants {} < {SHOWCASE_PLANTS_MIN}",
        manifest.flora_instances
    );

    // Prototype cells and instance-expanded cells are different quantities and
    // must never be conflated: nothing is duplicated in memory.
    assert!(counts.unique_stored_cells < counts.expanded_occupied_cells);
    assert_eq!(
        manifest.flora_expanded_cells + manifest.terrain_expanded_cells,
        counts.expanded_occupied_cells,
        "every expanded cell belongs to exactly one accounted class"
    );

    // Buried terrain alone must not satisfy the density objective.
    assert!(manifest.flora_expanded_cells >= 2_000_000);

    // Only the six existing catalogue species are placed here. The additional
    // archetypes are a separate catalogue task; this test must fail loudly if
    // anyone claims ten types from this generator.
    assert_eq!(
        manifest.species_instances.len(),
        6,
        "showcase places exactly the six existing flora prototypes"
    );
    for (species, placed) in &manifest.species_instances {
        assert!(*placed >= 100, "{species} is token-present with {placed}");
    }
}

#[test]
fn map_is_a_full_128_m_source_map_not_a_single_tile() {
    let showcase = map();
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for draw in showcase.scene.draws() {
        for axis in 0..3 {
            min[axis] = min[axis].min(draw.transform.translation_m[axis]);
            max[axis] = max[axis].max(draw.transform.translation_m[axis]);
        }
    }
    assert!(min[0] <= 1.0 && min[2] <= 1.0, "map does not start at origin");
    assert!(
        max[0] >= MAP_EDGE_M - 8.0 && max[2] >= MAP_EDGE_M - 8.0,
        "placed content spans only {max:?}, not a 128 m map"
    );
    // Vertical extent must be real relief, not a flat slab.
    assert!(max[1] - min[1] >= 24.0, "vertical extent {}", max[1] - min[1]);
    assert_eq!(TERRAIN_CELL_M, 0.25, "terrain source stays at 25 cm");
}

// ---------------------------------------------------------------------------
// Source and derived-mesh budgets, for bounded physics/render integration
// ---------------------------------------------------------------------------

#[test]
fn every_prototype_fits_the_mesh_preflight_and_the_aggregate_cache_budget() {
    let mut showcase = build_showcase(SHOWCASE_SEED).expect("generation");
    let counts = showcase.scene.counts();
    assert!(
        counts.source_bytes <= MAX_SCENE_SOURCE_BYTES,
        "source payload {} exceeds budget",
        counts.source_bytes
    );

    let mut aggregate = 0usize;
    let mut collision_prototypes = 0usize;
    let mut collision_prototype_cells = 0usize;
    for id in showcase.scene.prototype_ids() {
        let volume = showcase.scene.prototype(&id).expect("listed prototype");
        assert!(
            volume.occupied_cells() <= MAX_PROTOTYPE_CELLS,
            "{id} has {} cells, above the mesh preflight ceiling",
            volume.occupied_cells()
        );
        let has_collision = volume
            .iter_cells()
            .any(|(_, m)| material_policy(m) == MaterialPolicy::Collision);
        if has_collision {
            collision_prototypes += 1;
            collision_prototype_cells += volume.occupied_cells();
        }
        // Meshing must succeed at Lod::Source for every prototype; measure it,
        // then release it so the cache bound is never the thing under test.
        showcase
            .scene
            .prototype_mesh(&id, Lod::Source)
            .unwrap_or_else(|e| panic!("{id} failed Lod::Source meshing: {e}"));
        aggregate += showcase.scene.counts().cached_mesh_bytes;
        showcase.scene.invalidate(&id);
        assert_eq!(showcase.scene.counts().cached_mesh_bytes, 0);
    }
    assert!(
        aggregate <= MAX_SCENE_CACHE_BYTES,
        "aggregate Lod::Source mesh bytes {aggregate} exceed the {MAX_SCENE_CACHE_BYTES} cache budget"
    );
    // Physics integration bound: recorded so the collision working set is sized
    // from measurement rather than guessed.
    assert!(collision_prototypes > 0 && collision_prototype_cells > 0);
    println!(
        "collision prototypes {collision_prototypes}, collision prototype cells {collision_prototype_cells}, aggregate source mesh bytes {aggregate}"
    );
}

#[test]
fn deep_base_tiles_are_shared_fully_solid_reuse_not_regenerated_data() {
    let showcase = map();
    let expected_cells = (TILE_CELLS * TILE_CELLS * matterweave_detail::BAND_CELLS) as usize;
    for id in [BASE_ROCK_ID, BASE_SOIL_ID] {
        let volume = showcase.scene.prototype(id).expect("shared base prototype");
        assert_eq!(
            volume.occupied_cells(),
            expected_cells,
            "{id} must be genuinely fully solid, with no air padding"
        );
        // Fully dense: one byte of source payload per stored cell.
        assert_eq!(volume.source_bytes(), expected_cells);
    }
    // Reuse must be real: far more base instances than base prototypes.
    assert_eq!(showcase.manifest.terrain_base_prototypes, 2);
    assert!(showcase.manifest.terrain_base_instances > 500);
    // ... and the shell prototypes must not be duplicate LOD copies of them.
    assert!(showcase.manifest.terrain_shell_prototypes > 500);
}

// ---------------------------------------------------------------------------
// Terrain / water / query consistency, including seams and negative inputs
// ---------------------------------------------------------------------------

#[test]
fn surface_query_agrees_with_authoritative_voxels_including_tile_seams() {
    let showcase = map();
    let mut checked = 0usize;
    let mut columns: Vec<(f32, f32)> = Vec::new();
    // Interior sample lattice.
    for xi in (2..MAP_EDGE_CELLS - 2).step_by(37) {
        for zi in (2..MAP_EDGE_CELLS - 2).step_by(41) {
            columns.push((
                (xi as f32 + 0.5) * TERRAIN_CELL_M,
                (zi as f32 + 0.5) * TERRAIN_CELL_M,
            ));
        }
    }
    // Both sides of every 8 m tile seam along two scanlines.
    for tile in 1..16 {
        let seam = tile as f32 * 8.0;
        for offset in [-0.125f32, 0.125] {
            columns.push((seam + offset, 61.0));
            columns.push((37.0, seam + offset));
        }
    }

    for (x, z) in columns {
        let surface = showcase
            .terrain
            .surface_at_metres(x, z)
            .expect("in-map column");
        let top_y = (surface.top_cell as f32 + 0.5) * TERRAIN_CELL_M;
        // The queried top cell is solid in the actual scene ...
        if carved(x, top_y, z) {
            continue; // carved cavities are checked by the overhang test
        }
        // Terrain instances only: an overlapping plant is a different question.
        let terrain_hit = showcase
            .scene
            .sample_all_world_metres([x, top_y, z])
            .expect("valid point")
            .into_iter()
            .find(|(id, _)| id.starts_with("i_shell_") || id.starts_with("base_"));
        let (_, found) =
            terrain_hit.unwrap_or_else(|| panic!("no terrain voxel at queried surface {x},{z}"));
        assert_eq!(
            found, surface.material,
            "query material disagrees with source at {x},{z}"
        );
        // ... and nothing solid sits above it except declared water.
        let above = showcase
            .scene
            .sample_all_world_metres([x, top_y + TERRAIN_CELL_M, z])
            .expect("valid point")
            .into_iter()
            .find(|(id, _)| id.starts_with("i_shell_") || id.starts_with("base_"));
        if let Some((_, above_material)) = above {
            assert_eq!(
                material_policy(above_material),
                MaterialPolicy::Liquid,
                "solid {above_material} above the reported surface at {x},{z}"
            );
            assert!(surface.water_depth_m > 0.0);
        } else {
            assert_eq!(surface.water_depth_m, 0.0);
        }
        checked += 1;
    }
    assert!(checked > 150, "only {checked} columns checked");
}

#[test]
fn water_sits_on_terrain_with_a_visible_surface_and_a_declared_level() {
    let showcase = map();
    let counts = showcase.scene.counts();
    assert!(
        counts.expanded_liquid_cells > 50_000,
        "water body is too small to read: {} cells",
        counts.expanded_liquid_cells
    );
    // Water is never a physical wall.
    assert_eq!(material_policy(material::WATER), MaterialPolicy::Liquid);

    let mut flooded = 0usize;
    for xi in (0..MAP_EDGE_CELLS).step_by(29) {
        for zi in (0..MAP_EDGE_CELLS).step_by(31) {
            let x = (xi as f32 + 0.5) * TERRAIN_CELL_M;
            let z = (zi as f32 + 0.5) * TERRAIN_CELL_M;
            let surface = showcase.terrain.surface_at_metres(x, z).expect("in map");
            let Some(water) = showcase.terrain.water_surface_at_metres(x, z) else {
                assert_eq!(surface.water_depth_m, 0.0);
                continue;
            };
            flooded += 1;
            assert!(
                water > surface.height_m,
                "water surface {water} is not above the bed {} at {x},{z}",
                surface.height_m
            );
            assert!(
                water <= 20.0 + TERRAIN_CELL_M,
                "water surface {water} above the creek head level"
            );
            // Submerged ground is bank stone, so the bed reads as wet rock.
            assert_eq!(surface.material, material::BANK_STONE);
        }
    }
    assert!(flooded > 20, "only {flooded} flooded sample columns");
    assert!(BASIN_WATER_LEVEL_M > 0.0);
}

#[test]
fn out_of_map_and_negative_inputs_are_rejected_not_clamped() {
    let showcase = map();
    for (x, z) in [
        (-0.1f32, 40.0f32),
        (40.0, -0.1),
        (-64.0, -64.0),
        (MAP_EDGE_M, 40.0),
        (40.0, MAP_EDGE_M + 12.0),
        (f32::NAN, 10.0),
        (10.0, f32::INFINITY),
    ] {
        assert!(
            showcase.terrain.surface_at_metres(x, z).is_none(),
            "surface query accepted out-of-map point {x},{z}"
        );
        assert!(showcase.terrain.height_at_metres(x, z).is_none());
    }
    // A valid metre point outside the content is a miss, not an error ...
    assert_eq!(
        showcase.scene.sample_world_metres([-4.0, 6.0, -4.0]),
        Ok(None)
    );
    // ... while a non-finite point is an explicit error.
    assert!(showcase
        .scene
        .sample_world_metres([f32::NAN, 0.0, 0.0])
        .is_err());
    assert!(!showcase
        .scene
        .is_collidable_world_metres([-4.0, 6.0, -4.0])
        .expect("valid point"));
}

#[test]
fn carved_cavities_are_real_voxel_overhangs() {
    let showcase = map();
    assert!(
        showcase.manifest.overhang_columns >= 500,
        "only {} columns have solid material above air",
        showcase.manifest.overhang_columns
    );
    let cavities: Vec<_> = showcase
        .landmarks
        .iter()
        .filter(|l| l.kind == "cavity")
        .collect();
    assert_eq!(cavities.len(), 3);
    for landmark in cavities {
        let [x, y, z] = landmark.position_m;
        assert!(
            showcase
                .scene
                .sample_world_metres([x, y, z])
                .expect("valid point")
                .is_none(),
            "{} interior is not open",
            landmark.name
        );
        // A real roof somewhere over the cavity footprint: a column where solid
        // voxels sit above open air, which a heightfield cannot represent.
        let mut roof_columns = 0usize;
        for ox in -4..=4i32 {
            for oz in -4..=4i32 {
                let (cx, cz) = (x + ox as f32 * 0.75, z + oz as f32 * 0.75);
                if !carved(cx, y, cz) {
                    continue;
                }
                let open = showcase
                    .scene
                    .sample_world_metres([cx, y, cz])
                    .expect("valid point")
                    .is_none();
                let solid_above = (1..=24).any(|step| {
                    showcase
                        .scene
                        .is_collidable_world_metres([cx, y + step as f32 * TERRAIN_CELL_M, cz])
                        .unwrap_or(false)
                });
                if open && solid_above {
                    roof_columns += 1;
                }
            }
        }
        assert!(
            roof_columns >= 4,
            "{} has only {roof_columns} roofed columns",
            landmark.name
        );
    }
}

// ---------------------------------------------------------------------------
// Route, spawn and landmark APIs
// ---------------------------------------------------------------------------

#[test]
fn ground_loop_is_a_closed_walkable_route_of_the_required_duration() {
    let showcase = map();
    assert!(showcase.route.len() > 200, "route is too coarse");
    let first = showcase.route.first().copied().expect("route start");
    let last = showcase.route.last().copied().expect("route end");
    let closing = ((first[0] - last[0]).powi(2) + (first[2] - last[2]).powi(2)).sqrt();
    assert!(closing < 3.0, "route does not close: {closing} m apart");

    let seconds = showcase.route_walk_seconds();
    assert!(
        (180.0..=300.0).contains(&seconds),
        "loop takes {seconds:.0} s at {WALK_SPEED_M_S} m/s; target is a 3-5 minute walk"
    );
    assert!((showcase.route_length_m() - showcase.manifest.route_length_m).abs() < 0.5);

    // Elevated alternate route exists and is genuinely higher than the basin.
    assert!(showcase.elevated_route.len() > 20);
    let max_elevated = showcase
        .elevated_route
        .iter()
        .map(|p| p[1])
        .fold(f32::NEG_INFINITY, f32::max);
    assert!(
        max_elevated > BASIN_WATER_LEVEL_M + 12.0,
        "elevated route peaks at {max_elevated} m"
    );
}

#[test]
fn every_route_point_has_ground_under_it_and_clearance_above_it() {
    let showcase = map();
    let mut checked = 0usize;
    for point in showcase.route.iter().step_by(2) {
        let [x, y, z] = *point;
        let surface = showcase
            .terrain
            .surface_at_metres(x, z)
            .expect("route point inside the map");
        assert!((surface.height_m - y).abs() < 1e-3, "route point floats");
        assert!(surface.is_dry_land(), "route point {x},{z} is under water");
        assert!(surface.slope <= 0.85, "route point {x},{z} is a cliff");
        // Footing: the cell under the foot is collidable.
        assert!(
            showcase
                .scene
                .is_collidable_world_metres([x, y - TERRAIN_CELL_M * 0.5, z])
                .expect("valid point"),
            "no footing under route point {x},{z}"
        );
        // Head clearance: nothing collidable in the walking body volume.
        for step in 1..=6 {
            let head = y + step as f32 * 0.3;
            assert!(
                !showcase
                    .scene
                    .is_collidable_world_metres([x, head, z])
                    .expect("valid point"),
                "route point {x},{z} is blocked at +{:.1} m",
                step as f32 * 0.3
            );
        }
        checked += 1;
    }
    assert!(checked > 120, "only {checked} route points checked");
}

#[test]
fn spawn_and_landmarks_are_usable_positions() {
    let showcase = map();
    let [sx, sy, sz] = showcase.spawn_eye;
    let surface = showcase
        .terrain
        .surface_at_metres(sx, sz)
        .expect("spawn inside the map");
    assert!(surface.is_dry_land());
    assert!(sy > surface.height_m + 1.0 && sy < surface.height_m + 2.5);
    assert!(!showcase
        .scene
        .is_collidable_world_metres([sx, sy, sz])
        .expect("valid point"));

    let names: Vec<&str> = showcase.landmarks.iter().map(|l| l.name).collect();
    for required in [
        "basin",
        "creek_mouth",
        "creek_head",
        "fungal_grove",
        "destruction_clearing",
        "high_viewpoint",
        "west_rock_shelter",
        "west_slope_undercut",
    ] {
        assert!(names.contains(&required), "missing landmark {required}");
    }
    for landmark in &showcase.landmarks {
        assert!(landmark.position_m.iter().all(|v| v.is_finite()));
        assert!(landmark.position_m[0] >= 0.0 && landmark.position_m[0] <= MAP_EDGE_M);
        assert!(landmark.position_m[2] >= 0.0 && landmark.position_m[2] <= MAP_EDGE_M);
    }
}

// ---------------------------------------------------------------------------
// Placement quality and determinism
// ---------------------------------------------------------------------------

#[test]
fn flora_is_rooted_clustered_and_off_the_route() {
    let showcase = map();
    let mut flora = 0usize;
    let mut cluster_cells = std::collections::BTreeSet::new();
    for draw in showcase.scene.draws() {
        if showcase_class(&draw.prototype) == "terrain" {
            continue;
        }
        flora += 1;
        let [x, y, z] = draw.transform.translation_m;
        let surface = showcase
            .terrain
            .surface_at_metres(x, z)
            .expect("plant inside the map");
        // Rooted exactly on the ground: no float, no sink, slope included.
        assert!(
            (y - surface.height_m).abs() < 1e-3,
            "plant {} floats by {} m",
            draw.instance,
            y - surface.height_m
        );
        assert!(surface.is_dry_land(), "plant {} stands in water", draw.instance);
        assert!(surface.slope <= 0.85);
        assert!(
            surface.material == material::MOSS_TURF || surface.material == material::DETAIL_SOIL,
            "plant {} is rooted in {}",
            draw.instance,
            surface.material
        );
        cluster_cells.insert(((x / 8.0) as i32, (z / 8.0) as i32));
    }
    assert_eq!(flora, showcase.manifest.flora_instances);
    // Clustered, not a uniform grid over the whole map: plants occupy a clear
    // minority of the 8 m cells, yet reach many distinct areas.
    assert!(
        cluster_cells.len() >= 40 && cluster_cells.len() <= 220,
        "plants occupy {} of 256 tiles, which is not habitat clustering",
        cluster_cells.len()
    );
}

#[test]
fn generation_is_deterministic_for_the_frozen_seed() {
    let first = map();
    let second = build_showcase(SHOWCASE_SEED).expect("second generation");

    assert_eq!(composition_hash(first), composition_hash(&second));
    assert_eq!(first.scene.counts(), second.scene.counts());
    assert_eq!(first.manifest, second.manifest);
    assert_eq!(first.spawn_eye, second.spawn_eye);
    assert_eq!(first.route, second.route);
    assert_eq!(first.landmarks, second.landmarks);

    // Voxel layout, not only aggregate counts.
    for id in [BASE_ROCK_ID, "shell_5_9_2", "parasol_mushroom"] {
        assert_eq!(
            prototype_content_hash(&first.scene, id),
            prototype_content_hash(&second.scene, id),
            "{id} content differs between runs"
        );
    }
}

#[test]
fn a_different_seed_produces_a_different_map() {
    let showcase = map();
    let other = build_showcase(SHOWCASE_SEED + 1).expect("alternate seed generation");
    assert_ne!(composition_hash(showcase), composition_hash(&other));
    assert_eq!(other.manifest.seed, SHOWCASE_SEED + 1);
    // The alternate seed must still satisfy the same acceptance criteria, so the
    // thresholds are a property of the generator, not of one lucky seed.
    assert!(other.scene.counts().expanded_occupied_cells >= SHOWCASE_EXPANDED_CELLS_MIN);
    assert!(other.manifest.flora_expanded_cells >= SHOWCASE_FLORA_CELLS_MIN);
    assert!(other.manifest.flora_instances >= SHOWCASE_PLANTS_MIN);
}
