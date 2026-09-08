# opus-p01-a2 — P01 instrumentation correction (attempt 2)

Task `P01-instrumentation`, attempt 2, owner `opus-p01-a2`, base
`755620472e8457d8e55760165e759b6d8013c6db` plus attempt-1 work in the same
worktree. Bounded measurement-correctness correction of the review findings in
`muse-p01-behavior.md`, `muse-p01-lifetime.md` and `gemini-p01-schema.md`. No
commits, no branch changes, no device access, no engine optimization. The
attempt-1 log is unmodified.

## Actions taken

1. Read all three review logs and the board entry for attempt 2, then re-read the
   affected code: renderer upload/retain/draw fence-wait sites, present handling,
   explorer draw clock ordering, `save()`, and the metrics schema.
2. Wrote regression tests first, ran them RED, then implemented:
   - **Fence-wait scope (Gemini F1).** The real blocking wait happens in
     `retain_chunks`/`upload_chunk`/`upload`/`upload_dynamic` before the draw's
     wait. No wait was moved or removed (lifetime invariant preserved). Added a
     small `WaitTally` in the renderer that times each upload/retain wait at its
     own call site and accumulates count and total, reset explicitly by
     `Renderer::begin_frame_diagnostics()` which the explorer calls **before**
     mesh sync. `draw()` clears only its own fields, so pre-draw waits cannot be
     wiped or leaked. Schema: `fence_wait_wall_ms` became
     `render_fence_wait_wall_ms`, plus new `mesh_sync_fence_wait_wall_ms` and
     `mesh_sync_fence_waits`. The document states the exact scope: these cover
     every `Commands::wait` call site, and driver-internal waits inside other
     Vulkan calls remain unmeasured; neither column may be reported as "the" wait.
   - **CPU/wall containment (Muse F1/F3).** `now = Instant::now()` is taken
     first, then `metrics::CpuBusySpan::begin(capturing)`; the span finishes
     immediately before `self.cpu_ms = now.elapsed()`, with no renderer or
     diagnostics queries in between. Busy is now contained in wall. `dt` still
     measures frame start to frame start. `CpuBusySpan::begin(false)` reads no
     clock, so the non-capture path is unchanged.
   - **Shadow counter (Gemini F2 / Muse F4).** `shadow_caster_meshes` is written
     through `metrics::shadow_casters_for_attempt`, which requires this attempt's
     `submitted_frame_id`. A retry that recorded no command buffer no longer
     inherits the previous pass's count; a submitted attempt whose presentation
     retried keeps its real shadow work.
   - **Present accounting (Muse F2).** `vkQueuePresentKHR` returning
     `ERROR_OUT_OF_DATE_KHR` previously set `recreate` and fell through to
     `Presented`. It now returns `FrameResult::Retry`, so `presented_count` no
     longer advances for a frame that was not presented. The submission and its
     identity are untouched, as are semaphore/fence handling and the recreate
     flag. Classification lives in a pure `classify_present` helper tested for
     success, suboptimal success, out-of-date and fatal errors without a driver.
   - **Save accounting (Muse F1).** `saves_performed` is replaced by
     `save_attempts` and `save_failures`; wall time covers failed attempts too.
     A `SaveAccounting` value is consumed per row by `take_save_accounting`.
   - **Schema regression (Gemini F3).** A test fills all 37 columns with
     distinguishable values and asserts the whole name-to-cell mapping, so a swap
     between adjacent fields in `FrameRow::write` now fails. Added coverage for
     `gpu_prev_shadows` as `""`/`0`/`1` and for retries with and without a
     submission id.
   - **Physics breakdown (Gemini F4).** The initial-body test asserts the whole
     `BodyActivity` value (`total 2, active 2, sleeping 0, not_simulated 0`).
3. Rewrote the affected parts of `docs/performance/measurement-v2.md`: fence-wait
   scope section, retry-before/after-submission rules, present semantics, CPU/wall
   ordering, save columns, the below-world case in `not_simulated`, the
   normal emptiness of `completed_*`/`gpu_prev_*`, and the corrected "no
   capture-only clock readings" wording. Version stays 2 and the document is
   labelled as the in-place final v2 contract.

## RED then GREEN evidence

No competing build: `ps -eo pid,etime,cmd | grep -E "cargo|rustc|gradle|java"`
returned nothing before every run. All commands used
`CARGO_TARGET_DIR=/mnt/bench/matterweave-dev/performance/target`.

RED (tests written before the production changes):

```text
$ cargo test -p matterweave-render --lib
error[E0432]: unresolved imports `super::classify_present`, `super::PresentOutcome`, `super::WaitTally`

$ cargo test -p matterweave-explorer --lib
error[E0560]: struct `metrics::FrameRow` has no field named `mesh_sync_fence_wait_wall_ms`
error[E0560]: struct `metrics::FrameRow` has no field named `render_fence_wait_wall_ms`
error[E0560]: struct `metrics::FrameRow` has no field named `mesh_sync_fence_waits`
error[E0560]: struct `metrics::FrameRow` has no field named `save_attempts`
error[E0560]: struct `metrics::FrameRow` has no field named `save_failures`
error[E0425]: cannot find function `shadow_casters_for_attempt` in this scope
```

The strengthened physics assertion changes no production behaviour, so it could
not fail through a missing symbol. Its value was proven by mutation instead:
temporarily classifying awake bodies as sleeping made it fail, and the mutation
was reverted:

```text
assertion `left == right` failed: restored bodies start awake and simulated; ...
  left: BodyActivity { total: 2, active: 0, sleeping: 2, not_simulated: 0 }
 right: BodyActivity { total: 2, active: 2, sleeping: 0, not_simulated: 0 }
test result: FAILED. 0 passed; 1 failed
```

GREEN, after implementation:

```text
$ cargo test -p matterweave-render --lib
test tests::disabled_wait_tally_takes_no_clock_and_reports_nothing ... ok
test tests::enabled_wait_tally_distinguishes_zero_one_and_many_waits ... ok
test tests::resetting_a_tally_drops_waits_from_the_previous_attempt ... ok
test tests::present_results_map_to_honest_frame_outcomes ... ok
test result: ok. 18 passed; 0 failed

$ cargo test -p matterweave-explorer --lib
test metrics::tests::every_column_carries_its_own_value_in_the_declared_order ... ok
test metrics::tests::shadow_flag_distinguishes_unknown_from_disabled_and_enabled ... ok
test metrics::tests::shadow_work_belongs_only_to_attempts_that_submitted ... ok
test metrics::tests::retry_attempts_stay_visible_before_and_after_a_submission ... ok
test metrics::tests::cpu_busy_span_reads_the_clock_only_when_enabled ... ok
test tests::save_accounting_separates_attempts_from_failures_and_resets_per_row ... ok
test result: ok. 29 passed; 0 failed

$ cargo test --workspace
all suites ok: 29 (explorer) + 18 (render) + 13 (render lighting/frustum incl.) +
17 (physics behavior) + 3 (physics counters) + 8 + 7 + 1 + doc-tests; 0 failed

$ cargo clippy --workspace --all-targets     # no warnings outside vendored winit
$ cargo fmt --all -- --check                 # clean
$ python3 tools/check_docs.py
PASS: 61 Markdown files, 223 local links, 15 ADRs and 20 requirements.

$ python3 (column/doc cross-check)
columns: 37; missing from docs: []
```

Status: **pass** — RED/GREEN cycle, focused and workspace tests, clippy, fmt,
doc check, schema/doc cross-check. **Not run** — Android build, host graphics
smoke, device capture, instrumentation overhead and repeatability (lead-owned;
phone unavailable). No performance or mobile claim is made here.

## Issues and friction

- **Self-inflicted data loss.** To prove the strengthened physics assertion could
  fail, I mutated the production classifier and reverted with
  `git checkout crates/matterweave-physics/src/lib.rs`, which reset the file to
  the base commit and destroyed the whole attempt-1 physics change. I restored it
  byte-for-byte from the frozen candidate
  (`/mnt/bench/matterweave-dev/performance/run-01/p01-candidate-01`), verified
  `sha256 614a04a80231407567b3709433bfe96e78a5df6ddff6815d04285996548e1c0e`
  against `sha256.json`, then re-applied the attempt-2 edits. Net effect on the
  delivered tree: none, confirmed by the manifest hash and by re-running the
  tests. The lesson is that `git checkout <path>` is not an undo for uncommitted
  work; the frozen artifact is what made this recoverable.
- The draw's `DrawDiagnostics` reset had to become partial. A whole-struct reset
  at the top of `draw()` would erase the mesh-sync waits recorded earlier in the
  same frame, while no reset at all would let a previous frame's waits leak. The
  split is: `begin_frame_diagnostics()` resets everything for the frame,
  `draw()` clears only the fields it owns.
- `presented_count` semantics and the out-of-date present sit on the boundary
  between instrumentation and behaviour. Returning `Retry` changes the value the
  smoke gate counts; it is nevertheless the truthful accounting the contract
  requires, and the submission lifetime is untouched.
- Testing the renderer headless is impossible here, so both new renderer
  behaviours were factored into pure helpers (`WaitTally`, `classify_present`)
  that carry the decision logic. This proves the mapping and the accumulation
  rules; it does not prove the call-site placement, which stays a lead check.

## Decisions and rationale

- **Two named wait columns, never a claimed total.** Merging pre-draw and draw
  waits into one number would hide which call site blocks. Naming them by stage
  and documenting the exact call sites keeps the claim inside what is measured.
- **`Some(0)` versus empty for waits.** A capture-active frame that made no
  upload wait reports `0` waits and `0.0000` ms — a measured zero. A frame with
  diagnostics disabled reports empty. Zero and unknown stay distinct.
- **Out-of-date present is a retry, not a presentation.** The submission is real
  and keeps its identity; only the presentation request failed. This removes the
  false "presented" row rather than weakening the documented definition, which is
  what the measurement contract asks for.
- **Attempts, not completions, for saves.** Failed saves consume time, so the
  wall time keeps them and the counters separate failure from completion instead
  of dropping either signal.
- **Shadow work gated on submission, not on `result`.** Gating on `Presented`
  would have discarded genuine shadow work from a submitted frame whose
  presentation retried.
- **v2 finalised in place.** The schema has never been shipped or accepted, so
  correcting its columns under version 2 is honest and avoids implying an
  upgrade path from a schema no consumer ever read. Once a capture is accepted,
  the normal version-increment rule applies.

## Solutions applied

- `WaitTally` with explicit enabled state, `reset`/`timed_begin`/`record`, giving
  zero/one/many-wait, reset and disabled coverage without any GPU data.
- `classify_present` as a pure mapping from the present call result plus the
  suboptimal flag to `Presented { recreate } | OutOfDate | Failed`.
- `metrics::CpuBusySpan` encapsulating clock ordering and the disabled case.
- `metrics::shadow_casters_for_attempt` making the submission gate explicit and
  testable outside the renderer.
- `SaveAccounting` plus `take_save_accounting`, testable without a renderer,
  covering success, failure and per-row reset.
- A full-row schema test comparing a name-to-value vector, which catches column
  reordering that width checks miss.

## Insights

- The pre-draw upload waits meant the previous `fence_wait_wall_ms` was
  structurally near zero on GPU-bound frames while the same time appeared inside
  `dynamic_upload_wall_ms`. An instrumentation slice can be internally consistent
  and still mislabel where a frame blocks; only tracing the wait to its call site
  fixes that.
- Every definition corrected in this attempt was a boundary question — what
  counts as presented, as a completed save, as the same span, as this attempt's
  shadow work. Those are exactly the places where a capture invites false joins.
- Retry is not a synonym for "did nothing". Modelling retries as a single class
  led to two separate defects (stale shadow counts and inflated presentation
  counts); the honest model is retry-before-submission versus
  retry-after-submission.
- Frozen artifacts are the recovery mechanism for worker mistakes. Without the
  candidate manifest, the destroyed physics file would have had to be rewritten
  from memory, which is exactly how silent divergence enters a measurement stack.

## Handoff

Changed in this attempt (all owned paths):

- `apps/explorer/src/metrics.rs` — renamed/added columns, `CpuBusySpan`,
  `shadow_casters_for_attempt`, full-contract and shadow-flag tests
- `apps/explorer/src/lib.rs` — clock ordering, `begin_frame_diagnostics` before
  mesh sync, shadow gating, `SaveAccounting`, save-accounting test
- `crates/matterweave-render/src/lib.rs` — `WaitTally` at the four upload/retain
  wait sites, per-frame reset, `classify_present` returning `Retry` on
  out-of-date present, renderer unit tests
- `crates/matterweave-physics/src/lib.rs` — full `BodyActivity` assertion and the
  corrected `not_simulated` doc comment
- `docs/performance/measurement-v2.md` — corrected contract
- `docs/performance/logs/opus-p01-a2.md` — this log

Unchanged: gameplay, save/session formats, world/physics behaviour, renderer
lifetimes/synchronisation, opt-in `profile-frames.txt` semantics, and the
attempt-1 log. The only behavioural change is the truthful present-result
accounting (out-of-date present now reports `Retry`).

Remaining lead checks: independent correction reviews, Android build, host
graphics smoke, a real device capture inspected against the corrected schema
(especially `mesh_sync_fence_waits` on frames that upload nothing, and a
resize/rotation sequence exercising the out-of-date present path), phone overhead
and same-build repeatability, and validator-lane integration. Deferred counters
are unchanged: streaming jobs, collision publication, edit events, queue depths,
allocations/retained capacity, and the reason a body is `not_simulated`.
