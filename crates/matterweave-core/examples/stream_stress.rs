//! Native/Android sustained correctness gate, not a rendering benchmark.
use matterweave_core::{
    AsyncWorld, World, MAX_MESH_RESULTS, MAX_MESH_RESULT_BYTES, MAX_QUEUED_MESH_JOBS,
};
use std::{
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

fn bounds(worker: &AsyncWorld, world: &World) {
    let s = worker.stats();
    assert!(worker.available(), "background worker exited");
    assert!(s.queued_meshes <= MAX_QUEUED_MESH_JOBS);
    assert!(s.mesh_results <= MAX_MESH_RESULTS);
    assert!(s.mesh_result_bytes <= MAX_MESH_RESULT_BYTES);
    assert!(s.queued_streams <= 1 && s.stream_results <= 1 && s.inflight <= 1);
    assert!(world.stats().chunks <= 147 && world.stats().stored_overrides <= 512);
    assert!(world.stream_resident_chunks().unwrap().len() <= 147);
}

fn equal_world(actual: &World, expected: &World) {
    assert_eq!(
        actual.stream_resident_chunks(),
        expected.stream_resident_chunks()
    );
    assert_eq!(actual.chunk_keys(), expected.chunk_keys());
    for key in expected.chunk_keys() {
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

fn drain(worker: &mut AsyncWorld, world: &World) -> usize {
    let mut count = 0;
    while let Some((key, mesh)) = worker.poll_mesh(world) {
        let reference = world.mesh_chunk(key);
        assert_eq!(mesh.revision, reference.revision);
        assert_eq!(mesh.indices, reference.indices);
        assert_eq!(
            bytemuck::cast_slice::<_, u8>(&mesh.vertices),
            bytemuck::cast_slice::<_, u8>(&reference.vertices)
        );
        count += 1;
    }
    count
}

fn main() {
    let mut args = std::env::args().skip(1);
    let seconds: u64 = args
        .next()
        .expect("usage: stream_stress SECONDS SAVE_PATH")
        .parse()
        .unwrap();
    assert!((1..=1800).contains(&seconds));
    let path = PathBuf::from(args.next().expect("explicit disposable save path required"));
    assert!(args.next().is_none());
    assert!(!path.exists(), "refusing to overwrite an existing save");
    let mut world = World::new(20260908);
    // Unmergeable faces pressure the mesh queues, including negative chunk boundaries.
    for x in -32_i32..32 {
        for y in 0_i32..16 {
            for z in -32_i32..32 {
                if (x + y + z).rem_euclid(2) == 0 {
                    world.set([x, y, z], 7);
                }
            }
        }
    }
    world.enable_streaming();
    world.stream_around([0., 8., 0.]);
    let mut worker = AsyncWorld::new();
    let start = Instant::now();
    let mut cycles = 0usize;
    let mut meshes = 0usize;
    let mut resets = 0usize;
    let mut reloads = 0usize;
    let mut peak_result_bytes = 0usize;
    let mut peak_queued = 0usize;
    let mut position = [0., 8., 0.];
    let route = [
        [208., 8., 208.],
        [-208., 8., -208.],
        [0., 8., 0.],
        [208., 8., -208.],
        [-208., 8., 208.],
        [0., 8., 0.],
    ];
    while start.elapsed() < Duration::from_secs(seconds) || cycles < 6 {
        for key in world.chunk_keys() {
            worker.request_mesh(&world, key);
        }
        let cell = [position[0] as i32, 12, position[2] as i32];
        let material = if world.get(cell) == 255 { 254 } else { 255 };
        assert!(world.set(cell, material));
        // A->B->A requests exercise supersession while mesh work is also pending.
        let destination = route[cycles % route.len()];
        worker.request_stream(&world, destination);
        worker.request_stream(&world, [-destination[0], 8., -destination[2]]);
        worker.request_stream(&world, destination);
        if cycles.is_multiple_of(7) {
            worker.reset();
            resets += 1;
            worker.request_stream(&world, destination);
        }
        let mut reference = world.clone();
        let changed = reference.stream_around(destination);
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            bounds(&worker, &world);
            let s = worker.stats();
            peak_result_bytes = peak_result_bytes.max(s.mesh_result_bytes);
            peak_queued = peak_queued.max(s.queued_meshes);
            meshes += drain(&mut worker, &world);
            if worker.poll_stream(&mut world) {
                assert!(changed);
                break;
            }
            if !changed {
                break;
            }
            assert!(Instant::now() < deadline, "stream publication timed out");
            thread::sleep(Duration::from_millis(1));
        }
        equal_world(&world, &reference);
        bounds(&worker, &world);
        position = destination;
        if cycles.is_multiple_of(11) {
            // Queue work before replacing the authoritative world from persistence.
            worker.request_stream(&world, [96., 8., 96.]);
            world.save(&path).unwrap();
            let mut restored = World::load(&path).unwrap();
            restored.stream_around(position);
            equal_world(&restored, &world);
            worker.reset();
            resets += 1;
            world = restored;
            reloads += 1;
        }
        cycles += 1;
        if cycles.is_multiple_of(6) {
            println!("{{\"elapsed_s\":{:.3},\"cycles\":{cycles},\"meshes_checked\":{meshes},\"resets\":{resets},\"reloads\":{reloads},\"peak_result_bytes\":{peak_result_bytes},\"peak_queued\":{peak_queued},\"discarded\":{}}}",start.elapsed().as_secs_f64(),worker.stats().discarded);
        }
    }
    // Verify progress of a stable final mesh request, not just rejection of stale jobs.
    worker.reset();
    let key = world.chunk_keys()[0];
    assert!(worker.request_mesh(&world, key));
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        bounds(&worker, &world);
        let n = drain(&mut worker, &world);
        if n > 0 {
            meshes += n;
            break;
        }
        assert!(Instant::now() < deadline, "mesh publication timed out");
        thread::sleep(Duration::from_millis(1));
    }
    drop(worker);
    std::fs::remove_file(&path).unwrap();
    println!("PASS stream stress: seconds={:.3} cycles={cycles} meshes={meshes} resets={resets} reloads={reloads} peak_result_bytes={peak_result_bytes} peak_queued={peak_queued}; CPU correctness only",start.elapsed().as_secs_f64());
}
