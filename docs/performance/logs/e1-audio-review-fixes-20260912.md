# E1 audio corrective review fixes — 2026-09-12

## Actions Taken

- Resumed at `dd6c305`; preserved the earlier audio suspension/recovery work and
  untouched uncommitted `docs/STATUS.md` / ADR-0016 changes. Read the relevant
  service, mixer, FIFO, failure-backend and allocation-test implementation.
- Added deterministic regressions before fixing behavior. RED checkpoint
  `540b4d4` records **nine new failing unit tests and two failing PCM integration
  tests**; the six existing unit tests remained green.
- Reproduced the three lead findings and both added Gemini lifecycle findings.
  The callback/enqueue race is real, not merely a documentation ambiguity.
- Repaired Unload generation selection, pause-failure compensation, completed
  command acknowledgment and voice-generation gating. Fixed stale error clearing
  order and pre-poll property visibility in both AAudio and mock implementations.
- Extended the allocation detector to cover Play/Unload silencing and command
  acknowledgment inside the measured 10,000-callback interval.
- Ran focused host tests, strict scoped clippy, formatting, a twenty-cycle host
  diagnostic, documentation checks and diff checks. No descendants, phone access,
  Android builds, global documentation edits or pushes.

## Issues & Friction

- **Unload generation:** the control side sent the next generation but the mixer
  compared against the currently loaded generation. Output remained audible after
  unregister; the control mirror alone had hidden the rejection. RED selective
  silencing expected `[0.25, 0.25]` but received `[0.5, 0.5]`; full-pool retirement
  expected silence but received `[0.25, 0.25]`.
- **Pause failure:** one vacant FIFO slot accepted Suspend, then discarded the
  failed compensating Resume. The failure-backend test observed silent PCM rather
  than the remaining exact ramp while the service still reported running.
- **Callback epoch is not command acknowledgment:** a callback drained first; the
  fixture enqueued Unload; that callback mixed old PCM and incremented its epoch.
  The old implementation then accepted full-pool registration, despite Unload
  remaining queued. RED returned `Ok(ClipHandle { slot: 0, generation: 3 })` where
  `RangeNotYetAcknowledged` was required. No unsafe concurrent overwrite is needed
  to demonstrate the invalid reuse decision.
- **Same race affected voices:** eight Play commands enqueued after drain were
  mistakenly treated as finished when that callback completed. A subsequent Stop
  was discarded as a no-op, producing `[0.25, 0.25]` instead of silence. A new voice
  generation could also observe its predecessor's live bit before its own Play.
- **Device errors:** a failed start's callback notification survived replacement
  and caused another teardown. Properties remained visible when only the shared
  error callback flag was set. Clearing errors at the end of recreation could
  erase the replacement stream's error code.
- Two exact-edit batches contained an unnecessary unmatched entry and were
  rejected atomically; corrected targeted edits applied. This was an edit-input
  error, not a source or verification failure.

## Decisions & Rationale

### FIFO acknowledgment proof (not a complete Rust aliasing proof)

1. Reuse the existing pinned `ringbuf` SPSC queue and existing cumulative command
   counter, rather than introducing a second queue, waits or guessed callback
   delays. The sole producer increments `commands_pushed` only after a successful
   push. Each retired range records its Unload sequence; each voice records its
   Play sequence (`AudioService::unregister_clip`, `play`).
2. `MixerCore` counts applied FIFO commands locally, then publishes that cumulative
   count with **Release after all PCM reads and live-mask writes**. Incrementing a
   shared atomic during command drain was insufficient; a separate regression
   verifies that progress is not published at the after-drain cut point.
3. `reclaim_acknowledged_ranges` loads completed progress with **Acquire** and only
   reuses ranges whose Unload sequence is covered. A callback that drained before
   enqueue cannot advance that sequence. The matching Unload has silenced every
   voice referencing the clip before publication; FIFO ordering and stale-handle
   validation prevent a later old-clip Play from reintroducing the reference.
4. Voice allocation, finished-voice checks and `is_voice_active` require the Play
   sequence to be acknowledged before trusting its live bit. Acquire of the
   sequence precedes the mask read. A later mask for the same generation may report
   completion conservatively; an older-generation bit cannot authorize a newly
   queued Play. Unregister retires control mirrors immediately, but its Unload
   precedes any replacement Play in the FIFO.
5. `ack_epoch` remains public diagnostic telemetry only. `commands_applied` now means
   commands covered by a completed callback; `pending_commands` includes the
   in-flight batch as well as FIFO occupancy. No public gameplay signature changed.

### Other minimal repairs

- Unload carries the **current** clip generation; the control mirror still advances
  its generation to invalidate handles. The mixer retains its existing equality
  guard, so a stale Unload cannot retire a successor. Integration tests check
  actual selective PCM, full-pool reuse and stale clip/voice handle rejection.
- Suspend reserves **two** vacant FIFO slots before any mutation. With fewer it
  returns `CommandQueueFull`. If device pause fails, the reserved compensating
  Resume cannot fail: there is one producer and the consumer can only free space.
  Tests cover zero, one and two vacant slots and exact subsequent PCM.
- Reuse the existing private `FaultOutput` seam for pause/start failures and an
  error callback during close. Shared old-stream error state is cleared **after
  close/drop joins old callbacks and before opening replacement**. No clearing
  follows new open/start. The test opener asserts old state is clear, injects a
  new error and verifies its code survives for the next poll.
- AAudio `properties()` now reads shared `disconnected` with Acquire as well as its
  control-side `lost` flag. The mock follows the same rule. A host regression writes
  precisely the error-callback atomics, without setting mock `lost`, and verifies
  properties disappear before poll. The AAudio method itself is source-reviewed,
  not Android-compiled or device-executed in this worker.
- The deterministic fixture invokes the actual mixer with a private generic
  after-drain closure. Production supplies a no-op; tests enqueue synchronously
  while the mock core is detached, without threads, locks, sleeps or allocations
  in the production callback path. No new dependency or public seam was needed.

## Solutions Applied

Changed audio paths: crate README; `src/service.rs`, `src/mixer.rs`,
`src/command.rs`, `src/lib.rs`, `src/backend/{aaudio,mock}.rs`;
`src/service/tests.rs` and its `lifetime.rs` / `device_errors.rs` submodules;
`tests/lifetime.rs` and `tests/rt_allocations.rs`.

All Cargo commands use:

```sh
export CARGO_TARGET_DIR=/mnt/bench/matterweave-dev/completion-20260912/targets/audio-review-fix
export CARGO_BUILD_JOBS=2
cargo test -p matterweave-audio --locked
cargo clippy -p matterweave-audio --all-targets --locked -- -D warnings
cargo fmt -p matterweave-audio -- --check
cargo run -p matterweave-audio --locked --example audio_diagnostic -- --cycles 20
python3 tools/check_docs.py
git diff --check
```

| Criterion | Result | Evidence |
| --- | --- | --- |
| Correct unregister silencing and acknowledged PCM reuse | PASS (host) | Two PCM integration tests; deterministic enqueue-after-drain full-pool regression; selective silencing and stale handles checked. |
| Pause failure cannot leave mixer silently suspended | PASS (host) | Failed-pause tests with zero/one/two vacant FIFO slots and exact ramp continuation. |
| Lifetime/voice sequencing under delayed acknowledgment | PASS for sequence ordering; overall soundness BLOCKED | Four additional deterministic command-publication/voice-generation regressions pass. The separate live-core/PCM aliasing issue below remains unresolved. |
| Package tests, allocations, clippy, fmt and diagnostic | PASS | 43 tests: 15 unit, 21 acceptance, 2 lifetime, 4 recovery, 1 allocation; strict scoped clippy/fmt; twenty diagnostic recovery/shutdown cycles. |
| Gemini stale/new error lifecycle and pre-poll properties | PASS (host + source inspection) | Three fault regressions, including error during close, failure during start, error during replacement open and atomics-only notification. Native method execution NOT RUN. |
| Live-core/PCM reference aliasing | FAIL / unresolved delivery blocker | Late lead audit identifies overlapping mutable ownership; source evidence and required repair below. |
| Independent corrective review and repeated Android checks | NOT RUN | Lead-owned; this tree needs the aliasing repair, re-review and a new device run before delivery. |

`python3 tools/check_docs.py` PASS: 161 Markdown files, 529 local links, 16 ADRs
and 20 requirements. `git diff --check` PASS.

The host diagnostic holds recreated-paused cycles at 2 callbacks / 1,920 frames,
service suspended with buffered Suspend and RT mirror false. Each resume reaches
5 callbacks / 4,800 frames, both suspension flags false, followed by shutdown.
These synchronous mock counts prove neither native timing nor audibility.
The lead reported earlier **4 × 20 AAudio cycles PASS at integrated `22070ca`**;
that result predates these corrective changes and is not validation of this tree.

Retained local logs:
`/mnt/bench/matterweave-dev/completion-20260912/audio-review-fixes/`
contains `red-unit.txt`, `red-lifetime.txt`, `test.txt`, `clippy.txt`, `fmt.txt`,
`diagnostic.txt` and `docs.txt`. The isolated build target is removed after all
checks finish; these text logs remain.

Global changes were neither edited nor staged; initial/final SHA-256:

- STATUS: `73076404114a4e5726b10349f21f93fda1b42b1a8c531e89bb0f2a27d6cf2f9f`
- ADR-0016: `1295b92ca250d8588ca01b5847760a05002c1496beb0394f90c79c588918a343`

### Unresolved late finding: live-core/PCM reference aliasing

The lead raised this directly related soundness concern near the end of the bounded
worker run. Source inspection supports it:

- `src/service.rs:240` retains `CorePtr(&mut *core as *mut MixerCore)` for callbacks.
- `src/service.rs:336` registers PCM through
  `self.core.as_mut().expect("core present").pool[...] .copy_from_slice(...)`.
- `src/backend/aaudio.rs:126` calls `(*core_ptr).render(out, channels)` while the
  callback may be live; `src/mixer.rs:252` takes `&mut self` for that render.
- `src/mixer.rs:126` stores the entire PCM allocation as `Box<[f32]>` inside the
  same core. Merely proving that sample indices do not overlap does **not** justify
  simultaneous mutable core/Box/slice references under Rust's aliasing rules.

**Unfixed delivery blocker.** The sequence repair is necessary for avoiding premature
PCM reuse, but does not establish reference soundness. Synchronous mock tests cannot
validate concurrent aliasing, and no Miri/alias-model validation was run. A safe
follow-up should separately own the shared PCM allocation, keep registration from
borrowing the live MixerCore at all, and review narrow UnsafeCell/raw range access
contracts together with the FIFO publication/retirement proof. Avoid forming a
mutable reference to the entire pool merely to select a disjoint subrange.
This ownership/unsafe-contract repair and its validation exceed the remaining
900-second worker budget; no speculative partial unsafe rewrite was attempted.
Do not treat the current tree as lifetime-sound or ready for delivery. The lead
reports Oracle is independently evaluating the frozen `22070ca` pool alias design;
no findings were available at this handoff. A resumed worker should read that
review before selecting the bounded PCM ownership repair.

## Insights

- A completed callback acknowledges its own drain, not all commands that happened
  to be enqueued before its completion. Publishing the exact consumed FIFO prefix
  after PCM reads resolves both reclamation and voice-generation races.
- Control mirrors and constant-signal tests can conceal audible lifetime errors;
  discriminating PCM and controlled interleaving exposed them directly.
- Error flags belong to a stream lifetime even when their storage is shared across
  replacements. The safe reset boundary is after old callbacks join, before new
  callbacks can publish.
- No instrumentation-based coverage percentage or native runtime result is
  claimed. Host checks pass, but the live-core/PCM aliasing repair is an explicit
  delivery blocker before independent corrective review and the lead's Android
  rebuild/rerun. The command-sequence proof must not be presented as full lifetime
  soundness.
