//! Landscape generator acceptance: determinism, relief, biomes, water, tiles,
//! flora population and the authoritative streaming path that uses them.
use matterweave_core::landmarks;
use matterweave_core::landscape::{
    self, Biome, Clip, RingTile, TileFilter, FLORA_CELL_M, LANDSCAPE_GENERATOR_VERSION,
    LANDSCAPE_RINGS, LOD_TILE_CELLS, MAX_SURFACE_Y, MIN_SURFACE_Y, SEA_LEVEL,
};
use matterweave_core::{material, Mesh, TerrainSource, Vertex, World, CHUNK_EDGE};
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
    // 4: the blend-and-landmark change and the near-field flora density profile
    // landed in the same window, and both move what the generator produces.
    assert_eq!(LANDSCAPE_GENERATOR_VERSION, 4);
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
    //
    // The blend-and-landmark change moved it from 0x3e12_047e_a85c_b89e to
    // 0x5231_cffe_2b4dc609: the biome mixture, its material roll, the blended
    // vegetation pressures and the landmark shaping all reach `landscape::column`,
    // which is what this digest samples. Heights changed only inside landmark
    // footprints; every column kept its hard-classified biome label. The
    // near-field flora density profile that landed beside it does not appear here
    // - it changes what the runtime plans from the population, not the population
    // this samples - which is why this value did not move again.
    assert_eq!(
        fingerprint(SEED, 12),
        0x5231_cffe_2b4d_c609,
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
            for channel in vertex.normal {
                mix(channel.to_bits() as i64);
            }
            mix(vertex.color[0].to_bits() as i64);
        }
        for index in &mesh.indices {
            mix(*index as i64);
        }
    }
    // The columns, materials and flora above are the generator's own output. The
    // tile meshes are derived render output: this digest moves whenever the mesh
    // builder changes, so a value change is expected exactly when the builder
    // changed on purpose, and the old and new values belong in the change that
    // made it. This value moved with the per-voxel surface tone
    // (`material::tone`), which reaches every tile as `TopRect::tone` /
    // `WallRun::tone`: 0x63ec_1387_d1e7_09da before the tone, this after.
    // Vertex positions, normals, counts and indices are unchanged by it, and the
    // per-voxel tone alone moved this value to 0xc8c5_258b_e67a_f422. The
    // vegetation form change moves it again, to the value below: flora prototypes
    // are larger and the tile builder carries their tone, so the two changes
    // together are one digest, measured after the merge rather than copied from
    // either change. It stands at 0x238c_a60d_0189_c137 for v2.
    //
    // The blend-and-landmark change moved it to 0x457d_182a_5cc9_e146: the
    // surface tone now carries the
    // blend's per-patch material (sand thinning into grass, stone into soil),
    // the flora population follows the mixed pressures, and a landmark's own
    // rock face reaches the tiles as STONE/GRAVEL. Vertex counts move with the
    // new material patches, because a merged top needs equal colours.
    assert_eq!(
        value, 0x457d_182a_5cc9_e146,
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

/// One flat-shaded face read back from a tile mesh.
#[derive(Clone, Copy, Debug)]
struct Face {
    normal: [f32; 3],
    colour: [f32; 3],
    /// Footprint in world metres: min x, max x, min z, max z.
    span: [f32; 4],
    /// Low and high y of the face.
    limits: [f32; 2],
}

impl Face {
    fn is_top(&self) -> bool {
        self.normal == [0.0, 1.0, 0.0]
    }
}

/// Read a tile mesh back as faces, checking the flat-shading and winding
/// contract of every quad on the way: four vertices, one normal, one colour, and
/// a winding whose cross product points along that normal.
fn tile_faces(mesh: &Mesh) -> Vec<Face> {
    assert!(!mesh.indices.is_empty());
    assert_eq!(mesh.indices.len() % 6, 0, "quads are two triangles");
    let mut faces = Vec::new();
    for quad in mesh.indices.chunks(6) {
        let base = quad[0];
        assert_eq!(
            quad,
            [base, base + 1, base + 2, base, base + 2, base + 3],
            "quads are emitted low-index first"
        );
        let vertices: Vec<Vertex> = (base..base + 4)
            .map(|index| mesh.vertices[index as usize])
            .collect();
        let normal = vertices[0].normal;
        let colour = vertices[0].color;
        let mut limits = [f32::INFINITY, f32::NEG_INFINITY];
        for vertex in &vertices {
            assert_eq!(vertex.normal, normal, "one normal per face");
            assert_eq!(vertex.color, colour, "one colour per face");
            assert!(vertex.position.iter().all(|value| value.is_finite()));
            limits[0] = limits[0].min(vertex.position[1]);
            limits[1] = limits[1].max(vertex.position[1]);
        }
        let span = [
            vertices
                .iter()
                .map(|v| v.position[0])
                .fold(f32::INFINITY, f32::min),
            vertices
                .iter()
                .map(|v| v.position[0])
                .fold(f32::NEG_INFINITY, f32::max),
            vertices
                .iter()
                .map(|v| v.position[2])
                .fold(f32::INFINITY, f32::min),
            vertices
                .iter()
                .map(|v| v.position[2])
                .fold(f32::NEG_INFINITY, f32::max),
        ];
        let cross = cross_product(&vertices);
        assert_eq!(
            cross, normal,
            "back-face culling reads this winding: {vertices:?}"
        );
        faces.push(Face {
            normal,
            colour,
            span,
            limits,
        });
    }
    faces
}

/// `cross(p1 - p0, p2 - p0)`, normalised. A quad's corners are axis-aligned, so
/// every component of the result is exact.
fn cross_product(vertices: &[Vertex]) -> [f32; 3] {
    let u: [f32; 3] =
        std::array::from_fn(|axis| vertices[1].position[axis] - vertices[0].position[axis]);
    let v: [f32; 3] =
        std::array::from_fn(|axis| vertices[2].position[axis] - vertices[0].position[axis]);
    let cross = [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ];
    let length = (cross[0] * cross[0] + cross[1] * cross[1] + cross[2] * cross[2]).sqrt();
    assert!(
        length > 0.0,
        "a degenerate face has no facing: {vertices:?}"
    );
    cross.map(|value| value / length)
}

/// The height one coarse cell carries: the generator sampled at the cell centre,
/// floored to the level's step.
fn cell_height(seed: u64, level: u32, key: [i32; 2], cx: i32, cz: i32) -> i32 {
    let cell = landscape::lod_cell_m(level);
    let step = landscape::lod_step_m(level);
    let centre = [
        (key[0] * LOD_TILE_CELLS + cx) * cell + cell / 2,
        (key[1] * LOD_TILE_CELLS + cz) * cell + cell / 2,
    ];
    let sample = landscape::lod_vertex(seed, centre[0], centre[1]);
    sample.height.div_euclid(step) * step
}

#[test]
fn lod_cells_are_flat_tops_at_their_quantised_height() {
    for level in [1, 2, 4, 6] {
        let cell = landscape::lod_cell_m(level);
        let key = [1, -1];
        let mesh = landscape::lod_tile_mesh(SEED, level, key, TileFilter::default());
        let faces = tile_faces(&mesh);
        let mut tops: BTreeMap<(i32, i32), f32> = BTreeMap::new();
        for face in faces.iter().filter(|face| face.is_top()) {
            assert_eq!(
                face.limits[0], face.limits[1],
                "a top face is flat: one height for all four corners"
            );
            for (low, high) in [(face.span[0], face.span[1]), (face.span[2], face.span[3])] {
                assert_eq!(low % cell as f32, 0.0, "top edges align to the cell grid");
                assert_eq!(high % cell as f32, 0.0, "top edges align to the cell grid");
                assert!(high > low, "a rectangle has area");
            }
            let first = [
                (face.span[0] / cell as f32) as i32 - key[0] * LOD_TILE_CELLS,
                (face.span[2] / cell as f32) as i32 - key[1] * LOD_TILE_CELLS,
            ];
            let across = [
                ((face.span[1] - face.span[0]) / cell as f32) as i32,
                ((face.span[3] - face.span[2]) / cell as f32) as i32,
            ];
            for cz in first[1]..first[1] + across[1] {
                for cx in first[0]..first[0] + across[0] {
                    let previous = tops.insert((cx, cz), face.limits[0]);
                    assert!(
                        previous.is_none(),
                        "cell ({cx}, {cz}) is covered by two top faces"
                    );
                }
            }
        }
        assert_eq!(
            tops.len(),
            (LOD_TILE_CELLS * LOD_TILE_CELLS) as usize,
            "a default-filter tile draws every cell"
        );
        for ((cx, cz), height) in tops {
            assert_eq!(
                height,
                cell_height(SEED, level, key, cx, cz) as f32,
                "level {level} cell ({cx}, {cz}) must be flat at its own height"
            );
        }
    }
}

#[test]
fn lod_faces_carry_their_own_normal_and_colour() {
    for level in [1, 3, 6] {
        let mesh = landscape::lod_tile_mesh(SEED, level, [2, -3], TileFilter::default());
        let faces = tile_faces(&mesh);
        let (mut tops, mut walls) = (0, 0);
        for face in &faces {
            let axes = face.normal.iter().filter(|value| **value != 0.0).count();
            assert_eq!(axes, 1, "{:?} is not axis aligned", face.normal);
            assert_eq!(
                face.normal.iter().map(|value| value * value).sum::<f32>(),
                1.0,
                "{:?} is not a unit axis normal",
                face.normal
            );
            if face.is_top() {
                tops += 1;
            } else {
                walls += 1;
                assert_eq!(face.normal[1], 0.0, "a wall stands upright");
            }
        }
        assert!(
            tops > 0 && walls > 0,
            "level {level}: {tops} tops, {walls} walls"
        );
    }
}

/// Every face of `meshes` that lies in the plane `plane` normal to world axis
/// `plane_axis` (0 for x, 2 for z) and covers the run `low .. high` along the
/// other horizontal axis.
fn walls_in_plane(
    meshes: &[&Mesh],
    plane_axis: usize,
    plane: f32,
    low: f32,
    high: f32,
) -> Vec<[f32; 2]> {
    let along_start = 2 - plane_axis;
    let mut found = Vec::new();
    for mesh in meshes {
        for face in tile_faces(mesh) {
            if face.normal[plane_axis] == 0.0 || face.normal[1] != 0.0 {
                continue;
            }
            let (flat_low, flat_high) = (face.span[plane_axis], face.span[plane_axis + 1]);
            if (flat_low - plane).abs() > 1.0e-3 || flat_high != plane {
                continue;
            }
            let (run_low, run_high) = (face.span[along_start], face.span[along_start + 1]);
            if run_low <= low + 1.0e-3 && run_high >= high - 1.0e-3 {
                found.push(face.limits);
            }
        }
    }
    found
}

/// Two tiles side by side must agree on the wall between them: one wall per cell
/// row, spanning the whole step, and never two walls over the same ground.
/// `plane_axis` is 0 for tiles sharing an x border and 2 for a z border.
fn assert_border_walls(level: u32, key: [i32; 2], neighbour: [i32; 2], plane_axis: usize) {
    let cell = landscape::lod_cell_m(level);
    let span = LOD_TILE_CELLS * cell;
    let key_axis = plane_axis / 2;
    let along_axis = 1 - key_axis;
    let left = landscape::lod_tile_mesh(SEED, level, key, TileFilter::default());
    let right = landscape::lod_tile_mesh(SEED, level, neighbour, TileFilter::default());
    let plane = ((key[key_axis] + 1) * span) as f32;
    for run in 0..LOD_TILE_CELLS {
        let low = ((key[along_axis] * LOD_TILE_CELLS + run) * cell) as f32;
        let high = low + cell as f32;
        let mut cells = [0i32; 2];
        for (side, tile) in [key, neighbour].into_iter().enumerate() {
            let mut index = [0i32; 2];
            index[key_axis] = if side == 0 { LOD_TILE_CELLS - 1 } else { 0 };
            index[along_axis] = run;
            cells[side] = cell_height(SEED, level, tile, index[0], index[1]);
        }
        let walls = walls_in_plane(&[&left, &right], plane_axis, plane, low, high);
        if cells[0] == cells[1] {
            assert!(
                walls.is_empty(),
                "equal heights need no wall at level {level} key {key:?} run {run}"
            );
            continue;
        }
        let expected = [cells[0].min(cells[1]) as f32, cells[0].max(cells[1]) as f32];
        assert_eq!(
            walls.len(),
            1,
            "level {level} key {key:?} run {run}: {walls:?} walls for one step of {cells:?}"
        );
        assert_eq!(
            walls[0], expected,
            "the wall spans the full height difference at level {level} key {key:?} run {run}"
        );
    }
}

#[test]
fn lod_tiles_meet_across_their_x_borders_with_one_wall() {
    // The x pair cannot detect a z-row inversion on its own: the z pair below
    // covers that, and both are checked at two levels so a coarse step is
    // covered as well as a fine one.
    for level in [1, 4] {
        for key_x in [-3, 0, 4] {
            assert_border_walls(level, [key_x, 1], [key_x + 1, 1], 0);
        }
    }
}

#[test]
fn lod_tiles_meet_across_their_z_borders_with_one_wall() {
    for level in [1, 4] {
        for key_z in [-3, 0, 4] {
            assert_border_walls(level, [1, key_z], [1, key_z + 1], 2);
        }
    }
}

#[test]
fn lod_hole_edges_wall_the_step_into_the_hole() {
    // A clipped tile must not stop in mid-air. Every drawn cell facing a cell
    // the filter cut out carries one wall, and that wall starts at the cell top
    // and reaches at least the old skirt depth below it.
    let level = 2;
    let cell = landscape::lod_cell_m(level);
    let key = [2, -3];
    let span = LOD_TILE_CELLS * cell;
    let hole = Clip {
        min: [key[0] * span, key[1] * span],
        max: [key[0] * span + span / 2, key[1] * span + span / 2],
    };
    let mesh = landscape::lod_tile_mesh(
        SEED,
        level,
        key,
        TileFilter {
            hole: Some(hole),
            ..TileFilter::default()
        },
    );
    // The hole is the low quadrant, so only the two faces facing it can be
    // filtered; the other two meet an equally drawn neighbour tile.
    let mut checked = 0;
    for cz in 0..LOD_TILE_CELLS {
        for cx in 0..LOD_TILE_CELLS {
            let at = |index: [i32; 2]| {
                [
                    (key[0] * LOD_TILE_CELLS + index[0]) * cell + cell / 2,
                    (key[1] * LOD_TILE_CELLS + index[1]) * cell + cell / 2,
                ]
            };
            if hole.contains_centre(at([cx, cz])) {
                continue;
            }
            for (plane_axis, offset) in [(0usize, [-1, 0]), (2usize, [0, -1])] {
                let (nx, nz) = (cx + offset[0], cz + offset[1]);
                if nx < 0 || nz < 0 || !hole.contains_centre(at([nx, nz])) {
                    continue;
                }
                // The face lies in the drawn cell's own edge, and its run is
                // that cell's own extent along the other horizontal axis.
                let key_axis = plane_axis / 2;
                let along_axis = 1 - key_axis;
                let plane_cell = if key_axis == 0 { cx } else { cz };
                let run = if along_axis == 0 { cx } else { cz };
                let plane = ((key[key_axis] * LOD_TILE_CELLS + plane_cell) * cell) as f32;
                let low = ((key[along_axis] * LOD_TILE_CELLS + run) * cell) as f32;
                let top = cell_height(SEED, level, key, cx, cz) as f32;
                let walls = walls_in_plane(&[&mesh], plane_axis, plane, low, low + cell as f32);
                assert_eq!(
                    walls.len(),
                    1,
                    "cell ({cx}, {cz}) faces the hole on axis {plane_axis} with {walls:?} walls"
                );
                assert_eq!(walls[0][1], top, "the wall starts at the cell's own top");
                assert!(
                    walls[0][0] <= top - 2.0 * cell as f32,
                    "the wall {walls:?} must reach the skirt depth below {top}"
                );
                checked += 1;
            }
        }
    }
    assert!(
        checked >= LOD_TILE_CELLS,
        "only {checked} hole-edge faces were checked"
    );
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
        // Surface vertices stay inside the tile footprint: a wall stands on a
        // cell edge and only reaches down.
        let x = vertex.position[0] as i32;
        let z = vertex.position[2] as i32;
        assert!(
            (key[0] * span..=key[0] * span + span).contains(&x)
                && (key[1] * span..=key[1] * span + span).contains(&z),
            "vertex outside its tile footprint: {vertex:?}"
        );
    }
    // The bound removes the far quarter of the tile, so it must contain fewer
    // vertices, while a hole must keep the ring and add wall geometry.
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
    // Every vertex of the ring must lie outside the hole or be wall geometry on
    // the hole's own cell boundary.
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
fn the_landscape_domain_is_wide_and_the_island_domain_is_not() {
    // The rings render 8 km, so the editable domain has to let a player travel
    // that far; the legacy island keeps the sandbox square it was authored in.
    let landscape = World::landscape(SEED);
    assert_eq!(
        landscape.stream_x_limit(),
        matterweave_core::LANDSCAPE_WORLD_LIMIT
    );
    assert!(landscape.contains_stream_cell([8000, 4, -8000]));
    assert!(!landscape.contains_stream_cell([9000, 4, 0]));
    assert!(!landscape.contains_stream_cell([0, 400, 0]));
    let legacy = World::generate(SEED);
    assert_eq!(legacy.stream_x_limit(), matterweave_core::WORLD_LIMIT);
    assert!(legacy.contains_stream_cell([200, 0, 0]));
    assert!(!legacy.contains_stream_cell([300, 0, 0]));
    // A window published far from the origin is still the full 7x7 x 13 band.
    let mut far = World::landscape(SEED);
    assert!(far.stream_around([7000.0, 40.0, -7000.0]));
    assert_eq!(far.stream_resident_chunks().unwrap().len(), 49 * 13);
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
        let plan = landscape::ring_plan(eye, landscape::fine_clip(eye), &LANDSCAPE_RINGS);
        assert!(!plan.is_empty());

        // Premise of the block granularity: every claim boundary is a multiple
        // of CLAIM_BLOCK_M, so a block is claimed as a whole or not at all.
        for edge in [fine.min[0], fine.min[1], fine.max[0], fine.max[1]] {
            assert_eq!(edge.rem_euclid(CLAIM_BLOCK_M), 0, "fine edge {edge}");
        }
        // The innermost ring takes no hole: it covers the streaming window too,
        // drawing its flooded cells as the bed under the fine water surface, so
        // the ground under the camera is drawn before the fine meshes land.
        // Every other ring is cut with the square inside it.
        assert_eq!(
            plan[0].filter.hole, None,
            "the innermost ring must not take a hole at eye {eye:?}"
        );
        assert_eq!(
            plan[0].filter.bed,
            Some(fine),
            "the innermost ring must treat the streaming window as its bed at eye {eye:?}"
        );
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
            for edge in [bound.min[0], bound.min[1], bound.max[0], bound.max[1]] {
                assert_eq!(edge.rem_euclid(landscape::lod_cell_m(tile.level)), 0);
            }
            if let Some(hole) = tile.filter.hole {
                for edge in [hole.min[0], hole.min[1], hole.max[0], hole.max[1]] {
                    assert_eq!(edge.rem_euclid(landscape::lod_cell_m(tile.level)), 0);
                }
            }
        }

        let outer = plan
            .last()
            .and_then(|tile| tile.filter.bound)
            .expect("the coarsest ring is planned last");
        let mut grid = ClaimGrid::new(outer);
        // The streaming window must be nested inside the innermost ring; the
        // ring draws that ground itself now, so a window that escaped it would
        // leave the part outside covered by neither.
        let inner = plan[0].filter.bound.unwrap();
        assert!(
            inner.min[0] <= fine.min[0]
                && inner.min[1] <= fine.min[1]
                && inner.max[0] >= fine.max[0]
                && inner.max[1] >= fine.max[1],
            "fine window {fine:?} escaped the innermost ring {inner:?} at eye {eye:?}"
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
fn the_ring_plan_covers_the_ground_under_a_moving_eye() {
    // The innermost ring is not cut with the streaming window, so the union of
    // planned tiles draws the ground cell under the camera at every eye
    // position: at walking and flight speed, across chunk, tile and ring
    // boundaries. The fine meshes are drawn over that ground, not instead of
    // it, which is what makes the coverage hold before they arrive.
    for (name, per_frame) in [("walking", 5.0f32 / 60.0), ("flight", 90.0 / 60.0)] {
        for step in 0..900 {
            let t = step as f32;
            let eye = [
                -400.0 + t * per_frame + 40.0 * (t * 0.031).sin(),
                40.0 + (t * 0.017).cos() * 20.0,
                300.0 - t * per_frame * 0.5 + 40.0 * (t * 0.043).cos(),
            ];
            let fine = landscape::fine_clip(eye);
            let plan = landscape::ring_plan(eye, fine, &LANDSCAPE_RINGS);
            let x = eye[0].floor() as i32;
            let z = eye[2].floor() as i32;
            assert!(
                plan.iter().any(|tile| tile.covers_point(x, z)),
                "{name} step {step} at {eye:?}: no planned tile draws the ground under the eye"
            );
            // The window the fine meshes draw in is inside the innermost ring,
            // and that ring treats it as its bed.
            let inner = plan[0].filter.bound.expect("the innermost ring is bounded");
            assert!(inner.min[0] <= fine.min[0] && inner.max[0] >= fine.max[0]);
            assert!(inner.min[1] <= fine.min[1] && inner.max[1] >= fine.max[1]);
            assert_eq!(plan[0].filter.bed, Some(fine), "{name} step {step}");
        }
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
            let tile_size = config.tile_size_m();
            let first = plan
                .iter()
                .find(|tile| tile.level == config.level)
                .unwrap_or_else(|| panic!("ring {config:?} planned no tiles"));
            // The nearest tile is planned first, so a frame's bounded uploads
            // fill the ring around the eye before the far side of the square -
            // which is what lets the sample cover the camera's own ground
            // before anything else in the ring.
            assert_eq!(
                first.key,
                [
                    (eye[0].floor() as i32).div_euclid(tile_size),
                    (eye[2].floor() as i32).div_euclid(tile_size),
                ],
                "ring {config:?} did not plan the eye's own tile first at {eye:?}"
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
            // Inside one chunk the streaming window does not move either, so
            // nothing at all has to be rebuilt.
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
fn a_flat_sea_tile_costs_one_quad() {
    // Two open-water tiles at different levels: 1024 cells of one height and one
    // colour must merge into a single top quad however many cells they hold.
    for (level, key) in [(3u32, [-1, -2]), (4, [4, 3])] {
        let cell = landscape::lod_cell_m(level);
        let mesh = landscape::lod_tile_mesh(SEED, level, key, TileFilter::default());
        let faces = tile_faces(&mesh);
        assert_eq!(faces.len(), 1, "level {level} key {key:?}: {faces:?}");
        assert!(faces[0].is_top());
        assert_eq!(
            mesh.indices.len(),
            6,
            "one quad is two triangles, not {} cells' worth",
            LOD_TILE_CELLS * LOD_TILE_CELLS
        );
        assert_eq!(mesh.vertices.len(), 4);
        assert_eq!(faces[0].limits, [SEA_LEVEL as f32, SEA_LEVEL as f32]);
        assert_eq!(faces[0].colour, material::color(material::WATER));
        assert_eq!(
            faces[0].span,
            [
                (key[0] * LOD_TILE_CELLS * cell) as f32,
                ((key[0] + 1) * LOD_TILE_CELLS * cell) as f32,
                (key[1] * LOD_TILE_CELLS * cell) as f32,
                ((key[1] + 1) * LOD_TILE_CELLS * cell) as f32,
            ]
        );
    }
}

#[test]
fn a_bed_square_draws_the_ground_under_a_fine_water_surface() {
    // The tile above is open water: every cell draws the sea surface as one
    // merged quad. A bed square makes its flooded cells draw the ground under
    // them instead. That is what the innermost ring does under the
    // authoritative streaming window: the fine bed stands a metre above the
    // ring's quantised surface and the fine translucent water is drawn over it,
    // so an opaque second sea surface there would hide the bed the fine water
    // is meant to show through.
    let (level, key) = (4u32, [4, 3]);
    let cell = landscape::lod_cell_m(level);
    let span = LOD_TILE_CELLS * cell;
    let square = Clip {
        min: [key[0] * span, key[1] * span],
        max: [key[0] * span + span, key[1] * span + span],
    };
    let plain = landscape::lod_tile_mesh(SEED, level, key, TileFilter::default());
    let faces = tile_faces(&plain);
    assert_eq!(faces.len(), 1, "the open-water tile is one quad");
    assert_eq!(faces[0].limits, [SEA_LEVEL as f32, SEA_LEVEL as f32]);

    let bed = landscape::lod_tile_mesh(
        SEED,
        level,
        key,
        TileFilter {
            bed: Some(square),
            ..TileFilter::default()
        },
    );
    let faces = tile_faces(&bed);
    assert!(!faces.is_empty(), "a bed tile still draws its ground");
    for face in &faces {
        assert!(
            face.limits[1] < SEA_LEVEL as f32,
            "every bed face is below the water plane: {face:?}"
        );
        assert_ne!(
            face.colour,
            material::color(material::WATER),
            "a bed cell must not draw the water material: {face:?}"
        );
    }
    // Each bed top is its own cell's quantised ground height, not one shared
    // plane: the bed follows the floor it stands for.
    let step = landscape::lod_step_m(level);
    let quantised = |height: i32| (height.div_euclid(step) * step) as f32;
    for face in faces.iter().filter(|face| face.is_top()) {
        // The mesher samples each cell at its centre, and a top's span starts
        // on a cell boundary, so the first cell the face covers is its centre
        // plus half a cell.
        let x = face.span[0] as i32 + cell / 2;
        let z = face.span[2] as i32 + cell / 2;
        let ground = landscape::column(SEED, x, z).height;
        assert_eq!(
            face.limits,
            [quantised(ground), quantised(ground)],
            "bed top at ({x}, {z}) must stand at its own ground height"
        );
    }

    // A bed square covering half the tile changes only that half: the rest is
    // still the sea surface, so the ring can draw both from one mesh.
    let half = Clip {
        min: square.min,
        max: [square.min[0] + span / 2, square.min[1] + span],
    };
    let partial = landscape::lod_tile_mesh(
        SEED,
        level,
        key,
        TileFilter {
            bed: Some(half),
            ..TileFilter::default()
        },
    );
    let faces = tile_faces(&partial);
    assert!(
        faces
            .iter()
            .any(|face| face.limits == [SEA_LEVEL as f32, SEA_LEVEL as f32]),
        "the uncovered half still draws the sea surface"
    );
    assert!(
        faces.iter().any(|face| face.limits[1] < SEA_LEVEL as f32),
        "the bed half draws the ground"
    );
}

/// Geometry the pre-voxel mesh builder produced for the same plan and eye, at
/// 36 bytes a vertex and 4 bytes an index: 57240 KiB and 697792 triangles, which
/// is what the sample's own counters reported on the device and on the host. A
/// vertex grid plus boundary skirts; kept here as the budget this mesh is
/// measured against.
const PRE_VOXEL_PLAN_BYTES: usize = 57_240 * 1024;
const PRE_VOXEL_PLAN_TRIANGLES: usize = 697_792;

#[test]
fn ring_plan_geometry_stays_within_its_budget() {
    // The sample's spawn camera at its own seed (see `spawn_camera` in the
    // explorer): the position the device numbers were taken at.
    let eye = [-192.0, 11.0, 1472.0];
    let plan = landscape::ring_plan(eye, landscape::fine_clip(eye), &LANDSCAPE_RINGS);
    assert_eq!(plan.len(), 384, "the shipped ring plan is the one measured");
    let mut bytes = 0usize;
    let mut triangles = 0usize;
    for tile in &plan {
        let mesh = landscape::lod_tile_mesh(SEED, tile.level, tile.key, tile.filter);
        bytes += mesh.vertices.len() * std::mem::size_of::<matterweave_core::Vertex>()
            + mesh.indices.len() * std::mem::size_of::<u32>();
        triangles += mesh.indices.len() / 3;
    }
    assert!(
        bytes * 10 <= PRE_VOXEL_PLAN_BYTES * 13,
        "{bytes} bytes of tile geometry exceeds 1.3x the {PRE_VOXEL_PLAN_BYTES} byte budget"
    );
    assert!(
        triangles <= PRE_VOXEL_PLAN_TRIANGLES,
        "{triangles} triangles is more than the {PRE_VOXEL_PLAN_TRIANGLES} the old mesh used"
    );
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
fn a_landscape_save_from_another_generator_version_is_rejected() {
    // The generator identity is recorded with the save and validated on load, so
    // a world generated by an older landscape generator is refused instead of
    // loading terrain that no longer exists. A band or landmark change that moves
    // the fingerprint must move the recorded value too, which is what this test
    // would catch if it did not.
    let directory = std::env::temp_dir().join(format!(
        "matterweave-landscape-version-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("landscape.json");
    World::landscape(SEED).save(&path).unwrap();
    let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(
        saved["landscape_generator_version"],
        serde_json::json!(LANDSCAPE_GENERATOR_VERSION),
        "the save must record the generator that built it"
    );
    let mut older = saved.clone();
    older["landscape_generator_version"] = serde_json::json!(LANDSCAPE_GENERATOR_VERSION - 1);
    std::fs::write(&path, serde_json::to_vec(&older).unwrap()).unwrap();
    assert!(
        World::load(&path).is_err(),
        "a save from another landscape generator must be rejected"
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

// -- Biome transitions -------------------------------------------------------
//
// The generator used to classify every column hard, so two neighbouring metres
// either side of a threshold carried different ground materials and different
// vegetation pressures: a fence line. These tests are the acceptance for the
// blend that replaced it, and they measure the band rather than assuming it.

/// The demo region, where the transitions the sample walks through live.
const DEMO_SPAWN: [i32; 2] = [5632, 1408];

/// Sample a straight line of metre-columns.
fn line(from: [i32; 2], direction: (i32, i32), metres: i32) -> Vec<landscape::Column> {
    (0..metres)
        .map(|step| {
            landscape::column(
                SEED,
                from[0] + direction.0 * step,
                from[1] + direction.1 * step,
            )
        })
        .collect()
}

/// Whether a column is land. The sea is a hard edge by nature - the water plane
/// is the generator's one sea level - so a transition across the waterline is
/// not a band and these tests leave it out.
fn land(column: &landscape::Column) -> bool {
    !column.flooded() && column.biome != Biome::Ocean
}

#[test]
fn a_column_still_is_the_biome_the_hard_classifier_names() {
    // The blend changes transitions, not worlds: every column's label is the
    // classification the generator shipped before the bands existed, so biome
    // coverage, flora kinds and every claim about "where the plains are" still
    // describe the same world. Outside a band the mixture agrees with the label
    // too; inside it the mixture is what the material and cover are made of.
    let mut single = 0;
    let mut banded = 0;
    let mut disagreed = 0;
    for z in -300..300 {
        for x in -300..300 {
            let (wx, wz) = (x * 23, z * 23);
            let column = landscape::column(SEED, wx, wz);
            let inputs = landscape::blend_inputs_at(SEED, wx, wz);
            assert_eq!(
                column.biome,
                matterweave_core::biome_blend::hard_classify(inputs),
                "({wx}, {wz}) is labelled differently than the hard classifier names it"
            );
            if column.mix.len() == 1 {
                assert_eq!(
                    column.mix.dominant(),
                    column.biome,
                    "({wx}, {wz}) has left every band but still disagrees with its label"
                );
                single += 1;
            } else {
                banded += 1;
                if column.mix.dominant() != column.biome {
                    disagreed += 1;
                }
            }
        }
    }
    let share = 100.0 * banded as f64 / (banded + single) as f64;
    println!(
        "{single} columns outside a band, {banded} inside one ({share:.1}%), \
         {disagreed} of them dominated by the other biome"
    );
    assert!(single > 0 && banded > 0, "the sample must cover both cases");
    assert!(
        share > 20.0 && share < 60.0,
        "the bands must cover a real share of the ground without dissolving the \
         biomes into each other: {share:.1}%"
    );
}

/// `values[p]`, nearest rank. The samples here are small and deliberately so.
fn percentile(values: &[i32], p: usize) -> i32 {
    if values.is_empty() {
        0
    } else {
        values[(values.len() - 1) * p / 100]
    }
}

/// One boundary the generator puts in front of a walker, measured along a
/// straight line of metre-columns.
#[derive(Debug)]
struct Crossing {
    /// Where the mixture first holds more than one biome.
    at: [i32; 2],
    /// Metres of real mixture either side of it: both biomes hold at least a
    /// fifth of the ground, so a walker sees two kinds of ground, not a tail.
    band_m: i32,
    /// Biome the walk starts in and the one it ends in.
    from: Biome,
    to: Biome,
    /// Largest weight one metre of that band moves.
    worst_step: i32,
    /// Largest elevation one metre of that band crosses, in metres.
    worst_rise: i32,
    /// Metres of the band whose ground steps further than a walking player can.
    steep_m: i32,
}

/// Every material-changing boundary a straight walk from `from` crosses.
///
/// A boundary only counts when the two biomes lay down *different ground*: a
/// hills/plains edge is the same moss on both sides, and nothing about it can
/// read as a seam. The sea is left out too - the waterline is the generator's
/// one sea plane and an edge by nature.
fn crossings(from: [i32; 2], directions: &[(i32, i32)], metres: i32) -> Vec<Crossing> {
    let mut found = Vec::new();
    for direction in directions {
        let line = line(from, *direction, metres);
        let mut start = None;
        for step in 0..=line.len() {
            let inside = step < line.len() && land(&line[step]) && line[step].mix.mixed(51);
            match (start, inside) {
                (None, true) => start = Some(step),
                (Some(begin), false) => {
                    // The band's own first column: the column before it can be
                    // water, and a walker stepping out of the sea is not walking
                    // through a transition band.
                    let index = begin;
                    let subject = line[index].biome;
                    let after = line[step.min(line.len() - 1)].biome;
                    let at = [
                        from[0] + direction.0 * index as i32,
                        from[1] + direction.1 * index as i32,
                    ];
                    start = None;
                    if landscape::biome_surface(SEED, at[0], at[1], subject)
                        == landscape::biome_surface(SEED, at[0], at[1], after)
                    {
                        continue;
                    }
                    let window = &line[index..step];
                    let (mut worst_step, mut steep_m, mut worst_rise) = (0, 0, 0);
                    for pair in window.windows(2) {
                        let before = i32::from(pair[0].mix.weight(subject));
                        let after = i32::from(pair[1].mix.weight(subject));
                        worst_step = worst_step.max((after - before).abs());
                        worst_rise = worst_rise.max((pair[1].height - pair[0].height).abs());
                        // A diagonal line step is 1.414 m of ground.
                        let run = if direction.0 != 0 && direction.1 != 0 {
                            1414
                        } else {
                            1000
                        };
                        let rise = (pair[1].height - pair[0].height).abs();
                        if rise * 1000 * 1000 > run * landmarks::WALK_SLOPE_PERMILLE {
                            steep_m += 1;
                        }
                    }
                    found.push(Crossing {
                        at,
                        band_m: (step - begin) as i32,
                        from: subject,
                        to: after,
                        worst_step,
                        worst_rise,
                        steep_m,
                    });
                }
                _ => {}
            }
        }
    }
    found
}

/// The lines the demo region is walked along: eight bearings from the spawn and
/// from a ring of origins around it, so the sample is the world and not one
/// convenient direction.
fn demo_crossings() -> Vec<Crossing> {
    let directions = [
        (1, 0),
        (0, 1),
        (1, 1),
        (1, -1),
        (2, 1),
        (3, 1),
        (-1, 2),
        (2, -3),
        (-3, 1),
        (1, 3),
    ];
    let mut found = crossings(DEMO_SPAWN, &directions, 600);
    for origin_z in -2..3 {
        for origin_x in -2..3 {
            let origin = [
                DEMO_SPAWN[0] + origin_x * 2048,
                DEMO_SPAWN[1] + origin_z * 2048,
            ];
            found.extend(crossings(origin, &directions, 600));
        }
    }
    found
}

#[test]
fn a_biome_boundary_is_a_band_of_metres_not_a_fence_line() {
    let found = demo_crossings();
    let mut widths: Vec<i32> = found.iter().map(|crossing| crossing.band_m).collect();
    widths.sort_unstable();
    let mut pairs: BTreeMap<(&'static str, &'static str), (usize, i32)> = BTreeMap::new();
    for crossing in &found {
        let record = pairs
            .entry((crossing.from.name(), crossing.to.name()))
            .or_insert((0, 0));
        record.0 += 1;
        record.1 = record.1.max(crossing.band_m);
    }
    println!(
        "{} material-changing crossings: band p10 {} m, p25 {} m, p50 {} m, p75 {} m, p90 {} m, \
         widest {} m",
        widths.len(),
        percentile(&widths, 10),
        percentile(&widths, 25),
        percentile(&widths, 50),
        percentile(&widths, 75),
        percentile(&widths, 90),
        widths.last().copied().unwrap_or(0),
    );
    for ((from, to), (count, widest)) in &pairs {
        println!("  {from}->{to}: {count} crossings, widest {widest} m");
    }
    let steep = found.iter().filter(|crossing| crossing.steep_m > 0).count();
    println!("{steep} of them cross at least one step past the walk slope");
    assert!(
        widths.len() >= 20,
        "the sample must cross many boundaries: {}",
        widths.len()
    );
    assert!(
        percentile(&widths, 50) >= 16,
        "the median transition must be tens of metres of walking: {widths:?}"
    );
    assert!(
        percentile(&widths, 10) >= 4,
        "even the narrowest transition must span metres: {widths:?}"
    );
}

#[test]
fn no_single_metre_carries_more_than_a_quarter_of_a_transition() {
    // The acceptance: along a line crossing a boundary, no step may be larger
    // than the blend's own step. The blend's step is the *scalar* band it is
    // built from - a biome's share of the ground moves with the elevation the
    // walk crosses, at most the whole weight over twelve metres of it - plus the
    // climate fields' own budget for the same metre. A hard threshold moves the
    // whole weight in one metre whatever the ground does; this is the bound that
    // says the blend cannot.
    const CLIMATE_BUDGET: i32 = 32;
    let found = demo_crossings();
    let mut worst = 0;
    let mut worst_at = [0i32; 2];
    let mut flat_worst = 0;
    let mut over = 0;
    let mut worst_ratio = 0.0f64;
    let mut worst_ratio_line = String::new();
    for crossing in &found {
        let allowance = 256 * crossing.worst_rise / matterweave_core::biome_blend::SHORE_BAND_M;
        let allowed = CLIMATE_BUDGET + allowance;
        let ratio = crossing.worst_step as f64 / allowed.max(1) as f64;
        if ratio > worst_ratio {
            worst_ratio = ratio;
            worst_ratio_line = format!(
                "{:?} {}->{} step {} rise {}",
                crossing.at,
                crossing.from.name(),
                crossing.to.name(),
                crossing.worst_step,
                crossing.worst_rise
            );
        }
        if crossing.worst_step > allowed {
            over += 1;
        }
        if crossing.worst_step > worst {
            worst = crossing.worst_step;
            worst_at = crossing.at;
        }
        if crossing.steep_m == 0 {
            flat_worst = flat_worst.max(crossing.worst_step);
        }
    }
    println!(
        "{} crossings: worst one-metre weight step {worst}/256 at ({}, {}), \
         worst on entirely walkable ground {flat_worst}/256, {over} over the elevation budget, \
         worst share of the budget {:.2} ({worst_ratio_line})",
        found.len(),
        worst_at[0],
        worst_at[1],
        worst_ratio,
    );
    assert!(found.len() >= 20, "the sample must cross boundaries");
    assert!(
        over == 0,
        "a metre moved more weight than the elevation it crossed: {over} crossings"
    );
    assert!(
        flat_worst <= 64,
        "a walkable metre carried more than a quarter of a transition: {flat_worst}/256"
    );
}

#[test]
fn flora_pressures_slide_across_a_boundary_instead_of_stepping() {
    // Vegetation is the other half of the transition: a forest edge that thins
    // is a density slide, and the mean pressure per metre must not jump. The
    // bound follows from the weight bound: a metre of *walkable* ground moves a
    // biome's share by at most a quarter, so its pressure - at most 64 - moves
    // by at most a quarter of 64.
    let mut walkable_worst = 0;
    let mut walkable_at = [0i32; 2];
    let mut steep_worst = 0;
    for direction in [
        (1, 0),
        (0, 1),
        (1, 1),
        (2, 1),
        (-1, 2),
        (3, 1),
        (1, 3),
        (-2, 1),
    ] {
        let lines = line(DEMO_SPAWN, direction, 600);
        for step in 1..lines.len() {
            if !land(&lines[step]) || !land(&lines[step - 1]) {
                continue;
            }
            let before = lines[step - 1].mix.pressure(Biome::grass_density) as i32;
            let after = lines[step].mix.pressure(Biome::grass_density) as i32;
            let moved = (after - before).abs();
            let at = [
                DEMO_SPAWN[0] + direction.0 * step as i32,
                DEMO_SPAWN[1] + direction.1 * step as i32,
            ];
            let run = if direction.0 != 0 && direction.1 != 0 {
                1414
            } else if direction.1.abs() > 1 {
                // The longer steps of the shallow bearings: (1, 3) is 3.2 m.
                3162
            } else {
                1000
            };
            let rise = (lines[step].height - lines[step - 1].height).abs();
            if rise * 1000 * 1000 <= run * landmarks::WALK_SLOPE_PERMILLE {
                if moved > walkable_worst {
                    walkable_worst = moved;
                    walkable_at = at;
                }
            } else {
                steep_worst = steep_worst.max(moved);
            }
        }
    }
    println!(
        "worst grass pressure step: {walkable_worst}/64 on walkable ground at ({}, {}), \
         {steep_worst}/64 where the ground itself steps",
        walkable_at[0], walkable_at[1]
    );
    assert!(
        walkable_worst <= 16,
        "ground cover steps a quarter of its range in one walkable metre: {walkable_worst}/64"
    );
}

#[test]
fn the_material_roll_follows_the_mixture() {
    // The surface material is chosen by one deterministic roll per 8 m patch, so
    // a band is the two biomes' own palettes thinning into each other rather
    // than a line where one replaces the other. Pooled over every banded window
    // in the demo region, the share of each palette has to be the weight that
    // asked for it: an unbiased roll, not a lucky one.
    let mut expected: BTreeMap<u8, f64> = BTreeMap::new();
    let mut observed: BTreeMap<u8, i64> = BTreeMap::new();
    let mut windows = 0;
    for direction in [(1, 0), (0, 1), (1, 1), (2, -1), (-1, 3), (3, 2)] {
        let metres = 400;
        let lines = line(DEMO_SPAWN, direction, metres);
        for (start, window) in lines.windows(64).enumerate() {
            if !window.iter().all(land) || window.iter().all(|column| column.mix.len() == 1) {
                continue;
            }
            for (step, column) in window.iter().enumerate() {
                let along = (start + step) as i32;
                let (x, z) = (
                    DEMO_SPAWN[0] + direction.0 * along,
                    DEMO_SPAWN[1] + direction.1 * along,
                );
                for &(biome, weight) in column.mix.entries() {
                    let material = landscape::biome_surface(SEED, x, z, biome);
                    *expected.entry(material).or_default() += f64::from(weight) / 256.0;
                }
                *observed.entry(column.surface).or_default() += 1;
            }
            windows += 1;
        }
    }
    let total: f64 = expected.values().sum();
    println!(
        "{windows} banded windows, {total:.0} columns (the roll is per 8 m patch, so this \
         sample is correlated and the tolerance below is loose on purpose)"
    );
    let mut worst = 0.0f64;
    for (material, want) in &expected {
        let got = observed.get(material).copied().unwrap_or(0) as f64;
        let share = (got - want) / total;
        if share.abs() > worst.abs() {
            worst = share;
        }
        println!(
            "  {:<8} expected {:>7.1}, counted {:>7} ({:+.2}%)",
            matterweave_core::material::name(*material),
            want,
            got as i64,
            100.0 * share
        );
    }
    assert!(windows >= 8, "the sample must contain banded windows");
    assert!(
        worst.abs() < 0.05,
        "a material's share is off its weight by {:.2}%",
        100.0 * worst
    );
}
