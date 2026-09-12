# d2-3-streaming — reversal, eviction, cancellation and progress under pressure (2026-09-12)

Slice: D2.3 (streaming half of D2, M3 Phase A). Base `b0a500d` on branch
`phase-a/d2-3-streaming`. Run directory
`/mnt/bench/matterweave-dev/coarse-terrain/streaming-publication-resume/` with raw
logs in `logs/`. Target dir for every Cargo invocation:
`/mnt/bench/matterweave-dev/coarse-terrain/target-streaming-publication`,
`CARGO_BUILD_JOBS=2`.

**Verdict — core safety correct; sustained-edit streaming liveness OPEN.** The reported
*observation* is real and was reproduced exactly (an edit inside every preparation
interval publishes nothing). That refusal is the documented invariant doing its job, not
a stall, and no edit is lost, so the safety core needs no change and the production
publication and rejection logic is unchanged. It does **not** follow that application
progress under sustained edits is solved: with an edit inside *every* preparation
interval, no window can be published by any implementation that both never publishes a
stale window and replaces the authoritative world with the prepared snapshot, so a
caller that keeps editing never advances past its initial window. The discriminating
test proves *conditional* progress — one edit-free preparation interval publishes — and
that is weaker than sustained-edit liveness. Liveness with an edit in every preparation
interval stays **OPEN** until a caller-side bounded fallback (or another edit-preserving
approach, below) is implemented and device-verified. No core rejection logic was relaxed
to force a false fix.

## The candidate and the mechanism

`AsyncWorld::poll_stream` takes a prepared window and publishes it only when
`result.generation == queue.generation && Some(result.center) == requested_center &&
result.seed == world.seed() && result.source_revision == world.revision() &&
world.is_streaming()`. Publication is `*world = result.world`, so publishing a snapshot
taken before an edit would revert that edit. A second, upstream guard already exists:
`run_job` stages a completed preparation only while `queue.stream_requested` still
matches the job identity, so a fresh request supersedes an in-flight preparation
cleanly and the stale completion is discarded under the lock.

The candidate's mechanism (`source_revision == world.revision()` fails whenever the
revision advanced during preparation) is therefore correct as a mechanism and was never
in doubt. The open question was whether the *condition can clear* while edits continue,
or whether every completion is doomed once edits are frequent enough.

## The discriminating test

`async_world::tests::stale_stream_refusal_defers_until_one_preparation_interval_is_edit_free`
(in-crate, `AsyncWorld::manual()` workerless controller, no sleeps, no wall-clock
timing, no dependence on worker speed). One deterministic frame is: request → the
simulated worker takes the job → the caller edits → the preparation completes → the
caller polls. The three phases:

| Phase | Edit stream | Observed |
| --- | --- | --- |
| 1 (8 intervals) | one edit inside every preparation interval | 8/8 preparations completed and were staged (`stream_results == MAX_STREAM_RESULTS` before the poll), 8/8 refused by `poll_stream`, window centre still the origin, 0 published |
| 2 | edits continue at one edit per two intervals | the edited interval is refused; the following edit-free interval completes at the current revision and **publishes** (supported window asserted) |
| 3 (control) | edits stopped | the next preparation publishes immediately |

After the run the same edits and the two publications that happened are replayed through
the synchronous path; the resulting worlds are compared field by field — revision,
resident window, chunk set, per-chunk revisions and all 4096 materials per resident
chunk. They are identical, so every refusal discarded only work that a fresh request
replaced, and no edit was lost.

**Discriminator (one sentence): the refusal is per preparation interval, not per edit
stream — a preparation whose interval contained no edit publishes, so one edit-free
interval suffices to clear the condition, which a genuine stall could not do.** This
establishes *conditional* progress; it does not establish liveness when every preparation
interval carries an edit.

### Evidence (raw logs under `.../streaming-publication-resume/logs/`)

```
$ cargo test -p matterweave-core --locked --lib async_world::tests:: -- --test-threads=1
test async_world::tests::stale_stream_refusal_defers_until_one_preparation_interval_is_edit_free ... ok
test result: ok. 9 passed; 0 failed; 0 ignored; 22 filtered out     [new-tests.log]
```

Mutation probes (each applied to `async_world.rs`, run, then reverted; RED evidence that
the test discriminates):

| Probe | Mutation | Result |
| --- | --- | --- |
| A | force `acceptable = false` in `poll_stream` (a genuine stall) | FAILED: `the edit-free interval did not publish` — a stall cannot publish in phase 2/3 |
| C | drop `result.source_revision == world.revision()` from `acceptable` | FAILED: `frame 0: a stale window was published` — the test also catches the opposite failure, not just "nothing published" |
| B2 | drop the `queue.generation != job.generation` admission check for meshes in `run_job` | FAILED: `a cancelled mesh stayed staged` (after the test was strengthened, see Friction) |

## Why refusal is correct, and the open liveness residual

Publication is a whole-world replacement of the authoritative world by a snapshot. With
an edit inside every preparation interval, every completed snapshot predates an edit, so
publishing it would revert that edit; refusing is the invariant doing its job, and the
safety core is correct as it stands. That correctness is exactly why the *liveness*
condition cannot be removed in-core: the mechanism cannot be rate-independent and
edit-preserving at the same time without changing what publication means (merging
post-snapshot edits), which is explicitly excluded by decision on this slice. So
sustained-edit liveness remains **OPEN** at the application level and must be resolved
outside the rejection path, not by weakening it.

Bounded safe fallbacks, proposed, not implemented here (both need integration
ownership):

1. **Caller-side one-frame synchronous fallback.** The explorer already has the
   synchronous branch (`apps/explorer/src/lib.rs`, the `else if self.world.stream_around(..)`
   arm used when `preparation.available()` is false). Treating repeated stream refusals
   like an unavailable worker for one frame re-windows synchronously, which also
   invalidates the pending async result by advancing the revision — so the fallback
   cannot publish anything stale. Bounded to one frame per occurrence.
2. **Observability.** `AsyncStats::discarded` merges stream refusals with mesh drops, so a
   caller cannot count *stream* refusals today. A `stream_discarded` counter (or a refusal
   reason on `poll_stream`) would let an integration detect the condition without
   heuristics. Public API addition: proposed, not applied.

## Reversal

- `edit_then_reverse_leaves_the_window_equal_to_an_unedited_world`
  (`tests/async_world.rs`, real worker): a generated-air cell ("create then remove", its
  chunk is generated empty) and a generated-solid cell ("edit then restore"), then the
  window round-trips through the worker to a far centre and back. The result equals a
  never-edited world in chunk set, chunk count, solid voxels and every material; the
  resident set is exactly the declared span and every materialized chunk is inside it
  (no orphan, no duplicate). The reversal does leave an *explicit-empty* override entry
  (`stats().stored_overrides` +1), which is the documented authoritative record for
  "this chunk is empty by edit"; it is bounded by the streaming module's `MAX_OVERRIDES`
  (512) and is deliberately not part of the content comparison.
- The starvation test above also ends in a bit-identical world after 9 edits and 2
  publications, which covers reversal under the queue (edit/refuse/re-request) rather
  than only at the synchronous API.
- Pre-existing synchronous coverage: `tests/streaming.rs::streaming_reversals_invalidate_border_meshes_without_invalidating_interior`
  (chunk-revision invalidation across a window reversal).

## Eviction

- `eviction_and_rerequest_keep_every_published_window_complete` (in-crate, deterministic):
  while the replacement is in flight and while it is staged, the origin window stays
  complete (`stream_contains_position(origin, 1.0)` plus the exact 147-key span); one
  publication then moves residency to the far window in a single step, with the evicted
  window gone rather than half-evicted; re-entry restores the edit made before eviction
  and, compared field by field against a synchronous replay, is bit-identical.
- `eviction_then_rerequest_restores_the_window_and_supports_it_again`
  (`tests/async_world.rs`, real worker): the same round trip through the worker; every
  sampled interior position of each published window reports support, and the restored
  window equals a world that never left in materials, chunk set and counts.
- Residency is asserted as the declared `7x7x3 = 147` chunk span
  (`2 * STREAM_RADIUS_CHUNKS + 1` squared times three vertical layers) in both new
  tests, replacing the implicit `< 147` checks used elsewhere.
- **Support contract scope.** Core proves the *publication* support contract:
  `stream_contains_position` is true for every position inside a published window, and a
  window is never partially evicted. Whether a *physics body* is physically supported
  (nothing to stand on removed under it) is an Android/graphics integration gate and is
  **NOT RUN** here, per the lead's clarification.

## Cancellation

- `reset_mid_flight_states_the_baseline_and_cancels_every_pending_unit` (in-crate,
  deterministic). Stated baseline immediately after `reset()` with one unit executing:
  `queued_meshes`, `queued_streams`, `mesh_results`, `mesh_result_bytes`,
  `stream_results` all 0, `inflight == MAX_INFLIGHT_JOBS` (the executing unit is one
  bounded, unpreemptable job), `generation` incremented, and the authoritative world's
  revision, window centre and resident set untouched. Completing the cancelled units
  afterwards publishes nothing, occupies no staging slot, and does not unfreeze a
  request made after the cancel (the stale mesh's `active` removal is generation-guarded,
  so the re-requested key stays registered and still delivers a matching mesh).
- `reset_cancels_queued_and_in_flight_work_and_returns_to_the_baseline`
  (`tests/async_world.rs`, real worker): the same baseline at the public API, then after
  `settle` nothing cancelled is delivered (`poll_stream`/`poll_mesh` both empty),
  revision and residency are unchanged, `available()` is still true, and re-requested
  chunks deliver meshes whose revision matches the world.
- Supersession (never publishes): pre-existing
  `latest_request_matching_inflight_drops_a_superseded_pending_window` (in-crate),
  `results_from_an_old_generation_or_a_changed_world_are_rejected` and
  `returning_home_cancels_a_prepared_destination_and_repeated_requests_coalesce`
  (integration); the new bounds test adds that two different window requests never both
  queue and the pending slot holds only the latest request.

## Bounds and memory pressure

Declared bounds (`crates/matterweave-core/src/async_world.rs`, module doc
"Declared bounds"). `AsyncStats::within_bounds` restates only the queue, staging and
mesh-byte bounds it can observe; the residency window and override cap are enforced by
the streaming module (`stream_around`, `World::set`) and are not visible in `AsyncStats`:

| Held data | Bound | Declared as | Enforced at |
| --- | --- | --- | --- |
| pending mesh jobs | 32 halos (18³ each, 182 KiB) | `MAX_QUEUED_MESH_JOBS` | `request_mesh` |
| completed meshes | 8 results | `MAX_MESH_RESULTS` | `run_job` |
| completed mesh bytes | 8 MiB, one oversized result admitted into an empty queue | `MAX_MESH_RESULT_BYTES` | `run_job`, `within_bounds` |
| pending window preparations | 1 (latest request replaces it) | `MAX_QUEUED_STREAM_JOBS` (new) | `request_stream` |
| staged windows | 1 | `MAX_STREAM_RESULTS` (new) | `run_job`, `poll_stream` |
| executing units | 1 | `MAX_INFLIGHT_JOBS` (new) | `take_job` |
| resident chunks | 7x7x3 = 147 keys | `STREAM_RADIUS_CHUNKS` window (streaming module) | `stream_around` |
| stored edit overrides | 512 | streaming module `MAX_OVERRIDES` | `World::set` |

- `mesh_result_byte_bound_refuses_a_completion_below_the_count_cap` (in-crate,
  deterministic, production meshes): eight checkerboard chunks (2048 isolated solids
  each, 12288 unmergeable quads) are meshed through the real
  `request_mesh`/`take_job`/`run_job` path. Measured vertex+index capacities are
  2,752,512 B per chunk (not an assumed size), so three results retain 8,257,536 B of the
  8,388,608 B bound and the fourth is refused at a result count of 3 — well below
  `MAX_MESH_RESULTS == 8`. The refusal leaves the retained count and bytes unchanged and
  frees the key for re-request. Removing the `mesh_result_bytes + bytes >
  MAX_MESH_RESULT_BYTES` clause in `run_job` admits all eight and the test fails, so the
  real byte guard is what the test discriminates.
- `queue_and_staging_limits_refuse_cleanly_at_the_declared_bounds` (in-crate,
  deterministic): the mesh queue fills to exactly `MAX_QUEUED_MESH_JOBS` and the next
  request is refused, and refused again (no partial admission); staging fills to exactly
  `MAX_MESH_RESULTS`, the next completion is dropped instead of buffered and frees its
  key for re-request; the documented single-oversized-result exception is asserted with
  an `AsyncStats` literal (`mesh_results == 1` with bytes over the bound, which
  `within_bounds` admits by design). `within_bounds()` is asserted after every step.
- Worker corroboration: the pre-existing flood test
  `saturated_requests_stay_bounded_and_still_deliver_the_latest_meshes` now asserts
  `stats().within_bounds()` through the flood and at saturation.

**Snapshot accounting (separate from the queue counts).** `AsyncStats` reports at most
one queued window (`queued_streams`), one executing window (`inflight`) and one staged
window (`stream_results`), so a complete count of retained prepared snapshots is three,
plus one transient clone while `request_stream` replaces a pending job (the module doc's
"Snapshot cost"), four briefly. The bounds test's `MAX_QUEUED_STREAM_JOBS +
MAX_STREAM_RESULTS == 2` covers queue plus staging only; it does not include the
executing snapshot. `within_bounds()` checks those three singleton counts; it bounds
neither process memory nor the caller's resident `World` (147 chunks plus up to
`MAX_OVERRIDES == 512` stored overrides), whose payloads are shared with the snapshots.
Queue/staging bounds and process/global-memory bounds are separate claims.

## Source change

`poll_stream`, `request_stream`, `request_mesh`, `reset`, `run_job`, `take_job` and every
queue structure are unchanged; no behaviour change to publication or rejection. Added to
`async_world.rs` only: three `pub const` singleton bounds, one public method
`AsyncStats::within_bounds`, the module-doc "Declared bounds" section, the tests, and —
in the review follow-up below — a real-mesh byte-bound test plus corrected scope wording
on `within_bounds` and this log.
`crates/matterweave-core/lib.rs` was not touched (outside owned paths): the new constants
are reachable through `within_bounds` but not yet by path, so a one-line
`pub use async_world::{MAX_INFLIGHT_JOBS, MAX_QUEUED_STREAM_JOBS, MAX_STREAM_RESULTS};`
would expose them if integration wants to assert the bounds directly — proposed
integration seam, not applied.

Excluded paths were not touched: `streaming.rs` and `coarse.rs` are unmodified (the 147
residency bound is read through the public `STREAM_RADIUS_CHUNKS` and the override cap is
documented, not changed), as are `apps/explorer/**`, the other crates, `Cargo.lock`,
`docs/STATUS.md`, `docs/ROADMAP.md`, `docs/adr/**`, `docs/performance/board.json` and CI.

## Verification

```
$ cargo test -p matterweave-core --locked                 # lib 31, api 1, async_world 10, replay 10, streaming 8, world 13, doc 0
$ cargo fmt --all -- --check                              # clean (exit 0)
$ cargo clippy -p matterweave-core --all-targets --locked -- -D warnings   # clean (exit 0)
$ cargo test --workspace --locked                         # 45 suites: 564 passed, 0 failed, 3 ignored
$ cargo clippy --workspace --all-targets --locked -- -D warnings          # clean (exit 0)
```

Workspace test: 9 m 09 s from a cold target for every crate but core; the 3 ignored
suites are pre-existing (explorer/pacing), none in `matterweave-core`. Workspace clippy:
no warning from any workspace member; the vendored `winit` dependency emits one
pre-existing `direct cast of function item into an integer` warning, which `-D warnings`
does not deny because `winit` is a dependency, not a workspace member. Raw logs:
`logs/workspace-test.log`, `logs/workspace-fmt-clippy.log`.

Baseline at `b0a500d` before any change: `cargo test -p matterweave-core --locked` passed
(lib 27, api 1, async_world 7, replay 10, streaming 8, world 13), 47 s cold target
(`logs/baseline-core-test.log`).

### Definition of done

| Criterion | Verification | Owner | Result |
| --- | --- | --- | --- |
| Continuous-edit candidate characterized with one discriminating test and an explicit verdict | `stale_stream_refusal_defers_until_one_preparation_interval_is_edit_free`; safety core correct, sustained-edit liveness **OPEN** in this log | worker | **PASS (characterized) / OPEN (liveness)** |
| Test distinguishes a stall from correct refusal without sleeps/wall-clock, with a control showing publication when the blocking condition ends | manual workerless controller; phases 1/2/3; discards without sleeps; mutation probes A and C | worker | **PASS** |
| Core safety needs no source change; liveness deferred to the caller, not forced in-core | `poll_stream`/`run_job`/`reset` unchanged; only bounds constants, `within_bounds`, docs and tests added | worker | **PASS** |
| Edit-then-reverse leaves the resident window consistent with the authoritative source | `edit_then_reverse_leaves_the_window_equal_to_an_unedited_world`; bit-identical replay in the starvation test | worker | **PASS** |
| Eviction then re-request restores equivalent content and never removes support from a body | `eviction_and_rerequest_keep_every_published_window_complete`; `eviction_then_rerequest_restores_the_window_and_supports_it_again` (publication support contract; physical body support is the integration gate) | worker | **PASS** (core contract) / **NOT RUN** (physics) |
| Cancelled or superseded request never publishes; queue and residency return to a stated baseline | `reset_mid_flight_states_the_baseline_and_cancels_every_pending_unit`; `reset_cancels_queued_and_in_flight_work_and_returns_to_the_baseline`; pre-existing supersession tests | worker | **PASS** |
| Queue, staging and residency limits declared as named constants or documented values | module-doc table; `MAX_QUEUED_STREAM_JOBS`, `MAX_STREAM_RESULTS`, `MAX_INFLIGHT_JOBS` added; `AsyncStats::within_bounds` restates the queue/staging/mesh-byte bounds only (residency and the override cap are streaming-module bounds) | worker | **PASS** |
| At the limits, requests are refused cleanly with no unbounded growth | `queue_and_staging_limits_refuse_cleanly_at_the_declared_bounds`; real-mesh `mesh_result_byte_bound_refuses_a_completion_below_the_count_cap`; flood test with `within_bounds()` | worker | **PASS** |
| `cargo test --workspace --locked` | 45 suites: 564 passed, 0 failed, 3 pre-existing ignores; `logs/workspace-test.log` | worker | **PASS** |
| `cargo fmt --all -- --check` and `cargo clippy --workspace --all-targets --locked -- -D warnings` | both exit 0; `logs/workspace-fmt-clippy.log` | worker | **PASS** |
| Graphics-integrated Android streaming run | device run | integration session | **NOT RUN** |

## Issues & friction

- Reverting a mutation probe with `git checkout -- crates/matterweave-core/src/async_world.rs`
  also discarded uncommitted test additions to that same file. Caught by grepping the
  restored file, re-applied, and subsequent probes restored from a saved copy instead
  (`logs/async_world.tested-backup.rs`). Process error: a VCS revert is not a
  mutation-scoped revert when the file carries uncommitted work.
- The first cancellation mutation probe *passed*: dropping the mesh generation check in
  `run_job` still did not publish, because `poll_mesh` has its own generation guard and
  the test only observed the public delivery path. The test now also asserts the staging
  counts directly after the cancelled unit completes (0 results, 0 bytes), which is part
  of the stated baseline, and the mutation fails (`logs/mutation-B2-cancelled-mesh-publishes.log`).
- The strict world comparison in the in-crate tests (revision, residency, chunk revisions,
  materials) was expected to need relaxing against a synchronous replay and did not: the
  async publication path and the synchronous path produce bit-identical worlds for the
  same operation order. The integration helper deliberately compares content only, since
  the integration round trips exercise different window orders.
- No timing-based test was added for the candidate: a worker-based "edit every frame"
  reproduction would depend on preparation duration versus frame rate and could not
  discriminate a stall from refusal. The strict form is recorded here as a rate condition
  instead.

## Review follow-up (repair)

Lead-adjudicated findings corrected on 2026-09-12 in this file and
`crates/matterweave-core/src/async_world.rs`. No production rejection logic changed.

1. **Verdict wording.** The unqualified "DISMISSED as a defect" is replaced above by
   "core safety correct; sustained-edit liveness OPEN". The deterministic observation
   and the strict stale-snapshot rejection are retained; the edit-free interval is stated
   as *conditional* progress, not sustained-edit liveness.
2. **Bound scope.** `within_bounds` only checks `AsyncStats` queue/staging/mesh-byte
   counts; it does not check the 147-chunk resident window or the 512-override cap
   (enforced by the streaming module). The module doc and this log now say so, and the
   snapshot accounting above distinguishes the three retained snapshots plus one
   transient replacement clone from the queue/staging counts. The single-oversized-mesh
   exception is preserved, not treated as a defect.
3. **Real byte-admission test.**
   `mesh_result_byte_bound_refuses_a_completion_below_the_count_cap` replaces the
   byte-rule claim that relied only on synthetic `AsyncStats` literals: production
   checkerboard meshes saturate `MAX_MESH_RESULT_BYTES` at 3 results (each 2,752,512 B),
   the next completion is discarded with unchanged retained count and bytes, and its key
   is requestable again. Mutation probe (real byte clause removed) fails the test, so it
   discriminates the actual guard.

Repair DoD:

| Criterion | Verification | Result |
| --- | --- | --- |
| Conditional progress and outstanding app liveness stated honestly | source/log review; verdict section above | **PASS** |
| Queue-bound scope and simultaneous snapshot counts accurate | `within_bounds` doc and snapshot-accounting paragraph above | **PASS** |
| Real byte saturation/refusal path tested below count cap | `mesh_result_byte_bound_refuses_a_completion_below_the_count_cap`; 3/8 results, 8,257,536 B, refused `[3, 0, 0]`, key re-requestable | **PASS** |
| Existing stale refusal, reversal, cancellation contracts unchanged | `cargo test -p matterweave-core --locked` (all suites); scoped `clippy -D warnings`; `cargo fmt --all -- --check` | **PASS** |
| Source scoped and committed | `git diff` touches only `async_world.rs` + this log | **PASS** |
| Independent acceptance and Android/core/app integration | lead handoff | **NOT RUN** |

Repair commands (target dir
`/home/someotherguy/Documents/ChatGPT/Matterweave-recovery-20260912/streaming-target`,
raw logs in the sibling `streaming-repair-logs/`):

```
$ cargo test -p matterweave-core --locked          # lib 32, api 1, async_world 10, replay 10, streaming 8, world 13; 0 failed   [core-all.log]
$ cargo clippy -p matterweave-core --all-targets --locked -- -D warnings   # clean (exit 0)   [core-clippy.log]
$ cargo fmt --all -- --check                       # clean (exit 0)   [fmt.log]
$ cargo test -p matterweave-core --locked --lib mesh_result_byte_bound_refuses_a_completion_below_the_count_cap -- --nocapture
# byte saturation: 3 results retained, 8257536 bytes; refused [3, 0, 0] of 2752512 bytes (bound 8388608); sizes [2752512; 8]   [byte-test.log]
$ <remove the byte clause in run_job, rerun the same test>
# FAILED: no byte refusal occurred; workload too small: [2752512; 8]   [byte-probe-no-byte-guard.log]
```

Mutation-probe process: the pre-probe source was copied to
`streaming-repair-logs/async_world.pre-byte-probe.rs` (sha256
`302a6fbdf74968e0f843489ce16514c07a861791f3dd47bb1ed6902065af3b60`) and restored from that
copy afterwards; no VCS reset/checkout was used.

## Raw logs

`/mnt/bench/matterweave-dev/coarse-terrain/streaming-publication-resume/logs/`:
`baseline-core-test.log`, `unit-step1.log`, `integration-step1.log`,
`core-verify-2.log`, `new-tests.log`, `mutation-A-stall.log`,
`mutation-B-cancel-generation.log`, `mutation-B2-cancelled-mesh-publishes.log`,
`mutation-C-publish-stale.log`, `mutation-D-window-not-applied.log`,
`workspace-test.log`, `workspace-fmt-clippy.log`, and `async_world.tested-backup.rs`
(the tested copy used to restore the file after each probe).

Repair follow-up raw logs (sibling of the repository, never `/mnt/bench`):
`/home/someotherguy/Documents/ChatGPT/Matterweave-recovery-20260912/streaming-repair-logs/`:
`core-all.log`, `core-lib.log`, `core-clippy.log`, `fmt.log`, `byte-test.log`,
`byte-probe-no-byte-guard.log`, `async_world.pre-byte-probe.rs`.

## Next actions

1. Integration (lead): the sustained-edit liveness condition is **OPEN**. Implement and
   device-verify the caller-side one-frame synchronous fallback (or another
   edit-preserving approach) before claiming progress under continuous edits; decide
   whether the `stream_discarded` observability counter is wanted. Android
   graphics-integrated streaming run remains unrun.
2. Optional: `crates/matterweave-core/examples/stream_stress.rs` inlines its own bound
   comparisons and could call `AsyncStats::within_bounds()`; not in this slice's owned
   paths.
