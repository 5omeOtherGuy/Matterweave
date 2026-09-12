use matterweave_core::{
    AsyncWorld, Mesh, World, CHUNK_EDGE, MAX_MESH_RESULTS, MAX_QUEUED_MESH_JOBS,
    STREAM_RADIUS_CHUNKS,
};
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

/// Chunks in a published window: 7x7x3, from the declared stream radius.
const WINDOW_CHUNKS: usize =
    (2 * STREAM_RADIUS_CHUNKS as usize + 1) * (2 * STREAM_RADIUS_CHUNKS as usize + 1) * 3;

/// The declared residency window for the interior eye positions used here: the eye's
/// chunk plus `STREAM_RADIUS_CHUNKS` in x and z, and the three vertical layers.
fn window_keys(eye: [f32; 3]) -> BTreeSet<[i32; 3]> {
    let center = [eye[0], eye[2]].map(|v| (v.floor() as i32).div_euclid(CHUNK_EDGE));
    let mut keys = BTreeSet::new();
    for x in center[0] - STREAM_RADIUS_CHUNKS..=center[0] + STREAM_RADIUS_CHUNKS {
        for z in center[1] - STREAM_RADIUS_CHUNKS..=center[1] + STREAM_RADIUS_CHUNKS {
            for y in -1..2 {
                keys.insert([x, y, z]);
            }
        }
    }
    keys
}

/// No orphaned or duplicated chunks: residency is exactly the declared span and
/// every materialized chunk is inside it.
fn assert_window_shape(world: &World, eye: [f32; 3]) {
    let resident: BTreeSet<[i32; 3]> = world
        .stream_resident_chunks()
        .unwrap()
        .into_iter()
        .collect();
    assert_eq!(resident.len(), WINDOW_CHUNKS, "declared window size");
    assert_eq!(resident, window_keys(eye), "not the declared window span");
    assert!(
        world.chunk_keys().iter().all(|key| resident.contains(key)),
        "a materialized chunk is outside the resident window"
    );
}

/// Sampled interior positions report support. This is the publication contract that
/// keeps movement and collision off an evicted window; whether a physics body is
/// physically supported is an integration-level gate, not a core claim.
fn assert_window_supported(world: &World, eye: [f32; 3]) {
    assert_window_shape(world, eye);
    let base = [eye[0].floor(), eye[2].floor()];
    for dx in [-40.0, -24.0, -8.0, 8.0, 24.0, 40.0] {
        for dz in [-40.0, -24.0, -8.0, 8.0, 24.0, 40.0] {
            for y in [-14.0, 4.0, 30.0] {
                let position = [base[0] + dx, y, base[1] + dz];
                assert!(
                    world.stream_contains_position(position, 1.0),
                    "{position:?} is unsupported inside the window around {eye:?}"
                );
            }
        }
    }
}

/// The content publication must preserve: the resident window, the resident chunk set
/// and every material in it. Revisions legitimately differ between the asynchronous
/// and synchronous paths, so they are asserted present, not equal.
fn assert_same_content(actual: &World, expected: &World) {
    assert_eq!(actual.chunk_keys(), expected.chunk_keys());
    assert_eq!(actual.stats().chunks, expected.stats().chunks);
    assert_eq!(actual.stats().solid_voxels, expected.stats().solid_voxels);
    for key in expected.chunk_keys() {
        assert!(
            actual.chunk_revision(key).is_some(),
            "chunk {key:?} has no revision"
        );
        for z in 0..16 {
            for y in 0..16 {
                for x in 0..16 {
                    let cell = [key[0] * 16 + x, key[1] * 16 + y, key[2] * 16 + z];
                    assert_eq!(actual.get(cell), expected.get(cell), "cell {cell:?}");
                }
            }
        }
    }
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
    assert!(world.set([0, 20, 0], 1));
    let mut jobs = AsyncWorld::new();
    let keys = world.chunk_keys();
    assert!(keys.len() > MAX_QUEUED_MESH_JOBS);

    // Ensure the edited chunk has an old snapshot to reject, rather than
    // assuming the flood happened to admit it before its queue filled.
    assert!(jobs.request_mesh(&world, [0, 1, 0]));

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
    // Do not start draining while the flood is still executing. Admission
    // refusals are not result discards: a slow worker could otherwise deliver
    // everything without ever filling the result buffer. Wait on completion,
    // not elapsed time, so saturation is exercised under any worker schedule.
    let end = Instant::now() + LIMIT;
    loop {
        let stats = jobs.stats();
        assert!(stats.within_bounds(), "{stats:?}");
        assert!(stats.queued_meshes <= MAX_QUEUED_MESH_JOBS, "{stats:?}");
        assert!(stats.mesh_results <= MAX_MESH_RESULTS, "{stats:?}");
        assert!(
            stats.inflight <= 1 && stats.queued_streams <= 1,
            "{stats:?}"
        );
        if stats.queued_meshes == 0 && stats.inflight == 0 {
            assert!(stats.discarded > 0, "flood did not saturate: {stats:?}");
            break;
        }
        assert!(Instant::now() < end, "flood never completed: {stats:?}");
        std::thread::yield_now();
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

#[test]
fn returning_home_cancels_a_prepared_destination_and_repeated_requests_coalesce() {
    let mut world = streamed(20260907, [0., 4., 0.]);
    let revision = world.revision();
    let mut jobs = AsyncWorld::new();
    assert!(jobs.available());
    assert!(jobs.request_stream(&world, [120., 4., 0.]));
    for _ in 0..100 {
        assert!(!jobs.request_stream(&world, [120., 4., 0.]));
    }
    settle(&jobs);
    assert!(!jobs.request_stream(&world, [0., 4., 0.]));
    assert!(!jobs.poll_stream(&mut world));
    assert_eq!(world.revision(), revision);
    assert!(world.stream_contains_position([0., 4., 0.], 1.));
    assert!(jobs.request_stream(&world, [-120., 4., 0.]));
    publish(&mut jobs, &mut world);
    assert!(world.stream_contains_position([-120., 4., 0.], 1.));
}

#[test]
fn edit_then_reverse_leaves_the_window_equal_to_an_unedited_world() {
    let seed = 20260912;
    let eye = [0.0, 4.0, 0.0];
    let mut world = streamed(seed, eye);
    let untouched = streamed(seed, eye);

    // One generated-air cell whose chunk is generated empty, and one generated-solid
    // cell: the reversal shapes are "create then remove" and "edit then restore".
    let air = [0, 20, 0];
    let solid = [0, -4, 0];
    assert_eq!(world.get(air), 0);
    let solid_material = world.get(solid);
    assert_ne!(solid_material, 0);
    for (cell, original, temporary) in [(air, 0u8, 9u8), (solid, solid_material, 0u8)] {
        assert!(world.set(cell, temporary), "edit of {cell:?} was refused");
        assert!(world.set(cell, original), "reverse of {cell:?} was refused");
        assert_eq!(world.get(cell), original);
    }

    // Round-trip the window through the worker, so publication has every chance to
    // resurrect the temporary material or leave an orphaned chunk behind.
    let mut jobs = AsyncWorld::new();
    assert!(jobs.request_stream(&world, [120.0, 4.0, 0.0]));
    publish(&mut jobs, &mut world);
    assert!(jobs.request_stream(&world, eye));
    publish(&mut jobs, &mut world);

    assert_window_supported(&world, eye);
    assert_same_content(&world, &untouched);
    assert_eq!(world.get(air), 0);
    assert_eq!(world.get(solid), solid_material);
}

#[test]
fn eviction_then_rerequest_restores_the_window_and_supports_it_again() {
    let seed = 20260912;
    let origin = [0.0, 4.0, 0.0];
    let far = [120.0, 4.0, 0.0];
    let edit = [0, 20, 0];
    let mut world = streamed(seed, origin);
    assert!(world.set(edit, 9));
    let mut stayed = streamed(seed, origin);
    assert!(stayed.set(edit, 9));

    let mut jobs = AsyncWorld::new();
    assert!(jobs.request_stream(&world, far));
    publish(&mut jobs, &mut world);
    assert_window_supported(&world, far);
    assert!(!world.stream_contains_position(origin, 1.0));
    assert_eq!(world.get(edit), 0, "an evicted chunk stayed materialized");

    assert!(jobs.request_stream(&world, origin));
    publish(&mut jobs, &mut world);
    assert_window_supported(&world, origin);
    assert_eq!(
        world.get(edit),
        9,
        "the edit did not survive eviction and re-entry"
    );
    assert_same_content(&world, &stayed);
}

#[test]
fn reset_cancels_queued_and_in_flight_work_and_returns_to_the_baseline() {
    let eye = [0.0, 4.0, 0.0];
    let mut world = streamed(20260912, eye);
    let revision = world.revision();
    let resident = world.stream_resident_chunks().unwrap();
    let keys = world.chunk_keys();
    let mut jobs = AsyncWorld::new();
    for &key in &keys {
        jobs.request_mesh(&world, key);
    }
    assert!(jobs.request_stream(&world, [120.0, 4.0, 0.0]));
    jobs.reset();

    // Stated baseline: every queue and staging slot is empty, so nothing cancelled
    // can be delivered. Only the executing unit may still hold its single slot.
    let stats = jobs.stats();
    assert_eq!(stats.queued_meshes, 0);
    assert_eq!(stats.queued_streams, 0);
    assert_eq!(stats.mesh_results, 0);
    assert_eq!(stats.mesh_result_bytes, 0);
    assert_eq!(stats.stream_results, 0);
    assert_eq!(stats.generation, 1);
    assert!(stats.within_bounds(), "{stats:?}");

    // Once the executing unit finishes, the counts are at zero and nothing from
    // before the cancellation is ever delivered.
    settle(&jobs);
    assert_eq!(jobs.stats().inflight, 0);
    assert!(
        !jobs.poll_stream(&mut world),
        "a cancelled window was published"
    );
    assert!(
        jobs.poll_mesh(&world).is_none(),
        "a cancelled mesh was published"
    );
    assert_eq!(world.revision(), revision);
    assert_eq!(world.stream_resident_chunks().unwrap(), resident);
    assert!(jobs.available());

    // Cancelled work is immediately re-requestable and still correct.
    for &key in &keys {
        jobs.request_mesh(&world, key);
    }
    let (key, mesh) = next_mesh(&mut jobs, &world);
    assert_eq!(world.chunk_revision(key), Some(mesh.revision));
    assert_eq!(dump(&mesh), dump(&world.mesh_chunk(key)));
}
