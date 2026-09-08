//! Acceptance tests for the full 128 m showcase map.
//!
//! The map is generated ONCE per test binary and shared, because a full
//! generation is the expensive part of this suite. Only the determinism test
//! pays for a second generation, which is exactly what it is measuring.

use matterweave_detail::{
    build_showcase, carved, composition_hash, material, material_policy, showcase_class,
    species_source_radius_m, DetailScene, Lod, MaterialPolicy, Showcase, BASE_ROCK_ID,
    BASE_SOIL_ID, BASIN_WATER_LEVEL_M, LILY_PAD_HEIGHT_M, MAP_EDGE_CELLS, MAP_EDGE_M,
    MAX_PROTOTYPE_CELLS, MAX_SCENE_CACHE_BYTES, MAX_SCENE_SOURCE_BYTES, ROUTE_PLAYER_CLEARANCE_M,
    SHOWCASE_EXPANDED_CELLS_MIN, SHOWCASE_FLORA_CELLS_MIN, SHOWCASE_PLANTS_MIN, SHOWCASE_SEED,
    SHOWCASE_SPECIES, SPECIES_LILY, TERRAIN_CELL_M, TILE_CELLS, WALK_SPEED_M_S,
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

    // All ten archetypes are actually placed: the six accepted catalogue
    // species plus the four later originals. This must fail loudly if any of
    // them is missing or is only token-present.
    assert_eq!(
        manifest.species_instances.len(),
        SHOWCASE_SPECIES.len(),
        "placed species {:?}",
        manifest.species_instances.keys().collect::<Vec<_>>()
    );
    for id in SHOWCASE_SPECIES {
        let placed = manifest
            .species_instances
            .get(id)
            .copied()
            .unwrap_or_else(|| panic!("{id} was never placed"));
        assert!(placed >= 100, "{id} is token-present with {placed}");
    }
    println!("species instances: {:?}", manifest.species_instances);
    println!(
        "classes: {:?}",
        manifest
            .classes
            .iter()
            .map(|(k, v)| (k.clone(), v.instances, v.expanded_cells))
            .collect::<Vec<_>>()
    );

    // Woody and fungal populations are reported separately, so no manifest can
    // present shrubs as mushrooms.
    let woody = manifest.classes.get("flora-woody").expect("woody class");
    assert!(
        woody.instances >= 100,
        "woody instances {}",
        woody.instances
    );
    let fungus = manifest.classes.get("flora-fungus").expect("fungus class");
    assert!(fungus.instances >= 500);
    let fungus_species: usize = [
        "parasol_mushroom",
        "funnel_mushroom",
        "clustered_mushroom",
        "bracket_fungus",
    ]
    .iter()
    .map(|id| manifest.species_instances[*id])
    .sum();
    assert_eq!(
        fungus.instances, fungus_species,
        "fungus class instances must be exactly the four fungal archetypes"
    );
    assert_eq!(woody.instances, manifest.species_instances["twisted_shrub"]);

    // Unique prototype cells and instance-expanded cells are counted from
    // different sources and must not be swapped: one prototype per archetype.
    let flora_prototypes: usize = manifest
        .classes
        .iter()
        .filter(|(name, _)| *name != "terrain")
        .map(|(_, c)| c.prototypes)
        .sum();
    assert_eq!(flora_prototypes, SHOWCASE_SPECIES.len());
    let flora_instances: usize = manifest
        .classes
        .iter()
        .filter(|(name, _)| *name != "terrain")
        .map(|(_, c)| c.instances)
        .sum();
    assert_eq!(flora_instances, manifest.flora_instances);
    assert_eq!(
        manifest.species_instances.values().sum::<usize>(),
        manifest.flora_instances
    );
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
    assert!(
        min[0] <= 1.0 && min[2] <= 1.0,
        "map does not start at origin"
    );
    assert!(
        max[0] >= MAP_EDGE_M - 8.0 && max[2] >= MAP_EDGE_M - 8.0,
        "placed content spans only {max:?}, not a 128 m map"
    );
    // Vertical extent must be real relief, not a flat slab.
    assert!(
        max[1] - min[1] >= 24.0,
        "vertical extent {}",
        max[1] - min[1]
    );
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
        aggregate += showcase.scene.cached_mesh_bytes();
        showcase.scene.invalidate(&id);
        assert_eq!(showcase.scene.cached_mesh_bytes(), 0);
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
        // The reported surface cell is never a cell the generator carved away.
        assert!(
            !carved(x, top_y, z),
            "surface query at {x},{z} reports carved-away cell {top_y}"
        );
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
    for xi in (0..MAP_EDGE_CELLS).step_by(17) {
        for zi in (0..MAP_EDGE_CELLS).step_by(19) {
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
    const { assert!(BASIN_WATER_LEVEL_M > 0.0) };
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

/// The surface query must report the highest *uncarved* solid cell, never the
/// phantom pre-carve heightfield top, in every column a cavity intersects.
#[test]
fn surface_query_reports_real_solid_in_cavity_intersected_columns() {
    let showcase = map();
    // Terrain queries exclude independently placed plants. A mushroom cap may
    // validly occupy the air above a carved terrain column (regression below).
    let terrain: Vec<_> = showcase
        .scene
        .draws()
        .into_iter()
        .filter(|d| showcase_class(&d.prototype) == "terrain")
        .map(|d| {
            let source = showcase.scene.prototype(&d.prototype).unwrap();
            let bounds = source.bounds_world(&d.transform).unwrap().unwrap();
            (d.transform, source, bounds)
        })
        .collect();
    let solid_terrain = |point: [f32; 3]| {
        terrain.iter().any(|(transform, source, bounds)| {
            (0..3).all(|a| point[a] >= bounds.min[a] && point[a] < bounds.max[a])
                && material_policy(source.sample_world_metres(transform, point).unwrap())
                    == MaterialPolicy::Collision
        })
    };
    let mut lowered = 0usize;
    let mut roofed = 0usize;
    for xi in 0..MAP_EDGE_CELLS {
        let x = (xi as f32 + 0.5) * TERRAIN_CELL_M;
        for zi in 0..MAP_EDGE_CELLS {
            let z = (zi as f32 + 0.5) * TERRAIN_CELL_M;
            let surface = showcase.terrain.surface_at_metres(x, z).expect("in map");
            let top_y = (surface.top_cell as f32 + 0.5) * TERRAIN_CELL_M;
            // Never a carved cell, anywhere on the map. No column is skipped.
            assert!(!carved(x, top_y, z), "carved surface reported at {x},{z}");

            // The pre-carve heightfield top of this column, i.e. the phantom
            // height an uncarved heightfield query would have returned.
            let phantom = matterweave_detail::terrain_height_m(showcase.terrain.seed(), x, z);
            let phantom_top = (phantom / TERRAIN_CELL_M).floor() as i32 - 1;
            if phantom_top > surface.top_cell {
                lowered += 1;
                // Everything between the reported surface and the phantom top
                // was really carved away, and the voxels agree: no solid there.
                for y in (surface.top_cell + 1)..=phantom_top {
                    let cy = (y as f32 + 0.5) * TERRAIN_CELL_M;
                    assert!(carved(x, cy, z));
                    if surface.water_depth_m == 0.0 {
                        assert!(
                            !solid_terrain([x, cy, z]),
                            "solid voxel above the reported surface at {x},{z},{cy}"
                        );
                    }
                }
            }
            if surface.overhung {
                roofed += 1;
                // The reported top is solid, and there is real air below it.
                assert!(solid_terrain([x, top_y, z]));
                assert!((0..surface.top_cell)
                    .any(|y| { !solid_terrain([x, (y as f32 + 0.5) * TERRAIN_CELL_M, z]) }));
            }
        }
    }
    // Both cases must actually occur: cavity mouths that lower the surface, and
    // true overhangs that a heightfield cannot express.
    assert!(lowered > 0, "no column had its surface lowered by carving");
    assert!(roofed > 0, "no true overhang columns");
    assert_eq!(
        roofed, showcase.manifest.overhang_columns,
        "manifest overhang columns must be the unique (x, z) columns with solid above air"
    );
    println!("lowered columns {lowered}, overhang columns {roofed}");
}

#[test]
fn carved_cavities_are_real_voxel_overhangs() {
    let showcase = map();
    // Honest unique-column accounting: never more than one count per column,
    // and never more than the map has columns.
    assert!(
        showcase.manifest.overhang_columns >= 200,
        "only {} columns have solid material above air",
        showcase.manifest.overhang_columns
    );
    assert!(
        showcase.manifest.overhang_columns < (MAP_EDGE_CELLS * MAP_EDGE_CELLS) as usize,
        "overhang columns exceed the number of columns on the map"
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
        assert!(
            surface.water_depth_m <= 0.5,
            "route point {x},{z} exceeds the declared wading depth"
        );
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

/// Both routes are protected, and the protection accounts for each archetype's
/// own horizontal source radius, not only its placement origin: a wide cap or a
/// woody branch must not reach into the walked corridor.
#[test]
fn plant_source_geometry_clears_both_routes_including_its_own_radius() {
    let showcase = map();
    let mut radii = std::collections::BTreeMap::new();
    for id in SHOWCASE_SPECIES {
        let r = species_source_radius_m(id).expect("prototype builds");
        assert!(r > 0.0 && r < 4.0, "{id} source radius {r}");
        radii.insert(id.to_string(), r);
    }
    println!("source radii: {radii:?}");

    let mut checked = 0usize;
    for draw in showcase.scene.draws() {
        if showcase_class(&draw.prototype) == "terrain" {
            continue;
        }
        let radius = radii[&draw.prototype];
        let limit = ROUTE_PLAYER_CLEARANCE_M + radius;
        let [x, _, z] = draw.transform.translation_m;
        for (name, route) in [
            ("ground", &showcase.route),
            ("elevated", &showcase.elevated_route),
        ] {
            // Check intermediate positions as well as the stored route vertices.
            // Point-only exclusion misses plants halfway between the 2m samples.
            for point in route.windows(2).flat_map(|pair| {
                (0..=16).map(move |step| {
                    let t = step as f32 / 16.;
                    [
                        pair[0][0] + t * (pair[1][0] - pair[0][0]),
                        0.,
                        pair[0][2] + t * (pair[1][2] - pair[0][2]),
                    ]
                })
            }) {
                let d = ((point[0] - x).powi(2) + (point[2] - z).powi(2)).sqrt();
                assert!(
                    d >= limit,
                    "{} ({}) is {d:.2} m from the {name} route, inside its {limit:.2} m protected corridor",
                    draw.instance,
                    draw.prototype
                );
            }
        }
        checked += 1;
    }
    assert!(checked > 4_000, "only {checked} plants checked");
}

/// The elevated route is an OPEN polyline: it must actually reach its terminal
/// waypoint instead of stopping one densification step short of it.
#[test]
fn open_elevated_route_is_walkable_and_reaches_its_terminal_waypoint() {
    let showcase = map();
    let route = &showcase.elevated_route;
    assert!(route.len() > 20);
    let first = route.first().copied().expect("start");
    let last = route.last().copied().expect("end");
    // Terminal waypoint of the authored elevated spine (the western descent off
    // the viewpoint), snapped to walkable ground.
    let terminus = [92.0f32, 30.0f32];
    let gap = ((last[0] - terminus[0]).powi(2) + (last[2] - terminus[1]).powi(2)).sqrt();
    assert!(
        gap < 2.0,
        "open route ends {gap:.2} m from its terminal waypoint at {last:?}"
    );
    // An open route must not silently close on itself.
    let closing = ((last[0] - first[0]).powi(2) + (last[2] - first[2]).powi(2)).sqrt();
    assert!(closing > 4.0, "open route closed on itself");

    // Same walkability contract as the ground loop: footing under every point,
    // and a clear standing volume above it.
    for point in route {
        let [x, y, z] = *point;
        let surface = showcase.terrain.surface_at_metres(x, z).expect("in map");
        assert!((surface.height_m - y).abs() < 1e-3);
        assert!(surface.water_depth_m <= 0.5 && surface.slope <= 0.85);
        assert!(showcase
            .scene
            .is_collidable_world_metres([x, y - TERRAIN_CELL_M * 0.5, z])
            .expect("valid point"));
        for step in 1..=6 {
            assert!(
                !showcase
                    .scene
                    .is_collidable_world_metres([x, y + step as f32 * 0.3, z])
                    .expect("valid point"),
                "elevated route point {x},{z} is blocked above"
            );
        }
    }
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
        if draw.prototype == SPECIES_LILY {
            // The lily is the one aquatic archetype: it is rooted on the bed of
            // shallow standing water, its pads reach the water surface exactly,
            // and it never stands on dry ground.
            let water = showcase
                .terrain
                .water_surface_at_metres(x, z)
                .expect("lily stands in water");
            assert!(
                (y + LILY_PAD_HEIGHT_M - water).abs() < 1e-3,
                "lily pads at {} m do not float at the {water} m water surface",
                y + LILY_PAD_HEIGHT_M
            );
            assert!(y < water, "lily rhizome is not below the water surface");
            cluster_cells.insert(((x / 8.0) as i32, (z / 8.0) as i32));
            continue;
        }
        assert!(
            surface.is_dry_land(),
            "plant {} stands in water",
            draw.instance
        );
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

#[test]
fn cavity_column_discriminates_terrain_from_placed_flora() {
    let showcase = map();
    let point = [97.375, 16.125, 58.125];
    let mut terrain_solids = Vec::new();
    for draw in showcase.scene.draws() {
        let source = showcase.scene.prototype(&draw.prototype).unwrap();
        let sampled = source.sample_world_metres(&draw.transform, point).unwrap();
        if material_policy(sampled) == MaterialPolicy::Collision {
            eprintln!(
                "cavity intersection: {} / {} / material {}",
                draw.instance, draw.prototype, sampled
            );
            if showcase_class(&draw.prototype) == "terrain" {
                terrain_solids.push(draw.instance);
            }
        }
    }
    assert!(
        terrain_solids.is_empty(),
        "carved column contains terrain: {terrain_solids:?}"
    );
}

#[test]
fn source_radius_contains_every_occupied_voxel_corner() {
    for species in SHOWCASE_SPECIES {
        let source = matterweave_detail::showcase_prototype(species).unwrap();
        let radius = species_source_radius_m(species).unwrap();
        let scale = source.scale().metres();
        for (cell, _) in source.iter_cells() {
            for dx in [0, 1] {
                for dz in [0, 1] {
                    let x = (cell[0] + dx) as f32 * scale;
                    let z = (cell[2] + dz) as f32 * scale;
                    assert!(
                        x.hypot(z) <= radius + 1e-5,
                        "{species} corner [{x},{z}] exceeds protected radius {radius}"
                    );
                }
            }
        }
    }
}

#[test]
fn short_waterside_itinerary_is_a_recorded_contiguous_part_of_the_walked_loop() {
    let map = map();
    let waterside = map.waterside_route();
    assert!(waterside.len() >= 3, "short waterside itinerary is missing");
    assert_eq!(waterside, &map.route[..waterside.len()]);
    let length: f32 = waterside.windows(2).map(|p| {
        (p[1][0] - p[0][0]).hypot(p[1][2] - p[0][2])
    }).sum();
    assert!((20.0..=120.0).contains(&length), "waterside length {length}");
    assert!(waterside.iter().any(|p| map.terrain.surface_at_metres(p[0], p[2])
        .is_some_and(|surface| surface.water_depth_m > 0.0)),
        "itinerary never reaches the actual water margin");
    let end = waterside.last().unwrap();
    assert!((end[0] - 42.0).hypot(end[2] - 90.0) < 5.0,
        "waterside itinerary misses the west basin shore");
}
