use super::*;

fn payload(world: &World, key: [i32; 3]) -> *const [u8; CHUNK_VOLUME] {
    &*world.chunks[&key].voxels
}

#[test]
fn snapshots_share_payloads_and_detach_only_changed_chunks() {
    let source = World::generate(8712);
    let mut snapshot = source.clone();
    for key in source.chunk_keys() {
        assert_eq!(payload(&source, key), payload(&snapshot, key));
    }
    let cell = [-1, -8, 0];
    let touched = address(cell).0;
    let old = source.get(cell);
    assert!(snapshot.set(cell, old.wrapping_add(1)));
    assert_eq!(source.get(cell), old);
    for key in source.chunk_keys() {
        assert_eq!(payload(&source, key) == payload(&snapshot, key), key != touched);
    }
    let detached = payload(&snapshot, touched);
    assert!(snapshot.set(cell, old.wrapping_add(2)));
    assert_eq!(payload(&snapshot, touched), detached);
    assert_eq!(source.get(cell), old);
}

#[test]
fn streaming_overrides_share_and_repeated_edits_do_not_copy_again() {
    let mut world = World::generate(8712);
    world.enable_streaming();
    world.stream_around([0.0; 3]);
    let key = [0, -1, 0];
    let snapshot = world.clone();
    let old = snapshot.get([0, -8, 0]);
    let override_ptr = |world: &World| -> *const [u8; CHUNK_VOLUME] {
        &*world.streaming.as_ref().unwrap().overrides[&key].as_ref().unwrap().voxels
    };
    assert_eq!(payload(&world, key), override_ptr(&world));
    assert_eq!(payload(&world, key), payload(&snapshot, key));
    assert!(world.set([0, -8, 0], old.wrapping_add(1)));
    let detached = payload(&world, key);
    assert_ne!(detached, payload(&snapshot, key));
    for material in 20..24 {
        assert!(world.set([0, -8, 0], material));
        assert_eq!(payload(&world, key), detached);
        assert_eq!(override_ptr(&world), detached);
    }
    assert_eq!(snapshot.get([0, -8, 0]), old);
    world.stream_around([-150.0, 0.0, 0.0]);
    world.stream_around([0.0; 3]);
    assert_eq!(world.get([0, -8, 0]), 23);
    assert_eq!(payload(&world, key), detached);
}

#[test]
fn rejected_and_unchanged_edits_keep_shared_payloads() {
    let mut world = World::generate(8712);
    world.enable_streaming();
    world.stream_around([0.0; 3]);
    let snapshot = world.clone();
    let key = [0, -1, 0];
    let revision = world.revision();
    assert!(!world.set([0, -8, 0], world.get([0, -8, 0])));
    assert!(!world.set([255, -8, 255], 9));
    assert_eq!(world.revision(), revision);
    world.revision = u64::MAX;
    assert!(!world.set([0, -8, 0], 19));
    assert_eq!(payload(&world, key), payload(&snapshot, key));
}

#[test]
fn deletion_and_reentry_preserve_snapshot_and_empty_override() {
    let mut world = World::new(8712);
    let cell = [-1, -8, 0];
    assert!(world.set(cell, 9));
    world.enable_streaming();
    world.stream_around([0.0; 3]);
    let snapshot = world.clone();
    assert!(world.set(cell, 0));
    assert_eq!(snapshot.get(cell), 9);
    assert!(!world.chunks.contains_key(&address(cell).0));
    world.stream_around([-150.0, 0.0, 0.0]);
    world.stream_around([0.0; 3]);
    assert_eq!(world.get(cell), 0);
    assert_eq!(snapshot.get(cell), 9);
}
