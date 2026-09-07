# Matterweave core

Independent Rust world data, retained M1 reference algorithms and v0.2 chunk streaming. No Android, windowing,
graphics backend or physics types appear in the public interface.

The authoritative world uses sparse, deterministically ordered 16³ chunks with
one byte per material (zero is air, 1–255 are solid). Euclidean chunk addressing
works across negative coordinates. Empty chunks are reclaimed. Changed edits
advance the world revision; unchanged edits do not. `World::mesh()` returns an
owned derived result tagged with that revision. A caller replacing an entire
world must also invalidate pending jobs; revisions identify edits within a world,
not globally unique world instances. There are no asynchronous jobs in this crate.
An exhausted u64 revision makes the world read-only (`set` returns false) so an
imported snapshot cannot cause overflow, a panic or reuse of old revision numbers.

`World::generate(seed)` creates an original version-1, integer-generated terrain
fixture spanning x/z −32…31, with groves, an arch and mineral outcrops. This is a
small integration scene, not a world-size or visual-fidelity requirement.

`raycast` uses normalized double-precision grid DDA internally, returning distance
in world units. It clamps finite ranges to 4096 units and rejects zero/nonfinite
directions, invalid origins and negative/nonfinite ranges. Cells are half-open;
starting inside a solid returns distance zero and a zero normal. Exact simultaneous
crossings advance all tied axes, avoiding edge-only hits, and use the lowest
crossed axis for the normal. Queries cannot overflow the i32 coordinate space.

Persistence writes version 2 and accepts version 1. Non-streamed worlds store
complete snapshots. Streamed worlds store up to 512 authoritative chunk overrides
(including explicit air), the active window center and an extension-generator
version. Resident procedural terrain is derived and regenerated on load. A 12 MiB
JSON cap bounds the entire save including optional opaque app metadata. Unknown
versions, duplicate chunks, invalid coordinates, malformed lengths and empty
serialized nonempty chunks are rejected. `save_with_attachment` atomically stores
world and app state together; `save` preserves any loaded attachment. The app owns
validation of the attachment schema.

Saving uses a unique same-directory temporary file, buffered write, file sync and
atomic rename. Failure before rename preserves the previous destination. Directory
sync after rename is best-effort; power-loss durability is not guaranteed. Use the
application's private writable data directory.

`enable_streaming()` opts in; `World::new` stays empty and `generate` retains the
original reference fixture. `stream_around` synchronously publishes a radius-three
chunk window (at most 7×7×3 chunks), clipped to x/z [-256,256), y [-16,32). Only
resident cells inside that domain accept edits. The original x/z [-32,32) square
remains entirely snapshot-authoritative: missing legacy chunks mean air, including
fully removed chunks. Saved chunks outside the domain remain in the override
archive and are preserved on save, although they cannot be visited in this slice.
The surrounding integer terrain meets the original descending island edge and
gradually returns to rolling hills. This is bounded streaming, not virtualized LOD.

Each first edit of a generated chunk copies its authoritative 4096-byte payload
into a bounded override archive. Later edits update that copy. Reaching 512 stored
overrides rejects edits requiring another slot, preserving already saved data;
`stored_overrides` reports consumption. Eviction never discards an override.
`allocated_bytes` includes resident and override voxel payloads, excluding map and
allocator overhead, temporary serialization buffers and meshes. `chunks` and
`solid_voxels` describe resident content only.

`mesh()` retains the whole-world exposed-face reference. `mesh_chunk(key)` adapts
an 18³ padded material volume to block-mesh greedy quads, preserving material IDs,
cross-chunk occlusion and CCW winding. All materials are opaque. `chunk_revision`
changes only for a local edit, shared-face neighbor edit or relevant load/unload.
Absent chunks return None. A derived result can be published only if its key and
revision still match the current world; replacing a World invalidates all caches.
No asynchronous jobs exist, so collision/render consumers must synchronize before
the next simulation step. f32 vertices lose voxel precision at extreme legacy
coordinates; the streamed domain stays close to origin.

## Component assessment (2026-09-07)

[block-mesh 0.2.0](https://docs.rs/block-mesh/0.2.0/block_mesh/) is adopted for greedy
quad extraction after inspecting its padded-volume contract, merge-value support,
face orientation and output index code. The adapter uses material identity as the
merge key. The existing reference remains for equivalence tests. This reuses the
mature algorithm without introducing its coordinate or storage types into the
public engine API. Host geometry reduction is evidence for adoption of this
bounded mesh optimization, not a mobile frame-time comparison or final M2
ray/mesh/hybrid selection.

The equal-scene regression (seed 8712, three explicit edits in
`tests/streaming.rs`) measured 27,744 reference triangles and 6,660 greedy
triangles: 76.0% fewer, with identical unit faces and materials. Reproduce with
`cargo test -p matterweave-core greedy_surfaces -- --nocapture`. This is a geometry
count, not a frame-time or GPU benchmark.

Standard-library `BTreeMap` and fixed arrays supply storage; the project-specific
code owns coordinate/material/revision semantics. A general voxel engine would
also impose rendering/world abstractions unnecessary for this reference module.
Existing general-purpose components are adopted rather than reimplemented:

| Component | Exact version | Source | Upstream license | Role |
| --- | --- | --- | --- | --- |
| block-mesh | 0.2.0 | [crate](https://crates.io/crates/block-mesh/0.2.0) | MIT OR Apache-2.0 | Reused greedy surface extraction on padded chunk data. |
| bytemuck | 1.23.2 | [crate](https://crates.io/crates/bytemuck/1.23.2) | Zlib OR Apache-2.0 OR MIT | Checked plain-data derives for GPU vertex upload. |
| serde | 1.0.219 | [crate](https://crates.io/crates/serde/1.0.219) | MIT OR Apache-2.0 | Serialization contract and validated typed decoding. |
| serde_json | 1.0.140 | [crate](https://crates.io/crates/serde_json/1.0.140) | MIT OR Apache-2.0 | Inspectable versioned persistence; replaceable behind the save API. |

Direct versions are exact in the crate manifest; the workspace lock pins transitives.
These are dependency licenses, not a project-license choice.

## Verification

From the repository root: `cargo test -p matterweave-core`.
The v0.2 suite passes 22 integration tests on Rust 1.96.0 Linux x86_64.

v0.2 adds exact greedy/reference unit-face and material equivalence, boundary
revision locality, streaming bounds/reversals, legacy removed-chunk migration and
evicted edits reloaded together with opaque app state.

Tests exercise negative/chunk boundaries, sparse-reference edits, seeded terrain,
ray ranges/invalid inputs/edge crossings, surface winding and seam occlusion,
revision invalidation, snapshot round trips, malformed versions/chunks, save size
limits and temporary-file cleanup after a failed rename. Native integration and
device behavior are verified by the application, not by host core tests.

The original v0.1 run on 2026-09-07 used Rust 1.96.0 on Linux x86_64 and passed 14 integration tests,
`cargo clippy -p matterweave-core --all-targets --locked -- -D warnings`, and scoped
`rustfmt --check`. An instrumented host run measured 94.40% production-source line
coverage, 94.12% region coverage and 95% function coverage. Branch coverage was not
instrumented. These are correctness-test coverage quantities, not performance or
Android validation. The imported-revision regression was first observed to panic,
then passed after exhausted edits were made non-mutating.

Coverage can be reproduced with Rust's `llvm-tools` component and a **fresh**
target directory, keeping previous instrumented binaries/profiles out of the run:

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

The recorded run used `/mnt/bench/matterweave-dev/core-coverage`; substitute an
appropriate writable data path on other machines and the matching host triple in
`LLVM_BIN`. Keep `LLVM_PROFILE_FILE` set for instrumented Clippy/build commands too,
because procedural macros can also emit profile files.
