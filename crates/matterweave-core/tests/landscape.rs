//! Landscape generator acceptance: determinism, relief, biomes, water, tiles,
//! flora population and the authoritative streaming path that uses them.
use matterweave_core::landscape::{
    self, Biome, Clip, RingTile, TileFilter, FLORA_CELL_M, LANDSCAPE_GENERATOR_VERSION,
    LANDSCAPE_RINGS, LOD_TILE_CELLS, MAX_SURFACE_Y, MIN_SURFACE_Y, SEA_LEVEL,
};
use matterweave_core::{material, Mesh, TerrainSource, World, CHUNK_EDGE};
use std::collections::{BTreeMap, BTreeSet};

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
fn material_flora_and_tile_fingerprint_is_stable() {
    // The column fingerprint cannot see material thresholds, flora density or the
    // tile builder; this digest pins all three.
    let mut value = 0xcbf2_9ce4_8422_2325u64;
    let mut mix = |part: i64| {
        value ^= part as u64;
        value = value.wrapping_mul(0x1000_0000_01b3);
    };
    for z in -8..8 {
        for x in -8..8 {
            let world_x = x * 11;
            let world_z = z * 11;
            let column = landscape::column(SEED, world_x, world_z);
            for y in (column.height - 6).max(MIN_SURFACE_Y)..=column.height {
                mix(landscape::material_at(SEED, world_x, y, world_z) as i64);
            }
            let flora = landscape::flora_cell(SEED, x, z);
            mix(flora.len() as i64);
            for site in flora.sites() {
                mix(site.kind as i64);
                mix(site.x as i64);
                mix(site.z as i64);
                mix(site.scale_eighths as i64);
            }
            if let Some(tree) = landscape::tree_cell(SEED, x, z) {
                mix(tree.kind as i64);
                mix(tree.x as i64);
                mix(tree.z as i64);
                mix(tree.scale_eighths as i64);
            }
        }
    }
    for (level, key) in [(1, [0, 0]), (2, [1, -1]), (4, [0, 0])] {
        let mesh = landscape::lod_tile_mesh(SEED, level, key, TileFilter::default());
        mix(mesh.vertices.len() as i64);
        mix(mesh.indices.len() as i64);
        for vertex in &mesh.vertices {
            for channel in vertex.position {
                mix((channel * 8.0) as i64);
            }
            mix(vertex.normal[1].to_bits() as i64);
            mix(vertex.color[0].to_bits() as i64);
        }
        for index in &mesh.indices {
            mix(*index as i64);
        }
    }
    assert_eq!(
        value, 0xa5ee_08d9_33ba_1fc4,
        "materials, flora or tile output drifted from the recorded generator identity"
    );
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

/// Heights along one tile edge, keyed by the coordinate that runs *along* that
/// edge (z for an x-normal edge and vice versa), sorted by that coordinate.
fn tile_edge_heights(mesh: &Mesh, axis: usize, value: f32) -> Vec<(f32, f32)> {
    let along = if axis == 0 { 2 } else { 0 };
    let mut heights = Vec::new();
    for vertex in &mesh.vertices {
        if (vertex.position[axis] - value).abs() < 1.0e-3 {
            heights.push((vertex.position[along], vertex.position[1]));
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
fn lod_tiles_share_their_z_edges_exactly() {
    // The x pair alone cannot detect a z-row inversion: this is the seam that
    // caught one.
    let level = 1;
    let cell = landscape::lod_cell_m(level);
    for key_z in [-3, 0, 4] {
        let north = landscape::lod_tile_mesh(SEED, level, [1, key_z], TileFilter::default());
        let south = landscape::lod_tile_mesh(SEED, level, [1, key_z + 1], TileFilter::default());
        assert!(!north.vertices.is_empty() && !south.vertices.is_empty());
        let edge_z = ((key_z + 1) * landscape::LOD_TILE_CELLS) as f32 * cell as f32;
        let north_edge = tile_edge_heights(&north, 2, edge_z);
        let south_edge = tile_edge_heights(&south, 2, edge_z);
        assert_eq!(north_edge.len(), south_edge.len());
        for ((x, height), (other_x, other)) in north_edge.iter().zip(&south_edge) {
            assert!((x - other_x).abs() < 1.0e-3, "edge x mismatch");
            assert!(
                (height - other).abs() < 1.0e-3,
                "tile edge height mismatch at x {x}: {height} vs {other}"
            );
        }
    }
}

#[test]
fn tile_vertices_sample_the_generator_at_their_own_coordinate() {
    // A z-mirrored corner would put a neighbour's height at a grid point; taking
    // the highest vertex at each grid point catches exactly that.
    for level in [1, 2, 4] {
        let cell = landscape::lod_cell_m(level);
        let key = [1, -1];
        let mesh = landscape::lod_tile_mesh(SEED, level, key, TileFilter::default());
        assert!(!mesh.vertices.is_empty());
        let mut seen = std::collections::BTreeMap::new();
        for vertex in &mesh.vertices {
            for (gx, gz) in [(
                vertex.position[0] as i32 / cell,
                vertex.position[2] as i32 / cell,
            )] {
                if vertex.position[0] as i32 % cell != 0 || vertex.position[2] as i32 % cell != 0 {
                    continue;
                }
                let entry = seen.entry((gx, gz)).or_insert((i32::MIN, 0usize));
                entry.0 = entry.0.max(vertex.position[1] as i32);
                entry.1 += 1;
            }
        }
        assert!(!seen.is_empty());
        for ((gx, gz), (top, count)) in seen {
            assert!(count > 0);
            let x = gx * cell;
            let z = gz * cell;
            let column = landscape::column(SEED, x, z);
            let expected = if column.flooded() {
                SEA_LEVEL
            } else {
                column.height
            };
            assert_eq!(
                top, expected,
                "level {level} grid point ({x}, {z}) must carry its own column height"
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

// -- Distance rings ----------------------------------------------------------

/// Eye positions used for the ring proofs: the origin, two non-tile-aligned
/// positions with negative coordinates, and one against the world edge. All are
/// inside the simulation domain, which is where the planner's coverage contract
/// holds and where the sample keeps its camera.
const RING_EYES: [[f32; 3]; 4] = [
    [0.0, 40.0, 0.0],
    [-137.5, 62.25, -201.25],
    [67.75, 18.0, 250.9],
    [-255.0, 30.0, 12.0],
];

/// Claim counter over the outermost ring square, at the granularity every claim
/// boundary is aligned to. The test asserts that alignment separately, so one
/// block stands for its 16 metre cells rather than sampling them.
const CLAIM_BLOCK_M: i32 = 4;

struct ClaimGrid {
    origin: [i32; 2],
    side: usize,
    counts: Vec<u8>,
}

impl ClaimGrid {
    fn new(square: Clip) -> Self {
        let side = ((square.max[0] - square.min[0]) / CLAIM_BLOCK_M) as usize;
        Self {
            origin: square.min,
            side,
            counts: vec![0; side * side],
        }
    }

    /// Count one claim over `rect`. Returns the number of blocks that fell
    /// outside the grid, which must be zero for a nested plan.
    fn claim(&mut self, rect: Clip) -> usize {
        let mut outside = 0;
        let low = [0, 1].map(|axis| (rect.min[axis] - self.origin[axis]).div_euclid(CLAIM_BLOCK_M));
        let high =
            [0, 1].map(|axis| (rect.max[axis] - self.origin[axis]).div_euclid(CLAIM_BLOCK_M));
        for z in low[1]..high[1] {
            for x in low[0]..high[0] {
                if x < 0 || z < 0 || x as usize >= self.side || z as usize >= self.side {
                    outside += 1;
                    continue;
                }
                let slot = &mut self.counts[z as usize * self.side + x as usize];
                *slot = slot.saturating_add(1);
            }
        }
        outside
    }
}

/// The cells one planned tile actually meshes, as world-metre rectangles. This
/// mirrors `lod_tile_mesh`'s own inclusion rule; `ring_tiles_agree_with_their_meshes`
/// checks the two stay in step.
fn included_cells(tile: &RingTile) -> Vec<Clip> {
    let cell_m = landscape::lod_cell_m(tile.level);
    let mut cells = Vec::new();
    for cz in 0..LOD_TILE_CELLS {
        for cx in 0..LOD_TILE_CELLS {
            let x0 = (tile.key[0] * LOD_TILE_CELLS + cx) * cell_m;
            let z0 = (tile.key[1] * LOD_TILE_CELLS + cz) * cell_m;
            let centre = [x0 + cell_m / 2, z0 + cell_m / 2];
            let in_hole = tile
                .filter
                .hole
                .is_some_and(|hole| hole.contains_centre(centre));
            let in_bound = tile
                .filter
                .bound
                .is_none_or(|bound| bound.contains_centre(centre));
            if !in_hole && in_bound {
                cells.push(Clip {
                    min: [x0, z0],
                    max: [x0 + cell_m, z0 + cell_m],
                });
            }
        }
    }
    // A ring finer than the claim block (the 2 m ring) is merged into
    // CLAIM_BLOCK_M squares, so one claim still covers whole blocks. Every claim
    // boundary is block-aligned, so a block is uniformly included or excluded.
    let factor = (CLAIM_BLOCK_M / cell_m).max(1);
    if factor == 1 {
        return cells;
    }
    let mut merged = Vec::new();
    let mut seen = BTreeSet::new();
    for cell in &cells {
        let origin = [
            cell.min[0].div_euclid(CLAIM_BLOCK_M) * CLAIM_BLOCK_M,
            cell.min[1].div_euclid(CLAIM_BLOCK_M) * CLAIM_BLOCK_M,
        ];
        if !seen.insert(origin) {
            continue;
        }
        // All `factor x factor` members must be present; the alignment guarantee
        // above is what makes this an assertion rather than an assumption.
        for dz in 0..factor {
            for dx in 0..factor {
                let member = [origin[0] + dx * cell_m, origin[1] + dz * cell_m];
                assert!(
                    cells.iter().any(|cell| cell.min == member),
                    "block {origin:?} is only partially included"
                );
            }
        }
        merged.push(Clip {
            min: origin,
            max: [origin[0] + CLAIM_BLOCK_M, origin[1] + CLAIM_BLOCK_M],
        });
    }
    merged
}

#[test]
fn rings_cover_the_visible_square_exactly_once() {
    for eye in RING_EYES {
        let fine = landscape::fine_clip(eye);
        let plan = landscape::ring_plan(eye, fine, &LANDSCAPE_RINGS);
        assert!(!plan.is_empty());

        // Premise of the block granularity: every claim boundary is a multiple
        // of CLAIM_BLOCK_M, so a block is claimed as a whole or not at all.
        for edge in [fine.min[0], fine.min[1], fine.max[0], fine.max[1]] {
            assert_eq!(edge.rem_euclid(CLAIM_BLOCK_M), 0, "fine edge {edge}");
        }
        for tile in &plan {
            // The grid runs at CLAIM_BLOCK_M; a coarser cell is a whole number of
            // blocks and a finer one is merged up to a block by `included_cells`.
            let cell_m = landscape::lod_cell_m(tile.level);
            assert!(
                cell_m >= CLAIM_BLOCK_M || CLAIM_BLOCK_M % cell_m == 0,
                "level {} cells of {cell_m} m cannot be represented on a {CLAIM_BLOCK_M} m grid",
                tile.level
            );
            let bound = tile.filter.bound.expect("a ring tile is always bounded");
            let hole = tile.filter.hole.expect("a ring tile always has a hole");
            for edge in [bound.min[0], bound.min[1], bound.max[0], bound.max[1]] {
                assert_eq!(edge.rem_euclid(landscape::lod_cell_m(tile.level)), 0);
            }
            for edge in [hole.min[0], hole.min[1], hole.max[0], hole.max[1]] {
                assert_eq!(edge.rem_euclid(landscape::lod_cell_m(tile.level)), 0);
            }
        }

        let outer = plan
            .last()
            .and_then(|tile| tile.filter.bound)
            .expect("the coarsest ring is planned last");
        let mut grid = ClaimGrid::new(outer);
        // The fine streaming window must be nested inside the innermost ring,
        // otherwise it could claim a cell a ring also claims.
        let inner = plan[0].filter.bound.unwrap();
        assert!(
            inner.min[0] <= fine.min[0]
                && inner.min[1] <= fine.min[1]
                && inner.max[0] >= fine.max[0]
                && inner.max[1] >= fine.max[1],
            "fine window {fine:?} escaped the innermost ring {inner:?} at eye {eye:?}"
        );
        assert_eq!(
            grid.claim(fine),
            0,
            "the fine window left the visible square"
        );
        for tile in &plan {
            for cell in included_cells(tile) {
                assert_eq!(
                    grid.claim(cell),
                    0,
                    "tile {:?} meshed {cell:?} outside the visible square",
                    (tile.level, tile.key)
                );
            }
        }
        let mut unclaimed = 0usize;
        let mut overlapped = 0usize;
        for count in &grid.counts {
            match count {
                0 => unclaimed += 1,
                1 => {}
                _ => overlapped += 1,
            }
        }
        assert_eq!(
            (unclaimed, overlapped),
            (0, 0),
            "eye {eye:?}: {unclaimed} blocks of the visible square are drawn by nobody \
             and {overlapped} are drawn more than once"
        );
    }
}

#[test]
fn ring_plans_are_deterministic_and_bounded() {
    for eye in RING_EYES {
        let plan = landscape::ring_plan(eye, landscape::fine_clip(eye), &LANDSCAPE_RINGS);
        assert_eq!(
            plan,
            landscape::ring_plan(eye, landscape::fine_clip(eye), &LANDSCAPE_RINGS),
            "the same eye must plan the same tiles in the same order"
        );
        // A ring whose half-extent is a whole number of its tiles can plan at most
        // `(2h / tile + 1)^2` tiles; tiles wholly inside the ring's hole are dropped
        // by the sample. The bound is the budget property worth pinning.
        let bound: usize = LANDSCAPE_RINGS
            .iter()
            .map(|config| {
                let side = 2 * config.half_extent / config.tile_size_m() + 1;
                (side * side) as usize
            })
            .sum();
        assert!(
            plan.len() <= bound && plan.len() >= LANDSCAPE_RINGS.len() * 8,
            "plan of {} tiles is outside 8..={bound} per ring set",
            plan.len()
        );
        let unique: BTreeSet<_> = plan.iter().map(|tile| (tile.level, tile.key)).collect();
        assert_eq!(unique.len(), plan.len(), "a tile must be planned once");
        // Rings are emitted finest first, which is also the draw order.
        let levels: Vec<u32> = plan.iter().map(|tile| tile.level).collect();
        assert!(levels.windows(2).all(|pair| pair[0] <= pair[1]));
        for config in LANDSCAPE_RINGS {
            assert!(
                plan.iter().any(|tile| tile.level == config.level),
                "ring {config:?} planned no tiles"
            );
        }
    }
}

#[test]
fn ring_tiles_are_stable_while_the_eye_stays_in_one_tile() {
    let finest_tile = LANDSCAPE_RINGS[0].tile_size_m();
    // A tile the eye can cross inside the world, sampled at metre and sub-metre
    // offsets including the first and last position inside it.
    let base = -finest_tile as f32;
    let reference = landscape::ring_plan(
        [base, 40.0, base],
        landscape::fine_clip([base, 40.0, base]),
        &LANDSCAPE_RINGS,
    );
    let keys: Vec<_> = reference
        .iter()
        .map(|tile| (tile.level, tile.key))
        .collect();
    let last = (finest_tile - 1) as f32;
    for step in [0.0, 0.5, 1.0, 15.9, 16.0, last / 2.0, last] {
        let eye = [base + step, 40.0, base + step];
        let plan = landscape::ring_plan(eye, landscape::fine_clip(eye), &LANDSCAPE_RINGS);
        let moved: Vec<_> = plan.iter().map(|tile| (tile.level, tile.key)).collect();
        assert_eq!(
            moved, keys,
            "the tile set changed at {eye:?}, still inside the same {finest_tile} m tile"
        );
        if step < 16.0 {
            // Inside one chunk the hole does not move either, so nothing at all
            // has to be rebuilt.
            assert_eq!(plan, reference, "the plan changed inside one chunk");
        }
    }
    // Leaving the tile must move the set, otherwise the rings would not follow.
    let outside = [base + finest_tile as f32, 40.0, base];
    let plan = landscape::ring_plan(outside, landscape::fine_clip(outside), &LANDSCAPE_RINGS);
    let moved: Vec<_> = plan.iter().map(|tile| (tile.level, tile.key)).collect();
    assert_ne!(
        moved, keys,
        "the rings must follow the eye across a tile edge"
    );
}

#[test]
fn fine_clip_matches_the_published_streaming_window() {
    for eye in [
        [0.0, 40.0, 0.0],
        [-137.5, 62.25, -201.25],
        [250.0, 30.0, -255.9],
    ] {
        let mut world = World::landscape(SEED);
        assert!(world.stream_around(eye));
        let resident = world.stream_resident_chunks().expect("streaming world");
        let published_x = (
            resident.iter().map(|key| key[0]).min().unwrap(),
            resident.iter().map(|key| key[0]).max().unwrap(),
        );
        let published_z = (
            resident.iter().map(|key| key[2]).min().unwrap(),
            resident.iter().map(|key| key[2]).max().unwrap(),
        );
        let fine = landscape::fine_clip(eye);
        assert_eq!(
            (
                fine.min[0].div_euclid(CHUNK_EDGE),
                fine.max[0].div_euclid(CHUNK_EDGE) - 1
            ),
            published_x,
            "fine clip {fine:?} does not match the published window at {eye:?}"
        );
        assert_eq!(
            (
                fine.min[1].div_euclid(CHUNK_EDGE),
                fine.max[1].div_euclid(CHUNK_EDGE) - 1
            ),
            published_z,
            "fine clip {fine:?} does not match the published window at {eye:?}"
        );
    }
}

#[test]
fn ring_tiles_agree_with_their_meshes() {
    let eye = [67.75, 18.0, -201.25];
    let plan = landscape::ring_plan(eye, landscape::fine_clip(eye), &LANDSCAPE_RINGS);
    // Meshing every planned tile would sample millions of columns; a spread of
    // tiles covering fully-inside, hole-cut and bound-cut cases is enough to
    // show the planner's filter and the mesher's filter agree.
    for tile in plan.iter().step_by(17) {
        let cells = included_cells(tile);
        let mesh = landscape::lod_tile_mesh(SEED, tile.level, tile.key, tile.filter);
        assert_eq!(
            cells.is_empty(),
            mesh.vertices.is_empty(),
            "tile {:?} has {} cells but {} vertices",
            (tile.level, tile.key),
            cells.len(),
            mesh.vertices.len()
        );
        let cell_m = landscape::lod_cell_m(tile.level) as f32;
        for vertex in &mesh.vertices {
            let inside = cells.iter().any(|cell| {
                vertex.position[0] >= cell.min[0] as f32 - 0.5
                    && vertex.position[0] <= cell.max[0] as f32 + 0.5
                    && vertex.position[2] >= cell.min[1] as f32 - 0.5
                    && vertex.position[2] <= cell.max[1] as f32 + 0.5
            });
            assert!(
                inside,
                "vertex {:?} of tile {:?} ({cell_m} m cells) lies outside every included cell",
                vertex.position,
                (tile.level, tile.key)
            );
        }
    }
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
