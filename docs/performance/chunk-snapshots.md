# Shared chunk snapshots — 2026-09-08

## What problem this solves

World clones and streaming overrides copied every 4096-byte voxel payload, so a
snapshot or a streamed window paid a full deep copy even when almost nothing
changed.

## How it works

World clones and streaming overrides share immutable 4096-byte voxel payloads
through standard-library `Arc`. The change covers chunk storage, the generator
and the deserializer. The authoritative `World::set` detaches only its changed
chunk. Before editing, it removes its own redundant override reference;
otherwise every successive edit would copy the chunk again. Rejected and
unchanged edits retain all references. Serialization bytes, revision semantics
and the dependency set are unchanged.

## What was verified

Host only.

Lead RED `9bd18c1`: three runtime failures show that clones and overrides deeply
copied payloads; deletion behavior already passed. GREEN `94e51eb`: all four new
checks and the full core suite pass.

The tests compare live payload allocation identities directly, then cover
negative-coordinate edits, unchanged chunks, repeated edits, no-op/rejected
edits, revision exhaustion, deletion and eviction/re-entry behavior. Existing
asynchronous cancellation, mesh, boundary and persistence tests also pass.

```sh
export CARGO_TARGET_DIR=/mnt/bench/matterweave-dev/performance/completion-01/lead-target
export CARGO_BUILD_JOBS=1
cargo test -p matterweave-core --lib snapshot_tests
cargo test -p matterweave-core
cargo clippy -p matterweave-core --all-targets -- -D warnings
```

The initial test setup mistakenly used a nonexistent constructor; it was
corrected before the valid runtime RED checkpoint was recorded. The retained
partial patch and its raw failing tests remain under the `engine-02` artifact
root. Logs:
`/mnt/bench/matterweave-dev/performance/engine-02/cow-root-{red,green,clippy}.log`.

## Limits and open work

- A proposed unique-byte statistic could not prove sharing between worlds and
  added per-frame set allocation and scans; it was discarded.
- `allocated_bytes` describes logical resident-plus-override bytes, including
  repeated references, and is documented accordingly. It is not physical process
  memory.
- Cloning still allocates map metadata and increments references. New streamed
  terrain still allocates chunks; snapshots that independently edit every chunk
  can reach the previous per-snapshot payload bound.
- No Android timing, RSS, energy or thermal improvement is claimed from these
  host tests. Android lifecycle/build checks belong to the integrating
  engine-systems batch.
