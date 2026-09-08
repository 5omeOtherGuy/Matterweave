# opus-p01-a1 — P01 instrumentation slice 1

Task `P01-instrumentation`, attempt 1, owner `opus-p01-a1`, base
`755620472e8457d8e55760165e759b6d8013c6db`, workspace
`/mnt/bench/matterweave-dev/worktrees/performance-p00`. No commits, no branch
changes, no device access. Scope: the first instrumentation slice only.

## Actions taken

1. Verified ownership in `docs/performance/board.json` before editing: the nine
   paths listed for `P01-instrumentation` match the paths changed here. Read
   `docs/PERFORMANCE_TASKS.md` (P01) and the measurement contract in
   `docs/PERFORMANCE_PLAN.md`.
2. Read the existing instrumentation: `apps/explorer/src/metrics.rs` (v1 capture),
   `Explorer::draw`/`sync_render_meshes`/`save` in `apps/explorer/src/lib.rs`,
   `crates/matterweave-render/src/timing.rs` (GPU timestamp queries), the fence
   wait/acquire/submit/present boundaries in `crates/matterweave-render/src/lib.rs`
   and `step`/`step_objects`/body policy in `crates/matterweave-physics/src/lib.rs`.
3. Wrote failing tests first (RED, below), then implemented:
   - **Typed CSV schema v2** (`metrics.rs`): `FrameRow` with `u64`/`u32`
     identities and counters plus `Option<f64>` durations, `DrawOutcome`
     (`presented`/`retry`/`out_of_memory`), `COLUMNS` as the single column-order
     source, a `#matterweave-frame-capture schema_version=2` header line, and
     `frame-profile-v2-*.csv` output so v1 captures are preserved untouched.
     `record` takes `&FrameRow`; the old `[Option<f64>; 10]` float-array API and
     its float-converted GPU frame id are gone.
   - **Identities**: unique monotonically increasing `draw_attempt_id` for every
     attempt that reaches the renderer, `renderer_epoch` incremented on renderer
     creation/recreation, `presented_count` (successful present API calls),
     `submitted_gpu_frame_id` from a new renderer submission counter, and
     `completed_gpu_frame_id` + `completed_gpu_renderer_epoch`.
     `GpuCompletionTracker` deduplicates completions on the `(epoch, id)` pair.
   - **CPU busy time**: `metrics::thread_cpu_time` — a narrow `unsafe` wrapper
     around `clock_gettime(CLOCK_THREAD_CPUTIME_ID)` with status and `timespec`
     range checks, compiled only for Linux/Android; every other target returns
     `None`. `cpu_busy_ms` returns `None` for missing or backwards readings.
     Read only while a capture is active.
   - **Renderer diagnostics** (`matterweave-render`): opt-in `DrawDiagnostics`
     (`submitted_frame_id`, `fence_wait_ms`, `acquire_ms`, `present_ms`) measured
     at the true call boundaries, `set_diagnostics_enabled`/`draw_diagnostics`.
     Disabled by default: an uninstrumented run takes no extra clock readings.
   - **Physics counters**: `Physics::body_activity() -> BodyActivity`
     (`total`/`active`/`sleeping`/`not_simulated`), excluding the character and
     reporting residency-disabled distant bodies separately. Existing fixed-step
     return values are now recorded instead of discarded.
   - **Explorer wiring**: distinct physics, dynamic-mesh build, dynamic-upload and
     render wall times; retained stream elapsed labelled as elapsed; chunk upload,
     dynamic build/upload, saves and shadow-caster counters.
4. Wrote `docs/performance/measurement-v2.md` (units, missingness, identities and
   reset rules, overlap, explicitly not measured) and this log. Ran
   `python3 tools/check_docs.py`.

## RED then GREEN evidence

Baseline build state checked first: no `cargo`/`rustc`/`gradle` process was
running (`ps -eo pid,etime,cmd | grep -E "cargo|rustc|gradle"` → no matches), and
`/mnt/bench/matterweave-dev/performance/target/debug/deps` already held 489
built dependency artifacts, so no competing build was started. All commands used
`CARGO_TARGET_DIR=/mnt/bench/matterweave-dev/performance/target`.

RED (tests written before production code):

```text
$ cargo test -p matterweave-physics --lib
error[E0599]: no method named `body_activity` found for struct `Physics`
error: could not compile `matterweave-physics` (lib test) due to 6 previous errors

$ cargo test -p matterweave-explorer --lib metrics
error[E0061]: this method takes 2 arguments but 1 argument was supplied  (FrameLog::record)
error: could not compile `matterweave-explorer` (lib test) due to 41 previous errors
```

GREEN:

```text
$ cargo test -p matterweave-physics --lib
test counter_tests::body_activity_counts_voxel_bodies_and_excludes_the_character ... ok
test counter_tests::settled_bodies_become_sleeping_and_stepping_reports_fixed_steps ... ok
test counter_tests::bodies_outside_resident_columns_are_reported_as_not_simulated ... ok
test result: ok. 3 passed; 0 failed

$ cargo test -p matterweave-explorer --lib
test metrics::tests::header_declares_schema_version_two_and_every_column ... ok
test metrics::tests::identifiers_are_exact_beyond_float_precision ... ok
test metrics::tests::unsupported_or_invalid_measurements_stay_empty_cells ... ok
test metrics::tests::retry_attempts_stay_visible_without_inventing_submissions ... ok
test metrics::tests::completed_gpu_frames_deduplicate_within_but_not_across_renderer_epochs ... ok
test metrics::tests::cpu_busy_needs_two_valid_readings_from_the_same_clock ... ok
test metrics::tests::thread_cpu_time_is_available_only_where_the_clock_is_supported ... ok
test metrics::tests::capture_is_opt_in_bounded_and_preserves_earlier_captures ... ok
test metrics::tests::malformed_or_excessive_requests_are_retained_without_opening_a_capture ... ok
test result: ok. 24 passed; 0 failed

$ cargo test --workspace
all suites ok (0/1/7/8/13/24/3/17/14 passed, 0 failed)

$ cargo clippy --workspace --all-targets
only pre-existing vendored winit warning ("direct cast of function item into an integer")

$ cargo fmt --all           # applied, no further diff
$ python3 tools/check_docs.py
PASS: 53 Markdown files, 221 local links, 15 ADRs and 20 requirements.
```

Status summary: **pass** — compile, workspace tests, focused tests, clippy, fmt,
doc check. **Not run** — Android build, host graphics smoke, any device capture,
instrumentation-overhead and repeatability measurement (lead-owned; the phone is
unavailable per the board). No performance claim is made here.

## Issues and friction

- The v1 record API funnelled every value, including the GPU frame id, through
  `[Option<f64>; 10]`. Any id past 2^53 would have been silently rounded, and a
  positional array made column drift easy. This forced a typed row rather than an
  incremental column addition, so the schema version bump was unavoidable.
- Frame identity in v1 was `presented_count`, which does not advance on retries;
  retry attempts were invisible and consecutive rows could look like one frame
  sequence. Adding `draw_attempt_id` required moving the increment above the
  renderer call while keeping the pre-renderer early returns (unfocused, no
  window, zero size) out of the count.
- GPU submission ids restart with each renderer, and the previous completion
  filter compared bare ids, so after a recreation a stale id could suppress a
  real completion or join across renderers. Hence `renderer_epoch` and a keyed
  tracker instead of `last_gpu_frame: Option<u64>`.
- The renderer submits and presents deep inside one `unsafe` block; measuring
  acquire/present required binding the call results before matching on them so
  the clock reading happens at the real boundary rather than after error handling.
- The dynamic mesh was built inline in the `upload_dynamic` argument, so build and
  upload could not be separated without binding the mesh first. Behaviour is
  unchanged (still one build and one upload per frame); only the timing boundary
  is now explicit.
- Physics tests live in `crates/matterweave-physics/tests/behavior.rs`, which is
  not an owned path for this task, so the new counter tests are an inline
  `#[cfg(test)] mod counter_tests` in the owned `src/lib.rs`.

## Decisions and rationale

- **libc as a target-specific direct dependency**, pinned `=0.2.189`, exactly the
  version already in `Cargo.lock`; the lock change is one line adding `libc` to
  the explorer's dependency list. Rust's standard library exposes no thread CPU
  clock, and `CLOCK_THREAD_CPUTIME_ID` is the contract requirement, so a thin
  wrapper around the already-locked binding is preferable to a new crate.
- **Missing stays missing.** Unsupported platforms, failed clock calls,
  out-of-range `timespec`, backwards deltas and non-finite/negative durations all
  produce empty cells. Zero is a measurement; empty is its absence.
- **Diagnostics are opt-in.** Renderer wait timings and CPU busy readings are
  taken only while a capture is active, so ordinary runs keep their previous
  clock-reading cost. The pre-existing GPU timestamp queries are unaffected and
  are not treated as evidence about logging overhead.
- **Wall time is labelled as elapsed, never as work.** The streaming section
  keeps its elapsed label because a no-op poll and a real job are
  indistinguishable at this boundary; distinguishing them needs job counters,
  deferred to the next slice. Likewise `gpu_prev_render_ms` is documented as a
  queue timestamp span, not pure execution.
- **Epoch over global submission ids.** Making the epoch part of the completion
  identity is the minimal honest fix for counter resets; renumbering GPU frames
  globally would have required changing `timing.rs` ownership of its counter for
  no measurement gain.
- **Scope held to this slice.** Streaming-job, collision-publication, edit-event,
  queue-depth and allocation counters are documented as pending rather than
  approximated from available signals.

## Solutions applied

- `metrics::COLUMNS` is the single source of column order; `FrameRow::write`
  follows it and a test asserts the row width equals the header width, which
  catches column drift instead of trusting review.
- `GpuCompletionTracker` is a small pure type, so epoch/dedup behaviour is
  testable without a Vulkan device — the renderer itself cannot be exercised
  headless here.
- `StageCapture` carries the per-draw counters/timings out of
  `sync_render_meshes` without changing its control flow or upload budget.
- Body-state counting (`body_activity`) and every new clock reading run only when
  a capture is active, so an uninstrumented frame does no extra per-body work.
  Note that a row therefore always carries these counters when it exists.
- `apply_capture_diagnostics` keeps the renderer's diagnostics flag in sync with
  capture state on renderer creation and on capture completion/failure.
- Capture files are timestamped, created with `create_new` and now use a
  `frame-profile-v2-` prefix, so older captures cannot be overwritten or
  misparsed as v2.

## Insights

- The strongest discriminator this slice adds for "stationary CPU work versus
  waiting" is the pair (`main_wall_ms`, `main_cpu_busy_ms`) together with
  `fence_wait_wall_ms`/`acquire_wall_ms`/`present_wall_ms`. Wall time alone could
  not separate a busy stationary frame from a blocked one.
- The counters already show where P02's investigation should start without any
  measurement: `dynamic_mesh_builds` and `dynamic_mesh_uploads` are structurally 1
  per frame regardless of body state, and `voxel_bodies_sleeping` will show that
  those rebuilds cover unchanged bodies. That is a hypothesis for measurement, not
  a result.
- Anything joining CPU rows to GPU work must use the epoch pair. A consumer that
  keys on the bare frame id will silently produce false joins across a
  suspend/resume cycle, which is exactly the Android lifecycle case.
- Host results say nothing about phone behaviour: the CPU clock resolution,
  present-call cost and instrumentation overhead all need device measurement
  before any capture is used to select an optimization.

## Handoff

Changed paths (all within this task's ownership):

- `apps/explorer/src/metrics.rs` — schema v2 typed rows, CPU clock, completion tracker
- `apps/explorer/src/lib.rs` — draw/mesh/save wiring, epoch, attempt ids, counters
- `apps/explorer/Cargo.toml` — `libc = "=0.2.189"` for Linux/Android only
- `Cargo.lock` — one line: `libc` added to the explorer's dependencies
- `crates/matterweave-render/src/lib.rs` — opt-in `DrawDiagnostics`, submission ids
- `crates/matterweave-physics/src/lib.rs` — `BodyActivity` and counter tests
- `docs/performance/measurement-v2.md` — schema v2 contract
- `docs/performance/logs/opus-p01-a1.md` — this log

Unchanged by design: gameplay, save/session formats, world/physics behaviour,
render output, lifecycle handling and the opt-in `profile-frames.txt` request
semantics.

Remaining lead checks: independent Muse/Gemini review, Android build, host
graphics smoke run, an actual capture inspected against this schema, phone
instrumentation-overhead and same-build repeatability, and integration with the
Muse fixture and GLM validator lanes (the validator must reject a file whose
declared `schema_version` it does not implement). Deferred counters for a later
slice: streaming jobs, collision publication, edit events, queue depths,
allocations/retained capacity.
