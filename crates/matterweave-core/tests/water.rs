//! Derived water surface acceptance: coverage, plane, revision coupling, edits.
use matterweave_core::landscape::{self, SEA_LEVEL};
use matterweave_core::material;
use matterweave_core::{Mesh, World, CHUNK_EDGE};

const SEED: u64 = 20260913;

fn quad_area(mesh: &Mesh) -> f32 {
    assert_eq!(mesh.vertices.len() % 4, 0, "quads are four vertices");
    let mut area = 0.0;
    for quad in mesh.vertices.chunks(4) {
        let width = (quad[1].position[0] - quad[0].position[0]).abs();
        let depth = (quad[3].position[2] - quad[0].position[2]).abs();
        area += width * depth;
    }
    area
}

fn flooded_columns(world: &World, key: [i32; 3]) -> usize {
    let mut count = 0;
    for lx in 0..CHUNK_EDGE {
        for lz in 0..CHUNK_EDGE {
            let x = key[0] * CHUNK_EDGE + lx;
            let z = key[2] * CHUNK_EDGE + lz;
            let mut flooded = true;
            let (min_y, max_y) = world.stream_y_range();
            for y in SEA_LEVEL..max_y {
                if material::is_solid(world.get([x, y, z])) {
                    flooded = false;
                    break;
                }
            }
            if flooded {
                let mut wet = false;
                for y in min_y..SEA_LEVEL {
                    if material::is_solid(world.get([x, y, z])) {
                        wet = true;
                        break;
                    }
                }
                flooded = wet;
            }
            if flooded {
                count += 1;
            }
        }
    }
    count
}

/// A streamed landscape window with at least one flooded and one dry chunk column.
fn landscape_window() -> (World, [i32; 3]) {
    let level = matterweave_core::water::sea_level_chunk_y();
    assert_eq!(level, SEA_LEVEL.div_euclid(CHUNK_EDGE));
    for cx in (-240..240).step_by(24) {
        for cz in (-240..240).step_by(24) {
            // Probe for water with the generator, then stream a real window there.
            if landscape::height_at(SEED, cx, cz) > -5 {
                continue;
            }
            let mut world = World::landscape(SEED);
            world.stream_around([cx as f32, 30.0, cz as f32]);
            let centre = [cx.div_euclid(CHUNK_EDGE), level, cz.div_euclid(CHUNK_EDGE)];
            for tile in [[0, 0], [1, 0], [-1, 0], [0, 1], [0, -1], [1, 1]] {
                let probe = [centre[0] + tile[0], level, centre[2] + tile[1]];
                if world.stream_resident_chunks().unwrap().contains(&probe)
                    && flooded_columns(&world, probe) > 0
                {
                    return (world, probe);
                }
            }
        }
    }
    panic!("no flooded window found in the probed area");
}

#[test]
fn water_covers_exactly_the_flooded_columns_at_sea_level() {
    let (world, key) = landscape_window();
    let mesh = world.water_mesh_chunk(key);
    let expected = flooded_columns(&world, key);
    assert!(expected > 0);
    assert!(!mesh.vertices.is_empty());
    assert_eq!(
        quad_area(&mesh),
        expected as f32,
        "one square metre of surface per flooded column"
    );
    for vertex in &mesh.vertices {
        assert_eq!(vertex.position[1], SEA_LEVEL as f32);
        assert_eq!(vertex.normal, [0.0, 1.0, 0.0]);
        // The colour attribute of a water vertex is the bed depth under that
        // corner, normalized; the water fragment entry owns the palette.
        assert!(
            vertex.color[0] > 0.0 && vertex.color[0] <= 1.0,
            "every flooded column has at least one metre of water: {vertex:?}"
        );
        assert_eq!([vertex.color[1], vertex.color[2]], [0.0, 0.0]);
    }
    for index in &mesh.indices {
        assert!((*index as usize) < mesh.vertices.len());
    }
    // The mesh sits inside its own chunk footprint.
    for vertex in &mesh.vertices {
        let x = vertex.position[0];
        let z = vertex.position[2];
        assert!(
            (key[0] * CHUNK_EDGE) as f32 <= x && x <= ((key[0] + 1) * CHUNK_EDGE) as f32,
            "{vertex:?}"
        );
        assert!(
            (key[2] * CHUNK_EDGE) as f32 <= z && z <= ((key[2] + 1) * CHUNK_EDGE) as f32,
            "{vertex:?}"
        );
    }
    // Deterministic bytes.
    let again = world.water_mesh_chunk(key);
    assert_eq!(mesh.vertices.len(), again.vertices.len());
    assert_eq!(mesh.indices, again.indices);
}

#[test]
fn every_vertex_carries_the_bed_depth_under_its_own_corner() {
    use matterweave_core::water::{WATER_MAX_DEPTH_M, WATER_QUAD_MAX_M};
    let (world, key) = landscape_window();
    let mesh = world.water_mesh_chunk(key);
    assert!(!mesh.vertices.is_empty());
    let (min_y, _) = world.stream_y_range();
    let mut deepest = 0.0f32;
    for quad in mesh.vertices.chunks(4) {
        let x0 = quad[0].position[0];
        let x1 = quad[1].position[0];
        let z0 = quad[2].position[2];
        let z1 = quad[0].position[2];
        assert!(
            (x1 - x0).abs() <= WATER_QUAD_MAX_M as f32
                && (z1 - z0).abs() <= WATER_QUAD_MAX_M as f32,
            "a quad may not outrun its four depth samples: {quad:?}"
        );
        // Each corner names the flooded column it touches, which is the last
        // column inside the run on each axis.
        for (vertex, column) in [
            (&quad[0], [x0, z1 - 1.0]),
            (&quad[1], [x1 - 1.0, z1 - 1.0]),
            (&quad[2], [x1 - 1.0, z0]),
            (&quad[3], [x0, z0]),
        ] {
            let (x, z) = (column[0] as i32, column[1] as i32);
            let bed = (min_y..SEA_LEVEL)
                .rev()
                .find(|&y| material::is_solid(world.get([x, y, z])))
                .expect("a flooded column stands on a bed");
            let metres = vertex.color[0] * WATER_MAX_DEPTH_M;
            deepest = deepest.max(metres);
            assert_eq!(
                metres,
                ((SEA_LEVEL - bed) as f32).min(WATER_MAX_DEPTH_M),
                "vertex {vertex:?} over column {x},{z} with bed {bed}"
            );
        }
    }
    assert!(
        deepest > 1.0,
        "a shore window must carry more than one depth, deepest was {deepest} m"
    );
}

#[test]
fn only_the_sea_level_layer_carries_water() {
    let (world, key) = landscape_window();
    assert_eq!(key[1], matterweave_core::water::sea_level_chunk_y());
    for dy in [-2, -1, 1, 2, 5] {
        let other = [key[0], key[1] + dy, key[2]];
        let mesh = world.water_mesh_chunk(other);
        assert!(
            mesh.vertices.is_empty(),
            "chunk {other:?} must carry no water mesh"
        );
    }
    // A chunk key that is not resident is air: no water.
    let absent = [key[0] + 100, key[1], key[2] + 100];
    assert!(world.water_mesh_chunk(absent).vertices.is_empty());
}

#[test]
fn raising_land_above_sea_level_removes_its_water() {
    let (mut world, key) = landscape_window();
    let before = world.water_mesh_chunk(key);
    let before_area = quad_area(&before);
    let revision = before.revision;
    // Raise one flooded column into a tower above sea level.
    let mut target = None;
    for lx in 0..CHUNK_EDGE {
        for lz in 0..CHUNK_EDGE {
            let x = key[0] * CHUNK_EDGE + lx;
            let z = key[2] * CHUNK_EDGE + lz;
            let dry_above = (SEA_LEVEL..world.stream_y_range().1)
                .all(|y| !material::is_solid(world.get([x, y, z])));
            let wet_below = (world.stream_y_range().0..SEA_LEVEL)
                .any(|y| material::is_solid(world.get([x, y, z])));
            if dry_above && wet_below {
                target = Some([x, SEA_LEVEL + 4, z]);
                break;
            }
        }
        if target.is_some() {
            break;
        }
    }
    let target = target.expect("a flooded column to build on");
    assert!(world.set(target, material::STONE));
    let after = world.water_mesh_chunk(key);
    assert_ne!(
        after.revision, revision,
        "the edit must advance the revision"
    );
    assert_eq!(
        quad_area(&after),
        before_area - 1.0,
        "exactly the built-on column loses its surface"
    );
}

#[test]
fn dry_land_and_underground_carry_no_water() {
    // The legacy island fixture has no residency outside its own square; its
    // highest columns are dry, and a non-streaming world is fully authoritative.
    let world = World::generate(SEED);
    for x in [-20, -5, 0, 7, 20] {
        for z in [-20, -3, 0, 9, 20] {
            let top = (world.stream_y_range().0..world.stream_y_range().1)
                .rev()
                .find(|&y| material::is_solid(world.get([x, y, z])));
            let key = [x.div_euclid(CHUNK_EDGE), 0, z.div_euclid(CHUNK_EDGE)];
            let mesh = world.water_mesh_chunk(key);
            let flooded = top.is_some_and(|y| y < SEA_LEVEL);
            if !flooded {
                // The chunk may still hold other flooded columns; only assert that
                // a dry column contributes no surface by area accounting.
                assert_eq!(
                    quad_area(&mesh),
                    flooded_columns(&world, key) as f32,
                    "area must match the flooded count for {key:?}"
                );
            }
        }
    }
}
