use matterweave_core::{Mesh, World, WORLD_LIMIT};
use std::collections::BTreeSet;

type Surface = ([i32; 3], [i32; 3], [u32; 3]);
fn surfaces(mesh: &Mesh) -> BTreeSet<Surface> {
    let mut result = BTreeSet::new();
    for quad in mesh.vertices.chunks_exact(4) {
        let normal = quad[0].normal.map(|v| v as i32);
        let axis = normal.iter().position(|v| *v != 0).unwrap();
        let u = (axis + 1) % 3;
        let v = (axis + 2) % 3;
        let min: [i32; 3] =
            std::array::from_fn(|i| quad.iter().map(|p| p.position[i] as i32).min().unwrap());
        let max: [i32; 3] =
            std::array::from_fn(|i| quad.iter().map(|p| p.position[i] as i32).max().unwrap());
        for a in min[u]..max[u] {
            for b in min[v]..max[v] {
                let mut cell = min;
                cell[u] = a;
                cell[v] = b;
                assert!(
                    result.insert((cell, normal, quad[0].color.map(f32::to_bits))),
                    "duplicate face"
                );
            }
        }
    }
    for triangle in mesh.indices.chunks_exact(3) {
        let [a, b, c] = triangle
            .try_into()
            .unwrap_or([0; 3])
            .map(|i| mesh.vertices[i as usize]);
        let u: [f32; 3] = std::array::from_fn(|i| b.position[i] - a.position[i]);
        let v: [f32; 3] = std::array::from_fn(|i| c.position[i] - a.position[i]);
        let cross = [
            u[1] * v[2] - u[2] * v[1],
            u[2] * v[0] - u[0] * v[2],
            u[0] * v[1] - u[1] * v[0],
        ];
        assert!(cross.iter().zip(a.normal).map(|(x, n)| x * n).sum::<f32>() > 0.0);
    }
    result
}

#[test]
fn greedy_surfaces_exactly_match_reference_materials_boundaries_and_winding() {
    let mut world = World::generate(8712);
    world.set([-16, 0, -16], 255);
    world.set([15, 0, 15], 0);
    world.set([16, 0, 15], 7);
    let reference = world.mesh();
    let expected = surfaces(&reference);
    let mut actual = BTreeSet::new();
    let mut count = 0;
    for key in world.chunk_keys() {
        let mesh = world.mesh_chunk(key);
        assert_eq!(Some(mesh.revision), world.chunk_revision(key));
        count += mesh.indices.len();
        for face in surfaces(&mesh) {
            assert!(actual.insert(face));
        }
    }
    assert_eq!(actual, expected);
    assert!(count < reference.indices.len() / 2);
    println!(
        "seed 8712 with 3 edits: reference {} triangles, greedy {} triangles",
        reference.indices.len() / 3,
        count / 3
    );
}

#[test]
fn dirty_revisions_only_invalidate_local_and_shared_face_dependencies() {
    let mut world = World::new(0);
    for cell in [[0, 0, 0], [16, 0, 0], [48, 0, 0], [-16, 0, 0]] {
        world.set(cell, 1);
    }
    let east = world.chunk_revision([1, 0, 0]);
    let far = world.chunk_revision([3, 0, 0]);
    let west = world.chunk_revision([-1, 0, 0]);
    world.set([7, 7, 7], 1);
    assert_eq!(world.chunk_revision([1, 0, 0]), east);
    assert_eq!(world.chunk_revision([-1, 0, 0]), west);
    world.set([15, 0, 0], 1);
    assert_ne!(world.chunk_revision([1, 0, 0]), east);
    assert_eq!(world.chunk_revision([3, 0, 0]), far);
    let east = world.chunk_revision([1, 0, 0]);
    world.set([15, 0, 0], 0);
    assert_ne!(world.chunk_revision([1, 0, 0]), east);
    world.set([48, 0, 0], 0);
    assert_eq!(world.chunk_revision([3, 0, 0]), None);
    world.set([48, 0, 0], 1);
    assert_ne!(world.chunk_revision([3, 0, 0]), far);
}

#[test]
fn streamed_edits_and_air_survive_eviction_and_atomic_reload() {
    let dir = std::env::temp_dir().join(format!("matterweave-stream-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("world.json");
    let mut world = World::generate(42);
    // A completely removed legacy chunk must never regenerate terrain on upgrade.
    for x in 0..16 {
        for y in -16..0 {
            for z in 0..16 {
                world.set([x, y, z], 0);
            }
        }
    }
    world.save(&path).unwrap();
    let mut old: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    old["format_version"] = serde_json::json!(1);
    std::fs::write(&path, serde_json::to_vec(&old).unwrap()).unwrap();
    let mut world = World::load(&path).unwrap();
    world.enable_streaming();
    assert!(world.stream_around([0.0; 3]));
    assert_eq!(world.get([1, -1, 1]), 0);
    assert!(world.stream_around([100.0, 5.0, 0.0]));
    assert!(world.set([100, 12, 0], 255));
    assert!(world.set([100, -1, 0], 0));
    let revision = world.chunk_revision([6, 0, 0]);
    assert!(world.stream_around([-150.0, 5.0, 0.0]));
    assert_eq!(world.chunk_revision([6, 0, 0]), None);
    let attachment = serde_json::json!({"camera":[-150,5,0],"version":1});
    world
        .save_with_attachment(&path, Some(attachment.clone()))
        .unwrap();
    let mut world = World::load(&path).unwrap();
    assert!(world.is_streaming());
    assert_eq!(world.attachment(), Some(&attachment));
    assert!(world.stream_around([100.0, 5.0, 0.0]));
    assert_eq!(world.get([100, 12, 0]), 255);
    assert_eq!(world.get([100, -1, 0]), 0);
    assert_ne!(world.chunk_revision([6, 0, 0]), revision);
    assert!(world.stats().chunks <= 147);
    assert!(world.stream_around([0.0; 3]));
    assert_eq!(world.get([1, -1, 1]), 0);
    world.save(&path).unwrap();
    assert_eq!(World::load(&path).unwrap().attachment(), Some(&attachment));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn streaming_is_explicit_bounded_and_stable_with_invalid_inputs() {
    let mut world = World::new(1);
    assert!(!world.stream_around([0.0; 3]));
    assert_eq!(world.stats().chunks, 0);
    world.enable_streaming();
    assert!(world.stream_around([0.0; 3]));
    let revision = world.revision();
    assert!(!world.stream_around([1.0; 3]));
    assert!(!world.stream_around([f32::NAN; 3]));
    assert_eq!(revision, world.revision());
    assert!(!world.set([100, 1, 0], 1)); // not resident
    assert!(!world.set([WORLD_LIMIT, 0, 0], 1));
    assert!(world.set([0, 0, 0], 1)); // resident original square, initially air
    for x in (-256..256).step_by(16) {
        world.stream_around([x as f32, 0.0, x as f32]);
        assert!(world.stats().chunks <= 147);
        assert!(world
            .chunk_keys()
            .iter()
            .all(|key| (-16..16).contains(&key[0]) && (-16..16).contains(&key[2])));
    }
}

#[test]
fn override_capacity_rejects_new_chunks_without_losing_existing_edits() {
    let mut world = World::new(0);
    // A full legacy archive is legal even if most chunks lie outside this slice.
    for chunk in 0..512 {
        world.set([chunk * 16, 0, 0], 1);
    }
    world.enable_streaming();
    world.stream_around([0.0; 3]);
    assert_eq!(world.stats().stored_overrides, 512);
    assert!(!world.set([-1, 0, 0], 7));
    assert_eq!(world.get([-1, 0, 0]), 0);
    assert!(world.set([0, 0, 0], 7));
    assert_eq!(world.get([0, 0, 0]), 7);
}

#[test]
fn corrupted_streaming_metadata_and_oversized_attachment_preserve_save() {
    let dir =
        std::env::temp_dir().join(format!("matterweave-stream-invalid-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("world.json");
    let mut world = World::new(42);
    world.enable_streaming();
    world.stream_around([0.0; 3]);
    world.set([0, 0, 0], 1);
    world.set([0, 0, 0], 0);
    world.save(&path).unwrap();
    let original = std::fs::read(&path).unwrap();
    assert!(world
        .save_with_attachment(&path, Some(serde_json::json!("x".repeat(12 * 1024 * 1024))))
        .is_err());
    assert_eq!(std::fs::read(&path).unwrap(), original);
    let snapshot: serde_json::Value = serde_json::from_slice(&original).unwrap();
    for replacement in [
        serde_json::json!([[0, 0, 0], [0, 0, 0]]),
        serde_json::json!([[i32::MAX, 0, 0]]),
    ] {
        let mut corrupt = snapshot.clone();
        corrupt["streaming"]["empty_overrides"] = replacement;
        std::fs::write(&path, serde_json::to_vec(&corrupt).unwrap()).unwrap();
        assert!(World::load(&path).is_err());
    }
    let mut corrupt = snapshot.clone();
    corrupt["streaming"]["center"] = serde_json::json!([500, 0]);
    std::fs::write(&path, serde_json::to_vec(&corrupt).unwrap()).unwrap();
    assert!(World::load(&path).is_err());
    let mut saturated = snapshot;
    saturated["revision"] = serde_json::json!(u64::MAX);
    std::fs::write(&path, serde_json::to_vec(&saturated).unwrap()).unwrap();
    let mut world = World::load(&path).unwrap();
    assert_eq!(world.revision(), u64::MAX);
    assert!(!world.set([0, 0, 0], 1));
    assert!(!world.stream_around([100.0; 3]));
    assert!(world.stats().chunks > 0); // generated outer terrain was restored
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn terrain_extension_meets_both_legacy_edges_without_tall_walls() {
    let mut world = World::generate(8712);
    world.enable_streaming();
    world.stream_around([0.0; 3]);
    let height = |x, z| (-8..20).rev().find(|&y| world.get([x, y, z]) != 0).unwrap();
    for along in -31..31 {
        for (inner, outer) in [(31, 32), (-32, -33)] {
            assert!((height(inner, along) - height(outer, along)).abs() <= 1);
            assert!((height(along, inner) - height(along, outer)).abs() <= 1);
        }
    }
}

#[test]
fn streaming_reversals_invalidate_border_meshes_without_invalidating_interior() {
    let mut world = World::generate(8712);
    world.enable_streaming();
    world.stream_around([0.0; 3]);
    let interior = world.chunk_revision([0, 0, 0]);
    let border = world.chunk_revision([3, 0, 0]);
    assert!(border.is_some());
    world.stream_around([16.0, 0.0, 0.0]);
    assert_eq!(world.chunk_revision([0, 0, 0]), interior);
    assert_ne!(world.chunk_revision([3, 0, 0]), border);
    let updated = world.chunk_revision([3, 0, 0]);
    world.stream_around([0.0; 3]);
    assert_ne!(world.chunk_revision([3, 0, 0]), updated);
    assert_eq!(world.chunk_revision([0, 0, 0]), interior);
}
