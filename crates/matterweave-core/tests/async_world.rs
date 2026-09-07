use matterweave_core::{AsyncWorld, Mesh, World, MAX_MESH_RESULTS, MAX_QUEUED_MESH_JOBS};
use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

/// Generous only against a stalled worker; a healthy run finishes far sooner.
/// These tests assert termination and validity, never timing.
const LIMIT: Duration = Duration::from_secs(60);

fn settle(jobs: &AsyncWorld) {
    let end = Instant::now() + LIMIT;
    loop {
        let stats = jobs.stats();
        if stats.queued_meshes == 0 && stats.queued_streams == 0 && stats.inflight == 0 {
            return;
        }
        assert!(Instant::now() < end, "worker never settled: {stats:?}");
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn publish(jobs: &mut AsyncWorld, world: &mut World) {
    let end = Instant::now() + LIMIT;
    while !jobs.poll_stream(world) {
        assert!(Instant::now() < end, "window was never published");
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn next_mesh(jobs: &mut AsyncWorld, world: &World) -> ([i32; 3], Mesh) {
    let end = Instant::now() + LIMIT;
    loop {
        if let Some(result) = jobs.poll_mesh(world) {
            return result;
        }
        assert!(Instant::now() < end, "mesh was never produced");
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Bit-exact vertex and index comparison against the synchronous mesher.
fn dump(mesh: &Mesh) -> (Vec<[u32; 9]>, Vec<u32>) {
    let vertices = mesh
        .vertices
        .iter()
        .map(|vertex| {
            std::array::from_fn(|i| match i {
                0..=2 => vertex.position[i].to_bits(),
                3..=5 => vertex.normal[i - 3].to_bits(),
                _ => vertex.color[i - 6].to_bits(),
            })
        })
        .collect();
    (vertices, mesh.indices.clone())
}

type Surface = ([i32; 3], [i32; 3], [u32; 3]);
/// Unit faces covered by each quad, with an outward winding check.
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
                assert!(result.insert((cell, normal, quad[0].color.map(f32::to_bits))));
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

fn streamed(seed: u64, eye: [f32; 3]) -> World {
    let mut world = World::generate(seed);
    world.enable_streaming();
    assert!(world.stream_around(eye));
    world
}

/// Requests every dirty chunk, respecting the bounded queue, until each resident
/// chunk has an accepted mesh. Returns the accepted meshes by key.
fn mesh_all(jobs: &mut AsyncWorld, world: &World) -> BTreeMap<[i32; 3], Mesh> {
    let mut accepted: BTreeMap<[i32; 3], Mesh> = BTreeMap::new();
    let end = Instant::now() + LIMIT;
    while accepted.len() < world.chunk_keys().len() {
        for key in world.chunk_keys() {
            if !accepted.contains_key(&key) {
                jobs.request_mesh(world, key);
            }
        }
        while let Some((key, mesh)) = jobs.poll_mesh(world) {
            accepted.insert(key, mesh);
        }
        assert!(Instant::now() < end, "meshing never completed");
    }
    accepted
}

#[test]
fn background_window_and_meshes_equal_the_synchronous_reference() {
    let far = [120.0, 4.0, -60.0];
    let mut reference = streamed(8712, [0.0; 3]);
    assert!(reference.stream_around(far));

    let mut world = streamed(8712, [0.0; 3]);
    let mut jobs = AsyncWorld::new();
    assert!(jobs.request_stream(&world, far));
    publish(&mut jobs, &mut world);

    assert_eq!(world.revision(), reference.revision());
    assert_eq!(world.chunk_keys(), reference.chunk_keys());
    assert_eq!(world.stats(), reference.stats());
    for key in reference.chunk_keys() {
        assert_eq!(world.chunk_revision(key), reference.chunk_revision(key));
        for index in 0..4096 {
            let cell = [
                key[0] * 16 + (index % 16),
                key[1] * 16 + (index / 16) % 16,
                key[2] * 16 + index / 256,
            ];
            assert_eq!(world.get(cell), reference.get(cell), "cell {cell:?}");
        }
    }

    let accepted = mesh_all(&mut jobs, &world);
    let mut faces = BTreeSet::new();
    for (&key, mesh) in &accepted {
        assert_eq!(world.chunk_revision(key), Some(mesh.revision));
        assert_eq!(dump(mesh), dump(&world.mesh_chunk(key)));
        for face in surfaces(mesh) {
            assert!(faces.insert(face), "duplicate face in {key:?}");
        }
    }
    assert_eq!(faces, surfaces(&reference.mesh()));
}

#[test]
fn edited_neighbor_or_removed_chunk_makes_a_prepared_mesh_unacceptable() {
    let mut world = World::new(0);
    for cell in [[15, 5, 5], [16, 5, 5], [20, 5, 5]] {
        world.set(cell, 1);
    }
    let mut jobs = AsyncWorld::new();

    // A shared-face edit in the western neighbor invalidates the prepared mesh.
    assert!(jobs.request_mesh(&world, [1, 0, 0]));
    assert!(world.set([15, 5, 5], 0));
    settle(&jobs);
    assert!(jobs.poll_mesh(&world).is_none());
    assert!(jobs.stats().discarded >= 1);

    // The dropped chunk is immediately requestable again and now acceptable.
    assert!(jobs.request_mesh(&world, [1, 0, 0]));
    let (key, mesh) = next_mesh(&mut jobs, &world);
    assert_eq!(key, [1, 0, 0]);
    assert_eq!(world.chunk_revision(key), Some(mesh.revision));
    assert_eq!(dump(&mesh), dump(&world.mesh_chunk(key)));

    // Emptying the chunk removes it: prepared geometry is rejected, and no new
    // job can be requested for a chunk that no longer exists.
    assert!(jobs.request_mesh(&world, [1, 0, 0]));
    for cell in [[16, 5, 5], [20, 5, 5]] {
        assert!(world.set(cell, 0));
    }
    settle(&jobs);
    assert_eq!(world.chunk_revision([1, 0, 0]), None);
    assert!(jobs.poll_mesh(&world).is_none());
    assert!(!jobs.request_mesh(&world, [1, 0, 0]));
}

#[test]
fn results_from_an_old_generation_or_a_changed_world_are_rejected() {
    let mut world = streamed(4242, [0.0; 3]);
    let mut jobs = AsyncWorld::new();

    // reset() cancels queued, in-flight and completed work.
    let key = world.chunk_keys()[0];
    assert!(jobs.request_mesh(&world, key));
    assert!(jobs.request_stream(&world, [120.0, 4.0, 0.0]));
    jobs.reset();
    settle(&jobs);
    assert!(jobs.poll_mesh(&world).is_none());
    assert!(!jobs.poll_stream(&mut world));
    assert_eq!(jobs.stats().generation, 1);
    assert!(jobs.request_mesh(&world, key)); // requestable again after reset
    let (accepted, mesh) = next_mesh(&mut jobs, &world);
    assert_eq!(world.chunk_revision(accepted), Some(mesh.revision));

    // An edit after the snapshot was taken must never be undone by publication.
    assert!(jobs.request_stream(&world, [120.0, 4.0, 0.0]));
    assert!(world.set([0, 20, 0], 9));
    settle(&jobs);
    assert!(!jobs.poll_stream(&mut world));
    assert_eq!(world.get([0, 20, 0]), 9);
    assert!(world.stream_contains_position([0.0, 4.0, 0.0], 1.0));
    assert!(!world.stream_contains_position([120.0, 4.0, 0.0], 1.0));

    // A synchronous rewindow (eviction and reentry) also invalidates preparation.
    assert!(jobs.request_stream(&world, [120.0, 4.0, 0.0]));
    assert!(world.stream_around([-120.0, 4.0, 0.0]));
    assert!(world.stream_around([0.0, 4.0, 0.0]));
    settle(&jobs);
    assert!(!jobs.poll_stream(&mut world));
    assert!(world.stream_contains_position([0.0, 4.0, 0.0], 1.0));

    // A superseded center is not published even though nothing else changed.
    assert!(jobs.request_stream(&world, [120.0, 4.0, 0.0]));
    settle(&jobs);
    assert!(jobs.request_stream(&world, [-120.0, 4.0, 0.0]));
    publish(&mut jobs, &mut world);
    assert!(world.stream_contains_position([-120.0, 4.0, 0.0], 1.0));
    assert!(!world.stream_contains_position([120.0, 4.0, 0.0], 1.0));
    // The edit was evicted with its chunk, not lost: it returns with the window.
    assert!(world.stream_around([0.0, 4.0, 0.0]));
    assert_eq!(world.get([0, 20, 0]), 9);
}

#[test]
fn saturated_requests_stay_bounded_and_still_deliver_the_latest_meshes() {
    let mut world = streamed(31337, [0.0; 3]);
    let mut jobs = AsyncWorld::new();
    let keys = world.chunk_keys();
    assert!(keys.len() > MAX_QUEUED_MESH_JOBS);

    // Flood without polling: refusals keep every buffer inside its bound.
    for _ in 0..8 {
        for &key in &keys {
            jobs.request_mesh(&world, key);
            let stats = jobs.stats();
            assert!(stats.queued_meshes <= MAX_QUEUED_MESH_JOBS, "{stats:?}");
            assert!(stats.mesh_results <= MAX_MESH_RESULTS, "{stats:?}");
            assert!(
                stats.inflight <= 1 && stats.queued_streams <= 1,
                "{stats:?}"
            );
        }
    }
    // Edit after the flood: only meshes matching the current revisions are accepted.
    assert!(world.set([0, 20, 0], 7));
    let accepted = mesh_all(&mut jobs, &world);
    assert_eq!(accepted.len(), world.chunk_keys().len());
    for (&key, mesh) in &accepted {
        assert_eq!(world.chunk_revision(key), Some(mesh.revision));
        assert_eq!(dump(mesh), dump(&world.mesh_chunk(key)));
    }
    let stats = jobs.stats();
    assert!(
        stats.mesh_results <= MAX_MESH_RESULTS && stats.discarded > 0,
        "{stats:?}"
    );
}

#[test]
fn shutdown_completes_while_results_and_queues_are_full() {
    let world = streamed(99, [0.0; 3]);
    let mut jobs = AsyncWorld::new();
    let keys = world.chunk_keys();
    let end = Instant::now() + LIMIT;
    // Never poll: fill the result buffer and leave the job queue saturated.
    while jobs.stats().mesh_results < MAX_MESH_RESULTS {
        for &key in &keys {
            jobs.request_mesh(&world, key);
        }
        assert!(
            Instant::now() < end,
            "results never filled: {:?}",
            jobs.stats()
        );
    }
    for &key in &keys {
        jobs.request_mesh(&world, key);
    }
    let (done, wait) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        drop(jobs);
        let _ = done.send(());
    });
    wait.recv_timeout(LIMIT).expect("shutdown deadlocked");
}

#[test]
fn published_residency_gates_movement_at_window_boundaries() {
    let mut world = World::generate(8712);
    assert!(world.stream_contains_position([0.0; 3], 1.0)); // no streaming: resident
    world.enable_streaming();
    assert!(!world.stream_contains_position([0.0; 3], 1.0)); // no window published
    assert!(world.stream_around([0.0; 3]));

    // The window spans chunks -3..=3, so cells -48..=63 on x and z.
    assert!(world.stream_contains_position([0.0, 4.0, 0.0], 1.0));
    assert!(world.stream_contains_position([62.0, 4.0, 62.0], 1.0));
    assert!(!world.stream_contains_position([63.0, 4.0, 0.0], 1.0));
    assert!(!world.stream_contains_position([-48.0, 4.0, 0.0], 1.0));
    assert!(world.stream_contains_position([-48.0, 4.0, 0.0], 0.0));
    // Residency is not the nonempty chunk list: resident air is safe to enter.
    assert!(!world.chunk_keys().contains(&[0, 1, 0]));
    assert!(world.stream_contains_position([8.0, 24.0, 8.0], 1.0));
    // Outside the vertical simulation domain and invalid inputs are never safe.
    assert!(!world.stream_contains_position([0.0, 31.5, 0.0], 1.0));
    assert!(!world.stream_contains_position([0.0, -16.0, 0.0], 1.0));
    assert!(!world.stream_contains_position([f32::NAN, 4.0, 0.0], 1.0));
    assert!(!world.stream_contains_position([0.0, 4.0, 0.0], f32::INFINITY));
    assert!(!world.stream_contains_position([0.0, 4.0, 0.0], -1.0));

    // The old window stays complete and traversable until the new one is published.
    let mut jobs = AsyncWorld::new();
    assert!(!jobs.request_stream(&world, [f32::NAN; 3]));
    assert!(!jobs.request_stream(&world, [0.0, 4.0, 0.0])); // already published
    assert!(jobs.request_stream(&world, [70.0, 4.0, 0.0]));
    settle(&jobs);
    assert!(world.stream_contains_position([0.0, 4.0, 0.0], 1.0));
    assert!(!world.stream_contains_position([70.0, 4.0, 0.0], 1.0));
    publish(&mut jobs, &mut world);
    assert!(world.stream_contains_position([70.0, 4.0, 0.0], 1.0));
    assert!(world.stats().chunks <= 147);
}
