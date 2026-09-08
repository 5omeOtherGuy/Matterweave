# Asynchronous detail-collision preparation

Bounded off-thread preparation of load/edit-time detail collision, usable by any
engine client. This closes the *preparation scheduling* capability: a simulation
owner can request collision preparation for an authoritative
`matterweave_detail::DetailScene`, keep stepping physics, then publish the
finished shapes on its own thread. It does **not** close streaming publication,
frame-budget gating or device verification; see "Remaining lead-owned work".

## Where the code lives

- Controller: `crates/matterweave-physics/src/async_detail_collision.rs`
  (`AsyncDetailCollision`, `AsyncDetailStats`).
- Preparation contract it drives (owned separately, unchanged here):
  `PreparedDetailCollision::build` / `.stats()` / `.is_current()` and
  `Physics::publish_detail_scene` in
  `crates/matterweave-physics/src/detail_collision.rs`.
- Behavioural tests: `crates/matterweave-physics/tests/async_detail_collision.rs`.
- Host usage example (no UI/demo content):
  `crates/matterweave-physics/examples/async_detail_collision.rs`.

## Design and reuse

The controller mirrors the bounded-queue / generation / snapshot-revalidation
shape of `matterweave_core::AsyncWorld` (one worker, a `Condvar` wake, a poll
that revalidates against the live authoritative state) instead of introducing a
generic job framework. Only `std` threading, `Mutex`, `Condvar` and an owned
`JoinHandle` are used.

Differences are inherent to the workload, not new machinery:

- Job identity is the scene's opaque `SceneVersion` (an `Arc`-identity token),
  not a chunk key. Unrelated scenes never compare equal even when their local
  revision counters match; `fork_source` preserves the identity until mutation;
  derived mesh work does not change it. The controller relies on this contract
  rather than inventing a fingerprint or generation hash.
- There is a single pending / running / completed slot rather than a chunk
  queue, because the whole detail scene is replaced as one unit.

### Invariants

- At most one **pending** source snapshot (latest request wins; a superseded
  pending snapshot is counted as discarded).
- At most one job **running** on the worker.
- At most one **completed** result buffered for `poll`.
- `request` never builds shapes on the caller: it takes only a bounded
  `DetailScene::fork_source` copy and wakes the worker.
- A source whose identity already matches the pending, running or buffered work
  is deduplicated and refused.
- `poll(&current)` is nonblocking. It consumes and returns a result **only** if
  the result's identity matches `current` and it belongs to the live generation.
  A result built for a different source is left buffered (polling with the wrong
  scene is harmless); a reset-cancelled result is dropped. Therefore an error
  produced for an already-replaced source can never reject the current scene.
- Old live collision stays intact until `publish_detail_scene` succeeds;
  `publish` revalidates identity again, so an edit between `poll` and `publish`
  is rejected and live walls are retained.
- `reset` cancels the pending snapshot and buffered result and advances the
  generation, invalidating the in-flight job's result.
- `available()` is false after worker startup failure or unexpected exit; when
  the worker cannot start, every request is refused and the caller keeps the
  synchronous `replace_detail_scene` path.
- The worker never blocks on a full result slot (it replaces the single slot and
  records the drop), so shutdown is observed after at most one bounded job and
  the `Drop` join cannot deadlock. No thread is ever detached.

### Memory and work limits (documented bounds, not measured RSS)

- **Source snapshots:** at most three `fork_source` copies are live at once
  (pending, running, and one transient clone while replacing a pending job).
  Each is bounded by the detail crate's `MAX_SCENE_SOURCE_BYTES` = 32 MiB
  authoritative payload. `fork_source` eagerly clones detail payloads today;
  World's separate copy-on-write storage does not change this cost.
- **Built result:** at most one buffered `PreparedDetailCollision` plus one
  currently building/prepared result on the worker. Each shape cost is bounded by the existing caps enforced by `build`:
  `MAX_DETAIL_BOXES` (262,144 merged cuboids, an estimated 26-34 MiB of resident
  shape memory per the box-cost note in
  `docs/performance/p03/collision/opus-execution.md`) and
  `MAX_DETAIL_COLLIDERS` (16,384 static colliders).
- **Work per job:** one `build` call over one immutable scene snapshot; the
  worker checks shutdown/generation between jobs, so cancellation latency is at
  most one bounded preparation.

These are correctness/upper bounds only. No throughput or latency figure is
claimed; none was measured on the target device.

## Verification

Commands (exclusive target `performance/engine-02/async-collision-target`,
`CARGO_BUILD_JOBS=1`, host = x86-64 Linux dev machine, not a phone):

- `cargo test -p matterweave-physics --test async_detail_collision`
  -> `9 passed; 0 failed` (finished in 0.05 s).
- `cargo test -p matterweave-physics`
  -> detail_collision `19 passed`, dynamic_cache `8 passed`, counter/lib tests
  pass, showcase_traversal `2 ignored` (pre-existing campaign gate), doc-tests
  `0`. No regressions.
- `cargo run -p matterweave-physics --example async_detail_collision`
  -> requested, published `static_colliders=1 merged_boxes=4`, character grounded
  on the published detail floor = `true`.
- `cargo fmt -p matterweave-physics -- --check` -> clean.
- `cargo clippy -p matterweave-physics --tests --examples -- -D warnings` -> clean.

Tests avoid timing-dependent queue assumptions: worker progress is awaited by
spinning on the public `stats`/`poll` surface with an eventual bounded deadline
(10 s), and bound checks (`queued <= 1`, `inflight <= 1`, `results <= 1`) are
sampled as invariants that must always hold, never as a specific expected count
at a specific instant.

Behaviours covered:

- Worker preparation then actual published floor contact (character grounded at
  the slab top).
- Edit between request/poll and publication: stale rejection preserves live
  walls and published stats.
- Unrelated scene replacement with matching revision counters is rejected.
- Rapid supersession keeps bounds and yields the latest source; superseded
  snapshots are discarded.
- Reset/in-flight cancellation, then a fresh request succeeds.
- Invalid preparation (over the collider budget) propagates as an error for the
  current scene.
- A stale error does not reject a different current scene.
- Bounded queue counts and clean shutdown (drop with work in flight joins the
  worker without hang or panic).

## Engineering log

### Actions

- Read `AGENTS.md`, the physics crate (`lib.rs`, `detail_collision.rs`), the
  prepared-collision tests, `DetailScene`/`SceneVersion`/`fork_source` in the
  detail crate, and `AsyncWorld` for reuse assessment.
- Added `AsyncDetailCollision` + `AsyncDetailStats` mirroring `AsyncWorld`'s
  bounded-worker pattern, exported from `matterweave-physics`.
- TDD: RED checkpoint (stub + 9 behavioural tests, all failing), then GREEN
  checkpoint (real controller, all passing).
- Added a host usage example and this document.

### Issues

- `PreparedDetailCollision` is not `Debug` (it holds parry `SharedShape`), so
  `Result::expect_err` could not be used in a test.
- First GREEN attempt consumed the buffered result on any mismatched `poll`,
  which discarded a result still valid for its own scene (the unrelated-scene
  test failed on the follow-up poll).

### Decisions

- Job identity = `SceneVersion` (opaque `Arc` token), reusing the accepted
  contract rather than inventing a fingerprint/hash.
- Single pending/running/completed slots, not a queue: the detail scene is
  replaced whole.
- `poll` peeks and only consumes a result whose identity matches the passed
  scene; a foreign-source result is left buffered, a reset-cancelled one is
  dropped. This keeps `poll` idempotent for the wrong scene and guarantees a
  stale error cannot reject the current scene.
- `fork_source` clone is taken under the queue lock (only the worker could
  otherwise touch the queue, and the clone frees no invariant), matching
  `AsyncWorld`'s stream path.

### Solutions

- Test avoids `Debug` by matching on the `Result` instead of `expect_err`.
- Reworked `poll` to peek-then-take-on-match, discarding only generation
  (reset) mismatches.

### Insights

- The publication-side `is_current` recheck plus the controller-side identity
  match give two independent guards; an edit between poll and publish is caught
  by `publish` even if `poll` accepted, so live walls are never replaced by
  stale shapes.
- Because `SceneVersion` is `Arc` identity, "unrelated scene, matching counters"
  and "edited same scene" are the same rejection mechanism, which keeps the
  controller free of any content comparison.

## Remaining lead-owned work

- Integration into the app/simulation frame loop and residency/streaming
  publication cadence (lead owns integration).
- Frame-budget gating of `publish_detail_scene` (insertion/removal and body
  waking still run on the simulation thread; this capability schedules only the
  build).
- Native Android device verification of the worker under real memory/thermal
  conditions; no device numbers are claimed here.
- DetailScene snapshot payload sharing remains separate future work; the World
  copy-on-write change does not affect `DetailScene::fork_source`.

## Lead correction after worker follow-up

The follow-up Opus4.8/high run timed out after committing deterministic RED queue
regressions (`b6882fc` on its branch), before implementing the correction. Lead
integrated those tests, added buffered-A/running-B/request-A coverage and observed
four failures. The corrected controller uses an opaque Arc allocation for reset
validity and tracks the latest requested source independently. Generation remains
a saturating diagnostic counter. Reset accepts a fresh same-source request;
returning to a running/buffered source removes superseded pending work, and a
superseded in-flight completion cannot replace the newly desired buffered result.
Counters saturate; cancellation counts in-flight discard on completion once.

Lead verification: six deterministic queue tests, nine asynchronous integration
tests,19 detail collision tests, eight dynamic cache tests and three existing
physics unit tests pass. The two pre-existing full-showcase gates remain ignored
in this scoped run. Logs: `engine-02/async-collision-lead-{red,green}.log`.
World and DetailScene have different storage types; the earlier claim that World
COW would reduce `fork_source` cost was incorrect and is corrected above. Buffers
returned to callers are caller-owned and require their own retention budget.
