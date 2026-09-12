# D4.1 shared application audio adapter — 2026-09-12

Worker log for slice D4.1 of M6 Phase A. Branch `phase-a/d4-1-audio-adapter`, base
commit `7dace54`, worktree `/mnt/bench/matterweave-dev/worktrees/d4-1-audio-adapter`.
Owned paths: `apps/explorer/src/audio_service.rs` (module + unit tests) and this log.
Nothing was pushed and no PR was opened. No device was touched.

## Actions Taken

- Read the task slice, `crates/matterweave-audio/src/{service,error,clip,config,backend,
  backend/mock,handle,mixer}.rs`, `apps/explorer/{Cargo.toml,src/{lib,experience,
  wetland,voxel_relay}.rs}` and the two most recent audio engineering logs
  (`e1-audio-pcm-boundary`, `e1-audio-recovery`) before writing anything.
- Committed the lead's uncommitted worktree changes unchanged with this slice: the
  `mod audio_service;` declaration in `apps/explorer/src/lib.rs`, the per-target
  `matterweave-audio` dependency in `apps/explorer/Cargo.toml` and the resulting
  `Cargo.lock` entry. They were not edited.
- Wrote the adapter as one module, `apps/explorer/src/audio_service.rs` (1460 lines:
  920 production, 538 tests): `GameplayEvent` vocabulary, `AudioScope`, trigger
  outcomes and drop reasons, counters/status, clip synthesis and registration, voice
  bookkeeping, mute/volume/suspend/resume/switch cleanup, device open and recovery.
- Added twelve unit tests inside the module (the only place with access to the private
  service handle needed to drive the mock renderer and inject faults).
- Verified the tests discriminate: three mutation batches (guards removed, cleanup
  semantics removed, backpressure handling removed) each made the relevant tests fail.
  The mutated tree was restored byte for byte (`cmp`) before the final runs.
- Ran the full check set on the final tree: package tests, workspace tests, strict
  clippy, formatting and the docs link checker.

## Issues & Friction

- **`is_voice_active` is not a completion oracle.** The service documents it as
  "on a device the answer can lag one render callback behind reality", but it is also
  `false` for a voice whose `Play` command has not been *applied* yet
  (`commands_applied >= play_sequence` gates it). A first implementation reclaimed
  records on `!is_voice_active` alone; that can forget a voice that starts on the next
  callback, leaving an untracked voice that a sample switch would not stop. Fixed with
  a completed-invocation baseline per record (see Decisions); the mutation batch that
  removed the wait made five tests fail.
- **The FIFO sequence of a command is not observable exactly.** `health()` reads
  `commands_applied` and then computes `pending_commands` from the control-side push
  counter, so a callback completing between those two loads makes
  `applied + pending + 1` a *lower* bound on a pending command's sequence, not the
  sequence itself. Deriving "my play was applied" that way would have been
  momentarily wrong in an unsafe direction, so the adapter uses completed render
  invocations instead of service internals. No service change is wanted or needed.
- **`stop_voice` returns `Ok(())` both for "already finished" and for "stop queued".**
  The result alone cannot tell them apart. The adapter treats both as accepted and
  clears its record, which is safe because a stop for a queued play is consumed in
  FIFO order: the mock render confirms such a voice never produces a sample.
- **There is no "stop all voices" command in the service**, so sample-switch cleanup
  needs per-voice handles. That is the reason the adapter keeps a bounded voice table
  at all, and the reason reclamation had to be made safe rather than heuristic.
- **A device open cannot fail on the host.** The mock always opens and
  `backend::open_backend` has no injection seam, so the failed-open branch is driven
  by a `#[cfg(test)]`-only field (`open_failure`) that makes the next open return a
  chosen error. This is fault injection, not a mocked result: the retry it enables runs
  the real `AudioService::new()` path.
- **Host runs cannot be used to judge the adapter's voice pressure.** Nothing drives
  the mock mixer outside tests, so a host run is silent and its eight voice slots stay
  occupied once used. That is the instrument, and it is now stated in the module
  documentation so no reader mistakes it for adapter behaviour.

## Decisions & Rationale

### Shape of the seam

- One adapter per process, owned by the application (`Experience`) and lent to the
  active sample. Voices are tagged with the [`AudioScope`] that started them, so
  `switch_to` retires exactly the leaving sample's voices instead of guessing.
  `AudioScope` carries the three apps `Experience` can activate (`Wetland`,
  `Sandbox`, `VoxelRelay`); the legacy sandbox triggers no events, but the variant
  keeps the wetland → sandbox transition from leaking wetland voices.
- The device is opened by the first `resume()`, never by construction. Android's
  activity lifecycle is the right moment to open AAudio, `Experience::new()` runs
  before the event loop, and this keeps "never resumed ⇒ no device touched" true by
  construction. `poll_device()` deliberately never opens a device that was never
  opened, so a failing device cannot be reopened every frame; the next `resume()`
  retries. Both retry points are lifecycle events, not the frame loop.
- No `cfg(target_os)` and no backend type anywhere in the module: the backend is
  chosen by the target-scoped Cargo features the lead already added. The public
  surface is `GameplayEvent`, `AudioScope`, `TriggerOutcome`, `DropReason`,
  `AdapterCounters`, `AdapterStatus` and `AudioAdapter` methods. `AdapterStatus`
  carries `Option<AudioServiceError>`; that enum is documented in its own crate as
  "descriptive plain data: no backend (NDK) types leak through the public API", so it
  is plain Rust, and mirroring it would only duplicate a verified vocabulary.
- `Clips` is a struct of six named `ClipHandle` fields rather than an array indexed by
  a variant index: the compiler then forces a new variant to be registered, and no
  index can drift from the vocabulary. Clip handles are `Copy`, so this costs nothing.
- One synthetic clip per event (deterministic xorshift noise and swept sines, no
  dependency and no asset files). Measured on the final tree: footstep 2640 samples
  (55 ms, peak 0.247), jump 6720 (140 ms, 0.420), land 9120 (190 ms, 0.495),
  block-edit 3600 (75 ms, 0.377), objective 18720 (390 ms, 0.380), ui-confirm 4080
  (85 ms, 0.335) — 44880 samples, 179520 bytes retained, i.e. 4.3 % of the 4 MiB
  budget and 6 of 32 clip slots. Registration happens once per opened device; a
  registration failure drops the service and leaves the adapter silent, because a
  half-registered device is not a usable state.

### Reclamation of the voice table

- A record may be forgotten only when the service *confirms* the voice is not
  audible, never on a guess. The confirmation is
  `callback_count - baseline >= 2 && !is_voice_active(handle)`, where `baseline` is
  read after the play: commands are drained at the start of a render invocation, so
  two further completed invocations guarantee this voice's `Play` (or its `Stop`) was
  applied before the live mask that `is_voice_active` reads. The baseline is read
  after the play on purpose — later is strictly more conservative.
- Consequence: a full table means no service voice slot is free either, so the
  adapter reports `DropReason::VoiceLimit` instead of stealing a sounding voice. The
  service's own "voices are never stolen" rule is preserved at the adapter level.
- Bookkeeping is bounded by the service limit (8 records). Stops that the service
  cannot accept because its command queue is full are *deferred*, not dropped: the
  record keeps `scope = None` and the next maintenance pass (`poll_device`, `resume`)
  retries. Without that, a full queue could leave a leaving sample's voice sounding.
- Mute is a start gate plus a gain push: no voice is started while muted, and sounding
  voices get `set_voice_gain(..., 0.0)`. Unmute restores each record's own level.
  `set_volume` scales sounding and subsequent voices the same way. Failures of a gain
  push are counted (`cleanup_backpressure`) and retried; a `VoiceNotActive` /
  `StaleVoiceHandle` answer removes the record, because that answer is authoritative.
- `suspend` sets the intent flag first and then asks the service; if the device refuses
  the pause, the failure is recorded and the adapter stays suspended *by intent* (no
  new events, no device opens) while `AdapterStatus::device_errors` and the service's
  own `suspended` flag show the real state. Events during suspension are dropped
  rather than queued: a footstep replayed minutes later after resume is worse than a
  missing one, and dropping cannot leak records.
- Counters are the observability surface, and nothing in the module claims audibility.
  The counters are monotonic over the adapter lifetime and satisfy
  `started + dropped == triggered` and `dropped == Σ dropped_*`, which a test asserts
  so a future early return cannot silently skip accounting.

## Solutions Applied

### Verification

All commands were run in the worktree on the final tree unless stated otherwise.

| Check | Result | Evidence |
| --- | --- | --- |
| Adapter unit tests (12) | PASS | `cargo test --locked -p matterweave-explorer --lib audio_service` → `12 passed; 90 filtered out`, 0 failed |
| Workspace tests | PASS | `cargo test --workspace --locked` → exit 0, 506 tests passed in 44 suites, 0 failed |
| Formatting | PASS | `cargo fmt --all -- --check` clean after `cargo fmt --all` |
| Strict clippy | PASS | `cargo clippy --workspace --all-targets --locked -- -D warnings` → exit 0, no diagnostic in `audio_service.rs`; the single reported warning is the pre-existing `clippy::fn_to_numeric_cast` in vendored `winit` (dependency lints are capped; it is present without this change) |
| Test discriminating power | PASS | Three mutation batches, each rebuilt and rerun: (A) removing the muted/suspended/lost-stream guards failed 4 tests; (B) removing the switch retirement, the gain push and the two-callback wait failed 7 tests; (C) dropping records instead of deferring on queue-full, and removing the reclamation sweep, failed 2 tests. Mutated file replaced byte for byte afterwards (`cmp`) |
| Docs link/consistency checker | PASS | `python3 tools/check_docs.py` (recorded below) |
| Android build, device run, audibility | NOT RUN | Lead-owned; no device or native build was attempted by this worker |
| Miri / instrumented alias check | NOT RUN | Not applicable to this change (no `unsafe`, no new concurrency), and Miri is not installed |

DoD rows (worker-owned rows only; lead-owned rows are listed as NOT RUN):

| Criterion | Result | Evidence |
| --- | --- | --- |
| Event enum plus start/stop/update methods, no Android or backend types in that surface | PASS | Public surface is `GameplayEvent`, `AudioScope`, `TriggerOutcome`, `DropReason`, `AdapterCounters`, `AdapterStatus`, `AudioAdapter`; `grep -nE "ndk\|jni\|android_activity\|cfg\(target_os"` matches only the module docs that state their absence; no `unwrap`/`expect`/`panic!`/`debug_assert` before the test module |
| Clip registration once and reused, within `MAX_CLIPS` / `MAX_PCM_BYTES` | PASS | `clips_register_once_and_stay_within_service_limits`: 6 registered, 179520 bytes of 4 MiB, all six handles registered and distinct, `registered_clips`/`registered_pcm_bytes` unchanged after triggering all six events, `registration_failures == 0` |
| Backpressure: more concurrent events than `MAX_VOICES` drops without panicking, unwrapping or corrupting state; drop observable | PASS | `voice_pressure_drops_events_without_losing_adapter_state`: 9th event → `Dropped(VoiceLimit)`, `started == 8`, `dropped_voice_limit == 1`, table still 8 deep, the eight voices do render, and after two completed invocations of a finished clip the next event starts again (`started == 9`) |
| Mute/unmute without teardown; a muted adapter starts no audible voice | PASS | `mute_silences_without_tearing_down_the_device`: muted trigger → `Dropped(Muted)` with no record, mock render is exact zeros, the pre-mute voice is silenced, unmute restores it, `is_available()` stays true and `stream_recreations == 0` |
| Suspend then resume restores a working adapter; events during suspend leak nothing | PASS | `suspend_drops_events_and_resume_restores_playback`: two suspended events → `Dropped(Suspended)`, `tracked_voices` unchanged, service `suspended == true` and a mock render is silent; after resume the service is unsuspended, a new event starts and renders, `started == 2` |
| Sample switch cleanup: leaving sample's voices stopped, no live voice or stale handle left | PASS | `switch_to_retires_the_leaving_scope_voices`: three queued (never rendered) wetland voices, `switch_to(VoxelRelay)` → `tracked_voices == 0`, the next mock render is exact zeros and every captured handle is inactive; the arriving scope plays, and leaving it retires its voices too; a repeat switch changes no counter |
| Unopenable device degrades to silence, condition observable, no panic | PASS | `failed_open_degrades_to_silence_and_the_next_resume_retries` (injected open failure → `last_error`, `device_failures == 1`, `registered_clips == 0`, trigger → `Dropped(NoDevice)`, `poll_device` does not open, the next resume opens and plays) and `lost_device_degrades_to_silence_and_poll_recovers` (mock `inject_device_error(-2)` → `is_available()` false and `Dropped(NoDevice)`, `poll_device` recreates, `device_errors == 1`, playback resumes) |
| Backend selection explicit in the module doc comment (mock on host, AAudio on Android, by feature) | PASS | Module docs, "Backend selection is compile time, not a runtime guess"; no `cfg(target_os)` in the module |
| `cargo test --workspace --locked` | PASS | exit 0, 506 passed, 0 failed |
| `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings` | PASS | both exit 0 |
| `experience.rs` lifecycle wiring and both samples' call sites | NOT RUN | Lead-owned integration follow-up |
| Real Android run and human audibility confirmation | NOT RUN | Lead-owned device work; nothing here claims a phone produced sound |

Additional coverage in the same suite: vocabulary coverage and plain-data compile
check (`vocabulary_is_complete_and_plain_copy_data`), start-gain scaling and
sounding-voice attenuation (`volume_scales_new_voices`, `volume_updates_a_sounding_voice`),
deferred cleanup retry under a saturated command queue
(`deferred_cleanup_is_retried_by_the_next_maintenance_pass`) and counter accounting
(`counters_account_for_every_trigger`).

### Test-only scaffolding and temporary compromises

- `AudioAdapter::open_failure` (`#[cfg(test)]` field) is the only test hook added to
  the production type. It exists because the mock cannot fail an open; it is consumed
  by the next `open_device()` and cannot affect a non-test build.
- The module carries a temporary `#![allow(dead_code)]` with an explicit comment: the
  adapter is not wired yet, so strict clippy would otherwise fail on every unused
  public item (confirmed by removing it and observing the errors). It should be
  removed with the wiring commit. The repository already uses this pattern
  (`apps/explorer/src/wetland_state.rs:91`).

## Insights

- "Not audible" and "finished" are different facts and the service exposes no direct
  predicate for the second. The adapter reconstructs it from a public, monotonic
  signal (completed render invocations) rather than from the service's private FIFO
  sequence, which public counters cannot reproduce exactly because `health()` reads
  `commands_applied` and `pending_commands` non-atomically.
- A bounded voice table with an exact reclamation rule makes sample-switch cleanup
  correct without stealing, without locks and without touching the verified service.
  Heuristic reclamation is what makes the "forgot a live voice" bug possible; the
  two-invocation wait is what makes it impossible.
- Backpressure exists at two levels — the service's eight voices and its 64-command
  queue — and both must degrade into gameplay-visible drops rather than errors. The
  second one also applies to *cleanup* commands, which is why stops are deferred and
  retried instead of best-effort.
- Host tests exercise the mock, which never establishes device behaviour, and the
  mock's queue only drains when a test renders it. Both facts limit every claim in
  this log to "the adapter and the mixer", never to AAudio or to a phone.
- The lead still has to confirm the only things this slice cannot: that
  `Experience` wires `resume`/`suspend`/`poll_device`/`switch_to` at the right
  lifecycle points, and that a real device actually produces sound.
