# P01 Instrumentation Candidate Review (candidate-01)

Candidate directory: `/mnt/bench/matterweave-dev/performance/run-01/p01-candidate-01`  
Patch SHA256: `95b9be2626d1f1b101cf07af40797aa30377fc80910cc1799b5ffbb8c3d4bd00`  
Base commit: `755620472e8457d8e55760165e759b6d8013c6db`  
Verification mode: read-only static analysis leaf (no shell execution, no subagents; execution checks **NOT RUN**).

---

## Concrete Findings

### Finding 1: `fence_wait_wall_ms` is timed after the submission fence was already waited in `upload_dynamic`

- **file:line**: `crates/matterweave-render/src/lib.rs:1331` (see also `crates/matterweave-render/src/lib.rs:1236`, `apps/explorer/src/lib.rs:904`, `docs/performance/measurement-v2.md:85, 96`)
- **trigger**: Any frame where dynamic meshes are synchronized via `sync_render_meshes` prior to `render_with_lighting` (the default unconditional frame path).
- **consequence**: `upload_dynamic` calls `self.commands.wait()?` before `Renderer::draw` is invoked. Because `self.commands.fence` is already signaled when `draw` reaches its timed `self.commands.wait()?` at line 1332, `fence_wait_wall_ms` will record ~0 ms. Any real CPU blocking on the prior submission fence is absorbed into `dynamic_upload_wall_ms` and `mesh_sync_wall_ms`. This invalidates the schema claim that `fence_wait_wall_ms` captures the blocking wait "at the real call site" and that `render_wall_ms` contains `fence_wait_wall_ms`.
- **source evidence**:
  - `apps/explorer/src/lib.rs:904`: `sync_render_meshes` calls `renderer.upload_dynamic(&dynamic)?;` unconditionally every frame.
  - `crates/matterweave-render/src/lib.rs:1236`: `upload_dynamic` invokes `self.commands.wait()?;` immediately upon entry.
  - `crates/matterweave-render/src/lib.rs:1331-1333`: `Renderer::draw` times `let fence_begin = self.diagnostics_enabled.then(Instant::now); self.commands.wait()?; self.diagnostics.fence_wait_ms = fence_begin.map(elapsed_ms);`. Because the fence was already waited on and signaled in `upload_dynamic`, `vkWaitForFences` returns immediately.
  - `docs/performance/measurement-v2.md:85, 96`: states `fence_wait_wall_ms` is "Blocking wait on the previous submission's fence, at the real call site" and that "`render_wall_ms` contains `fence_wait_wall_ms`".
- **uncertainty**: On frame 1, `Commands::new` initializes the fence signaled, so neither call blocks. On later frames, if GPU execution finishes before CPU physics and dynamic mesh construction, both waits are ~0 ms; but whenever the frame is GPU-bound, the wait latency lands in `dynamic_upload_wall_ms` instead of `fence_wait_wall_ms`.
- **proposed discriminating check**: (NOT RUN) Inject a 20 ms GPU workload or artificial fence delay and inspect the captured row. Verify whether `fence_wait_wall_ms` remains ~0 ms while `dynamic_upload_wall_ms` increases by the wait duration.

---

### Finding 2: `shadow_caster_meshes` leaks the previous frame's count on retry draw attempts

- **file:line**: `crates/matterweave-render/src/lib.rs:1327`, `crates/matterweave-render/src/shadow.rs:277`, `apps/explorer/src/lib.rs:1082-1084`
- **trigger**: A draw attempt resulting in `FrameResult::Retry` (e.g. `acquire_next_image` returning `ERROR_OUT_OF_DATE_KHR` or zero extent) immediately following a frame that rendered shadow casters.
- **consequence**: The retry row in the CSV records `shadow_caster_meshes` from the *previous* frame rather than empty or 0, falsely reporting shadow caster draws on an attempt where no command buffer was recorded or submitted.
- **source evidence**:
  - `crates/matterweave-render/src/lib.rs:1327`: `draw` resets `self.diagnostics = DrawDiagnostics::default()`, but does not reset `self.shadow.caster_meshes`.
  - `crates/matterweave-render/src/shadow.rs:277`: `self.caster_meshes = 0;` occurs only inside `Shadow::record`.
  - `crates/matterweave-render/src/lib.rs:1329, 1373, 1395`: early returns for `FrameResult::Retry` exit before `Shadow::record` is reached.
  - `apps/explorer/src/lib.rs:1082-1084, 1137`: `Explorer::draw` queries `r.shadow_caster_meshes()` and logs it unconditionally into `FrameRow` even when `result == DrawOutcome::Retry`.
  - `docs/performance/measurement-v2.md:126, 142`: states "Counters are exact integers for this attempt unless stated otherwise" and "A retry row is a real draw attempt that produced no submission."
- **uncertainty**: If the retry occurs on the very first frame of a run (before any shadow pass), `self.shadow.caster_meshes` is 0. The stale count only leaks when retrying after a successful shadow pass.
- **proposed discriminating check**: (NOT RUN) Simulate an `ERROR_OUT_OF_DATE_KHR` retry in a harness after a frame that rendered non-zero shadow casters. Verify whether `shadow_caster_meshes` on the retry row outputs the prior count rather than empty or 0.

---

### Finding 3: Test assertion gap for work counters and stage timings in CSV schema v2

- **file:line**: `apps/explorer/src/metrics.rs:432-463`, `apps/explorer/src/lib.rs:1088-1140`
- **trigger**: Running existing test suites to catch CSV column position drift, counter formatting regressions, or missing fields.
- **consequence**: 26 of the 34 columns (including all work counters: `chunk_mesh_uploads`, `dynamic_mesh_builds`, `dynamic_mesh_uploads`, `saves_performed`, `shadow_caster_meshes`, `gpu_prev_shadows`, all `voxel_bodies_*` counts, and multiple stage timings) are never verified for cell position, formatting (`0` vs `""`), or value integrity. In addition, there are zero behavioral tests verifying that `Explorer::draw` populates `StageCapture` or hooks up `DrawDiagnostics` in an active capture session.
- **source evidence**:
  - `apps/explorer/src/metrics.rs:432-463`: `unsupported_or_invalid_measurements_stay_empty_cells` asserts only 8 cells (`main_wall_ms`, `main_cpu_busy_ms`, `physics_wall_ms`, `render_wall_ms`, `save_wall_ms`, `present_wall_ms`, `submitted_gpu_frame_id`, and `physics_fixed_steps`).
  - `apps/explorer/src/metrics.rs:360-388`: `header_declares_schema_version_two_and_every_column` only asserts `split(',').count() == COLUMNS.len()`.
  - `gpu_prev_shadows` mapping (`None` -> `""`, `Some(false)` -> `"0"`, `Some(true)` -> `"1"`) has zero test coverage.
  - None of the tests in `apps/explorer/src/lib.rs` (lines 1450–1625) test `StageCapture` or metrics row construction during a draw.
- **uncertainty**: Manual inspection confirms that `COLUMNS` and `FrameRow::write` currently match 1:1. The risk is regression vulnerability and lack of automated contract enforcement for 76% of the schema columns.
- **proposed discriminating check**: (NOT RUN) Swap adjacent counter fields in `FrameRow::write` (e.g. `voxel_bodies_active` and `voxel_bodies_sleeping`) without altering `COLUMNS`, and run `cargo test -p matterweave-explorer`. All existing tests will pass.

---

### Finding 4: Incomplete assertions in physics body activity unit tests

- **file:line**: `crates/matterweave-physics/src/lib.rs:976-981`
- **trigger**: Running `counter_tests::body_activity_counts_voxel_bodies_and_excludes_the_character` to verify body activity categorization.
- **consequence**: The test asserts `activity.total == 2` and `activity.active + activity.sleeping + activity.not_simulated == activity.total`, but never asserts the expected breakdown (`active == 2`, `sleeping == 0`, `not_simulated == 0`). A defect causing `Physics::body_activity` to classify newly restored bodies as `not_simulated` or `sleeping` would pass this test undetected.
- **source evidence**:
  - `crates/matterweave-physics/src/lib.rs:976-981`:
    ```rust
    let activity = physics.body_activity();
    assert_eq!(activity.total, 2, "the character is not a voxel body");
    assert_eq!(
        activity.active + activity.sleeping + activity.not_simulated,
        activity.total,
        "every body is counted exactly once"
    );
    ```
  - By contrast, lines 1017–1023 and 1051–1057 in the sibling counter tests explicitly check individual fields (`activity.sleeping == 1`, `activity.not_simulated == 1`).
- **uncertainty**: Minor test gap; the underlying categorization in `body_activity()` (`if !body.is_enabled() ... else if body.is_sleeping() ... else ...`) is straightforward.
- **proposed discriminating check**: (NOT RUN) In `crates/matterweave-physics/src/lib.rs:194`, temporarily change `activity.active += 1` to `activity.sleeping += 1` and run `cargo test -p matterweave-physics --lib counter_tests::body_activity_counts_voxel_bodies_and_excludes_the_character`. Confirm the test passes.

---

## Engineering Log

### Actions Taken
- Read frozen candidate files and checksum manifest at `/mnt/bench/matterweave-dev/performance/run-01/p01-candidate-01/` (`sha256.json`, `candidate.patch`, `apps/explorer/src/metrics.rs`, `apps/explorer/src/lib.rs`, `crates/matterweave-render/src/lib.rs`, `crates/matterweave-physics/src/lib.rs`, `docs/performance/measurement-v2.md`, `docs/performance/logs/opus-p01-a1.md`).
- Read unchanged supporting context in `/mnt/bench/matterweave-dev/worktrees/performance-p00` (`crates/matterweave-render/src/timing.rs`, `crates/matterweave-render/src/shadow.rs`, `apps/explorer/src/lib.rs`, `docs/performance/board.json`).
- Verified schema v2 contract against code: 34 columns, header `#matterweave-frame-capture schema_version=2`, integer identity exactness, empty cell rules, Linux/Android `clock_gettime(CLOCK_THREAD_CPUTIME_ID)` wrapper, and `GpuCompletionTracker`.
- Evaluated timing boundaries across `sync_render_meshes`, `upload_dynamic`, `render_with_lighting`, and `timing.rs`.

### Issues & Friction
- The renderer call graph performs multiple internal fence waits (`upload_dynamic`, `upload_chunk`, `retain_chunks`) before `Renderer::draw` executes. This architectural coupling undermines attempts to isolate pure submission wait times inside `Renderer::draw`.

### Decisions & Rationale
- Constrained review strictly to candidate-01 read-only inspection without generating code edits or running terminal commands.
- Bounded concrete findings to 4 high-signal items directly tied to schema validity, metric accuracy, and behavioral verification gaps.

### Solutions Applied
- Identified the root cause of `fence_wait_wall_ms` measurement nullification (pre-empted by `upload_dynamic` fence wait).
- Identified stale counter propagation on retry frames (`shadow_caster_meshes`).
- Documented precise test gaps in `metrics.rs` (unasserted columns) and `counter_tests` (under-asserted category distribution).

### Insights
- Separating dynamic mesh build from upload exposed that dynamic mesh upload was already serving as the implicit fence wait point for the whole engine. To obtain an accurate `fence_wait_wall_ms`, either `upload_dynamic` needs to defer its fence wait or the diagnostic fence wait must be placed at the first point where fence synchronization actually occurs in the frame sequence.
- When capturing retries as first-class rows, all stateful counters must be explicitly zeroed or gated by stage completion to avoid leaking prior frame values.

---

*Handoff: Ready for integration review and verification against `docs/performance/logs/gemini-p01-schema.md`.*
