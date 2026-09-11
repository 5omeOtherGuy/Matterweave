# Asynchronous detail-collision preparation

Bounded off-thread preparation of load/edit-time detail collision, usable by any
engine client. This closes the *preparation scheduling* capability: a simulation
owner can request collision preparation for an authoritative
`matterweave_detail::DetailScene`, keep stepping physics, then publish the
finished shapes on its own thread. It does **not** close streaming publication,
frame-budget gating or device verification (see Limits and open work).

## What problem this solves

Building the collision shapes for a detail scene runs over the whole scene and
must not block the simulation thread. A simulation owner needs to request that
build for an authoritative `DetailScene`, keep stepping physics, and publish the
finished shapes on its own thread without racing an edit.

## How it works

### Where the code lives

- Controller: `crates/matterweave-physics/src/async_detail_collision.rs`
  (`AsyncDetailCollision`, `AsyncDetailStats`).
- Preparation contract it drives (owned separately, unchanged here):
  `PreparedDetailCollision::build` / `.stats()` / `.is_current()` and
  `Physics::publish_detail_scene` in
  `crates/matterweave-physics/src/detail_collision.rs`.
- Behavioural tests: `crates/matterweave-physics/tests/async_detail_collision.rs`.
- Host usage example (no UI/demo content):
  `crates/matterweave-physics/examples/async_detail_collision.rs`.

### Design and reuse

The controller mirrors the bounded-queue / generation / snapshot-revalidation
shape of `matterweave_core::AsyncWorld` (one worker, a `Condvar` wake, a poll
that revalidates against the live authoritative state) instead of introducing a
generic job framework. Only `std` threading, `Mutex`, `Condvar` and an owned
`JoinHandle` are used. The `fork_source` clone is taken under the queue lock
(only the worker could otherwise touch the queue, and the clone frees no
invariant), matching `AsyncWorld`'s stream path.

Differences are inherent to the workload, not new machinery:

- Job identity is the scene's opaque `SceneVersion` (an `Arc`-identity token),
  not a chunk key. Unrelated scenes never compare equal even when their local
  revision counters match; `fork_source` preserves the identity until mutation;
  derived mesh work does not change it. The controller relies on this contract
  rather than inventing a fingerprint or generation hash.
- There is a single pending / running / completed slot rather than a chunk
  queue, because the whole detail scene is replaced as one unit.
- Reset validity uses an opaque `Arc` allocation, and the controller tracks the
  latest requested source independently. The generation remains a saturating
  diagnostic counter; reset accepts a fresh same-source request, returning to a
  running/buffered source removes superseded pending work, and a superseded
  in-flight completion cannot replace the newly desired buffered result.
  Counters saturate, and cancellation counts an in-flight discard on completion
  once.

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
  `poll` peeks and takes only on a match, so it stays idempotent for the wrong
  scene.
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
  authoritative payload. `DetailVolume` wraps `World`, so the core copy-on-write
  change shares chunk payloads through `fork_source`; map metadata and reference
  counts are still copied.
- **Built result:** at most one buffered `PreparedDetailCollision` plus one
  currently building/prepared result on the worker. Each shape cost is bounded
  by the existing caps enforced by `build`:
  `MAX_DETAIL_BOXES` (262,144 merged cuboids, an estimated 26-34 MiB of resident
  shape memory per the box-cost note in
  `docs/performance/p03/collision/opus-execution.md`) and
  `MAX_DETAIL_COLLIDERS` (16,384 static colliders).
- **Work per job:** one `build` call over one immutable scene snapshot; the
  worker checks shutdown/generation between jobs, so cancellation latency is at
  most one bounded preparation.

These are correctness/upper bounds only. No throughput or latency figure is
claimed; none was measured on the target device.

## What was verified

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

Lead verification: six deterministic queue tests, nine asynchronous integration
tests, 19 detail collision tests, eight dynamic cache tests and three existing
physics unit tests pass. The two pre-existing full-showcase gates remain ignored
in this scoped run. Logs:
`engine-02/async-collision-lead-{red,green}.log`.

Tests avoid timing-dependent queue assumptions: worker progress is awaited by
spinning on the public `stats`/`poll` surface with an eventual bounded deadline
(10 s), and bound checks (`queued <= 1`, `inflight <= 1`, `results <= 1`) are
sampled as invariants that must always hold, never as a specific expected count
at a specific instant. `PreparedDetailCollision` is not `Debug` (it holds parry
`SharedShape`), so a test matches on the `Result` rather than using
`expect_err`.

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

### Negative results and corrections

- Deterministic RED queue regressions committed as `b6882fc` were integrated
  with added buffered-A/running-B/request-A coverage; four failures were observed
  before the corrected controller landed.
- An earlier `poll` consumed the buffered result on any mismatched call, which
  discarded a result still valid for its own scene. The peek-then-take-on-match
  rule replaced it.
- An earlier claim that detail storage types prevented reuse of core chunk COW
  was wrong. `DetailVolume` wraps `World`; its clone inherits core chunk sharing,
  and `DetailScene::fork_source` clones those volumes. The full copy path is
  verified rather than introducing another storage implementation.
- Generating a result for an already-replaced source cannot reject the current
  scene: the publication-side `is_current` recheck and the controller-side
  identity match are two independent guards, so an edit between `poll` and
  `publish` is caught by `publish` even if `poll` accepted. Because
  `SceneVersion` is `Arc` identity, "unrelated scene, matching counters" and
  "edited same scene" share one rejection mechanism, keeping the controller free
  of content comparison.

### Native Android collision gate

At `9cf26a1`, the example is a failing-on-error gate for asynchronous floor
creation, character contact, authoritative floor removal and falling through
removed support. Both host execution and direct OnePlus 13 Android 16 execution
pass. The ARM64 native executable is built with NDK 28.2.13676358/API 28, Cargo
dev profile (`opt2`/`debug0`) and 16 KiB ELF alignment; SHA256
`4a8d2ab507957aca1628f445991c1fa41adfc6938ef67bd31f060378b650a73f`. It was
pushed to an owned `/data/local/tmp` path, run, and removed. This is a native
Android engine test; it is not APK frame-loop integration or a performance
claim. Raw binary, report and build manifest: `engine-02/phone-collision` under
the artifact root. The installed indirect-check APK does not yet contain this
newer controller.

## Limits and open work

- Integration into the app/simulation frame loop and residency/streaming
  publication cadence (lead owns integration).
- Frame-budget gating of `publish_detail_scene`: insertion/removal and body
  waking still run on the simulation thread; this capability schedules only the
  build.
- Native Android device verification of the worker under real memory/thermal
  conditions; no device numbers are claimed here.
- Measure actual frame-loop snapshot metadata and publication cost; chunk
  payloads are already shared through `DetailVolume`'s internal `World`.
- Buffers returned to callers are caller-owned and require their own retention
  budget.
