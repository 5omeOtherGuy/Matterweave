//! Landscape generator acceptance: determinism, relief, biomes, water, tiles,
//! flora population and the authoritative streaming path that uses them.
use matterweave_core::landscape::{
    self, Biome, Clip, TileFilter, FLORA_CELL_M, LANDSCAPE_GENERATOR_VERSION, MAX_SURFACE_Y,
    MIN_SURFACE_Y, SEA_LEVEL,
};
use matterweave_core::{material, Mesh, TerrainSource, World, CHUNK_EDGE};
use std::collections::BTreeMap;

const SEED: u64 = 20260913;

/// A stable digest of a sampled region. Used to detect accidental drift in the
/// generator: a deliberate change updates this constant with the change.
fn fingerprint(seed: u64, edge: i32) -> u64 {
    let mut value = 0xcbf2_9ce4_8422_2325u64;
    for z in -edge..edge {
        for x in -edge..edge {
            let column = landscape::column(seed, x * 7, z * 7);
            for part in [
                column.height as i64,
                column.surface as i64,
                column.sub_surface as i64,
                column.sub_depth as i64,
                column.water_level as i64,
                column.biome as i64,
            ] {
                value ^= part as u64;
                value = value.wrapping_mul(0x1000_0000_01b3);
            }
        }
    }
    value
}

#[test]
fn generation_is_deterministic_and_seed_dependent() {
    assert_eq!(LANDSCAPE_GENERATOR_VERSION, 1);
    for (x, z) in [(0, 0), (1, -1), (1000, -2000), (-9999, 30000)] {
        assert_eq!(
            landscape::column(SEED, x, z),
            landscape::column(SEED, x, z),
            "equal input must give an equal column"
        );
        assert_eq!(
            landscape::height_at(SEED, x, z),
            landscape::column(SEED, x, z).height
        );
        assert_eq!(
            landscape::biome_at(SEED, x, z),
            landscape::column(SEED, x, z).biome
        );
    }
    let a = fingerprint(SEED, 12);
    let b = fingerprint(SEED + 1, 12);
    assert_ne!(a, b, "a different seed must produce different terrain");
    assert_eq!(a, fingerprint(SEED, 12), "the region digest must be stable");
}

#[test]
fn generator_fingerprint_is_stable() {
    // Golden value: any change to the noise fields, biome rules or material
    // tables moves it. Updating the constant is a deliberate generator change
    // (and a LANDSCAPE_GENERATOR_VERSION bump), never a test fix.
    assert_eq!(
        fingerprint(SEED, 12),
        0x3e12_047e_a85c_b89e,
        "landscape output drifted from the recorded generator identity"
    );
}

#[test]
fn surfaces_stay_inside_the_declared_range() {
    let (mut low, mut high) = (i32::MAX, i32::MIN);
    for z in -200..200 {
        for x in -200..200 {
            let column = landscape::column(SEED, x * 13, z * 13);
            assert!(
                (MIN_SURFACE_Y..=MAX_SURFACE_Y).contains(&column.height),
                "{} at ({x}, {z})",
                column.height
            );
            low = low.min(column.height);
            high = high.max(column.height);
        }
    }
    assert!(
        low < -20,
        "the sample region must include deep water, lowest was {low}"
    );
    assert!(
        high > 90,
        "the sample region must include high peaks, highest was {high}"
    );
}

#[test]
fn terrain_has_mountains_without_vertical_cliffs() {
    // The ridged field is intentionally steep: its finest octave can move a vertex
    // several metres per metre. A step far beyond that would mean a field is being
    // sampled on the wrong lattice rather than a mountain being sharp.
    let mut steepest = 0;
    for z in -300..300 {
        for x in -300..300 {
            let here = landscape::column(SEED, x * 5, z * 5).height;
            let next = landscape::column(SEED, x * 5 + 1, z * 5).height;
            steepest = steepest.max((here - next).abs());
        }
    }
    assert!(
        (1..=10).contains(&steepest),
        "one metre must not step more than ten metres, steepest was {steepest}"
    );
}

#[test]
fn biomes_cover_the_climate_space() {
    let mut counts: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut total = 0usize;
    for z in -400..400 {
        for x in -400..400 {
            let biome = landscape::biome_at(SEED, x * 11, z * 11);
            *counts.entry(biome.name()).or_default() += 1;
            total += 1;
        }
    }
    for name in [
        "ocean", "beach", "plains", "forest", "hills", "mountain", "snow", "tundra", "desert",
        "swamp",
    ] {
        let count = counts.get(name).copied().unwrap_or(0);
        assert!(
            count > 0,
            "biome {name} never appears across {total} samples: {counts:?}"
        );
    }
    // Oceans and land must both be substantial; a generator that floods or dries
    // the whole world is a bug even when every biome name is present once.
    let ocean = counts.get("ocean").copied().unwrap_or(0);
    assert!(
        (total / 20..total * 4 / 5).contains(&ocean),
        "ocean fraction is implausible: {ocean}/{total}"
    );
    let mountains =
        counts.get("mountain").copied().unwrap_or(0) + counts.get("snow").copied().unwrap_or(0);
    assert!(
        mountains * 50 > total,
        "mountains and snow must be at least 2% of the world: {mountains}/{total}"
    );
}

#[test]
fn flooded_columns_report_water_and_never_store_it() {
    let mut flooded = 0;
    let mut sampled = 0;
    for z in -150..150 {
        for x in -150..150 {
            let column = landscape::column(SEED, x * 23, z * 23);
            sampled += 1;
            if column.flooded() {
                flooded += 1;
                assert_eq!(column.water_level, SEA_LEVEL);
                assert!(column.height < SEA_LEVEL);
                assert!(column.water_depth() > 0);
                // Above the ground and below the surface: still nothing stored.
                assert_eq!(
                    landscape::material_at(SEED, x * 23, column.height + 1, z * 23),
                    material::AIR
                );
            } else {
                assert_eq!(column.water_level, landscape::NO_WATER);
                assert_eq!(column.water_depth(), 0);
            }
        }
    }
    assert!(flooded > 0, "the sample region must include water");
    assert!(flooded < sampled, "the sample region must include land");
}

#[test]
fn material_profile_matches_the_surface_column() {
    for (x, z) in [(0, 0), (37, -91), (512, 512), (-777, 333)] {
        let column = landscape::column(SEED, x, z);
        assert_eq!(
            landscape::material_at(SEED, x, column.height, z),
            column.surface
        );
        assert_eq!(
            landscape::material_at(SEED, x, column.height + 1, z),
            material::AIR
        );
        // The per-voxel entry point and the chunk fill must agree everywhere.
        for y in (column.height - 8).max(MIN_SURFACE_Y)..=column.height {
            assert_eq!(
                landscape::material_at(SEED, x, y, z),
                landscape::material_in_column(SEED, &column, x, y, z),
                "({x}, {y}, {z})"
            );
        }
    }
}

#[test]
fn flora_population_follows_the_biome() {
    let mut grass = 0usize;
    let mut desert = 0usize;
    let mut kinds: BTreeMap<&'static str, usize> = BTreeMap::new();
    for cell in -400..400 {
        let flora = landscape::flora_cell(SEED, cell, -cell / 2);
        let biome = landscape::biome_at(SEED, cell * FLORA_CELL_M, (-cell / 2) * FLORA_CELL_M);
        for site in flora.sites() {
            *kinds.entry(site.kind.name()).or_default() += 1;
            assert_eq!(
                site.y,
                landscape::height_at(SEED, site.x, site.z),
                "a plant must stand on its own column"
            );
            match biome {
                Biome::Plains | Biome::Forest | Biome::Hills => grass += 1,
                Biome::Desert => desert += 1,
                _ => {}
            }
        }
    }
    assert!(kinds.contains_key("grass_tuft"), "{kinds:?}");
    assert!(
        kinds.keys().any(|kind| kind.starts_with("flower_")),
        "flowers must appear somewhere: {kinds:?}"
    );
    assert!(
        grass > desert * 8,
        "plains/forest/hills must carry far more grass than desert: {grass} vs {desert}"
    );
}

#[test]
fn forests_request_trees_and_cold_ground_requests_conifers() {
    let mut broadleaf = 0usize;
    let mut conifer = 0usize;
    let mut forest_cells = 0usize;
    for cell in -300..300 {
        let site = landscape::tree_cell(SEED, cell, -cell / 3);
        let biome = landscape::biome_at(SEED, cell * 8, (-cell / 3) * 8);
        if biome == Biome::Forest {
            forest_cells += 1;
        }
        if let Some(site) = site {
            // A tree stands on its own column, at most one per cell.
            assert_eq!(site.y, landscape::height_at(SEED, site.x, site.z));
            assert!((cell * 8..cell * 8 + 8).contains(&site.x) || site.x >= cell * 8);
            match site.kind {
                landscape::FloraKind::TreeBroadleaf => broadleaf += 1,
                landscape::FloraKind::TreeConifer => conifer += 1,
                other => panic!("tree_cell returned a non-tree: {other:?}"),
            }
        }
    }
    assert!(forest_cells > 0, "the sample region must contain forest");
    assert!(
        broadleaf > 0 && conifer > 0,
        "broadleaf {broadleaf}, conifer {conifer}"
    );
}

#[test]
fn flora_cells_are_deterministic_and_bounded() {
    for cell in -50..50 {
        let a = landscape::flora_cell(SEED, cell, cell + 3);
        let b = landscape::flora_cell(SEED, cell, cell + 3);
        assert_eq!(a.len(), b.len());
        assert!(a.len() <= landscape::MAX_FLORA_PER_CELL);
        for (left, right) in a.sites().iter().zip(b.sites()) {
            assert_eq!(left, right);
        }
    }
}

#[test]
fn lod_vertices_match_the_fine_surface() {
    for level in 0..=landscape::MAX_LOD_LEVEL {
        let step = landscape::lod_cell_m(level);
        for (x, z) in [
            (0, 0),
            (37 * step, -91 * step),
            (step * 5 + step / 2, step * 3),
        ] {
            let sample = landscape::lod_vertex(SEED, x, z);
            let column = landscape::column(SEED, x, z);
            if column.flooded() {
                assert_eq!(sample.height, SEA_LEVEL);
                assert_eq!(sample.surface, material::WATER);
                assert!(sample.flooded);
            } else {
                assert_eq!(sample.height, column.height);
                assert_eq!(sample.surface, column.surface);
                assert_eq!(sample.biome, column.biome);
                assert!(!sample.flooded);
            }
        }
    }
}

#[test]
fn lod_samples_are_conservative_aggregates() {
    for level in 0..=landscape::MAX_LOD_LEVEL {
        let cell = landscape::lod_cell_m(level);
        let lattice = (cell / 16).max(1);
        let slack = lattice * 10;
        for (cx, cz) in [(0, 0), (3, -5), (-11, 7)] {
            let sample = landscape::lod_sample(SEED, level, cx, cz);
            let mut highest = MIN_SURFACE_Y;
            let mut flooded = true;
            for dz in 0..cell {
                for dx in 0..cell {
                    let column = landscape::column(SEED, cx * cell + dx, cz * cell + dz);
                    highest = highest.max(column.height);
                    flooded &= column.flooded();
                }
            }
            if flooded {
                assert_eq!(sample.height, SEA_LEVEL);
            } else {
                // Levels up to 16 m sample every column; coarser levels sample a
                // bounded lattice and may miss relief narrower than their step.
                assert!(
                    sample.height <= highest,
                    "level {level} cell ({cx}, {cz}) reported {sample:?} above the true maximum {highest}"
                );
                assert!(
                    sample.height + slack >= highest,
                    "level {level} cell ({cx}, {cz}) missed {highest} by more than {slack}: {sample:?}"
                );
                if level <= 4 {
                    assert_eq!(sample.height, highest);
                }
            }
        }
    }
}

fn tile_edge_heights(mesh: &Mesh, axis: usize, value: f32) -> Vec<(f32, f32)> {
    let mut heights = Vec::new();
    for vertex in &mesh.vertices {
        if (vertex.position[axis] - value).abs() < 1.0e-3 {
            heights.push((vertex.position[1 - axis.min(1)], vertex.position[1]));
        }
    }
    heights.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    heights
}

#[test]
fn lod_tiles_share_their_edges_exactly() {
    // Two horizontally adjacent level-1 tiles must report identical heights on
    // the shared edge; otherwise a fine/coarse seam could open at run time.
    let level = 1;
    let cell = landscape::lod_cell_m(level);
    for key_x in [-3, 0, 4] {
        let left = landscape::lod_tile_mesh(SEED, level, [key_x, 1], TileFilter::default());
        let right = landscape::lod_tile_mesh(SEED, level, [key_x + 1, 1], TileFilter::default());
        assert!(!left.vertices.is_empty() && !right.vertices.is_empty());
        let edge_x = ((key_x + 1) * landscape::LOD_TILE_CELLS) as f32 * cell as f32;
        let left_edge = tile_edge_heights(&left, 0, edge_x);
        let right_edge = tile_edge_heights(&right, 0, edge_x);
        assert_eq!(
            left_edge.len(),
            right_edge.len(),
            "shared edge vertex count must match"
        );
        for ((z, height), (other_z, other)) in left_edge.iter().zip(&right_edge) {
            assert!((z - other_z).abs() < 1.0e-3, "edge z mismatch");
            assert!(
                (height - other).abs() < 1.0e-3,
                "tile edge height mismatch at z {z}: {height} vs {other}"
            );
        }
    }
}

#[test]
fn lod_tiles_are_well_formed_and_respect_their_filter() {
    let level = 2;
    let cell = landscape::lod_cell_m(level);
    let key = [2, -3];
    let span = landscape::LOD_TILE_CELLS * cell;
    let full = landscape::lod_tile_mesh(SEED, level, key, TileFilter::default());
    assert!(!full.vertices.is_empty());
    assert_eq!(full.indices.len() % 6, 0);
    for index in &full.indices {
        assert!((*index as usize) < full.vertices.len());
    }
    for vertex in &full.vertices {
        assert!(vertex.position.iter().all(|v| v.is_finite()));
        let normal = vertex.normal;
        let length = (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
        assert!((length - 1.0).abs() < 1.0e-4, "normal {normal:?}");
        // Surface vertices stay inside the tile footprint (skirts only go down).
        let x = vertex.position[0] as i32;
        let z = vertex.position[2] as i32;
        assert!(
            (key[0] * span..=key[0] * span + span).contains(&x)
                && (key[1] * span..=key[1] * span + span).contains(&z),
            "vertex outside its tile footprint: {vertex:?}"
        );
    }
    // The bound removes the far quarter of the tile, so it must contain fewer
    // vertices, while a hole must keep the ring and add skirt geometry.
    let bound = Clip {
        min: [key[0] * span, key[1] * span],
        max: [key[0] * span + span / 2, key[1] * span + span],
    };
    let bounded = landscape::lod_tile_mesh(
        SEED,
        level,
        key,
        TileFilter {
            bound: Some(bound),
            ..TileFilter::default()
        },
    );
    assert!(bounded.vertices.len() < full.vertices.len());
    assert!(!bounded.vertices.is_empty());
    for vertex in &bounded.vertices {
        assert!(
            (vertex.position[0] as i32) <= bound.max[0],
            "bound must exclude the far cells: {vertex:?}"
        );
    }
    let hole = Clip {
        min: [key[0] * span, key[1] * span],
        max: [key[0] * span + span / 2, key[1] * span + span / 2],
    };
    let holed = landscape::lod_tile_mesh(
        SEED,
        level,
        key,
        TileFilter {
            hole: Some(hole),
            ..TileFilter::default()
        },
    );
    assert!(
        !holed.vertices.is_empty(),
        "a hole must leave a ring behind"
    );
    assert!(holed.vertices.len() < full.vertices.len());
    // Every vertex of the ring must lie outside the hole or be skirt geometry
    // that shares a cell corner with the boundary.
    for vertex in &holed.vertices {
        let inside = (vertex.position[0] as i32) > hole.min[0]
            && (vertex.position[0] as i32) < hole.max[0]
            && (vertex.position[2] as i32) > hole.min[1]
            && (vertex.position[2] as i32) < hole.max[1];
        assert!(!inside, "hole cell leaked into the tile: {vertex:?}");
    }
    let everything = landscape::lod_tile_mesh(
        SEED,
        level,
        key,
        TileFilter {
            bound: Some(Clip {
                min: [0, 0],
                max: [0, 0],
            }),
            ..TileFilter::default()
        },
    );
    assert!(
        everything.vertices.is_empty(),
        "a cell filter that keeps nothing must produce an empty mesh"
    );
}

#[test]
fn streaming_landscape_uses_the_generator_and_the_wide_band() {
    let mut world = World::landscape(SEED);
    assert_eq!(world.terrain_source(), TerrainSource::Landscape);
    assert_eq!(world.stream_y_range(), (-48, 160));
    assert_eq!(world.stream_y_chunks(), -3..10);
    assert!(world.stream_around([8.0, world_surface(&world), 8.0]));
    let resident = world.stream_resident_chunks().expect("streaming world");
    assert_eq!(resident.len(), 49 * 13);
    assert!(world.stats().solid_voxels > 0);

    // Every resident chunk must be exactly what the generator says, layer by layer.
    let mut water_cells = 0;
    for key in world.chunk_keys() {
        for local_y in 0..CHUNK_EDGE {
            let y = key[1] * CHUNK_EDGE + local_y;
            for local_z in 0..CHUNK_EDGE {
                for local_x in 0..CHUNK_EDGE {
                    let x = key[0] * CHUNK_EDGE + local_x;
                    let z = key[2] * CHUNK_EDGE + local_z;
                    let expected = landscape::material_at(SEED, x, y, z);
                    assert_eq!(world.get([x, y, z]), expected, "({x}, {y}, {z})");
                    if world.get([x, y, z]) == material::WATER {
                        water_cells += 1;
                    }
                }
            }
        }
    }
    assert_eq!(water_cells, 0, "water must never be stored as a voxel");

    // A window prep must stay bounded: it samples each metre-column once for the
    // whole band. A generous host bound catches accidental quadratic work rather
    // than asserting a device timing.
    let start = std::time::Instant::now();
    let mut second = World::landscape(SEED);
    assert!(second.stream_around([200.0, 40.0, -160.0]));
    let elapsed = start.elapsed();
    assert!(
        elapsed.as_millis() < 1_500,
        "a fresh landscape window took {elapsed:?}"
    );
}

fn world_surface(world: &World) -> f32 {
    landscape::height_at(world.seed(), 8, 8) as f32
}

#[test]
fn landscape_worlds_round_trip_through_a_save() {
    let directory = std::env::temp_dir().join(format!(
        "matterweave-landscape-save-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("landscape.json");
    let mut world = World::landscape(SEED);
    world.stream_around([0.0, 30.0, 0.0]);
    let edit = [3, landscape::height_at(SEED, 3, 3), 3];
    assert!(world.set(edit, material::WOOD));
    world.save(&path).unwrap();

    let restored = World::load(&path).unwrap();
    assert_eq!(restored.terrain_source(), TerrainSource::Landscape);
    assert_eq!(restored.stream_y_range(), (-48, 160));
    assert_eq!(restored.seed(), world.seed());
    assert_eq!(restored.get(edit), material::WOOD);
    // Residency is reconstructed from the saved center with the landscape band.
    assert_eq!(
        restored.stream_resident_chunks().map(|keys| keys.len()),
        world.stream_resident_chunks().map(|keys| keys.len())
    );
    std::fs::remove_dir_all(&directory).ok();
}

#[test]
fn pre_landscape_saves_restore_as_the_legacy_island() {
    let directory = std::env::temp_dir().join(format!(
        "matterweave-legacy-save-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("legacy.json");
    let world = World::generate(SEED);
    world.save(&path).unwrap();
    // A format-2 document (no terrain source) must load as the legacy island.
    let text = std::fs::read_to_string(&path).unwrap();
    let downgraded = text
        .replace("\"format_version\": 3", "\"format_version\": 2")
        .replace(",\"terrain_source\":\"legacy_island\"", "")
        .replace("\"terrain_source\":\"legacy_island\",", "")
        .replace("\"terrain_source\":\"legacy_island\"", "");
    assert!(
        !downgraded.contains("terrain_source"),
        "the downgraded document must not carry a terrain source"
    );
    std::fs::write(&path, downgraded).unwrap();
    let restored = World::load(&path).unwrap();
    assert_eq!(restored.terrain_source(), TerrainSource::LegacyIsland);
    assert_eq!(restored.stream_y_range(), (-16, 32));
    assert_eq!(restored.get([0, 3, 0]), world.get([0, 3, 0]));
    std::fs::remove_dir_all(&directory).ok();
}
