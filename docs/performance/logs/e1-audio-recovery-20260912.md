# E1 audio recovery — 2026-09-12

## Actions Taken

- Read the prior worker's engineering log, initial diff, repository instructions,
  audio service/mixer/command queue, mock and AAudio implementations, and relevant
  project context. Preserved the earlier worker's audio suspension observability
  correction rather than replacing it.
- Added four acceptance regressions for device loss and diagnostic recreation,
  each with applied and buffered Suspend. All four executed and failed on pause
  preservation before the repair. The extended host diagnostic also failed at
  `cycle 1: recreation preserves pause` before the repair.
- RED checkpoint: `e582970` (`test(audio): reproduce suspended output recovery failure`).
  **Attribution:** this checkpoint includes the earlier audio-suspend-observability
  worker's uncommitted service/RT health split, acceptance assertions, mixer comment,
  README/lib documentation and diagnostic changes. Those are recovered prior work,
  not newly authored by this recovery worker. New work adds recovery regressions and
  repeated suspended-recreation diagnostic cycles.
- Repaired shared recreation handling and added five private service fault/state
  tests. Retained all existing acceptance and callback-allocation tests.
- Extended the diagnostic's ten default cycles to
  open/play/suspend/recreate-while-suspended/resume/stop/close. Every frame-progress
  target must be reached within two seconds; resumed callbacks must advance and
  report an unsuspended mixer. All services are explicitly dropped before PASS.

## Issues & Friction

- The initial poll path consumed the only device-error notification, then failed
  to retry if open/start failed. It also unconditionally started a suspended
  replacement while its mixer still had an applied or queued Suspend.
- The existing mock permits manual rendering while paused to model in-flight
  callbacks. Merely not calling that method cannot prove the service avoided start:
  a private backend wrapper therefore counts start requests as well as testing
  exact PCM. No public mock or gameplay API was added.
- Tool output summarized the RED test run misleadingly as skipped. The captured
  Cargo output shows **four executed failures**, all at the pause-preservation
  assertion; no tests were skipped.
- An exact edit initially matched two diagnostic fixture registrations and was
  rejected without modifying the file; narrowed replacements resolved it. A later
  test edit needed the rustfmt-normalized line rather than guessed whitespace.
- AAudio pause is an asynchronous request, not a join. Stable counters are checked
  only after controlled recreation has closed the old stream and left the new one
  unstarted, never immediately after suspend returns.

## Decisions & Rationale

- Reused the existing private `OutputBackend` trait and `backend::open_backend`.
  The only seam is a private `recreate_output_with` opener closure, enabling failed
  opens and wrapped start failures. No factory field, dependency, callback lock,
  callback mutation, wait loop or new queue was needed.
- Service `suspended` remains the successful pause intent across recreation and
  failed recovery, even while the stream is closed. This deliberately corrects the
  prior worker's clear-on-recreation behavior while retaining the observability
  split. `rt_suspended` remains the last callback's view, not device quiescence.
- `recovery_pending` survives consumed errors and failed opens/starts. Failed starts
  close/drop output and do not claim `running`; later polls retry. A suspended
  recovery only opens output. A failed resume retains suspension until an explicit
  resume retry succeeds (poll alone does not undo pause).
- `resume_queued` retains one FIFO Resume across failed starts. Tests retry 65 times
  (more than the 64-command capacity), with both applied and buffered Suspend.
  This avoids retry-induced queue saturation and preserves exact continuation.
- Recreation does not require queue space. A genuinely full queue still rejects a
  new Resume without falsely reporting running; the existing bounded backpressure
  policy is unchanged.
- Verified fixture facts: `make_sine_clip` generates 24,000 scalar mono samples
  (0.5 seconds at 48 kHz, 440 Hz). It was previously registered as stereo, making
  it 12,000 stereo frames with adjacent samples in different channels. Both
  diagnostic registrations now use `ClipSpec::mono`. PCM continuation regressions
  use exactly representable binary fractions, not a constant signal that could hide
  cursor resets.

## Solutions Applied

- `src/service.rs`: one recreation path shared by poll and diagnostic; preserve
  suspension, durable recovery retries, close failed starts, bounded Resume retry.
- `src/service/tests.rs`: five tests for no start/no callback during suspended
  replacement, failed opens in running/suspended states, failed recovery start,
  repeated failed resumes, and full-queue suspended recovery. Successful retries
  verify exact remaining PCM and one voice completion. Resume retry is exercised
  both directly and after a poll.
- `tests/recovery.rs`: four tests, each doing three repeated recreations with no
  callbacks while paused, preserving command count, resuming the exact remaining
  three frames, completing once and then rendering silence.
- `examples/audio_diagnostic.rs`: repeated paused recovery, stable post-close
  counters, bounded resumed progress, mono fixture correction and explicit shutdown.
  Submitted frames are explicitly not proof of nonzero PCM or audibility.
- Crate documentation explains pause intent and retry behavior. Global STATUS and
  ADR changes were neither edited nor staged. Their initial/final SHA-256 values:
  - `docs/STATUS.md`: `73076404114a4e5726b10349f21f93fda1b42b1a8c531e89bb0f2a27d6cf2f9f`
  - `docs/adr/0016-audio-service.md`: `1295b92ca250d8588ca01b5847760a05002c1496beb0394f90c79c588918a343`

### Verification

All Cargo commands use:

```sh
export CARGO_TARGET_DIR=/mnt/bench/matterweave-dev/completion-20260912/targets/e1-audio-recovery
export CARGO_BUILD_JOBS=2
cargo test -p matterweave-audio --locked
cargo clippy -p matterweave-audio --all-targets --locked -- -D warnings
cargo fmt -p matterweave-audio -- --check
cargo run -p matterweave-audio --locked --example audio_diagnostic -- --cycles 10
python3 tools/check_docs.py
git diff --check
```

| Criterion | Result | Evidence |
| --- | --- | --- |
| Suspended recovery preserves pause and exact PCM | PASS (host) | Four integration regressions plus backend start-count and FIFO tests; buffered/applied Suspend and diagnostic recreation covered. |
| Failed open/start retains truthful state and retry intent | PASS (host) | Five fault/state tests; closed properties and false running on failure, polls retry, explicit resume continues exact PCM without duplicate Resume. |
| Prior observability correction preserved | PASS (host) | All 21 prior acceptance tests pass; ten-cycle diagnostic reports service suspended with buffered Suspend and RT mirror false, then both false after resumed progress. |
| Focused tests, strict scoped clippy and formatting | PASS | 32 tests: 6 unit, 21 acceptance, 4 recovery, 1 allocation; clippy with `-D warnings`, scoped fmt and diff checks pass. |
| Real Android repeated lifecycle and independent reviews | NOT RUN | Lead-owned; no phone access, Android builds, descendants or independent reviewers used. |

Documentation verification: `python3 tools/check_docs.py` PASS — 160 Markdown files,
529 local links, 16 ADRs and 20 requirements. `git diff --check` PASS.

Host diagnostic: every recovery cycle holds at 2 callbacks / 1,920 submitted frames
with service suspended, RT mirror false and one pending command; resumes to 5
callbacks / 4,800 frames, both suspension flags false and no pending commands.
Each cycle reports shutdown; final service shutdown completes before PASS. These
are synchronous mock counts, not Android timing, acoustic or performance evidence.

Raw local command output is under
`/mnt/bench/matterweave-dev/completion-20260912/audio-recovery-{red,test,clippy,diagnostic-red,diagnostic}.txt`.
The initial diff is preserved there as `audio-recovery-initial.diff`. This log is the
durable summary; build targets are removed at handoff after checks complete.

## Insights

- Keeping the mixer core is necessary but insufficient: its queued lifecycle
  commands and the service's intended output state must survive together.
- Reopening an output is not equivalent to starting it. A paused replacement must
  not request start, even if no callback has yet applied Suspend.
- Callback/frame counters prove callback progress, not nonzero PCM or audibility.
  Exact continuation is established by deterministic host PCM tests; actual AAudio
  behavior and listening remain lead-owned gates.
- No instrumented coverage percentage or Android result is claimed. Failed native
  close/driver hangs remain governed by the existing ndk drop contract; this task
  adds no shutdown timeout or unsafe workaround. Real asynchronous error timing
  and repeated native shutdown still require the lead's device run and reviews.
