# Independent Review Report: P01 Candidate 02 & New Fixtures/Validators

## Overview & Scope Verification
- **Target under review**: Frozen candidate 02 at `/mnt/bench/matterweave-dev/performance/run-01/p01-candidate-02`, verified against `sha256.json` and `correction.patch`.
- **Review perspective**: Independent read-only correction/test-gap review, verification of the restored physics file, plus audit of the NEW conditions validator (`validate_conditions.py` + tests) and NEW world fixtures (`performance_replay.rs` + fixture).
- **Execution & claims**: No execution checks performed; explicitly **NOT RUN**. No mobile performance or energy claims made.

---

## Contract Verification Summary

### 1. Instrument Contract & Wait Separation
- **Typed IDs & Epoch**: `FrameRow` writes 37 typed columns. `draw_attempt_id`, `renderer_epoch`, `presented_count` are exact integers. Recreated renderer increments `renderer_epoch`, resetting GPU completion tracking cleanly (`metrics.rs:188-250`).
- **Wait Separation (GEMINI F1)**: No `Commands::wait` call was moved or removed. `upload`, `upload_chunk`, `retain_chunks`, and `upload_dynamic` each measure wait duration at their exact call sites via `WaitTally`. `begin_frame_diagnostics()` resets the tally before mesh sync. In `draw()`, `render_fence_wait_ms` times the draw fence wait separately. Pre-draw and draw fence waits are reported in distinct columns (`mesh_sync_fence_wait_wall_ms` / `render_fence_wait_wall_ms`) and never merged.
- **CPU/Wall Clock Containment (MUSE F1/F3)**: In `Explorer::draw`, `now = Instant::now()` begins immediately before `CpuBusySpan::begin(capturing)`; the CPU busy span ends via `cpu_busy.finish()` immediately before `now.elapsed()`. The busy interval is strictly contained inside the adjacent wall clock interval.
- **Retry & Present Semantics (MUSE F2)**: `classify_present` categorizes `vkQueuePresentKHR` returning `ERROR_OUT_OF_DATE_KHR` as `PresentOutcome::OutOfDate`, returning `Ok(FrameResult::Retry)` while preserving `submitted_frame_id`. `presented_count` increments only on `FrameResult::Presented`. Retries before submission (zero-size surface, acquire errors) retain `None` for `submitted_gpu_frame_id`.
- **Shadow Gate (GEMINI F2 / MUSE F4)**: `shadow_caster_meshes` is gated via `shadow_casters_for_attempt(submitted_gpu_frame_id, casters)`. A retry without a GPU submission writes empty rather than inheriting prior pass state; a submitted draw whose presentation retried retains its shadow casters.
- **Save Accounting (MUSE F1)**: `SaveAccounting` separates `attempts` and `failures`, tracking wall time across both. `take_save_accounting()` consumes and resets accounting per frame.
- **Schema & Physics Tests (GEMINI F3/F4)**: `every_column_carries_its_own_value_in_the_declared_order` validates all 37 columns name-to-value. Physics test explicitly asserts the full `BodyActivity { total: 2, active: 2, sleeping: 0, not_simulated: 0 }` struct.
- **Body-Activity Code Integrity**: Verified line-by-line against `p01-candidate-01` (`sha256 614a04a8...`). Production `body_activity(&self)` logic in `crates/matterweave-physics/src/lib.rs:184-203` is 100% intact; only docs and test assertions were updated. No body-activity code was lost.

### 2. NEW World Fixtures (`performance_replay.rs` & `performance_replay_v1.json`)
- Runs solely against existing public engine APIs (`World::generate`, `World::enable_streaming`, `World::stream_around`, `World::set`, `World::get`, `World::save`, `World::load`). Zero production engine API changes.
- Rejects unsupported fixture versions (`!= 1`), generator versions (`!= 1`), unknown ops, and empty/unbounded tapes (`1..=128` steps).
- Pinned seed (`20260907`), baseline stats (32 chunks, 42840 solid voxels), and expected stats (98 chunks, 131992 solid voxels, 34 stored overrides).
- Verifies repeated byte-for-byte save reproducibility and reload roundtrips.
- Verifies Euclidean negative chunk boundaries (`-17` in chunk `-2`, `-16` in chunk `-1`).
- Accurately declares host-only test proof; no app/phone input replay or 64-body physics integration claimed.

### 3. NEW Conditions Helper (`validate_conditions.py` & `test_validate_conditions.py`)
- Standard library only (`argparse`, `json`, `math`, `os`, `re`, `sys`). Input files opened strictly read-only (`"rb"`); no input mutation or process spawning.
- Enforces: $\ge 5$ samples, $\ge 120\text{ s}$ span, $\le 35\text{ s}$ gaps, strictly increasing finite non-negative `elapsed_s` (`bool` explicitly rejected).
- Battery dump: all 4 power fields (`AC`, `USB`, `Wireless`, `Dock`) required `false`, `status: 3` (discharging), `level` in $0..100$, temperature in $-20..80\text{ }^\circ\text{C}$; rejects `updates stopped`, `test mode`, `simulat*`.
- Thermal dump: `IsStatusOverride: false`, `Thermal Status: 0`. Live skin temperature extracted strictly between required boundaries `"Current temperatures from HAL:"` and `"Current cooling devices from HAL:"` (cached skin temperatures are structurally excluded). Exactly one finite skin reading in $-20..100\text{ }^\circ\text{C}$ required.
- Temperature stability: battery range $\le 1.0\text{ }^\circ\text{C}$, skin range $\le 2.0\text{ }^\circ\text{C}$.
- Fail-closed: `_reject_json_constant` rejects `NaN`/`Infinity`, `_reject_duplicate_keys` rejects duplicate JSON keys, `_parse_scalar_field` rejects duplicate dump keys. File size $\le 1\text{ MiB}$, line size $\le 128\text{ KiB}$.
- API raises `ValueError`; CLI prints concise `invalid: ...` to stderr with exit code 1 (no traceback). Output summary payload carries `"evidence": "idle_readiness_only"`.

---

## Concrete Candidate Findings

### Finding 1: Inverted Cardinal Direction and Eviction Semantics in Replay Test Comment
- **file:line**: `crates/matterweave-core/tests/performance_replay.rs:280-285`
- **trigger**: Developer or reviewer reading the test commentary to verify streaming eviction behavior at chunk boundaries.
- **consequence**: Directional confusion between test narration and coordinate math. The test moves the streaming center to `[-48.0, 0.0, 0.0]` (traveling WEST, $-X$) and verifies that cell `[48, 2, 0]` (chunk `[3, 0, 0]`, EAST, $+X$) is evicted to air (`0`). The comment states: `// Travel east evicts the western override: evicted chunks read as air and report no revision...`, which states the exact opposite of what the code executes.
- **evidence**:
  ```rust
  // crates/matterweave-core/tests/performance_replay.rs:280-285
  // Travel east evicts the western override: evicted chunks read as air and
  // report no revision, while the edit persists as a stored override.
  assert!(world.stream_around([-48.0, 0.0, 0.0]));
  assert_eq!(world.get([48, 2, 0]), 0);
  assert_eq!(world.chunk_revision([3, 0, 0]), None);
  assert_eq!(world.get([-17, 2, -17]), 5);
  ```
- **uncertainty**: The programmatic assertion passes and correctly verifies eviction of distant stored overrides and retention of near overrides (`[-17, 2, -17]`); the discrepancy is confined to the explanatory comment.
- **discriminating check**: Compare coordinate math: position `[-48, 0, 0]` has negative X (west). Chunk `[3, 0, 0]` at `[48, 2, 0]` has positive X (east). Updating the comment to `// Travel west evicts the eastern override:` aligns narration with code.
- **validity**: Commentary defect; **not a contract-breaking code defect**.

### Finding 2: Unclosed File Descriptor in Condition Validator Test
- **file:line**: `tools/performance/test_validate_conditions.py:519`
- **trigger**: Running the condition validator unittest suite under Python environments with warning filters configured to elevate or log `ResourceWarning`.
- **consequence**: File descriptor leak emits a `ResourceWarning` during test execution (captured in `conditions-lead-tests.log`). Under `-W error` CI runners, this will fail test execution.
- **evidence**:
  `conditions-lead-tests.log:1-2`:
  ```text
  .../tools/performance/test_validate_conditions.py:519: ResourceWarning: unclosed file <_io.BufferedReader name='/tmp/tmpr6jpwggn/ok.jsonl'>
    self.assertEqual(open(p, "rb").read(), before)
  ```
- **uncertainty**: Unittest exits 0 under default warning settings; no failure in standard execution.
- **discriminating check**: Run `python3 -W error::ResourceWarning -m unittest tools.performance.test_validate_conditions`. Replacing `open(p, "rb").read()` with `with open(p, "rb") as fh: self.assertEqual(fh.read(), before)` eliminates the warning.
- **validity**: Test hygiene defect; **not a production validator defect**.

### Finding 3: Missing Test Case for Battery `Dock powered: true` Rejection
- **file:line**: `tools/performance/test_validate_conditions.py:307-325`
- **trigger**: Auditing test suite coverage against required battery conditions.
- **consequence**: `validate_conditions.py:65` specifies `POWERED_KEYS = ("AC powered", "USB powered", "Wireless powered", "Dock powered")` and verifies each is `"false"`. The test suite provides dedicated tests for `ac="true"`, `usb="true"`, and `wireless="true"`, but omits `dock="true"` (only testing missing dock field).
- **evidence**: `test_validate_conditions.py` lines 307-325 contain `test_ac_powered_true_rejected`, `test_usb_powered_true_rejected`, and `test_wireless_powered_true_rejected`, with no corresponding `test_dock_powered_true_rejected`.
- **uncertainty**: Production implementation shares a single loop over `POWERED_KEYS`, so `Dock powered: true` is properly rejected at runtime; this is strictly an asymmetric test coverage gap.
- **discriminating check**: Add `def test_dock_powered_true_rejected(self): ...` with `dock="true"` and assert `ValueError`.
- **validity**: Test coverage gap; **not a production validator defect**.

### Finding 4: Untested Error Branches for Duplicate and Inverted HAL Section Delimiters
- **file:line**: `tools/performance/test_validate_conditions.py:380-405`
- **trigger**: Dumpsys thermalservice input containing duplicate HAL section headers (`len(starts) > 1`), duplicate end boundaries (`len(ends) > 1`), or inverted boundaries (`end <= start`).
- **consequence**: `validate_conditions.py:182-192` explicitly implements fail-closed branches for `len(starts) != 1`, `len(ends) != 1`, and `end <= start`. `test_validate_conditions.py` only tests zero occurrences (`len(starts) == 0` via `include_hal=False`, `len(ends) == 0` via `end_boundary=False`). The duplicate and inverted boundary rejection logic is never exercised by tests.
- **evidence**: `validate_conditions.py:182-192` contains branches for multiple headers and inverted ordering; `TestThermal` in `test_validate_conditions.py` has no tests with multiple section delimiters or inverted boundaries.
- **uncertainty**: The production implementation is simple and inspectable, raising clear `ValueError` exceptions; only test coverage is absent.
- **discriminating check**: Add tests providing thermal texts with duplicate `"Current temperatures from HAL:"` headers or end boundary before start header and verify `ValueError`.
- **validity**: Test coverage gap; **not a production validator defect**.

### Assessment
**None valid** as blocking measurement, engine, or contract-invalidating defects. The candidate-02 instrumentation, replay fixtures, and conditions validator strictly adhere to their specified contracts.

---

## Concise Engineering Log

### Actions Taken
1. Reviewed `sha256.json` and `correction.patch` comparing candidate 02 against candidate 01 and baseline worktree.
2. Verified `crates/matterweave-physics/src/lib.rs` line-by-line against candidate 01 (`sha256 614a04a8...`); confirmed `body_activity` implementation was preserved without code loss.
3. Audited `WaitTally` implementation and verified call sites across `upload`, `upload_chunk`, `retain_chunks`, and `upload_dynamic` in `crates/matterweave-render/src/lib.rs`.
4. Verified `classify_present`, `DrawOutcome::Retry` on `ERROR_OUT_OF_DATE_KHR`, and `submitted_frame_id` retention.
5. Audited `apps/explorer/src/lib.rs` clock ordering (`now` wall clock enclosing `CpuBusySpan`), shadow caster gating, and `SaveAccounting`.
6. Verified 37-column declared order and full name-to-value test in `apps/explorer/src/metrics.rs`.
7. Audited `crates/matterweave-core/tests/performance_replay.rs` and `performance_replay_v1.json` for deterministic tape replay, persistence roundtrips, and Euclidean negative coordinates.
8. Audited `tools/performance/validate_conditions.py` and `test_validate_conditions.py` against all readiness criteria (timing monotonicity, battery/thermal limits, HAL section parsing, fail-closed JSON hooks).

### Issues & Friction
- Read-only tooling without shell execution requires manual symbolic tracing of coordinate math (e.g. Euclidean div-mod on negative chunk coordinates like cell `[-17, 2, -17]` mapping to chunk `[-2, 0, -2]`).
- Directional terminology in test comments required careful coordinate verification against `World::stream_center_of` and `STREAM_RADIUS_CHUNKS` bounds.

### Decisions & Rationale
- Classified all 4 candidate findings as non-invalidating: Finding 1 is a comment direction slip; Finding 2 is test file descriptor hygiene; Findings 3 & 4 are minor test coverage gaps where production logic is already fail-closed.
- Maintained strict distinction between zero-waits (`Some(0)`) when diagnostics are active vs missing (`None`) when disabled.

### Solutions Applied
- Identified concrete discriminators for each candidate finding so the lead can address comment clarification and test warning hygiene without altering production logic.

### Insights
- Gating pre-draw upload fence waits via `WaitTally` at actual call sites resolves the architectural misattribution where GPU wait time was previously hidden inside `dynamic_upload_wall_ms`.
- Anchoring HAL skin parsing strictly between section boundary strings (`Current temperatures from HAL:` to `Current cooling devices from HAL:`) is effective protection against stale cached skin entries in Android dumpsys output.

---

Lead owns acceptance, source checks, and device qualification. Bounded review complete.
