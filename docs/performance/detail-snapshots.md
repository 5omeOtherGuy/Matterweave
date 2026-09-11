# Detail source snapshots — validation

## What problem this solves

A lead/physics-worker claim that detail storage types prevented reuse of core
chunk copy-on-write was **wrong**. The claim needed to be corrected and the
inherited sharing verified against the detail crate's own behavior.

## How it works

`DetailVolume` wraps `matterweave_core::World`, so the core COW mechanism
(described in [shared chunk snapshots](chunk-snapshots.md)) already benefits
detail volume clones, scene source forks and private instance edits. This
validates an already integrated engine benefit; it is not a new COW feature, an
optimization implementation or a RED/GREEN cycle. No production changes, new
dependencies or public test diagnostics were needed. The lead owns correction of
the central physics document and central status.

Some existing comments still describe copying prototype payloads (notably
`scene.rs:215–222` and the comment preceding `edit_instance`'s clone); they must
not be read as evidence of deep chunk-array copies. Their wording predates the
inherited sharing implementation.

Reuse evidence (line numbers at the base checkout `bf5ee0e`):

- `crates/matterweave-detail/src/lib.rs:424–430`: derived `Clone` includes
  `world: World`; `DetailVolume::set` delegates to `World::set` at line 507.
- `crates/matterweave-detail/src/scene.rs:224–235`: `fork_source` clones
  prototypes/instances and the source token, leaving derived caches empty.
- `crates/matterweave-detail/src/scene.rs:280–288`: `edit_instance` clones the
  shared prototype, changes its identity and edits the clone before insertion.
- `crates/matterweave-core/src/lib.rs:26–41,116`: `Chunk` and `World` derive
  `Clone`, chunk payloads are `Arc<[u8; CHUNK_VOLUME]>`, and mutation uses
  standard-library `Arc::make_mut`.
- `crates/matterweave-core/src/snapshot_tests.rs:8–29`: existing private tests
  compare live payload allocation identities across clones and prove that only
  the edited chunk detaches. Other tests cover repeated edits, streaming,
  rejected/no-op edits, deletion and re-entry. No duplicate pointer test added.

## What was verified

Existing tests already cover local/replayable instance edits, source token
changes, no-ops/rejections and single-scene cache invalidation. One integration
scenario was added in `crates/matterweave-detail/tests/source_snapshot.rs`,
after reading `tests/detail.rs`, `tests/instance_edits.rs` and
`tests/source_version.rs`.

The added scenario connects these operations across retained generations: clone
a two-chunk volume, cache all three LODs, fork a scene, privately edit one of two
instances, retain that candidate, then edit its original prototype. It checks
unchanged serialized source snapshots and source versions in older generations,
unchanged live draws, rebuilt original-prototype LOD geometry/revisions, and
continued reuse of private and original-live mesh caches. Mesh content is
checked, not merely cache counters. The test uses public behavior, not metadata
sizes or allocation assumptions.

Base checkout: `bf5ee0e`, including core COW integration `94e51eb`. Host only.

```sh
export CARGO_TARGET_DIR=/mnt/bench/matterweave-dev/performance/completion-01/lead-target
export CARGO_BUILD_JOBS=1
cargo test -p matterweave-detail
cargo test -p matterweave-core --lib snapshot_tests
rustfmt --edition 2021 --check crates/matterweave-detail/tests/source_snapshot.rs
python3 tools/check_docs.py
```

- Full detail suite: **101 passed**, zero failed/ignored; includes the new test.
  Raw log: `/mnt/bench/matterweave-dev/performance/detail-snapshot-tests.log`.
- Existing core allocation-contract tests: **4 passed**, 3 unrelated tests filtered.
- Formatting and documentation checks: passed.

No descendants or benchmarks were launched. Builds used the existing shared lead
target with one Cargo job; no worktree-local target was generated. The shared
integration target is preserved, not removed as temporary-worker output.

## Limits and open work

- Clones still copy map/identity/instance metadata and increment references.
  Source byte budgets remain logical accounting, not unique physical memory.
- Mutation may copy touched chunks; derived meshes and coarse volumes still
  require their own work.
- These host tests do not measure Android timing, RSS, thermal or energy
  improvements. No device run was performed and no new performance claim is made.
- Lead integration should carry this correction into the central engineering log
  and physics documentation; no second detail COW implementation is warranted.
