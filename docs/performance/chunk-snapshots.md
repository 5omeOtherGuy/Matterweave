# Shared chunk snapshots — 2026-09-08

World clones and streaming overrides now share immutable 4096-byte voxel payloads
through standard-library `Arc`. The authoritative `World::set` detaches only its
changed chunk. Before editing, it removes its own redundant override reference;
otherwise every successive edit would copy the chunk again. Rejected/unchanged
edits retain all references. Existing serialization bytes and revision semantics
remain unchanged. No dependency or format change is needed.

## Actions and verification

GLM5.3 Flash/high produced a partial implementation in its isolated worktree and
timed out at900 seconds. Lead retained its Arc storage/generator/deserializer
changes, corrected repeated override copying and replaced faulty allocation tests.
The partial patch and raw failed tests remain under the engine-02 artifact root.

Lead RED `9bd18c1`: three runtime failures demonstrate clones and overrides deeply
copied payloads; deletion behavior already passed. GREEN `94e51eb`: all four new
checks and the full core suite pass. The tests compare live allocation identities
directly, then verify negative-coordinate edits, unchanged chunks, repeated edits,
no-op/rejected edits, revision exhaustion, deletion and eviction/re-entry behavior.
Existing asynchronous cancellation, mesh, boundary and persistence tests also pass.

```sh
export CARGO_TARGET_DIR=/mnt/bench/matterweave-dev/performance/completion-01/lead-target
export CARGO_BUILD_JOBS=1
cargo test -p matterweave-core --lib snapshot_tests
cargo test -p matterweave-core
cargo clippy -p matterweave-core --all-targets -- -D warnings
```

Logs: `/mnt/bench/matterweave-dev/performance/engine-02/cow-root-{red,green,clippy}.log`.
The initial test setup mistakenly used a nonexistent constructor; it was corrected
before recording the valid runtime RED checkpoint.

## Issues, decisions and limits

The worker's proposed unique-byte statistic could not prove sharing between
worlds and added per-frame set allocation/scans. It was discarded. Existing
`allocated_bytes` describes logical resident-plus-override bytes, including repeated
references, and is now documented accordingly. It is not physical process memory.

Cloning still allocates map metadata and increments references. New streamed
terrain still allocates chunks; snapshots that independently edit every chunk can
reach the previous per-snapshot payload bound. No Android timing, RSS, energy or
thermal improvement is claimed from these host tests. Android lifecycle/build
checks belong to the integrating engine-systems batch.
