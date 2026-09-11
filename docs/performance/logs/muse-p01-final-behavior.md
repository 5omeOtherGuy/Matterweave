## P01-candidate02 bounded read-only review — NO VALID FINDINGS

Scope: `/mnt/bench/matterweave-dev/performance/run-01/p01-candidate-02` (frozen, `sha256.json`), `correction.patch` vs candidate01. NEW full files: `crates/matterweave-core/tests/performance_replay.rs`, `tests/fixtures/performance_replay_v1.json`, `tools/performance/validate_conditions.py` + tests. No execution checks — explicitly NOT RUN. No performance claim. Acceptance / source checks / device qualification remain with Lead.

Candidates examined as concrete findings; each was rejected on source evidence. Zero valid findings to report.

### Candidate 1 — Retry-after-submission loses submission identity (REJECTED)
- `file:line`: `crates/matterweave-render/src/lib.rs:1651-1658`, `apps/explorer/src/lib.rs:1114`
- Trigger: `vkQueuePresentKHR` returns `ERROR_OUT_OF_DATE_KHR` after a successful `queue_submit`.
- Consequence if true: retry row would drop `submitted_gpu_frame_id`, understating GPU work.
- Evidence (not a bug): `OutOfDate` arm sets `recreate=true` and `return Ok(Retry)` **without** clearing `diagnostics.submitted_frame_id` (assigned at submit, ~line 1640s `self.submissions += 1; submitted_frame_id = Some(...)`); explorer records `diagnostics.submitted_frame_id` directly; `metrics.rs` retry test asserts row 3 keeps `"2"`. Acquire-path / zero-size retries return before submit → correctly empty.
- Uncertainty: none on source path; device/Vulkan behavior not run.
- Discriminating check (Lead, device): capture with forced out-of-date present; assert retry row carries prior submission id and `presented_count` stands still.

### Candidate 2 — Fence-wait double-count / leak across rows (REJECTED)
- `file:line`: `crates/matterweave-render/src/lib.rs:1257-1311` (`WaitTally` at 4 upload/retain sites), `:1392-1410` (`draw()` preserves upload waits), `apps/explorer/src/lib.rs:962` (`begin_frame_diagnostics()` before mesh sync)
- Trigger: upload `Commands::wait` + draw `commands.wait()` on same fence; stale tally leaking to next row.
- Consequence if true: `mesh_sync_fence_wait_wall_ms` + `render_fence_wait_wall_ms` misattributed or summed wrong.
- Evidence (not a bug): each upload/retain site wraps only its own `commands.wait()` with `timed_begin/record`; draw clears only its own fields via `..self.diagnostics` spread, preserving upload tally; per-frame reset happens before mesh sync; disabled tally takes no clock and reports `None` vs `Some(0)`; unit tests cover 0/1/N, reset-drop, disabled-no-clock. Docs (`measurement-v2.md` fence scope) match: two named columns, never a single total; `device_wait_idle` recreate path correctly out of scope.
- Uncertainty: implicit driver-internal waits remain unmeasured by design (docs say so).
- Discriminating check (Lead): capture with known upload-heavy frame; assert `mesh_sync_fence_waits>=1`, render wait near-zero, no carryover to next idle row.

### Candidate 3 — Save failures hidden / wall misattributed (REJECTED)
- `file:line`: `apps/explorer/src/lib.rs:310-330` (`SaveAccounting`, `take_save_accounting` via `mem::take`), `:1106`, `:1130-1131`
- Trigger: failed save (bad path) or autosave outside draw.
- Consequence if true: `save_attempts/failures` mismatch, wall reset lost.
- Evidence (not a bug): `attempts+=1` at entry, `failures+=1` on `Err`, `wall_ms+=elapsed` on all paths; `take_save_accounting()` consumes per recorded row; test asserts 1-good then 2-attempt/1-failure mix with `wall_ms>0` and reset-to-default.
- Uncertainty: none on accounting; filesystem error kinds device-dependent, not run.
- Discriminating check (Lead): row-level capture across a forced failed save; assert `attempts=failures+successes`, `failures<=attempts`.

### Candidate 4 — Conditions validator accepts cached skin / bool elapsed / oversized input (REJECTED — Gemini owns coverage, independent spot-check only)
- `file:line`: `tools/performance/validate_conditions.py:parse_thermal_state` (HAL-only scan, cached ignored), `_finite_number` (bool reject), `load_jsonl` + `validate_readiness` (1 MiB / 128 KiB), power/status/gap/span/range checks
- Trigger: stale cached skin 45.979, `elapsed_s:true`, duplicate keys, NaN, `status!=3`, override/status0, gap>35s, span<120s, range violations.
- Consequence if true: unqualified device admitted as "ready".
- Evidence (not a bug on inspection): skin read strictly between single `Current temperatures from HAL:` / `Current cooling devices...` markers, exactly-one required, malformed entries rejected, missing→reject (never 0°C); all 4 power fields required `false`, `status==3`, `IsStatusOverride==false`, `Thermal Status==0`; `bool` rejected as number; `parse_constant` rejects NaN/Infinity; duplicate keys rejected; size bounds enforced on raw file and per-line plus re-encoded total; result `idle_readiness_only`. Test file header explicitly marks fixtures SYNTHETIC.
- Uncertainty: did not re-verify all 60 Lead tests (counts not trusted per brief); validator certifies reported-data readiness only, not energy/perf/calibration — correctly scoped in docstring.
- Discriminating check (Lead): feed record with HAL skin removed but cached skin present → must reject; `elapsed_s:true` → must reject.

### Candidate 5 — Fixture overclaims determinism / mutates production API; body_activity logic lost in wipe/restore (REJECTED)
- `file:line`: `crates/.../performance_replay.rs:1-12` (scope disclaimer), `:parse_fixture_strict`, `:replay`, `:baseline_generation...`, `:negative_boundary...`; fixture JSON (seed 20260907, 11 steps); `crates/matterweave-physics/src/lib.rs:181-200`
- Trigger: mutation-test `git checkout` wiped attempt1 physics file; restored from candidate01 hash + reapplied.
- Consequence if true: lost simulation-disable logic or hidden engine API change propping up replay.
- Evidence (not a bug on inspection): `correction.patch` physics hunk is comment-only + stricter `counter_tests` equality assertion (`total/active/sleeping/not_simulated`); `body_activity()` body still counts `!enabled→not_simulated / sleeping / active`, character excluded; disable rule at `:317-329` (`y<-32` clamp to `-32`, `set_enabled(supported && |dx|<40 && |dz|<40 && y>-32)`) matches doc `y<=-32` bucket (clamped value fails `>-32`); replay uses only public `World::generate/enable_streaming/stream_around/set/get/stats/save/load`; `sha256.json` lists no `matterweave-core/src` production file; negative-boundary test asserts eviction-then-reversal restores override (`[-17, 2, -17]==5`, `[48, 2, 0]==9` after return), save-bytes exact-equality across replays and reload.
- Uncertainty: `World::new(0)` chunk-math preamble uses seed 0 (unit check, not the pinned determinism claim); `stored_overrides:34`/counts are pinned values, not independently recomputed here (no run).
- Discriminating check (Lead): diff production `matterweave-core/src` vs baseline; re-run 10 replay tests on host; confirm `body_activity` logic hash unchanged apart from comment/test.

### Contract spot-checks (all match, no finding)
- Typed integer IDs + epoch: `draw_attempt_id/presented_count/submitted/completed` are `u64` via `write_id`; epoch-keyed `GpuCompletionTracker`; 37-column name→value test present.
- `presented_count` = successful `vkQueuePresentKHR` only (`Ok` incl. suboptimal; OOD→Retry, no increment); docs explicitly not scanout.
- CPU span: wall `Instant::now()` → `CpuBusySpan::begin` → `finish()` immediately before wall stop, no queries between; `cpu_busy_ms` returns `None` on missing/backwards; Linux/Android-only, never zero-on-unsupported.
- Shadows: `shadow_casters_for_attempt(submitted, casters)` = `None` when no submission; docs match.
- Docs: missingness (diagnostics-disabled + pacing-gap `gpu_prev_*`), clock baseline (4 pre-existing readings + save/GPU bookkeeping still budgeted), scope (streaming/collision/edit/queue, overhead, energy explicitly deferred/unqualified) — consistent with code.
- Overhead / deferred counters remain unqualified by design; worker fmt/clippy PASS and 10/60-test counts not relied upon.

## Engineering log
- **Actions Taken**: Listed candidate02 + worktree; read `sha256.json`, full `correction.patch`; read `metrics.rs` (schema, `CpuBusySpan`, `shadow_casters_for_attempt`, `FrameLog`), explorer draw path (attempt identity, CPU/wall order, `begin_frame_diagnostics`, save accounting, row build), renderer diagnostics/`WaitTally`/`classify_present`/submit+present path, physics `body_activity` + disable rule, `measurement-v2.md` tail (fence scope, CPU containment, missingness, not-measured, change rules), `validate_conditions.py` (full) + test header/builders, replay test + fixture JSON; targeted greps for submission/present/wait/save/shadow/below-world sites.
- **Issues & Friction**: `grep` without tight path scope returned cross-tree matches (noisy); mitigated with file-scoped follow-up reads. `measurement-v2.md` offset beyond EOF on first read; re-read correct tail range. No shell/execution available per brief — all checks are source-inspection only.
- **Decisions & Rationale**: Treated "none valid" as: examine each plausible defect as a concrete candidate with required fields, reject on evidence, report zero valid findings rather than inflating nits. Did not trust worker pass-counts; inspected fixture/validator assumptions directly.
- **Solutions Applied**: None (read-only leaf; no edits).
- **Insights**: The correction's strength is making "missing vs zero" and "before vs after submission retry" explicit in types, tests, and docs; residual risk is all on device qualification (overhead, thermal/battery lead records, real out-of-date presents) which this review explicitly did not run.

