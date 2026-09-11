# matterweave-core

Authoritative voxel world data, deterministic reference queries, derived surface meshes,
versioned persistence, a platform-independent input service and bounded asynchronous
preparation. No Android, windowing, graphics-backend or physics types cross the public
interface.

## What it provides

| Area | What it is |
| --- | --- |
| `World` | Sparse, deterministically ordered 16³ chunks, one byte per material (0 air, 1–255 solid), Euclidean addressing across negative coordinates, reclaimed empty chunks and a monotonic content revision. |
| Generation | `World::generate(seed)`: the original version-1 integer terrain fixture across x/z −32…31 with groves, an arch and mineral outcrops. A small integration scene, not a world-size or fidelity requirement. |
| Queries | `World::raycast`: normalized double-precision grid DDA returning distance in world units, with half-open cells and tie handling. |
| Meshing | `World::mesh` (whole-world exposed-face reference) and `World::mesh_chunk` (18³ padded block-mesh greedy quads), both returning an owned `Mesh` tagged with the world revision. |
| Persistence | Version-2 writer accepting version 1, with atomic save and an optional opaque app attachment. |
| Streaming | Opt-in `enable_streaming` / `stream_around` with a bounded resident window and a bounded authoritative override archive. |
| Input | `InputService`: platform-independent multi-touch pointer, zone and keyboard tracker, with `VirtualKey` mappings. |
| Async preparation | `AsyncWorld`: one standard-library worker preparing complete streaming windows and greedy chunk meshes within fixed bounds. |

### Key contracts

- Changed edits advance `revision`; unchanged edits do not. `World::mesh()` tags its result
  with that revision. Revisions identify edits within one world, not globally unique world
  instances, so a caller replacing a whole world must invalidate pending work.
- An exhausted u64 revision makes the world read-only (`set` returns false), so an imported
  snapshot cannot overflow, panic or reuse old revision numbers.
- `raycast` clamps finite ranges to `MAX_RAY_DISTANCE` (4096 units) and rejects zero/nonfinite
  directions, invalid origins and negative/nonfinite ranges. Cells are half-open; starting
  inside a solid returns distance zero and a zero normal. Exact simultaneous crossings
  advance all tied axes, and the lowest crossed axis supplies the normal. Queries cannot
  overflow the i32 coordinate space.
- Persistence writes version 2 and accepts version 1. Non-streamed worlds store complete
  snapshots; streamed worlds store up to 512 authoritative chunk overrides (including
  explicit air), the active window centre and an extension-generator version. Resident
  procedural terrain is derived and regenerated on load. A 12 MiB JSON cap bounds the whole
  save including optional app metadata. Unknown versions, duplicate chunks, invalid
  coordinates, malformed lengths and empty serialized nonempty chunks are rejected.
  `save_with_attachment` atomically stores world and app state together; `save` preserves any
  loaded attachment. The app owns validation of the attachment schema.
- Saving uses a unique same-directory temporary file, buffered write, file sync and atomic
  rename. Failure before rename preserves the previous destination. Directory sync after
  rename is best-effort; power-loss durability is not guaranteed. Use the application's
  private writable data directory.
- Streaming is opt-in; `World::new` stays empty and `generate` retains the original reference
  fixture. `stream_around` synchronously publishes a radius-three chunk window (at most
  7×7×3 chunks), clipped to x/z [−256, 256), y [−16, 32). Only resident cells inside that
  domain accept edits. The original x/z [−32, 32) square remains snapshot-authoritative:
  missing legacy chunks mean air, including fully removed chunks. Saved chunks outside the
  domain remain in the override archive and are preserved on save, although they cannot be
  visited in this slice. This is bounded streaming, not virtualized LOD.
- The first edit of a generated chunk copies its authoritative 4096-byte payload into a
  bounded override archive; later edits update that copy. Reaching 512 stored overrides
  rejects edits requiring another slot, preserving already saved data; `stored_overrides`
  reports consumption. Eviction never discards an override.
- `mesh_chunk` adapts an 18³ padded material volume to block-mesh greedy quads, preserving
  material IDs, cross-chunk occlusion and CCW winding. All materials are opaque.
  `chunk_revision` changes only for a local edit, a shared-face
  neighbour edit or a relevant load/unload; an absent chunk returns `None`. A derived result
  may be published only if its key and revision still match the current world.
- `allocated_bytes` includes resident and override voxel payloads, excluding map and
  allocator overhead, temporary serialization buffers and meshes; `chunks` and `solid_voxels`
  describe resident content only. f32 vertices lose voxel precision at extreme legacy
  coordinates; the streamed domain stays close to origin.
- `AsyncWorld` prepares complete windows and greedy chunk meshes on one worker. Stream
  snapshots carry seed, source revision, controller generation and the latest desired centre;
  mesh jobs copy only an 18³ material halo and validate current chunk revisions. Reversal,
  edits, reset and eviction reject stale publications. Repeated identities coalesce, and
  stream and mesh service alternate when both are ready.
- The `AsyncWorld` queue holds 32 mesh jobs, at most one pending window, one executing job,
  eight mesh results and at most 8 MiB allocated mesh-vector capacity. Window snapshots are
  bounded in count; the roughly 2.6 MiB maximum per snapshot is voxel payload only, excluding
  attachment/collection overhead. Cloning still occurs on the caller.
- Poll before requesting dirty meshes again, cap requests/uploads per frame, and retry
  refused or dropped work later. `available()` permits a synchronous fallback after worker
  startup/exit failure. `reset()` invalidates work after world replacement or synchronous
  rewindowing. Collision publication remains synchronous and precedes movement; use
  `stream_contains_position` for character gating and `stream_resident_chunks` to keep nearby
  dynamic bodies frozen until their supporting collision window is published. A published
  empty chunk is resident air, not missing work.

## Public surface

| Item | Notes |
| --- | --- |
| `World` | `new`, `seed`, `revision`, `get`, `set`, `chunk_keys`, `chunk_revision`, `stats`, `generate`; plus `mesh`, `mesh_chunk`, persistence and streaming methods. |
| `Mesh`, `Vertex` | Owned derived surface: `vertices`, `indices`, `revision`; `Vertex` is `repr(C)` Pod (`position`, `normal`, `color`). |
| `WorldStats` | `chunks`, `solid_voxels`, `stored_overrides`, `allocated_bytes`. |
| `RayHit`, `MAX_RAY_DISTANCE` | `cell`, `normal`, `distance`, `material`. |
| Persistence | `World::save`, `save_with_attachment`, `load`, `attachment`. |
| Streaming | `enable_streaming`, `is_streaming`, `contains_stream_cell`, `stream_resident_chunks`, `stream_contains_position`, `stream_around`; `STREAM_RADIUS_CHUNKS`, `STREAM_MIN_Y`, `STREAM_MAX_Y`, `WORLD_LIMIT`. |
| `InputService`, `VirtualKey` | Touch/keyboard tracker: zone setup, pointer/key events, motion/look/action consumption. |
| `AsyncWorld`, `AsyncStats` | `new`, `request_stream`, `poll_stream`, `request_mesh`, `poll_mesh`, `reset`, `available`, `stats`; `MAX_QUEUED_MESH_JOBS`, `MAX_MESH_RESULTS`, `MAX_MESH_RESULT_BYTES`. |
| Constants | `CHUNK_EDGE` (16), `CHUNK_VOLUME` (4096), `GENERATOR_VERSION` (1), `FORMAT_VERSION` (2). |

## Invariants and guarantees

- Deterministic ordering: equal worlds produce equal mesh bytes and round-trip to equal
  snapshot bytes.
- `World` holds no asynchronous jobs; `AsyncWorld` is a separate, explicitly polled owner.
- No `unsafe` code in the crate. Dependencies are pinned exactly in `Cargo.toml`; see
  [DEPENDENCIES](../../docs/DEPENDENCIES.md) for source, version and upstream license. Those
  are dependency licenses, not a project-license choice.
- [block-mesh 0.2.0](https://docs.rs/block-mesh/0.2.0/block_mesh/) is adopted for greedy
  quad extraction after inspecting its padded-volume contract, merge-value support, face
  orientation and output index code. The adapter uses material identity as the merge key; the
  original reference mesher remains for equivalence tests. Host geometry reduction is
  evidence for this bounded optimization, not a mobile frame-time comparison or a final
  ray/mesh/hybrid decision.
- The equal-scene regression (seed 8712, three explicit edits in `tests/streaming.rs`)
  measured 27,744 reference triangles and 6,660 greedy triangles: 76.0% fewer, with identical
  unit faces and materials. Reproduce with
  `cargo test -p matterweave-core greedy_surfaces -- --nocapture`. This is a geometry count,
  not a frame-time or GPU benchmark.

## Limits and what it does not do

- `World::mesh()` is a whole-world exposed-face reference, not greedy meshing, LOD or
  streaming; `mesh_chunk` is the bounded greedy path. Collision and render consumers must
  synchronize because no asynchronous jobs exist inside `World`.
- The `generate` fixture is a small integration scene, not a world-size requirement.
- Streaming is a fixed radius-three window over a bounded domain, with a 512-override
  archive. It is not virtualized LOD, and saved chunks outside the window cannot be visited.
- Power-loss durability after rename is not guaranteed; directory sync is best-effort.
- `allocated_bytes` and `MAX_MESH_RESULT_BYTES` are logical payload budgets, not measured
  process memory. No measured mobile speed advantage follows from the async bounds alone.
- No Android, windowing, graphics or physics integration is provided by this crate; native
  integration and device behaviour are verified by the application.

## How it is tested

From the repository root:

```sh
cargo test -p matterweave-core
```

- The v0.2 suite passes 22 integration tests on Rust 1.96.0 Linux x86_64. It adds exact
  greedy/reference unit-face and material equivalence, boundary revision locality, streaming
  bounds and reversals, legacy removed-chunk migration, and evicted edits reloaded together
  with opaque app state.
- Tests exercise negative and chunk boundaries, sparse-reference edits, seeded terrain, ray
  ranges/invalid inputs/edge crossings, surface winding and seam occlusion, revision
  invalidation, snapshot round trips, malformed versions/chunks, save size limits and
  temporary-file cleanup after a failed rename.
- The original v0.1 run on 2026-09-07 used Rust 1.96.0 on Linux x86_64 and passed 14
  integration tests, `cargo clippy -p matterweave-core --all-targets --locked -- -D warnings`
  and scoped `rustfmt --check`. An instrumented host run measured 94.40% production-source
  line coverage, 94.12% region coverage and 95% function coverage; branch coverage was not
  instrumented. These are correctness-test coverage quantities, not performance or Android
  validation. The imported-revision regression was first observed to panic, then passed after
  exhausted edits were made non-mutating.
- `examples/stream_stress.rs` exercises the asynchronous preparation path. Native integration
  and device behaviour are verified by the application, not by host core tests.

Coverage is reproducible with Rust's `llvm-tools` component and a **fresh** target directory,
keeping previous instrumented binaries and profiles out of the run:

```bash
rustup component add llvm-tools
export CARGO_TARGET_DIR=/mnt/bench/matterweave-dev/core-coverage-fresh
export CARGO_BUILD_JOBS=2 CARGO_PROFILE_DEV_DEBUG=0
export RUSTFLAGS='-C instrument-coverage'
mkdir -p "$CARGO_TARGET_DIR/profiles"
export LLVM_PROFILE_FILE="$CARGO_TARGET_DIR/profiles/core-%m-%p.profraw"
cargo test -p matterweave-core --tests --locked
LLVM_BIN="$(rustc --print sysroot)/lib/rustlib/x86_64-unknown-linux-gnu/bin"
"$LLVM_BIN/llvm-profdata" merge -sparse "$CARGO_TARGET_DIR"/profiles/*.profraw \
  -o "$CARGO_TARGET_DIR/core.profdata"
objects=()
for binary in "$CARGO_TARGET_DIR"/debug/deps/api-* "$CARGO_TARGET_DIR"/debug/deps/world-*; do
  if [[ -f "$binary" && -x "$binary" ]]; then objects+=(--object "$binary"); fi
done
"$LLVM_BIN/llvm-cov" report "${objects[@]}" \
  --instr-profile "$CARGO_TARGET_DIR/core.profdata" \
  --ignore-filename-regex '(/.cargo/|/tests/|/rustc/)'
```

The recorded run used `/mnt/bench/matterweave-dev/core-coverage`; substitute an appropriate
writable data path and the matching host triple in `LLVM_BIN` on other machines. Keep
`LLVM_PROFILE_FILE` set for instrumented Clippy/build commands too, because procedural macros
can also emit profile files.
