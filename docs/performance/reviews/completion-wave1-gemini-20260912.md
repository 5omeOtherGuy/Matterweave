# Frozen wave 1 review — gemini

Target `22070ca`, base `704bb4a`. Independent source review; unverified candidates require lead triage.

### Candidate Findings

#### 1. Dropped `Resume` Compensation on Full Command FIFO Leaves Mixer Permanently Muted
- **File / Line:** `crates/matterweave-audio/src/service.rs:517-526`
- **Classification:** Pre-existing (present in `704bb4a`, retained in `22070ca`)
- **Trigger:** `AudioService::suspend()` is called when the command ring buffer has exactly one vacant slot (`queue::vacant(&self.producer) == 1`) and `backend.suspend()` fails (e.g. AAudio `request_pause` returns an error).
- **Consequence:** `suspend()` checks `queue::vacant(&self.producer) == 0` (false), pushes `Command::Suspend` (leaving queue capacity at 0 vacant slots), and calls `backend.suspend()`. When `backend.suspend()` returns an error, the compensation attempt `let _ = self.push(Command::Resume)` fails due to backpressure (`CommandQueueFull`), and the error is discarded (`let _ =`). `suspend()` returns `Err(device_error)` without setting `self.running = false` or `self.suspended = true`. Because device pause failed, the audio stream continues delivering data callbacks; the render thread pops `Command::Suspend` from the queue and freezes `MixerCore` in permanent silence. A subsequent `service.resume()` call short-circuits at `if self.running { return Ok(()); }` and does nothing. Output remains muted and unrecoverable without recreating the service.
- **Proof / Source:**
  `src/service.rs:517-526`:
  ```rust
  if queue::vacant(&self.producer) == 0 {
      self.rejected_commands += 1;
      return Err(AudioServiceError::CommandQueueFull);
  }
  self.push(Command::Suspend).expect("queue space checked above");
  if let Some(backend) = self.backend.as_mut() {
      if let Err(device_error) = backend.suspend() {
          let _ = self.push(Command::Resume);
          return Err(device_error);
      }
  }
  ```
  coupled with `resume()` line 535: `if self.running { return Ok(()); }`.
- **Uncertainty:** Low on logic and queue arithmetic; depends on backend reporting a pause failure when the queue is nearly saturated.

---

#### 2. Unconsumed `disconnected` Flag Across Failed Start Triggers Spurious Second Stream Teardown
- **File / Line:** `crates/matterweave-audio/src/service.rs:280-288`, `679` & `crates/matterweave-audio/src/backend/aaudio.rs:185-195`
- **Classification:** Introduced (commit `243e59e` / `22070ca`)
- **Trigger:** A stream encounters an error callback (setting `shared.disconnected = true`) while stopped or paused, and the application subsequently calls `resume()` before `poll_device()`. `start_output()` calls `backend.start()`, which fails.
- **Consequence:** In `start_output()`:
  ```rust
  if let Err(error) = backend.start() {
      let _ = backend.close();
      self.backend = None;
      self.running = false;
      self.recovery_pending = true;
      return Err(error);
  }
  ```
  `self.backend` is dropped without calling `backend.take_error()`, leaving `self.shared.disconnected` as `true`. When recreation runs (`recreate_output_with`), it opens a new stream and resets `self.shared.error_code.store(0, Ordering::Relaxed)`, but does not clear `self.shared.disconnected`. On the subsequent frame's `poll_device()`, `new_backend.take_error()` executes `self._shared.disconnected.swap(false, Ordering::AcqRel)`, reading `true`. The new, healthy stream marks itself `lost = true` and emits `BackendError { code: 0 }`. `poll_device()` records a spurious device error and immediately tears down and recreates the healthy stream a second time.
- **Proof / Source:**
  - `src/backend/aaudio.rs:186`: `self._shared.disconnected.swap(false, Ordering::AcqRel)`
  - `src/service.rs:280-288`: `backend.start()` error branch closes backend and drops it without consuming or resetting `shared.disconnected`.
  - `src/service.rs:679`: `recreate_output_with` resets `shared.error_code` to 0 but leaves `shared.disconnected` untouched.
- **Uncertainty:** Low. The atomic lifecycle across backend destruction and recreation is clear.

---

#### 3. Real AAudio Backend Does Not Invalidate `stream_properties` Upon Disconnect Prior to `poll_device`
- **File / Line:** `crates/matterweave-audio/src/backend/aaudio.rs:150-156` (vs `src/backend/mock.rs:70-76, 137-142`)
- **Classification:** Pre-existing (present in `704bb4a`, retained in `22070ca`)
- **Trigger:** An AAudio error callback fires on device disconnect, setting `shared.disconnected = true`. The caller invokes `AudioService::stream_properties()` before calling `poll_device()`.
- **Consequence:** `AaudioOutput::properties(&self)` checks only `if self.lost { None } else { self.props }`. Unlike `MockOutput::inject_device_error` which explicitly executes `self.lost.set(true)`, `AaudioOutput::lost` is set to `true` exclusively inside `take_error(&mut self)` when polled. Consequently, on device builds, `stream_properties()` continues returning `Some(props)` on a disconnected stream until `poll_device()` runs, diverging from the mock test contract in `tests/acceptance.rs:677` ("properties hidden while failed").
- **Proof / Source:**
  `src/backend/aaudio.rs:150-156`:
  ```rust
  fn properties(&self) -> Option<StreamProperties> {
      if self.lost { None } else { self.props }
  }
  ```
  `self._shared.disconnected` is never inspected in `AaudioOutput::properties(&self)`.
- **Uncertainty:** Low. Direct divergence between mock test harness and native AAudio implementation.

---

#### 4. Qualification Resolution Reports Overhead "Exceeds Noise" When Both Overhead and Noise Are Zero
- **File / Line:** `tools/performance/qualify_measurements.py:434-439`
- **Classification:** Introduced (commit `22070ca`)
- **Trigger:** A qualification run on metrics where repeats across both states show zero spread (`floor == 0.0`) and the mean on-vs-off difference is zero (`mean_delta == 0.0`).
- **Consequence:** In `resolution`:
  ```python
  "verdict": ("below same-build noise range"
              if floor > 0 and abs(mean_delta) <= floor
              else "exceeds same-build noise range")
  ```
  When `floor == 0.0`, `floor > 0` evaluates to `False`, forcing the ternary expression to the `else` clause. The report emits `"exceeds same-build noise range"` even when `mean_delta == 0.0`. Zero overhead is reported as exceeding zero noise.
- **Proof / Source:**
  `tools/performance/qualify_measurements.py:437-438`.
- **Uncertainty:** Low. Deterministic evaluation for metrics with zero repeat variance.

---

#### 5. Scheduler State Coverage Assertion in Detail Cadence Movement Test Is Tautological
- **File / Line:** `crates/matterweave-physics/tests/detail_cadence.rs:191-200`
- **Classification:** Introduced (commit `d8810da` / `22070ca`)
- **Trigger:** Running `edit_during_active_movement_blocks_until_publication_then_frees_the_path()`.
- **Consequence:** Lines 191-192 record:
  ```rust
  saw_pending |= tracked.results == 0;
  saw_buffered |= tracked.results == 1;
  ```
  Line 199 asserts:
  ```rust
  assert!(saw_pending || saw_buffered, "the withheld walk must observe a real scheduler state");
  ```
  `tracked.results` is `usize::from(queue.result.is_some())`, which can only ever be `0` or `1` for a single queued job. The expression `(results == 0) || (results == 1)` is a tautology that evaluates to `true` on the very first frame under all possible worker states. The assertion cannot fail and provides no verification that any specific scheduler state was actually observed during the walk.
- **Proof / Source:**
  `crates/matterweave-physics/src/async_detail_collision.rs:352`: `results: usize::from(queue.result.is_some())`.
  `crates/matterweave-physics/tests/detail_cadence.rs:191-200`.
- **Uncertainty:** Low. Boolean logic over a binary field.

---

#### 6. Teleportation Publication Gate Test Retains Non-Discriminating Single-Cell Geometry
- **File / Line:** `crates/matterweave-physics/tests/detail_cadence.rs:935-950`
- **Classification:** Pre-existing (present in `704bb4a`, retained in `22070ca`)
- **Trigger:** Evaluating character blocking in `teleported_character_is_gated_before_the_next_physics_step`.
- **Consequence:** `edit_during_active_movement_blocks_until_publication_then_frees_the_path` was upgraded to `scene_with_floor_and_tall_wall` (0.5 m column) because the single-cell wall (0.25 m) was below the 0.30 m autostep limit and allowed the character to climb over solid obstacles. However, `teleported_character_is_gated_before_the_next_physics_step` still uses `scene_with_floor_and_wall` (0.25 m). The test stops immediately at asserting collider count without stepping physics; if stepped, the fixture would not discriminate between publication gating and autostep climbing.
- **Proof / Source:**
  `crates/matterweave-physics/tests/detail_cadence.rs:935-950` and explanatory doc comment at lines 73-82.
- **Uncertainty:** Medium. The test as written does not step physics to failure, but the fixture geometry lacks discrimination for physical movement.

---

### Compact Engineering Log

- **Actions Taken:**
  - Inspected worktree status and git common-dir reflog across `704bb4a..22070ca`.
  - Examined the exact frozen diff `/mnt/bench/matterweave-dev/completion-20260912/wave1-frozen.diff`.
  - Audited `crates/matterweave-audio` (`service.rs`, `mixer.rs`, `backend/aaudio.rs`, `backend/mock.rs`, `service/tests.rs`, `tests/recovery.rs`, `tests/acceptance.rs`, `examples/audio_diagnostic.rs`).
  - Audited `crates/matterweave-physics/tests/detail_cadence.rs` and related cadence logic in `src/detail_cadence.rs` and `src/async_detail_collision.rs`.
  - Audited `tools/performance/qualify_measurements.py` and `test_qualify_measurements.py` against `docs/performance/measurement-qualification.md` and historical evidence `2026-09-08-p01-overhead.json`.
  - Evaluated state machine transitions, error propagation, atomics, queue boundaries, and arithmetic.

- **Issues & Friction:**
  - Absence of shell execution tools required locating and reading git reflogs and loose diff files directly via read tools.

- **Decisions & Rationale:**
  - Classified findings strictly into pre-existing vs introduced relative to base `704bb4a`.
  - Focused strictly on actionable correctness, concurrency, queue boundaries, and arithmetic defects rather than stylistic choices.

- **Solutions Applied:**
  - Provided exact file:line references, concrete triggers, observable consequences, and underlying code proofs for each candidate finding.

- **Insights:**
  - Decoupling control-side intent from asynchronous backend callbacks requires strict hygiene around shared atomics; when an output stream dies and is dropped without calling `take_error()`, unconsumed flags must be explicitly purged during recreation.
  - Ring buffer compensation actions (like pushing `Resume` when `backend.suspend()` fails) must reserve queue capacity ahead of time or risk silent, permanent state divergence.

---

### Explicit Checks NOT RUN
- **Cargo Builds / Native Compilation:** No `cargo test`, `cargo build`, or Android cross-compilation (`--target aarch64-linux-android`) was executed (read-only Pi leaf).
- **Test Executions:** Neither `cargo test -p matterweave-audio` nor `cargo test -p matterweave-physics --test detail_cadence` was run.
- **Python Unittests:** `python3 tools/performance/test_qualify_measurements.py` was not executed.
- **Hardware / Device Verification:** No Android device access (OnePlus 13 or emulator) was attempted; no real AAudio stream was opened, paused, or recovered on hardware.
