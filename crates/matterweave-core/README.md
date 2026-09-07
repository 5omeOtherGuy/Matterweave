# Matterweave core

Independent Rust world data and M1 reference algorithms. No Android, windowing,
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

Persistence stores complete material snapshots, seed, generator/format versions
and revision. Version 1 import/export is bounded to 512 chunks and 12 MiB of JSON;
unknown versions, duplicate chunks, invalid coordinates, malformed lengths and
empty serialized chunks are rejected. Loading creates a separate validated world.
Saving uses a unique same-directory temporary file, buffered write, file sync and
atomic rename. Failure before rename preserves the previous destination. Directory
sync after rename is best-effort; power-loss durability is not guaranteed. Use the
application's private writable data directory. This is not a compressed streaming
format, and no migration from future format/generator versions is implemented.

`allocated_bytes` counts allocated voxel payload only, excluding BTreeMap nodes,
allocator overhead, temporary serialization buffers and meshes. Mesh extraction
is synchronous whole-world exposed-face generation with stable order, CCW outward
triangles and cross-chunk neighbor queries. All materials are opaque. There is no
greedy meshing, LOD, lighting cache or collision representation. f32 vertex positions
lose single-voxel precision at large coordinates; the small fixture is near origin.

## Component assessment (2026-09-07)

[block-mesh 0.2.0](https://docs.rs/block-mesh/0.2.0/block_mesh/) supplies existing
visible-face and greedy-quad algorithms, with an 18³ padded input for 16³ chunks.
It is a credible optimized meshing candidate. This M1 implementation keeps a tiny
direct neighbor-query reference for correctness comparisons without a padded-copy
adapter. This is a bounded reference implementation, not a claim that custom
meshing outperforms the dependency. Compare/adopt the existing greedy mesher in M2
using equal scenes and total extraction/upload/memory costs. No benchmark numbers
from the library are presented as Matterweave measurements.

Standard-library `BTreeMap` and fixed arrays supply storage; the project-specific
code owns coordinate/material/revision semantics. A general voxel engine would
also impose rendering/world abstractions unnecessary for this reference module.
Existing general-purpose components are adopted rather than reimplemented:

| Component | Exact version | Source | Upstream license | Role |
| --- | --- | --- | --- | --- |
| bytemuck | 1.23.2 | [crate](https://crates.io/crates/bytemuck/1.23.2) | Zlib OR Apache-2.0 OR MIT | Checked plain-data derives for GPU vertex upload. |
| serde | 1.0.219 | [crate](https://crates.io/crates/serde/1.0.219) | MIT OR Apache-2.0 | Serialization contract and validated typed decoding. |
| serde_json | 1.0.140 | [crate](https://crates.io/crates/serde_json/1.0.140) | MIT OR Apache-2.0 | Inspectable versioned persistence; replaceable behind the save API. |

Direct versions are exact in the crate manifest; the workspace lock pins transitives.
These are dependency licenses, not a project-license choice.

## Verification

From the repository root: `cargo test -p matterweave-core`.
Tests exercise negative/chunk boundaries, sparse-reference edits, seeded terrain,
ray ranges/invalid inputs/edge crossings, surface winding and seam occlusion,
revision invalidation, snapshot round trips, malformed versions/chunks, save size
limits and temporary-file cleanup after a failed rename. Native integration and
device behavior are verified by the application, not by host core tests.

On 2026-09-07, Rust 1.96.0 on Linux x86_64 passed 14 integration tests,
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
