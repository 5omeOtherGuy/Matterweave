use matterweave_core::World;
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_TEST: AtomicU64 = AtomicU64::new(0);
struct SavePath(PathBuf);
impl SavePath {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "matterweave-core-{}-{}",
            std::process::id(),
            NEXT_TEST.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&dir).unwrap();
        Self(dir.join("world.json"))
    }
}
impl Drop for SavePath {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(self.0.parent().unwrap());
    }
}

#[test]
fn edits_cross_negative_chunk_boundaries_and_release_empty_chunks() {
    let mut world = World::new(42);
    let positions = [
        [-17, 0, 0],
        [-16, 0, 0],
        [-1, 0, 0],
        [0, 0, 0],
        [15, 0, 0],
        [16, 0, 0],
        [i32::MIN, i32::MAX, 0],
    ];
    for (i, cell) in positions.iter().enumerate() {
        assert_eq!(world.get(*cell), 0);
        assert!(world.set(*cell, (i + 1) as u8));
        assert!(!world.set(*cell, (i + 1) as u8));
    }
    assert_eq!(world.revision(), positions.len() as u64);
    assert_eq!(world.stats().solid_voxels, positions.len());
    for (i, cell) in positions.iter().enumerate() {
        assert_eq!(world.get(*cell), (i + 1) as u8);
    }
    for cell in positions {
        assert!(world.set(cell, 0));
    }
    assert_eq!(world.stats().chunks, 0);
    assert!(!world.set([200, 30, -90], 0));
}

#[test]
fn seeded_generation_is_reproducible_and_seed_changes_terrain() {
    let a = World::generate(42);
    let b = World::generate(42);
    let c = World::generate(43);
    assert_eq!(a.seed(), 42);
    assert_eq!(a.stats(), b.stats());
    assert!(a.stats().solid_voxels > 10_000);
    let mut changed = false;
    for x in -32..32 {
        for z in -32..32 {
            for y in -8..20 {
                assert_eq!(a.get([x, y, z]), b.get([x, y, z]));
                changed |= a.get([x, y, z]) != c.get([x, y, z]);
            }
        }
    }
    assert!(changed);
}

#[test]
fn rays_normalize_direction_and_hit_negative_cells() {
    let mut world = World::new(0);
    world.set([-17, 2, 0], 4);
    let hit = world
        .raycast([-20.5, 2.5, 0.5], [10.0, 0.0, 0.0], 10.0)
        .unwrap();
    assert_eq!(hit.cell, [-17, 2, 0]);
    assert_eq!(hit.normal, [-1, 0, 0]);
    assert_eq!(hit.material, 4);
    assert!((hit.distance - 3.5).abs() < 1e-5);
    assert!(world
        .raycast([-20.5, 2.5, 0.5], [1.0, 0.0, 0.0], 3.49)
        .is_none());
    let hit = world
        .raycast([-15.0, 2.5, 0.5], [-1.0, 0.0, 0.0], 2.0)
        .unwrap();
    assert_eq!(hit.normal, [1, 0, 0]);
    assert_eq!(hit.distance, 1.0);
}

#[test]
fn ray_inside_solid_and_boundary_ties_have_defined_results() {
    let mut world = World::new(0);
    world.set([0, 0, 0], 1);
    let hit = world.raycast([0.5; 3], [1.0, 0.0, 0.0], 0.0).unwrap();
    assert_eq!(hit.distance, 0.0);
    assert_eq!(hit.normal, [0; 3]);
    let mut corners = World::new(0);
    corners.set([1, 0, 0], 1); // A ray touching only its edge must not count.
    corners.set([1, 1, 0], 2);
    let hit = corners
        .raycast([0.5, 0.5, 0.5], [1.0, 1.0, 0.0], 2.0)
        .unwrap();
    assert_eq!(hit.cell, [1, 1, 0]);
}

#[test]
fn invalid_or_unbounded_rays_are_rejected() {
    let world = World::generate(1);
    for direction in [[0.0; 3], [f32::NAN, 0.0, 0.0], [f32::INFINITY, 0.0, 0.0]] {
        assert!(world.raycast([0.0; 3], direction, 100.0).is_none());
    }
    for distance in [-1.0, f32::NAN, f32::INFINITY] {
        assert!(world.raycast([0.0; 3], [1.0, 0.0, 0.0], distance).is_none());
    }
    assert!(world
        .raycast([f32::MAX; 3], [1.0, 0.0, 0.0], 10.0)
        .is_none());
    assert!(world
        .raycast([f32::NAN; 3], [1.0, 0.0, 0.0], 10.0)
        .is_none());
}

#[test]
fn surface_mesh_occludes_chunk_seams_and_winds_outward() {
    let mut world = World::new(0);
    world.set([-1, 0, 0], 1);
    world.set([0, 0, 0], 2);
    let mesh = world.mesh();
    assert_eq!(mesh.revision, world.revision());
    assert_eq!(mesh.vertices.len(), 10 * 4);
    assert_eq!(mesh.indices.len(), 10 * 6);
    for triangle in mesh.indices.chunks_exact(3) {
        let a = mesh.vertices[triangle[0] as usize];
        let b = mesh.vertices[triangle[1] as usize];
        let c = mesh.vertices[triangle[2] as usize];
        let u = std::array::from_fn::<_, 3, _>(|i| b.position[i] - a.position[i]);
        let v = std::array::from_fn::<_, 3, _>(|i| c.position[i] - a.position[i]);
        let cross = [
            u[1] * v[2] - u[2] * v[1],
            u[2] * v[0] - u[0] * v[2],
            u[0] * v[1] - u[1] * v[0],
        ];
        assert!(cross.iter().zip(a.normal).map(|(x, n)| x * n).sum::<f32>() > 0.0);
    }
    world.set([0, 0, 0], 0);
    assert_eq!(world.mesh().indices.len(), 36);
    assert_ne!(mesh.revision, world.mesh().revision);
}

#[test]
fn persistence_preserves_seed_edits_materials_and_revision() {
    let path = SavePath::new();
    let mut world = World::generate(8712);
    world.set([-16, -8, 4], 0);
    world.set([80, 33, -45], 255);
    world.save(&path.0).unwrap();
    let loaded = World::load(&path.0).unwrap();
    assert_eq!(loaded.seed(), world.seed());
    assert_eq!(loaded.revision(), world.revision());
    assert_eq!(loaded.stats(), world.stats());
    assert_eq!(loaded.get([-16, -8, 4]), 0);
    assert_eq!(loaded.get([80, 33, -45]), 255);
    world.set([80, 33, -45], 3);
    world.save(&path.0).unwrap();
    assert_eq!(World::load(&path.0).unwrap().get([80, 33, -45]), 3);
    assert_eq!(fs::read_dir(path.0.parent().unwrap()).unwrap().count(), 1);
}

#[test]
fn malformed_or_incompatible_saves_are_rejected() {
    let path = SavePath::new();
    for input in ["", "{", "[]", r#"{"format_version":900}"#] {
        fs::write(&path.0, input).unwrap();
        assert!(World::load(&path.0).is_err());
    }
    let mut world = World::new(8);
    world.set([0, 0, 0], 1);
    world.save(&path.0).unwrap();
    let original: serde_json::Value = serde_json::from_slice(&fs::read(&path.0).unwrap()).unwrap();
    for field in ["format_version", "generator_version"] {
        let mut data = original.clone();
        data[field] = serde_json::json!(999);
        fs::write(&path.0, serde_json::to_vec(&data).unwrap()).unwrap();
        assert!(World::load(&path.0).is_err());
    }
    let mut short = original.clone();
    short["chunks"][0]["voxels"] = serde_json::json!([1, 2]);
    fs::write(&path.0, serde_json::to_vec(&short).unwrap()).unwrap();
    assert!(World::load(&path.0).is_err());
    let mut duplicate = original.clone();
    duplicate["chunks"]
        .as_array_mut()
        .unwrap()
        .push(original["chunks"][0].clone());
    fs::write(&path.0, serde_json::to_vec(&duplicate).unwrap()).unwrap();
    assert!(World::load(&path.0).is_err());
}

#[test]
fn oversized_save_failure_preserves_previous_save() {
    let path = SavePath::new();
    let mut world = World::new(4);
    world.set([0, 0, 0], 1);
    world.save(&path.0).unwrap();
    let previous = fs::read(&path.0).unwrap();
    for chunk in 0..513 {
        world.set([chunk * 16, 0, 0], 1);
    }
    assert!(world.save(&path.0).is_err());
    assert_eq!(fs::read(&path.0).unwrap(), previous);
    assert_eq!(World::load(&path.0).unwrap().stats().solid_voxels, 1);
}

#[test]
fn failed_rename_cleans_up_temporary_file() {
    let path = SavePath::new();
    fs::create_dir(&path.0).unwrap();
    fs::write(path.0.join("sentinel"), "keep").unwrap();
    assert!(World::new(1).save(&path.0).is_err());
    assert_eq!(fs::read_to_string(path.0.join("sentinel")).unwrap(), "keep");
    assert_eq!(fs::read_dir(path.0.parent().unwrap()).unwrap().count(), 1);
    assert!(World::new(1)
        .save(path.0.join("missing").join("save.json"))
        .is_err());
}

#[test]
fn empty_world_roundtrip_and_extreme_coordinates_remain_valid() {
    let path = SavePath::new();
    let mut world = World::new(99);
    world.save(&path.0).unwrap();
    assert_eq!(World::load(&path.0).unwrap().stats().chunks, 0);
    world.set([i32::MIN, i32::MAX, 0], 3);
    world.save(&path.0).unwrap();
    let loaded = World::load(&path.0).unwrap();
    assert_eq!(loaded.get([i32::MIN, i32::MAX, 0]), 3);
    assert_eq!(loaded.mesh().indices.len(), 36);
}

#[test]
fn voxel_edits_match_independent_sparse_reference() {
    let mut world = World::new(18);
    let mut reference = std::collections::HashMap::new();
    let mut state = 19_u64;
    for _ in 0..12_000 {
        let cell = std::array::from_fn(|_| {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            ((state >> 32) % 65) as i32 - 32
        });
        let material = ((state >> 16) % 4) as u8;
        let old = reference.get(&cell).copied().unwrap_or(0);
        assert_eq!(world.set(cell, material), old != material);
        reference.insert(cell, material);
        assert_eq!(world.get(cell), material);
    }
    for (cell, material) in reference {
        assert_eq!(world.get(cell), material);
    }
}

#[test]
fn imported_near_exhausted_revision_never_wraps_or_panics_on_edits() {
    let path = SavePath::new();
    World::new(1).save(&path.0).unwrap();
    let mut snapshot: serde_json::Value =
        serde_json::from_slice(&fs::read(&path.0).unwrap()).unwrap();
    snapshot["revision"] = serde_json::json!(u64::MAX - 1);
    fs::write(&path.0, serde_json::to_vec(&snapshot).unwrap()).unwrap();
    let mut world = World::load(&path.0).unwrap();
    assert!(world.set([0, 0, 0], 1));
    assert!(!world.set([0, 0, 0], 2));
    assert_eq!(world.get([0, 0, 0]), 1);
    assert_eq!(world.revision(), u64::MAX);
    world.save(&path.0).unwrap();
    let loaded = World::load(&path.0).unwrap();
    assert_eq!(loaded.revision(), u64::MAX);
    assert_eq!(loaded.get([0, 0, 0]), 1);
}
